use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};

use crate::proto::interface as proto;

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
    pub block_hashes: Vec<Vec<u8>>,
    pub user_data: Vec<u8>,
}

impl FromProto<proto::AttestationReport> for AttestationReport {
    fn from_proto(p: proto::AttestationReport) -> Self {
        Self {
            ephemeral_public_key: p.ephemeral_public_key,
            block_hashes: p.block_hashes,
            user_data: p.user_data,
        }
    }
}

impl IntoProto<proto::AttestationReport> for AttestationReport {
    fn to_proto(&self) -> proto::AttestationReport {
        let mut out = proto::AttestationReport::default();
        out.ephemeral_public_key = self.ephemeral_public_key.clone();
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

impl FromProto<proto::SecretIdentifier> for SecretId {
    fn from_proto(p: proto::SecretIdentifier) -> Self {
        Self {
            chain_id: p.chain_id,
            identity_address: p.identity_address.parse().unwrap_or(Address::ZERO), // Handle parse error gracefully
            identity_id: p.identity_id.parse().unwrap_or(U256::ZERO), // Handle parse error
        }
    }
}

impl IntoProto<proto::SecretIdentifier> for SecretId {
    fn to_proto(&self) -> proto::SecretIdentifier {
        let mut out = proto::SecretIdentifier::default();
        out.chain_id = self.chain_id;
        out.identity_address = format!("{:#x}", self.identity_address);
        out.identity_id = self.identity_id.to_string();
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumerInfo {
    pub code_hash: Vec<u8>,
    pub signature: Vec<u8>,
}

impl FromProto<proto::ConsumerInfo> for ConsumerInfo {
    fn from_proto(p: proto::ConsumerInfo) -> Self {
        Self {
            code_hash: p.code_hash,
            signature: p.signature,
        }
    }
}

impl IntoProto<proto::ConsumerInfo> for ConsumerInfo {
    fn to_proto(&self) -> proto::ConsumerInfo {
        let mut out = proto::ConsumerInfo::default();
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

impl FromProto<proto::SecretRequest> for SecretRequest {
    fn from_proto(p: proto::SecretRequest) -> Self {
        Self {
            secret_id: SecretId::from_proto(p.secret_id.unwrap_or_default()),
            consumer: ConsumerInfo::from_proto(p.consumer.unwrap_or_default()),
        }
    }
}

impl IntoProto<proto::SecretRequest> for SecretRequest {
    fn to_proto(&self) -> proto::SecretRequest {
        let mut out = proto::SecretRequest::default();
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

impl FromProto<proto::EnvReport> for EnvReport {
    fn from_proto(p: proto::EnvReport) -> Self {
        Self {
            attestation: AttestationReport::from_proto(p.attestation.unwrap_or_default()),
            operator_signature: p.operator_signature,
            node_id: p.node_id,
        }
    }
}

impl IntoProto<proto::EnvReport> for EnvReport {
    fn to_proto(&self) -> proto::EnvReport {
        let mut out = proto::EnvReport::default();
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
    pub signature: Vec<u8>, // Signature over encrypted_payload made with sender's *signing* key
    pub alg: String,
    pub contained_secret_ids: Vec<SecretId>,
}

impl SecretsBox {
    pub fn new_empty() -> Self {
        Self {
            encrypted_payload: vec![],
            sender_public_key: vec![],
            signature: vec![],
            alg: "X25519_AES-GCM-SIV_Ed25519".to_string(), // Default algorithm
            contained_secret_ids: vec![],
        }
    }
}

impl FromProto<proto::SecretsBox> for SecretsBox {
    fn from_proto(p: proto::SecretsBox) -> Self {
        Self {
            encrypted_payload: p.encrypted_payload,
            sender_public_key: p.sender_public_key,
            signature: p.signature,
            alg: p.alg,
            contained_secret_ids: p
                .contained_secret_ids
                .into_iter()
                .map(SecretId::from_proto)
                .collect(),
        }
    }
}

impl IntoProto<proto::SecretsBox> for SecretsBox {
    fn to_proto(&self) -> proto::SecretsBox {
        let mut out = proto::SecretsBox::default();
        out.encrypted_payload = self.encrypted_payload.clone();
        out.sender_public_key = self.sender_public_key.clone();
        out.signature = self.signature.clone();
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

impl FromProto<proto::PolicyExecutionRequest> for PolicyExecutionRequest {
    fn from_proto(p: proto::PolicyExecutionRequest) -> Self {
        Self {
            secret_ids: p.secret_ids.into_iter().map(SecretId::from_proto).collect(),
            consumer: ConsumerInfo::from_proto(p.consumer.unwrap_or_default()),
            env_report: EnvReport::from_proto(p.env_report.unwrap_or_default()),
        }
    }
}

impl IntoProto<proto::PolicyExecutionRequest> for PolicyExecutionRequest {
    fn to_proto(&self) -> proto::PolicyExecutionRequest {
        proto::PolicyExecutionRequest {
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
