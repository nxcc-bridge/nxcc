use std::{collections::HashMap, convert::TryInto};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// TDX-specific constants
pub const TEE_TYPE_TDX: u32 = 0x81;
pub const QUOTE_VERSION_4: u16 = 4;

/// TDX Quote Header (48 bytes)
#[derive(Debug, Clone)]
pub struct TdxQuoteHeader {
    pub version: u16,
    pub att_key_type: u16,
    pub tee_type: u32,
    pub reserved: u32,
    pub vendor_id: [u8; 16],
    pub user_data: [u8; 20],
}

/// SGX Report 2 / TD Report (584 bytes)
#[derive(Debug, Clone)]
pub struct TdReport {
    pub tcb_svn: [u8; 16],         // TCB Security Version Number
    pub mr_seam: [u8; 48],         // Measurement of SEAM module
    pub mr_signer_seam: [u8; 48],  // Measurement of SEAM signer
    pub seam_attributes: u64,      // SEAM attributes
    pub td_attributes: u64,        // TD attributes (includes debug flag)
    pub xfam: u64,                 // Extended features available mask
    pub mrtd: [u8; 48],            // Measurement of initial TD contents
    pub mr_config_id: [u8; 48],    // Configuration ID
    pub mr_owner: [u8; 48],        // Owner measurement
    pub mr_owner_config: [u8; 48], // Owner configuration
    pub rtmr: [[u8; 48]; 4],       // Runtime measurement registers 0-3
    pub report_data: [u8; 64],     // User-provided report data
}

/// Quote signature data structure
#[derive(Debug, Clone)]
pub struct QuoteSignature {
    pub ecdsa_signature: [u8; 64],  // ECDSA P-256 signature
    pub ecdsa_public_key: [u8; 64], // ECDSA P-256 public key
    pub cert_data_type: u16,        // Certification data type
    pub cert_data_size: u32,        // Certification data size
    pub qe_report: [u8; 384],       // QE Report
    pub qe_signature: [u8; 64],     // QE Report signature
    pub qe_auth_data_size: u16,     // QE Auth data size
    pub qe_auth_data: Vec<u8>,      // QE Auth data
    pub cert_type: u16,             // Certificate type
    pub cert_size: u32,             // Certificate data size
    pub cert_data: Vec<u8>,         // Certificate data (PEM chain)
}

/// Complete TDX Quote structure
#[derive(Debug, Clone)]
pub struct TdxQuote {
    pub header: TdxQuoteHeader,
    pub td_report: TdReport,
    pub signature_len: u32,
    pub signature: Option<QuoteSignature>,
    pub raw_signature_data: Vec<u8>,
}

/// Standardized attestation claims extracted from TDX quote
#[derive(Debug, Clone)]
pub struct TdxAttestationClaims {
    // Core measurements
    pub mrtd: Vec<u8>,         // Measurement of TD initial contents
    pub rtmr0: Vec<u8>,        // Runtime measurement register 0
    pub rtmr1: Vec<u8>,        // Runtime measurement register 1
    pub rtmr2: Vec<u8>,        // Runtime measurement register 2
    pub rtmr3: Vec<u8>,        // Runtime measurement register 3
    pub mr_config_id: Vec<u8>, // Configuration measurement
    pub mr_owner: Vec<u8>,     // Owner measurement
    pub mr_seam: Vec<u8>,      // SEAM measurement

    // Security attributes
    pub debug_enabled: bool,  // Debug mode enabled
    pub td_attributes: u64,   // Raw TD attributes
    pub seam_attributes: u64, // SEAM attributes
    pub tcb_svn: Vec<u8>,     // TCB security version

    // User data and keys
    pub report_data: Vec<u8>,           // User report data
    pub user_data: Vec<u8>,             // Header user data
    pub ephemeral_key: Option<Vec<u8>>, // Extracted ephemeral key

