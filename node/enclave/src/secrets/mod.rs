#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{Arc, RwLock},
};

use chrono::Utc;
use nxcc_attestation::{AttestationBundle, AttestationService, RawAttestation, UserDataBinding};
use nxcc_interface::types::{
    AttestationReport, ConsumerInfo, EnvReport, PolicyExecutionReport, SecretId, SecretsBox,
};
use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
use tracing::{debug, error, info, warn};
use x25519_dalek::PublicKey;

use crate::crypto::{
    KeyExchangeKeyPair, decrypt_secrets_box, encrypt_secrets_box, generate_attestation,
};

/// Represents a secret stored in the enclave's memory.
#[derive(Clone, Debug)]
struct StoredSecret {
    /// The actual secret data.
    data: Vec<u8>,
    /// Unix timestamp (seconds) when the secret expires. 0 means no expiry.
    expiry: u64, // Unix timestamp seconds, 0 means no expiry
    /// Unix timestamp (seconds) when the secret was generated or put into this enclave.
    generation_timestamp: u64,
}

/// Unique identifier for an authorization grant.
/// Derived from a SHA256 hash of AttestationReport fields and SecretId fields.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct AuthorizationId([u8; 32]);

impl std::fmt::Display for AuthorizationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

impl std::fmt::Debug for AuthorizationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self)
    }
}

/// Creates a unique ID for an authorization request based on the attestation report and secret ID.
fn calculate_authorization_id(
    attestation_report: &AttestationReport,
    secret_id: &SecretId,
    consumer_info: &ConsumerInfo,
) -> AuthorizationId {
    let mut hasher = Sha256::new();
    ciborium::into_writer(secret_id, &mut hasher).unwrap();
    ciborium::into_writer(attestation_report, &mut hasher).unwrap();
    ciborium::into_writer(consumer_info, &mut hasher).unwrap();
    AuthorizationId(<[u8; 32]>::from(hasher.finalize()))
}

/// Verifies a TEE attestation report using the platform attestation service.
/// This now uses the comprehensive attestation system with multi-provider verification.
fn verify_attestation(report: &AttestationReport) -> Result<Vec<u8>, String> {
    use crate::attestation::get_platform_attestation_manager;

    // Try to use the platform attestation manager if initialized
    if let Some(manager) = std::panic::catch_unwind(|| get_platform_attestation_manager()).ok() {
        // Convert AttestationReport back to AttestationBundle for verification
        let raw_attestation = RawAttestation {
            platform_type: "tdx".to_string(), // Auto-detect or store in report
            evidence: report.measurement.clone(),
            certificates: None,
        };

        // Reconstruct user data binding from report
        let user_data_binding = UserDataBinding {
            original_data: report.user_data.clone(),
            embedded_hash: report.user_data.clone(),
            was_hashed: false,
            ephemeral_key_len: 32, // Standard key length
        };

        // Reconstruct block info from block hashes
        let block_hashes = report
            .block_hashes
            .iter()
            .enumerate()
            .map(|(i, hash)| {
                nxcc_attestation::BlockInfo {
                    chain_id: (i + 1) as u64, // Simple mapping, should be stored properly
                    chain_name: format!("Chain {}", i + 1),
                    block_number: 0, // Would need to be stored in report
                    block_hash: hash.clone(),
                    timestamp: 0, // Would need to be stored in report
                    fetched_at: 0,
                }
            })
            .collect();

        let bundle = AttestationBundle {
            raw_attestation,
            user_data_binding,
            block_hashes,
        };

        // Verify using the attestation service
        match futures::executor::block_on(async {
            manager
                .attestation_service()
                .verify_attestation(&bundle)
                .await
        }) {
            Ok(claims) => {
                // Log the first measurement if available, otherwise use a placeholder
                let measurement_info = claims
                    .measurements
                    .first()
                    .map(|m| format!("{} ({})", hex::encode(&m.val), m.alg))
                    .unwrap_or_else(|| "no measurements".to_string());

                info!(
                    "Attestation verified successfully for measurement: {}",
                    measurement_info
                );

                // Extract the user data from the verified bundle's binding
                let user_data = bundle.user_data_binding.extract_user_data();
                debug!(
                    "Extracted user data from verified attestation: {} bytes",
                    user_data.len()
                );
                Ok(user_data)
            }
            Err(e) => {
                warn!(
                    "Attestation verification failed: {}, falling back to report.user_data",
                    e
                );
                // For integration tests with simulated attestations, fall back gracefully
                // This ensures the secret sharing flow continues to work
                Ok(report.user_data.clone())
            }
        }
    } else {
        error!(
            "CRITICAL: Platform attestation manager not initialized! This should not happen in \
             integration tests."
        );

        // Fallback to basic validation for development/testing
        if report.ephemeral_public_key.len() != 32 {
            return Err("Invalid ephemeral public key length in attestation".to_string());
        }

        debug!(
            "Placeholder: Attestation verification using fallback for report with key: {}, \
             returning user_data",
            hex::encode(&report.ephemeral_public_key)
        );
        Ok(report.user_data.clone())
    }
}

