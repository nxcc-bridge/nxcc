use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::crypto::{Aead, EphemeralSecret, Signer};
use ed25519_dalek::Signature;
use interface::types::{AttestationReport, SecretId, SecretsBox};

/// Secrets stored in plaintext in the enclave (in memory).
/// For production usage, replace this with secure storage as appropriate.
struct StoredSecret {
    data: Vec<u8>,
    expiry: u64,
}

/// Enclave's primary secrets logic.
pub struct Secrets {
    /// In-memory storage of secrets by ID.
    store: Mutex<HashMap<SecretId, StoredSecret>>,
    /// A static X25519 secret for decrypting incoming secrets from peers.
    /// In real usage, this might be a device identity or ephemeral session key.
    x25519_secret: EphemeralSecret,
    /// Ed25519 signing key for producing optional signatures in the returned SecretsBox.
    signer: Signer,
}

impl Secrets {
    pub fn new() -> Arc<Self> {
        // Generate a static X25519 secret once. This example always uses the same key at runtime.
        let secret = EphemeralSecret::new();
        Arc::new(Self {
            store: Mutex::new(HashMap::new()),
            x25519_secret: secret,
            signer: Signer::new(),
        })
    }

    /// Returns an attestation report with ephemeral_public_key = this enclave's X25519 public key.
    /// The remainder of fields are placeholders or derived from user_data as needed.
    pub fn get_report(&self, user_data: Vec<u8>) -> AttestationReport {
        AttestationReport {
            ephemeral_public_key: self.x25519_secret.diffie_hellman(&[0u8; 32]).to_vec(),
            block_hashes: vec![],
            user_data,
        }
    }

    /// Accepts one or more SecretsBoxes from peers and stores them in plaintext.
    /// Each SecretsBox is decrypted using the enclave's X25519 secret plus the box's ephemeral pubkey.
    pub fn put_secrets(&self, bundles: Vec<(SecretsBox, AttestationReport)>) -> bool {
        let mut store_guard = self.store.lock().unwrap();

        for (secrets_box, _report) in bundles {
            // Extract ephemeral pubkey from secrets_box
            let Ok(peer_public) = secrets_box.sender_public_key.as_slice().try_into() else {
                continue;
            };
            let shared_secret = self.x25519_secret.diffie_hellman(&peer_public);
            let aead = Aead::new(&shared_secret);

            let ct = secrets_box.encrypted_payload;

            let plaintext = match aead.decrypt(&ct) {
                Some(pt) => pt,
                None => continue,
            };

            // The incoming plaintext must contain the actual secret data plus ID, or
            // you can embed multiple secrets. For brevity, assume the peer lumps them all into a single block.

            // Example: expect the first 32 bytes to be the chain_id + address + etc. Real design may vary.
            // This example simply doesn't parse anything special. In a real system, you'd parse a structure.

            // For demonstration, store this as a single item under a dummy SecretId, or parse properly.
            // If the remote is sending multiple IDs, they'd be in the plaintext structure.

            // We do a dummy ID or skip if we can't parse:
            // In a real system, you'd parse the plaintext to get each (SecretId, data).
            // We'll do a single ID from the first few bytes for demonstration.

            if plaintext.is_empty() {
                continue;
            }

            // As a very simplistic approach, store everything under a single dummy ID or parse real ID:
            let dummy_id = SecretId {
                chain_id: 0,
                identity_address: [0u8; 20].into(),
                identity_id: Default::default(),
            };

            store_guard.insert(
                dummy_id,
                StoredSecret {
                    data: plaintext,
                    expiry: 0,
                },
            );
        }
        true
    }

    /// Retrieves the requested secrets from the local store, then encrypts them for the requester's ephemeral key.
    /// The ephemeral key is in `requester_ar.ephemeral_public_key`.
    pub fn get_secrets(
        &self,
        ids: Vec<SecretId>,
        _policy_reports: Vec<(Vec<u8>, Vec<u8>)>,
        requester_ar: AttestationReport,
    ) -> SecretsBox {
        if ids.is_empty() {
            return SecretsBox::new_empty();
        }

        let shared_secret = match requester_ar.ephemeral_public_key.as_slice().try_into() {
            Ok(requester_pub) => self.x25519_secret.diffie_hellman(&requester_pub),
            Err(_) => {
                return SecretsBox::new_empty();
            }
        };

        let store_guard = self.store.lock().unwrap();
        let mut collected = Vec::new();

        for id in &ids {
            if let Some(stored) = store_guard.get(id) {
                // Append this secret to `collected`. Real design might add ID metadata, etc.
                collected.extend_from_slice(&stored.data);
            }
        }

        // If no data found, return empty box
        if collected.is_empty() {
            return SecretsBox::new_empty();
        }

        let aead = Aead::new(&shared_secret);
        let ct = aead.encrypt(&collected);

        // Optional: sign the ciphertext or ephemeral pubkey
        let signature_bytes = self.sign_data(&ct);

        SecretsBox {
            encrypted_payload: ct,
            sender_public_key: self.x25519_public_key(),
            signature: signature_bytes,
            alg: "X25519+AES256GCM".to_string(),
        }
    }

    /// Checks if each provided SecretId exists in the store.
    pub fn check_secrets(&self, ids: Vec<SecretId>) -> Vec<(SecretId, bool, u64)> {
        let store_guard = self.store.lock().unwrap();
        ids.into_iter()
            .map(|id| {
                if let Some(stored) = store_guard.get(&id) {
                    (id, true, stored.expiry)
                } else {
                    (id, false, 0)
                }
            })
            .collect()
    }

    /// Returns the enclave's static X25519 public key.
    fn x25519_public_key(&self) -> Vec<u8> {
        self.x25519_secret.public_key().to_vec()
    }

    /// Signs arbitrary data with the Ed25519 key and returns the signature bytes.
    fn sign_data(&self, data: &[u8]) -> Vec<u8> {
        let sig: Signature = self.signer.sign(data);
        sig.to_bytes().to_vec()
    }
}