    // Quote metadata
    pub quote_version: u16, // Quote version
    pub tee_type: u32,      // TEE type
    pub att_key_type: u16,  // Attestation key type

    // Signature info (if available)
    pub has_valid_signature: bool, // Whether signature could be parsed
    pub signature_present: bool,   // Whether signature data exists
    pub cert_chain_present: bool,  // Whether cert chain is available
}

/// Parsed TDX quote with extracted measurements and claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedTdxQuote {
    /// Version of the quote
    pub version: u16,
    /// TEE type
    pub tee_type: u32,
    /// Software measurement (MRTD)
    pub mrtd: Vec<u8>,
    /// Configuration ID
    pub mrconfigid: Vec<u8>,
    /// Owner ID
    pub mrowner: Vec<u8>,
    /// Owner config ID
    pub mrownerconfig: Vec<u8>,
    /// Runtime measurements
    pub rtmrs: HashMap<String, Vec<u8>>,
    /// TD attributes
    pub td_attributes: Vec<u8>,
    /// XFAM
    pub xfam: Vec<u8>,
    /// CPU SVN
    pub cpu_svn: Vec<u8>,
    /// Report data (user provided)
    pub report_data: Vec<u8>,
    /// Quote signature data
    pub signature_data: Vec<u8>,
    /// Debug flag from attributes
    pub debug_enabled: bool,
    /// Security version (derived from TCB info)
    pub security_version: u64,
}

/// Working TDX Quote Parser
pub struct TdxParser;

impl TdxParser {
    /// Parse a complete TDX quote
    pub fn parse_quote(quote_bytes: &[u8]) -> Result<TdxQuote> {
        let mut offset = 0;

        // Parse header (48 bytes)
        let header = Self::parse_header(quote_bytes, &mut offset)?;

        // Validate quote
        Self::validate_quote_header(&header)?;

        // Parse TD Report (584 bytes)
        let td_report = Self::parse_td_report(quote_bytes, &mut offset)?;

        // Parse signature length (4 bytes)
        let signature_len = Self::parse_u32_le(quote_bytes, &mut offset)? as usize;

        // Bound the signature buffer
        if quote_bytes.len() < offset + signature_len {
            return Err(anyhow!(
                "Truncated signature: need {}, have {}",
                signature_len,
                quote_bytes.len() - offset
            ));
        }
        let raw_signature_data = quote_bytes[offset..offset + signature_len].to_vec();
        offset += signature_len;

        // Optional: warn if there are trailing bytes (collateral etc.)
        if quote_bytes.len() > offset {
            tracing::warn!(
                "{} trailing bytes after the quote",
                quote_bytes.len() - offset
            );
        }

        // Try to parse signature structure if we have enough data
        let signature = if raw_signature_data.len() >= 134 {
            Self::parse_signature_structure(&raw_signature_data).ok()
        } else {
            None
        };

        Ok(TdxQuote {
            header,
            td_report,
            signature_len: signature_len as u32,
            signature,
            raw_signature_data,
        })
    }

    /// Parse quote header (48 bytes)
    fn parse_header(data: &[u8], offset: &mut usize) -> Result<TdxQuoteHeader> {
        if data.len() < *offset + 48 {
            return Err(anyhow!("Insufficient data for quote header"));
        }

        let version = Self::parse_u16_le(data, offset)?;
        let att_key_type = Self::parse_u16_le(data, offset)?;
        let tee_type = Self::parse_u32_le(data, offset)?;
        let reserved = Self::parse_u32_le(data, offset)?;
        let vendor_id = Self::parse_bytes(data, offset, 16)?;
        let user_data = Self::parse_bytes(data, offset, 20)?;

        Ok(TdxQuoteHeader {
            version,
            att_key_type,
            tee_type,
            reserved,
            vendor_id: vendor_id.try_into().unwrap(),
            user_data: user_data.try_into().unwrap(),
        })
    }

