use std::path::PathBuf;

use clap::Parser;
use serde::{Deserialize, Serialize};

/// Configuration for the Enclave service
#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
#[command(author, version, about, long_about = None)]
pub struct EnclaveConfig {
    #[arg(short, long, default_value_t = false, env = "NXCC_ENCLAVE_VERBOSE")]
    pub verbose: bool,

    /// gRPC server configuration
    #[clap(flatten)]
    pub grpc: GrpcConfig,
}

/// Configuration for the gRPC interface
#[derive(Debug, Clone, Serialize, Deserialize, clap::Args)]
pub struct GrpcConfig {
    /// Mode for the gRPC interface: "vsock" or "uds"
    #[clap(
        long = "grpc-mode",
        default_value = "uds",
        env = "NXCC_ENCLAVE_GRPC_MODE"
    )]
    pub mode: String,

    /// When using vsock: the vsock CID (e.g. host CID)
    #[clap(
        long = "grpc-vsock-cid",
        default_value_t = 3,
        env = "NXCC_ENCLAVE_GRPC_VSOCK_CID"
    )]
    pub vsock_cid: u32,

    /// When using vsock: the vsock port to listen on
    #[clap(
        long = "grpc-vsock-port",
        default_value_t = 60051,
        env = "NXCC_ENCLAVE_GRPC_VSOCK_PORT"
    )]
    pub vsock_port: u32,

    /// When using UDS: the path to the Unix Domain Socket
    #[clap(
        long = "grpc-uds-path",
        default_value = "/tmp/nxcc_enclave.sock",
        env = "NXCC_ENCLAVE_GRPC_UDS_PATH"
    )]
    pub uds_path: String,
}

impl Default for GrpcConfig {
    fn default() -> Self {
        // Create a temporary parser wrapper to get clap defaults
        #[derive(clap::Parser)]
        struct TempWrapper {
            #[clap(flatten)]
            grpc: GrpcConfig,
        }

        TempWrapper::try_parse_from(&[""]).unwrap().grpc
    }
}

impl Default for EnclaveConfig {
    fn default() -> Self {
        use clap::Parser;
        // Parse with empty arguments to get clap's default values
        EnclaveConfig::try_parse_from(&[""]).unwrap()
    }
}

impl EnclaveConfig {
    /// Load configuration from environment variables and CLI arguments using clap.
    /// Environment variables use the NXCC_ENCLAVE_ prefix.
    pub fn load() -> Self {
        EnclaveConfig::parse()
    }
}