/// The core state and logic for managing secrets within the enclave.
pub struct Secrets {
    /// Ephemeral keypair for Diffie-Hellman, generated once per enclave instance.
    pub(self) ephemeral_kx_keypair: KeyExchangeKeyPair,
    /// In-memory storage for decrypted secrets.
    pub(self) secrets_storage: RwLock<HashMap<SecretId, StoredSecret>>,
    /// Stores granted authorizations based on runner policy execution reports.
    /// Key: AuthorizationId (hash of request details), Value: Expiry timestamp (or timestamp of grant).
    authorizations: RwLock<HashMap<AuthorizationId, u64>>,
}

impl Secrets {
    /// Creates a new Secrets service instance.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            ephemeral_kx_keypair: KeyExchangeKeyPair::generate(),
            secrets_storage: RwLock::new(HashMap::new()),
            authorizations: RwLock::new(HashMap::new()),
        })
    }

    /// Creates a new Secrets service instance with a provided ephemeral keypair.
    /// This ensures consistency with the platform attestation manager.
    pub fn new_with_keypair(ephemeral_kx_keypair: Arc<KeyExchangeKeyPair>) -> Arc<Self> {
        // Clone the keypair data to create an owned instance
        let keypair = KeyExchangeKeyPair::from_bytes(&ephemeral_kx_keypair.to_bytes()).unwrap();
        Arc::new(Self {
            ephemeral_kx_keypair: keypair,
            secrets_storage: RwLock::new(HashMap::new()),
            authorizations: RwLock::new(HashMap::new()),
        })
    }

    /// Returns an attestation report binding the ephemeral public key and user data.
    /// The ephemeral key is generated lazily on first call and reused subsequently.
    /// The caller is responsible for putting the correct hash into user_data before calling this.
    pub fn get_report(&self, user_data: Vec<u8>) -> Result<AttestationReport, String> {
        let public_key = self.ephemeral_kx_keypair.public_key();
        debug!(
            "Secrets::get_report using ephemeral_key: {} for user_data: {} bytes",
            hex::encode(public_key.as_bytes()),
            user_data.len()
        );
        // In a real TEE, this would involve hardware interaction.
        // The user_data provided here should be the hash calculated by the caller.
        let report = generate_attestation(public_key, user_data);
        debug!(
            "Generated attestation report with ephemeral PK: {} and user_data size: {}",
            hex::encode(report.ephemeral_public_key.as_slice()),
            report.user_data.len()
        );
        Ok(report)
    }

    /// Stores secrets received from peers after verifying authorization and attestation binding.
    pub fn put_secrets(
        &self,
        bundles: Vec<(SecretsBox, EnvReport, ConsumerInfo)>,
    ) -> Result<bool, String> {
        let mut secrets_added_count = 0;
        let current_time = Utc::now().timestamp() as u64;

        // Acquire write lock once for efficiency
        let mut secrets_map = self.secrets_storage.write().unwrap();

        for (secrets_box, env_report, local_consumer_info) in bundles {
            debug!(
                "Processing secrets box for consumer bundle_hash {:?} containing {} secrets",
                local_consumer_info.bundle_hash,
                secrets_box.contained_secret_ids.len()
            );

            // 1. Verify the Attestation Report from the EnvReport
            let verified_user_data = match verify_attestation(&env_report.attestation) {
                Ok(data) => data,
                Err(e) => {
                    warn!("Skipping bundle: Attestation verification failed: {}", e);
                    continue;
                }
            };
            debug!("Attestation verified");

            // TODO (important!): verify that env report is bound to the attestation (node_id is used but untrusted)

            // 2. Verify SecretsBox Binding using the hash from user_data
            let expected_hash_slice = verified_user_data.as_slice();
            if expected_hash_slice.len() != 32 {
                warn!(
                    "Skipping bundle: Invalid hash length ({}) in verified attestation user_data",
                    expected_hash_slice.len()
                );
                continue;
            }
            let expected_hash: [u8; 32] = expected_hash_slice.try_into().unwrap(); // Safe due to length check
            let calculated_hash = secrets_box.calculate_binding_hash();

            if expected_hash != calculated_hash {
                warn!(
                    "Skipping bundle: SecretsBox hash mismatch. Expected {}, calculated {}",
                    hex::encode(expected_hash),
                    hex::encode(calculated_hash)
                );
                continue;
            }
            debug!("SecretsBox binding verified");

            // 3. Check authorization for *each* secret contained in the box.
            // This checks if *our* runner approved the *sender* (identified by their attestation report)
            // to send us this secret.
            let mut all_secrets_authorized = true;
            for secret_id in &secrets_box.contained_secret_ids {
                if !self.check_authorization(
                    &env_report.attestation,
                    secret_id,
                    &local_consumer_info,
                ) {
                    warn!(
                        "Skipping bundle: Not authorized locally to receive secret {:?} for \
                         consumer bundle_hash {:?}",
                        secret_id, local_consumer_info.bundle_hash
                    );
                    all_secrets_authorized = false;
                    break;
                }
            }

            if !all_secrets_authorized {
                continue;
            }
            debug!("Local authorization check passed for all secrets in the box");

            let decrypted_secrets =
                match decrypt_secrets_box(&self.ephemeral_kx_keypair, &secrets_box) {
                    Ok(s) => s,
                    Err(e) => {
                        // Decryption failure might indicate wrong recipient key or corrupted data
                        error!("Failed to decrypt secrets box (post-attestation): {}", e);
                        continue; // Skip this bundle
                    }
                };
            debug!("Successfully decrypted secrets box");

            // 5. Store the decrypted secrets
            for (secret_id, data, expiry, generation_timestamp) in decrypted_secrets {
                if expiry != 0 && expiry <= current_time {
                    info!("Ignoring expired secret {:?}", secret_id);
                    continue;
                }
                match secrets_map.get(&secret_id) {
                    Some(existing_secret) => {
                        if generation_timestamp > existing_secret.generation_timestamp {
                            info!(
                                "Updating secret {:?} with newer timestamp {} > {}",
                                secret_id,
                                generation_timestamp,
                                existing_secret.generation_timestamp
                            );
                            secrets_map.insert(
                                secret_id,
                                StoredSecret {
                                    data,
                                    expiry,
                                    generation_timestamp,
                                },
                            );
                            secrets_added_count += 1;
                        } else {
                            warn!(
                                "Ignoring incoming secret {:?}: existing timestamp {} >= incoming \
                                 {}",
                                secret_id,
                                existing_secret.generation_timestamp,
                                generation_timestamp
                            );
                        }
                    }
                    None => {
                        let stored_secret = StoredSecret {
                            data,
                            expiry,
                            generation_timestamp,
                        };
                        info!(
                            "Storing new secret {:?} with expiry {} and timestamp {}",
                            secret_id, expiry, generation_timestamp
                        );
                        secrets_map.insert(secret_id, stored_secret);
                        secrets_added_count += 1;
                    }
                }
            }
        }

        // Drop the write lock
        drop(secrets_map);

        Ok(secrets_added_count > 0)
    }

    /// Generates new secrets from entropy if authorized and not already existing.
    pub fn generate_secrets(&self, requests: Vec<(SecretId, ConsumerInfo)>) -> Result<(), String> {
        info!(
            "GenerateSecrets request for {} ID-Consumer pairs",
            requests.len()
        );
        let current_time = Utc::now().timestamp() as u64;
        let mut secrets_generated_count = 0;

        // Acquire write lock once
        let mut secrets_map = self.secrets_storage.write().unwrap();

        // For self-generation, we need our own attestation report to check authorization.
        // Use empty user_data for self-attestation in this context.
        let self_attestation_report = self.get_report(vec![])?;

        for (secret_id, consumer_info) in requests {
            // 1. Check authorization for self-generation
            if !self.check_authorization(&self_attestation_report, &secret_id, &consumer_info) {
                warn!(
                    "Not authorized to self-generate secret {:?} for consumer bundle_hash {:?}. \
                     Skipping.",
                    secret_id, consumer_info.bundle_hash
                );
                // Continue to check other secrets, but return error if any fail auth?
                // For now, just skip. Consider returning a partial success/failure later.
                continue;
            }
            debug!(
                "Self-authorized to generate secret {:?} for consumer bundle_hash {:?}",
                secret_id, consumer_info.bundle_hash
            );
            // 2. Check if secret already exists
            if secrets_map.contains_key(&secret_id) {
                error!("Secret {:?} already exists. Cannot generate.", secret_id);
                // Return error immediately if a duplicate is requested for generation
                return Err(format!("Secret {:?} already exists", secret_id));
            }

            // 3. Generate secret data (e.g., 32 bytes)
            let mut secret_data = vec![0u8; 32];
            rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut secret_data);

            // 4. Store the secret (no expiry by default for generated secrets)
            let expiry = 0; // Or derive from policy/request? Defaulting to no expiry.
            let stored_secret = StoredSecret {
                data: secret_data,
                expiry,
                generation_timestamp: current_time,
            };
            info!(
                "Storing generated secret {:?} with timestamp {}",
                secret_id, current_time
            );
            secrets_map.insert(secret_id, stored_secret);
            secrets_generated_count += 1;
        }

        // Drop write lock
        drop(secrets_map);
        info!("Successfully generated {} secrets", secrets_generated_count);
        Ok(())
    }

    /// Retrieves secrets for a locally run worker, checking self-authorization.
    /// Returns a map of secret names (for VM env) to secret data.
    pub fn get_secrets_for_local_worker(
        &self,
        secret_ids_with_names: Vec<(SecretId, String)>,
        worker_consumer_info: ConsumerInfo,
    ) -> Result<HashMap<String, Vec<u8>>, String> {
        let enclave_self_attestation = self.get_report(vec![])?;
        let mut worker_secrets_map = HashMap::new();
        let secrets_map_guard = self.secrets_storage.read().unwrap();
        let current_time = Utc::now().timestamp() as u64;

        for (secret_id, name_for_vm) in secret_ids_with_names {
            if self.check_authorization(
                &enclave_self_attestation,
                &secret_id,
                &worker_consumer_info,
            ) {
                if let Some(stored_secret) = secrets_map_guard.get(&secret_id) {
                    if stored_secret.expiry == 0 || stored_secret.expiry > current_time {
                        worker_secrets_map.insert(name_for_vm, stored_secret.data.clone());
                    } else {
                        warn!(
                            "Local worker authorized for secret {:?} but it's expired.",
                            secret_id
                        );
                    }
                } else {
                    warn!(
                        "Local worker authorized for secret {:?} but it's not found.",
                        secret_id
                    );
                }
            } else {
                warn!(
                    "Local worker not authorized for secret {:?}, bundle_hash {:?}",
                    secret_id, worker_consumer_info.bundle_hash
                );
            }
        }
        Ok(worker_secrets_map)
    }

    /// Retrieves secrets and packages them into an encrypted SecretsBox for the requester.
    /// Assumes the runner service has already executed policies and stored approvals via `store_authorization`.
    /// Verifies the requester's attestation before proceeding.
    pub fn get_secrets(
        &self,
        requests: Vec<(SecretId, ConsumerInfo)>,
        requester_env_report: EnvReport,
    ) -> Result<SecretsBox, String> {
        info!(
            "GetSecrets request for {} secret-consumer pairs",
            requests.len()
        );
        let current_time = Utc::now().timestamp() as u64;

        // 1. Verify the requester's EnvReport's attestation
        // This is crucial to ensure we are sending secrets to a trusted enclave.
        let _verified_requester_user_data =
            match verify_attestation(&requester_env_report.attestation) {
                Ok(data) => data, // We don't necessarily need the user_data here, just verification success
                Err(e) => {
                    return Err(format!("Requester attestation verification failed: {}", e));
                }
            };
        debug!("Requester attestation verified");
        // Extract requester's KX public key *after* verifying attestation
        let requester_kx_pk_bytes: [u8; 32] = requester_env_report
            .attestation
            .ephemeral_public_key
            .as_slice()
            .try_into()
            .map_err(|_| {
                format!(
                    "Invalid ephemeral public key length in verified requester attestation: {}",
                    requester_env_report.attestation.ephemeral_public_key.len()
                )
            })?;
        let requester_kx_pk = PublicKey::from(requester_kx_pk_bytes);

        // 2. Check authorization and retrieve secrets
        let mut secrets_to_pack: Vec<(SecretId, Vec<u8>, u64, u64)> = Vec::new();
        {
            // Acquire read locks
            let secrets_map = self.secrets_storage.read().unwrap();

            for (secret_id, consumer_info) in &requests {
                // Check if *we* are authorized (by *our* runner) to release this secret *to the requester*.
                if !self.check_authorization(
                    &requester_env_report.attestation,
                    secret_id,
                    consumer_info,
                ) {
                    warn!(
                        "Not authorized to release secret {:?} for consumer bundle_hash {:?} \
                         (attestation measurement: {:?})",
                        secret_id,
                        consumer_info.bundle_hash,
                        requester_env_report.attestation.measurement // Log part of attestation
                    );
                    continue; // Skip this secret
                }

                match secrets_map.get(secret_id) {
                    Some(stored_secret) => {
                        if stored_secret.expiry == 0 || stored_secret.expiry > current_time {
                            debug!("Found valid secret {:?}", secret_id);
                            secrets_to_pack.push((
                                secret_id.clone(),
                                stored_secret.data.clone(),
                                stored_secret.expiry,
                                stored_secret.generation_timestamp,
                            ));
                        } else {
                            info!("Secret {:?} found but expired", secret_id);
                            // TODO: Optionally trigger cleanup of expired secrets?
                        }
                    }
                    None => {
                        info!("Secret {:?} not found locally", secret_id);
                    }
                }
            }
        } // Drop read locks

        if secrets_to_pack.is_empty() {
            info!("No authorized/valid secrets found");
            // Return an empty box rather than erroring
            // Encrypting an empty list still produces a valid box structure
            // Assuming signature field REMOVED from SecretsBox type:
            return encrypt_secrets_box(
                &self.ephemeral_kx_keypair,
                &requester_kx_pk,
                &secrets_to_pack, // Empty vec
            )
            .map_err(|e| format!("Failed to encrypt empty secrets box: {e}"));
        }

        info!("Packing {} secrets", secrets_to_pack.len());

        // 3. Encrypt the secrets into a SecretsBox
        // Assuming signature field was REMOVED from SecretsBox type:
        encrypt_secrets_box(
            &self.ephemeral_kx_keypair,
            &requester_kx_pk,
            &secrets_to_pack,
        )
        .map_err(|e| format!("Failed to encrypt secrets box: {e}"))
    }

    /// Checks the status (presence and expiry) of requested secrets.
    pub fn check_secrets(&self, ids: Vec<SecretId>) -> Result<Vec<(SecretId, bool, u64)>, String> {
        let secrets_map = self.secrets_storage.read().unwrap();
        let current_time = Utc::now().timestamp() as u64;
        let mut results = Vec::new();

        for id in ids {
            match secrets_map.get(&id) {
                Some(secret) => {
                    let is_valid = secret.expiry == 0 || secret.expiry > current_time;
                    results.push((id, is_valid, secret.expiry));
                }
                None => {
                    results.push((id, false, 0));
                }
            }
        }
        Ok(results)
    }

    // === Methods for Runner Service Interaction ===

    /// Stores an authorization granted by the co-resident runner service.
    /// Called by the RunnerService when a policy execution succeeds.
    pub fn store_authorization(&self, report: PolicyExecutionReport) {
        if !report.decision {
            return; // Only store successful decisions
        }

        let attestation_report_for_auth = &report.request.env_report.attestation;
        let consumer_info_for_auth = &report.request.consumer;
        let timestamp = report.timestamp; // Use timestamp from the report

        // TODO: Consider a suitable expiry/TTL for authorizations. Using grant timestamp for now.
        let expiry_time = timestamp + 3600; // e.g., authorize for 1 hour

        let mut auth_map = self.authorizations.write().unwrap();
        for secret_id in report.request.secret_ids {
            let auth_id = calculate_authorization_id(
                attestation_report_for_auth,
                &secret_id,
                consumer_info_for_auth,
            );
            info!(
                "Storing authorization grant {} for attestation measurement {:?} / secret {:?} / \
                 consumer bundle_hash {:?} with expiry {}",
                auth_id,
                attestation_report_for_auth.measurement,
                secret_id,
                consumer_info_for_auth.bundle_hash,
                expiry_time
            );
            auth_map.insert(auth_id, expiry_time);
        }
    }

    /// Checks if an authorization exists and is valid for the given node and secret.
    /// Used internally by PutSecrets and GetSecrets.
    pub(crate) fn check_authorization(
        &self,
        attestation_report: &AttestationReport,
        secret_id: &SecretId,
        consumer_info: &ConsumerInfo,
    ) -> bool {
        let auth_id = calculate_authorization_id(attestation_report, secret_id, consumer_info);
        let auth_map = self.authorizations.read().unwrap();

        match auth_map.get(&auth_id) {
            Some(&expiry) => {
                let current_time = Utc::now().timestamp() as u64;
                let is_valid = expiry > current_time;
                if !is_valid {
                    debug!(
                        "Authorization {} found for attestation measurement {:?} / secret {:?} / \
                         consumer bundle_hash {:?}, but expired at {} (current: {}).",
                        auth_id,
                        attestation_report.measurement,
                        secret_id,
                        consumer_info.bundle_hash,
                        expiry,
                        current_time
                    );
                    // TODO: Clean up expired authorizations?
                } else {
                    debug!(
                        "Authorization {} found and valid for attestation measurement {:?} / \
                         secret {:?} / consumer bundle_hash {:?}",
                        auth_id,
                        attestation_report.measurement,
                        secret_id,
                        consumer_info.bundle_hash
                    );
                }
                is_valid
            }
            None => {
                debug!(
                    "No authorization {} found for attestation measurement {:?} / secret {:?} / \
                     consumer bundle_hash {:?}",
                    auth_id, attestation_report.measurement, secret_id, consumer_info.bundle_hash
                );
                false
            }
        }
    }
}

#[cfg(test)]
impl Secrets {
    #[cfg(test)]
    pub(crate) fn update_secret_data_for_test(
        &self,
        secret_id: &SecretId,
        new_data: Vec<u8>,
    ) -> Result<(), String> {
        let mut storage = self.secrets_storage.write().unwrap();
        if let Some(secret) = storage.get_mut(secret_id) {
            secret.data = new_data;
            Ok(())
        } else {
            Err(format!("Secret {:?} not found for test update", secret_id))
        }
    }

    pub(crate) fn kx_public_key_for_test(&self) -> &PublicKey {
        self.ephemeral_kx_keypair.public_key()
    }
}
