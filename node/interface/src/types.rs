use std::collections::HashMap;

use alloy_primitives::{Address, B256, U256};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use serde::{Deserialize, Serialize};

use crate::proto::{enclave, interface};

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

impl From<interface::SecretIdentifier> for SecretId {
    fn from(p: interface::SecretIdentifier) -> Self {
        Self {
            chain_id: p.chain_id,
            identity_address: p.identity_address.parse().unwrap_or(Address::ZERO), // Handle parse error gracefully (identity contract cannot be deployed to the zero address)
            identity_id: p.identity_id.parse().unwrap_or(U256::ZERO), // Handle parse error (the zero identity is unassignable)
        }
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

impl From<interface::SecretRequest> for SecretRequest {
    fn from(p: interface::SecretRequest) -> Self {
        Self {
            secret_id: p
                .secret_id
                .map(SecretId::from)
                .unwrap_or_else(|| SecretId::from(interface::SecretIdentifier::default())),
            consumer: p
                .consumer
                .map(ConsumerInfo::from)
                .unwrap_or_else(|| ConsumerInfo::from(interface::ConsumerInfo::default())),
        }
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

impl From<interface::EnvReport> for EnvReport {
    fn from(p: interface::EnvReport) -> Self {
        Self {
            attestation: p
                .attestation
                .map(AttestationReport::from)
                .unwrap_or_else(
                    || AttestationReport::from(interface::AttestationReport::default()),
                ),
            operator_signature: p.operator_signature,
            node_id: p.node_id,
        }
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

impl From<interface::SecretsBox> for SecretsBox {
    fn from(p: interface::SecretsBox) -> Self {
        Self {
            encrypted_payload: p.encrypted_payload,
            sender_public_key: p.sender_public_key,
            alg: p.alg,
            contained_secret_ids: p
                .contained_secret_ids
                .into_iter()
                .map(SecretId::from)
                .collect(),
        }
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

impl From<interface::PolicyExecutionRequest> for PolicyExecutionRequest {
    fn from(p: interface::PolicyExecutionRequest) -> Self {
        Self {
            secret_ids: p.secret_ids.into_iter().map(SecretId::from).collect(),
            consumer: p
                .consumer
                .map(ConsumerInfo::from)
                .unwrap_or_else(|| ConsumerInfo::from(interface::ConsumerInfo::default())),
            env_report: p
                .env_report
                .map(EnvReport::from)
                .unwrap_or_else(|| EnvReport::from(interface::EnvReport::default())),
        }
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

impl From<enclave::VmAddress> for VmAddress {
    fn from(p: enclave::VmAddress) -> Self {
        match p.address_type {
            Some(enclave::vm_address::AddressType::Tcp(tcp)) => {
                VmAddress::Tcp(TcpAddress::from(tcp))
            }
            Some(enclave::vm_address::AddressType::Uds(uds)) => {
                VmAddress::Uds(UdsAddress::from(uds))
            }
            Some(enclave::vm_address::AddressType::Vsock(vsock)) => {
                VmAddress::Vsock(VsockAddress::from(vsock))
            }
            None => panic!("VmAddress proto is missing address_type"), // Or return an error
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerManifest {
    /// An authenticated pointer to a `WorkerBundle`.
    pub bundle: WorkerBundlePointer,
    /// The set of identities that the worker needs for execution.
    /// These will be bound by the VM into the worker.
    /// Policy workers are not allowed to request identities.
    pub identities: Vec<(SecretId, String)>,
    /// Arbitrary data passed by the creator of the worker manifest.
    /// Untrusted from the perspective of the nXCC system.
    pub userdata: HashMap<String, String>,
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
    fn dsse_envelope(&self) -> Result<DsseEnvelope, serde_json::Error> {
        serde_json::from_slice(&self.0)
    }

    /// Retrieves the `WorkerBundlePayload` from the DSSE envelope.
    pub fn payload(&self) -> WorkerBundlePayload {
        let envelope = self
            .dsse_envelope()
            .expect("Failed to parse DSSE envelope from WorkerBundle bytes");
        if envelope.payload_type != DSSE_WORKER_BUNDLE_PAYLOAD_TYPE {
            panic!(
                "Unexpected DSSE payloadType: expected {}, got {}",
                DSSE_WORKER_BUNDLE_PAYLOAD_TYPE, envelope.payload_type
            );
        }
        let payload_bytes = BASE64_STANDARD
            .decode(&envelope.payload)
            .expect("Failed to base64 decode DSSE payload");
        serde_json::from_slice(&payload_bytes[..])
            .expect("Failed to decode WorkerBundlePayload from DSSE payload")
    }

    /// Calculates the SHA512 hash of the encoded `WorkerBundlePayload`.
    /// This hash is used for `ConsumerInfo.bundle_hash`.
    // TODO: remove this in favor of having the enclave verify the signer or having the hash of the executable be part of the signed data or something. right now it's totally broken, as the consumer cannot be verified with all of the arbitrary metadata in it
    pub fn hash_signed_payload(&self) -> Vec<u8> {
        use sha2::{Digest, Sha512};
        let payload_struct = self.payload();
        let payload_bytes = serde_json::to_vec(&payload_struct)
            .expect("Failed to encode WorkerBundlePayload for hashing");
        Sha512::digest(payload_bytes).to_vec()
    }

    /// Extracts the first signature from the DSSE envelope.
    pub fn get_dsse_signature(&self) -> Vec<u8> {
        let envelope = self
            .dsse_envelope()
            .expect("Failed to parse DSSE envelope for signature extraction");
        if envelope.signatures.is_empty() {
            panic!("DSSE envelope has no signatures");
        }
        // Return the raw bytes of the first signature
        BASE64_STANDARD
            .decode(&envelope.signatures[0].sig)
            .expect("Failed to base64 decode DSSE signature")
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    HttpRequestTrigger,
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
}

impl From<interface::Web3EventConfig> for Web3Event {
    fn from(p: interface::Web3EventConfig) -> Self {
        Self {
            chain: p.chain_id,
            address: p
                .address
                .into_iter()
                .map(|s| s.parse().unwrap_or_default())
                .collect(),
            topics: p
                .topics
                .into_iter()
                .map(|topic_filter| {
                    topic_filter
                        .values
                        .into_iter()
                        .map(|s| s.parse().unwrap_or_default())
                        .collect()
                })
                .collect(),
        }
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

impl From<interface::Web3Log> for Web3Log {
    fn from(p_log: interface::Web3Log) -> Self {
        Self {
            address: Address::from_slice(&p_log.address),
            topics: p_log
                .topics
                .into_iter()
                .map(|b| B256::from_slice(&b))
                .collect(),
            data: p_log.data.into(),
            block_hash: if p_log.block_hash.is_empty() {
                None
            } else {
                Some(B256::from_slice(&p_log.block_hash))
            },
            block_number: if p_log.block_number == 0 && p_log.block_hash.is_empty() {
                None
            } else {
                Some(p_log.block_number)
            }, // Heuristic for optional
            transaction_hash: if p_log.transaction_hash.is_empty() {
                None
            } else {
                Some(B256::from_slice(&p_log.transaction_hash))
            },
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
            }, // Heuristic for optional
            removed: p_log.removed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum EventPayload<'a> {
    Web3Log(Web3Log),
    Launch,
    HttpRequestTrigger,
    #[serde(borrow)]
    _Phantom(std::marker::PhantomData<&'a ()>), // Future event types
}

impl From<interface::EventPayload> for EventPayload<'_> {
    fn from(p_payload: interface::EventPayload) -> Self {
        match p_payload.payload {
            Some(interface::event_payload::Payload::Web3Log(log)) => {
                EventPayload::Web3Log(Web3Log::from(log))
            }
            Some(interface::event_payload::Payload::LaunchEvent(_)) => EventPayload::Launch,
            Some(interface::event_payload::Payload::HttpRequestTrigger(_)) => {
                EventPayload::HttpRequestTrigger
            }
            None => panic!("EventPayload proto is empty"), // Or handle as error
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
            EventPayload::HttpRequestTrigger => Self {
                payload: Some(interface::event_payload::Payload::HttpRequestTrigger(())),
            },
            EventPayload::_Phantom(_) => panic!("Cannot convert _Phantom EventPayload"),
        }
    }
}
