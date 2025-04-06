pub mod proto {
    pub mod daemon {
        tonic::include_proto!("daemon");
    }

    pub mod enclave {
        tonic::include_proto!("enclave");
    }
}

pub mod policy {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PolicyManifest {
        pub version: String,
        pub name: String,
        pub description: String,
        pub allowed_consumers: Vec<String>,
        pub execution_constraints: ExecutionConstraints,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ExecutionConstraints {
        pub max_memory_mb: u32,
        pub max_execution_time_ms: u32,
        pub allowed_network_calls: bool,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PolicyBundle {
        pub manifest: PolicyManifest,
        pub executable: Vec<u8>,
    }
}

/// Domain-level representation of an attestation report
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AttestationReport {
    pub ephemeral_public_key: Vec<u8>,
    pub block_hashes: Vec<Vec<u8>>,
    pub user_data: Vec<u8>,
}

#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct SecretId {
    pub chain_id: u64,
    pub identity_address: ethers::types::Address,
    pub identity_id: ethers::types::U256,
}

/// A single secret stored in or returned by an enclave
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Secret {
    pub id: SecretId,
    pub data: Vec<u8>,
    // Optionally track expiry or other metadata
    pub expiry: Option<u64>,
}

/// The "box" containing one or more secrets, encrypted to the requester's ephemeral pubkey
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
}

/// Minimal data needed to request a secret
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SecretRequest {
    pub consumer: Vec<u8>,
}

/// Minimal data describing the requester's environment
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SecretRequesterInfo {
    pub report: Vec<u8>,
    pub public_key: Vec<u8>,
}
