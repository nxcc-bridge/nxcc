// TDX Hardware Integration
// Implements actual TDX quote generation using /dev/tdx_guest device

use std::{fs::File, io, os::unix::io::AsRawFd};

use anyhow::{anyhow, Result};

/// TDX IOCTL commands as defined in Linux kernel
const TDX_CMD_GET_REPORT0: u32 = 0xc4004401;

/// Maximum size for TDREPORT structure (1024 bytes as per TDX spec)
const TDREPORT_SIZE: usize = 1024;

/// Maximum user data size for TDX reports (64 bytes)
const TDX_REPORT_DATA_SIZE: usize = 64;

/// TDX report request structure matching kernel interface
#[repr(C)]
#[derive(Debug)]
struct TdxReportReq {
    /// Subtype (currently only 0 is supported)
    subtype: u8,
    /// User data to include in report (up to 64 bytes)
    reportdata: [u8; TDX_REPORT_DATA_SIZE],
    /// Output buffer for TDREPORT (1024 bytes)
    tdreport: [u8; TDREPORT_SIZE],
}

impl Default for TdxReportReq {
    fn default() -> Self {
        Self {
            subtype: 0,
            reportdata: [0; TDX_REPORT_DATA_SIZE],
            tdreport: [0; TDREPORT_SIZE],
        }
    }
}

/// TDX hardware interface for quote generation
pub struct TdxHardware {
    device_path: String,
}

impl Default for TdxHardware {
    fn default() -> Self {
        Self::new()
    }
}

impl TdxHardware {
    pub fn new() -> Self {
        Self {
            device_path: "/dev/tdx_guest".to_string(),
        }
    }

    /// Check if TDX hardware is available
    pub fn is_available(&self) -> bool {
        std::path::Path::new(&self.device_path).exists()
    }

    /// Get TDX report from hardware
    pub fn get_tdreport(&self, user_data: &[u8]) -> Result<Vec<u8>> {
        if !self.is_available() {
            return Err(anyhow!("TDX device not available at {}", self.device_path));
        }

        // Prepare report request
        let mut req = TdxReportReq::default();

        // Copy user data (up to 64 bytes)
        let copy_len = std::cmp::min(user_data.len(), TDX_REPORT_DATA_SIZE);
        req.reportdata[..copy_len].copy_from_slice(&user_data[..copy_len]);

        // Open device and perform IOCTL
        let file = File::options()
            .read(true)
            .write(true)
            .open(&self.device_path)
            .map_err(|e| anyhow!("Failed to open TDX device: {}", e))?;

        let fd = file.as_raw_fd();

        // Perform IOCTL call
        let result = unsafe {
            libc::ioctl(
                fd,
                TDX_CMD_GET_REPORT0 as libc::c_ulong,
                &mut req as *mut TdxReportReq,
            )
        };

        if result != 0 {
            let errno = io::Error::last_os_error();
            return Err(anyhow!("TDX IOCTL failed: {}", errno));
        }

        // Return the TDREPORT
        Ok(req.tdreport.to_vec())
    }

    /// Generate a complete TDX quote (requires additional attestation service)
    pub fn generate_quote(&self, user_data: &[u8]) -> Result<Vec<u8>> {
        // Step 1: Get TDREPORT from hardware
        let tdreport = self.get_tdreport(user_data)?;

        // Step 2: Send TDREPORT to Quoting Enclave (QE) for quote generation
        // This typically involves contacting a local or remote attestation service
        // For now, we'll return a placeholder that indicates this needs QE integration

        log::warn!("TDX quote generation requires Quoting Enclave integration");
        log::info!("Generated TDREPORT of {} bytes", tdreport.len());

        // In a real implementation, this would contact:
        // - Local QE via AESM service
        // - Cloud attestation service (GCS, Azure Attestation, etc.)
        // - Intel Attestation Service

        Err(anyhow!(
            "Quote generation requires Quoting Enclave - only TDREPORT available"
        ))
    }
}

/// TDX hardware simulator for testing without real hardware
pub struct TdxSimulator {
    simulate_failures: bool,
    custom_measurements: Option<TdxSimulatorConfig>,
}

#[derive(Debug, Clone)]
pub struct TdxSimulatorConfig {
    pub mrtd: [u8; 48],
    pub rtmr0: [u8; 48],
    pub rtmr1: [u8; 48],
    pub rtmr2: [u8; 48],
    pub rtmr3: [u8; 48],
    pub debug_enabled: bool,
}

impl Default for TdxSimulatorConfig {
    fn default() -> Self {
        Self {
            mrtd: [0x42; 48], // Dummy measurement
            rtmr0: [0x11; 48],
            rtmr1: [0x22; 48],
            rtmr2: [0x33; 48],
            rtmr3: [0x44; 48],
            debug_enabled: false,
        }
    }
}

