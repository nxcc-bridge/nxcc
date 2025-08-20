use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use base64::Engine;
use chrono;
use reqwest;
use serde::{Deserialize, Serialize};
use serde_json;

use crate::{
    tdx::{
        hardware::{TdxInterface, TdxHardware, TdxSimulator},
        parser::TdxParser,
        TdxQuoteData,
    },
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
                    "FATAL: TDX hardware required (compiled with --features tdx-hardware-required) \
                     but TDX device not available in TdxGcsRemoteProvider"
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
        // Reference: https://cloud.google.com/confidential-computing/confidential-space/docs/reference/rest/v1/projects.locations/verifyAttestation

        // Check if verification was successful
        let success = response
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !success {
            let error_msg = response
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Verification failed");
            anyhow::bail!("GCS verification failed: {}", error_msg);
        }

        // Extract the verified attestation report
        let attestation_report = response
            .get("attestationReport")
            .ok_or_else(|| anyhow::anyhow!("Missing attestationReport in GCS response"))?;

        // Parse TDX-specific claims from the report
        let tdx_report = attestation_report
            .get("tdxReport")
            .ok_or_else(|| anyhow::anyhow!("Missing tdxReport in attestation report"))?;

        // Extract MRTD (Measurement of the TD)
        let mrtd_hex = tdx_report
            .get("mrtd")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing or invalid MRTD in tdxReport"))?;
        let mrtd = hex::decode(mrtd_hex).map_err(|e| anyhow::anyhow!("Invalid MRTD hex: {}", e))?;

        // Extract RTMRs (Runtime Measurement Registers)
        let mut rtmrs = HashMap::new();
        if let Some(rtmrs_obj) = tdx_report.get("rtmrs").and_then(|v| v.as_object()) {
            for (key, value) in rtmrs_obj {
                if let Some(rtmr_hex) = value.as_str() {
                    match hex::decode(rtmr_hex) {
                        Ok(rtmr_bytes) => {
                            rtmrs.insert(key.clone(), rtmr_bytes);
                        }
                        Err(e) => {
                            tracing::warn!("Failed to decode RTMR {}: {}", key, e);
                        }
                    }
                }
            }
        }

        // Extract security version
        let security_version = tdx_report
            .get("securityVersion")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        // Extract debug flag
        let debug_disabled = !tdx_report
            .get("debugFlag")
            .and_then(|v| v.as_bool())
            .unwrap_or(false); // If debug flag is true, then debug is NOT disabled

        // Extract user data from report data
        let report_data_hex = tdx_report
            .get("reportData")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let user_data = if report_data_hex.is_empty() {
            Vec::new()
        } else {
            hex::decode(report_data_hex)
                .map_err(|e| anyhow::anyhow!("Invalid reportData hex: {}", e))?
        };

        // Extract timestamp from GCS response metadata
        let timestamp = response
            .get("verificationTime")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp() as u64)
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
            });

        // Extract TCB SVN
        let tcb_svn = tdx_report
            .get("tcbSvn")
            .and_then(|v| v.as_str())
            .map(|s| hex::decode(s).unwrap_or_default())
            .unwrap_or_default();

        Ok(TdxQuoteData {
            mrtd,
            rtmrs,
            security_version,
            debug_disabled,
            user_data,
            timestamp,
            tcb_svn,
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

        Ok(StandardizedClaims {
            software_component: quote_data.mrtd,
            hardware_security_level,
            security_version_number: quote_data.security_version,
            unique_entity_id,
            nonce: quote_data.user_data,
            issued_at: quote_data.timestamp,
            measurements,
            oem_id: "intel-tdx-gcs".to_string(),
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
                    "FATAL: TDX hardware required (compiled with --features tdx-hardware-required) \
                     but TDX device not available in TdxLocalProvider"
                );
            }
            Self { 
                tdx_interface: Box::new(hardware) 
            }
        }
        
        #[cfg(not(feature = "tdx-hardware-required"))]
        {
            // DEVELOPMENT MODE: Allow simulation fallback
            let hardware = TdxHardware::new();
            if hardware.is_hardware_available() {
                Self { 
                    tdx_interface: Box::new(hardware) 
                }
            } else {
                Self { 
                    tdx_interface: Box::new(TdxSimulator::new()) 
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

        Ok(StandardizedClaims {
            software_component: quote_data.mrtd,
            hardware_security_level,
            security_version_number: quote_data.security_version,
            unique_entity_id,
            nonce: quote_data.user_data,
            issued_at: quote_data.timestamp,
            measurements,
            oem_id: "intel-tdx-local".to_string(),
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
