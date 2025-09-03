use serde::{Deserialize, Serialize};

use crate::proto::interface;

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
