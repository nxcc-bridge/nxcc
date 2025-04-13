use crate::proto::interface as proto;
use alloy_primitives::{Address, U256};

/// A remote-verifiable TEE attestation report in domain form.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AttestationReport {
    pub ephemeral_public_key: Vec<u8>,
    pub block_hashes: Vec<Vec<u8>>,
    pub user_data: Vec<u8>,
}

impl AttestationReport {
    pub fn from_proto(p: proto::AttestationReport) -> Self {
        Self {
            ephemeral_public_key: p.ephemeral_public_key,
            block_hashes: p.block_hashes,
            user_data: p.user_data,
        }
    }

    pub fn to_proto(&self) -> proto::AttestationReport {
        let mut out = proto::AttestationReport::default();
        out.ephemeral_public_key = self.ephemeral_public_key.clone();
        out.block_hashes = self.block_hashes.clone();
        out.user_data = self.user_data.clone();
        out
    }
}

/// An identifier for a secret (chain ID, identity address, identity ID).
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct SecretId {
    pub chain_id: u64,
    pub identity_address: Address,
    pub identity_id: U256,
}

impl SecretId {
    pub fn from_proto(p: proto::SecretIdentifier) -> Self {
        Self {
            chain_id: p.chain_id,
            identity_address: p
                .identity_address
                .parse::<Address>()
                .expect("Invalid address"),
            identity_id: p.identity_id.parse::<U256>().expect("Invalid U256"),
        }
    }

    pub fn to_proto(&self) -> proto::SecretIdentifier {
        let mut out = proto::SecretIdentifier::default();
        out.chain_id = self.chain_id;
        out.identity_address = format!("{:#x}", self.identity_address); // Use 0x prefix for consistency
        out.identity_id = self.identity_id.to_string(); // U256::to_string() is usually sufficient
        out
    }
}

/// The "box" containing one or more secrets, encrypted for some ephemeral pubkey.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SecretsBox {
    pub encrypted_payload: Vec<u8>,
    pub nonce: Vec<u8>,
    pub sender_public_key: Vec<u8>,
    pub signature: Vec<u8>,
    pub alg: String,
}

impl SecretsBox {
    pub fn new_empty() -> Self {
        SecretsBox {
            encrypted_payload: Vec::new(),
            nonce: Vec::new(),
            sender_public_key: Vec::new(),
            signature: Vec::new(),
            alg: "X25519+AES256GCM".to_string(),
        }
    }

    pub fn from_proto(p: proto::SecretsBox) -> Self {
        Self {
            encrypted_payload: p.encrypted_payload,
            nonce: p.nonce,
            sender_public_key: p.sender_public_key,
            signature: p.signature,
            alg: p.alg,
        }
    }

    pub fn to_proto(&self) -> proto::SecretsBox {
        let mut out = proto::SecretsBox::default();
        out.encrypted_payload = self.encrypted_payload.clone();
        out.nonce = self.nonce.clone();
        out.sender_public_key = self.sender_public_key.clone();
        out.signature = self.signature.clone();
        out.alg = self.alg.clone();
        out
    }
}

/// Minimal data needed to request a secret
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SecretRequest {
    pub consumer: Vec<u8>,
}

impl SecretRequest {
    pub fn from_proto(p: proto::SecretRequest) -> Self {
        Self {
            consumer: p.consumer,
        }
    }

    pub fn to_proto(&self) -> proto::SecretRequest {
        let mut out = proto::SecretRequest::default();
        out.consumer = self.consumer.clone();
        out
    }
}

/// Minimal data describing the requester's environment
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SecretRequesterInfo {
    pub report: Vec<u8>,
    pub public_key: Vec<u8>,
}

impl SecretRequesterInfo {
    pub fn from_proto(p: proto::SecretRequesterInfo) -> Self {
        Self {
            report: p.report,
            public_key: p.public_key,
        }
    }

    pub fn to_proto(&self) -> proto::SecretRequesterInfo {
        let mut out = proto::SecretRequesterInfo::default();
        out.report = self.report.clone();
        out.public_key = self.public_key.clone();
        out
    }
}
