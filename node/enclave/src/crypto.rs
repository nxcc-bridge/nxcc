use aes_gcm_siv::{
    Aes256GcmSiv, Key, Nonce,
    aead::{Aead as AeadTrait, KeyInit},
};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier, VerifyingKey};
use rand_core::RngCore;
use x25519_dalek::{EphemeralSecret, PublicKey as X25519PublicKey};

use std::fmt;

/// Ephemeral key for X25519 key exchange
pub struct Ephemeral {
    pub(crate) secret: EphemeralSecret,
    pub(crate) public: X25519PublicKey,
}

impl Ephemeral {
    /// Generate a new ephemeral key pair for X25519
    pub fn new() -> Self {
        let secret = EphemeralSecret::random();
        let public = X25519PublicKey::from(&secret);
        Self { secret, public }
    }

    /// Perform Diffie-Hellman key exchange with peer's public key
    pub fn diffie_hellman(&self, peer_public: &[u8; 32]) -> [u8; 32] {
        let peer_key = X25519PublicKey::from(*peer_public);
        let shared_secret = self.secret.diffie_hellman(&peer_key);
        *shared_secret.as_bytes()
    }

    /// Get the public key bytes
    pub fn public_key(&self) -> &[u8; 32] {
        self.public.as_bytes()
    }
}

impl Clone for Ephemeral {
    fn clone(&self) -> Self {
        // Note: This is inefficient and should only be used when absolutely necessary
        // since we need to regenerate the secret key (which doesn't implement Clone).
        // In practice, avoid cloning ephemeral keys.
        let _secret_bytes = self.public.as_bytes();
        Self::new()
    }
}

/// AEAD encryption with AES-256-GCM-SIV
pub struct Aead {
    cipher: Aes256GcmSiv,
}

impl Aead {
    /// Create a new AEAD cipher from a 32-byte key
    pub fn new(key_material: &[u8; 32]) -> Self {
        let key = Key::<Aes256GcmSiv>::from_slice(key_material);
        let cipher = Aes256GcmSiv::new(key);
        Self { cipher }
    }

    /// Encrypt data with AES-256-GCM-SIV
    ///
    /// Returns ciphertext with nonce prepended (first 12 bytes)
    pub fn encrypt(&self, plaintext: &[u8]) -> Vec<u8> {
        // Generate a random 96-bit nonce
        let mut nonce_bytes = [0u8; 12];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        // Encrypt
        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext)
            .expect("encryption should not fail with valid inputs");

        // Prepend the nonce to the ciphertext
        let mut output = nonce_bytes.to_vec();
        output.extend_from_slice(&ciphertext);
        output
    }

    /// Decrypt data with AES-256-GCM-SIV
    ///
    /// Expects ciphertext with nonce prepended (first 12 bytes)
    pub fn decrypt(&self, ciphertext: &[u8]) -> Option<Vec<u8>> {
        if ciphertext.len() < 12 {
            return None;
        }

        let nonce_bytes = &ciphertext[..12];
        let payload = &ciphertext[12..];

        let nonce = Nonce::from_slice(nonce_bytes);
        self.cipher.decrypt(nonce, payload).ok()
    }
}

/// Ed25519 signing and verification
pub struct Signer {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
}

impl Signer {
    /// Generate a new Ed25519 signing key
    pub fn new() -> Self {
        let signing_key = SigningKey::generate(&mut rand_core_0_6::OsRng);
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
        }
    }

    /// Sign a message with the Ed25519 private key
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }

    /// Verify a signature with the Ed25519 public key
    pub fn verify(&self, message: &[u8], signature: &Signature) -> bool {
        self.verifying_key.verify(message, signature).is_ok()
    }

    /// Get the public key bytes
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.verifying_key.to_bytes()
    }
}

impl fmt::Debug for Signer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Signer")
            .field("public_key", &hex::encode(self.verifying_key.to_bytes()))
            .finish_non_exhaustive() // Don't print the private key
    }
}