    /// Parse TD Report (584 bytes)
    fn parse_td_report(data: &[u8], offset: &mut usize) -> Result<TdReport> {
        if data.len() < *offset + 584 {
            return Err(anyhow!("Insufficient data for TD report"));
        }

        let tcb_svn = Self::parse_bytes(data, offset, 16)?.try_into().unwrap();
        let mr_seam = Self::parse_bytes(data, offset, 48)?.try_into().unwrap();
        let mr_signer_seam = Self::parse_bytes(data, offset, 48)?.try_into().unwrap();
        let seam_attributes = Self::parse_u64_le(data, offset)?;
        let td_attributes = Self::parse_u64_le(data, offset)?;
        let xfam = Self::parse_u64_le(data, offset)?;
        let mrtd = Self::parse_bytes(data, offset, 48)?.try_into().unwrap();
        let mr_config_id = Self::parse_bytes(data, offset, 48)?.try_into().unwrap();
        let mr_owner = Self::parse_bytes(data, offset, 48)?.try_into().unwrap();
        let mr_owner_config = Self::parse_bytes(data, offset, 48)?.try_into().unwrap();

        // Parse 4 RTMRs
        let mut rtmr = [[0u8; 48]; 4];
        for item in &mut rtmr {
            *item = Self::parse_bytes(data, offset, 48)?.try_into().unwrap();
        }

        let report_data = Self::parse_bytes(data, offset, 64)?.try_into().unwrap();

        Ok(TdReport {
            tcb_svn,
            mr_seam,
            mr_signer_seam,
            seam_attributes,
            td_attributes,
            xfam,
            mrtd,
            mr_config_id,
            mr_owner,
            mr_owner_config,
            rtmr,
            report_data,
        })
    }

    /// Parse signature structure from raw signature data
    fn parse_signature_structure(sig: &[u8]) -> Result<QuoteSignature> {
        let mut o = 0;

        // Helper function to check bounds
        let check_bounds = |offset: usize, len: usize| -> Result<()> {
            if offset + len > sig.len() {
                Err(anyhow!("Signature section truncated"))
            } else {
                Ok(())
            }
        };

        check_bounds(o, 64)?;
        let ecdsa_signature = sig[o..o + 64].try_into().unwrap();
        o += 64;

        check_bounds(o, 64)?;
        let ecdsa_public_key = sig[o..o + 64].try_into().unwrap();
        o += 64;

        check_bounds(o, 2)?;
        let cert_data_type = u16::from_le_bytes(sig[o..o + 2].try_into().unwrap());
        o += 2;

        check_bounds(o, 4)?;
        let cert_data_size = u32::from_le_bytes(sig[o..o + 4].try_into().unwrap());
        o += 4;

        check_bounds(o, 384)?;
        let qe_report = sig[o..o + 384].try_into().unwrap();
        o += 384;

        check_bounds(o, 64)?;
        let qe_signature = sig[o..o + 64].try_into().unwrap();
        o += 64;

        check_bounds(o, 2)?;
        let qe_auth_data_size = u16::from_le_bytes(sig[o..o + 2].try_into().unwrap());
        o += 2;

        check_bounds(o, qe_auth_data_size as usize)?;
        let qe_auth_data = sig[o..o + qe_auth_data_size as usize].to_vec();
        o += qe_auth_data_size as usize;

        // Certificate blob (may be zero)
        let (cert_type, cert_size, cert_data) = if o + 6 <= sig.len() {
            let cert_type = u16::from_le_bytes(sig[o..o + 2].try_into().unwrap());
            o += 2;
            let cert_size = u32::from_le_bytes(sig[o..o + 4].try_into().unwrap());
            o += 4;
            check_bounds(o, cert_size as usize)?;
            let cert_data = sig[o..o + cert_size as usize].to_vec();
            (cert_type, cert_size, cert_data)
        } else {
            (0u16, 0u32, Vec::new())
        };

        Ok(QuoteSignature {
            ecdsa_signature,
            ecdsa_public_key,
            cert_data_type,
            cert_data_size,
            qe_report,
            qe_signature,
            qe_auth_data_size,
            qe_auth_data,
            cert_type,
            cert_size,
            cert_data,
        })
    }

