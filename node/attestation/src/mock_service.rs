// Mock Attestation Service for Testing
// Implements the same interface as the real service but uses local verification

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use nxcc_interface::types::attestation::{AttestationBundle, RawAttestation};

use crate::{
    tdx::{TdxAttestationClaims, TdxQuote},
    user_data_binding, AttestationProvider, Measurement, StandardizedClaims, VerificationResult,
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
        _bundle: &AttestationBundle,
    ) -> Result<StandardizedClaims> {
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

        // Convert runtime measurements from HashMap to Vec<Measurement> using exact field names
        let rtmr_measurements: Vec<Measurement> = runtime_measurements
            .into_iter()
            .map(|(key, value)| Measurement {
                val: value,
                alg: "sha-384".to_string(), // Use exact algorithm name from spec
                measurement_type: Some(key),
                vendor: Some("mock".to_string()),
                version: None,
            })
            .collect();

        // Add the primary software measurement (MRTD)
        let mut all_measurements = vec![Measurement {
            val: tdx_claims.mrtd.clone(),
            alg: "sha-384".to_string(),
            measurement_type: Some("application".to_string()), // Primary enclave measurement
            vendor: Some("mock".to_string()),
            version: None,
        }];
        all_measurements.extend(rtmr_measurements);

        Ok(StandardizedClaims {
            // Core freshness and context
            iat: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            eat_nonce: Some(tdx_claims.report_data.clone()),

            // Identity and provenance
            ueid: Some(tdx_claims.mrtd[0..32.min(tdx_claims.mrtd.len())].to_vec()),
            sueids: None,
            oemid: Some("mock-intel-tdx".to_string()),
            hwmodel: Some("mock-tdx".to_string()),
            hwversion: Some("1.0".to_string()),

            // Debug and boot status
            dbgstat: if tdx_claims.debug_enabled { 4 } else { 0 }, // 4=enabled, 0=disabled (production)
            oemboot: Some(true),

            // Software identity
            swname: Some("mock-enclave".to_string()),
            swversion: Some("1.0".to_string()),
            manifests: None,

            // Measurements and results (required - at least one)
            measurements: all_measurements,
            measres: None,

            // Execution structure breakdown
            submods: None, // Could group RTMRs here for better structure

            // Key binding
            cnf: None,    // Could be populated with ephemeral key
            intuse: None, // Would be 5 if cnf is present

            // Lifecycle freshness
            uptime: None,
            bootcount: None,
            bootseed: None,

            // Profile selection (required)
            eat_profile: "urn:nxcc:profile:tdx-v1".to_string(),

            // Assurance artifacts
            dloas: None,
        })
    }
}

#[async_trait]
impl AttestationProvider for MockTdxProvider {
    fn platform_type(&self) -> &str {
        "tdx"
    }