impl Default for TdxSimulator {
    fn default() -> Self {
        Self::new()
    }
}

impl TdxSimulator {
    pub fn new() -> Self {
        Self {
            simulate_failures: false,
            custom_measurements: None,
        }
    }

    pub fn with_config(config: TdxSimulatorConfig) -> Self {
        Self {
            simulate_failures: false,
            custom_measurements: Some(config),
        }
    }

    pub fn with_failures(mut self) -> Self {
        self.simulate_failures = true;
        self
    }

    /// Simulate TDX TDREPORT generation
    pub fn get_tdreport(&self, user_data: &[u8]) -> Result<Vec<u8>> {
        if self.simulate_failures {
            return Err(anyhow!("Simulated TDX hardware failure"));
        }

        // Create a simulated TDREPORT structure
        let mut tdreport = vec![0u8; TDREPORT_SIZE];

        let default_config = TdxSimulatorConfig::default();
        let config = self.custom_measurements.as_ref().unwrap_or(&default_config);

        // Simulate TDREPORT structure (simplified version)
        // Real TDREPORT has complex structure with many fields

        // TEE TCB SVN (16 bytes at offset 0)
        tdreport[0..16].copy_from_slice(&[
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10,
        ]);

        // MRTD (48 bytes at offset 112)
        tdreport[112..160].copy_from_slice(&config.mrtd);

        // RTMRs (4 x 48 bytes starting at offset 304)
        tdreport[304..352].copy_from_slice(&config.rtmr0);
        tdreport[352..400].copy_from_slice(&config.rtmr1);
        tdreport[400..448].copy_from_slice(&config.rtmr2);
        tdreport[448..496].copy_from_slice(&config.rtmr3);

        // TD Attributes (8 bytes at offset 64) - include debug flag
        let mut attributes = 0u64;
        if config.debug_enabled {
            attributes |= 0x1; // Debug bit
        }
        tdreport[64..72].copy_from_slice(&attributes.to_le_bytes());

        // Report data (64 bytes at offset 960)
        let copy_len = std::cmp::min(user_data.len(), TDX_REPORT_DATA_SIZE);
        tdreport[960..960 + copy_len].copy_from_slice(&user_data[..copy_len]);

        Ok(tdreport)
    }

    /// Simulate complete quote generation with simulated QE
    pub fn generate_quote(&self, user_data: &[u8]) -> Result<Vec<u8>> {
        // Get simulated TDREPORT
        let tdreport = self.get_tdreport(user_data)?;

        // Create a simulated quote using our real parser's test data as template
        use base64::{engine::general_purpose, Engine as _};
        let base_quote = general_purpose::STANDARD.decode(
            "BAACAIEAAAAAAAAAk5pyM/ecTKmUCg2zlX8GB5/OUj/OJupF09PbkG1RcaEAAAAAAwAFAAAAAAAAAAAAAAAAAC/SecFhZKk91b83PYNDKNRgCMK2k6+eu4ZbCLLO0yDJqJtIaan6tg++nQxaU2PGVgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAEAAAAADnAgYAAAAAALZeoAnkJOb3Yf3T18iWJDlFOzfs32LaBPe8XTJ2hruLr8il0kqcMc7mDkq6h8L3GwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAOGvdeYZJ0EOQrVLOfZoHPmwv7rlErFehw5MjZ1aXLOFVxsOHcL3C/nM7whWDworWCFf8fwMMUQsHwYaMXvkCUCxgsE9Q8bbLlsqV33em+6T1FKv091GxuEvmzA5EvMQsQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAEhlbGxvIGZyb20gRWRnZWxlc3MgU3lzdGVtcyEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADMEAAAYbPmffGRNtL5ViDWxe44+/k3th7PC6R186hE9iAfQQG6Mf45s2kK"
        ).map_err(|e| anyhow!("Failed to decode base quote: {}", e))?;

        let mut quote = base_quote;

        // Replace TD Report section with our simulated data
        // Quote structure: Header(48) + TD Report(584) + Signature Length(4) + Signature Data
        let tdreport_in_quote = &tdreport[0..584]; // TD Report is first 584 bytes of TDREPORT
        quote[48..48 + 584].copy_from_slice(tdreport_in_quote);

        // Update user data in the quote (offset 48 + 520 = 568)
        let report_data_offset = 48 + 520;
        let copy_len = std::cmp::min(user_data.len(), TDX_REPORT_DATA_SIZE);
        quote[report_data_offset..report_data_offset + copy_len]
            .copy_from_slice(&user_data[..copy_len]);

        Ok(quote)
    }
}

