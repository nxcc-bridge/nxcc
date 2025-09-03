use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::BlockInfo;

/// The structured data that is serialized and bound to the attestation quote.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UserData {
    /// The ephemeral public key for key exchange.
    pub ephemeral_public_key: Vec<u8>,
    /// Freshness information from various blockchains.
    pub block_hashes: Vec<BlockInfo>,
}

impl UserData {
    pub fn new(ephemeral_public_key: Vec<u8>, block_hashes: Vec<BlockInfo>) -> Self {
        Self {
            ephemeral_public_key,
            block_hashes,
        }
    }

    /// Serializes the UserData struct into a canonical byte representation using CBOR.
    pub fn to_cbor(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        ciborium::into_writer(self, &mut bytes)?;
        Ok(bytes)
    }

    /// Deserializes a byte slice into a UserData struct.
    pub fn from_cbor(bytes: &[u8]) -> Result<Self> {
        let userdata: UserData = ciborium::from_reader(bytes)?;
        Ok(userdata)
    }
}

/// Hashes the serialized user data payload to produce a 32-byte hash for quote binding.
pub fn hash_userdata(serialized_userdata: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(serialized_userdata);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_userdata_serialization_roundtrip() {
        let user_data = UserData {
            ephemeral_public_key: vec![1; 32],
            block_hashes: vec![BlockInfo {
                chain_id: 1,
                chain_name: "ethereum".to_string(),
                block_number: 12345,
                block_hash: vec![0xaa; 32],
                timestamp: 1234567890,
                fetched_at: 1234567900,
            }],
        };

        let cbor_bytes = user_data.to_cbor().unwrap();
        assert!(!cbor_bytes.is_empty());

        let deserialized_user_data = UserData::from_cbor(&cbor_bytes).unwrap();
        assert_eq!(user_data, deserialized_user_data);
    }

    #[test]
    fn test_userdata_hashing() {
        let user_data = UserData {
            ephemeral_public_key: vec![1; 32],
            block_hashes: vec![],
        };

        let cbor_bytes = user_data.to_cbor().unwrap();
        let hash = hash_userdata(&cbor_bytes);

        assert_eq!(hash.len(), 32);

        // A different payload should produce a different hash
        let user_data2 = UserData {
            ephemeral_public_key: vec![2; 32],
            block_hashes: vec![],
        };
        let cbor_bytes2 = user_data2.to_cbor().unwrap();
        let hash2 = hash_userdata(&cbor_bytes2);

        assert_ne!(hash, hash2);
    }
}
