use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{gateway, proto::interface};

/// EAT-compliant measurement entry for interface
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceMeasurement {
    /// Hash value
    pub val: Vec<u8>,
    /// Hash algorithm: "sha-256", "sha-384", or "sha-512"
    pub alg: String,
    /// Category: "boot", "firmware", "kernel", "initrd", "vmm", "application", "policy", etc.
    pub measurement_type: Option<String>,
    /// Vendor information
    pub vendor: Option<String>,
    /// Version information
    pub version: Option<String>,
}

/// JWK structure for cnf claim (interface)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceJwk {
    /// Key type: "EC", "RSA", "OKP"
    pub kty: String,
    /// Curve for EC/OKP keys: "P-256", "P-384", "P-521", "X25519", "Ed25519"
    pub crv: Option<String>,
    /// X coordinate (for EC keys) or raw key (for OKP)
    pub x: Option<String>,
    /// Y coordinate (for EC keys)
    pub y: Option<String>,
}

/// EAT confirmation claim for interface
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InterfaceConfirmationMethod {
    /// JSON-profile style
    Jwk { jwk: InterfaceJwk },
    /// COSE-profile style
    CoseKey { cose_key: Vec<u8> },
}

/// Standardized attestation claims following IETF EAT (RFC 9711) - Interface Version
/// This contains the essential claims needed by interface consumers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandardizedAttestationClaims {
    // == Core freshness and context ==
    /// Issued-at time of the evidence production or verification moment
    pub iat: u64,
    /// Verifier challenge to prevent replay (if used)
    pub eat_nonce: Option<Vec<u8>>,

    // == Identity and provenance ==
    /// Stable device/realm identity
    pub ueid: Option<Vec<u8>>,
    /// Manufacturer identifier
    pub oemid: Option<String>,
    /// Hardware model descriptor
    pub hwmodel: Option<String>,
    /// Hardware/firmware version string
    pub hwversion: Option<String>,

    // == Debug and boot status ==
    /// Debug/production mode: 0=debug disabled (production), 4=debug enabled
    pub dbgstat: u8,
    /// OEM-authorized secure boot active
    pub oemboot: Option<bool>,

    // == Software identity ==
    /// Product or component name of the attested software root
    pub swname: Option<String>,
    /// Version string of the attested software root
    pub swversion: Option<String>,

    // == Measurements and results ==
    /// Cryptographic measurements relevant to trust decisions (required - at least one)
    pub measurements: Vec<InterfaceMeasurement>,

    // == Key binding ==
    /// Proof-of-possession key bound to this attested state
    pub cnf: Option<InterfaceConfirmationMethod>,
    /// Intended use for the token/key (typically 5 for proof-of-possession)
    pub intuse: Option<u8>,

    // == Lifecycle freshness ==
    /// Seconds since last boot according to the attested environment
    pub uptime: Option<u64>,
    /// Number of boots observed
    pub bootcount: Option<u64>,
    /// Per-boot unique random seed to distinguish boot instances
    pub bootseed: Option<Vec<u8>>,

    // == Profile selection ==
    /// URI-like identifier of the interpretation profile for platform specifics
    pub eat_profile: String,
}

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
    /// Create binding with ephemeral key, hashing if necessary for platform constraints
    pub fn new_with_ephemeral_key(
        ephemeral_key: Vec<u8>,
        user_data: Vec<u8>,
        max_size: usize,
    ) -> Self {
        let mut combined_data = ephemeral_key.clone();
        combined_data.extend_from_slice(&user_data);

        if combined_data.len() <= max_size {
            Self {
                embedded_hash: combined_data,
                original_data: user_data,
                was_hashed: false,
                ephemeral_key_len: ephemeral_key.len(),
            }
        } else {
            let mut hasher = Sha256::new();
            hasher.update(&combined_data);
            let hash = hasher.finalize().to_vec();

            Self {
                embedded_hash: hash,
                original_data: user_data,
                was_hashed: true,
                ephemeral_key_len: ephemeral_key.len(),
            }
        }
    }

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

        if !self.was_hashed && self.embedded_hash.len() >= self.ephemeral_key_len {
            // When not hashed, embedded_hash contains the combined ephemeral_key + user_data
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

        if !self.was_hashed && self.embedded_hash.len() > self.ephemeral_key_len {
            // When not hashed, embedded_hash contains ephemeral_key + user_data
            self.embedded_hash[self.ephemeral_key_len..].to_vec()
        } else if self.was_hashed {
            // If data was hashed, return original user data (stored separately)
            self.original_data.clone()
        } else {
            // Fallback case
            Vec::new()
        }
    }

    /// Verify that extracted ephemeral key and user data match the binding
    pub fn verify_extraction(&self, ephemeral_key: &[u8], user_data: &[u8]) -> bool {
        let expected_ephemeral = self.extract_ephemeral_key();
        let expected_user_data = self.extract_user_data();

        ephemeral_key == expected_ephemeral && user_data == expected_user_data
    }
}

/// Platform-specific raw attestation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawAttestation {
    pub platform_type: String,              // "tdx", "sgx", "nitro"
    pub evidence: Vec<u8>,                  // Quote, report, or evidence blob
    pub certificates: Option<Vec<Vec<u8>>>, // Certificate chain for verification
}

