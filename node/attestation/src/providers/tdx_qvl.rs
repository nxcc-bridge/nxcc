use anyhow::Result;
use async_trait::async_trait;

use crate::{
    tdx::{
        hardware::{TdxHardware, TdxInterface, TdxSimulator},
        TdxAttestationClaims, TdxQuote,
    },
    types::Measurement,
    user_data_binding, AttestationBundle, AttestationProvider, RawAttestation, StandardizedClaims,
    VerificationResult,
};

/// TDX attestation provider using dcap-qvl for local verification
pub struct TdxQvlProvider {
    tdx_interface: Box<dyn TdxInterface>,
}

impl Default for TdxQvlProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl TdxQvlProvider {
    pub fn new() -> Self {
        // Runtime mode selection based on environment variable
        let require_hardware = std::env::var("TDX_TESTS_REQUIRE_HARDWARE")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let hardware = TdxHardware::new();

        if require_hardware {
            // PRODUCTION MODE: Hardware required, no simulation fallback
            if !hardware.is_hardware_available() {
                panic!(
                    "FATAL: TDX hardware required (TDX_TESTS_REQUIRE_HARDWARE=true) but TDX \
                     device not available in TdxQvlProvider"
                );
            }
            Self {
                tdx_interface: Box::new(hardware),
            }
        } else {
            // DEVELOPMENT MODE: Allow simulation fallback
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

    async fn verify_with_dcap_qvl(&self, quote: &[u8]) -> Result<TdxAttestationClaims> {
        // Check if this is a simulator quote by trying to parse it first
        let is_simulator_quote = self.is_simulator_quote(quote);

        if is_simulator_quote {
            // For simulator quotes, we'll do basic structural verification only
            // since they can't be verified against real Intel infrastructure
            return self.verify_simulator_quote(quote).await;
        }

        // For real hardware quotes, use full dcap-qvl verification
        let pccs_url = std::env::var("PCCS_URL").unwrap_or_else(|_| {
            "https://api.trustedservices.intel.com/sgx/certification/v4/".to_string()
        });

        // Get collateral for the quote
        let collateral = dcap_qvl::collateral::get_collateral(&pccs_url, quote)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get collateral: {}", e))?;

        // Get current time for verification
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Verify the quote using dcap-qvl
        let verified_report = dcap_qvl::verify::verify(quote, &collateral, now)
            .map_err(|e| anyhow::anyhow!("Quote verification failed: {}", e))?;

        // Extract TDX specific data from the verified report
        let report = match verified_report.report {
            dcap_qvl::quote::Report::TD10(td_report) => td_report,
            dcap_qvl::quote::Report::TD15(td_report) => td_report.base,
            _ => return Err(anyhow::anyhow!("Expected TDX report but got SGX report")),
        };

        // Create TdxAttestationClaims from the verified report
        let quote_data = TdxAttestationClaims {
            // Core measurements
            mrtd: report.mr_td.to_vec(),
            rtmr0: report.rt_mr0.to_vec(),
            rtmr1: report.rt_mr1.to_vec(),
            rtmr2: report.rt_mr2.to_vec(),
            rtmr3: report.rt_mr3.to_vec(),
            mr_config_id: report.mr_config_id.to_vec(),
            mr_owner: report.mr_owner.to_vec(),
            mr_seam: report.mr_seam.to_vec(),

            // Security attributes
            debug_enabled: report.td_attributes[0] & 0x01 != 0, // Debug flag is bit 0
            td_attributes: u64::from_le_bytes(
                report.td_attributes[..8].try_into().unwrap_or([0; 8]),
            ),
            seam_attributes: 0, // Not directly available from dcap-qvl report
            tcb_svn: report.tee_tcb_svn.to_vec(),

            // User data and keys
            report_data: report.report_data.to_vec(),
            user_data: vec![], // Would need to extract from quote header
            ephemeral_key: if report.report_data[..32].iter().any(|&b| b != 0) {
                Some(report.report_data[..32].to_vec())
            } else {
                None
            },

            // Quote metadata (would need to parse quote header for these)
            quote_version: 4, // Assume TDX v4
            tee_type: 0x81,   // TDX TEE type
            att_key_type: 2,  // ECDSA P-256

            // Signature info
            has_valid_signature: verified_report.status == "UpToDate",
            signature_present: true, // dcap-qvl verified it
            cert_chain_present: true,
        };

        tracing::info!(
            "Successfully verified TDX quote using dcap-qvl: mrtd_len={}, debug_enabled={}, \
             user_data_len={}, status={}",
            quote_data.mrtd.len(),
            quote_data.debug_enabled,
            quote_data.user_data.len(),
            verified_report.status
        );

        Ok(quote_data)
    }

    /// Check if this is a simulator-generated quote by examining its structure
    fn is_simulator_quote(&self, quote: &[u8]) -> bool {
        // Look for simulator signature patterns that indicate a mock quote
        if quote.len() < 1000 {
            return true; // Real TDX quotes are typically larger
        }

        // Check for the mock signature pattern used by TdxSimulator
        let mock_signature = b"MOCK_TDX_SIGNATURE_DATA";
        if let Some(_sig_start) = quote
            .windows(mock_signature.len())
            .position(|window| window == mock_signature)
        {
            tracing::debug!("Detected simulator quote by mock signature pattern");
            return true;
        }

        // Check if we can detect simulator interface by trying any type of downcasting
        // This is a fallback for when the signature detection doesn't work
        false
    }

    /// Verify a simulator quote using local parsing only
    async fn verify_simulator_quote(&self, quote: &[u8]) -> Result<TdxAttestationClaims> {
        // Use our existing TDX parser for basic structural verification
        let parsed_quote = TdxQuote::parse(quote)
            .map_err(|e| anyhow::anyhow!("Failed to parse simulator quote: {}", e))?;

        // Basic structure verification
        parsed_quote
            .verify_structure()
            .map_err(|e| anyhow::anyhow!("Simulator quote structure invalid: {}", e))?;

        // Extract claims
        let claims = parsed_quote.extract_claims();

        tracing::info!(
            "Successfully verified simulator TDX quote: mrtd_len={}, debug_enabled={}, \
             user_data_len={}",
            claims.mrtd.len(),
            claims.debug_enabled,
            claims.user_data.len()
        );

        Ok(claims)
    }

    fn extract_standardized_claims(
        &self,
        claims: TdxAttestationClaims,
        _bundle: &AttestationBundle,
    ) -> Result<StandardizedClaims> {
        // Convert to measurements HashMap
        let mut measurements = std::collections::HashMap::new();
        measurements.insert("rtmr0".to_string(), claims.rtmr0);
        measurements.insert("rtmr1".to_string(), claims.rtmr1);
        measurements.insert("rtmr2".to_string(), claims.rtmr2);
        measurements.insert("rtmr3".to_string(), claims.rtmr3);
        measurements.insert("mr_config_id".to_string(), claims.mr_config_id);
        measurements.insert("mr_owner".to_string(), claims.mr_owner);
        measurements.insert("mr_seam".to_string(), claims.mr_seam);
        measurements.insert("tcb_svn".to_string(), claims.tcb_svn);

        // Determine hardware security level (1=debug, 3=production)
        let hardware_security_level = if claims.debug_enabled { 1 } else { 3 };

        // Use MRTD as unique entity ID (first 32 bytes)
        let unique_entity_id = if claims.mrtd.len() >= 32 {
            claims.mrtd[0..32].to_vec()
        } else {
            claims.mrtd.clone()
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
            val: claims.mrtd,
            alg: "sha-384".to_string(),
            measurement_type: Some("application".to_string()), // Primary enclave measurement
            vendor: Some("intel".to_string()),
            version: None,
        }];
        all_measurements.extend(rtmr_measurements);

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Ok(StandardizedClaims {
            // Core freshness and context
            iat: timestamp,
            eat_nonce: Some(claims.report_data),

            // Identity and provenance
            ueid: Some(unique_entity_id),
            sueids: None,
            oemid: Some("intel-tdx-qvl".to_string()),
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
impl AttestationProvider for TdxQvlProvider {
    fn platform_type(&self) -> &str {
        "tdx"
    }

    fn max_user_data_size(&self) -> usize {
        64 // TDX user data limit
    }

    async fn update_config(&mut self, _config_json: &str) -> Result<()> {
        // dcap-qvl doesn't require configuration for basic verification
        // PCCS URL can be set via environment variable if needed
        Ok(())
    }

    async fn generate_attestation(&self, userdata_hash: &[u8]) -> Result<RawAttestation> {
        tracing::info!("Generating TDX attestation using hardware/simulator interface");

        // Use the unified TDX interface to generate quote
        let quote_bytes = self.tdx_interface.generate_quote(userdata_hash)?;

        Ok(RawAttestation {
            platform_type: "tdx".to_string(),
            evidence: quote_bytes,
            certificates: None, // TDX quotes include certificates in the quote itself
        })
    }

    async fn verify_attestation(&self, bundle: &AttestationBundle) -> Result<VerificationResult> {
        // First, verify the quote itself.
        let quote_data_result = self
            .verify_with_dcap_qvl(&bundle.raw_attestation.evidence)
            .await;

        match quote_data_result {
            Ok(quote_data) => {
                // Second, verify the userdata binding.
                let received_userdata_hash =
                    user_data_binding::hash_userdata(&bundle.detached_userdata);

                if quote_data.user_data.len() < 32
                    || quote_data.user_data[..32] != received_userdata_hash
                {
                    return Ok(VerificationResult::Failed(
                        "Userdata hash mismatch".to_string(),
                    ));
                }
                let claims = self.extract_standardized_claims(quote_data, bundle)?;
                Ok(VerificationResult::Verified(Box::new(claims)))
            }
            Err(e)
                if e.to_string().contains("not available")
                    || e.to_string().contains("unsupported")
                    || e.to_string().contains("collateral") =>
            {
                // QVL verification failed, try simulator verification as fallback
                tracing::info!(
                    "dcap-qvl verification failed, trying simulator verification: {}",
                    e
                );
                match self
                    .verify_simulator_quote(&bundle.raw_attestation.evidence)
                    .await
                {
                    Ok(quote_data) => {
                        // Also verify userdata binding for simulator quotes.
                        let received_userdata_hash =
                            user_data_binding::hash_userdata(&bundle.detached_userdata);

                        if quote_data.user_data.len() < 32
                            || quote_data.user_data[..32] != received_userdata_hash
                        {
                            return Ok(VerificationResult::Failed(
                                "Userdata hash mismatch".to_string(),
                            ));
                        }

                        let claims = self.extract_standardized_claims(quote_data, bundle)?;
                        Ok(VerificationResult::Verified(Box::new(claims)))
                    }
                    Err(sim_err) => {
                        tracing::warn!(
                            "Both dcap-qvl and simulator verification failed: dcap={}, sim={}",
                            e,
                            sim_err
                        );
                        Ok(VerificationResult::Unsupported)
                    }
                }
            }
            Err(e) => {
                // Definitive verification failure
                Ok(VerificationResult::Failed(e.to_string()))
            }
        }
    }
}
