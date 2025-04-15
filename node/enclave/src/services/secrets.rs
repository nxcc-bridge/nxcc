use ciborium::{de::from_reader, ser::into_writer};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use crate::crypto::{Aead, EphemeralSecret, Signer};
use interface::types::{AttestationReport, PolicyExecutionReport, SecretId, SecretsBox};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Secret {
    pub id: SecretId,
    pub data: Vec<u8>,
    pub expiry: u64,
}

/// Authorized ephemeral requests.
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
        AttestationReport {
            ephemeral_public_key: self.x25519_secret.public_key().to_vec(),
            block_hashes: vec![],
            user_data,
        }
    }

    pub fn put_secrets(&self, bundles: Vec<(SecretsBox, AttestationReport)>) -> bool {
        for (sb, _att) in bundles {
            let Ok(peer_pub) = sb.sender_public_key.as_slice().try_into() else {
                return false;
            };
            let shared = self.x25519_secret.diffie_hellman(&peer_pub);
            let aead = Aead::new(&shared);
            let Some(plaintext) = aead.decrypt(&sb.encrypted_payload) else {
                return false;
            };
            let items: Vec<Secret> = match from_reader(plaintext.as_slice()) {
                Ok(v) => v,
                Err(_) => return false,
            };
            let mut guard = self.store.lock().unwrap();
            for s in items {
                guard.insert(s.id.clone(), s);
            }
        }
        true
    }

    pub fn get_secrets(
        &self,
        ids: Vec<SecretId>,
        _unused: Vec<()>,
        requester_att: AttestationReport,
    ) -> SecretsBox {
        self.expire_auth();
        let mut needed = ids.clone();
        if !self.check_authorizations(&mut needed, &requester_att) {
            return SecretsBox::new_empty();
        }
        let Ok(pk) = requester_att.ephemeral_public_key.as_slice().try_into() else {
            return SecretsBox::new_empty();
        };
        let shared = self.x25519_secret.diffie_hellman(&pk);
        let mut results = Vec::new();
        {
            let st = self.store.lock().unwrap();
            let now = current_unix_time();
            for sid in ids {
                if let Some(sec) = st.get(&sid) {
                    if sec.expiry == 0 || sec.expiry > now {
                        results.push(sec.clone());
                    }
                }
            }
        }
        if results.is_empty() {
            return SecretsBox::new_empty();
        }

        // Serialize with ciborium
        let mut buffer = Vec::new();
        if let Err(e) = into_writer(&results, &mut buffer) {
            return SecretsBox::new_empty();
        }
        let aead = Aead::new(&shared);
        let ciphertext = aead.encrypt(&buffer);
        let sig = self.signer.sign(&ciphertext);
        SecretsBox {
            encrypted_payload: ciphertext,
            sender_public_key: self.x25519_secret.public_key().to_vec(),
            signature: sig.to_bytes().to_vec(),
            alg: "X25519+AES256GCM".to_string(),
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
        while let Some(front) = guard.front() {
            if now.saturating_sub(front.created_at) > MAX_AGE_SECS {
                guard.pop_front();
            } else {
                break;
            }
        }
    }

    /// Each secret ID in `needed_ids` must have a valid ephemeral authorization matching
    /// the requester's ephemeral key. Once matched, that ID is removed from the entry
    /// so it cannot be reused. Partial leftover secrets remain in the queue.
    fn check_authorizations(
        &self,
        needed_ids: &mut Vec<SecretId>,
        att: &AttestationReport,
    ) -> bool {
        if needed_ids.is_empty() {
            return true;
        }
        let ephemeral_pk = &att.ephemeral_public_key;
        let mut guard = self.authz.lock().unwrap();

        // Drain all existing entries, check matches, then re-insert leftover parts if any.
        let old_entries: Vec<AuthEntry> = guard.drain(..).collect();
        let mut new_entries = Vec::new();

        for entry in old_entries {
            let rep = &entry.report;
            if rep.decision
                && rep.request.env_report.attestation.ephemeral_public_key == *ephemeral_pk
            {
                let mut leftover = Vec::new();
                for sid in &rep.request.secret_ids {
                    if let Some(pos) = needed_ids.iter().position(|x| x == sid) {
                        needed_ids.remove(pos);
                        if needed_ids.is_empty() {
                            // Re-inject leftover secrets if any remain in this entry
                            leftover.clear();
                            break;
                        }
                    } else {
                        leftover.push(sid.clone());
                    }
                }
                if !leftover.is_empty() {
                    let mut copy = rep.request.clone();
                    copy.secret_ids = leftover;
                    new_entries.push(AuthEntry {
                        report: PolicyExecutionReport {
                            request: copy,
                            decision: true,
                            timestamp: rep.timestamp,
                        },
                        created_at: entry.created_at,
                    });
                }
                // If needed_ids is empty, we can reinsert everything else and return success
                if needed_ids.is_empty() {
                    // Reinsert leftover entries
                    for e in new_entries {
                        guard.push_back(e);
                    }
                    return true;
                }
            } else {
                new_entries.push(entry);
            }
        }
        for e in new_entries {
            guard.push_back(e);
        }
        false
    }
}

fn current_unix_time() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(_) => 0,
    }
}
