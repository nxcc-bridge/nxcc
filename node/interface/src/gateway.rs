use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    pub chain_id: u64,
    pub rpc_url: String,
    pub ws_url: Option<String>,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockInfo {
    pub chain_id: u64,
    pub chain_name: String,
    pub block_number: u64,
    pub block_hash: Vec<u8>,
    pub timestamp: u64,
    pub fetched_at: u64,
}

/// Provider for blockchain gateway access
#[async_trait]
pub trait GatewayProvider: Send + Sync {
    /// Get configured gateways for a chain
    async fn get_gateways(&self, chain_id: u64) -> Result<Vec<GatewayConfig>>;

    /// Add user-provided gateway override
    async fn add_user_gateway(&self, gateway: GatewayConfig) -> Result<()>;

    /// Fetch latest block info from a chain
    async fn fetch_latest_block(&self, chain_id: u64) -> Result<BlockInfo>;

    /// Fetch latest blocks from multiple chains concurrently
    async fn fetch_multiple_latest_blocks(&self, chain_ids: &[u64]) -> Result<Vec<BlockInfo>>;
}
