use std::{collections::HashMap, sync::Arc};

use alloy_primitives::{Address, U256};
use alloy_provider::{DynProvider, Provider, ProviderBuilder};
use alloy_sol_types::sol;
use alloy_transport_http::Http;
use nxcc_chainlist::{RpcType, get_rpcs_for_chain};
use nxcc_interface::types::secrets::ChainIdentifier;
use tokio::sync::RwLock;
use tracing::debug;
use url::Url;

use crate::error::AppError;

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
    gateways: Arc<RwLock<HashMap<ChainIdentifier, Arc<EventGateway>>>>,
}

impl GatewayManager {
    pub fn new() -> Self {
        Self {
            gateways: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get_gateway(
        &self,
        chain: &ChainIdentifier,
    ) -> Result<Arc<EventGateway>, AppError> {
        match chain {
            ChainIdentifier::GatewayUrl(url) => {
                // Don't cache custom gateway URLs to prevent DoS attacks
                let rpc_urls = vec![url.to_string()];
                Ok(Arc::new(EventGateway::new(rpc_urls)))
            }
            ChainIdentifier::GatewayUrls(urls) => {
                let rpc_urls = urls.iter().map(|url| url.to_string()).collect();
                Ok(Arc::new(EventGateway::new(rpc_urls)))
            }
            ChainIdentifier::ChainId(chain_id) => {
                {
                    let gateways = self.gateways.read().await;
                    if let Some(g) = gateways.get(chain) {
                        return Ok(g.clone());
                    }
                }

                let mut gateways = self.gateways.write().await;
                if let Some(g) = gateways.get(chain) {
                    return Ok(g.clone());
                }

                let rpc_urls = self.get_rpc_urls_for_chain_id(*chain_id)?;
                let gateway = Arc::new(EventGateway::new(rpc_urls));
                gateways.insert(chain.clone(), gateway.clone());
                Ok(gateway)
            }
        }
    }

    /// Returns a gateway for a specific event. If `urls` is empty, the default
    /// gateway for `chain` is used. Otherwise a new gateway with the provided
    /// URLs is created.
    pub async fn gateways_for_event(
        &self,
        chain: &ChainIdentifier,
        urls: &[String],
    ) -> Result<Arc<EventGateway>, AppError> {
        if urls.is_empty() {
            self.get_gateway(chain).await
        } else {
            Ok(Arc::new(EventGateway::new(urls.to_vec())))
        }
    }

    fn get_rpc_urls_for_chain_id(&self, chain_id: u64) -> Result<Vec<String>, AppError> {
        let rpc_url = match chain_id {
            0 => "mock://gateway.example.com".to_string(),
            1337 | 31337 => "ws://127.0.0.1:8545".to_string(), // Ganache/Anvil default
            _ => get_rpcs_for_chain(chain_id, RpcType::Wss)
                .and_then(|mut urls| urls.next())
                .map(|url| url.to_string())
                .ok_or_else(|| {
                    AppError::Service(format!("No RPC URL configured for chain ID {}", chain_id))
                })?,
        };
        Ok(vec![rpc_url])
    }

    pub async fn get_policy_url(
        &self,
        chain: &ChainIdentifier,
        identity_address: Address,
        identity_id: U256,
    ) -> Result<String, AppError> {
        let rpc_urls = match chain {
            ChainIdentifier::GatewayUrl(url) => vec![url.to_string()],
            ChainIdentifier::GatewayUrls(urls) => urls.iter().map(|url| url.to_string()).collect(),
            ChainIdentifier::ChainId(chain_id) => self.get_rpc_urls_for_chain_id(*chain_id)?,
        };

        let rpc_url = rpc_urls
            .first()
            .ok_or_else(|| AppError::Service("No RPC URL available for chain".to_string()))?;

        if rpc_url.starts_with("mock://") {
            debug!("Using mock policy URL for chain={:?}", chain);
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
            "Retrieved policy URL for identity {identity_address:#x} on chain {:?}: {policy_url}",
            chain
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
        let chain = ChainIdentifier::ChainId(31337);
        let gw = manager.get_gateway(&chain).await.unwrap();
        assert_eq!(gw.urls(), &["ws://127.0.0.1:8545".to_string()]);
    }

    #[tokio::test]
    async fn custom_gateways_override() {
        let manager = GatewayManager::new();
        let chain = ChainIdentifier::ChainId(31337);
        let urls = vec![
            "ws://example.com".to_string(),
            "ws://backup.example.com".to_string(),
        ];
        let gw = manager.gateways_for_event(&chain, &urls).await.unwrap();
        assert_eq!(gw.urls(), urls);
    }

    #[tokio::test]
    async fn chainlist_resolves_known_chain() {
        let manager = GatewayManager::new();
        let chain = ChainIdentifier::ChainId(1);
        let gw = manager.get_gateway(&chain).await.unwrap();
        let url = gw.urls().first().unwrap();
        assert!(url.starts_with("wss://") || url.starts_with("ws://"));
    }

    #[tokio::test]
    async fn custom_gateway_url() {
        let manager = GatewayManager::new();
        let custom_url = Url::parse("ws://custom.gateway.com").unwrap();
        let chain = ChainIdentifier::GatewayUrl(custom_url.clone());
        let gw = manager.get_gateway(&chain).await.unwrap();
        assert_eq!(gw.urls(), &[custom_url.to_string()]);
    }

    #[tokio::test]
    async fn custom_gateway_urls() {
        let manager = GatewayManager::new();
        let urls = vec![
            Url::parse("ws://custom-a.gateway.com").unwrap(),
            Url::parse("ws://custom-b.gateway.com").unwrap(),
        ];
        let chain = ChainIdentifier::GatewayUrls(urls.clone());
        let gw = manager.get_gateway(&chain).await.unwrap();
        let expected: Vec<String> = urls.iter().map(|u| u.to_string()).collect();
        assert_eq!(gw.urls(), expected);
    }

    #[tokio::test]
    async fn test_custom_gateway_not_cached() {
        let manager = GatewayManager::new();
        let url1 = "wss://custom1.com".parse().unwrap();
        let url2 = "wss://custom2.com".parse().unwrap();

        let chain1 = ChainIdentifier::GatewayUrl(url1);
        let chain2 = ChainIdentifier::GatewayUrl(url2);

        let gw1 = manager.get_gateway(&chain1).await.unwrap();
        let gw2 = manager.get_gateway(&chain2).await.unwrap();

        // Should be different instances (not cached)
        assert!(!Arc::ptr_eq(&gw1, &gw2));
    }

    #[tokio::test]
    async fn test_custom_gateway_urls_not_cached() {
        let manager = GatewayManager::new();
        let urls_a = vec![
            Url::parse("wss://custom1-a.com").unwrap(),
            Url::parse("wss://custom1-b.com").unwrap(),
        ];
        let urls_b = vec![
            Url::parse("wss://custom2-a.com").unwrap(),
            Url::parse("wss://custom2-b.com").unwrap(),
        ];

        let chain1 = ChainIdentifier::GatewayUrls(urls_a);
        let chain2 = ChainIdentifier::GatewayUrls(urls_b);

        let gw1 = manager.get_gateway(&chain1).await.unwrap();
        let gw2 = manager.get_gateway(&chain2).await.unwrap();

        // Should be different instances (not cached)
        assert!(!Arc::ptr_eq(&gw1, &gw2));
    }

    #[tokio::test]
    async fn test_chain_id_is_cached() {
        let manager = GatewayManager::new();
        let chain = ChainIdentifier::ChainId(1);

        let gw1 = manager.get_gateway(&chain).await.unwrap();
        let gw2 = manager.get_gateway(&chain).await.unwrap();

        // Should be same instance (cached)
        assert!(Arc::ptr_eq(&gw1, &gw2));
    }

    #[tokio::test]
    async fn test_malicious_gateway_urls() {
        let manager = GatewayManager::new();
        let malicious_urls = [
            "javascript:alert('xss')",
            "file:///etc/passwd",
            "ftp://evil.com",
            "data:text/html,<script>alert('xss')</script>",
        ];

        for malicious in malicious_urls {
            if let Ok(url) = Url::parse(malicious) {
                let chain = ChainIdentifier::GatewayUrl(url);
                // Should handle gracefully, not crash
                let result = manager.get_gateway(&chain).await;
                // Depending on implementation, might succeed or fail - but shouldn't panic
                assert!(result.is_ok()); // Our implementation should handle any valid URL
            }
        }
    }

    #[tokio::test]
    async fn test_memory_usage_with_many_custom_gateways() {
        let manager = GatewayManager::new();

        // Create many different custom gateway URLs
        for i in 0..100 {
            // Reduced from 1000 to keep test fast
            let url = format!("wss://custom-{}.com", i).parse().unwrap();
            let chain = ChainIdentifier::GatewayUrl(url);
            let _gateway = manager.get_gateway(&chain).await.unwrap();
        }

        // Verify internal cache doesn't grow (only ChainId variants should be cached)
        let gateways_len = manager.gateways.read().await.len();
        assert_eq!(gateways_len, 0); // No custom URLs should be cached
    }

    #[tokio::test]
    async fn test_policy_url_with_different_chain_types() {
        let manager = GatewayManager::new();

        // Test with chain ID (mock)
        let chain_id = ChainIdentifier::ChainId(0); // Uses mock
        let address = Address::from([1u8; 20]);
        let id = U256::from(123);

        let result = manager.get_policy_url(&chain_id, address, id).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "mock://policy.example.com");

        // Test with custom gateway URL (mock)
        let gateway_url = "mock://custom.gateway.com".parse().unwrap();
        let chain_gateway = ChainIdentifier::GatewayUrl(gateway_url);

        let result = manager.get_policy_url(&chain_gateway, address, id).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "mock://policy.example.com");

        // Test with multiple custom gateway URLs (mock)
        let chain_gateway_multi = ChainIdentifier::GatewayUrls(vec![
            "mock://custom-a.gateway.com".parse().unwrap(),
            "mock://custom-b.gateway.com".parse().unwrap(),
        ]);

        let result = manager
            .get_policy_url(&chain_gateway_multi, address, id)
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "mock://policy.example.com");
    }
}
