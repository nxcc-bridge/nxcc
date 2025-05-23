use std::{collections::HashMap, sync::Arc};

use alloy_primitives::{Address, U256};
use alloy_provider::{DynProvider, Provider, ProviderBuilder};
use alloy_sol_types::sol;
use alloy_transport_http::Http;
use reqwest::Client;
use tokio::sync::RwLock;
use tracing::debug;
use url::Url;

use crate::error::AppError;

sol!(
    #[sol(rpc)] // Add rpc attribute for contract calls
    Identity,
    "src/web3/Identity.json"
);

#[derive(Debug, Clone)]
pub struct GatewayManager {
    providers: Arc<RwLock<HashMap<u64, DynProvider>>>,
}

impl GatewayManager {
    pub fn new() -> Self {
        Self {
            providers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get_provider(&self, chain_id: u64) -> Result<DynProvider, AppError> {
        let providers = self.providers.read().await;

        if let Some(provider) = providers.get(&chain_id) {
            return Ok(provider.clone());
        }

        drop(providers); // Release read lock before acquiring write lock

        let mut providers = self.providers.write().await;

        // Check again in case another thread added it while we were waiting
        if let Some(provider) = providers.get(&chain_id) {
            return Ok(provider.clone());
        }

        // Create a new provider based on chain_id
        let rpc_url = self.get_rpc_url_for_chain(chain_id)?;
        let url = rpc_url
            .parse::<Url>()
            .map_err(|e| AppError::Service(format!("Invalid RPC URL {}: {}", rpc_url, e)))?;
        let provider = ProviderBuilder::new()
            .on_ws(alloy_provider::WsConnect::new(url))
            .await
            .map_err(|e| AppError::Service(format!("Failed to connect provider to {rpc_url}")))?;

        providers.insert(chain_id, provider.clone().erased());
        Ok(provider.erased())
    }

    fn get_rpc_url_for_chain(&self, chain_id: u64) -> Result<String, AppError> {
        // In a real implementation, this would come from configuration
        // For now, we'll use hardcoded values for common chains
        match chain_id {
            0 => Ok("mock://gateway.example.com".to_string()),
            1 => Ok("wss://eth.llamarpc.com".to_string()),
            5 => Ok("wss://rpc.ankr.com/eth_goerli".to_string()),
            1337 | 31337 => Ok("ws://127.0.0.1:8545".to_string()),
            11155111 => Ok("wss://rpc.sepolia.org".to_string()),
            // Add more chains as needed
            _ => Err(AppError::Service(format!(
                "No RPC URL configured for chain ID {}",
                chain_id
            ))),
        }
    }

    pub async fn get_policy_url(
        &self,
        chain_id: u64,
        identity_address: Address,
        identity_id: U256,
    ) -> Result<String, AppError> {
        let rpc_url = self.get_rpc_url_for_chain(chain_id)?;
        if rpc_url.starts_with("mock://") {
            // Return a dummy URL rather than calling the contract
            debug!("Using mock policy URL for chain_id={chain_id}");
            return Ok("mock://policy.example.com".to_string());
        }

        let url = rpc_url
            .parse::<Url>()
            .map_err(|e| AppError::Service(format!("Invalid RPC URL {}: {}", rpc_url, e)))?;
        let provider = ProviderBuilder::new().on_http(url);

        let identity_contract = Identity::new(identity_address, provider.clone()); // Provider is likely Arc-wrapped internally
        let policy_url: String = identity_contract
            .tokenURI(identity_id)
            .call()
            .await
            .map_err(|e| AppError::Service(format!("Failed to fetch policy URL: {e}")))?;

        debug!(
            "Retrieved policy URL for identity {identity_address:#x} on chain {chain_id}: \
             {policy_url}"
        );
        Ok(policy_url)
    }
}
