use std::convert::TryInto;

use aes_gcm_siv::{
    AeadCore as _, Aes256GcmSiv,
    aead::{Aead, KeyInit, OsRng, generic_array::GenericArray},
};
use nxcc_interface::types::{AttestationReport, SecretId, SecretsBox};
use sha2::{Digest, Sha256};
use thiserror::Error;
use x25519_dalek::{PublicKey, SharedSecret, StaticSecret};
use zeroize::Zeroize;

const AES_NONCE_SIZE: usize = 12; // 96 bits for AES-GCM-SIV

#[derive(Error, Debug)]
pub enum CryptoError {
    #[error("Invalid key length: expected {expected}, got {got}")]
    InvalidKeyLength { expected: usize, got: usize },
    #[error("Cryptography operation failed: {0}")]
    OperationFailed(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Deserialization error: {0}")]
    Deserialization(String),
}

/// Represents an X25519 keypair for key exchange.
/// Uses StaticSecret for the private part to allow repeated use for GetReport.
pub struct KeyExchangeKeyPair {
    secret: StaticSecret,
    public: PublicKey,
}

impl KeyExchangeKeyPair {
    /// Generates a new X25519 keypair.
    pub fn generate() -> Self {
        let secret = StaticSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    /// Returns the public key.
    pub fn public_key(&self) -> &PublicKey {
        &self.public
    }

    /// Computes the Diffie-Hellman shared secret.
    pub fn diffie_hellman(&self, their_public: &PublicKey) -> SharedSecret {
        self.secret.diffie_hellman(their_public)
    }

    /// Creates a keypair from raw bytes. Input must be 32 bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        let secret_bytes: [u8; 32] =
            bytes
                .try_into()
                .map_err(|_| CryptoError::InvalidKeyLength {
                    expected: 32,
                    got: bytes.len(),
                })?;
        let secret = StaticSecret::from(secret_bytes);
        let public = PublicKey::from(&secret);
        Ok(Self { secret, public })
    }

    /// Returns the secret key bytes.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.secret.to_bytes()
    }
}

impl Drop for KeyExchangeKeyPair {
    fn drop(&mut self) {
        self.secret.zeroize();
    }
}

/// Derives a symmetric key from a shared secret using SHA-256.
fn derive_symmetric_key(shared_secret: &SharedSecret) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(shared_secret.as_bytes());
    hasher.finalize().into()
}

/// Encrypts data using AES-GCM-SIV with the derived key.
/// Returns (nonce, ciphertext).
fn encrypt_aead(
    symmetric_key: &[u8; 32],
    plaintext: &[u8],
    aad: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
    let key = GenericArray::from_slice(symmetric_key);
    let cipher = Aes256GcmSiv::new(key);
    let nonce = Aes256GcmSiv::generate_nonce(&mut OsRng); // 96-bits; unique per message
    let ciphertext = cipher
        .encrypt(
            &nonce,
            aes_gcm_siv::aead::Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|e| CryptoError::OperationFailed(format!("AEAD encryption failed: {e}")))?;
    Ok((nonce.to_vec(), ciphertext))
}

/// Decrypts data using AES-GCM-SIV with the derived key.
fn decrypt_aead(
    symmetric_key: &[u8; 32],
    nonce: &[u8],
    ciphertext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if nonce.len() != AES_NONCE_SIZE {
        return Err(CryptoError::InvalidKeyLength {
            expected: AES_NONCE_SIZE,
            got: nonce.len(),
        });
    }
    let key = GenericArray::from_slice(symmetric_key);
    let cipher = Aes256GcmSiv::new(key);
    let nonce_arr = GenericArray::from_slice(nonce);
    let plaintext = cipher
        .decrypt(
            nonce_arr,
            aes_gcm_siv::aead::Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|e| CryptoError::OperationFailed(format!("AEAD decryption failed: {e}")))?;
    Ok(plaintext)
}

