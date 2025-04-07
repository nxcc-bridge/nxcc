use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tracing::{debug, warn};

use interface::{
    AttestationReport as DomainAttestationReport, Secret as DomainSecret, SecretId,
    SecretsBox as DomainSecretsBox,
};

use crate::crypto::{Aead, Ephemeral, Signer};

pub struct SecretsEnclave {
    store: RwLock<HashMap<SecretId, DomainSecret>>,
    ephemeral_key: RwLock<Option<Ephemeral>>,
    signer: Signer,
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

    pub fn get_report(&self, user_data: Vec<u8>) -> DomainAttestationReport {
        let ephemeral = Ephemeral::new();
        let pk = ephemeral.public_key().to_vec();

        *self.ephemeral_key.write().unwrap() = Some(ephemeral);

        DomainAttestationReport {
            ephemeral_public_key: pk,
            block_hashes: self.platform_block_hashes.clone(),
            user_data,
        }
    }

    pub fn put_secrets(&self, bundles: Vec<(DomainSecretsBox, DomainAttestationReport)>) -> bool {
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

    pub fn get_secrets(
        &self,
        request_ids: Vec<SecretId>,
        _policy_reports: Vec<(Vec<u8>, Vec<u8>)>,
        requester_attestation: DomainAttestationReport,
    ) -> DomainSecretsBox {
        let secrets = self.collect_secrets(request_ids);

        let local_ephemeral = Ephemeral::new();
        let local_pubkey = local_ephemeral.public_key().to_vec();

        let Ok(peer_pk) =
            <[u8; 32]>::try_from(requester_attestation.ephemeral_public_key.as_slice())
        else {
            warn!("Invalid requester's ephemeral public key length, returning empty box");
            return DomainSecretsBox::new_empty();
        };

        let shared_secret = local_ephemeral.diffie_hellman(&peer_pk);

        let aead = Aead::new(&shared_secret);

        let mut plaintext = Vec::new();
        if let Err(e) = ciborium::into_writer(&secrets, &mut plaintext) {
            warn!("Failed to serialize secrets: {}, returning empty box", e);
            return DomainSecretsBox::new_empty();
        }

        let ciphertext = aead.encrypt(&plaintext);

        let signature = self.signer.sign(&ciphertext).to_bytes().to_vec();

        DomainSecretsBox {
            encrypted_payload: ciphertext,
            nonce: vec![],
            sender_public_key: local_pubkey,
            signature,
            alg: "X25519+AES256GCM".to_string(),
        }
    }

    pub fn check_secrets(&self, ids: Vec<SecretId>) -> Vec<(SecretId, bool, u64)> {
        let store_guard = self.store.read().unwrap();
        ids.into_iter()
            .map(|id| {
                let found = store_guard.contains_key(&id);
                (id, found, 0)
            })
            .collect()
    }

    fn store_secret(&self, secret: DomainSecret) {
        self.store
            .write()
            .unwrap()
            .insert(secret.id.clone(), secret);
    }

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

    fn decrypt_secrets_box(
        &self,
        ephemeral: &Ephemeral,
        box_: &DomainSecretsBox,
    ) -> Result<Vec<DomainSecret>, String> {
        let peer_pub = <[u8; 32]>::try_from(box_.sender_public_key.as_slice())
            .map_err(|_| "Invalid sender_public_key length".to_string())?;

        let shared_secret = ephemeral.diffie_hellman(&peer_pub);

        let aead = Aead::new(&shared_secret);

        let decrypted = aead
            .decrypt(&box_.encrypted_payload)
            .ok_or_else(|| "AEAD decrypt failed".to_string())?;

        let secrets: Vec<DomainSecret> = ciborium::from_reader(&decrypted[..])
            .map_err(|e| format!("Deserialization failed: {:?}", e))?;

        Ok(secrets)
    }
}
