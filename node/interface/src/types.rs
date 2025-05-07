use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};

use crate::proto::{enclave, interface};

/// Trait for converting a type to its Protocol Buffers representation
pub trait IntoProto<P> {
    fn to_proto(&self) -> P;
}

/// Trait for converting from a Protocol Buffers representation to a type
pub trait FromProto<P> {
    fn from_proto(proto: P) -> Self;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttestationReport {
    pub ephemeral_public_key: Vec<u8>,
    /// PCR0 or MRENCLAVE
    pub measurement: Vec<u8>,
    pub block_hashes: Vec<Vec<u8>>,
    pub user_data: Vec<u8>,
}

impl FromProto<interface::AttestationReport> for AttestationReport {
    fn from_proto(p: interface::AttestationReport) -> Self {
        Self {
            ephemeral_public_key: p.ephemeral_public_key,
            measurement: p.measurement,
            block_hashes: p.block_hashes,
            user_data: p.user_data,
        }
    }
}

impl IntoProto<interface::AttestationReport> for AttestationReport {
    fn to_proto(&self) -> interface::AttestationReport {
        let mut out = interface::AttestationReport::default();
        out.ephemeral_public_key = self.ephemeral_public_key.clone();
        out.measurement = self.measurement.clone();
        out.block_hashes = self.block_hashes.clone();
        out.user_data = self.user_data.clone();
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretId {
    pub chain_id: u64,
    pub identity_address: Address,
    pub identity_id: U256,
}

impl FromProto<interface::SecretIdentifier> for SecretId {
    fn from_proto(p: interface::SecretIdentifier) -> Self {
        Self {
            chain_id: p.chain_id,
            identity_address: p.identity_address.parse().unwrap_or(Address::ZERO), // Handle parse error gracefully
            identity_id: p.identity_id.parse().unwrap_or(U256::ZERO), // Handle parse error
        }
    }
}

impl IntoProto<interface::SecretIdentifier> for SecretId {
    fn to_proto(&self) -> interface::SecretIdentifier {
        let mut out = interface::SecretIdentifier::default();
        out.chain_id = self.chain_id;
        out.identity_address = format!("{:#x}", self.identity_address);
        out.identity_id = self.identity_id.to_string();
        out
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ConsumerInfo {
    pub code_hash: Vec<u8>,
    pub signature: Vec<u8>,
}

impl FromProto<interface::ConsumerInfo> for ConsumerInfo {
    fn from_proto(p: interface::ConsumerInfo) -> Self {
        Self {
            code_hash: p.code_hash,
            signature: p.signature,
        }
    }
}

impl IntoProto<interface::ConsumerInfo> for ConsumerInfo {
    fn to_proto(&self) -> interface::ConsumerInfo {
        let mut out = interface::ConsumerInfo::default();
        out.code_hash = self.code_hash.clone();
        out.signature = self.signature.clone();
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretRequest {
    pub secret_id: SecretId,
    pub consumer: ConsumerInfo,
}

impl FromProto<interface::SecretRequest> for SecretRequest {
    fn from_proto(p: interface::SecretRequest) -> Self {
        Self {
            secret_id: SecretId::from_proto(p.secret_id.unwrap_or_default()),
            consumer: ConsumerInfo::from_proto(p.consumer.unwrap_or_default()),
        }
    }
}

impl IntoProto<interface::SecretRequest> for SecretRequest {
    fn to_proto(&self) -> interface::SecretRequest {
        let mut out = interface::SecretRequest::default();
        out.secret_id = Some(self.secret_id.to_proto());
        out.consumer = Some(self.consumer.to_proto());
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvReport {
    pub attestation: AttestationReport,
    pub operator_signature: Vec<u8>,
    pub node_id: String,
}

impl FromProto<interface::EnvReport> for EnvReport {
    fn from_proto(p: interface::EnvReport) -> Self {
        Self {
            attestation: AttestationReport::from_proto(p.attestation.unwrap_or_default()),
            operator_signature: p.operator_signature,
            node_id: p.node_id,
        }
    }
}

impl IntoProto<interface::EnvReport> for EnvReport {
    fn to_proto(&self) -> interface::EnvReport {
        let mut out = interface::EnvReport::default();
        out.attestation = Some(self.attestation.to_proto());
        out.operator_signature = self.operator_signature.clone();
        out.node_id = self.node_id.clone();
        out
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

impl FromProto<interface::SecretsBox> for SecretsBox {
    fn from_proto(p: interface::SecretsBox) -> Self {
        Self {
            encrypted_payload: p.encrypted_payload,
            sender_public_key: p.sender_public_key,
            alg: p.alg,
            contained_secret_ids: p
                .contained_secret_ids
                .into_iter()
                .map(SecretId::from_proto)
                .collect(),
        }
    }
}

impl IntoProto<interface::SecretsBox> for SecretsBox {
    fn to_proto(&self) -> interface::SecretsBox {
        let mut out = interface::SecretsBox::default();
        out.encrypted_payload = self.encrypted_payload.clone();
        out.sender_public_key = self.sender_public_key.clone();
        out.alg = self.alg.clone();
        out.contained_secret_ids = self
            .contained_secret_ids
            .iter()
            .map(|id| id.to_proto())
            .collect();
        out
    }
}

/// A request for the policy runner that references multiple secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyExecutionRequest {
    pub secret_ids: Vec<SecretId>,
    pub consumer: ConsumerInfo,
    pub env_report: EnvReport, // The EnvReport of the entity being evaluated
}

impl FromProto<interface::PolicyExecutionRequest> for PolicyExecutionRequest {
    fn from_proto(p: interface::PolicyExecutionRequest) -> Self {
        Self {
            secret_ids: p.secret_ids.into_iter().map(SecretId::from_proto).collect(),
            consumer: ConsumerInfo::from_proto(p.consumer.unwrap_or_default()),
            env_report: EnvReport::from_proto(p.env_report.unwrap_or_default()),
        }
    }
}

impl IntoProto<interface::PolicyExecutionRequest> for PolicyExecutionRequest {
    fn to_proto(&self) -> interface::PolicyExecutionRequest {
        interface::PolicyExecutionRequest {
            secret_ids: self.secret_ids.iter().map(|id| id.to_proto()).collect(),
            consumer: Some(self.consumer.to_proto()),
            env_report: Some(self.env_report.to_proto()),
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

impl FromProto<enclave::TcpAddress> for TcpAddress {
    fn from_proto(p: enclave::TcpAddress) -> Self {
        Self {
            host: p.host,
            port: p.port,
        }
    }
}

impl IntoProto<enclave::TcpAddress> for TcpAddress {
    fn to_proto(&self) -> enclave::TcpAddress {
        enclave::TcpAddress {
            host: self.host.clone(),
            port: self.port,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UdsAddress {
    pub path: String,
}

impl FromProto<enclave::UdsAddress> for UdsAddress {
    fn from_proto(p: enclave::UdsAddress) -> Self {
        Self { path: p.path }
    }
}

impl IntoProto<enclave::UdsAddress> for UdsAddress {
    fn to_proto(&self) -> enclave::UdsAddress {
        enclave::UdsAddress {
            path: self.path.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VsockAddress {
    pub cid: u32,
    pub port: u32,
}

impl FromProto<enclave::VsockAddress> for VsockAddress {
    fn from_proto(p: enclave::VsockAddress) -> Self {
        Self {
            cid: p.cid,
            port: p.port,
        }
    }
}

impl IntoProto<enclave::VsockAddress> for VsockAddress {
    fn to_proto(&self) -> enclave::VsockAddress {
        enclave::VsockAddress {
            cid: self.cid,
            port: self.port,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VmAddress {
    Tcp(TcpAddress),
    Uds(UdsAddress),
    Vsock(VsockAddress),
}

impl FromProto<enclave::VmAddress> for VmAddress {
    fn from_proto(p: enclave::VmAddress) -> Self {
        match p.address_type {
            Some(enclave::vm_address::AddressType::Tcp(tcp)) => {
                VmAddress::Tcp(TcpAddress::from_proto(tcp))
            }
            Some(enclave::vm_address::AddressType::Uds(uds)) => {
                VmAddress::Uds(UdsAddress::from_proto(uds))
            }
            Some(enclave::vm_address::AddressType::Vsock(vsock)) => {
                VmAddress::Vsock(VsockAddress::from_proto(vsock))
            }
            None => panic!("VmAddress proto is missing address_type"), // Or return an error
        }
    }
}

impl IntoProto<enclave::VmAddress> for VmAddress {
    fn to_proto(&self) -> enclave::VmAddress {
        let address_type = match self {
            VmAddress::Tcp(tcp) => enclave::vm_address::AddressType::Tcp(tcp.to_proto()),
            VmAddress::Uds(uds) => enclave::vm_address::AddressType::Uds(uds.to_proto()),
            VmAddress::Vsock(vsock) => enclave::vm_address::AddressType::Vsock(vsock.to_proto()),
        };
        enclave::VmAddress {
            address_type: Some(address_type),
        }
    }
}