/// Encrypts secrets into a SecretsBox for a recipient.
/// Uses X25519 for key exchange and AES-GCM-SIV for encryption.
/// Includes the sender's public key in the box.
pub fn encrypt_secrets_box(
    our_kx_keypair: &KeyExchangeKeyPair,
    recipient_kx_pk: &PublicKey,
    secrets: &Vec<(SecretId, Vec<u8>, u64, u64)>, // (id, data, expiry, generation_ts)
) -> Result<SecretsBox, CryptoError> {
    let shared_secret = our_kx_keypair.diffie_hellman(recipient_kx_pk);
    let symmetric_key = derive_symmetric_key(&shared_secret);

    // Serialize secrets using ciborium
    let mut payload = Vec::new();
    ciborium::into_writer(secrets, &mut payload)
        .map_err(|e| CryptoError::Serialization(e.to_string()))?;

    // Construct AAD: recipient pubkey + sender pubkey
    let mut aad = Vec::new();
    aad.extend_from_slice(recipient_kx_pk.as_bytes());
    aad.extend_from_slice(our_kx_keypair.public_key().as_bytes());

    // Encrypt: prefix ciphertext with nonce
    let (nonce, ciphertext) = encrypt_aead(&symmetric_key, &payload, &aad)?;
    let mut encrypted_payload = nonce;
    encrypted_payload.extend(ciphertext);

    // Extract contained IDs
    let contained_secret_ids: Vec<SecretId> =
        secrets.iter().map(|(id, _, _, _)| id.clone()).collect();

    Ok(SecretsBox {
        encrypted_payload,
        sender_public_key: our_kx_keypair.public_key().as_bytes().to_vec(),
        alg: "X25519_AES-GCM-SIV".to_string(),
        contained_secret_ids,
    })
}

/// Decrypts secrets from a SecretsBox.
/// Assumes the SecretsBox integrity has been verified via attestation binding.
/// Uses X25519 for key exchange and AES-GCM-SIV for decryption.
pub fn decrypt_secrets_box(
    our_kx_keypair: &KeyExchangeKeyPair,
    secrets_box: &SecretsBox,
) -> Result<Vec<(SecretId, Vec<u8>, u64, u64)>, CryptoError> {
    if secrets_box.alg != "X25519_AES-GCM-SIV" {
        return Err(CryptoError::OperationFailed(format!(
            "Unsupported SecretsBox algorithm: {}",
            secrets_box.alg
        )));
    }

    // Extract sender's KX public key
    let sender_kx_pk_bytes: [u8; 32] = secrets_box
        .sender_public_key
        .as_slice()
        .try_into()
        .map_err(|_| CryptoError::InvalidKeyLength {
            expected: 32,
            got: secrets_box.sender_public_key.len(),
        })?;
    let sender_kx_pk = PublicKey::from(sender_kx_pk_bytes);

    // Derive key
    let shared_secret = our_kx_keypair.diffie_hellman(&sender_kx_pk);
    let symmetric_key = derive_symmetric_key(&shared_secret);

    // Extract nonce and ciphertext
    if secrets_box.encrypted_payload.len() < AES_NONCE_SIZE {
        return Err(CryptoError::OperationFailed(
            "Encrypted payload too short".to_string(),
        ));
    }
    let (nonce, ciphertext) = secrets_box.encrypted_payload.split_at(AES_NONCE_SIZE);

    // Construct AAD: our pubkey + sender pubkey
    let mut aad = Vec::new();
    aad.extend_from_slice(our_kx_keypair.public_key().as_bytes());
    aad.extend_from_slice(sender_kx_pk.as_bytes());

    // Decrypt
    let plaintext = decrypt_aead(&symmetric_key, nonce, ciphertext, &aad)?;

    // Deserialize secrets
    let secrets: Vec<(SecretId, Vec<u8>, u64, u64)> =
        ciborium::from_reader(plaintext.as_slice())
            .map_err(|e| CryptoError::Deserialization(e.to_string()))?;

    Ok(secrets)
}

