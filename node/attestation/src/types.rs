use std::collections::HashMap;

use nxcc_interface::gateway::BlockInfo;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// User data that exceeds platform limits gets hashed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDataBinding {
    /// The actual user data (may be large)
    pub original_data: Vec<u8>,
    /// Hash that was embedded in the attestation
    pub embedded_hash: Vec<u8>,
    /// Whether the data was hashed due to size constraints
    pub was_hashed: bool,
    /// Ephemeral public key length (used for extraction)
    pub ephemeral_key_len: usize,
}

impl UserDataBinding {
    /// Create binding, hashing if necessary for platform constraints
    pub fn new(data: Vec<u8>, max_size: usize) -> Self {
        if data.len() <= max_size {
            Self {
                embedded_hash: data.clone(),
                original_data: data,
                was_hashed: false,
                ephemeral_key_len: 0, // No ephemeral key separation when data fits
            }
        } else {
            let mut hasher = Sha256::new();
            hasher.update(&data);
            let hash = hasher.finalize().to_vec();

            Self {
                embedded_hash: hash,
                original_data: data,
                was_hashed: true,
                ephemeral_key_len: 0,
            }
        }
    }

    /// Create binding with explicit ephemeral key and user data
    pub fn new_with_ephemeral_key(ephemeral_key: &[u8], user_data: &[u8], max_size: usize) -> Self {
        let mut combined_data = Vec::new();
        combined_data.extend_from_slice(ephemeral_key);
        combined_data.extend_from_slice(user_data);

        if combined_data.len() <= max_size {
            Self {
                embedded_hash: combined_data.clone(),
                original_data: combined_data,
                was_hashed: false,
                ephemeral_key_len: ephemeral_key.len(),
            }
        } else {
            // Hash the combined data if too large
            let mut hasher = Sha256::new();
            hasher.update(&combined_data);
            let hash = hasher.finalize().to_vec();

            Self {
                embedded_hash: hash,
                original_data: combined_data,
                was_hashed: true,
                ephemeral_key_len: ephemeral_key.len(),
            }
        }
    }

    /// Verify that the embedded hash matches the original data
    pub fn verify_binding(&self) -> bool {
        if !self.was_hashed {
            // Direct data comparison
            self.embedded_hash == self.original_data
        } else {
            // Hash verification
            let mut hasher = Sha256::new();
            hasher.update(&self.original_data);
            let computed_hash = hasher.finalize().to_vec();
            computed_hash == self.embedded_hash
        }
    }

    /// Extract ephemeral key from the binding
    pub fn extract_ephemeral_key(&self) -> Vec<u8> {
        if self.ephemeral_key_len == 0 {
            return Vec::new();
        }

        if !self.was_hashed && self.original_data.len() >= self.ephemeral_key_len {
            self.original_data[..self.ephemeral_key_len].to_vec()
        } else if !self.was_hashed && self.embedded_hash.len() >= self.ephemeral_key_len {
            self.embedded_hash[..self.ephemeral_key_len].to_vec()
        } else {
            // If data was hashed, we can't extract the original ephemeral key
            Vec::new()
        }
    }

    /// Extract user data from the binding
    pub fn extract_user_data(&self) -> Vec<u8> {
        if self.ephemeral_key_len == 0 {
            return if self.was_hashed {
                self.original_data.clone()
            } else {
                self.embedded_hash.clone()
            };
        }

        if !self.was_hashed && self.original_data.len() > self.ephemeral_key_len {
            self.original_data[self.ephemeral_key_len..].to_vec()
        } else if !self.was_hashed && self.embedded_hash.len() > self.ephemeral_key_len {
            self.embedded_hash[self.ephemeral_key_len..].to_vec()
        } else {
            // If data was hashed, return original user data portion
            if self.original_data.len() > self.ephemeral_key_len {
                self.original_data[self.ephemeral_key_len..].to_vec()
            } else {
                Vec::new()
            }
        }
    }

    /// Verify that extracted ephemeral key and user data match the binding
    pub fn verify_extraction(&self, ephemeral_key: &[u8], user_data: &[u8]) -> bool {
        let expected_ephemeral = self.extract_ephemeral_key();
        let expected_user_data = self.extract_user_data();

        ephemeral_key == expected_ephemeral && user_data == expected_user_data
    }
}

/// Complete attestation bundle with all verification data
#[derive(Debug, Clone)]
pub struct AttestationBundle {
    pub raw_attestation: RawAttestation,
    pub user_data_binding: UserDataBinding,
    pub block_hashes: Vec<BlockInfo>,
}

/// Platform-specific raw attestation
#[derive(Debug, Clone)]
pub struct RawAttestation {
    pub platform_type: String,              // "tdx", "sgx", "nitro"
    pub evidence: Vec<u8>,                  // Quote, report, or evidence blob
    pub certificates: Option<Vec<Vec<u8>>>, // Certificate chain for verification
}

/// Standardized attestation claims following IEATS/RATS format
/// Based on draft-ietf-rats-eat and draft-ietf-rats-architecture
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandardizedClaims {
    /// EAT claim: Software components (primary measurement)
    /// TDX: MRTD, SGX: MRENCLAVE
    #[serde(rename = "swcomp")]
    pub software_component: Vec<u8>,

    /// EAT claim: Hardware security level
    /// 1=debug, 3=hardware/production
    #[serde(rename = "hwslvl")]
    pub hardware_security_level: u32,

    /// EAT claim: Security version number
    #[serde(rename = "svn")]
    pub security_version_number: u64,

    /// EAT claim: Platform instance identifier
    #[serde(rename = "ueid")]
    pub unique_entity_id: Vec<u8>,

    /// EAT claim: Nonce bound to attestation
    #[serde(rename = "nonce")]
    pub nonce: Vec<u8>,

    /// EAT claim: Issued at timestamp (Unix epoch)
    #[serde(rename = "iat")]
    pub issued_at: u64,

    /// Platform-specific measurements (RTMRs, PCRs)
    #[serde(rename = "measur")]
    pub measurements: HashMap<String, Vec<u8>>,

    /// Platform identifier
    #[serde(rename = "oemid")]
    pub oem_id: String,
}

// BlockInfo is now defined in nxcc_interface::gateway

/// Result of attestation verification
#[derive(Debug)]
pub enum VerificationResult {
    /// Verification successful with extracted claims
    Verified(StandardizedClaims),
    /// Provider cannot handle this attestation type (try next provider)
    Unsupported,
    /// Verification failed definitively (attestation is invalid)
    Failed(String),
}
