use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
};

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub mod hardware;
pub mod parser;
pub mod quote_parser;

// Re-export commonly used items
pub use parser::{TdxAttestationClaims, TdxParser, QUOTE_VERSION_4, TEE_TYPE_TDX};

/// TDX Guest interface for quote generation
pub struct TdxGuest;

/// TDX quote structure based on Intel TDX specification
#[derive(Debug, Clone)]
pub struct TdxQuote {
    /// Quote header
    pub header: TdxQuoteHeader,
    /// TDX report data
    pub td_report: TdReport,
    /// Quote signature data
    pub signature_data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct TdxQuoteHeader {
    /// Version of the quote
    pub version: u16,
    /// Attestation key type
    pub ak_type: u16,
    /// TEE type (4 for TDX)
    pub tee_type: u32,
    /// Quote signature data length
    pub qe_svn: u16,
    /// PCE security version number
    pub pce_svn: u16,
    /// QE vendor ID
    pub qe_vendor_id: [u8; 16],
    /// User data (first 20 bytes)
    pub user_data: [u8; 20],
}

#[derive(Debug, Clone)]
pub struct TdReport {
    /// Report type
    pub report_type: u8,
    /// CPU security version number
    pub cpu_svn: [u8; 16],
    /// TEE TCB info
    pub tee_tcb_info: [u8; 239],
    /// TEE info
    pub tee_info: [u8; 512],
    /// Report data (contains user data)
    pub report_data: [u8; 64],
}

/// TDX measurement register data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TdxMeasurements {
    /// Measurement of the initial contents of the TD (MRTD)
    pub mrtd: Vec<u8>,
    /// Runtime measurement registers (RTMRs)
    pub rtmr0: Vec<u8>,
    pub rtmr1: Vec<u8>,
    pub rtmr2: Vec<u8>,
    pub rtmr3: Vec<u8>,
    /// Security version number
    pub security_version: u64,
    /// Debug flag
    pub debug: bool,
}

/// Parsed and verified TDX quote data
#[derive(Debug, Clone)]
pub struct TdxQuoteData {
    /// Software measurement (MRTD)
    pub mrtd: Vec<u8>,
    /// Runtime measurements (RTMRs)
    pub rtmrs: HashMap<String, Vec<u8>>,
    /// Security version number
    pub security_version: u64,
    /// Whether debug mode is disabled
    pub debug_disabled: bool,
    /// User data from quote
    pub user_data: Vec<u8>,
    /// Timestamp when quote was generated
    pub timestamp: u64,
    /// TEE TCB SVN
    pub tcb_svn: Vec<u8>,
}

impl TdxGuest {
    /// Check if TDX is available on this system
    pub fn is_available() -> bool {
        std::path::Path::new("/dev/tdx_guest").exists()
    }

    /// Generate a TDX quote with the provided report data
    pub fn get_quote(report_data: &[u8; 64]) -> Result<Vec<u8>> {
        if !Self::is_available() {
            anyhow::bail!("TDX guest device not available at /dev/tdx_guest");
        }

        // Open the TDX guest device
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .mode(0o666)
            .open("/dev/tdx_guest")
            .map_err(|e| anyhow::anyhow!("Failed to open /dev/tdx_guest: {}", e))?;

        // TDX_CMD_GET_QUOTE command structure
        // This is a simplified version - actual implementation would need proper IOCTL
        let mut quote_buffer = vec![0u8; 8192]; // Typical quote size

        // Write report data to device (simplified - actual implementation uses ioctl)
        file.write_all(report_data)
            .map_err(|e| anyhow::anyhow!("Failed to write report data: {}", e))?;

        // Read quote from device (simplified - actual implementation uses ioctl)
        let bytes_read = file
            .read(&mut quote_buffer)
            .map_err(|e| anyhow::anyhow!("Failed to read quote: {}", e))?;

        quote_buffer.truncate(bytes_read);
        Ok(quote_buffer)
    }

    /// Generate a TD report (used internally by quote generation)
    pub fn get_td_report(report_data: &[u8; 64]) -> Result<TdReport> {
        // In a real implementation, this would make an IOCTL call to get the TD report
        // For now, we'll create a mock report structure

        if !Self::is_available() {
            anyhow::bail!("TDX guest device not available");
        }

        // Mock TD report - in practice this would come from the TDX module
        Ok(TdReport {
            report_type: 0x81,        // TDX report type
            cpu_svn: [0u8; 16],       // Would contain actual CPU SVN
            tee_tcb_info: [0u8; 239], // Would contain actual TCB info
            tee_info: [0u8; 512],     // Would contain actual TEE info
            report_data: *report_data,
        })
    }

    /// Parse a TDX quote and extract key information
    pub fn parse_quote(quote_bytes: &[u8]) -> Result<TdxQuote> {
        use crate::tdx::quote_parser::TdxQuoteV4;

        // Use the proper quote parser
        let parsed_quote = TdxQuoteV4::parse(quote_bytes)?;
        let claims = parsed_quote.extract_claims();

        // Convert to legacy format for compatibility
        let header = TdxQuoteHeader {
            version: claims.version,
            ak_type: 0, // Not used in V4
            tee_type: claims.tee_type,
            qe_svn: 0,               // Not directly available in V4
            pce_svn: 0,              // Not directly available in V4
            qe_vendor_id: [0u8; 16], // Would need to extract from signature data
            user_data: claims.report_data[..20].try_into().unwrap_or([0u8; 20]),
        };

        let td_report = TdReport {
            report_type: 0x81, // TDX report type
            cpu_svn: claims.cpu_svn.try_into().unwrap_or([0u8; 16]),
            tee_tcb_info: [0u8; 239], // Would need proper parsing
            tee_info: {
                let mut tee_info = [0u8; 512];
                // Pack MRTD and RTMRs into tee_info for legacy compatibility
                if claims.mrtd.len() <= 48 {
                    tee_info[0..claims.mrtd.len()].copy_from_slice(&claims.mrtd);
                }
                let mut offset = 48;
                for rtmr in claims.rtmrs.values() {
                    if offset + rtmr.len() <= 512 {
                        let end = offset + rtmr.len();
                        tee_info[offset..end].copy_from_slice(rtmr);
                        offset = end;
                    }
                }
                tee_info
            },
            report_data: claims.report_data.try_into().unwrap_or([0u8; 64]),
        };

        Ok(TdxQuote {
            header,
            td_report,
            signature_data: claims.signature_data,
        })
    }

