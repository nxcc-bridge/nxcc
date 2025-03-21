use std::{collections::HashMap, sync::Arc};

use ethers::{
    prelude::*,
    providers::{Http, Provider},
};
use tokio::sync::RwLock;
use tracing::debug;

use crate::error::AppError;

abigen!(Identity, "../contracts/evm/out/Identity.sol/Identity.json");

#[derive(Debug, Clone)]
pub struct GatewayManager {
    providers: Arc<RwLock<HashMap<u64, Provider<Http>>>>,
}

impl GatewayManager {
    pub fn new() -> Self {
        Self {
            providers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get_provider(&self, chain_id: u64) -> Result<Provider<Http>, AppError> {
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
        let provider = Provider::<Http>::try_from(rpc_url)
            .map_err(|e| AppError::Service(format!("Failed to create provider: {}", e)))?;

        providers.insert(chain_id, provider.clone());
        Ok(provider)
    }

    fn get_rpc_url_for_chain(&self, chain_id: u64) -> Result<String, AppError> {
        // In a real implementation, this would come from configuration
        // For now, we'll use hardcoded values for common chains
        match chain_id {
            1 => Ok("https://eth.llamarpc.com".to_string()),
            5 => Ok("https://rpc.ankr.com/eth_goerli".to_string()),
            11155111 => Ok("https://rpc.sepolia.org".to_string()),
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
        let provider = self.get_provider(chain_id).await?;

        let identity_contract = Identity::new(identity_address, Arc::new(provider));

        // Call the tokenURI function to get the policy URL
        let policy_url = identity_contract
            .token_uri(identity_id)
            .call()
            .await
            .map_err(|e| AppError::Service(format!("Failed to fetch policy URL: {}", e)))?;

        debug!(
            "Retrieved policy URL for identity {:#x} on chain {}: {}",
            identity_address, chain_id, policy_url
        );

        Ok(policy_url)
    }
}
