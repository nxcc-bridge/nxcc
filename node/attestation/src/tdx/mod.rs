use std::{
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
};

use anyhow::Result;

pub mod hardware;
pub mod parser;

// Re-export commonly used items from parser
pub use parser::{
    ParsedTdxQuote, QuoteSignature, TdReport, TdxAttestationClaims, TdxQuote, TdxQuoteHeader,
    QUOTE_VERSION_4, TEE_TYPE_TDX,
};

/// TDX Guest interface for quote generation
pub struct TdxGuest;

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

    /// Parse a TDX quote and extract key information
    pub fn parse_quote(quote_bytes: &[u8]) -> Result<TdxQuote> {
        TdxQuote::parse(quote_bytes)
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

        let result = TdxQuote::parse(&quote);
        assert!(result.is_ok());

        let parsed_quote = result.unwrap();
        assert_eq!(parsed_quote.header.version, 4);
        assert_eq!(parsed_quote.header.tee_type, 0x00000081);
    }
}