    /// Validate quote header constraints
    fn validate_quote_header(header: &TdxQuoteHeader) -> Result<()> {
        if header.version != QUOTE_VERSION_4 {
            return Err(anyhow!("Unsupported quote version: {}", header.version));
        }

        if header.tee_type != TEE_TYPE_TDX {
            return Err(anyhow!(
                "Invalid TEE type: expected TDX (0x{:x}), got 0x{:x}",
                TEE_TYPE_TDX,
                header.tee_type
            ));
        }

        Ok(())
    }

    /// Extract IEATS/RATS standardized claims from TDX quote
    pub fn extract_standardized_claims(quote: &TdxQuote) -> crate::types::StandardizedClaims {
        use std::{
            collections::HashMap,
            time::{SystemTime, UNIX_EPOCH},
        };

        let td_report = &quote.td_report;

        // Map TDX measurements to standardized EAT claims
        let mut measurements = HashMap::new();
        measurements.insert("rtmr0".to_string(), td_report.rtmr[0].to_vec());
        measurements.insert("rtmr1".to_string(), td_report.rtmr[1].to_vec());
        measurements.insert("rtmr2".to_string(), td_report.rtmr[2].to_vec());
        measurements.insert("rtmr3".to_string(), td_report.rtmr[3].to_vec());
        measurements.insert("mr_config_id".to_string(), td_report.mr_config_id.to_vec());
        measurements.insert("mr_owner".to_string(), td_report.mr_owner.to_vec());
        measurements.insert("mr_seam".to_string(), td_report.mr_seam.to_vec());

        // Extract security version from TCB SVN
        let _security_version =
            u64::from_le_bytes(td_report.tcb_svn[0..8].try_into().unwrap_or([0; 8]));

        // Determine hardware security level from debug bit
        let hardware_security_level = if (td_report.td_attributes & 0x1) != 0 {
            1
        } else {
            3
        };

        // Extract unique entity ID from MRTD (first 32 bytes)
        let unique_entity_id = td_report.mrtd[0..32].to_vec();

        // Extract nonce from report data (user-provided)
        let nonce = td_report.report_data.to_vec();

        // Current timestamp for issued_at
        let issued_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Convert measurements HashMap to Vec<Measurement> using exact field names
        let rtmr_measurements: Vec<crate::types::Measurement> = measurements
            .into_iter()
            .map(|(key, value)| crate::types::Measurement {
                val: value,
                alg: "sha-384".to_string(), // Use exact algorithm name from spec
                measurement_type: Some(key),
                vendor: Some("intel".to_string()),
                version: None,
            })
            .collect();

        // Add the primary software measurement (MRTD)
        let mut all_measurements = vec![crate::types::Measurement {
            val: td_report.mrtd.to_vec(),
            alg: "sha-384".to_string(),
            measurement_type: Some("application".to_string()), // Primary enclave measurement
            vendor: Some("intel".to_string()),
            version: None,
        }];
        all_measurements.extend(rtmr_measurements);

        crate::types::StandardizedClaims {
            // Core freshness and context
            iat: issued_at,
            eat_nonce: Some(nonce),

            // Identity and provenance
            ueid: Some(unique_entity_id),
            sueids: None,
            oemid: Some("intel-tdx".to_string()),
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
        }
    }

