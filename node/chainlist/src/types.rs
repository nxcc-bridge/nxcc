use serde::{Deserialize, Serialize};

/// Represents a chain as fetched from the chainlist.org source.
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SourceChain {
    pub name: String,
    pub chain_id: u64,
    pub rpc: Vec<SourceRpc>,
}

/// Represents an RPC endpoint from the chainlist.org source.
#[derive(Deserialize, Debug, Clone)]
pub struct SourceRpc {
    pub url: String,
    pub tracking: Option<String>,
}

/// Represents the final, processed RPC URLs for a chain, categorized by protocol.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct RpcEndpoints {
    pub https: Vec<String>,
    pub wss: Vec<String>,
}

impl RpcEndpoints {
    /// Checks if there are any RPC endpoints defined.
    pub fn is_empty(&self) -> bool {
        self.https.is_empty() && self.wss.is_empty()
    }
}

/// Represents a chain in the final, checked-in JSON format.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Chain {
    pub chain_id: u64,
    pub name: String,
    pub rpcs: RpcEndpoints,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_block_time_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_time_variance_ms: Option<f64>,
}
