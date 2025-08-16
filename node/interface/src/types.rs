use std::collections::HashMap;

use alloy_primitives::{Address, B256, U256};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::proto::{enclave, interface};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttestationReport {
    pub ephemeral_public_key: Vec<u8>,
    /// PCR0 or MRENCLAVE
    pub measurement: Vec<u8>,
    pub block_hashes: Vec<Vec<u8>>,
    pub user_data: Vec<u8>,
}

impl From<interface::AttestationReport> for AttestationReport {
    fn from(p: interface::AttestationReport) -> Self {
        Self {
            ephemeral_public_key: p.ephemeral_public_key,
            measurement: p.measurement,
            block_hashes: p.block_hashes,
            user_data: p.user_data,
        }
    }
}

impl From<AttestationReport> for interface::AttestationReport {
    fn from(value: AttestationReport) -> Self {
        Self {
            ephemeral_public_key: value.ephemeral_public_key,
            measurement: value.measurement,
            block_hashes: value.block_hashes,
            user_data: value.user_data,
        }
    }
}

impl From<&AttestationReport> for interface::AttestationReport {
    fn from(value: &AttestationReport) -> Self {
        interface::AttestationReport {
            ephemeral_public_key: value.ephemeral_public_key.clone(),
            measurement: value.measurement.clone(),
            block_hashes: value.block_hashes.clone(),
            user_data: value.user_data.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretId {
    pub chain_id: u64,
    pub identity_address: Address,
    pub identity_id: U256,
}

impl TryFrom<interface::SecretIdentifier> for SecretId {
    type Error = ConversionError;
    fn try_from(p: interface::SecretIdentifier) -> Result<Self, Self::Error> {
        Ok(Self {
            chain_id: p.chain_id,
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
            chain_id: value.chain_id,
            identity_address: format!("{:#x}", value.identity_address),
            identity_id: value.identity_id.to_string(),
        }
    }
}

impl From<&SecretId> for interface::SecretIdentifier {
    fn from(value: &SecretId) -> Self {
        interface::SecretIdentifier {
            chain_id: value.chain_id,
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
pub struct EnvReport {
    pub attestation: AttestationReport,
    pub operator_signature: Vec<u8>,
    pub node_id: String,
}

impl TryFrom<interface::EnvReport> for EnvReport {
    type Error = ConversionError;
    fn try_from(p: interface::EnvReport) -> Result<Self, Self::Error> {
        Ok(Self {
            attestation: p
                .attestation
                .map(AttestationReport::from)
                .ok_or(ConversionError::MissingField("attestation".to_string()))?,
            operator_signature: p.operator_signature,
            node_id: p.node_id,
        })
    }
}

impl From<EnvReport> for interface::EnvReport {
    fn from(value: EnvReport) -> Self {
        interface::EnvReport {
            attestation: Some(value.attestation.into()),
            operator_signature: value.operator_signature,
            node_id: value.node_id,
        }
    }
}

impl From<&EnvReport> for interface::EnvReport {
    fn from(value: &EnvReport) -> Self {
        interface::EnvReport {
            attestation: Some(value.attestation.clone().into()),
            operator_signature: value.operator_signature.clone(),
            node_id: value.node_id.clone(),
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

/// A request for the policy runner that references multiple secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyExecutionRequest {
    pub secret_ids: Vec<SecretId>,
    pub consumer: ConsumerInfo,
    pub env_report: EnvReport, // The EnvReport of the entity being evaluated
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
    pub chain: u64,
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
            chain: p.chain_id,
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
            chain_id: value.chain,
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
}
