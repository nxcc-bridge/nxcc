use std::{collections::HashMap, sync::Arc};

use alloy_primitives::{Address, U256};
use alloy_provider::{DynProvider, Provider, ProviderBuilder};
use alloy_sol_types::sol;
use alloy_transport_http::Http;
use tokio::sync::RwLock;
use tracing::debug;
use url::Url;

use crate::error::AppError;
use nxcc_chainlist::{RpcType, get_rpcs_for_chain};

sol!(
    #[sol(rpc)]
    Identity,
    "src/web3/Identity.json",
);

/// Abstraction over one or more gateways for a chain.
/// Currently it connects to all provided URLs and uses the first provider,
/// but the structure allows future consensus or redundancy strategies.
#[derive(Debug, Clone)]
pub struct EventGateway {
    urls: Vec<String>,
    providers: Arc<RwLock<Vec<DynProvider>>>,
}

impl EventGateway {
    pub fn new(urls: Vec<String>) -> Self {
        Self {
            urls,
            providers: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn urls(&self) -> &[String] {
        &self.urls
    }

    pub async fn provider(&self) -> Result<DynProvider, AppError> {
        {
            let providers = self.providers.read().await;
            if let Some(p) = providers.first() {
                return Ok(p.clone());
            }
        }

        let mut providers = self.providers.write().await;
        if providers.is_empty() {
            for rpc_url in &self.urls {
                let url = rpc_url.parse::<Url>().map_err(|e| {
                    AppError::Service(format!("Invalid RPC URL {}: {}", rpc_url, e))
                })?;
                let provider = ProviderBuilder::new()
                    .on_ws(alloy_provider::WsConnect::new(url))
                    .await
                    .map_err(|_| {
                        AppError::Service(format!("Failed to connect provider to {rpc_url}"))
                    })?;
                providers.push(provider.erased());
            }
        }

        providers
            .first()
            .cloned()
            .ok_or_else(|| AppError::Service("No gateways available".to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct GatewayManager {
    gateways: Arc<RwLock<HashMap<u64, Arc<EventGateway>>>>,
}

impl GatewayManager {
    pub fn new() -> Self {
        Self {
            gateways: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get_gateway(&self, chain_id: u64) -> Result<Arc<EventGateway>, AppError> {
        {
            let gateways = self.gateways.read().await;
            if let Some(g) = gateways.get(&chain_id) {
                return Ok(g.clone());
            }
        }

        let mut gateways = self.gateways.write().await;
        if let Some(g) = gateways.get(&chain_id) {
            return Ok(g.clone());
        }

        let rpc_url = self.get_rpc_url_for_chain(chain_id)?;
        let gateway = Arc::new(EventGateway::new(vec![rpc_url]));
        gateways.insert(chain_id, gateway.clone());
        Ok(gateway)
    }

    /// Returns a gateway for a specific event. If `urls` is empty, the default
    /// gateway for `chain_id` is used. Otherwise a new gateway with the provided
    /// URLs is created.
    pub async fn gateways_for_event(
        &self,
        chain_id: u64,
        urls: &[String],
    ) -> Result<Arc<EventGateway>, AppError> {
        if urls.is_empty() {
            self.get_gateway(chain_id).await
        } else {
            Ok(Arc::new(EventGateway::new(urls.to_vec())))
        }
    }

    fn get_rpc_url_for_chain(&self, chain_id: u64) -> Result<String, AppError> {
        match chain_id {
            0 => Ok("mock://gateway.example.com".to_string()),
            1337 | 31337 => Ok("ws://127.0.0.1:8545".to_string()), // Ganache/Anvil default
            _ => get_rpcs_for_chain(chain_id, RpcType::Wss)
                .and_then(|mut urls| urls.next())
                .map(|url| url.to_string())
                .ok_or_else(|| {
                    AppError::Service(format!("No RPC URL configured for chain ID {}", chain_id))
                }),
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
            debug!("Using mock policy URL for chain_id={chain_id}");
            return Ok("mock://policy.example.com".to_string());
        }

        let url = rpc_url
            .parse::<Url>()
            .map_err(|e| AppError::Service(format!("Invalid RPC URL {}: {}", rpc_url, e)))?;
        let provider = ProviderBuilder::new().on_http(url);

        let identity_contract = Identity::new(identity_address, provider.clone());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn default_gateway_urls() {
        let manager = GatewayManager::new();
        let gw = manager.get_gateway(31337).await.unwrap();
        assert_eq!(gw.urls(), &["ws://127.0.0.1:8545".to_string()]);
    }

    #[tokio::test]
    async fn custom_gateways_override() {
        let manager = GatewayManager::new();
        let urls = vec![
            "ws://example.com".to_string(),
            "ws://backup.example.com".to_string(),
        ];
        let gw = manager.gateways_for_event(31337, &urls).await.unwrap();
        assert_eq!(gw.urls(), urls);
    }

    #[tokio::test]
    async fn chainlist_resolves_known_chain() {
        let manager = GatewayManager::new();
        let gw = manager.get_gateway(1).await.unwrap();
        let url = gw.urls().first().unwrap();
        assert!(url.starts_with("wss://") || url.starts_with("ws://"));
    }
}