    fn is_available(&self) -> bool {
        true
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

    async fn generate_attestation(&self, userdata_hash: &[u8]) -> Result<RawAttestation> {
        // Generate mock TDX quote with user data
        let quote_data = self.generate_mock_quote(userdata_hash)?;

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
        let quote = match TdxQuote::parse(&bundle.raw_attestation.evidence) {
            Ok(quote) => quote,
            Err(e) => {
                return Ok(VerificationResult::Failed(format!(
                    "Quote parsing failed: {}",
                    e
                )))
            }
        };

        // Verify quote structure
        if let Err(e) = quote.verify_structure() {
            return Ok(VerificationResult::Failed(format!(
                "Quote structure invalid: {}",
                e
            )));
        }

        // Extract TDX-specific claims
        let tdx_claims = quote.extract_claims();

        // Verify userdata binding
        let received_userdata_hash = user_data_binding::hash_userdata(&bundle.detached_userdata);
        if tdx_claims.report_data.len() < 32
            || tdx_claims.report_data[..32] != received_userdata_hash[..]
        {
            return Ok(VerificationResult::Failed(
                "Userdata hash mismatch".to_string(),
            ));
        }

        // Convert to standardized claims
        match self.extract_standardized_claims(&tdx_claims, bundle) {
            Ok(claims) => Ok(VerificationResult::Verified(Box::new(claims))),
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
    pub async fn generate_attestation(&self) -> Result<AttestationBundle> {
        let user_data_payload = user_data_binding::UserData {
            ephemeral_public_key: vec![0x42; 32],
            block_hashes: vec![],
        };
        let detached_userdata = user_data_payload.to_cbor()?;
        let userdata_hash = user_data_binding::hash_userdata(&detached_userdata);

        let raw_attestation = self.provider.generate_attestation(&userdata_hash).await?;

        Ok(AttestationBundle {
            raw_attestation,
            detached_userdata,
        })
    }

    /// Verify attestation using mock provider
    pub async fn verify_attestation(
        &self,
        bundle: &AttestationBundle,
    ) -> Result<StandardizedClaims> {
        match self.provider.verify_attestation(bundle).await? {
            VerificationResult::Verified(claims) => Ok(*claims),
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

/// Test provider for non-TDX specific tests
#[cfg(test)]
pub struct TestProvider;

#[cfg(test)]
#[async_trait]
impl AttestationProvider for TestProvider {
    fn platform_type(&self) -> &str {
        "test"
    }

    fn is_available(&self) -> bool {
        true
    }

    fn max_user_data_size(&self) -> usize {
        64
    }

    async fn update_config(&mut self, _config_json: &str) -> Result<()> {
        Ok(())
    }

    async fn generate_attestation(&self, userdata_hash: &[u8]) -> Result<RawAttestation> {
        let mut evidence = b"test-evidence-".to_vec();
        evidence.extend_from_slice(userdata_hash);
        Ok(RawAttestation {
            platform_type: "test".to_string(),
            evidence,
            certificates: None,
        })
    }

    async fn verify_attestation(&self, bundle: &AttestationBundle) -> Result<VerificationResult> {
        if bundle.raw_attestation.platform_type != "test" {
            return Ok(VerificationResult::Unsupported);
        }

        let received_userdata_hash = user_data_binding::hash_userdata(&bundle.detached_userdata);
        let expected_prefix = b"test-evidence-";
        if !bundle.raw_attestation.evidence.starts_with(expected_prefix) {
            return Ok(VerificationResult::Failed(
                "Invalid evidence prefix".to_string(),
            ));
        }
        let evidence_hash = &bundle.raw_attestation.evidence[expected_prefix.len()..];

        if evidence_hash != received_userdata_hash {
            return Ok(VerificationResult::Failed(
                "Userdata hash mismatch".to_string(),
            ));
        }

        let claims = StandardizedClaims {
            iat: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            eat_nonce: Some(received_userdata_hash.to_vec()),
            ueid: Some(vec![0xDE; 32]),
            oemid: Some("test-platform".to_string()),
            hwmodel: Some("test-hw".to_string()),
            hwversion: Some("1.0".to_string()),
            dbgstat: 0,
            measurements: vec![Measurement {
                val: vec![0xAD; 32],
                alg: "sha-256".to_string(),
                measurement_type: Some("application".to_string()),
                vendor: Some("test".to_string()),
                version: None,
            }],
            eat_profile: "urn:nxcc:profile:test-v1".to_string(),
            sueids: None,
            oemboot: None,
            swname: None,
            swversion: None,
            manifests: None,
            measres: None,
            submods: None,
            cnf: None,
            intuse: None,
            uptime: None,
            bootcount: None,
            bootseed: None,
            dloas: None,
        };

        Ok(VerificationResult::Verified(Box::new(claims)))
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
        let parsed_quote = TdxQuote::parse(&quote).unwrap();
        let claims = parsed_quote.extract_claims();

        // Verify user data was embedded
        let extracted_msg = TdxQuote::extract_user_message(&claims.report_data);
        assert!(extracted_msg.starts_with("test message"));
    }

    #[tokio::test]
    async fn test_mock_attestation_flow() {
        let service = MockAttestationService::new();

        // Generate attestation
        let bundle = service.generate_attestation().await.unwrap();
        assert_eq!(bundle.raw_attestation.platform_type, "tdx");

        // Verify attestation
        let claims = service.verify_attestation(&bundle).await.unwrap();
        assert_eq!(claims.oemid, Some("mock-intel-tdx".to_string()));
        assert_eq!(claims.dbgstat, 0); // Production (debug disabled)
        assert!(!claims.measurements.is_empty()); // Ensure measurements are present
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
        let bundle = service.generate_attestation().await.unwrap();
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
        let bundle = service.generate_attestation().await.unwrap();
        let claims = service.verify_attestation(&bundle).await.unwrap();

        // Configure with the actual MRTD as expected
        let config = serde_json::json!({
            "expected_measurements": {
                "mrtd": hex::encode(
                    claims.measurements.iter()
                        .find(|m| m.measurement_type.as_ref().is_some_and(|t| t.contains("application") || t.contains("mrtd")))
                        .map(|m| &m.val)
                        .unwrap_or(&vec![0u8; 48])
                )
            }
        });

        service.update_config(&config.to_string()).await.unwrap();

        // Verification should succeed now
        let verified_claims = service.verify_attestation(&bundle).await.unwrap();
        // Verify the measurements are consistent
        assert_eq!(
            verified_claims.measurements.len(),
            claims.measurements.len()
        );
    }
}
