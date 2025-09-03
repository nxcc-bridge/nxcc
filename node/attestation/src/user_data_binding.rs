// Enhanced User Data Binding for Real Quotes
// Handles ephemeral keys, large data hashing, and freshness proofs

use anyhow::{anyhow, Result};
use nxcc_interface::types::attestation::UserDataBinding;
use sha2::{Digest, Sha256};

use crate::BlockInfo;

/// Enhanced user data binding that handles ephemeral keys and freshness
pub struct EnhancedUserDataBinding {
    pub original_data: Vec<u8>,
    pub ephemeral_public_key: Vec<u8>,
    pub freshness_data: Vec<u8>,
    pub embedded_hash: Vec<u8>,
    pub was_hashed: bool,
    pub includes_ephemeral_key: bool,
    pub includes_freshness: bool,
}

impl EnhancedUserDataBinding {
    /// Create binding with ephemeral key and freshness proof
    pub fn new_with_ephemeral_and_freshness(
        ephemeral_key: &[u8],
        user_data: &[u8],
        block_hashes: &[BlockInfo],
        max_size: usize,
    ) -> Self {
        // Create freshness data from block hashes
        let freshness_data = Self::create_freshness_data(block_hashes);

        // Combine ephemeral key + user data + freshness
        let mut combined_data = Vec::new();
        combined_data.extend_from_slice(ephemeral_key);
        combined_data.extend_from_slice(user_data);
        combined_data.extend_from_slice(&freshness_data);

        // Hash if too large for platform
        let (embedded_hash, was_hashed) = if combined_data.len() <= max_size {
            (combined_data.clone(), false)
        } else {
            (Self::hash_data(&combined_data), true)
        };

        Self {
            original_data: user_data.to_vec(),
            ephemeral_public_key: ephemeral_key.to_vec(),
            freshness_data,
            embedded_hash,
            was_hashed,
            includes_ephemeral_key: !ephemeral_key.is_empty(),
            includes_freshness: !block_hashes.is_empty(),
        }
    }

    /// Create binding with just ephemeral key
    pub fn new_with_ephemeral_key(ephemeral_key: &[u8], user_data: &[u8], max_size: usize) -> Self {
        let mut combined_data = Vec::new();
        combined_data.extend_from_slice(ephemeral_key);
        combined_data.extend_from_slice(user_data);

        let (embedded_hash, was_hashed) = if combined_data.len() <= max_size {
            (combined_data.clone(), false)
        } else {
            (Self::hash_data(&combined_data), true)
        };

        Self {
            original_data: user_data.to_vec(),
            ephemeral_public_key: ephemeral_key.to_vec(),
            freshness_data: Vec::new(),
            embedded_hash,
            was_hashed,
            includes_ephemeral_key: !ephemeral_key.is_empty(),
            includes_freshness: false,
        }
    }

    /// Create simple binding without ephemeral key or freshness
    pub fn new_simple(user_data: &[u8], max_size: usize) -> Self {
        let (embedded_hash, was_hashed) = if user_data.len() <= max_size {
            (user_data.to_vec(), false)
        } else {
            (Self::hash_data(user_data), true)
        };

        Self {
            original_data: user_data.to_vec(),
            ephemeral_public_key: Vec::new(),
            freshness_data: Vec::new(),
            embedded_hash,
            was_hashed,
            includes_ephemeral_key: false,
            includes_freshness: false,
        }
    }

    /// Create freshness data from block hashes
    fn create_freshness_data(block_hashes: &[BlockInfo]) -> Vec<u8> {
        let mut freshness_data = Vec::new();

        for block in block_hashes {
            // Include chain ID (8 bytes) + block number (8 bytes) + hash (32 bytes)
            freshness_data.extend_from_slice(&block.chain_id.to_le_bytes());
            freshness_data.extend_from_slice(&block.block_number.to_le_bytes());
            freshness_data.extend_from_slice(&block.block_hash);
        }

        freshness_data
    }

