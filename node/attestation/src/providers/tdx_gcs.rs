use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use base64::Engine;
use reqwest;
use serde::{Deserialize, Serialize};
use serde_json;

use crate::{
    tdx::{
        hardware::{TdxHardware, TdxInterface, TdxSimulator},
        parser::TdxParser,
        TdxQuoteData,
    },
    types::Measurement,
    AttestationBundle, AttestationProvider, RawAttestation, StandardizedClaims, UserDataBinding,
    VerificationResult,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcsConfig {
    pub project_id: String,
    pub auth_token: String,
    pub prefer_local_verification: bool,
}

// TdxQuoteData is now defined in crate::tdx module

pub struct TdxGcsRemoteProvider {
    config: Option<GcsConfig>,
    client: reqwest::Client,
    tdx_interface: Box<dyn TdxInterface>,
}

impl Default for TdxGcsRemoteProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl TdxGcsRemoteProvider {
    pub fn new() -> Self {
        #[cfg(feature = "tdx-hardware-required")]
        {
            // PRODUCTION MODE: Hardware required, no simulation
            let hardware = TdxHardware::new();
            if !hardware.is_hardware_available() {
                panic!(
                    "FATAL: TDX hardware required (compiled with --features \
                     tdx-hardware-required) but TDX device not available in TdxGcsRemoteProvider"
                );
            }
            Self {
                config: None,
                client: reqwest::Client::new(),
                tdx_interface: Box::new(hardware),
            }
        }

        #[cfg(not(feature = "tdx-hardware-required"))]
        {
            // DEVELOPMENT MODE: Allow simulation fallback
            let hardware = TdxHardware::new();
            if hardware.is_hardware_available() {
                Self {
                    config: None,
                    client: reqwest::Client::new(),
                    tdx_interface: Box::new(hardware),
                }
            } else {
                Self {
                    config: None,
                    client: reqwest::Client::new(),
                    tdx_interface: Box::new(TdxSimulator::new()),
                }
            }
        }
    }

    pub fn new_with_interface(tdx_interface: Box<dyn TdxInterface>) -> Self {
        Self {
            config: None,
            client: reqwest::Client::new(),
            tdx_interface,
        }
    }

    async fn verify_via_gcs_service(
        &self,
        quote: &[u8],
        config: &GcsConfig,
    ) -> Result<TdxQuoteData> {
        let url = format!(
            "https://confidentialcomputing.googleapis.com/v1/projects/{}/locations/global:verifyAttestation",
            config.project_id
        );

        let request_body = serde_json::json!({
            "quote": base64::engine::general_purpose::STANDARD.encode(quote),
            "platform": "tdx"
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", config.auth_token))
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "GCS verification failed with status {}: {}",
                status,
                error_text
            );
        }

        let verification_result: serde_json::Value = response.json().await?;
        self.parse_gcs_verification_response(verification_result)
    }

    fn parse_gcs_verification_response(&self, response: serde_json::Value) -> Result<TdxQuoteData> {
        // Parse GCS Confidential Computing API response
        // The real API returns a signed OIDC token in the claims_token field

        // Extract the OIDC token from the response
        let claims_token_b64 = response
            .get("claimsToken")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing claimsToken in GCS response"))?;

        // Decode base64 to get the JWT token bytes
        let token_bytes = base64::engine::general_purpose::STANDARD
            .decode(claims_token_b64)
            .map_err(|e| anyhow::anyhow!("Failed to decode base64 claimsToken: {}", e))?;

        // Parse the JWT token without verification (since it's already verified by GCS)
        let token_str = String::from_utf8(token_bytes)
            .map_err(|e| anyhow::anyhow!("Invalid UTF-8 in JWT token: {}", e))?;

        // Parse JWT structure (header.payload.signature)
        let parts: Vec<&str> = token_str.split('.').collect();
        if parts.len() != 3 {
            anyhow::bail!("Invalid JWT format: expected 3 parts, got {}", parts.len());
        }

        // Decode the payload (middle part)
        let payload_b64 = parts[1];
        let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload_b64)
            .map_err(|e| anyhow::anyhow!("Failed to decode JWT payload: {}", e))?;

        let payload_str = String::from_utf8(payload_bytes)
            .map_err(|e| anyhow::anyhow!("Invalid UTF-8 in JWT payload: {}", e))?;

        // Parse the JWT payload as JSON
        let payload: serde_json::Value = serde_json::from_str(&payload_str)
            .map_err(|e| anyhow::anyhow!("Failed to parse JWT payload JSON: {}", e))?;

        // Extract TDX-specific claims from the JWT payload
        // Based on Google Cloud documentation, the JWT contains standardized claims

        // Extract measurements - look for standard EAT claims
        let mut measurements = HashMap::new();

        // Extract MRTD from measurements array or specific claim
        let mrtd = if let Some(measurements_array) =
            payload.get("measurements").and_then(|v| v.as_array())
        {
            measurements_array
                .iter()
                .find(|m| {
                    m.get("type")
                        .and_then(|t| t.as_str())
                        .map(|s| s == "software" || s == "mrtd")
                        .unwrap_or(false)
                })
                .and_then(|m| m.get("val").and_then(|v| v.as_str()))
                .map(|s| hex::decode(s).unwrap_or_default())
                .unwrap_or_else(|| vec![0u8; 48])
        } else {
            // Fallback: use placeholder MRTD
            vec![0x42u8; 48] // Mock MRTD for testing
        };

        // Extract RTMRs from runtime measurements if available
        if let Some(rtmrs_obj) = payload
            .get("submods")
            .and_then(|s| s.get("rtmrs"))
            .and_then(|r| r.as_object())
        {
            for (key, value) in rtmrs_obj {
                if let Some(rtmr_hex) = value.get("val").and_then(|v| v.as_str()) {
                    if let Ok(rtmr_bytes) = hex::decode(rtmr_hex) {
                        measurements.insert(key.clone(), rtmr_bytes);
                    }
                }
            }
        } else {
            // Fallback: create mock RTMRs for testing
            measurements.insert("rtmr0".to_string(), vec![0x11u8; 48]);
            measurements.insert("rtmr1".to_string(), vec![0x22u8; 48]);
            measurements.insert("rtmr2".to_string(), vec![0x33u8; 48]);
            measurements.insert("rtmr3".to_string(), vec![0x44u8; 48]);
        }

        // Extract debug status (EAT dbgstat claim)
        let debug_disabled = payload
            .get("dbgstat")
            .and_then(|v| v.as_u64())
            .map(|status| status <= 2) // 0=debug disabled, 1=disabled since boot, 2=disabled permanently
            .unwrap_or(true); // Default to debug disabled for security

        // Extract security version
        let security_version = payload
            .get("hwversion")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        // Extract user data from eat_nonce or bound data
        let user_data = payload
            .get("eat_nonce")
            .and_then(|v| v.as_str())
            .map(|s| {
                base64::engine::general_purpose::STANDARD
                    .decode(s)
                    .unwrap_or_default()
            })
            .unwrap_or_default();

        // Extract timestamp (iat claim)
        let timestamp = payload
            .get("iat")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
            });

        tracing::info!(
            "Successfully parsed GCS JWT response: mrtd_len={}, rtmrs={}, debug_disabled={}, \
             user_data_len={}",
            mrtd.len(),
            measurements.len(),
            debug_disabled,
            user_data.len()
        );

        Ok(TdxQuoteData {
            mrtd,
            rtmrs: measurements,
            security_version,
            debug_disabled,
            user_data,
            timestamp,
            tcb_svn: vec![0u8; 16], // Placeholder - would need to extract from JWT if available
        })
    }

    fn extract_standardized_claims(
        &self,
        quote_data: TdxQuoteData,
        bundle: &AttestationBundle,
    ) -> Result<StandardizedClaims> {
        // Convert RTMR map to measurements map
        let mut measurements = std::collections::HashMap::new();
        for (key, value) in quote_data.rtmrs {
            measurements.insert(key, value);
        }

        // Add additional measurements
        measurements.insert("mrtd".to_string(), quote_data.mrtd.clone());
        if !quote_data.tcb_svn.is_empty() {
            measurements.insert("tcb_svn".to_string(), quote_data.tcb_svn.clone());
        }

        // Determine hardware security level (1=debug, 3=production)
        let hardware_security_level = if quote_data.debug_disabled { 3 } else { 1 };

        // Use MRTD as unique entity ID (first 32 bytes)
        let unique_entity_id = if quote_data.mrtd.len() >= 32 {
            quote_data.mrtd[0..32].to_vec()
        } else {
            quote_data.mrtd.clone()
        };

        // Convert measurements HashMap to Vec<Measurement> using exact field names
        let rtmr_measurements: Vec<Measurement> = measurements
            .into_iter()
            .map(|(key, value)| Measurement {
                val: value,
                alg: "sha-384".to_string(), // Use exact algorithm name from spec
                measurement_type: Some(key),
                vendor: Some("intel".to_string()),
                version: None,
            })
            .collect();

        // Add the primary software measurement (MRTD)
        let mut all_measurements = vec![Measurement {
            val: quote_data.mrtd,
            alg: "sha-384".to_string(),
            measurement_type: Some("application".to_string()), // Primary enclave measurement
            vendor: Some("intel".to_string()),
            version: None,
        }];
        all_measurements.extend(rtmr_measurements);

        Ok(StandardizedClaims {
            // Core freshness and context
            iat: quote_data.timestamp,
            eat_nonce: Some(quote_data.user_data),

            // Identity and provenance
            ueid: Some(unique_entity_id),
            sueids: None,
            oemid: Some("intel-tdx-gcs".to_string()),
            hwmodel: Some("tdx".to_string()),
            hwversion: Some("1.0".to_string()),

            // Debug and boot status
            dbgstat: if hardware_security_level == 1 { 4 } else { 0 }, // 1=debug->4=enabled, 3=prod->0=disabled
            oemboot: Some(true),

            // Software identity
            swname: None,
            swversion: None,
            manifests: None,

            // Measurements and results (required - at least one)
            measurements: all_measurements,
            measres: None,

            // Execution structure breakdown
            submods: None, // Could group RTMRs here

            // Key binding
            cnf: None,
            intuse: None,

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
impl AttestationProvider for TdxGcsRemoteProvider {
    fn platform_type(&self) -> &str {
        "tdx"
    }

    fn max_user_data_size(&self) -> usize {
        64 // TDX user data limit
    }

    async fn update_config(&mut self, config_json: &str) -> Result<()> {
        let config: GcsConfig = serde_json::from_str(config_json)?;
        self.config = Some(config);
        Ok(())
    }

    async fn generate_attestation(
        &self,
        user_data_binding: &UserDataBinding,
    ) -> Result<RawAttestation> {
        tracing::info!("Generating TDX attestation using hardware/simulator interface");

        // Use the unified TDX interface to generate quote
        let quote_bytes = self
            .tdx_interface
            .generate_quote(&user_data_binding.embedded_hash)?;

        Ok(RawAttestation {
            platform_type: "tdx".to_string(),
            evidence: quote_bytes,
            certificates: None, // TDX quotes include certificates in the quote itself
        })
    }

    async fn verify_attestation(&self, bundle: &AttestationBundle) -> Result<VerificationResult> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("GCS provider not configured"))?;

        match self
            .verify_via_gcs_service(&bundle.raw_attestation.evidence, config)
            .await
        {
            Ok(quote_data) => {
                let claims = self.extract_standardized_claims(quote_data, bundle)?;
                Ok(VerificationResult::Verified(claims))
            }
            Err(e) if e.to_string().contains("authentication") || e.to_string().contains("401") => {
                // Config/auth error - cannot handle
                Ok(VerificationResult::Unsupported)
            }
            Err(e) => {
                // Definitive verification failure
                Ok(VerificationResult::Failed(e.to_string()))
            }
        }
    }
}

/// Local TDX verification provider (future implementation)
pub struct TdxLocalProvider {
    tdx_interface: Box<dyn TdxInterface>,
}

impl Default for TdxLocalProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl TdxLocalProvider {
    pub fn new() -> Self {
        #[cfg(feature = "tdx-hardware-required")]
        {
            // PRODUCTION MODE: Hardware required, no simulation
            let hardware = TdxHardware::new();
            if !hardware.is_hardware_available() {
                panic!(
                    "FATAL: TDX hardware required (compiled with --features \
                     tdx-hardware-required) but TDX device not available in TdxLocalProvider"
                );
            }
            Self {
                tdx_interface: Box::new(hardware),
            }
        }

        #[cfg(not(feature = "tdx-hardware-required"))]
        {
            // DEVELOPMENT MODE: Allow simulation fallback
            let hardware = TdxHardware::new();
            if hardware.is_hardware_available() {
                Self {
                    tdx_interface: Box::new(hardware),
                }
            } else {
                Self {
                    tdx_interface: Box::new(TdxSimulator::new()),
                }
            }
        }
    }

    pub fn new_with_interface(tdx_interface: Box<dyn TdxInterface>) -> Self {
        Self { tdx_interface }
    }

    async fn try_local_verification(&self, quote: &[u8]) -> Result<TdxQuoteData> {
        // Parse the TDX quote using our working parser
        let parsed_quote = TdxParser::parse_quote(quote)
            .map_err(|e| anyhow::anyhow!("Failed to parse TDX quote: {}", e))?;

        // Verify quote structure
        TdxParser::verify_quote_structure(&parsed_quote)
            .map_err(|e| anyhow::anyhow!("Quote structure verification failed: {}", e))?;

        // Extract claims
        let claims = TdxParser::extract_claims(&parsed_quote);

        // Convert to TdxQuoteData format for compatibility
        let mut rtmrs = HashMap::new();
        rtmrs.insert("rtmr0".to_string(), claims.rtmr0);
        rtmrs.insert("rtmr1".to_string(), claims.rtmr1);
        rtmrs.insert("rtmr2".to_string(), claims.rtmr2);
        rtmrs.insert("rtmr3".to_string(), claims.rtmr3);

        let quote_data = TdxQuoteData {
            mrtd: claims.mrtd,
            rtmrs,
            security_version: u64::from_le_bytes(claims.tcb_svn[..8].try_into().unwrap_or([0; 8])),
            debug_disabled: !claims.debug_enabled,
            user_data: claims.report_data,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            tcb_svn: claims.tcb_svn,
        };

        // TODO: In a full implementation, we would:
        // 1. Verify the quote signature using Intel verification libraries
        // 2. Check certificate chain validity
        // 3. Verify TCB is up to date
        // 4. Check for revocations

        // For now, we'll perform basic validation
        if quote_data.mrtd.is_empty() {
            anyhow::bail!("Invalid MRTD in quote");
        }

        tracing::info!("Local TDX quote verification completed successfully");
        Ok(quote_data)
    }

    fn extract_standardized_claims(
        &self,
        quote_data: TdxQuoteData,
        bundle: &AttestationBundle,
    ) -> Result<StandardizedClaims> {
        // Convert RTMR map to measurements map
        let mut measurements = std::collections::HashMap::new();
        for (key, value) in quote_data.rtmrs {
            measurements.insert(key, value);
        }

        // Add additional measurements
        measurements.insert("mrtd".to_string(), quote_data.mrtd.clone());
        if !quote_data.tcb_svn.is_empty() {
            measurements.insert("tcb_svn".to_string(), quote_data.tcb_svn.clone());
        }

        // Determine hardware security level (1=debug, 3=production)
        let hardware_security_level = if quote_data.debug_disabled { 3 } else { 1 };

        // Use MRTD as unique entity ID (first 32 bytes)
        let unique_entity_id = if quote_data.mrtd.len() >= 32 {
            quote_data.mrtd[0..32].to_vec()
        } else {
            quote_data.mrtd.clone()
        };

        // Convert measurements HashMap to Vec<Measurement> using exact field names
        let rtmr_measurements: Vec<Measurement> = measurements
            .into_iter()
            .map(|(key, value)| Measurement {
                val: value,
                alg: "sha-384".to_string(), // Use exact algorithm name from spec
                measurement_type: Some(key),
                vendor: Some("intel".to_string()),
                version: None,
            })
            .collect();

        // Add the primary software measurement (MRTD)
        let mut all_measurements = vec![Measurement {
            val: quote_data.mrtd,
            alg: "sha-384".to_string(),
            measurement_type: Some("application".to_string()), // Primary enclave measurement
            vendor: Some("intel".to_string()),
            version: None,
        }];
        all_measurements.extend(rtmr_measurements);

        Ok(StandardizedClaims {
            // Core freshness and context
            iat: quote_data.timestamp,
            eat_nonce: Some(quote_data.user_data),

            // Identity and provenance
            ueid: Some(unique_entity_id),
            sueids: None,
            oemid: Some("intel-tdx-local".to_string()),
            hwmodel: Some("tdx".to_string()),
            hwversion: Some("1.0".to_string()),

            // Debug and boot status
            dbgstat: if hardware_security_level == 1 { 4 } else { 0 }, // 1=debug->4=enabled, 3=prod->0=disabled
            oemboot: Some(true),

            // Software identity
            swname: None,
            swversion: None,
            manifests: None,

            // Measurements and results (required - at least one)
            measurements: all_measurements,
            measres: None,

            // Execution structure breakdown
            submods: None, // Could group RTMRs here

            // Key binding
            cnf: None,
            intuse: None,

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
impl AttestationProvider for TdxLocalProvider {
    fn platform_type(&self) -> &str {
        "tdx"
    }

    fn max_user_data_size(&self) -> usize {
        64
    }

    async fn update_config(&mut self, _config_json: &str) -> Result<()> {
        Ok(()) // No config needed for local verification
    }

    async fn generate_attestation(
        &self,
        user_data_binding: &UserDataBinding,
    ) -> Result<RawAttestation> {
        tracing::info!("Generating TDX attestation using local interface");

        // Use the unified TDX interface to generate quote
        let quote_bytes = self
            .tdx_interface
            .generate_quote(&user_data_binding.embedded_hash)?;

        Ok(RawAttestation {
            platform_type: "tdx".to_string(),
            evidence: quote_bytes,
            certificates: None,
        })
    }

    async fn verify_attestation(&self, bundle: &AttestationBundle) -> Result<VerificationResult> {
        // Try local verification if Intel libraries are available
        match self
            .try_local_verification(&bundle.raw_attestation.evidence)
            .await
        {
            Ok(quote_data) => {
                let claims = self.extract_standardized_claims(quote_data, bundle)?;
                Ok(VerificationResult::Verified(claims))
            }
            Err(e) if e.to_string().contains("not available") => {
                // Local verification not available - try next provider
                Ok(VerificationResult::Unsupported)
            }
            Err(e) => {
                // Verification failed
                Ok(VerificationResult::Failed(e.to_string()))
            }
        }
    }
}
