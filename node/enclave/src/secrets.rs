use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::{Arc, RwLock},
};

use chrono::Utc;
use nxcc_interface::types::{
    AttestationReport, EnvReport, PolicyExecutionReport, SecretId, SecretsBox,
};
use once_cell::sync::Lazy;
use sha2::Digest as _;
use tracing::{debug, error, info, warn};
use x25519_dalek::PublicKey;

use crate::crypto::{
    KeyExchangeKeyPair, decrypt_secrets_box, encrypt_secrets_box, generate_attestation,
};

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

/// Placeholder for actual TEE attestation verification.
/// In a real implementation, this would use the TEE SDK (e.g., Intel SGX SDK, AWS Nitro Enclaves SDK)
/// to verify the quote/report against known CAs/roots and platform state.
/// It should return the verified user_data (report_data in Nitro) if successful.
fn verify_attestation(report: &AttestationReport) -> Result<Vec<u8>, String> {
    // --- Placeholder Implementation ---
    // WARNING: This is NOT secure and bypasses actual verification.
    // In a real TEE:
    // 1. Parse the report structure specific to the TEE type.
    // 2. Verify the report's signature using the TEE platform's keys/CAs.
    // 3. Check measurements (PCRs/MRENCLAVE) against expected values.
    // 4. Check security properties (e.g., debug status).
    // 5. If all checks pass, extract and return the user_data/report_data.
    debug!(
        "Placeholder: Attestation verification skipped for report with key: {}",
        hex::encode(&report.ephemeral_public_key)
    );
    // For testing/dev, we just return the user_data assuming it's valid.
    if report.ephemeral_public_key.len() != 32 {
        return Err("Invalid ephemeral public key length in attestation".to_string());
    }
    Ok(report.user_data.clone())
    // --- End Placeholder ---
}