/// Unified TDX interface that tries hardware first, falls back to simulator
pub struct TdxInterface {
    hardware: TdxHardware,
    simulator: Option<TdxSimulator>,
    force_simulation: bool,
}

impl Default for TdxInterface {
    fn default() -> Self {
        Self::new()
    }
}

impl TdxInterface {
    pub fn new() -> Self {
        Self {
            hardware: TdxHardware::new(),
            simulator: None,
            force_simulation: false,
        }
    }

    pub fn with_simulator(mut self, simulator: TdxSimulator) -> Self {
        self.simulator = Some(simulator);
        self
    }

    pub fn force_simulation(mut self) -> Self {
        self.force_simulation = true;
        self
    }

    pub fn is_hardware_available(&self) -> bool {
        !self.force_simulation && self.hardware.is_available()
    }

    /// Get TDREPORT using hardware or simulator
    pub fn get_tdreport(&self, user_data: &[u8]) -> Result<Vec<u8>> {
        if self.is_hardware_available() {
            log::info!("Using TDX hardware for TDREPORT generation");
            self.hardware.get_tdreport(user_data)
        } else if let Some(ref simulator) = self.simulator {
            log::info!("Using TDX simulator for TDREPORT generation");
            simulator.get_tdreport(user_data)
        } else {
            Err(anyhow!(
                "No TDX hardware available and no simulator configured"
            ))
        }
    }

    /// Generate quote using hardware or simulator
    pub fn generate_quote(&self, user_data: &[u8]) -> Result<Vec<u8>> {
        if self.is_hardware_available() {
            log::info!("Attempting TDX hardware quote generation");
            // Try hardware first, but it will likely fail without QE
            match self.hardware.generate_quote(user_data) {
                Ok(quote) => Ok(quote),
                Err(e) => {
                    log::warn!("Hardware quote generation failed: {}", e);
                    // Fall back to simulator if available
                    if let Some(ref simulator) = self.simulator {
                        log::info!("Falling back to TDX simulator for quote generation");
                        simulator.generate_quote(user_data)
                    } else {
                        Err(e)
                    }
                }
            }
        } else if let Some(ref simulator) = self.simulator {
            log::info!("Using TDX simulator for quote generation");
            simulator.generate_quote(user_data)
        } else {
            Err(anyhow!(
                "No TDX hardware available and no simulator configured"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tdx_hardware_availability() {
        let hw = TdxHardware::new();
        // This will be false on non-TDX systems
        let available = hw.is_available();
        println!("TDX hardware available: {}", available);
    }

    #[test]
    fn test_tdx_simulator_tdreport() {
        let sim = TdxSimulator::new();
        let user_data = b"test data for simulator";

        let tdreport = sim.get_tdreport(user_data).unwrap();
        assert_eq!(tdreport.len(), TDREPORT_SIZE);

        // Check that user data was embedded
        assert_eq!(&tdreport[960..960 + user_data.len()], user_data);
    }

    #[test]
    fn test_tdx_simulator_custom_config() {
        let config = TdxSimulatorConfig {
            mrtd: [0xFF; 48],
            debug_enabled: true,
            ..Default::default()
        };

        let sim = TdxSimulator::with_config(config);
        let tdreport = sim.get_tdreport(b"test").unwrap();

        // Check custom MRTD
        assert_eq!(&tdreport[112..160], &[0xFF; 48]);

        // Check debug flag is set
        let attributes = u64::from_le_bytes(tdreport[64..72].try_into().unwrap());
        assert_eq!(attributes & 0x1, 1);
    }

    #[test]
    fn test_tdx_simulator_quote_generation() {
        let sim = TdxSimulator::new();
        let user_data = b"quote test data";

        let quote = sim.generate_quote(user_data).unwrap();
        assert!(quote.len() > 600); // Should be a valid quote size

        // Parse the quote to verify it's valid
        use crate::tdx::parser::TdxParser;
        let parsed_quote = TdxParser::parse_quote(&quote).unwrap();
        let claims = TdxParser::extract_claims(&parsed_quote);

        // Verify user data was embedded
        let extracted_msg = TdxParser::extract_user_message(&claims.report_data);
        assert!(extracted_msg.starts_with("quote test"));
    }

    #[test]
    fn test_tdx_interface_simulation() {
        let sim = TdxSimulator::new();
        let interface = TdxInterface::new().with_simulator(sim).force_simulation();

        assert!(!interface.is_hardware_available());

        let quote = interface.generate_quote(b"interface test").unwrap();
        assert!(quote.len() > 600);
    }

    #[test]
    fn test_tdx_simulator_failure_mode() {
        let sim = TdxSimulator::new().with_failures();
        let result = sim.get_tdreport(b"test");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Simulated TDX hardware failure"));
    }
}
