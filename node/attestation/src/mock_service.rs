// Mock Attestation Service for Testing
// Implements the same interface as the real service but uses local verification

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use async_trait::async_trait;

use crate::{
    tdx::parser::{TdxAttestationClaims, TdxParser},
    types::*,
    AttestationProvider,
};

/// Mock TDX provider that uses local parsing for testing
pub struct MockTdxProvider {
    config: MockTdxConfig,
}

#[derive(Debug, Clone)]
pub struct MockTdxConfig {
    pub require_debug_disabled: bool,
    pub expected_measurements: HashMap<String, Vec<u8>>,
    pub simulate_failures: Vec<String>,
}

impl Default for MockTdxConfig {
    fn default() -> Self {
        Self {
            require_debug_disabled: true,
            expected_measurements: HashMap::new(),
            simulate_failures: Vec::new(),
        }
    }
}

impl Default for MockTdxProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MockTdxProvider {
    pub fn new() -> Self {
        Self {
            config: MockTdxConfig::default(),
        }
    }

    pub fn new_with_config(config: MockTdxConfig) -> Self {
        Self { config }
    }

    /// Generate a mock TDX quote with embedded user data
    pub fn generate_mock_quote(&self, user_data: &[u8]) -> Result<Vec<u8>> {
        // Use real TDX quote as base and modify the report data
        let base_quote = std::fs::read("test_data/real_tdx_quote.bin")
            .expect("Failed to load real TDX quote from test_data/real_tdx_quote.bin");

        let mut mock_quote = base_quote;

        // Report data is at the end of TD Report structure
        // Header(48) + TD Report starts with: TCB SVN(16) + MR SEAM(48) + MR SIGNER SEAM(48) +
        // SEAM Attributes(8) + TD Attributes(8) + XFAM(8) + MRTD(48) + MR Config ID(48) +
        // MR Owner(48) + MR Owner Config(48) + 4 RTMRs(48*4=192) = 520 bytes before report data
        let report_data_offset = 48 + 520;
        let mut report_data = [0u8; 64];

        // Copy user data up to 64 bytes
        let copy_len = std::cmp::min(user_data.len(), 64);
        report_data[..copy_len].copy_from_slice(&user_data[..copy_len]);

        // Update the quote with new report data
        mock_quote[report_data_offset..report_data_offset + 64].copy_from_slice(&report_data);

        Ok(mock_quote)
    }

    /// Extract standardized claims from TDX attestation claims
    fn extract_standardized_claims(
        &self,
        tdx_claims: &TdxAttestationClaims,
        user_data_binding: &UserDataBinding,
    ) -> Result<StandardizedClaims> {
        // Verify user data binding
        if !user_data_binding.verify_binding() {
            return Err(anyhow!("User data binding verification failed"));
        }

        // Check if we should simulate failures
        for failure_type in &self.config.simulate_failures {
            match failure_type.as_str() {
                "debug_enabled" if !tdx_claims.debug_enabled => {
                    return Err(anyhow!("Simulated debug check failure"));
                }
                "measurement_mismatch" => {
                    return Err(anyhow!("Simulated measurement mismatch"));
                }
                "signature_invalid" => {
                    return Err(anyhow!("Simulated signature verification failure"));
                }
                _ => {}
            }
        }

        // Check debug requirements
        if self.config.require_debug_disabled && tdx_claims.debug_enabled {
            return Err(anyhow!(
                "Debug mode is enabled but policy requires it disabled"
            ));
        }

        // Check expected measurements
        for (measurement_name, expected_value) in &self.config.expected_measurements {
            let actual_value = match measurement_name.as_str() {
                "mrtd" => &tdx_claims.mrtd,
                "rtmr0" => &tdx_claims.rtmr0,
                "rtmr1" => &tdx_claims.rtmr1,
                "rtmr2" => &tdx_claims.rtmr2,
                "rtmr3" => &tdx_claims.rtmr3,
                _ => continue,
            };

            if actual_value != expected_value {
                return Err(anyhow!(
                    "Measurement mismatch for {}: expected {}, got {}",
                    measurement_name,
                    hex::encode(expected_value),
                    hex::encode(actual_value)
                ));
            }
        }

        // Create runtime measurements map
        let mut runtime_measurements = HashMap::new();
        runtime_measurements.insert("rtmr0".to_string(), tdx_claims.rtmr0.clone());
        runtime_measurements.insert("rtmr1".to_string(), tdx_claims.rtmr1.clone());
        runtime_measurements.insert("rtmr2".to_string(), tdx_claims.rtmr2.clone());
        runtime_measurements.insert("rtmr3".to_string(), tdx_claims.rtmr3.clone());
        runtime_measurements.insert("mr_config_id".to_string(), tdx_claims.mr_config_id.clone());
        runtime_measurements.insert("mr_owner".to_string(), tdx_claims.mr_owner.clone());
        runtime_measurements.insert("mr_seam".to_string(), tdx_claims.mr_seam.clone());

        Ok(StandardizedClaims {
            software_component: tdx_claims.mrtd.clone(),
            hardware_security_level: if tdx_claims.debug_enabled { 1 } else { 3 },
            security_version_number: u64::from_le_bytes(
                tdx_claims.tcb_svn[..8].try_into().unwrap_or([0; 8]),
            ),
            unique_entity_id: tdx_claims.mrtd[0..32.min(tdx_claims.mrtd.len())].to_vec(),
            nonce: user_data_binding.embedded_hash.clone(),
            issued_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            measurements: runtime_measurements,
            oem_id: "mock-intel-tdx".to_string(),
        })
    }
}

