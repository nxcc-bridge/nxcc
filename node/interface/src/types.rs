use std::{collections::HashMap, fmt};

use alloy_primitives::{Address, B256, U256};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;
use thiserror::Error;
use url::Url;

use crate::{
    gateway,
    proto::{enclave, interface},
};

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

#[derive(Debug, Error)]
pub enum ConversionError {
    #[error("Missing field in protobuf message: {0}")]
    MissingField(String),
    #[error("Invalid value for field {field}: {message}")]
    InvalidValue { field: String, message: String },
    #[error("JSON deserialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Base64 decoding error: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("Invalid DSSE payload type: expected {expected}, got {got}")]
    InvalidDssePayloadType { expected: String, got: String },
    #[error("Invalid byte slice length for {name}: expected {expected}, got {got}")]
    InvalidSliceLength {
        name: String,
        expected: usize,
        got: usize,
    },
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
            let mut hasher = sha2::Sha256::new();
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
            let mut hasher = sha2::Sha256::new();
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
            let mut hasher = sha2::Sha256::new();
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

/// Identifies a chain either by its numeric ID or by a custom gateway URL.
/// Custom gateways are treated as separate chains even if they have the same chain_id,
/// since we cannot verify that a custom gateway actually represents the intended chain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(untagged)]
pub enum ChainIdentifier {
    /// Standard chain identified by its numeric chain ID
    ChainId(u64),
    /// Custom chain identified by a gateway URL
    GatewayUrl(Url),
}

impl ChainIdentifier {
    /// Returns the chain ID if this is a ChainId variant, otherwise returns None
    pub fn chain_id(&self) -> Option<u64> {
        match self {
            ChainIdentifier::ChainId(id) => Some(*id),
            ChainIdentifier::GatewayUrl(_) => None,
        }
    }

    /// Returns the gateway URL if this is a GatewayUrl variant, otherwise returns None
    pub fn gateway_url(&self) -> Option<&Url> {
        match self {
            ChainIdentifier::ChainId(_) => None,
            ChainIdentifier::GatewayUrl(url) => Some(url),
        }
    }
}

impl Default for ChainIdentifier {
    fn default() -> Self {
        ChainIdentifier::ChainId(0)
    }
}

impl fmt::Display for ChainIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChainIdentifier::ChainId(id) => write!(f, "{}", id),
            ChainIdentifier::GatewayUrl(url) => write!(f, "{}", url),
        }
    }
}

impl TryFrom<interface::ChainIdentifier> for ChainIdentifier {
    type Error = ConversionError;
    fn try_from(p: interface::ChainIdentifier) -> Result<Self, Self::Error> {
        match p.identifier {
            Some(interface::chain_identifier::Identifier::ChainId(id)) => {
                Ok(ChainIdentifier::ChainId(id))
            }
            Some(interface::chain_identifier::Identifier::GatewayUrl(url)) => {
                let parsed_url = Url::parse(&url).map_err(|e| ConversionError::InvalidValue {
                    field: "gateway_url".to_string(),
                    message: e.to_string(),
                })?;
                Ok(ChainIdentifier::GatewayUrl(parsed_url))
            }
            None => Err(ConversionError::MissingField("identifier".to_string())),
        }
    }
}

impl From<ChainIdentifier> for interface::ChainIdentifier {
    fn from(value: ChainIdentifier) -> Self {
        let identifier = match value {
            ChainIdentifier::ChainId(id) => interface::chain_identifier::Identifier::ChainId(id),
            ChainIdentifier::GatewayUrl(url) => {
                interface::chain_identifier::Identifier::GatewayUrl(url.to_string())
            }
        };
        interface::ChainIdentifier {
            identifier: Some(identifier),
        }
    }
}

