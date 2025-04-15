use crate::crypto::{
    KeyExchangeKeyPair, SigningKeyPair, decrypt_secrets_box, encrypt_secrets_box,
    generate_attestation,
};
use chrono::Utc;
use ed25519_dalek::VerifyingKey;
use interface::types::{AttestationReport, EnvReport, PolicyExecutionReport, SecretId, SecretsBox};
use once_cell::sync::Lazy;
use sha2::Digest as _;
use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::{Arc, RwLock},
};
use tracing::{debug, error, info, warn};
use x25519_dalek::PublicKey;

/// Represents a secret stored in the enclave's memory.
#[derive(Clone, Debug)]
struct StoredSecret {
    data: Vec<u8>,
    expiry: u64, // Unix timestamp seconds, 0 means no expiry
}

/// Unique identifier for an authorization grant.
type AuthorizationId = u64;

/// Creates a unique ID for an authorization request based on relevant details.
fn calculate_authorization_id(requester_node_id: &str, secret_id: &SecretId) -> AuthorizationId {
    let mut hasher = DefaultHasher::new();
    requester_node_id.hash(&mut hasher);
    secret_id.hash(&mut hasher);
    hasher.finish()
}

/// The core state and logic for managing secrets within the enclave.
pub struct Secrets {
    /// Ephemeral keypair for Diffie-Hellman, generated once per enclave instance.
    ephemeral_kx_keypair: Lazy<KeyExchangeKeyPair>,
    /// TODO: Persistent signing keypair for the enclave identity (needs secure storage/provisioning).
    /// Using an ephemeral one for now.
    signing_keypair: Lazy<SigningKeyPair>,
    /// In-memory storage for decrypted secrets.
    secrets_storage: RwLock<HashMap<SecretId, StoredSecret>>,
    /// Stores granted authorizations based on runner policy execution reports.
    /// Key: AuthorizationId (hash of request details), Value: Expiry timestamp (or timestamp of grant).
    authorizations: RwLock<HashMap<AuthorizationId, u64>>,
}

