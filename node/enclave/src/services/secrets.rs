use crate::crypto::{Aead, EphemeralSecret, Signer};
use ed25519_dalek::Signature;
use interface::types::{AttestationReport, SecretId, SecretsBox};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// A secret that is stored in plaintext in the enclave.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Secret {
    pub id: SecretId,
    pub data: Vec<u8>,
    pub expiry: u64,
}

#[derive(Debug)]
pub enum SecretError {
    ReconstructionFailed,
}

/// Reconstructs a single secret from multiple provided shares.
pub fn reconstruct_secret(shares: Vec<Secret>) -> Result<Secret, SecretError> {
    let first = shares.get(0).ok_or(SecretError::ReconstructionFailed)?;
    for share in &shares[1..] {
        if share.data != first.data || share.expiry != first.expiry {
            return Err(SecretError::ReconstructionFailed);
        }
    }
    Ok(first.clone())
}

/// The enclave's secrets manager.
pub struct Secrets {
    store: Mutex<HashMap<SecretId, Secret>>,
    x25519_secret: EphemeralSecret,
    signer: Signer,
}

impl Secrets {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            store: Mutex::new(HashMap::new()),
            x25519_secret: EphemeralSecret::new(),
            signer: Signer::new(),
        })
    }

    pub fn get_report(&self, user_data: Vec<u8>) -> AttestationReport {
        AttestationReport {
            ephemeral_public_key: self.x25519_secret.public_key().to_vec(),
            block_hashes: vec![],
            user_data,
        }
    }

    /// Decrypts and reconstructs secrets from incoming SecretsBoxes.
    pub fn put_secrets(&self, bundles: Vec<(SecretsBox, AttestationReport)>) -> bool {
        let mut partials_map: HashMap<SecretId, Vec<Secret>> = HashMap::new();
        for (secrets_box, _) in bundles {
            let Ok(peer_public) = secrets_box.sender_public_key.as_slice().try_into() else {
                continue;
            };
            let shared_secret = self.x25519_secret.diffie_hellman(&peer_public);
            let aead = Aead::new(&shared_secret);
            let Some(plaintext) = aead.decrypt(&secrets_box.encrypted_payload) else {
                continue;
            };
            let Ok(secrets_list) = ciborium::de::from_reader::<Vec<Secret>, _>(&plaintext[..])
            else {
                continue;
            };
            for secret_part in secrets_list {
                partials_map
                    .entry(secret_part.id.clone())
                    .or_default()
                    .push(secret_part);
            }
        }

        let mut guard = self.store.lock().unwrap();
        for (id, parts) in partials_map {
            if let Ok(reconstructed) = reconstruct_secret(parts) {
                guard.insert(id, reconstructed);
            }
        }
        true
    }

    /// Retrieves secrets by ID, encrypts them into a SecretsBox for the requester.
    pub fn get_secrets(
        &self,
        ids: Vec<SecretId>,
        _policy_reports: Vec<(Vec<u8>, Vec<u8>)>,
        requester_ar: AttestationReport,
    ) -> SecretsBox {
        if ids.is_empty() {
            return SecretsBox::new_empty();
        }
        let Ok(requester_pub) = requester_ar.ephemeral_public_key.as_slice().try_into() else {
            return SecretsBox::new_empty();
        };
        let shared_secret = self.x25519_secret.diffie_hellman(&requester_pub);
        let now = current_unix_time();
        let guard = self.store.lock().unwrap();
        let mut gathered = Vec::new();
        for id in ids {
            if let Some(stored) = guard.get(&id) {
                if stored.expiry == 0 || stored.expiry > now {
                    gathered.push(stored.clone());
                }
            }
        }
        if gathered.is_empty() {
            return SecretsBox::new_empty();
        }
        let mut plaintext = Vec::new();
        if ciborium::ser::into_writer(&gathered, &mut plaintext).is_err() {
            return SecretsBox::new_empty();
        }
        let aead = Aead::new(&shared_secret);
        let ciphertext = aead.encrypt(&plaintext);
        let signature = self.sign_data(&ciphertext);
        SecretsBox {
            encrypted_payload: ciphertext,
            sender_public_key: self.x25519_public_key(),
            signature,
            alg: "X25519+AES256GCM".to_string(),
        }
    }

    /// Checks whether each SecretId is present and not expired.
    pub fn check_secrets(&self, ids: Vec<SecretId>) -> Vec<(SecretId, bool, u64)> {
        let guard = self.store.lock().unwrap();
        let now = current_unix_time();
        ids.into_iter()
            .map(|id| {
                if let Some(stored) = guard.get(&id) {
                    let found = stored.expiry == 0 || stored.expiry > now;
                    (id, found, stored.expiry)
                } else {
                    (id, false, 0)
                }
            })
            .collect()
    }

    fn x25519_public_key(&self) -> Vec<u8> {
        self.x25519_secret.public_key().to_vec()
    }

    fn sign_data(&self, data: &[u8]) -> Vec<u8> {
        let sig: Signature = self.signer.sign(data);
        sig.to_bytes().to_vec()
    }
}

/// Returns the current UNIX time in seconds.
fn current_unix_time() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(_) => 0,
    }
}
