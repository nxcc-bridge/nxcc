use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use nxcc_attestation::{
    AttestationBundle, AttestationService, GatewayProvider, StandardizedClaims,
    providers::{TdxGcsRemoteProvider, TdxLocalProvider},
};
use nxcc_interface::gateway::{BlockInfo, GatewayConfig};

use crate::crypto::KeyExchangeKeyPair;

pub struct PlatformAttestationManager {
    service: AttestationService,
    ephemeral_kx_keypair: Arc<KeyExchangeKeyPair>,
}

impl PlatformAttestationManager {
    pub fn new(
        ephemeral_kx_keypair: Arc<KeyExchangeKeyPair>,
        gateway_provider: Arc<dyn GatewayProvider>,
    ) -> Result<Self> {
        let mut service = AttestationService::new(gateway_provider);

        // Register TDX providers in priority order (local first, then remote)
        service.register_provider("tdx".to_string(), Box::new(TdxLocalProvider::new()));
        service.register_provider("tdx".to_string(), Box::new(TdxGcsRemoteProvider::new()));

        Ok(Self {
            service,
            ephemeral_kx_keypair,
        })
    }

    /// Configure attestation providers from runtime config
    pub async fn configure_providers(&mut self, config: HashMap<String, String>) -> Result<()> {
        if let Some(gcs_config) = config.get("gcs") {
            self.service
                .update_provider_config("tdx", gcs_config)
                .await?;
        }
        Ok(())
    }

    /// Generate attestation with user data + ephemeral key binding
    pub async fn generate_bound_attestation(&self, user_data: &[u8]) -> Result<AttestationBundle> {
        // Use the new ephemeral key binding method
        self.service
            .generate_attestation_with_ephemeral_key(
                self.ephemeral_kx_keypair.public_key().as_bytes(),
                user_data.to_vec(),
            )
            .await
    }

    /// Verify attestation and return standardized claims
    pub async fn verify_and_extract_claims(
        &self,
        bundle: &AttestationBundle,
    ) -> Result<StandardizedClaims> {
        self.service.verify_attestation(bundle).await
    }

    /// Get access to the underlying attestation service
    pub fn attestation_service(&self) -> &AttestationService {
        &self.service
    }
}

// Global instance (to be initialized at startup)
use std::sync::OnceLock;
static PLATFORM_ATTESTATION_MANAGER: OnceLock<PlatformAttestationManager> = OnceLock::new();

/// Initialize the platform attestation manager
pub fn initialize_platform_attestation_manager(
    ephemeral_kx_keypair: Arc<KeyExchangeKeyPair>,
    gateway_provider: Arc<dyn GatewayProvider>,
) -> Result<()> {
    let manager = PlatformAttestationManager::new(ephemeral_kx_keypair, gateway_provider)?;
    PLATFORM_ATTESTATION_MANAGER
        .set(manager)
        .map_err(|_| anyhow::anyhow!("Platform attestation manager already initialized"))?;
    Ok(())
}

/// Get the global platform attestation manager
pub fn get_platform_attestation_manager() -> &'static PlatformAttestationManager {
    PLATFORM_ATTESTATION_MANAGER
        .get()
        .expect("Platform attestation manager not initialized")
}

pub struct MockGatewayProvider;

#[async_trait]
impl GatewayProvider for MockGatewayProvider {
    async fn get_gateways(&self, _chain_id: u64) -> Result<Vec<GatewayConfig>> {
        Ok(vec![])
    }

    async fn add_user_gateway(&self, _gateway: GatewayConfig) -> Result<()> {
        Ok(())
    }

    async fn fetch_latest_block(&self, chain_id: u64) -> Result<BlockInfo> {
        Ok(BlockInfo {
            chain_id,
            chain_name: "Mock Chain".to_string(),
            block_number: 12345,
            block_hash: vec![0u8; 32],
            timestamp: 1234567890,
            fetched_at: 1234567890,
        })
    }

    async fn fetch_multiple_latest_blocks(&self, chain_ids: &[u64]) -> Result<Vec<BlockInfo>> {
        let mut blocks = Vec::new();
        for &chain_id in chain_ids {
            blocks.push(self.fetch_latest_block(chain_id).await?);
        }
        Ok(blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_attestation_manager_creation() {
        let keypair = Arc::new(KeyExchangeKeyPair::generate());
        let gateway_provider = Arc::new(MockGatewayProvider);

        let manager = PlatformAttestationManager::new(keypair, gateway_provider);
        assert!(manager.is_ok());
    }

    #[tokio::test]
    async fn test_config_update() {
        let keypair = Arc::new(KeyExchangeKeyPair::generate());
        let gateway_provider = Arc::new(MockGatewayProvider);
        let mut manager = PlatformAttestationManager::new(keypair, gateway_provider).unwrap();

        let mut config = HashMap::new();
        config.insert(
            "gcs".to_string(),
            r#"{"project_id": "test", "auth_token": "token", "prefer_local_verification": false}"#
                .to_string(),
        );

        let result = manager.configure_providers(config).await;
        assert!(result.is_ok());
    }
}