/// The core state and logic for managing secrets within the enclave.
pub struct Secrets {
    /// Ephemeral keypair for Diffie-Hellman, generated once per enclave instance.
    ephemeral_kx_keypair: Lazy<KeyExchangeKeyPair>,
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
            secrets_storage: RwLock::new(HashMap::new()),
            authorizations: RwLock::new(HashMap::new()),
        })
    }

    /// Returns an attestation report binding the ephemeral public key and user data.
    /// The ephemeral key is generated lazily on first call and reused subsequently.
    /// The caller is responsible for putting the correct hash into user_data before calling this.
    pub fn get_report(&self, user_data: Vec<u8>) -> Result<AttestationReport, String> {
        let public_key = self.ephemeral_kx_keypair.public_key();
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
    pub fn put_secrets(&self, bundles: Vec<(SecretsBox, EnvReport)>) -> Result<bool, String> {
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

            // 1. Verify the Attestation Report from the EnvReport
            let verified_user_data = match verify_attestation(&env_report.attestation) {
                Ok(data) => data,
                Err(e) => {
                    warn!(
                        "Skipping bundle from node {}: Attestation verification failed: {}",
                        env_report.node_id, e
                    );
                    continue;
                }
            };
            debug!("Attestation verified for node {}", env_report.node_id);

            // 2. Verify SecretsBox Binding using the hash from user_data
            let expected_hash_slice = verified_user_data.as_slice();
            if expected_hash_slice.len() != 32 {
                warn!(
                    "Skipping bundle from node {}: Invalid hash length ({}) in verified \
                     attestation user_data",
                    env_report.node_id,
                    expected_hash_slice.len()
                );
                continue;
            }
            let expected_hash: [u8; 32] = expected_hash_slice.try_into().unwrap(); // Safe due to length check
            let calculated_hash = secrets_box.calculate_binding_hash();

            if expected_hash != calculated_hash {
                warn!(
                    "Skipping bundle from node {}: SecretsBox hash mismatch. Expected {}, \
                     calculated {}",
                    env_report.node_id,
                    hex::encode(expected_hash),
                    hex::encode(calculated_hash)
                );
                continue;
            }
            debug!(
                "SecretsBox binding verified for node {}",
                env_report.node_id
            );

            // 3. Check authorization for *each* secret contained in the box
            // This remains the same: checks if *our* runner approved the *sender*
            let mut all_secrets_authorized = true;
            for secret_id in &secrets_box.contained_secret_ids {
                if !self.check_authorization(&env_report.node_id, secret_id) {
                    warn!(
                        "Skipping bundle from node {}: Not authorized locally to receive secret \
                         {:?} from this node",
                        env_report.node_id, secret_id
                    );
                    all_secrets_authorized = false;
                    break;
                }
            }

            if !all_secrets_authorized {
                continue;
            }
            debug!(
                "Local authorization check passed for all secrets in the box from node {}",
                env_report.node_id
            );

            // 4. Decrypt the SecretsBox (no signature verification needed now)
            // REMOVED HACK: No need to synthesize sender_sig_pk
            // let mut hasher = sha2::Sha256::new(); ... // REMOVED HACK BLOCK

            let decrypted_secrets = match decrypt_secrets_box(
                &self.ephemeral_kx_keypair, // Our KX keypair
                // &sender_sig_pk, // REMOVED
                &secrets_box,
            ) {
                Ok(s) => s,
                Err(e) => {
                    // Decryption failure might indicate wrong recipient key or corrupted data
                    error!(
                        "Failed to decrypt secrets box from node {} (post-attestation): {}",
                        env_report.node_id, e
                    );
                    continue; // Skip this bundle
                }
            };
            debug!(
                "Successfully decrypted secrets box from node {}",
                env_report.node_id
            );

            // 5. Store the decrypted secrets
            for (secret_id, data, expiry) in decrypted_secrets {
                if expiry != 0 && expiry <= current_time {
                    info!(
                        "Ignoring expired secret {:?} from node {}",
                        secret_id, env_report.node_id
                    );
                    continue;
                }
                let stored_secret = StoredSecret { data, expiry };
                info!("Storing secret {:?} with expiry {}", secret_id, expiry);
                secrets_map.insert(secret_id, stored_secret);
                secrets_added_count += 1;
            }
        }

        // Drop the write lock
        drop(secrets_map);

        Ok(secrets_added_count > 0)
    }

    /// Retrieves secrets and packages them into an encrypted SecretsBox for the requester.
    /// Assumes the runner service has already executed policies and stored approvals via `store_authorization`.
    /// Verifies the requester's attestation before proceeding.
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

        // 1. Verify the requester's EnvReport's attestation
        // This is crucial to ensure we are sending secrets to a trusted enclave.
        let _verified_requester_user_data =
            match verify_attestation(&requester_env_report.attestation) {
                Ok(data) => data, // We don't necessarily need the user_data here, just verification success
                Err(e) => {
                    return Err(format!(
                        "Requester attestation verification failed for node {}: {}",
                        requester_env_report.node_id, e
                    ));
                }
            };
        debug!(
            "Requester attestation verified for node {}",
            requester_env_report.node_id
        );
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
            // Encrypting an empty list still produces a valid box structure
            // Assuming signature field REMOVED from SecretsBox type:
            return encrypt_secrets_box(
                &self.ephemeral_kx_keypair,
                &requester_kx_pk,
                &secrets_to_pack, // Empty vec
            )
            .map_err(|e| format!("Failed to encrypt empty secrets box: {e}"));
        }

        info!(
            "Packing {} secrets for node {}",
            secrets_to_pack.len(),
            requester_env_report.node_id
        );

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

        let requester_node_id = report.request.env_report.node_id;
        let timestamp = report.timestamp; // Use timestamp from the report

        // TODO: Consider a suitable expiry/TTL for authorizations. Using grant timestamp for now.
        let expiry_time = timestamp + 3600; // e.g., authorize for 1 hour

        let mut auth_map = self.authorizations.write().unwrap();
        for secret_id in report.request.secret_ids {
            let auth_id = calculate_authorization_id(&requester_node_id, &secret_id);
            info!(
                "Storing authorization grant {} for node {} / secret {:?} with expiry {}",
                auth_id, requester_node_id, secret_id, expiry_time
            );
            auth_map.insert(auth_id, expiry_time);
        }
    }

    /// Checks if an authorization exists and is valid for the given node and secret.
    /// Used internally by PutSecrets and GetSecrets.
    pub(crate) fn check_authorization(&self, node_id: &str, secret_id: &SecretId) -> bool {
        let auth_id = calculate_authorization_id(node_id, secret_id);
        let auth_map = self.authorizations.read().unwrap();

        match auth_map.get(&auth_id) {
            Some(&expiry) => {
                let current_time = Utc::now().timestamp() as u64;
                let is_valid = expiry > current_time;
                if !is_valid {
                    debug!(
                        "Authorization {} found for node {} / secret {:?}, but expired at {} \
                         (current: {}).",
                        auth_id, node_id, secret_id, expiry, current_time
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

#[cfg(test)]
mod tests {
    use alloy_primitives::U256;
    use chrono::Utc;
    use nxcc_interface::types::{ConsumerInfo, PolicyExecutionReport, PolicyExecutionRequest};

    use super::*;

    // Helper function to create a test SecretId
    fn test_secret_id(id: u64) -> SecretId {
        SecretId {
            chain_id: 1,
            identity_address: format!("0x{:040x}", id).parse().unwrap(),
            identity_id: U256::from(id),
        }
    }

    // Helper function to create a test EnvReport with a valid key and specified user_data
    fn test_env_report_with_hash(node_id: &str, user_data_hash: Vec<u8>) -> EnvReport {
        let kx_keypair = KeyExchangeKeyPair::generate();
        EnvReport {
            attestation: AttestationReport {
                ephemeral_public_key: kx_keypair.public_key().as_bytes().to_vec(),
                block_hashes: vec![vec![1, 2, 3]],
                user_data: user_data_hash, // Use the provided hash
            },
            operator_signature: vec![7; 64],
            node_id: node_id.to_string(),
        }
    }

    // Helper function to create a test PolicyExecutionRequest
    fn test_policy_request(node_id: &str, secret_ids: Vec<SecretId>) -> PolicyExecutionRequest {
        PolicyExecutionRequest {
            secret_ids,
            consumer: ConsumerInfo {
                code_hash: vec![1; 32],
                signature: vec![2; 64],
            },
            // Use a dummy hash for the env report inside the policy request itself
            env_report: test_env_report_with_hash(node_id, vec![0u8; 32]),
        }
    }

    // Helper function to create a test PolicyExecutionReport
    fn test_policy_report(
        request: PolicyExecutionRequest,
        decision: bool,
    ) -> PolicyExecutionReport {
        PolicyExecutionReport {
            request,
            decision,
            timestamp: Utc::now().timestamp() as u64,
        }
    }

    #[test]
    fn test_new_secrets_service() {
        let secrets = Secrets::new();

        // Verify initial state
        assert!(secrets.secrets_storage.read().unwrap().is_empty());
        assert!(secrets.authorizations.read().unwrap().is_empty());

        // Verify lazy initialization of keypairs
        let _pk = secrets.ephemeral_kx_keypair.public_key();

        // These should now be initialized
        assert!(Lazy::get(&secrets.ephemeral_kx_keypair).is_some());
    }

    #[test]
    fn test_get_report() {
        let secrets = Secrets::new();
        let user_data = vec![1, 2, 3, 4]; // This would be the hash in practice

        let report = secrets.get_report(user_data.clone()).unwrap();

        // Verify report contents
        assert_eq!(
            report.ephemeral_public_key,
            secrets.ephemeral_kx_keypair.public_key().as_bytes()
        );
        assert_eq!(report.user_data, user_data);
        assert!(!report.block_hashes.is_empty()); // Should have at least one block hash
    }

    #[test]
    fn test_store_and_check_authorization() {
        let secrets = Secrets::new();
        let node_id = "test-node-1";
        let secret_id = test_secret_id(123);

        // Initially, no authorization exists
        assert!(!secrets.check_authorization(node_id, &secret_id));

        // Create and store a policy report with a positive decision
        let request = test_policy_request(node_id, vec![secret_id.clone()]);
        let report = test_policy_report(request, true);
        secrets.store_authorization(report);

        // Now authorization should exist
        assert!(secrets.check_authorization(node_id, &secret_id));

        // Check a different node/secret combination (should not be authorized)
        assert!(!secrets.check_authorization("other-node", &secret_id));
        assert!(!secrets.check_authorization(node_id, &test_secret_id(456)));
    }

    #[test]
    fn test_store_authorization_with_negative_decision() {
        let secrets = Secrets::new();
        let node_id = "test-node-2";
        let secret_id = test_secret_id(234);

        // Create and store a policy report with a negative decision
        let request = test_policy_request(node_id, vec![secret_id.clone()]);
        let report = test_policy_report(request, false);
        secrets.store_authorization(report);

        // Authorization should not exist since decision was negative
        assert!(!secrets.check_authorization(node_id, &secret_id));
    }

    #[test]
    fn test_authorization_expiry() {
        let secrets = Secrets::new();
        let node_id = "test-node-3";
        let secret_id = test_secret_id(345);

        // Create a request
        let request = test_policy_request(node_id, vec![secret_id.clone()]);

        // Create a report with an already-expired timestamp (1 hour ago)
        let past_time = Utc::now().timestamp() as u64 - 3601; // Authorization lasts 1 hour (3600s)
        let mut report = test_policy_report(request, true);
        report.timestamp = past_time; // Grant happened in the past

        // Store the authorization
        secrets.store_authorization(report);

        // Authorization should not be valid because it's expired (expiry = past_time + 3600)
        assert!(!secrets.check_authorization(node_id, &secret_id));

        // Manually check the authorizations map to confirm it was stored but is expired
        let auth_id = calculate_authorization_id(node_id, &secret_id);
        let auth_map = secrets.authorizations.read().unwrap();
        assert!(auth_map.contains_key(&auth_id));
        assert!(*auth_map.get(&auth_id).unwrap() < Utc::now().timestamp() as u64);
    }

    #[test]
    fn test_put_secrets_attestation_binding_success() {
        let secrets = Secrets::new(); // Receiver
        let sender_node_id = "test-sender-node";
        let secret_id = test_secret_id(456);
        let secret_data = vec![10, 20, 30];
        let expiry = Utc::now().timestamp() as u64 + 3600;

        // Authorize the sender node locally (simulates prior policy execution)
        let auth_request = test_policy_request(sender_node_id, vec![secret_id.clone()]);
        let auth_report = test_policy_report(auth_request, true);
        secrets.store_authorization(auth_report);
        assert!(secrets.check_authorization(sender_node_id, &secret_id));

        // --- Simulate Sender Side ---
        let sender_kx = KeyExchangeKeyPair::generate();
        // let sender_sig = SigningKeyPair::generate(); // No longer needed for encryption

        // Create secrets box (unsigned)
        let secrets_to_send = vec![(secret_id.clone(), secret_data.clone(), expiry)];
        let secrets_box = encrypt_secrets_box(
            &sender_kx,
            secrets.ephemeral_kx_keypair.public_key(), // Receiver's public KX key
            // &sender_sig, // REMOVED
            &secrets_to_send,
        )
        .unwrap();

        // Calculate the binding hash
        let binding_hash = secrets_box.calculate_binding_hash();

        // Create EnvReport with the hash in user_data
        let env_report = test_env_report_with_hash(sender_node_id, binding_hash.to_vec());
        // --- End Sender Simulation ---

        // --- Receiver Side ---
        // Put the secrets (uses placeholder verify_attestation)
        let result = secrets.put_secrets(vec![(secrets_box.clone(), env_report)]);

        assert!(result.is_ok(), "put_secrets failed: {:?}", result.err());
        assert!(result.unwrap(), "put_secrets returned false, expected true");

        // Verify the secret was stored
        let secrets_map = secrets.secrets_storage.read().unwrap();
        assert!(secrets_map.contains_key(&secret_id));
        let stored = secrets_map.get(&secret_id).unwrap();
        assert_eq!(stored.data, secret_data);
        assert_eq!(stored.expiry, expiry);
    }

    #[test]
    fn test_put_secrets_attestation_binding_hash_mismatch() {
        let secrets = Secrets::new(); // Receiver
        let sender_node_id = "test-sender-node-mismatch";
        let secret_id = test_secret_id(457);
        let secret_data = vec![11, 21, 31];
        let expiry = Utc::now().timestamp() as u64 + 3600;

        // Authorize the sender node locally
        let auth_request = test_policy_request(sender_node_id, vec![secret_id.clone()]);
        let auth_report = test_policy_report(auth_request, true);
        secrets.store_authorization(auth_report);

        // --- Simulate Sender Side ---
        let sender_kx = KeyExchangeKeyPair::generate();
        let secrets_to_send = vec![(secret_id.clone(), secret_data.clone(), expiry)];
        let secrets_box = encrypt_secrets_box(
            &sender_kx,
            secrets.ephemeral_kx_keypair.public_key(),
            &secrets_to_send,
        )
        .unwrap();

        // Calculate the correct binding hash
        let correct_binding_hash = secrets_box.calculate_binding_hash();
        // Create an *incorrect* hash for the report
        let mut incorrect_hash = correct_binding_hash;
        incorrect_hash[0] ^= 0xff; // Flip some bits

        // Create EnvReport with the *incorrect* hash in user_data
        let env_report = test_env_report_with_hash(sender_node_id, incorrect_hash.to_vec());
        // --- End Sender Simulation ---

        // --- Receiver Side ---
        let result = secrets.put_secrets(vec![(secrets_box.clone(), env_report)]);

        assert!(result.is_ok());
        // Expect false because the hash mismatch should cause the bundle to be skipped
        assert!(!result.unwrap());

        // Verify the secret was NOT stored
        let secrets_map = secrets.secrets_storage.read().unwrap();
        assert!(!secrets_map.contains_key(&secret_id));
    }

    #[test]
    fn test_put_secrets_unauthorized_with_attestation() {
        let secrets = Secrets::new(); // Receiver
        let sender_node_id = "test-sender-node-unauth";
        let secret_id = test_secret_id(567);

        // Do NOT authorize the node locally
        assert!(!secrets.check_authorization(sender_node_id, &secret_id));

        // --- Simulate Sender Side ---
        let sender_kx = KeyExchangeKeyPair::generate();
        let secrets_to_send = vec![(secret_id.clone(), vec![10, 20, 30], 0)];
        let secrets_box = encrypt_secrets_box(
            &sender_kx,
            secrets.ephemeral_kx_keypair.public_key(),
            &secrets_to_send,
        )
        .unwrap();
        let binding_hash = secrets_box.calculate_binding_hash();
        let env_report = test_env_report_with_hash(sender_node_id, binding_hash.to_vec());
        // --- End Sender Simulation ---

        // --- Receiver Side ---
        let result = secrets.put_secrets(vec![(secrets_box, env_report)]);

        assert!(result.is_ok());
        // We expect false because the local authorization check fails
        assert!(!result.unwrap());

        // Verify the secret was NOT stored
        let secrets_map = secrets.secrets_storage.read().unwrap();
        assert!(!secrets_map.contains_key(&secret_id));
    }

    #[test]
    fn test_put_secrets_expired() {
        let secrets = Secrets::new();
        let node_id = "test-node-6";
        let secret_id = test_secret_id(678);
        let secret_data = vec![10, 20, 30];
        let expiry = Utc::now().timestamp() as u64 - 3600; // Expired

        // Authorize the node
        let request = test_policy_request(node_id, vec![secret_id.clone()]);
        let report = test_policy_report(request, true);
        secrets.store_authorization(report);

        // Create secrets box with expired secret
        let sender_kx = KeyExchangeKeyPair::generate();
        let secrets_to_send = vec![(secret_id.clone(), secret_data.clone(), expiry)];
        let secrets_box = encrypt_secrets_box(
            &sender_kx,
            secrets.ephemeral_kx_keypair.public_key(),
            &secrets_to_send,
        )
        .unwrap();
        let binding_hash = secrets_box.calculate_binding_hash();
        let env_report = test_env_report_with_hash(node_id, binding_hash.to_vec());

        let result = secrets.put_secrets(vec![(secrets_box, env_report)]);
        assert!(result.is_ok());
        // Result should be false because the only secret was expired and ignored during storage
        assert!(!result.unwrap());

        // Verify the secret was NOT stored (or was immediately considered invalid)
        let secrets_map = secrets.secrets_storage.read().unwrap();
        assert!(!secrets_map.contains_key(&secret_id)); // Should not be stored if expired on arrival
    }

    #[test]
    fn test_put_secrets_multiple_bundles() {
        let secrets = Secrets::new();
        let node_id1 = "test-node-7a";
        let node_id2 = "test-node-7b";
        let secret_id1 = test_secret_id(789);
        let secret_id2 = test_secret_id(790);
        let secret_id3_unauth = test_secret_id(791); // Unauthorized secret in bundle 1

        // Authorize node1 for secret1, node2 for secret2
        let request1 = test_policy_request(node_id1, vec![secret_id1.clone()]);
        let report1 = test_policy_report(request1, true);
        secrets.store_authorization(report1);

        let request2 = test_policy_request(node_id2, vec![secret_id2.clone()]);
        let report2 = test_policy_report(request2, true);
        secrets.store_authorization(report2);

        let mut bundles = Vec::new();

        // --- Bundle 1 (node1, secret1 - authorized, secret3 - unauthorized) ---
        let sender_kx1 = KeyExchangeKeyPair::generate();
        let secrets_to_send1 = vec![
            (secret_id1.clone(), vec![1, 2, 3], 0),
            (secret_id3_unauth.clone(), vec![9, 9, 9], 0), // Unauthorized
        ];
        let secrets_box1 = encrypt_secrets_box(
            &sender_kx1,
            secrets.ephemeral_kx_keypair.public_key(),
            &secrets_to_send1,
        )
        .unwrap();
        let binding_hash1 = secrets_box1.calculate_binding_hash();
        let env_report1 = test_env_report_with_hash(node_id1, binding_hash1.to_vec());
        bundles.push((secrets_box1, env_report1));

        // --- Bundle 2 (node2, secret2 - authorized) ---
        let sender_kx2 = KeyExchangeKeyPair::generate();
        let secrets_to_send2 = vec![(secret_id2.clone(), vec![4, 5, 6], 0)];
        let secrets_box2 = encrypt_secrets_box(
            &sender_kx2,
            secrets.ephemeral_kx_keypair.public_key(),
            &secrets_to_send2,
        )
        .unwrap();
        let binding_hash2 = secrets_box2.calculate_binding_hash();
        let env_report2 = test_env_report_with_hash(node_id2, binding_hash2.to_vec());
        bundles.push((secrets_box2, env_report2));

        // Put all secrets
        let result = secrets.put_secrets(bundles);

        assert!(result.is_ok());
        // Expect true because secret2 from bundle 2 should be added successfully
        assert!(result.unwrap());

        // Verify storage state
        let secrets_map = secrets.secrets_storage.read().unwrap();
        assert!(!secrets_map.contains_key(&secret_id1)); // Bundle 1 skipped due to unauthorized secret3
        assert!(secrets_map.contains_key(&secret_id2)); // Bundle 2 processed
        assert!(!secrets_map.contains_key(&secret_id3_unauth)); // Bundle 1 skipped

        assert_eq!(secrets_map.get(&secret_id2).unwrap().data, vec![4, 5, 6]);
    }

    #[test]
    fn test_get_secrets_authorization_check() {
        let secrets = Secrets::new(); // Our enclave
        let requester_node_id = "test-requester-node";
        let secret_id1 = test_secret_id(890);
        let secret_id2 = test_secret_id(891);
        let secret_data = vec![10, 20, 30];

        // Store secrets directly
        {
            let mut secrets_map = secrets.secrets_storage.write().unwrap();
            secrets_map.insert(
                secret_id1.clone(),
                StoredSecret {
                    data: secret_data.clone(),
                    expiry: 0,
                },
            );
            secrets_map.insert(
                secret_id2.clone(),
                StoredSecret {
                    data: secret_data.clone(),
                    expiry: 0,
                },
            );
        }

        // Authorize the requester for secret_id1 only
        let auth_request = test_policy_request(requester_node_id, vec![secret_id1.clone()]);
        let auth_report = test_policy_report(auth_request.clone(), true);
        secrets.store_authorization(auth_report.clone());

        // Simulate a valid EnvReport from the requester (placeholder verification will pass)
        let requester_env_report = test_env_report_with_hash(requester_node_id, vec![0u8; 32]); // Hash content doesn't matter for GetSecrets verification logic itself

        // Get both secrets
        let result = secrets.get_secrets(
            vec![secret_id1.clone(), secret_id2.clone()],
            requester_env_report.clone(),
            vec![], // Policy reports unused
        );

        assert!(result.is_ok(), "get_secrets failed: {:?}", result.err());
        let secrets_box = result.unwrap();

        // Verify the secrets box contains only the authorized secret ID
        assert_eq!(secrets_box.contained_secret_ids.len(), 1);
        assert!(secrets_box.contained_secret_ids.contains(&secret_id1));
        assert!(!secrets_box.contained_secret_ids.contains(&secret_id2));
        // assert!(secrets_box.signature.is_empty()); // If signature field removed
    }

    #[test]
    fn test_get_secrets_unauthorized() {
        let secrets = Secrets::new();
        let requester_node_id = "test-node-9";
        let secret_id = test_secret_id(901);
        let secret_data = vec![10, 20, 30];

        // Store a secret directly
        {
            let mut secrets_map = secrets.secrets_storage.write().unwrap();
            secrets_map.insert(
                secret_id.clone(),
                StoredSecret {
                    data: secret_data.clone(),
                    expiry: 0, // No expiry
                },
            );
        }

        // Do NOT authorize the node

        // Simulate a valid EnvReport from the requester
        let requester_env_report = test_env_report_with_hash(requester_node_id, vec![0u8; 32]);

        // Get the secrets
        let result = secrets.get_secrets(
            vec![secret_id.clone()],
            requester_env_report,
            vec![], // Empty policy reports
        );

        assert!(result.is_ok());
        let secrets_box = result.unwrap();

        // Verify the secrets box is empty because authorization failed
        assert!(secrets_box.contained_secret_ids.is_empty());
        // Even with empty secrets, the encrypted payload will still contain encryption metadata
        // So we don't check if it's empty, just that it contains no secret IDs
    }

    #[test]
    fn test_get_secrets_expired() {
        let secrets = Secrets::new();
        let requester_node_id = "test-node-10";
        let secret_id = test_secret_id(1010);
        let secret_data = vec![10, 20, 30];
        let expiry = Utc::now().timestamp() as u64 - 3600; // 1 hour ago (expired)

        // Store an expired secret directly
        {
            let mut secrets_map = secrets.secrets_storage.write().unwrap();
            secrets_map.insert(
                secret_id.clone(),
                StoredSecret {
                    data: secret_data.clone(),
                    expiry,
                },
            );
        }

        // Authorize the node
        let request = test_policy_request(requester_node_id, vec![secret_id.clone()]);
        let report = test_policy_report(request, true);
        secrets.store_authorization(report.clone());

        // Simulate a valid EnvReport from the requester
        let requester_env_report = test_env_report_with_hash(requester_node_id, vec![0u8; 32]);

        // Get the secrets
        let result = secrets.get_secrets(vec![secret_id.clone()], requester_env_report, vec![]);

        assert!(result.is_ok());
        let secrets_box = result.unwrap();

        // Verify the secrets box is empty (expired secret not included)
        assert!(secrets_box.contained_secret_ids.is_empty());
        // Even with empty secrets, the encrypted payload will still contain encryption metadata
        // So we don't check if it's empty, just that it contains no secret IDs
    }

    #[test]
    fn test_check_secrets() {
        let secrets = Secrets::new();
        let secret_id1 = test_secret_id(1201);
        let secret_id2 = test_secret_id(1202);
        let secret_id3 = test_secret_id(1203);
        let secret_id4 = test_secret_id(1204); // Not stored

        let current_time = Utc::now().timestamp() as u64;
        let future_time = current_time + 3600; // 1 hour in the future
        let past_time = current_time - 3600; // 1 hour in the past

        // Store secrets directly
        {
            let mut secrets_map = secrets.secrets_storage.write().unwrap();
            secrets_map.insert(
                secret_id1.clone(),
                StoredSecret {
                    data: vec![1, 2, 3],
                    expiry: 0, // No expiry
                },
            );
            secrets_map.insert(
                secret_id2.clone(),
                StoredSecret {
                    data: vec![4, 5, 6],
                    expiry: future_time, // Valid
                },
            );
            secrets_map.insert(
                secret_id3.clone(),
                StoredSecret {
                    data: vec![7, 8, 9],
                    expiry: past_time, // Expired
                },
            );
        }

        // Check all secrets
        let result = secrets.check_secrets(vec![
            secret_id1.clone(),
            secret_id2.clone(),
            secret_id3.clone(),
            secret_id4.clone(),
        ]);

        assert!(result.is_ok());
        let status = result.unwrap();

        assert_eq!(status.len(), 4);

        // Find each secret in the results
        let status1 = status.iter().find(|s| s.0 == secret_id1).unwrap();
        let status2 = status.iter().find(|s| s.0 == secret_id2).unwrap();
        let status3 = status.iter().find(|s| s.0 == secret_id3).unwrap();
        let status4 = status.iter().find(|s| s.0 == secret_id4).unwrap();

        // Verify status
        assert!(status1.1); // Valid (no expiry)
        assert_eq!(status1.2, 0); // No expiry

        assert!(status2.1); // Valid (future expiry)
        assert_eq!(status2.2, future_time); // Future expiry time

        assert!(!status3.1); // Invalid (expired)
        assert_eq!(status3.2, past_time); // Past expiry time

        assert!(!status4.1); // Not found
        assert_eq!(status4.2, 0); // Zero expiry for not found
    }

    #[test]
    fn test_authorization_id_calculation() {
        let node_id = "test-node";
        let secret_id1 = test_secret_id(1301);
        let secret_id2 = test_secret_id(1302);

        // Same node, different secrets should have different IDs
        let id1 = calculate_authorization_id(node_id, &secret_id1);
        let id2 = calculate_authorization_id(node_id, &secret_id2);
        assert_ne!(id1, id2);

        // Different nodes, same secret should have different IDs
        let id3 = calculate_authorization_id("other-node", &secret_id1);
        assert_ne!(id1, id3);

        // Same inputs should produce same ID (deterministic)
        let id1_repeat = calculate_authorization_id(node_id, &secret_id1);
        assert_eq!(id1, id1_repeat);
    }
}
