use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;

use crate::freshness::{FreshnessProof, FreshnessService};

pub mod error;
pub mod freshness;
pub mod mock_service;
pub mod providers;
pub mod tdx;
pub mod types;
pub mod user_data_binding;

#[cfg(test)]
mod integration_tests;

pub use error::AttestationError;
// Re-export gateway types from interface crate
pub use nxcc_interface::gateway::{BlockInfo, GatewayConfig, GatewayProvider};
// Re-export attestation types from interface crate
pub use nxcc_interface::types::attestation::{AttestationBundle, RawAttestation};
pub use types::*;

/// Platform-specific attestation provider
#[async_trait]
pub trait AttestationProvider: Send + Sync {
    /// Platform type this provider handles
    fn platform_type(&self) -> &str;

    /// Maximum user data size for this platform
    fn max_user_data_size(&self) -> usize;

    /// Update provider configuration (called from enclave runtime)
    async fn update_config(&mut self, config_json: &str) -> Result<()>;

    /// Generate attestation with user data binding
    async fn generate_attestation(&self, userdata_hash: &[u8]) -> Result<RawAttestation>;

    /// Verify attestation and extract claims
    /// This method is responsible for verifying both the quote and the userdata binding.
    async fn verify_attestation(&self, bundle: &AttestationBundle) -> Result<VerificationResult>;
}

/// Multi-provider attestation service with fallback chains
pub struct AttestationService {
    providers: HashMap<String, Vec<Box<dyn AttestationProvider>>>,
    _gateway_provider: Arc<dyn GatewayProvider>,
    freshness_service: FreshnessService,
}

impl AttestationService {
    pub fn new(gateway_provider: Arc<dyn GatewayProvider>) -> Self {
        let freshness_service = FreshnessService::new(gateway_provider.clone());
        Self {
            providers: HashMap::new(),
            _gateway_provider: gateway_provider,
            freshness_service,
        }
    }

    /// Create AttestationService with custom freshness configuration
    pub fn new_with_config(
        gateway_provider: Arc<dyn GatewayProvider>,
        freshness_config: freshness::FreshnessConfig,
    ) -> Self {
        let freshness_service =
            FreshnessService::new_with_config(gateway_provider.clone(), freshness_config);
        Self {
            providers: HashMap::new(),
            _gateway_provider: gateway_provider,
            freshness_service,
        }
    }

    /// Register a provider for a specific platform type
    pub fn register_provider(
        &mut self,
        platform_type: String,
        provider: Box<dyn AttestationProvider>,
    ) {
        self.providers
            .entry(platform_type)
            .or_default()
            .push(provider);
    }

    /// Generate attestation (auto-detects platform)
    pub async fn generate_attestation(&self, ephemeral_key: &[u8]) -> Result<AttestationBundle> {
        // TODO: Auto-detect platform type
        let platform_type = "tdx";

        let providers = self
            .providers
            .get(platform_type)
            .ok_or_else(|| AttestationError::NoProvidersAvailable(platform_type.to_string()))?;

        if providers.is_empty() {
            return Err(AttestationError::NoProvidersAvailable(platform_type.to_string()).into());
        }

        // Fetch freshness proof
        let freshness_proof = self.freshness_service.fetch_freshness_proof().await?;

        let user_data_payload =
            user_data_binding::UserData::new(ephemeral_key.to_vec(), freshness_proof.blocks);

        let detached_userdata = user_data_payload.to_cbor()?;
        let userdata_hash = user_data_binding::hash_userdata(&detached_userdata);

        // Generate raw attestation
        let provider = &providers[0];
        let raw_attestation = provider.generate_attestation(&userdata_hash).await?;

        Ok(AttestationBundle {
            raw_attestation,
            detached_userdata,
        })
    }

    /// Verify attestation bundle with fallback chain
    pub async fn verify_attestation(
        &self,
        bundle: &AttestationBundle,
    ) -> Result<Box<StandardizedClaims>> {
        let platform_type = &bundle.raw_attestation.platform_type;

        let providers = self
            .providers
            .get(platform_type)
            .ok_or_else(|| AttestationError::NoProvidersAvailable(platform_type.clone()))?;

        let mut last_error = None;

        // Try each provider in order
        for provider in providers {
            match provider.verify_attestation(bundle).await {
                Ok(VerificationResult::Verified(claims)) => {
                    // The provider has already verified the quote and the binding of the detached_userdata.
                    // Now, we can trust the contents of detached_userdata and verify freshness.
                    let userdata =
                        user_data_binding::UserData::from_cbor(&bundle.detached_userdata)?;

                    if !userdata.block_hashes.is_empty() {
                        let freshness_proof = FreshnessProof::new(
                            userdata.block_hashes,
                            self.freshness_service.config().clone(),
                        );
                        self.freshness_service
                            .verify_freshness_proof(&freshness_proof)
                            .await
                            .map_err(|e| {
                                AttestationError::VerificationFailed(format!(
                                    "Freshness proof verification failed: {}",
                                    e
                                ))
                            })?;
                    } else if self.freshness_service.config().enabled {
                        return Err(AttestationError::VerificationFailed(
                            "Freshness proof missing but required by policy".to_string(),
                        )
                        .into());
                    } else {
                        tracing::warn!("No block hashes provided for freshness verification");
                    }

                    return Ok(claims);
                }
                Ok(VerificationResult::Unsupported) => {
                    // Try next provider
                    continue;
                }
                Ok(VerificationResult::Failed(error)) => {
                    last_error = Some(error);
                    // Definitive failure, don't try other providers
                    break;
                }
                Err(e) => {
                    last_error = Some(e.to_string());
                    continue;
                }
            }
        }

        let error_msg = last_error.unwrap_or_else(|| "All providers failed".to_string());
        Err(AttestationError::AllProvidersFailed(error_msg).into())
    }

    /// Update provider configuration
    pub async fn update_provider_config(
        &mut self,
        platform_type: &str,
        _config_json: &str,
    ) -> Result<()> {
        if let Some(_providers) = self.providers.get_mut(platform_type) {
            // Update all providers for this platform type
            // Note: We can't call mutable methods on trait objects in a Vec
            // This is a design limitation that would need to be addressed
            // For now, we'll implement a different approach in the concrete implementation
            tracing::warn!("Provider config update not implemented for trait objects");
        }
        Ok(())
    }

    /// Get access to the freshness service for configuration
    pub fn freshness_service(&mut self) -> &mut FreshnessService {
        &mut self.freshness_service
    }
}