impl Secrets {
    /// Creates a new Secrets service instance.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            ephemeral_kx_keypair: Lazy::new(KeyExchangeKeyPair::generate),
            signing_keypair: Lazy::new(SigningKeyPair::generate),
            secrets_storage: RwLock::new(HashMap::new()),
            authorizations: RwLock::new(HashMap::new()),
        })
    }

    /// Returns an attestation report binding the ephemeral public key and user data.
    /// The ephemeral key is generated lazily on first call and reused subsequently.
    pub fn get_report(&self, user_data: Vec<u8>) -> Result<AttestationReport, String> {
        let public_key = self.ephemeral_kx_keypair.public_key();
        // In a real TEE, this would involve hardware interaction.
        let report = generate_attestation(public_key, user_data);
        debug!(
            "Generated attestation report with ephemeral PK: {}",
            hex::encode(report.ephemeral_public_key.as_slice())
        );
        Ok(report)
    }

    /// Stores secrets received from peers after verifying authorization.
    /// Assumes the runner service has already executed policies and stored approvals via `store_authorization`.
    pub fn put_secrets(
        &self,
        bundles: Vec<(SecretsBox, EnvReport)>,
        // TODO: Add threshold parameter if needed
    ) -> Result<bool, String> {
        let mut secrets_added_count = 0;
        let current_time = Utc::now().timestamp() as u64;

        // Acquire write lock once for efficiency
        let mut secrets_map = self.secrets_storage.write().unwrap();

        for (secrets_box, env_report) in bundles {
            debug!(
                "Processing secrets box from node {} containing {} secrets",
                env_report.node_id,
                secrets_box.contained_secret_ids.len()
            );

            // 1. Basic validation of the EnvReport's attestation (placeholder)
            // TODO: Perform actual attestation verification using a TEE SDK/library
            if env_report.attestation.ephemeral_public_key.len() != 32 {
                warn!(
                    "Skipping bundle from node {}: Invalid ephemeral public key length in \
                     attestation",
                    env_report.node_id
                );
                continue;
            }
            // TODO: Verify operator signature if needed

            // 2. Check authorization for *each* secret contained in the box
            // The authorization check verifies if *our* runner approved the *sender* (env_report.node_id)
            // to provide these specific secrets.
            let mut all_secrets_authorized = true;
            for secret_id in &secrets_box.contained_secret_ids {
                if !self.check_authorization(&env_report.node_id, secret_id) {
                    warn!(
                        "Skipping bundle from node {}: Not authorized to receive secret {:?} from \
                         this node",
                        env_report.node_id, secret_id
                    );
                    all_secrets_authorized = false;
                    break; // No need to check further secrets in this box
                }
            }

            if !all_secrets_authorized {
                continue; // Move to the next bundle
            }
            debug!(
                "Authorization check passed for all secrets in the box from node {}",
                env_report.node_id
            );

            // 3. Decrypt the SecretsBox
            // We need the sender's *signing* public key to verify the box signature.
            // This should ideally be part of the EnvReport or obtained via a trusted channel.
            // For now, let's assume it can be derived or is implicitly trusted via attestation.
            // We will *synthesize* a VerifyingKey from a hash of the Node ID as a HACK.
            // FIXME: Replace this HACK with proper public key management.
            let mut hasher = sha2::Sha256::new();
            hasher.update(env_report.node_id.as_bytes());
            let sender_sig_pk_hash = hasher.finalize();
            let sender_sig_pk =
                VerifyingKey::from_bytes(sender_sig_pk_hash.as_slice().try_into().unwrap())
                    .map_err(|e| format!("Failed to create placeholder verifying key: {e}"))?;

            let decrypted_secrets = match decrypt_secrets_box(
                &self.ephemeral_kx_keypair, // Our KX keypair
                &sender_sig_pk,             // Sender's SIG public key (HACK)
                &secrets_box,
            ) {
                Ok(s) => s,
                Err(e) => {
                    error!(
                        "Failed to decrypt secrets box from node {}: {}",
                        env_report.node_id, e
                    );
                    continue; // Skip this bundle
                }
            };
            debug!(
                "Successfully decrypted secrets box from node {}",
                env_report.node_id
            );

            // 4. Store the decrypted secrets
            for (secret_id, data, expiry) in decrypted_secrets {
                if expiry != 0 && expiry <= current_time {
                    info!(
                        "Ignoring expired secret {:?} from node {}",
                        secret_id, env_report.node_id
                    );
                    continue;
                }
                // TODO: Implement secret merging/sharing logic if needed. For now, overwrite.
                let stored_secret = StoredSecret { data, expiry };
                info!("Storing secret {:?} with expiry {}", secret_id, expiry);
                secrets_map.insert(secret_id, stored_secret);
                secrets_added_count += 1;
            }
        }

        // Drop the write lock
        drop(secrets_map);

        // TODO: Implement actual threshold logic if needed. For now, succeed if any were added.
        Ok(secrets_added_count > 0)
    }

    /// Retrieves secrets and packages them into an encrypted SecretsBox for the requester.
    /// Assumes the runner service has already executed policies and stored approvals via `store_authorization`.
    pub fn get_secrets(
        &self,
        secret_ids: Vec<SecretId>,
        requester_env_report: EnvReport,
        // policy_reports are currently unused per instructions, checking local auth store instead
        _policy_reports: Vec<PolicyExecutionReport>,
    ) -> Result<SecretsBox, String> {
        info!(
            "GetSecrets request from node {} for {} secrets",
            requester_env_report.node_id,
            secret_ids.len()
        );
        let current_time = Utc::now().timestamp() as u64;

        // 1. Basic validation of the requester's EnvReport (placeholder)
        // TODO: Perform actual attestation verification
        let requester_kx_pk_bytes: [u8; 32] = requester_env_report
            .attestation
            .ephemeral_public_key
            .as_slice()
            .try_into()
            .map_err(|_| {
                format!(
                    "Invalid ephemeral public key length in requester attestation: {}",
                    requester_env_report.attestation.ephemeral_public_key.len()
                )
            })?;
        let requester_kx_pk = PublicKey::from(requester_kx_pk_bytes);
        // TODO: Verify operator signature if needed

        // 2. Check authorization and retrieve secrets
        let mut secrets_to_pack: Vec<(SecretId, Vec<u8>, u64)> = Vec::new();
        {
            // Acquire read locks
            let secrets_map = self.secrets_storage.read().unwrap();

            for secret_id in &secret_ids {
                // Check if *we* are authorized (by *our* runner) to release this secret *to the requester*.
                if !self.check_authorization(&requester_env_report.node_id, secret_id) {
                    warn!(
                        "Not authorized to release secret {:?} to node {}",
                        secret_id, requester_env_report.node_id
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
            info!(
                "No authorized/valid secrets found for node {}",
                requester_env_report.node_id
            );
            // Return an empty box rather than erroring
            return Ok(SecretsBox {
                encrypted_payload: Vec::new(),
                sender_public_key: self.ephemeral_kx_keypair.public_key().as_bytes().to_vec(),
                signature: Vec::new(), // No payload, no signature needed? Or sign empty? Let's sign empty.
                alg: "X25519_AES-GCM-SIV_Ed25519".to_string(),
                contained_secret_ids: Vec::new(),
            });
        }

        info!(
            "Packing {} secrets for node {}",
            secrets_to_pack.len(),
            requester_env_report.node_id
        );

        // 3. Encrypt the secrets into a SecretsBox
        encrypt_secrets_box(
            &self.ephemeral_kx_keypair,
            &requester_kx_pk,
            &self.signing_keypair,
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

        let requester_node_id = report.request.env_report.node_id;
        let timestamp = report.timestamp; // Use timestamp from the report

        // TODO: Consider a suitable expiry/TTL for authorizations. Using grant timestamp for now.
        let expiry_time = timestamp + 3600; // e.g., authorize for 1 hour

        let mut auth_map = self.authorizations.write().unwrap();
        for secret_id in report.request.secret_ids {
            let auth_id = calculate_authorization_id(&requester_node_id, &secret_id);
            info!(
                "Storing authorization grant {} for node {} / secret {:?}",
                auth_id, requester_node_id, secret_id
            );
            auth_map.insert(auth_id, expiry_time);
        }
    }

    /// Checks if an authorization exists and is valid for the given node and secret.
    /// Used internally by PutSecrets and GetSecrets.
    fn check_authorization(&self, node_id: &str, secret_id: &SecretId) -> bool {
        let auth_id = calculate_authorization_id(node_id, secret_id);
        let auth_map = self.authorizations.read().unwrap();

        match auth_map.get(&auth_id) {
            Some(&expiry) => {
                let current_time = Utc::now().timestamp() as u64;
                let is_valid = expiry > current_time;
                if !is_valid {
                    debug!(
                        "Authorization {} found for node {} / secret {:?}, but expired.",
                        auth_id, node_id, secret_id
                    );
                    // TODO: Clean up expired authorizations?
                } else {
                    debug!(
                        "Authorization {} found and valid for node {} / secret {:?}",
                        auth_id, node_id, secret_id
                    );
                }
                is_valid
            }
            None => {
                debug!(
                    "No authorization {} found for node {} / secret {:?}",
                    auth_id, node_id, secret_id
                );
                false
            }
        }
    }
}