#[async_trait]
impl AttestationProvider for MockTdxProvider {
    fn platform_type(&self) -> &str {
        "tdx"
    }

    fn max_user_data_size(&self) -> usize {
        64 // TDX report data size
    }

    async fn update_config(&mut self, config_json: &str) -> Result<()> {
        // Parse configuration for testing scenarios
        let config: serde_json::Value = serde_json::from_str(config_json)?;

        if let Some(require_debug_disabled) = config.get("require_debug_disabled") {
            self.config.require_debug_disabled = require_debug_disabled.as_bool().unwrap_or(true);
        }

        if let Some(failures) = config.get("simulate_failures") {
            if let Some(failures_array) = failures.as_array() {
                self.config.simulate_failures = failures_array
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
            }
        }

        if let Some(measurements) = config.get("expected_measurements") {
            if let Some(measurements_obj) = measurements.as_object() {
                for (key, value) in measurements_obj {
                    if let Some(hex_str) = value.as_str() {
                        if let Ok(bytes) = hex::decode(hex_str) {
                            self.config.expected_measurements.insert(key.clone(), bytes);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn generate_attestation(
        &self,
        user_data_binding: &UserDataBinding,
    ) -> Result<RawAttestation> {
        // Generate mock TDX quote with user data
        let quote_data = self.generate_mock_quote(&user_data_binding.embedded_hash)?;

        Ok(RawAttestation {
            platform_type: "tdx".to_string(),
            evidence: quote_data,
            certificates: None, // Mock provider doesn't include cert chains
        })
    }

    async fn verify_attestation(&self, bundle: &AttestationBundle) -> Result<VerificationResult> {
        // Check if this is a TDX attestation
        if bundle.raw_attestation.platform_type != "tdx" {
            return Ok(VerificationResult::Unsupported);
        }

        // Parse the TDX quote using our working parser
        let quote = match TdxParser::parse_quote(&bundle.raw_attestation.evidence) {
            Ok(quote) => quote,
            Err(e) => {
                return Ok(VerificationResult::Failed(format!(
                    "Quote parsing failed: {}",
                    e
                )))
            }
        };

        // Verify quote structure
        if let Err(e) = TdxParser::verify_quote_structure(&quote) {
            return Ok(VerificationResult::Failed(format!(
                "Quote structure invalid: {}",
                e
            )));
        }

        // Extract TDX-specific claims
        let tdx_claims = TdxParser::extract_claims(&quote);

        // Convert to standardized claims
        match self.extract_standardized_claims(&tdx_claims, &bundle.user_data_binding) {
            Ok(claims) => Ok(VerificationResult::Verified(claims)),
            Err(e) => Ok(VerificationResult::Failed(e.to_string())),
        }
    }
}

/// Mock attestation service for testing
pub struct MockAttestationService {
    provider: MockTdxProvider,
}

impl Default for MockAttestationService {
    fn default() -> Self {
        Self::new()
    }
}

impl MockAttestationService {
    pub fn new() -> Self {
        Self {
            provider: MockTdxProvider::new(),
        }
    }

    pub fn new_with_config(config: MockTdxConfig) -> Self {
        Self {
            provider: MockTdxProvider::new_with_config(config),
        }
    }

    /// Generate attestation using mock provider
    pub async fn generate_attestation(&self, user_data: Vec<u8>) -> Result<AttestationBundle> {
        let max_size = self.provider.max_user_data_size();
        let user_data_binding = UserDataBinding::new(user_data, max_size);

        let raw_attestation = self
            .provider
            .generate_attestation(&user_data_binding)
            .await?;

        Ok(AttestationBundle {
            raw_attestation,
            user_data_binding,
            block_hashes: Vec::new(), // Mock service doesn't use block hashes
        })
    }

    /// Verify attestation using mock provider
    pub async fn verify_attestation(
        &self,
        bundle: &AttestationBundle,
    ) -> Result<StandardizedClaims> {
        match self.provider.verify_attestation(bundle).await? {
            VerificationResult::Verified(claims) => Ok(claims),
            VerificationResult::Unsupported => Err(anyhow!(
                "Mock provider does not support this attestation type"
            )),
            VerificationResult::Failed(error) => {
                Err(anyhow!("Attestation verification failed: {}", error))
            }
        }
    }

    /// Update provider configuration
    pub async fn update_config(&mut self, config_json: &str) -> Result<()> {
        self.provider.update_config(config_json).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_quote_generation() {
        let provider = MockTdxProvider::new();
        let user_data = b"test message for mock quote";

        let quote = provider.generate_mock_quote(user_data).unwrap();
        assert!(quote.len() > 600); // Should be similar to real quote size

        // Parse the generated quote
        let parsed_quote = TdxParser::parse_quote(&quote).unwrap();
        let claims = TdxParser::extract_claims(&parsed_quote);

        // Verify user data was embedded
        let extracted_msg = TdxParser::extract_user_message(&claims.report_data);
        assert!(extracted_msg.starts_with("test message"));
    }

    #[tokio::test]
    async fn test_mock_attestation_flow() {
        let service = MockAttestationService::new();
        let user_data = b"Hello, mock attestation!".to_vec();

        // Generate attestation
        let bundle = service
            .generate_attestation(user_data.clone())
            .await
            .unwrap();
        assert_eq!(bundle.raw_attestation.platform_type, "tdx");
        assert!(bundle.user_data_binding.verify_binding());

        // Verify attestation
        let claims = service.verify_attestation(&bundle).await.unwrap();
        assert_eq!(claims.oem_id, "mock-intel-tdx");
        assert_eq!(claims.hardware_security_level, 3); // Production (not debug)
        assert!(!claims.software_component.iter().all(|&b| b == 0));
    }

    #[tokio::test]
    async fn test_mock_config_and_failures() {
        let mut service = MockAttestationService::new();

        // Configure to simulate a measurement mismatch
        let config = serde_json::json!({
            "simulate_failures": ["measurement_mismatch"],
            "require_debug_disabled": true
        });

        service.update_config(&config.to_string()).await.unwrap();

        // Generate and try to verify - should fail
        let bundle = service
            .generate_attestation(b"test".to_vec())
            .await
            .unwrap();
        let result = service.verify_attestation(&bundle).await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("measurement mismatch"));
    }

    #[tokio::test]
    async fn test_mock_expected_measurements() {
        let mut service = MockAttestationService::new();

        // First generate an attestation to get the actual MRTD
        let bundle = service
            .generate_attestation(b"test".to_vec())
            .await
            .unwrap();
        let claims = service.verify_attestation(&bundle).await.unwrap();

        // Configure with the actual MRTD as expected
        let config = serde_json::json!({
            "expected_measurements": {
                "mrtd": hex::encode(&claims.software_component)
            }
        });

        service.update_config(&config.to_string()).await.unwrap();

        // Verification should succeed now
        let verified_claims = service.verify_attestation(&bundle).await.unwrap();
        assert_eq!(
            verified_claims.software_component,
            claims.software_component
        );
    }

    #[tokio::test]
    async fn test_large_user_data_hashing() {
        let service = MockAttestationService::new();
        let large_data = vec![0x42; 1000]; // 1KB of data, exceeds 64-byte limit

        let bundle = service
            .generate_attestation(large_data.clone())
            .await
            .unwrap();

        // Should have been hashed due to size
        assert!(bundle.user_data_binding.was_hashed);
        assert_eq!(bundle.user_data_binding.original_data, large_data);
        assert_eq!(bundle.user_data_binding.embedded_hash.len(), 32); // SHA256 hash

        // Verification should still work
        let claims = service.verify_attestation(&bundle).await.unwrap();
        assert_eq!(
            claims.nonce,
            bundle.user_data_binding.embedded_hash
        );
    }
}