    /// Extract standardized attestation claims from parsed quote
    pub fn extract_claims(quote: &TdxQuote) -> TdxAttestationClaims {
        let td_report = &quote.td_report;

        // Extract debug flag from TD attributes (bit 0)
        let debug_enabled = (td_report.td_attributes & 0x1) != 0;

        // Try to extract ephemeral key from report data (first 32 bytes)
        let ephemeral_key = if td_report.report_data.iter().take(32).any(|&b| b != 0) {
            Some(td_report.report_data[..32].to_vec())
        } else {
            None
        };

        // Check signature availability
        let signature_present = !quote.raw_signature_data.is_empty();
        let has_valid_signature = quote.signature.is_some();
        let cert_chain_present = quote
            .signature
            .as_ref()
            .map(|s| !s.cert_data.is_empty())
            .unwrap_or(false);

        TdxAttestationClaims {
            // Core measurements
            mrtd: td_report.mrtd.to_vec(),
            rtmr0: td_report.rtmr[0].to_vec(),
            rtmr1: td_report.rtmr[1].to_vec(),
            rtmr2: td_report.rtmr[2].to_vec(),
            rtmr3: td_report.rtmr[3].to_vec(),
            mr_config_id: td_report.mr_config_id.to_vec(),
            mr_owner: td_report.mr_owner.to_vec(),
            mr_seam: td_report.mr_seam.to_vec(),

            // Security attributes
            debug_enabled,
            td_attributes: td_report.td_attributes,
            seam_attributes: td_report.seam_attributes,
            tcb_svn: td_report.tcb_svn.to_vec(),

            // User data and keys
            report_data: td_report.report_data.to_vec(),
            user_data: quote.header.user_data.to_vec(),
            ephemeral_key,

            // Quote metadata
            quote_version: quote.header.version,
            tee_type: quote.header.tee_type,
            att_key_type: quote.header.att_key_type,

            // Signature info
            has_valid_signature,
            signature_present,
            cert_chain_present,
        }
    }

    /// Extract claims into the serializable ParsedTdxQuote format
    pub fn extract_parsed_quote(quote: &TdxQuote) -> ParsedTdxQuote {
        let td_report = &quote.td_report;

        let mut rtmrs = HashMap::new();
        rtmrs.insert("rtmr0".to_string(), td_report.rtmr[0].to_vec());
        rtmrs.insert("rtmr1".to_string(), td_report.rtmr[1].to_vec());
        rtmrs.insert("rtmr2".to_string(), td_report.rtmr[2].to_vec());
        rtmrs.insert("rtmr3".to_string(), td_report.rtmr[3].to_vec());

        // Extract debug flag from attributes (bit 0)
        let debug_enabled = (td_report.td_attributes & 0x01) != 0;

        // Extract security version from TCB SVN
        let security_version =
            u64::from_le_bytes(td_report.tcb_svn[0..8].try_into().unwrap_or([0u8; 8]));

        ParsedTdxQuote {
            version: quote.header.version,
            tee_type: quote.header.tee_type,
            mrtd: td_report.mrtd.to_vec(),
            mrconfigid: td_report.mr_config_id.to_vec(),
            mrowner: td_report.mr_owner.to_vec(),
            mrownerconfig: td_report.mr_owner_config.to_vec(),
            rtmrs,
            td_attributes: td_report.td_attributes.to_le_bytes().to_vec(),
            xfam: td_report.xfam.to_le_bytes().to_vec(),
            cpu_svn: td_report.tcb_svn.to_vec(),
            report_data: td_report.report_data.to_vec(),
            signature_data: quote.raw_signature_data.clone(),
            debug_enabled,
            security_version,
        }
    }

    /// Verify quote structure and basic constraints (without upstream services)
    pub fn verify_quote_structure(quote: &TdxQuote) -> Result<()> {
        // Check header constraints
        Self::validate_quote_header(&quote.header)?;

        // Check that measurements are not all zeros
        if quote.td_report.mrtd.iter().all(|&b| b == 0) {
            return Err(anyhow!("MRTD measurement is all zeros"));
        }

        // With the new bound, these are always equal
        let expected_total = 48 + 584 + 4 + quote.signature_len as usize;
        let actual_total = 48 + 584 + 4 + quote.raw_signature_data.len();
        if expected_total != actual_total {
            return Err(anyhow!(
                "Signature length mismatch: declared {}, got {}",
                quote.signature_len,
                quote.raw_signature_data.len()
            ));
        }

        Ok(())
    }

