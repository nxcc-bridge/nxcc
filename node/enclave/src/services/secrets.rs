// File: enclave/src/services/secrets.rs

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tracing::{debug, warn};

use interface::{
    AttestationReport as DomainAttestationReport, Secret as DomainSecret, SecretId,
    SecretsBox as DomainSecretsBox,
};

use crate::crypto::{Aead, Ephemeral, Signer};

/// A simple in-enclave store of secrets, plus ephemeral data for attestation & encryption.
pub struct SecretsEnclave {
    // For demonstration: a map from SecretId to the actual secret data.
    store: RwLock<HashMap<SecretId, DomainSecret>>,

    // This node's ephemeral key used for ECDH in get_report().
    ephemeral_key: RwLock<Option<Ephemeral>>,

    // Long-term signing key for the enclave
    signer: Signer,

    // For demonstration, store block_hashes or other "platform data" we might want in
    // the attestation, along with any user_data from the host side.
    platform_block_hashes: Vec<Vec<u8>>,
}

impl SecretsEnclave {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            store: RwLock::new(HashMap::new()),
            ephemeral_key: RwLock::new(None),
            signer: Signer::new(),
            platform_block_hashes: Vec::new(),
        })
    }

    /// Generate a fresh ephemeral key pair & produce an attestation report (public key, etc.).
    ///
    /// In a real TEE, you'd also sign a quote proving the ephemeral key is inside the enclave,
    /// possibly with the TEE's long-term key. We skip that for brevity.
    pub fn get_report(&self, user_data: Vec<u8>) -> DomainAttestationReport {
        // Generate ephemeral X25519 key
        let ephemeral = Ephemeral::new();
        let pk = ephemeral.public_key().to_vec();

        // Store ephemeral so we can decrypt inbound messages that are ECDH'd to this pubkey
        *self.ephemeral_key.write().unwrap() = Some(ephemeral);

        DomainAttestationReport {
            ephemeral_public_key: pk,
            block_hashes: self.platform_block_hashes.clone(),
            user_data,
        }
    }

    /// put_secrets handles a set of secrets boxes from peers. Each secrets box is
    /// AEAD-encrypted to our ephemeral key. We decrypt & store them in our local store.
    pub fn put_secrets(&self, bundles: Vec<(DomainSecretsBox, DomainAttestationReport)>) -> bool {
        // We'll take the ephemeral out (moving it) so we only use it once.
        let ephemeral_opt = self.ephemeral_key.write().unwrap().take();
        let Some(ephemeral) = ephemeral_opt else {
            warn!("No ephemeral key available; cannot decrypt");
            return false;
        };

        for (secret_box, _attestation) in bundles {
            match self.decrypt_secrets_box(&ephemeral, &secret_box) {
                Ok(secrets) => {
                    for s in secrets {
                        debug!("Enclave storing secret: {:?}", s.id);
                        self.store_secret(s);
                    }
                }
                Err(e) => {
                    warn!("Failed to decrypt secrets_box: {}", e);
                }
            }
        }
        true
    }

    /// get_secrets returns an AEAD-encrypted secrets box (containing each requested secret)
    /// for the requester's ephemeral public key (in their attestation).
    pub fn get_secrets(
        &self,
        request_ids: Vec<SecretId>,
        _policy_reports: Vec<(Vec<u8>, Vec<u8>)>, // not used in this example
        requester_attestation: DomainAttestationReport,
    ) -> DomainSecretsBox {
        // 1. Verify the requester's attestation. For now, we assume it's valid.

        // 2. Collect the requested secrets from our local store
        let secrets = self.collect_secrets(request_ids);

        // 3. We generate a fresh ephemeral for the encryption back to the requester
        //    (so the ephemeral secret is ephemeral again).
        let local_ephemeral = Ephemeral::new();
        let local_pubkey = local_ephemeral.public_key().to_vec();

        // 4. Use the requester's ephemeral to do ECDH
        let Ok(peer_pk) =
            <[u8; 32]>::try_from(requester_attestation.ephemeral_public_key.as_slice())
        else {
            warn!("Invalid requester's ephemeral public key length, returning empty box");
            return DomainSecretsBox::new_empty();
        };

        let shared_secret = local_ephemeral.diffie_hellman(&peer_pk);

        // 5. Create AEAD from the shared secret
        let aead = Aead::new(&shared_secret);

        // 6. Serialize secrets via CBOR
        let plaintext = match serde_cbor::to_vec(&secrets) {
            Ok(vec) => vec,
            Err(_) => {
                warn!("Failed to serialize secrets, returning empty box");
                return DomainSecretsBox::new_empty();
            }
        };

        // 7. Encrypt
        let ciphertext = aead.encrypt(&plaintext);

        // 8. Sign the ciphertext with our long-term key
        let signature = self.signer.sign(&ciphertext).to_bytes().to_vec();

        // Build the DomainSecretsBox
        DomainSecretsBox {
            encrypted_payload: ciphertext,
            nonce: vec![], // We embed nonce in the ciphertext; left empty
            sender_public_key: local_pubkey,
            signature,
            alg: "X25519+AES256GCM".to_string(),
        }
    }

    /// check_secrets returns a list of statuses for each secret.
    /// For the demo, we simply mark secrets as "found" if present in the store, "expiry=0".
    pub fn check_secrets(&self, ids: Vec<SecretId>) -> Vec<(SecretId, bool, u64)> {
        let store_guard = self.store.read().unwrap();
        ids.into_iter()
            .map(|id| {
                let found = store_guard.contains_key(&id);
                (id, found, 0)
            })
            .collect()
    }

    /// Helper to store a single secret
    fn store_secret(&self, secret: DomainSecret) {
        self.store
            .write()
            .unwrap()
            .insert(secret.id.clone(), secret);
    }

    /// Helper to collect secrets from the store
    fn collect_secrets(&self, ids: Vec<SecretId>) -> Vec<DomainSecret> {
        let store_guard = self.store.read().unwrap();
        let mut results = Vec::new();
        for id in ids {
            if let Some(s) = store_guard.get(&id) {
                results.push(s.clone());
            }
        }
        results
    }

    /// Decrypt a SecretsBox with our ephemeral key.
    /// (We pass ephemeral explicitly from `put_secrets()` so it can be used only once.)
    fn decrypt_secrets_box(
        &self,
        ephemeral: &Ephemeral,
        box_: &DomainSecretsBox,
    ) -> Result<Vec<DomainSecret>, String> {
        // ephemeral ECDH
        let peer_pub = <[u8; 32]>::try_from(box_.sender_public_key.as_slice())
            .map_err(|_| "Invalid sender_public_key length".to_string())?;

        // Perform Diffie-Hellman key exchange
        let shared_secret = ephemeral.diffie_hellman(&peer_pub);

        // Create AEAD cipher from shared secret
        let aead = Aead::new(&shared_secret);

        // Decrypt AEAD. Our ciphertext includes the nonce as the first 12 bytes.
        let decrypted = aead
            .decrypt(&box_.encrypted_payload)
            .ok_or_else(|| "AEAD decrypt failed".to_string())?;

        // The payload is a CBOR-serialized Vec<DomainSecret>
        let secrets: Vec<DomainSecret> = serde_cbor::from_slice(&decrypted)
            .map_err(|e| format!("Deserialization failed: {:?}", e))?;

        Ok(secrets)
    }
}