/// Complete attestation bundle with all verification data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationBundle {
    pub raw_attestation: RawAttestation,
    pub user_data_binding: UserDataBinding,
    pub block_hashes: Vec<gateway::BlockInfo>,
}

impl From<interface::RawAttestation> for RawAttestation {
    fn from(p: interface::RawAttestation) -> Self {
        Self {
            platform_type: p.platform_type,
            evidence: p.evidence,
            certificates: if p.certificates.is_empty() {
                None
            } else {
                Some(p.certificates)
            },
        }
    }
}

impl From<RawAttestation> for interface::RawAttestation {
    fn from(value: RawAttestation) -> Self {
        Self {
            platform_type: value.platform_type,
            evidence: value.evidence,
            certificates: value.certificates.unwrap_or_default(),
        }
    }
}

impl From<interface::UserDataBinding> for UserDataBinding {
    fn from(p: interface::UserDataBinding) -> Self {
        Self {
            original_data: p.original_data,
            embedded_hash: p.embedded_hash,
            was_hashed: p.was_hashed,
            ephemeral_key_len: p.ephemeral_key_len as usize,
        }
    }
}

impl From<UserDataBinding> for interface::UserDataBinding {
    fn from(value: UserDataBinding) -> Self {
        Self {
            original_data: value.original_data,
            embedded_hash: value.embedded_hash,
            was_hashed: value.was_hashed,
            ephemeral_key_len: value.ephemeral_key_len as u32,
        }
    }
}

impl From<interface::AttestationBundle> for AttestationBundle {
    fn from(p: interface::AttestationBundle) -> Self {
        Self {
            raw_attestation: p
                .raw_attestation
                .map(RawAttestation::from)
                .unwrap_or_else(|| RawAttestation {
                    platform_type: String::new(),
                    evidence: Vec::new(),
                    certificates: None,
                }),
            user_data_binding: p
                .user_data_binding
                .map(UserDataBinding::from)
                .unwrap_or_else(|| UserDataBinding {
                    original_data: Vec::new(),
                    embedded_hash: Vec::new(),
                    was_hashed: false,
                    ephemeral_key_len: 0,
                }),
            block_hashes: p
                .block_hashes
                .into_iter()
                .map(|b| gateway::BlockInfo {
                    chain_id: b.chain_id,
                    chain_name: b.chain_name,
                    block_number: b.block_number,
                    block_hash: b.block_hash,
                    timestamp: b.timestamp,
                    fetched_at: b.fetched_at,
                })
                .collect(),
        }
    }
}

impl From<AttestationBundle> for interface::AttestationBundle {
    fn from(value: AttestationBundle) -> Self {
        Self {
            raw_attestation: Some(value.raw_attestation.into()),
            user_data_binding: Some(value.user_data_binding.into()),
            block_hashes: value
                .block_hashes
                .into_iter()
                .map(|b| interface::BlockInfo {
                    chain_id: b.chain_id,
                    chain_name: b.chain_name,
                    block_number: b.block_number,
                    block_hash: b.block_hash,
                    timestamp: b.timestamp,
                    fetched_at: b.fetched_at,
                })
                .collect(),
        }
    }
}

impl From<&AttestationBundle> for interface::AttestationBundle {
    fn from(value: &AttestationBundle) -> Self {
        Self {
            raw_attestation: Some(value.raw_attestation.clone().into()),
            user_data_binding: Some(value.user_data_binding.clone().into()),
            block_hashes: value
                .block_hashes
                .iter()
                .map(|b| interface::BlockInfo {
                    chain_id: b.chain_id,
                    chain_name: b.chain_name.clone(),
                    block_number: b.block_number,
                    block_hash: b.block_hash.clone(),
                    timestamp: b.timestamp,
                    fetched_at: b.fetched_at,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorSignature {
    pub cose_sign1: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvReport {
    pub attestation: AttestationBundle,
    pub operator_signature: Option<OperatorSignature>,
}

impl From<interface::OperatorSignature> for OperatorSignature {
    fn from(p: interface::OperatorSignature) -> Self {
        Self {
            cose_sign1: p.cose_sign1,
        }
    }
}

impl From<OperatorSignature> for interface::OperatorSignature {
    fn from(value: OperatorSignature) -> Self {
        interface::OperatorSignature {
            cose_sign1: value.cose_sign1,
        }
    }
}

impl TryFrom<interface::EnvReport> for EnvReport {
    type Error = super::error::ConversionError;
    fn try_from(p: interface::EnvReport) -> Result<Self, Self::Error> {
        Ok(Self {
            attestation: p
                .attestation
                .map(AttestationBundle::from)
                .ok_or(Self::Error::MissingField("attestation".to_string()))?,
            operator_signature: p.operator_signature.map(OperatorSignature::from),
        })
    }
}

impl From<EnvReport> for interface::EnvReport {
    fn from(value: EnvReport) -> Self {
        interface::EnvReport {
            attestation: Some(value.attestation.into()),
            operator_signature: value.operator_signature.map(|sig| sig.into()),
        }
    }
}

impl From<&EnvReport> for interface::EnvReport {
    fn from(value: &EnvReport) -> Self {
        interface::EnvReport {
            attestation: Some(value.attestation.clone().into()),
            operator_signature: value.operator_signature.clone().map(|sig| sig.into()),
        }
    }
}