    /// Hash data using SHA256
    fn hash_data(data: &[u8]) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().to_vec()
    }

    /// Verify the binding integrity
    pub fn verify_binding(&self) -> bool {
        if !self.was_hashed {
            // Direct embedding - check that embedded hash matches combined data
            let mut expected_data = Vec::new();

            if self.includes_ephemeral_key {
                expected_data.extend_from_slice(&self.ephemeral_public_key);
            }
            expected_data.extend_from_slice(&self.original_data);
            if self.includes_freshness {
                expected_data.extend_from_slice(&self.freshness_data);
            }

            self.embedded_hash == expected_data
        } else {
            // Hashed embedding - verify hash
            let mut data_to_hash = Vec::new();

            if self.includes_ephemeral_key {
                data_to_hash.extend_from_slice(&self.ephemeral_public_key);
            }
            data_to_hash.extend_from_slice(&self.original_data);
            if self.includes_freshness {
                data_to_hash.extend_from_slice(&self.freshness_data);
            }

            let expected_hash = Self::hash_data(&data_to_hash);
            self.embedded_hash == expected_hash
        }
    }

    /// Extract ephemeral key from embedded data (if not hashed)
    pub fn extract_ephemeral_key(&self) -> Option<Vec<u8>> {
        if self.includes_ephemeral_key && !self.was_hashed && !self.ephemeral_public_key.is_empty()
        {
            Some(self.ephemeral_public_key.clone())
        } else if self.includes_ephemeral_key && !self.was_hashed && self.embedded_hash.len() >= 32
        {
            // Try to extract from embedded data
            Some(self.embedded_hash[..32].to_vec())
        } else {
            None
        }
    }

    /// Convert to legacy UserDataBinding for compatibility
    pub fn to_legacy_binding(&self) -> UserDataBinding {
        UserDataBinding {
            original_data: self.original_data.clone(),
            embedded_hash: self.embedded_hash.clone(),
            was_hashed: self.was_hashed,
            ephemeral_key_len: if self.includes_ephemeral_key {
                self.ephemeral_public_key.len()
            } else {
                0
            },
        }
    }

    /// Create from legacy UserDataBinding
    pub fn from_legacy_binding(binding: &UserDataBinding) -> Self {
        Self {
            original_data: binding.original_data.clone(),
            ephemeral_public_key: Vec::new(),
            freshness_data: Vec::new(),
            embedded_hash: binding.embedded_hash.clone(),
            was_hashed: binding.was_hashed,
            includes_ephemeral_key: false,
            includes_freshness: false,
        }
    }
}

/// Freshness proof validator
pub struct FreshnessValidator {
    max_block_age_seconds: u64,
    required_chains: Vec<u64>,
}

impl FreshnessValidator {
    pub fn new(max_block_age_seconds: u64, required_chains: Vec<u64>) -> Self {
        Self {
            max_block_age_seconds,
            required_chains,
        }
    }

    /// Validate freshness of block hashes
    pub fn validate_freshness(&self, block_hashes: &[BlockInfo]) -> Result<()> {
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Check that all required chains are represented
        for &required_chain in &self.required_chains {
            if !block_hashes
                .iter()
                .any(|block| block.chain_id == required_chain)
            {
                return Err(anyhow!(
                    "Missing block hash for required chain {}",
                    required_chain
                ));
            }
        }

        // Validate age of each block
        for block in block_hashes {
            let block_age = current_time.saturating_sub(block.timestamp);
            if block_age > self.max_block_age_seconds {
                return Err(anyhow!(
                    "Block {} on chain {} is too old: {} seconds (max: {})",
                    block.block_number,
                    block.chain_id,
                    block_age,
                    self.max_block_age_seconds
                ));
            }
        }

        Ok(())
    }

