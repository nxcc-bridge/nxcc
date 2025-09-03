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
pub use nxcc_interface::types::{AttestationBundle, RawAttestation, UserDataBinding};
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
    async fn generate_attestation(
        &self,
        user_data_binding: &UserDataBinding,
    ) -> Result<RawAttestation>;

    /// Verify attestation and extract claims
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
    pub async fn generate_attestation(&self, user_data: Vec<u8>) -> Result<AttestationBundle> {
        // Auto-detect platform type (for now, assume TDX)
        let platform_type = "tdx";

        let providers = self
            .providers
            .get(platform_type)
            .ok_or_else(|| AttestationError::NoProvidersAvailable(platform_type.to_string()))?;

        if providers.is_empty() {
            return Err(AttestationError::NoProvidersAvailable(platform_type.to_string()).into());
        }

        // Use the first provider for generation
        let provider = &providers[0];
        let max_size = provider.max_user_data_size();
        let user_data_binding = UserDataBinding::new(user_data, max_size);

        // Fetch freshness proof
        let freshness_proof = self.freshness_service.fetch_freshness_proof().await?;

        // Generate raw attestation
        let raw_attestation = provider.generate_attestation(&user_data_binding).await?;

        Ok(AttestationBundle {
            raw_attestation,
            user_data_binding,
            block_hashes: freshness_proof.blocks,
        })
    }

    /// Generate attestation with explicit ephemeral key binding
    pub async fn generate_attestation_with_ephemeral_key(
        &self,
        ephemeral_key: &[u8],
        user_data: Vec<u8>,
    ) -> Result<AttestationBundle> {
        // Auto-detect platform type (for now, assume TDX)
        let platform_type = "tdx";

        let providers = self
            .providers
            .get(platform_type)
            .ok_or_else(|| AttestationError::NoProvidersAvailable(platform_type.to_string()))?;

        if providers.is_empty() {
            return Err(AttestationError::NoProvidersAvailable(platform_type.to_string()).into());
        }

        // Fetch freshness proof first
        let freshness_proof = self.freshness_service.fetch_freshness_proof().await?;

        // Combine user data with freshness proof for binding
        let mut combined_user_data = user_data;
        combined_user_data.extend_from_slice(freshness_proof.get_compact_representation());

        // Use the first provider for generation
        let provider = &providers[0];
        let max_size = provider.max_user_data_size();
        let user_data_binding = UserDataBinding::new_with_ephemeral_key(
            ephemeral_key.to_vec(),
            combined_user_data,
            max_size,
        );

        // Generate raw attestation
        let raw_attestation = provider.generate_attestation(&user_data_binding).await?;

        Ok(AttestationBundle {
            raw_attestation,
            user_data_binding,
            block_hashes: freshness_proof.blocks,
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
                    // Verify user data binding
                    if !bundle.user_data_binding.verify_binding() {
                        return Err(AttestationError::VerificationFailed(
                            "User data binding verification failed".to_string(),
                        )
                        .into());
                    }

                    // Verify block hash freshness
                    if !bundle.block_hashes.is_empty() {
                        let freshness_proof = FreshnessProof::new(
                            bundle.block_hashes.clone(),
                            self.freshness_service.config().clone(),
                        );
                        if let Err(e) = self
                            .freshness_service
                            .verify_freshness_proof(&freshness_proof)
                            .await
                        {
                            tracing::warn!("Freshness verification failed: {}", e);
                            // Continue verification but log warning
                        }
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
