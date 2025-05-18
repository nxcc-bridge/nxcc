use std::collections::{HashMap, HashSet};

use alloy_primitives::{Address, U256};
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretId {
    pub chain_id: u64,
    pub identity_address: Address,
    pub identity_id: U256,
}

impl From<interface::SecretIdentifier> for SecretId {
    fn from(p: interface::SecretIdentifier) -> Self {
        Self {
            chain_id: p.chain_id,
            identity_address: p.identity_address.parse().unwrap_or(Address::ZERO), // Handle parse error gracefully
            identity_id: p.identity_id.parse().unwrap_or(U256::ZERO), // Handle parse error
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

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ConsumerInfo {
    pub code_hash: Vec<u8>,
    pub signature: Vec<u8>,
}

impl From<interface::ConsumerInfo> for ConsumerInfo {
    fn from(p: interface::ConsumerInfo) -> Self {
        Self {
            code_hash: p.code_hash,
            signature: p.signature,
        }
    }
}

impl From<ConsumerInfo> for interface::ConsumerInfo {
    fn from(value: ConsumerInfo) -> Self {
        interface::ConsumerInfo {
            code_hash: value.code_hash,
            signature: value.signature,
        }
    }
}

impl From<&ConsumerInfo> for interface::ConsumerInfo {
    fn from(value: &ConsumerInfo) -> Self {
        interface::ConsumerInfo {
            code_hash: value.code_hash.clone(),
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

/// Represents a unique identifier for an identity that can be bound to a worker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct IdentityId(pub String); // Using String for simplicity, can be a more complex type.

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
    pub identities: HashSet<IdentityId>,
    /// Arbitrary data passed by the creator of the worker manifest.
    /// Untrusted from the perspective of the nXCC system.
    pub userdata: HashMap<String, String>,
}

/// The inner payload of a `WorkerBundle` that gets signed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerBundlePayload {
    /// The VM in which the worker must execute (e.g., "nxcc/workerd").
    pub vm: String,
    /// The executable code (e.g., JS, Python, WASM).
    pub executable: Vec<u8>,
    /// Arbitrary metadata added by the publisher. Not interpreted by nXCC.
    pub metadata: HashMap<String, String>,
}

/// An executable (WorkerBundlePayload) that is signed by its author/publisher.
/// This struct holds the COSE Sign1 envelope as raw bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerBundle(pub Vec<u8>);

impl WorkerBundle {
    /// Creates a new `WorkerBundle` from its payload.
    /// In a real implementation, this would CBOR encode and sign the payload.
    /// For now, it just CBOR encodes the payload.
    pub fn new_from_payload(payload: &WorkerBundlePayload) -> Self {
        let mut bytes = Vec::new();
        ciborium::into_writer(payload, &mut bytes)
            .expect("Failed to CBOR encode WorkerBundlePayload");
        Self(bytes)
    }

    /// Retrieves the `WorkerBundlePayload` from the bundle.
    /// In a real implementation, this would verify the COSE signature before decoding.
    /// For now, it just CBOR decodes the payload.
    pub fn payload(&self) -> WorkerBundlePayload {
        ciborium::from_reader(&self.0[..])
            .expect("Failed to CBOR decode WorkerBundlePayload from WorkerBundle")
    }
}

/// A structure combining a policy's `WorkerManifest` and its resolved `WorkerBundle`.
/// This replaces the old `PolicyBundle` for policy execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullPolicyPackage {
    pub manifest: WorkerManifest,
    pub bundle: WorkerBundle,
}
