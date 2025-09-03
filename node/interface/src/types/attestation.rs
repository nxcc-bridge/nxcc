use std::collections::HashMap;

use ed25519_dalek::{Signer, Verifier};
use serde::{Deserialize, Serialize};

use crate::proto::interface;

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

/// Alias for backward compatibility - will be removed once all code is updated
pub type StandardizedAttestationClaims = StandardizedClaims;

/// Result of attestation verification
#[derive(Debug)]
pub enum VerificationResult {
    /// Verification successful with extracted claims
    Verified(Box<StandardizedClaims>),
    /// Provider cannot handle this attestation type (try next provider)
    Unsupported,
    /// Verification failed definitively (attestation is invalid)
    Failed(String),
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
    /// The detached user data payload that was hashed and included in the quote.
    /// This is typically a serialized structure containing an ephemeral public key and freshness information.
    pub detached_userdata: Vec<u8>,
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
            detached_userdata: p.detached_userdata,
        }
    }
}

impl From<AttestationBundle> for interface::AttestationBundle {
    fn from(value: AttestationBundle) -> Self {
        Self {
            raw_attestation: Some(value.raw_attestation.into()),
            detached_userdata: value.detached_userdata,
        }
    }
}

impl From<&AttestationBundle> for interface::AttestationBundle {
    fn from(value: &AttestationBundle) -> Self {
        Self {
            raw_attestation: Some(value.raw_attestation.clone().into()),
            detached_userdata: value.detached_userdata.clone(),
        }
    }
}

/// Operator signature over attestation evidence using COSE_Sign1 format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorSignature {
    /// COSE_Sign1 signature (RFC 8152) over raw attestation evidence
    pub cose_sign1: Vec<u8>,
}

impl OperatorSignature {
    /// Create a new operator signature over raw attestation evidence using COSE-like format
    pub fn new(
        signing_key: &[u8],
        raw_attestation: &RawAttestation,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if signing_key.len() != 32 {
            return Err("Ed25519 signing key must be 32 bytes".into());
        }

        // Create Ed25519 signing key
        let signing_key_array: [u8; 32] = signing_key
            .try_into()
            .map_err(|_| "Signing key must be exactly 32 bytes")?;
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&signing_key_array);
        let public_key = signing_key.verifying_key();

        // Sign the raw attestation evidence directly
        let signature = signing_key.sign(&raw_attestation.evidence);

        // Create a COSE-like structure with signature info (foundation for future full COSE upgrade)
        let signature_data = std::collections::BTreeMap::from([
            (
                "alg".to_string(),
                ciborium::Value::Text("EdDSA".to_string()),
            ),
            (
                "sig".to_string(),
                ciborium::Value::Bytes(signature.to_bytes().to_vec()),
            ),
            (
                "key".to_string(),
                ciborium::Value::Bytes(public_key.to_bytes().to_vec()),
            ),
        ]);

        // Serialize to CBOR
        let mut cose_bytes = Vec::new();
        ciborium::into_writer(&signature_data, &mut cose_bytes)
            .map_err(|e| format!("Failed to serialize signature data: {}", e))?;

