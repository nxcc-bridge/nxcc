use anyhow::Result;
use async_trait::async_trait;

use crate::{
    tdx::{
        hardware::{TdxHardware, TdxInterface},
        TdxAttestationClaims,
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
        Self {
            tdx_interface: Box::new(TdxHardware::new()),
        }
    }

    pub fn new_with_interface(tdx_interface: Box<dyn TdxInterface>) -> Self {
        Self { tdx_interface }
    }

    async fn verify_with_dcap_qvl(&self, quote: &[u8]) -> Result<TdxAttestationClaims> {
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

    fn is_available(&self) -> bool {
        self.tdx_interface.is_hardware_available()
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

                if quote_data.report_data.len() < 32
                    || quote_data.report_data[..32] != received_userdata_hash
                {
                    return Ok(VerificationResult::Failed(
                        "Userdata hash mismatch".to_string(),
                    ));
                }
                let claims = self.extract_standardized_claims(quote_data, bundle)?;
                Ok(VerificationResult::Verified(Box::new(claims)))
            }
            Err(e) => {
                let e_str = e.to_string();
                if e_str.contains("not available")
                    || e_str.contains("unsupported")
                    || e_str.contains("collateral")
                {
                    tracing::warn!("dcap-qvl verification unsupported: {}", e);
                    Ok(VerificationResult::Unsupported)
                } else {
                    // Definitive verification failure
                    Ok(VerificationResult::Failed(e.to_string()))
                }
            }
        }
    }
}