    /// Extract user message from report data (for testing)
    pub fn extract_user_message(report_data: &[u8]) -> String {
        // Find null-terminated string
        let end_pos = report_data
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(report_data.len());
        String::from_utf8_lossy(&report_data[..end_pos]).to_string()
    }

    // Helper parsing functions

    fn parse_u16_le(data: &[u8], offset: &mut usize) -> Result<u16> {
        if data.len() < *offset + 2 {
            return Err(anyhow!("Insufficient data for u16"));
        }
        let value = u16::from_le_bytes([data[*offset], data[*offset + 1]]);
        *offset += 2;
        Ok(value)
    }

    fn parse_u32_le(data: &[u8], offset: &mut usize) -> Result<u32> {
        if data.len() < *offset + 4 {
            return Err(anyhow!("Insufficient data for u32"));
        }
        let value = u32::from_le_bytes([
            data[*offset],
            data[*offset + 1],
            data[*offset + 2],
            data[*offset + 3],
        ]);
        *offset += 4;
        Ok(value)
    }

    fn parse_u64_le(data: &[u8], offset: &mut usize) -> Result<u64> {
        if data.len() < *offset + 8 {
            return Err(anyhow!("Insufficient data for u64"));
        }
        let value = u64::from_le_bytes([
            data[*offset],
            data[*offset + 1],
            data[*offset + 2],
            data[*offset + 3],
            data[*offset + 4],
            data[*offset + 5],
            data[*offset + 6],
            data[*offset + 7],
        ]);
        *offset += 8;
        Ok(value)
    }

