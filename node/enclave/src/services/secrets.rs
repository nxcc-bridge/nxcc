use ciborium::{de::from_reader, ser::into_writer};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use crate::crypto::{Aead, EphemeralSecret, Signer};
use interface::types::{AttestationReport, EnvReport, PolicyExecutionReport, SecretId, SecretsBox};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Secret {
    pub id: SecretId,
    pub data: Vec<u8>,
    pub expiry: u64,
}

/// Authorized ephemeral requests.
#[derive(Clone)]
struct AuthEntry {
    report: PolicyExecutionReport,
    created_at: u64,
}

/// Holds secrets and ephemeral policy authorizations.
pub struct Secrets {
    store: Mutex<HashMap<SecretId, Secret>>,
    authz: Mutex<VecDeque<AuthEntry>>,
    signer: Signer,
    x25519_secret: EphemeralSecret,
}

impl Secrets {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            store: Mutex::new(HashMap::new()),
            authz: Mutex::new(VecDeque::new()),
            signer: Signer::new(),
            x25519_secret: EphemeralSecret::new(),
        })
    }

    pub fn get_report(&self, user_data: Vec<u8>) -> AttestationReport {
        // TODO: This should eventually return an EnvReport signed by the operator key
        // For now, just return the AttestationReport part
        AttestationReport {
            ephemeral_public_key: self.x25519_secret.public_key().to_vec(),
            block_hashes: vec![], // TODO: Populate block hashes if needed by policy
            user_data,
        }
    }

    pub fn put_secrets(&self, bundles: Vec<(SecretsBox, EnvReport)>) -> bool {
        for (sb, _env_report) in bundles {
            // TODO: Validate env_report signature and attestation if needed
            let Ok(peer_pub) = sb.sender_public_key.as_slice().try_into() else {
                tracing::warn!("Invalid sender public key in SecretsBox");
                return false;
            };
            let shared = self.x25519_secret.diffie_hellman(&peer_pub);
            let aead = Aead::new(&shared);
            let Some(plaintext) = aead.decrypt(&sb.encrypted_payload) else {
                tracing::warn!("Failed to decrypt SecretsBox payload");
                return false;
            };
            let items: Vec<Secret> = match from_reader(plaintext.as_slice()) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("Failed to deserialize secrets from SecretsBox: {}", e);
                    return false;
                }
            };
            let mut guard = self.store.lock().unwrap();
            for s in items {
                // TODO: Check contained_secret_ids against actual secrets?
                tracing::debug!("Storing secret: {:?}", s.id);
                guard.insert(s.id.clone(), s);
            }
        }
        true
    }

    // Accepts EnvReport now
    pub fn get_secrets(&self, ids: Vec<SecretId>, requester_env_report: EnvReport) -> SecretsBox {
        self.expire_auth();
        let mut needed = ids.clone();
        if !self.check_authorizations(&mut needed, &requester_env_report) {
            tracing::warn!("Authorization check failed for secrets: {:?}", ids);
            return SecretsBox::new_empty();
        }
        // Extract attestation part for crypto
        let requester_att = requester_env_report.attestation;
        let Ok(pk) = requester_att.ephemeral_public_key.as_slice().try_into() else {
            tracing::warn!("Invalid ephemeral public key in requester EnvReport");
            return SecretsBox::new_empty();
        };
        let shared = self.x25519_secret.diffie_hellman(&pk);
        let mut results = Vec::new();
        let mut retrieved_ids = Vec::new(); // Keep track of IDs actually included
        {
            let st = self.store.lock().unwrap();
            let now = current_unix_time();
            for sid in ids {
                // Only include secrets that were originally requested AND found/valid
                if let Some(sec) = st.get(&sid) {
                    if sec.expiry == 0 || sec.expiry > now {
                        results.push(sec.clone());
                        retrieved_ids.push(sid.clone()); // Add to list of included IDs
                    } else {
                        tracing::debug!("Secret {:?} found but expired", sid);
                    }
                } else {
                    tracing::debug!("Secret {:?} not found", sid);
                }
            }
        }
        if results.is_empty() {
            tracing::debug!("No valid secrets found or authorized to return");
            return SecretsBox::new_empty();
        }

        let mut buffer = Vec::new();
        into_writer(&results, &mut buffer).unwrap();
        let aead = Aead::new(&shared);
        let ciphertext = aead.encrypt(&buffer);
        let sig = self.signer.sign(&ciphertext);
        SecretsBox {
            encrypted_payload: ciphertext,
            sender_public_key: self.x25519_secret.public_key().to_vec(),
            signature: sig.to_bytes().to_vec(),
            alg: "X25519+AES256GCM".to_string(),
            contained_secret_ids: retrieved_ids,
        }
    }

    pub fn check_secrets(&self, ids: Vec<SecretId>) -> Vec<(SecretId, bool, u64)> {
        let now = current_unix_time();
        let st = self.store.lock().unwrap();
        ids.into_iter()
            .map(|sid| {
                if let Some(sec) = st.get(&sid) {
                    let valid = sec.expiry == 0 || sec.expiry > now;
                    (sid, valid, sec.expiry)
                } else {
                    (sid, false, 0)
                }
            })
            .collect()
    }

    pub fn store_authorization(&self, report: PolicyExecutionReport) {
        if !report.decision {
            return;
        }
        tracing::debug!(
            "Storing authorization for {:?} from node {}",
            report.request.secret_ids,
            report.request.env_report.node_id
        );
        let entry = AuthEntry {
            report,
            created_at: current_unix_time(),
        };
        let mut guard = self.authz.lock().unwrap();
        guard.push_back(entry);
    }

    fn expire_auth(&self) {
        const MAX_AGE_SECS: u64 = 60;
        let now = current_unix_time();
        let mut guard = self.authz.lock().unwrap();
        let initial_len = guard.len();
        while let Some(front) = guard.front() {
            if now.saturating_sub(front.created_at) > MAX_AGE_SECS {
                guard.pop_front();
            } else {
                break;
            }
        }
        if initial_len > guard.len() {
            tracing::debug!(
                "Expired {} authorization entries",
                initial_len - guard.len()
            );
        }
    }

    /// Each secret ID in `needed_ids` must have a valid ephemeral authorization matching
    /// the requester's ephemeral key (from their EnvReport). Once matched, that ID is removed from the entry
    /// so it cannot be reused. Partial leftover secrets remain in the queue.
    fn check_authorizations(
        &self,
        needed_ids: &mut Vec<SecretId>,
        requester_env_report: &EnvReport, // Expect full EnvReport
    ) -> bool {
        if needed_ids.is_empty() {
            return true;
        }
        // Use ephemeral key from the attestation part of the EnvReport
        let ephemeral_pk = &requester_env_report.attestation.ephemeral_public_key;
        let mut guard = self.authz.lock().unwrap();

        // Drain all existing entries, check matches, then re-insert leftover parts if any.
        let old_entries: Vec<AuthEntry> = guard.drain(..).collect();
        let mut new_entries = Vec::new();
        let mut found_match = false;

        for entry in old_entries.iter().cloned() {
            let rep = &entry.report;
            // Compare the ephemeral key from the stored report's EnvReport's AttestationReport
            if rep.decision
                && rep.request.env_report.attestation.ephemeral_public_key == *ephemeral_pk
            {
                tracing::debug!(
                    "Found potentially matching auth entry for node {} with secrets {:?}",
                    rep.request.env_report.node_id,
                    rep.request.secret_ids
                );
                let mut leftover_ids_in_entry = Vec::new();
                let mut matched_in_this_entry = false;

                for sid_in_entry in &rep.request.secret_ids {
                    if let Some(pos) = needed_ids.iter().position(|needed| needed == sid_in_entry) {
                        tracing::debug!("Matched needed secret: {:?}", needed_ids[pos]);
                        needed_ids.remove(pos); // Remove the matched ID from the needed list
                        matched_in_this_entry = true;
                        if needed_ids.is_empty() {
                            break; // All needed secrets are now authorized
                        }
                    } else {
                        // This secret ID from the auth entry wasn't needed for the current request
                        leftover_ids_in_entry.push(sid_in_entry.clone());
                    }
                }

                // If this entry was used and still has leftover secrets, create a new entry for them
                if matched_in_this_entry && !leftover_ids_in_entry.is_empty() {
                    tracing::debug!(
                        "Re-queuing leftover authorized secrets: {:?}",
                        leftover_ids_in_entry
                    );
                    let mut updated_request = rep.request.clone();
                    updated_request.secret_ids = leftover_ids_in_entry;
                    new_entries.push(AuthEntry {
                        report: PolicyExecutionReport {
                            request: updated_request,
                            decision: true, // It was already approved
                            timestamp: rep.timestamp,
                        },
                        created_at: entry.created_at, // Keep original timestamp
                    });
                } else if !matched_in_this_entry {
                    // If this entry didn't match any needed secrets, put it back unchanged
                    new_entries.push(entry);
                }
                // If needed_ids is empty, we are done searching
                if needed_ids.is_empty() {
                    found_match = true;
                    break;
                }
            } else {
                // This entry didn't match the ephemeral key or wasn't approved, put it back
                new_entries.push(entry);
            }
        }

        // Reinsert any remaining entries (leftovers from matched entries or completely unmatched entries)
        for e in new_entries {
            guard.push_back(e);
        }
        // If we broke early because all secrets were found, reinsert the rest of the old entries
        if found_match {
            for entry in old_entries.into_iter().skip_while(|e| {
                e.report.request.env_report.attestation.ephemeral_public_key == *ephemeral_pk
            }) {
                guard.push_back(entry);
            }
        }

        needed_ids.is_empty() // Return true if all needed IDs were found and removed
    }
}

fn current_unix_time() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(_) => 0,
    }
}