impl From<&ChainIdentifier> for interface::ChainIdentifier {
    fn from(value: &ChainIdentifier) -> Self {
        let identifier = match value {
            ChainIdentifier::ChainId(id) => interface::chain_identifier::Identifier::ChainId(*id),
            ChainIdentifier::GatewayUrl(url) => {
                interface::chain_identifier::Identifier::GatewayUrl(url.to_string())
            }
        };
        interface::ChainIdentifier {
            identifier: Some(identifier),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretId {
    pub chain: ChainIdentifier,
    pub identity_address: Address,
    pub identity_id: U256,
}

impl TryFrom<interface::SecretIdentifier> for SecretId {
    type Error = ConversionError;
    fn try_from(p: interface::SecretIdentifier) -> Result<Self, Self::Error> {
        Ok(Self {
            chain: p
                .chain
                .ok_or(ConversionError::MissingField("chain".to_string()))?
                .try_into()?,
            identity_address: p.identity_address.parse().map_err(
                |e: alloy_primitives::hex::FromHexError| ConversionError::InvalidValue {
                    field: "identity_address".to_string(),
                    message: e.to_string(),
                },
            )?,
            identity_id: p.identity_id.parse().map_err(
                |e: alloy_primitives::ruint::ParseError| ConversionError::InvalidValue {
                    field: "identity_id".to_string(),
                    message: e.to_string(),
                },
            )?,
        })
    }
}

impl From<SecretId> for interface::SecretIdentifier {
    fn from(value: SecretId) -> Self {
        interface::SecretIdentifier {
            chain: Some(value.chain.into()),
            identity_address: format!("{:#x}", value.identity_address),
            identity_id: value.identity_id.to_string(),
        }
    }
}

impl From<&SecretId> for interface::SecretIdentifier {
    fn from(value: &SecretId) -> Self {
        interface::SecretIdentifier {
            chain: Some(value.chain.clone().into()),
            identity_address: format!("{:#x}", value.identity_address),
            identity_id: value.identity_id.to_string(),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ConsumerInfo {
    pub bundle_hash: Vec<u8>,
    pub signature: Vec<u8>,
}

impl From<interface::ConsumerInfo> for ConsumerInfo {
    fn from(p: interface::ConsumerInfo) -> Self {
        Self {
            bundle_hash: p.bundle_hash,
            signature: p.signature,
        }
    }
}

impl From<ConsumerInfo> for interface::ConsumerInfo {
    fn from(value: ConsumerInfo) -> Self {
        interface::ConsumerInfo {
            bundle_hash: value.bundle_hash,
            signature: value.signature,
        }
    }
}

impl From<&ConsumerInfo> for interface::ConsumerInfo {
    fn from(value: &ConsumerInfo) -> Self {
        interface::ConsumerInfo {
            bundle_hash: value.bundle_hash.clone(),
            signature: value.signature.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretRequest {
    pub secret_id: SecretId,
    pub consumer: ConsumerInfo,
}

impl TryFrom<interface::SecretRequest> for SecretRequest {
    type Error = ConversionError;
    fn try_from(p: interface::SecretRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            secret_id: p
                .secret_id
                .ok_or(ConversionError::MissingField("secret_id".to_string()))?
                .try_into()?,
            consumer: p
                .consumer
                .map(ConsumerInfo::from)
                .ok_or(ConversionError::MissingField("consumer".to_string()))?,
        })
    }
}

impl From<SecretRequest> for interface::SecretRequest {
    fn from(value: SecretRequest) -> Self {
        interface::SecretRequest {
            secret_id: Some(value.secret_id.into()),
            consumer: Some(value.consumer.into()),
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
    type Error = ConversionError;
    fn try_from(p: interface::EnvReport) -> Result<Self, Self::Error> {
        Ok(Self {
            attestation: p
                .attestation
                .map(AttestationBundle::from)
                .ok_or(ConversionError::MissingField("attestation".to_string()))?,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretsBox {
    pub encrypted_payload: Vec<u8>,
    pub sender_public_key: Vec<u8>, // This is the sender's *ephemeral key exchange* public key
    pub alg: String,
    pub contained_secret_ids: Vec<SecretId>,
}

impl SecretsBox {
    pub fn new_empty() -> Self {
        Self {
            encrypted_payload: vec![],
            sender_public_key: vec![],
            alg: "X25519_AES-GCM-SIV_Ed25519".to_string(), // Default algorithm
            contained_secret_ids: vec![],
        }
    }

    pub fn calculate_binding_hash(&self) -> [u8; 32] {
        use sha2::Digest as _;
        let mut hasher = sha2::Sha256::default();
        hasher.update(&self.encrypted_payload);
        hasher.update(&self.sender_public_key);
        hasher.update(self.alg.as_bytes());
        // Hash contained IDs consistently (sort them first)
        let mut sorted_ids = self.contained_secret_ids.clone();
        sorted_ids.sort();
        let mut id_bytes = Vec::new();
        ciborium::into_writer(&sorted_ids, &mut id_bytes).unwrap();
        hasher.update(&id_bytes);
        hasher.finalize().into()
    }
}

impl TryFrom<interface::SecretsBox> for SecretsBox {
    type Error = ConversionError;
    fn try_from(p: interface::SecretsBox) -> Result<Self, Self::Error> {
        Ok(Self {
            encrypted_payload: p.encrypted_payload,
            sender_public_key: p.sender_public_key,
            alg: p.alg,
            contained_secret_ids: p
                .contained_secret_ids
                .into_iter()
                .map(SecretId::try_from)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

impl From<SecretsBox> for interface::SecretsBox {
    fn from(value: SecretsBox) -> Self {
        interface::SecretsBox {
            encrypted_payload: value.encrypted_payload,
            sender_public_key: value.sender_public_key,
            alg: value.alg,
            contained_secret_ids: value
                .contained_secret_ids
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}

impl From<&SecretsBox> for interface::SecretsBox {
    fn from(value: &SecretsBox) -> Self {
        interface::SecretsBox {
            encrypted_payload: value.encrypted_payload.clone(),
            sender_public_key: value.sender_public_key.clone(),
            alg: value.alg.clone(),
            contained_secret_ids: value
                .contained_secret_ids
                .iter()
                .cloned()
                .map(Into::into)
                .collect(),
        }
    }
}

/// Sanitized attestation bundle for policy workers, excluding system userdata
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyAttestationBundle {
    /// Platform type for the attestation
    pub platform_type: String,
    /// Raw attestation evidence
    pub evidence: Vec<u8>,
    /// User-provided data only (no ephemeral keys, no block hashes)
    pub user_data: Vec<u8>,
}

/// Sanitized environment report for policy workers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyEnvReport {
    pub attestation: PolicyAttestationBundle,
    pub operator_signature: Option<OperatorSignature>,
}

impl PolicyEnvReport {
    /// Create a sanitized policy environment report from a full environment report
    /// This removes system userdata (ephemeral keys, block hashes) while preserving
    /// user data and platform measurements for policy decisions
    pub fn from_env_report(env_report: &EnvReport, user_provided_data: Vec<u8>) -> Self {
        Self {
            attestation: PolicyAttestationBundle {
                platform_type: env_report.attestation.raw_attestation.platform_type.clone(),
                evidence: env_report.attestation.raw_attestation.evidence.clone(),
                user_data: user_provided_data,
            },
            operator_signature: env_report.operator_signature.clone(),
        }
    }

    /// Convert back to a full EnvReport for protobuf serialization
    /// Note: This reconstructs minimal system fields for compatibility
    pub fn to_env_report(&self) -> EnvReport {
        EnvReport {
            attestation: AttestationBundle {
                raw_attestation: RawAttestation {
                    platform_type: self.attestation.platform_type.clone(),
                    evidence: self.attestation.evidence.clone(),
                    certificates: None, // Empty - system data removed
                },
                user_data_binding: UserDataBinding {
                    original_data: self.attestation.user_data.clone(),
                    embedded_hash: self.attestation.user_data.clone(),
                    was_hashed: false,
                    ephemeral_key_len: 0, // Empty - system data removed
                },
                block_hashes: Vec::new(), // Empty - system data removed
            },
            operator_signature: self.operator_signature.clone(),
        }
    }
}

/// A request for the policy runner that references multiple secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyExecutionRequest {
    pub secret_ids: Vec<SecretId>,
    pub consumer: ConsumerInfo,
    pub env_report: EnvReport, // The EnvReport of the entity being evaluated
    /// Standardized attestation claims extracted from the verified env_report
    /// These are available when the attestation system successfully verifies the report
    pub attestation_claims: Option<StandardizedAttestationClaims>,
}

impl PolicyExecutionRequest {
    /// Create a sanitized version for policy worker execution
    /// This removes system userdata while preserving user data and claims
    pub fn for_policy_worker(
        &self,
        user_provided_data: Vec<u8>,
    ) -> PolicyExecutionContextForWorker {
        PolicyExecutionContextForWorker {
            secret_ids: self.secret_ids.clone(),
            consumer: self.consumer.clone(),
            env_report: PolicyEnvReport::from_env_report(&self.env_report, user_provided_data),
            attestation_claims: self.attestation_claims.clone(),
        }
    }
}

/// Sanitized context sent to policy workers (excludes system userdata)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyExecutionContextForWorker {
    pub secret_ids: Vec<SecretId>,
    pub consumer: ConsumerInfo,
    pub env_report: PolicyEnvReport, // Sanitized EnvReport without system userdata
    /// Standardized attestation claims extracted from the verified env_report
    pub attestation_claims: Option<StandardizedAttestationClaims>,
}

impl TryFrom<interface::PolicyExecutionRequest> for PolicyExecutionRequest {
    type Error = ConversionError;
    fn try_from(p: interface::PolicyExecutionRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            secret_ids: p
                .secret_ids
                .into_iter()
                .map(SecretId::try_from)
                .collect::<Result<Vec<_>, _>>()?,
            consumer: p
                .consumer
                .map(ConsumerInfo::from)
                .ok_or(ConversionError::MissingField("consumer".to_string()))?,
            env_report: p
                .env_report
                .ok_or(ConversionError::MissingField("env_report".to_string()))?
                .try_into()?,
            attestation_claims: None, // Populated by enclave after verification
        })
    }
}

impl From<PolicyExecutionRequest> for interface::PolicyExecutionRequest {
    fn from(value: PolicyExecutionRequest) -> Self {
        interface::PolicyExecutionRequest {
            secret_ids: value.secret_ids.into_iter().map(Into::into).collect(),
            consumer: Some(value.consumer.into()),
            env_report: Some(value.env_report.into()),
        }
    }
}

impl From<&PolicyExecutionRequest> for interface::PolicyExecutionRequest {
    fn from(value: &PolicyExecutionRequest) -> Self {
        interface::PolicyExecutionRequest {
            secret_ids: value.secret_ids.iter().cloned().map(Into::into).collect(),
            consumer: Some(value.consumer.clone().into()),
            env_report: Some(value.env_report.clone().into()),
        }
    }
}

/// The runner's final judgment about a request. This structure is used internally within the enclave
/// between the runner and secrets service. It's distinct from the proto message used for gRPC transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyExecutionReport {
    pub request: PolicyExecutionRequest,
    pub decision: bool,
    pub timestamp: u64, // Unix timestamp
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TcpAddress {
    pub host: String,
    pub port: u32,
}

impl From<enclave::TcpAddress> for TcpAddress {
    fn from(p: enclave::TcpAddress) -> Self {
        Self {
            host: p.host,
            port: p.port,
        }
    }
}

impl From<TcpAddress> for enclave::TcpAddress {
    fn from(value: TcpAddress) -> Self {
        enclave::TcpAddress {
            host: value.host,
            port: value.port,
        }
    }
}

impl From<&TcpAddress> for enclave::TcpAddress {
    fn from(value: &TcpAddress) -> Self {
        enclave::TcpAddress {
            host: value.host.clone(),
            port: value.port,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UdsAddress {
    pub path: String,
}

impl From<enclave::UdsAddress> for UdsAddress {
    fn from(p: enclave::UdsAddress) -> Self {
        Self { path: p.path }
    }
}

impl From<UdsAddress> for enclave::UdsAddress {
    fn from(value: UdsAddress) -> Self {
        enclave::UdsAddress { path: value.path }
    }
}

impl From<&UdsAddress> for enclave::UdsAddress {
    fn from(value: &UdsAddress) -> Self {
        enclave::UdsAddress {
            path: value.path.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VsockAddress {
    pub cid: u32,
    pub port: u32,
}

impl From<enclave::VsockAddress> for VsockAddress {
    fn from(p: enclave::VsockAddress) -> Self {
        Self {
            cid: p.cid,
            port: p.port,
        }
    }
}

impl From<VsockAddress> for enclave::VsockAddress {
    fn from(value: VsockAddress) -> Self {
        enclave::VsockAddress {
            cid: value.cid,
            port: value.port,
        }
    }
}

impl From<&VsockAddress> for enclave::VsockAddress {
    fn from(value: &VsockAddress) -> Self {
        enclave::VsockAddress {
            cid: value.cid,
            port: value.port,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VmAddress {
    Tcp(TcpAddress),
    Uds(UdsAddress),
    Vsock(VsockAddress),
}

impl TryFrom<enclave::VmAddress> for VmAddress {
    type Error = ConversionError;
    fn try_from(p: enclave::VmAddress) -> Result<Self, Self::Error> {
        match p.address_type {
            Some(enclave::vm_address::AddressType::Tcp(tcp)) => {
                Ok(VmAddress::Tcp(TcpAddress::from(tcp)))
            }
            Some(enclave::vm_address::AddressType::Uds(uds)) => {
                Ok(VmAddress::Uds(UdsAddress::from(uds)))
            }
            Some(enclave::vm_address::AddressType::Vsock(vsock)) => {
                Ok(VmAddress::Vsock(VsockAddress::from(vsock)))
            }
            None => Err(ConversionError::MissingField("address_type".to_string())),
        }
    }
}

impl From<VmAddress> for enclave::VmAddress {
    fn from(value: VmAddress) -> Self {
        let address_type = match value {
            VmAddress::Tcp(tcp) => enclave::vm_address::AddressType::Tcp(tcp.into()),
            VmAddress::Uds(uds) => enclave::vm_address::AddressType::Uds(uds.into()),
            VmAddress::Vsock(vsock) => enclave::vm_address::AddressType::Vsock(vsock.into()),
        };
        enclave::VmAddress {
            address_type: Some(address_type),
        }
    }
}

impl From<&VmAddress> for enclave::VmAddress {
    fn from(value: &VmAddress) -> Self {
        let address_type = match value {
            VmAddress::Tcp(tcp) => enclave::vm_address::AddressType::Tcp(tcp.into()),
            VmAddress::Uds(uds) => enclave::vm_address::AddressType::Uds(uds.into()),
            VmAddress::Vsock(vsock) => enclave::vm_address::AddressType::Vsock(vsock.into()),
        };
        enclave::VmAddress {
            address_type: Some(address_type),
        }
    }
}

/// Describes how to locate a `WorkerBundle`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerBundlePointer {
    /// The location of the `WorkerBundle`. May be a data URL for direct embedding
    /// or other schemes like http, ipfs, etc.
    pub source: url::Url,
    /// The expected SHA-512 hash of the `WorkerBundle`'s COSE envelope.
    /// Useful for mutable source URLs or content integrity checks.
    pub hash: Option<Vec<u8>>,
}

/// Describes a worker (or policy) and its inputs.
/// This is what is pointed to by the on-chain root of trust where policies are concerned.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkerManifest {
    /// An authenticated pointer to a `WorkerBundle`.
    pub bundle: WorkerBundlePointer,
    /// The set of identities that the worker needs for execution.
    /// These will be bound by the VM into the worker.
    /// Policy workers are not allowed to request identities.
    pub identities: Vec<(SecretId, String)>,
    /// Arbitrary data passed by the creator of the worker manifest.
    /// Untrusted from the perspective of the nXCC system.
    pub userdata: HashMap<String, Value>,
}

/// Represents a signature entry in a DSSE envelope.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DsseSignatureEntry {
    #[serde(rename = "keyid", skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    pub sig: String, // base64 encoded
}

/// Represents a DSSE (Dead Simple Signing Envelope).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DsseEnvelope {
    pub payload: String, // base64 encoded
    #[serde(rename = "payloadType")]
    pub payload_type: String,
    pub signatures: Vec<DsseSignatureEntry>,
}

/// The inner payload of a `WorkerBundle` that gets signed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerBundlePayload {
    /// The VM in which the worker must execute (e.g., "nxcc/workerd").
    pub vm: String,
    /// The executable code (e.g., JS, Python, WASM).
    #[serde(with = "serde_base64")]
    pub executable: Vec<u8>,
    /// Arbitrary metadata added by the publisher. Not interpreted by nXCC.
    pub metadata: HashMap<String, String>,
}

/// An executable `WorkerBundlePayload` wrapped in a DSSE envelope.
/// This struct holds the DSSE envelope as raw JSON bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerBundle(pub Vec<u8>);

/// The IANA media type for the WorkerBundlePayload when wrapped in DSSE.
pub const DSSE_WORKER_BUNDLE_PAYLOAD_TYPE: &str =
    "application/vnd.nxcc.workerbundlepayload.v1+json";

/// The IANA media type for the WorkOrderPayload when wrapped in DSSE.
pub const DSSE_WORK_ORDER_PAYLOAD_TYPE: &str = "application/vnd.nxcc.workorderpayload.v1+json";

impl WorkerBundle {
    /// Parses the DSSE envelope from the raw bytes of the WorkerBundle.
    fn dsse_envelope(&self) -> Result<DsseEnvelope, ConversionError> {
        serde_json::from_slice(&self.0).map_err(Into::into)
    }

    /// Retrieves the `WorkerBundlePayload` from the DSSE envelope.
    pub fn payload(&self) -> Result<WorkerBundlePayload, ConversionError> {
        let envelope = self.dsse_envelope()?;
        if envelope.payload_type != DSSE_WORKER_BUNDLE_PAYLOAD_TYPE {
            return Err(ConversionError::InvalidDssePayloadType {
                expected: DSSE_WORKER_BUNDLE_PAYLOAD_TYPE.to_string(),
                got: envelope.payload_type,
            });
        }
        let payload_bytes = BASE64_STANDARD.decode(&envelope.payload)?;
        serde_json::from_slice(&payload_bytes[..]).map_err(Into::into)
    }

    /// Calculates the SHA512 hash of the encoded `WorkerBundlePayload`.
    /// This hash is used for `ConsumerInfo.bundle_hash`.
    // TODO: remove this in favor of having the enclave verify the signer or having the hash of the executable be part of the signed data or something. right now it's totally broken, as the consumer cannot be verified with all of the arbitrary metadata in it
    pub fn hash_signed_payload(&self) -> Result<Vec<u8>, ConversionError> {
        use sha2::{Digest, Sha512};
        let payload_struct = self.payload()?;
        let payload_bytes = serde_json::to_vec(&payload_struct)?;
        Ok(Sha512::digest(payload_bytes).to_vec())
    }

    /// Extracts the first signature from the DSSE envelope.
    pub fn get_dsse_signature(&self) -> Result<Vec<u8>, ConversionError> {
        let envelope = self.dsse_envelope()?;
        if envelope.signatures.is_empty() {
            return Err(ConversionError::InvalidValue {
                field: "signatures".to_string(),
                message: "DSSE envelope has no signatures".to_string(),
            });
        }
        // Return the raw bytes of the first signature
        BASE64_STANDARD
            .decode(&envelope.signatures[0].sig)
            .map_err(Into::into)
    }
}

mod serde_base64 {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&BASE64_STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        BASE64_STANDARD
            .decode(s.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}

/// A structure combining a policy's `WorkerManifest` and its resolved `WorkerBundle`.
/// This replaces the old `PolicyBundle` for policy execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullPolicyPackage {
    pub manifest: WorkerManifest,
    pub bundle: WorkerBundle,
}

/// The inner payload of a `WorkOrder` that gets signed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkOrderPayload {
    /// An arbitrary identifier for this work order.
    /// Useful for debugging and ensuring uniqueness when broadcasting over the p2p network.
    pub id: String,
    /// The worker to run, and its inputs and configuration.
    pub worker: WorkerManifest,
    /// Event listeners for the daemon to set up. The daemon will invoke the worker when they happen.
    pub events: Vec<WorkerEvent>,
}

/// An event that can trigger a worker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerEvent {
    /// The name of the function in the worker to handle this event.
    pub handler: String,
    #[serde(flatten)]
    pub kind: WorkerEventKind,
}

/// The kind of an event that can trigger a worker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum WorkerEventKind {
    /// Runs whenever the worker is freshly started.
    Launch,
    /// Describes a Web3 event subscription.
    Web3Event(Web3Event),
    /// Indicates the worker can handle HTTP requests.
    HttpRequest,
    /// Describes a scheduled event with timing configuration.
    Scheduled(Schedule),
}

/// Top-level schedule config using "first-match" deserialization.
/// Put `Schedule::Rate` first so missing/unknown `mode` falls back to rate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Schedule {
    Rate(RateMode),
    // Add other modes later (e.g., Calendar) after Rate to keep Rate the default.
}

/// Mode discriminator.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Rate,
}

/// Catch-up strategy for late ticks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CatchUp {
    /// Drop overdue ticks and schedule only the next on-time one.
    #[default]
    Skip,
    /// Merge all missed ticks into a single immediate tick.
    Coalesce,
    /// Enqueue missed ticks for the handler to process.
    Queue,
}

/// Optional policy tuning. All fields optional.
/// Omitted => sane best-effort.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Policy {
    /// What to do if a tick is late. Default: `skip`.
    #[serde(default)]
    pub catch_up: CatchUp,
    /// Drop a tick if it fires later than this many ms. Omit to disable.
    #[serde(default)]
    pub max_lateness_ms: Option<u64>,
    /// Used for monitoring/SLOs only. Omit if not needed.
    #[serde(default)]
    pub jitter_budget_ms: Option<u64>,
}

/// High-resolution, monotonic, rate-based schedule.
///
/// Required:
/// - `period_ms`
///
/// Defaults:
/// - `mode` = "rate"
/// - `phase_ms` = 0
/// - `start_at` = immediate (None)
/// - `end_at` = never (None)
/// - `max_occurrences` = infinite (None)
/// - `policy` = best-effort (None)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RateMode {
    /// Optional discriminator. Defaults to `"rate"`. May be omitted.
    #[serde(default)]
    pub mode: Mode,

    /// Period between ticks in milliseconds. Required.
    pub period_ms: u64,

    /// Phase offset from the start boundary in milliseconds. Default 0.
    #[serde(default)]
    pub phase_ms: u64,

    /// When to start. `None` means start immediately. Default None.
    #[serde(default)]
    pub start_at: Option<DateTime<Utc>>,

    /// When to stop. `None` means never. Default None.
    #[serde(default)]
    pub end_at: Option<DateTime<Utc>>,

    /// Max number of ticks. `None` means infinite. Default None.
    #[serde(default)]
    pub max_occurrences: Option<u64>,

    /// Optional policy. Omit for best-effort defaults.
    #[serde(default)]
    pub policy: Option<Policy>,
}

impl RateMode {
    /// Helper: minimal constructor with only the required field.
    pub fn new(period_ms: u64) -> Self {
        Self {
            mode: Mode::Rate,
            period_ms,
            phase_ms: 0,
            start_at: None,
            end_at: None,
            max_occurrences: None,
            policy: None,
        }
    }
}

/// Configuration for a Web3 event listener, mirroring Alloy's Filter structure.
/// This is the Rust representation of the JSON `Web3Event` type in the work order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Web3Event {
    pub chain: ChainIdentifier,
    /// Contract addresses to filter for.
    /// - `None` or empty `Vec` typically means wildcard (any address), depending on RPC interpretation.
    ///   Our interpretation: empty Vec means wildcard.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub address: Vec<Address>,
    /// Topic filters. A `Vec<Vec<B256>>`.
    /// Outer Vec corresponds to topic0, topic1, etc. Max 4.
    /// Inner Vec contains alternative values for that topic.
    /// - `topics: []` (empty outer Vec) -> wildcard for all topic positions.
    /// - `topics: [vec![]]` -> topic0 must be empty (FilterSet::Values([])), rest wildcard.
    /// - `topics: [vec!["0x...".parse().unwrap()]]` -> topic0 specific, rest wildcard.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topics: Vec<Vec<B256>>,
    /// Explicit WebSocket gateways to use instead of the default for this chain.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gateways: Vec<String>,
}

impl TryFrom<interface::Web3EventConfig> for Web3Event {
    type Error = ConversionError;
    fn try_from(p: interface::Web3EventConfig) -> Result<Self, Self::Error> {
        Ok(Self {
            chain: p
                .chain
                .ok_or(ConversionError::MissingField("chain".to_string()))?
                .try_into()?,
            address: p
                .address
                .into_iter()
                .map(|s| {
                    s.parse().map_err(|e| ConversionError::InvalidValue {
                        field: "address".to_string(),
                        message: format!("failed to parse address '{}': {}", s, e),
                    })
                })
                .collect::<Result<_, _>>()?,
            topics: p
                .topics
                .into_iter()
                .map(|topic_filter| {
                    topic_filter
                        .values
                        .into_iter()
                        .map(|s| {
                            s.parse().map_err(|e| ConversionError::InvalidValue {
                                field: "topics".to_string(),
                                message: format!("failed to parse topic '{}': {}", s, e),
                            })
                        })
                        .collect()
                })
                .collect::<Result<_, _>>()?,
            gateways: p.gateways,
        })
    }
}

impl From<Web3Event> for interface::Web3EventConfig {
    fn from(value: Web3Event) -> Self {
        interface::Web3EventConfig {
            chain: Some(value.chain.into()),
            address: value.address.iter().map(|a| format!("{a:#x}")).collect(),
            topics: value
                .topics
                .iter()
                .map(|topic_values| interface::ProtoTopicFilter {
                    values: topic_values.iter().map(|t| format!("{t:#x}")).collect(),
                })
                .collect(),
            gateways: value.gateways,
        }
    }
}

// --- Event Delivery Types ---

/// Represents a Web3 log event, mirroring `alloy_rpc_types::Log`.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Web3Log {
    pub address: Address,
    pub topics: Vec<B256>,
    pub data: alloy_primitives::Bytes,
    pub block_hash: Option<B256>,
    pub block_number: Option<u64>,
    pub transaction_hash: Option<B256>,
    pub transaction_index: Option<u64>, // usize in alloy, u64 is fine for proto
    pub log_index: Option<u64>,         // usize in alloy, u64 is fine for proto
    pub removed: bool,
}

impl From<alloy_rpc_types::Log> for Web3Log {
    fn from(log: alloy_rpc_types::Log) -> Self {
        Self {
            address: log.inner.address,
            topics: log.inner.topics().to_vec(), // Access topics through TopicList
            data: log.inner.data.data,
            block_hash: log.block_hash,
            block_number: log.block_number,
            transaction_hash: log.transaction_hash,
            transaction_index: log.transaction_index,
            log_index: log.log_index,
            removed: log.removed,
        }
    }
}

impl From<Web3Log> for interface::Web3Log {
    fn from(log: Web3Log) -> Self {
        Self {
            address: log.address.to_vec(),
            topics: log.topics.iter().map(|t| t.to_vec()).collect(),
            data: log.data.to_vec(),
            block_hash: log.block_hash.map_or_else(Vec::new, |h| h.to_vec()),
            block_number: log.block_number.unwrap_or(0),
            transaction_hash: log.transaction_hash.map_or_else(Vec::new, |h| h.to_vec()),
            transaction_index: log.transaction_index.unwrap_or(0),
            log_index: log.log_index.unwrap_or(0),
            removed: log.removed,
        }
    }
}

impl TryFrom<interface::Web3Log> for Web3Log {
    type Error = ConversionError;
    fn try_from(p_log: interface::Web3Log) -> Result<Self, Self::Error> {
        let address = Address::try_from(p_log.address.as_slice()).map_err(|e| {
            ConversionError::InvalidValue {
                field: "address".to_string(),
                message: e.to_string(),
            }
        })?;
        let topics = p_log
            .topics
            .into_iter()
            .map(|b| {
                B256::try_from(b.as_slice()).map_err(|e| ConversionError::InvalidValue {
                    field: "topics".to_string(),
                    message: e.to_string(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let block_hash = if p_log.block_hash.is_empty() {
            None
        } else {
            Some(B256::try_from(p_log.block_hash.as_slice()).map_err(|e| {
                ConversionError::InvalidValue {
                    field: "block_hash".to_string(),
                    message: e.to_string(),
                }
            })?)
        };
        let transaction_hash = if p_log.transaction_hash.is_empty() {
            None
        } else {
            Some(
                B256::try_from(p_log.transaction_hash.as_slice()).map_err(|e| {
                    ConversionError::InvalidValue {
                        field: "transaction_hash".to_string(),
                        message: e.to_string(),
                    }
                })?,
            )
        };

        Ok(Self {
            address,
            topics,
            data: p_log.data.into(),
            block_hash,
            block_number: if p_log.block_number == 0 && p_log.block_hash.is_empty() {
                None
            } else {
                Some(p_log.block_number)
            },
            transaction_hash,
            transaction_index: if p_log.transaction_index == 0 && p_log.transaction_hash.is_empty()
            {
                None
            } else {
                Some(p_log.transaction_index)
            },
            log_index: if p_log.log_index == 0 && p_log.transaction_hash.is_empty() {
                None
            } else {
                Some(p_log.log_index)
            },
            removed: p_log.removed,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum EventPayload<'a> {
    Web3Log(Web3Log),
    Launch,
    HttpRequest,
    Scheduled,
    #[serde(borrow)]
    _Phantom(std::marker::PhantomData<&'a ()>), // Future event types
}

impl TryFrom<interface::EventPayload> for EventPayload<'_> {
    type Error = ConversionError;
    fn try_from(p_payload: interface::EventPayload) -> Result<Self, Self::Error> {
        match p_payload.payload {
            Some(interface::event_payload::Payload::Web3Log(log)) => {
                Ok(EventPayload::Web3Log(Web3Log::try_from(log)?))
            }
            Some(interface::event_payload::Payload::LaunchEvent(_)) => Ok(EventPayload::Launch),
            Some(interface::event_payload::Payload::HttpRequest(_)) => {
                Ok(EventPayload::HttpRequest)
            }
            Some(interface::event_payload::Payload::ScheduledEvent(_)) => {
                Ok(EventPayload::Scheduled)
            }
            None => Err(ConversionError::MissingField("payload".to_string())),
        }
    }
}

impl From<EventPayload<'_>> for interface::EventPayload {
    fn from(payload: EventPayload) -> Self {
        match payload {
            EventPayload::Web3Log(log) => Self {
                payload: Some(interface::event_payload::Payload::Web3Log(log.into())),
            },
            EventPayload::Launch => Self {
                payload: Some(interface::event_payload::Payload::LaunchEvent(())),
            },
            EventPayload::HttpRequest => Self {
                payload: Some(interface::event_payload::Payload::HttpRequest(())),
            },
            EventPayload::Scheduled => Self {
                payload: Some(interface::event_payload::Payload::ScheduledEvent(())),
            },
            EventPayload::_Phantom(_) => panic!("Cannot convert _Phantom EventPayload"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_work_order_payload_deserialization_with_complex_userdata() {
        let json_str = r#"{
            "id": "test-wo-123",
            "worker": {
                "bundle": {
                    "source": "data:application/json;base64,e30="
                },
                "identities": [],
                "userdata": {
                    "config_string": "some-value",
                    "config_number": 123,
                    "config_bool": true,
                    "config_object": {
                        "nested_key": "nested_value",
                        "nested_array": [1, "two", false]
                    }
                }
            },
            "events": [
                { "handler": "onLaunch", "kind": "launch" },
                { "handler": "onEvent", "kind": "web3_event", "chain": 1, "address": [], "topics": [] },
                { "handler": "onScheduled", "kind": "scheduled", "period_ms": 60000 }
            ]
        }"#;

        let result: Result<WorkOrderPayload, _> = serde_json::from_str(json_str);
        assert!(result.is_ok(), "Failed to deserialize: {:?}", result.err());

        let payload = result.unwrap();
        assert_eq!(payload.id, "test-wo-123");
        assert_eq!(payload.events.len(), 3);

        let userdata = &payload.worker.userdata;
        assert_eq!(
            userdata.get("config_string").unwrap(),
            &Value::String("some-value".to_string())
        );
        assert_eq!(
            userdata.get("config_number").unwrap(),
            &Value::Number(123.into())
        );
        assert!(userdata.get("config_object").unwrap().is_object());

        // Verify the scheduled event was parsed correctly
        if let WorkerEventKind::Scheduled(Schedule::Rate(rate_mode)) = &payload.events[2].kind {
            assert_eq!(rate_mode.period_ms, 60000);
            assert_eq!(rate_mode.mode, Mode::Rate);
        } else {
            panic!("Expected scheduled event kind");
        }
    }

    #[test]
    fn test_work_order_with_scheduled_events() {
        // Test a full work order with scheduled events to ensure it serializes/deserializes correctly
        let work_order = WorkOrderPayload {
            id: "test-scheduled".to_string(),
            worker: WorkerManifest {
                bundle: WorkerBundlePointer {
                    source: url::Url::parse(
                        "data:application/javascript;base64,Y29uc29sZS5sb2coImhlbGxvIik=",
                    )
                    .unwrap(),
                    hash: None,
                },
                identities: vec![],
                userdata: std::collections::HashMap::new(),
            },
            events: vec![
                WorkerEvent {
                    handler: "launch".to_string(),
                    kind: WorkerEventKind::Launch,
                },
                WorkerEvent {
                    handler: "tick".to_string(),
                    kind: WorkerEventKind::Scheduled(Schedule::Rate(RateMode::new(5000))),
                },
            ],
        };

        // Test serialization
        let json = serde_json::to_string_pretty(&work_order).expect("Serialization should work");
        println!("Work order JSON: {}", json);

        // Test deserialization
        let deserialized: WorkOrderPayload =
            serde_json::from_str(&json).expect("Deserialization should work");
        assert_eq!(deserialized.id, work_order.id);
        assert_eq!(deserialized.events.len(), 2);

        // Verify the scheduled event was preserved
        if let WorkerEventKind::Scheduled(Schedule::Rate(rate_mode)) = &deserialized.events[1].kind
        {
            assert_eq!(rate_mode.period_ms, 5000);
        } else {
            panic!("Expected scheduled event");
        }
    }

    #[test]
    fn test_schedule_serialization_deserialization() {
        // Test minimal schedule
        let schedule = Schedule::Rate(RateMode::new(1000));
        let json = serde_json::to_string(&schedule).unwrap();
        let deserialized: Schedule = serde_json::from_str(&json).unwrap();
        assert_eq!(schedule, deserialized);

        // Test full schedule with all options
        let start_time = DateTime::from_timestamp(1640995200, 0).unwrap(); // 2022-01-01 00:00:00 UTC
        let end_time = DateTime::from_timestamp(1672531200, 0).unwrap(); // 2023-01-01 00:00:00 UTC

        let policy = Policy {
            catch_up: CatchUp::Coalesce,
            max_lateness_ms: Some(5000),
            jitter_budget_ms: Some(100),
        };

        let rate_mode = RateMode {
            mode: Mode::Rate,
            period_ms: 30000,
            phase_ms: 1000,
            start_at: Some(start_time),
            end_at: Some(end_time),
            max_occurrences: Some(100),
            policy: Some(policy),
        };

        let schedule = Schedule::Rate(rate_mode);
        let json = serde_json::to_string(&schedule).unwrap();
        let deserialized: Schedule = serde_json::from_str(&json).unwrap();
        assert_eq!(schedule, deserialized);

        // Test that minimal JSON works (only period_ms specified)
        let minimal_json = r#"{"period_ms": 5000}"#;
        let schedule: Schedule = serde_json::from_str(minimal_json).unwrap();
        let Schedule::Rate(rate_mode) = schedule;
        assert_eq!(rate_mode.period_ms, 5000);
        assert_eq!(rate_mode.mode, Mode::Rate);
        assert_eq!(rate_mode.phase_ms, 0);
        assert!(rate_mode.start_at.is_none());
        assert!(rate_mode.end_at.is_none());
        assert!(rate_mode.max_occurrences.is_none());
        assert!(rate_mode.policy.is_none());
    }

    #[test]
    fn test_chain_identifier_serialization() {
        // Test JSON serialization backwards compatibility
        let chain_id = ChainIdentifier::ChainId(1);
        let json = serde_json::to_string(&chain_id).unwrap();
        assert_eq!(json, "1");

        let gateway = ChainIdentifier::GatewayUrl("wss://custom.com".parse().unwrap());
        let json = serde_json::to_string(&gateway).unwrap();
        assert_eq!(json, "\"wss://custom.com/\"");
    }

    #[test]
    fn test_chain_identifier_deserialization_edge_cases() {
        // Test invalid URLs
        let result: Result<ChainIdentifier, _> = serde_json::from_str("\"not-a-url\"");
        assert!(result.is_err());

        // Test various URL schemes
        let schemes = ["ws://", "wss://", "http://", "https://"];
        for scheme in schemes {
            let url = format!("\"{}example.com\"", scheme);
            let result: Result<ChainIdentifier, _> = serde_json::from_str(&url);
            assert!(result.is_ok());
        }
    }

    #[test]
    fn test_chain_identifier_display() {
        let chain_id = ChainIdentifier::ChainId(42);
        assert_eq!(chain_id.to_string(), "42");

        let gateway = ChainIdentifier::GatewayUrl("wss://test.com".parse().unwrap());
        assert_eq!(gateway.to_string(), "wss://test.com/");
    }

    #[test]
    fn test_chain_identifier_helper_methods() {
        let chain_id = ChainIdentifier::ChainId(123);
        assert_eq!(chain_id.chain_id(), Some(123));
        assert_eq!(chain_id.gateway_url(), None);

        let url: Url = "wss://example.com".parse().unwrap();
        let gateway = ChainIdentifier::GatewayUrl(url.clone());
        assert_eq!(gateway.chain_id(), None);
        assert_eq!(gateway.gateway_url(), Some(&url));
    }

    #[test]
    fn test_chain_identifier_protobuf_conversion() {
        // Test ChainId conversion
        let chain_id = ChainIdentifier::ChainId(42);
        let proto: interface::ChainIdentifier = chain_id.clone().into();
        let back: ChainIdentifier = proto.try_into().unwrap();
        assert_eq!(chain_id, back);

        // Test GatewayUrl conversion
        let gateway = ChainIdentifier::GatewayUrl("wss://test.com".parse().unwrap());
        let proto: interface::ChainIdentifier = gateway.clone().into();
        let back: ChainIdentifier = proto.try_into().unwrap();
        assert_eq!(gateway, back);
    }

    #[test]
    fn test_protobuf_conversion_errors() {
        // Test missing identifier field
        let proto = interface::ChainIdentifier { identifier: None };
        let result: Result<ChainIdentifier, _> = proto.try_into();
        assert!(result.is_err());

        // Test invalid URL in protobuf
        let proto = interface::ChainIdentifier {
            identifier: Some(interface::chain_identifier::Identifier::GatewayUrl(
                "not-a-valid-url".to_string(),
            )),
        };
        let result: Result<ChainIdentifier, _> = proto.try_into();
        assert!(result.is_err());
    }

    #[test]
    fn test_secret_id_with_custom_gateway() {
        let gateway_url = "wss://custom.chain.com".parse().unwrap();
        let secret_id = SecretId {
            chain: ChainIdentifier::GatewayUrl(gateway_url),
            identity_address: Address::from([1u8; 20]),
            identity_id: U256::from(456),
        };

        // Test serialization roundtrip
        let json = serde_json::to_string(&secret_id).unwrap();
        let deserialized: SecretId = serde_json::from_str(&json).unwrap();
        assert_eq!(secret_id, deserialized);
    }

    #[test]
    fn test_web3_event_with_custom_gateway() {
        let gateway_url = "wss://custom.chain.com".parse().unwrap();
        let event = Web3Event {
            chain: ChainIdentifier::GatewayUrl(gateway_url),
            address: vec![],
            topics: vec![],
            gateways: vec![],
        };

        // Test that custom gateway works in event context
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: Web3Event = serde_json::from_str(&json).unwrap();
        assert_eq!(event, deserialized);
    }
}