    fn parse_bytes(data: &[u8], offset: &mut usize, len: usize) -> Result<Vec<u8>> {
        if data.len() < *offset + len {
            return Err(anyhow!("Insufficient data for {} bytes", len));
        }
        let bytes = data[*offset..*offset + len].to_vec();
        *offset += len;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_test_quote() -> Vec<u8> {
        std::fs::read("test_data/real_tdx_quote.bin")
            .expect("Failed to load real TDX quote from test_data/real_tdx_quote.bin")
    }

    #[test]
    fn test_parse_complete_quote() {
        let quote_bytes = get_test_quote();
        let result = TdxParser::parse_quote(&quote_bytes);
        assert!(result.is_ok(), "Failed to parse quote: {:?}", result.err());

        let quote = result.unwrap();
        assert_eq!(quote.header.version, 4);
        assert_eq!(quote.header.tee_type, TEE_TYPE_TDX);
        assert_eq!(quote.signature_len, 4299);
        assert_eq!(quote.raw_signature_data.len(), quote.signature_len as usize);
        assert!(quote.signature.is_some()); // Real quote has parseable signature

        // Verify structure
        assert!(TdxParser::verify_quote_structure(&quote).is_ok());
    }

    #[test]
    fn test_extract_claims() {
        let quote_bytes = get_test_quote();
        let quote = TdxParser::parse_quote(&quote_bytes).unwrap();
        let claims = TdxParser::extract_claims(&quote);

        // Verify key claims
        assert!(
            !claims.mrtd.iter().all(|&b| b == 0),
            "MRTD should not be all zeros"
        );
        assert_eq!(claims.quote_version, 4);
        assert_eq!(claims.tee_type, TEE_TYPE_TDX);
        assert!(!claims.debug_enabled);
        assert!(claims.signature_present);
        assert!(claims.has_valid_signature); // Real quote has parseable signature
        assert!(claims.ephemeral_key.is_some());

        let user_msg = TdxParser::extract_user_message(&claims.report_data);
        assert_eq!(user_msg, "NXCC says: Hello from TDX!");
    }

    #[test]
    fn test_measurements_extraction() {
        let quote_bytes = get_test_quote();
        let quote = TdxParser::parse_quote(&quote_bytes).unwrap();
        let claims = TdxParser::extract_claims(&quote);

        // Verify all measurements are 48 bytes
        assert_eq!(claims.mrtd.len(), 48);
        assert_eq!(claims.mr_config_id.len(), 48);
        assert_eq!(claims.mr_owner.len(), 48);
        assert_eq!(claims.mr_seam.len(), 48);
        assert_eq!(claims.rtmr0.len(), 48);
        assert_eq!(claims.rtmr1.len(), 48);
        assert_eq!(claims.rtmr2.len(), 48);
        assert_eq!(claims.rtmr3.len(), 48);
        assert_eq!(claims.tcb_svn.len(), 16);

        // Verify measurements contain real data
        assert!(!claims.mrtd.iter().all(|&b| b == 0));
        assert!(!claims.rtmr0.iter().all(|&b| b == 0));
    }

    #[test]
    fn test_signature_parsing() {
        let quote_bytes = get_test_quote();
        let quote = TdxParser::parse_quote(&quote_bytes).unwrap();

        // Verify signature data properties
        assert_eq!(quote.signature_len, 4299);
        assert_eq!(quote.raw_signature_data.len(), quote.signature_len as usize);
        assert!(quote.signature.is_some()); // Real quote has full signature data
        assert!(!quote.raw_signature_data.is_empty());
    }

    #[test]
    fn test_standardized_claims_extraction() {
        use crate::tdx::hardware::{TdxInterface, TdxSimulator};

        // Generate a quote using the simulator instead of using stored data
        let simulator = TdxSimulator::new();
        let test_data = b"IEATS/RATS standardized claims test";
        let quote_bytes = simulator.generate_quote(test_data).unwrap();
        let quote = TdxParser::parse_quote(&quote_bytes).unwrap();
        let claims = TdxParser::extract_standardized_claims(&quote);

        // Verify EAT standard fields
        assert!(!claims.measurements.is_empty()); // Should have measurements
        assert_eq!(claims.dbgstat, 0); // Production (debug disabled)
        assert!(claims.ueid.is_some() && !claims.ueid.as_ref().unwrap().is_empty());
        assert_eq!(claims.eat_nonce.as_ref().unwrap().len(), 64); // Report data
        assert!(claims.iat > 0);
        assert_eq!(claims.oemid, Some("intel-tdx".to_string()));

        // Verify measurements vector (7 RTMRs/MRs + 1 MRTD = 8 total)
        assert_eq!(claims.measurements.len(), 8);
        // Find rtmr0 measurement by searching the vector
        let rtmr0_found = claims.measurements.iter().any(|m| {
            m.measurement_type
                .as_ref()
                .is_some_and(|t| t.contains("rtmr0"))
        });
        assert!(rtmr0_found, "Should contain rtmr0 measurement");

        // Verify MRTD measurement is not all zeros - look for it in measurements
        let mrtd_measurement = claims.measurements.iter().find(|m| {
            m.measurement_type
                .as_ref()
                .is_some_and(|t| t.contains("application") || t.contains("mrtd"))
        });
        if let Some(mrtd) = mrtd_measurement {
            assert!(!mrtd.val.iter().all(|&b| b == 0));
        }
    }

    #[test]
    fn test_complete_quote_verification() {
        let quote_bytes = get_test_quote();
        let quote = TdxParser::parse_quote(&quote_bytes).unwrap();

        // Test structure verification - should pass despite signature length mismatch
        let verification_result = TdxParser::verify_quote_structure(&quote);
        assert!(verification_result.is_ok());

        // Test claims extraction
        let claims = TdxParser::extract_claims(&quote);

        // Test user message extraction
        let user_msg = TdxParser::extract_user_message(&claims.report_data);
        assert_eq!(user_msg, "NXCC says: Hello from TDX!");

        // Test measurement validation
        assert!(!claims.mrtd.iter().all(|&b| b == 0));
        assert!(claims.rtmr0.iter().any(|&b| b != 0));
    }
}
