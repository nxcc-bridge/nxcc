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

/// EAT-compliant measurement entry following exact specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measurement {
    /// Hash value
    #[serde(rename = "val")]
    pub val: Vec<u8>,
    /// Hash algorithm: "sha-256", "sha-384", or "sha-512"
    #[serde(rename = "alg")]
    pub alg: String,
    /// Category: "boot", "firmware", "kernel", "initrd", "vmm", "application", "policy", etc.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub measurement_type: Option<String>,
    /// Vendor information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    /// Version information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// JWK structure for cnf claim (JSON-profile style)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Jwk {
    /// Key type: "EC", "RSA", "OKP"
    pub kty: String,
    /// Curve for EC/OKP keys: "P-256", "P-384", "P-521", "X25519", "Ed25519"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crv: Option<String>,
    /// X coordinate (for EC keys) or raw key (for OKP)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    /// Y coordinate (for EC keys)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<String>,
}

/// EAT confirmation claim for ephemeral key proof-of-possession
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConfirmationMethod {
    /// JSON-profile style
    Jwk { jwk: Jwk },
    /// COSE-profile style
    CoseKey { cose_key: Vec<u8> },
}

/// Measurement comparison result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasurementResult {
    /// Label of the reference check
    pub name: String,
    /// Pass/fail result
    pub result: bool,
    /// Reference identifier or version
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

/// Manifest reference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// URI to the manifest
    pub uri: String,
    /// Hash of the manifest payload
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<Vec<u8>>,
}

/// Standardized attestation claims following exact EAT/RATS specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandardizedClaims {
    // == Core freshness and context ==
    /// Issued-at time of the evidence production or verification moment
    #[serde(rename = "iat")]
    pub iat: u64,

    /// Verifier challenge to prevent replay (omit if no challenge used)
    #[serde(rename = "eat_nonce", skip_serializing_if = "Option::is_none")]
    pub eat_nonce: Option<Vec<u8>>,

    // == Identity and provenance ==
    /// Stable device/realm identity (omit if not available)
    #[serde(rename = "ueid", skip_serializing_if = "Option::is_none")]
    pub ueid: Option<Vec<u8>>,

    /// Semi-permanent or auxiliary IDs
    #[serde(rename = "sueids", skip_serializing_if = "Option::is_none")]
    pub sueids: Option<Vec<Vec<u8>>>,

    /// Manufacturer identifier
    #[serde(rename = "oemid", skip_serializing_if = "Option::is_none")]
    pub oemid: Option<String>,

    /// Hardware model descriptor
    #[serde(rename = "hwmodel", skip_serializing_if = "Option::is_none")]
    pub hwmodel: Option<String>,

    /// Hardware/firmware version string
    #[serde(rename = "hwversion", skip_serializing_if = "Option::is_none")]
    pub hwversion: Option<String>,

    // == Debug and boot status ==
    /// Debug/production mode: 0=debug disabled (production), 1=debug disabled since boot,
    /// 2=debug disabled permanently, 3=debug fully and permanently disabled, 4=debug enabled
    #[serde(rename = "dbgstat")]
    pub dbgstat: u8,

    /// OEM-authorized secure boot active
    #[serde(rename = "oemboot", skip_serializing_if = "Option::is_none")]
    pub oemboot: Option<bool>,

    // == Software identity ==
    /// Product or component name of the attested software root
    #[serde(rename = "swname", skip_serializing_if = "Option::is_none")]
    pub swname: Option<String>,

    /// Version string of the attested software root
    #[serde(rename = "swversion", skip_serializing_if = "Option::is_none")]
    pub swversion: Option<String>,

    /// References to accepted software manifests or SBOMs
    #[serde(rename = "manifests", skip_serializing_if = "Option::is_none")]
    pub manifests: Option<Vec<Manifest>>,

    // == Measurements and results ==
    /// Cryptographic measurements relevant to trust decisions (required - at least one)
    #[serde(rename = "measurements")]
    pub measurements: Vec<Measurement>,

    /// Comparison outcomes against known-good references
    #[serde(rename = "measres", skip_serializing_if = "Option::is_none")]
    pub measres: Option<Vec<MeasurementResult>>,

    // == Execution structure breakdown ==
    /// Hierarchical submodules for component-scoped claims
    #[serde(rename = "submods", skip_serializing_if = "Option::is_none")]
    pub submods: Option<HashMap<String, StandardizedClaims>>,

    // == Key binding ==
    /// Proof-of-possession key bound to this attested state
    #[serde(rename = "cnf", skip_serializing_if = "Option::is_none")]
    pub cnf: Option<ConfirmationMethod>,

    /// Intended use for the token/key (typically 5 for proof-of-possession)
    #[serde(rename = "intuse", skip_serializing_if = "Option::is_none")]
    pub intuse: Option<u8>,

    // == Lifecycle freshness ==
    /// Seconds since last boot according to the attested environment
    #[serde(rename = "uptime", skip_serializing_if = "Option::is_none")]
    pub uptime: Option<u64>,

    /// Number of boots observed
    #[serde(rename = "bootcount", skip_serializing_if = "Option::is_none")]
    pub bootcount: Option<u64>,

    /// Per-boot unique random seed to distinguish boot instances
    #[serde(rename = "bootseed", skip_serializing_if = "Option::is_none")]
    pub bootseed: Option<Vec<u8>>,

    // == Profile selection ==
    /// URI-like identifier of the interpretation profile for platform specifics (required)
    #[serde(rename = "eat_profile")]
    pub eat_profile: String,

    // == Assurance artifacts ==
    /// Declarations of conformity, certifications, or assurance statements
    #[serde(rename = "dloas", skip_serializing_if = "Option::is_none")]
    pub dloas: Option<Vec<String>>,
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
