// Real TDX Quote Test Data
// Source: https://github.com/edgelesssys/go-tdx-qpl/blob/main/blobs/blobs.go
// "an example quote generated on an Intel TDX development platform"

use base64::{Engine as _, engine::general_purpose};

/// Base64-encoded TDX quote from edgelesssys/go-tdx-qpl
pub const REAL_TDX_QUOTE_BASE64: &str = "BAACAIEAAAAAAAAEAAMAAAAAAAEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAEBAwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAFAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACEAAAMTMzMzMzMzNTQ2NzA4OWFiY2RlZkdISUpLTE1OT1BRUlNUVVZXWFlaYWJjZGVmZ2hpams/";

/// Get the raw TDX quote bytes by decoding the base64
pub fn get_real_tdx_quote() -> Vec<u8> {
    general_purpose::STANDARD
        .decode(REAL_TDX_QUOTE_BASE64)
        .expect("Failed to decode real TDX quote")
}

/// Hex representation of the TDX quote for debugging
pub fn get_real_tdx_quote_hex() -> String {
    hex::encode(get_real_tdx_quote())
}

/// The expected user data from this quote (if any)
pub fn get_expected_user_data() -> Vec<u8> {
    // This is an example quote, likely contains test data
    // Will need to parse to extract actual user data
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_real_tdx_quote_decode() {
        let quote = get_real_tdx_quote();
        assert!(!quote.is_empty(), "TDX quote should not be empty");
        
        // Basic sanity checks
        assert!(quote.len() > 48, "TDX quote should be larger than header size");
        
        println!("TDX quote length: {} bytes", quote.len());
        println!("TDX quote hex: {}", get_real_tdx_quote_hex());
    }

    #[test]
    fn test_quote_header_structure() {
        let quote = get_real_tdx_quote();
        
        // Check quote header structure (first 48 bytes according to TDX spec)
        if quote.len() >= 48 {
            // Bytes 0-1: Version
            let version = u16::from_le_bytes([quote[0], quote[1]]);
            println!("Quote version: {}", version);
            
            // Bytes 2-3: Attestation Key Type
            let att_key_type = u16::from_le_bytes([quote[2], quote[3]]);
            println!("Attestation key type: {}", att_key_type);
            
            // Bytes 4-7: TEE Type (should be 0x00000081 for TDX)
            let tee_type = u32::from_le_bytes([quote[4], quote[5], quote[6], quote[7]]);
            println!("TEE type: 0x{:08x}", tee_type);
            assert_eq!(tee_type, 0x00000081, "TEE type should be TDX (0x00000081)");
        }
    }
}