    /// Extract freshness data from user data binding
    pub fn extract_freshness_data(&self, binding: &EnhancedUserDataBinding) -> Option<Vec<u8>> {
        if binding.includes_freshness {
            Some(binding.freshness_data.clone())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enhanced_binding_with_ephemeral_key() {
        let ephemeral_key = &[0x42; 32];
        let user_data = b"test user data";
        let max_size = 64; // TDX limit

        let binding =
            EnhancedUserDataBinding::new_with_ephemeral_key(ephemeral_key, user_data, max_size);

        assert!(binding.includes_ephemeral_key);
        assert!(!binding.includes_freshness);
        assert!(!binding.was_hashed); // 32 + 14 = 46 bytes, under limit
        assert!(binding.verify_binding());

        let extracted_key = binding.extract_ephemeral_key().unwrap();
        assert_eq!(extracted_key, ephemeral_key);
    }

    #[test]
    fn test_enhanced_binding_with_freshness() {
        let ephemeral_key = &[0x42; 32];
        let user_data = b"test";
        let block_hashes = vec![BlockInfo {
            chain_id: 1,
            chain_name: "ethereum".to_string(),
            block_number: 12345,
            block_hash: vec![0xaa; 32],
            timestamp: 1234567890,
            fetched_at: 1234567900,
        }];
        let max_size = 64;

        let binding = EnhancedUserDataBinding::new_with_ephemeral_and_freshness(
            ephemeral_key,
            user_data,
            &block_hashes,
            max_size,
        );

        assert!(binding.includes_ephemeral_key);
        assert!(binding.includes_freshness);
        assert!(binding.was_hashed); // Combined data exceeds limit
        assert!(binding.verify_binding());
        assert_eq!(binding.embedded_hash.len(), 32); // SHA256 hash
    }

    #[test]
    fn test_large_data_hashing() {
        let ephemeral_key = &[0x42; 32];
        let large_data = vec![0x55; 1000]; // Large user data
        let max_size = 64;

        let binding =
            EnhancedUserDataBinding::new_with_ephemeral_key(ephemeral_key, &large_data, max_size);

        assert!(binding.was_hashed);
        assert_eq!(binding.embedded_hash.len(), 32); // SHA256 hash
        assert!(binding.verify_binding());
        assert_eq!(binding.original_data, large_data);
    }

    #[test]
    fn test_freshness_validator() {
        let validator = FreshnessValidator::new(300, vec![1, 137]); // 5 minutes, Ethereum + Polygon

        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let fresh_blocks = vec![
            BlockInfo {
                chain_id: 1,
                chain_name: "ethereum".to_string(),
                block_number: 12345,
                block_hash: vec![0xaa; 32],
                timestamp: current_time - 100, // 100 seconds ago
                fetched_at: current_time,
            },
            BlockInfo {
                chain_id: 137,
                chain_name: "polygon".to_string(),
                block_number: 67890,
                block_hash: vec![0xbb; 32],
                timestamp: current_time - 200, // 200 seconds ago
                fetched_at: current_time,
            },
        ];

        assert!(validator.validate_freshness(&fresh_blocks).is_ok());

        // Test with old block
        let old_blocks = vec![BlockInfo {
            chain_id: 1,
            chain_name: "ethereum".to_string(),
            block_number: 12345,
            block_hash: vec![0xaa; 32],
            timestamp: current_time - 500, // 500 seconds ago (too old)
            fetched_at: current_time,
        }];

        assert!(validator.validate_freshness(&old_blocks).is_err());
    }

    #[test]
    fn test_legacy_compatibility() {
        let original_data = b"test data".to_vec();
        let legacy_binding = UserDataBinding::new(original_data.clone(), 64);

        let enhanced = EnhancedUserDataBinding::from_legacy_binding(&legacy_binding);
        assert_eq!(enhanced.original_data, original_data);
        assert!(!enhanced.includes_ephemeral_key);
        assert!(!enhanced.includes_freshness);

        let converted_back = enhanced.to_legacy_binding();
        assert_eq!(converted_back.original_data, legacy_binding.original_data);
        assert_eq!(converted_back.embedded_hash, legacy_binding.embedded_hash);
        assert_eq!(converted_back.was_hashed, legacy_binding.was_hashed);
    }
}