    /// Extract measurements and claims from a TDX quote
    pub fn extract_quote_data(quote: &TdxQuote) -> Result<TdxQuoteData> {
        // Extract from the parsed TEE info structure
        let tee_info = &quote.td_report.tee_info;

        // MRTD is at offset 0 in TEE info (48 bytes)
        let mrtd = if tee_info.len() >= 48 {
            tee_info[0..48].to_vec()
        } else {
            vec![0u8; 48]
        };

        // RTMRs are at specific offsets (48 bytes each)
        let mut rtmrs = HashMap::new();
        if tee_info.len() >= 240 {
            // 48 + 4*48
            rtmrs.insert("rtmr0".to_string(), tee_info[48..96].to_vec());
            rtmrs.insert("rtmr1".to_string(), tee_info[96..144].to_vec());
            rtmrs.insert("rtmr2".to_string(), tee_info[144..192].to_vec());
            rtmrs.insert("rtmr3".to_string(), tee_info[192..240].to_vec());
        }

        // Extract security version from CPU SVN
        let security_version = if quote.td_report.cpu_svn.len() >= 8 {
            u64::from_le_bytes(quote.td_report.cpu_svn[0..8].try_into().unwrap_or([0u8; 8]))
        } else {
            0
        };

        // Debug flag is inverted from the header version (we store debug_disabled)
        let debug_disabled = quote.header.version >= 4; // Assume newer versions disable debug by default

        // User data from report data
        let user_data = quote.td_report.report_data.to_vec();

        // TCB SVN from CPU SVN
        let tcb_svn = quote.td_report.cpu_svn.to_vec();

        Ok(TdxQuoteData {
            mrtd,
            rtmrs,
            security_version,
            debug_disabled,
            user_data,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            tcb_svn,
        })
    }

    /// Create report data from user data and ephemeral key
    pub fn create_report_data(user_data: &[u8], ephemeral_key: &[u8]) -> [u8; 64] {
        use sha2::{Digest, Sha512};

        let mut report_data = [0u8; 64];

        if user_data.len() + ephemeral_key.len() <= 64 {
            // If combined data fits, use it directly
            let combined_len = user_data.len() + ephemeral_key.len();
            report_data[..ephemeral_key.len()].copy_from_slice(ephemeral_key);
            report_data[ephemeral_key.len()..combined_len].copy_from_slice(user_data);
        } else {
            // If too large, hash the combined data
            let mut hasher = Sha512::new();
            hasher.update(ephemeral_key);
            hasher.update(user_data);
            let hash = hasher.finalize();
            report_data.copy_from_slice(&hash[..64]);
        }

        report_data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_available() {
        // This will return false in most test environments
        let available = TdxGuest::is_available();
        println!("TDX available: {}", available);
    }

    #[test]
    fn test_create_report_data() {
        let user_data = b"test user data";
        let ephemeral_key = &[1u8; 32];

        let report_data = TdxGuest::create_report_data(user_data, ephemeral_key);

        // First 32 bytes should be the ephemeral key
        assert_eq!(&report_data[..32], ephemeral_key);
        // Next bytes should be user data
        assert_eq!(&report_data[32..32 + user_data.len()], user_data);
    }

    #[test]
    fn test_create_report_data_large() {
        let user_data = &[2u8; 100]; // Large user data
        let ephemeral_key = &[1u8; 32];

        let report_data = TdxGuest::create_report_data(user_data, ephemeral_key);

        // Should be hashed since combined data is > 64 bytes
        assert_ne!(&report_data[..32], ephemeral_key); // Should be different due to hashing
    }

    #[test]
    fn test_parse_invalid_quote() {
        let short_quote = vec![0u8; 10];
        let result = TdxGuest::parse_quote(&short_quote);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_mock_quote() {
        // Create a mock quote with proper structure for TDX v4
        let mut quote = vec![0u8; 1024];

        // Set version to 4 (required for TDX v4)
        quote[0..2].copy_from_slice(&4u16.to_le_bytes());
        // Set TEE type to TDX (0x81)
        quote[4..8].copy_from_slice(&0x00000081u32.to_le_bytes());

        // Set signature data length at the correct offset (48 + 584 = 632)
        let sig_len_offset = 48 + 584;
        if quote.len() > sig_len_offset + 4 {
            quote[sig_len_offset..sig_len_offset + 4].copy_from_slice(&64u32.to_le_bytes());
        }

        let result = TdxGuest::parse_quote(&quote);
        assert!(result.is_ok());

        let parsed_quote = result.unwrap();
        assert_eq!(parsed_quote.header.version, 4);
        assert_eq!(parsed_quote.header.tee_type, 0x00000081);
    }
}
