use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use nxcc_attestation::{
    AttestationBundle, AttestationProvider, AttestationService, GatewayProvider, RawAttestation,
    StandardizedClaims, VerificationResult, providers::TdxQvlProvider, user_data_binding,
};
use nxcc_interface::gateway::{BlockInfo, GatewayConfig};

use crate::crypto::KeyExchangeKeyPair;

/// Test provider for platform type "test" used in enclave tests
struct TestAttestationProvider;

#[async_trait]
impl AttestationProvider for TestAttestationProvider {
    fn platform_type(&self) -> &str {
        "test"
    }

    fn max_user_data_size(&self) -> usize {
        64
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn update_config(&mut self, _config_json: &str) -> Result<()> {
        Ok(())
    }

    async fn generate_attestation(&self, _userdata_hash: &[u8]) -> Result<RawAttestation> {
        Ok(RawAttestation {
            platform_type: "test".to_string(),
            evidence: vec![0u8; 32],
            certificates: None,
        })
    }

    async fn verify_attestation(&self, bundle: &AttestationBundle) -> Result<VerificationResult> {
        if bundle.raw_attestation.platform_type != "test" {
            return Ok(VerificationResult::Unsupported);
        }

        // Try to parse the userdata to detect invalid CBOR
        match user_data_binding::UserData::from_cbor(&bundle.detached_userdata) {
            Ok(_) => {
                // For valid CBOR, return a basic verified result
                let claims = Box::new(StandardizedClaims {
                    // Core freshness and context
                    iat: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    eat_nonce: None,

                    // Identity and provenance
                    ueid: Some(vec![0x42; 32]), // Test UUID
                    sueids: None,
                    oemid: Some("test".to_string()),
                    hwmodel: Some("test-model".to_string()),
                    hwversion: Some("1.0".to_string()),

                    // Debug and boot status
                    dbgstat: 0, // Production (debug disabled)
                    oemboot: None,

                    // Software identity
                    swname: None,
                    swversion: None,
                    manifests: None,

                    // Measurements
                    measurements: vec![],
                    measres: None,

                    // Execution structure
                    submods: None,

                    // Key binding
                    cnf: None,
                    intuse: None,

                    // Lifecycle freshness
                    uptime: None,
                    bootcount: None,
                    bootseed: None,

                    // Profile selection
                    eat_profile: "test-profile".to_string(),

                    // Assurance artifacts
                    dloas: None,
                });
                Ok(VerificationResult::Verified(claims))
            }
            Err(_) => Ok(VerificationResult::Failed(
                "Failed to parse requester userdata".to_string(),
            )),
        }
    }
}

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

        // Register test provider for enclave tests (used by test platform type)
        service.register_provider("test".to_string(), Box::new(TestAttestationProvider));

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