        Ok(Self {
            cose_sign1: cose_bytes,
        })
    }

    /// Verify the operator signature against raw attestation evidence
    pub fn verify(
        &self,
        raw_attestation: &RawAttestation,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Parse CBOR signature data
        let signature_data: std::collections::BTreeMap<String, ciborium::Value> =
            ciborium::from_reader(&self.cose_sign1[..])
                .map_err(|e| format!("Failed to parse signature data: {}", e))?;

        // Extract algorithm
        let alg = signature_data
            .get("alg")
            .and_then(|v| {
                if let ciborium::Value::Text(s) = v {
                    Some(s)
                } else {
                    None
                }
            })
            .ok_or("Missing or invalid algorithm")?;

        if alg != "EdDSA" {
            return Err(format!("Unsupported algorithm: {}", alg).into());
        }

        // Extract signature
        let sig_bytes = signature_data
            .get("sig")
            .and_then(|v| {
                if let ciborium::Value::Bytes(b) = v {
                    Some(b)
                } else {
                    None
                }
            })
            .ok_or("Missing or invalid signature")?;

        // Extract public key
        let key_bytes = signature_data
            .get("key")
            .and_then(|v| {
                if let ciborium::Value::Bytes(b) = v {
                    Some(b)
                } else {
                    None
                }
            })
            .ok_or("Missing or invalid public key")?;

        // Verify signature
        if key_bytes.len() != 32 {
            return Err("Ed25519 public key must be 32 bytes".into());
        }
        if sig_bytes.len() != 64 {
            return Err("Ed25519 signature must be 64 bytes".into());
        }

        let public_key_array: [u8; 32] = key_bytes
            .as_slice()
            .try_into()
            .map_err(|_| "Public key must be exactly 32 bytes")?;
        let signature_array: [u8; 64] = sig_bytes
            .as_slice()
            .try_into()
            .map_err(|_| "Signature must be exactly 64 bytes")?;

        let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&public_key_array)
            .map_err(|e| format!("Invalid Ed25519 public key: {}", e))?;
        let signature = ed25519_dalek::Signature::from_bytes(&signature_array);

        verifying_key
            .verify(&raw_attestation.evidence, &signature)
            .map_err(|e| format!("Signature verification failed: {}", e).into())
    }

    /// Extract the public key from the signature data
    pub fn extract_public_key(&self) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        // Parse CBOR signature data
        let signature_data: std::collections::BTreeMap<String, ciborium::Value> =
            ciborium::from_reader(&self.cose_sign1[..])
                .map_err(|e| format!("Failed to parse signature data: {}", e))?;

        // Extract public key
        let key_bytes = signature_data
            .get("key")
            .and_then(|v| {
                if let ciborium::Value::Bytes(b) = v {
                    Some(b.clone())
                } else {
                    None
                }
            })
            .ok_or("Missing or invalid public key")?;

        Ok(key_bytes)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operator_signature_creation_and_verification() {
        // Create a test signing key
        let signing_key = [0u8; 32]; // Simple test key

        // Create a mock raw attestation
        let raw_attestation = RawAttestation {
            platform_type: "test".to_string(),
            evidence: vec![1, 2, 3, 4, 5], // Simple test evidence
            certificates: None,
        };

        // Create operator signature
        let operator_signature = OperatorSignature::new(&signing_key, &raw_attestation)
            .expect("Failed to create operator signature");

        // Verify the signature
        assert!(
            operator_signature.verify(&raw_attestation).is_ok(),
            "Operator signature verification should succeed"
        );

        // Verify COSE-like structure
        assert!(!operator_signature.cose_sign1.is_empty());
        assert!(operator_signature.cose_sign1.len() > 10); // Should be a reasonable size for CBOR data

        // Verify we can parse the CBOR signature data
        let parsed: Result<std::collections::BTreeMap<String, ciborium::Value>, _> =
            ciborium::from_reader(&operator_signature.cose_sign1[..]);
        assert!(
            parsed.is_ok(),
            "Should be able to parse CBOR signature data"
        );
    }

    #[test]
    fn test_operator_signature_verification_fails_with_wrong_evidence() {
        let signing_key = [0u8; 32];

        let original_attestation = RawAttestation {
            platform_type: "test".to_string(),
            evidence: vec![1, 2, 3, 4, 5],
            certificates: None,
        };

        let modified_attestation = RawAttestation {
            platform_type: "test".to_string(),
            evidence: vec![1, 2, 3, 4, 6], // Different evidence
            certificates: None,
        };

        // Create signature with original evidence
        let operator_signature = OperatorSignature::new(&signing_key, &original_attestation)
            .expect("Failed to create operator signature");

        // Verify should fail with modified evidence
        assert!(
            operator_signature.verify(&modified_attestation).is_err(),
            "Operator signature verification should fail with modified evidence"
        );
    }

    #[test]
    fn test_operator_signature_invalid_key_size() {
        let invalid_key = [0u8; 16]; // Wrong size
        let raw_attestation = RawAttestation {
            platform_type: "test".to_string(),
            evidence: vec![1, 2, 3],
            certificates: None,
        };

        let result = OperatorSignature::new(&invalid_key, &raw_attestation);
        assert!(result.is_err(), "Should fail with invalid key size");
    }
}
