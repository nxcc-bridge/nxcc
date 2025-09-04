use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use nxcc_attestation::{
    AttestationProvider, AttestationService, GatewayProvider,
    providers::{NullAttestationProvider, TdxQvlProvider},
    user_data_binding,
};
use nxcc_interface::{
    gateway::{BlockInfo, GatewayConfig},
    types::attestation::{
        AttestationBundle, RawAttestation, StandardizedClaims, VerificationResult,
    },
};

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

        // Register TDX QVL provider
        service.register_provider("tdx".to_string(), Box::new(TdxQvlProvider::new()));

        // Register null provider as fallback
        service.register_provider("null".to_string(), Box::new(NullAttestationProvider::new()));

        Ok(Self {
            service,
            ephemeral_kx_keypair,
        })
    }

    /// Configure attestation providers from runtime config
    pub async fn configure_providers(&mut self, config: HashMap<String, String>) -> Result<()> {
        if let Some(qvl_config) = config.get("qvl") {
            self.service
                .update_provider_config("tdx", qvl_config)
                .await?;
        }
        Ok(())
    }

    /// Generate attestation with user data + ephemeral key binding
    pub async fn generate_attestation(&self) -> Result<AttestationBundle> {
        self.service
            .generate_attestation(self.ephemeral_kx_keypair.public_key().as_bytes())
            .await
    }

    /// Verify attestation and return standardized claims
    pub async fn verify_and_extract_claims(
        &self,
        bundle: &AttestationBundle,
    ) -> Result<Box<StandardizedClaims>> {
        self.service.verify_attestation(bundle).await
    }

    /// Get access to the underlying attestation service
    pub fn attestation_service(&self) -> &AttestationService {
        &self.service
    }
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
            "qvl".to_string(),
            r#"{"pccs_url": "https://api.trustedservices.intel.com/tdx/certification/v4"}"#
                .to_string(),
        );

        let result = manager.configure_providers(config).await;
        assert!(result.is_ok());
    }
}