/// Generates a dummy attestation report. In a real TEE, this would query the hardware.
pub fn generate_attestation(ephemeral_kx_pk: &PublicKey, user_data: Vec<u8>) -> AttestationReport {
    // TODO: Integrate with actual TEE attestation mechanism
    AttestationReport {
        ephemeral_public_key: ephemeral_kx_pk.as_bytes().to_vec(),
        measurement: vec![0u8; 32],                       // Placeholder
        block_hashes: vec![b"dummy_block_hash".to_vec()], // Placeholder
        user_data,
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256};

    use super::*;

    #[test]
    fn test_key_exchange_aead() {
        let alice_kx = KeyExchangeKeyPair::generate();
        let bob_kx = KeyExchangeKeyPair::generate();

        let alice_shared = alice_kx.diffie_hellman(bob_kx.public_key());
        let bob_shared = bob_kx.diffie_hellman(alice_kx.public_key());

        assert_eq!(alice_shared.as_bytes(), bob_shared.as_bytes());

        let alice_key = derive_symmetric_key(&alice_shared);
        let bob_key = derive_symmetric_key(&bob_shared);
        assert_eq!(alice_key, bob_key);

        let plaintext = b"Hello, secure world!";
        let aad = b"additional_authenticated_data";

        let (nonce, ciphertext) = encrypt_aead(&alice_key, plaintext, aad).unwrap();
        let decrypted = decrypt_aead(&bob_key, &nonce, &ciphertext, aad).unwrap();

        assert_eq!(plaintext.to_vec(), decrypted);

        // Test tampering with ciphertext
        let mut tampered_ciphertext = ciphertext.clone();
        tampered_ciphertext[0] ^= 1;
        let decrypt_err = decrypt_aead(&bob_key, &nonce, &tampered_ciphertext, aad);
        assert!(decrypt_err.is_err());

        // Test tampering with AAD
        let tampered_aad = b"tampered_aad";
        let decrypt_err_aad = decrypt_aead(&bob_key, &nonce, &ciphertext, tampered_aad);
        assert!(decrypt_err_aad.is_err());
    }

    #[test]
    fn test_secrets_box_roundtrip() {
        let sender_kx = KeyExchangeKeyPair::generate();
        let recipient_kx = KeyExchangeKeyPair::generate();

        let secret_id1 = SecretId {
            chain_id: 1,
            identity_address: Address::random(),
            identity_id: U256::from(123),
        };
        let secret_id2 = SecretId {
            chain_id: 5,
            identity_address: Address::random(),
            identity_id: U256::from(456),
        };
        let secrets_to_send = vec![
            (secret_id1.clone(), b"secret_data_1".to_vec(), 1000, 1),
            (secret_id2.clone(), b"secret_data_2".to_vec(), 2000, 2),
        ];

        let secrets_box =
            encrypt_secrets_box(&sender_kx, recipient_kx.public_key(), &secrets_to_send).unwrap();

        assert_eq!(secrets_box.contained_secret_ids.len(), 2);
        assert!(secrets_box.contained_secret_ids.contains(&secret_id1));
        assert!(secrets_box.contained_secret_ids.contains(&secret_id2));
        assert_eq!(secrets_box.alg, "X25519_AES-GCM-SIV");

        // Recipient decrypts
        let decrypted_secrets = decrypt_secrets_box(
            &recipient_kx, // Recipient uses their KX private key
            &secrets_box,
        )
        .unwrap();

        assert_eq!(secrets_to_send, decrypted_secrets);

        // Test decryption failure with wrong recipient key
        let wrong_recipient_kx = KeyExchangeKeyPair::generate();
        let decrypt_err = decrypt_secrets_box(&wrong_recipient_kx, &secrets_box);
        assert!(decrypt_err.is_err());
        assert!(matches!(
            decrypt_err.unwrap_err(),
            CryptoError::OperationFailed(_)
        )); // AEAD decrypt fails
    }

    #[test]
    fn test_keypair_serialization() {
        let kx_orig = KeyExchangeKeyPair::generate();
        let kx_bytes = kx_orig.to_bytes();
        let kx_recon = KeyExchangeKeyPair::from_bytes(&kx_bytes).unwrap();
        assert_eq!(kx_orig.public_key(), kx_recon.public_key());
        assert_eq!(
            kx_orig.secret.to_bytes(),
            kx_recon.secret.to_bytes() // Compare secrets directly
        );
    }
}
