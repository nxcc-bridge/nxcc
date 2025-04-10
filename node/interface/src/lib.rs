pub mod proto {
    pub mod daemon {
        tonic::include_proto!("daemon");
    }
    pub mod enclave {
        tonic::include_proto!("enclave");
    }
    pub mod interface {
        tonic::include_proto!("interface");
    }
}

// Move domain types into types.rs
pub mod types;

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
