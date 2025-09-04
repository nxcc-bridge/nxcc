use std::path::PathBuf;

use clap::Parser;
use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};

/// Configuration for the Enclave service
#[derive(Parser, Debug, Clone, Serialize, Deserialize, Default)]
#[command(author, version, about, long_about = None)]
pub struct EnclaveConfig {
    /// Path to the TOML configuration file
    #[arg(short, long)]
    #[serde(skip)]
    pub config_path: Option<PathBuf>,

    #[arg(short, long, default_value_t = false)]
    pub verbose: bool,

    /// gRPC server configuration
    #[serde(default)]
    #[clap(flatten)]
    pub grpc: GrpcConfig,

    /// Enable test attestation providers for integration testing
    #[cfg(feature = "test-attestation")]
    #[arg(long)]
    #[serde(default)]
    pub enable_test_providers: bool,
}

/// Configuration for the gRPC interface
#[derive(Debug, Clone, Serialize, Deserialize, clap::Args)]
pub struct GrpcConfig {
    /// Mode for the gRPC interface: "vsock" or "uds"
    #[clap(long = "grpc-mode", default_value_t = default_grpc_mode())]
    #[serde(default = "default_grpc_mode")]
    pub mode: String,

    /// When using vsock: the vsock CID (e.g. host CID)
    #[clap(long = "grpc-vsock-cid", default_value_t = default_vsock_cid())]
    #[serde(default = "default_vsock_cid")]
    pub vsock_cid: u32,

    /// When using vsock: the vsock port to listen on
    #[clap(long = "grpc-vsock-port", default_value_t = default_vsock_port())]
    #[serde(default = "default_vsock_port")]
    pub vsock_port: u32,

    /// When using UDS: the path to the Unix Domain Socket
    #[clap(long = "grpc-uds-path", default_value_t = default_uds_path())]
    #[serde(default = "default_uds_path")]
    pub uds_path: String,
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            mode: default_grpc_mode(),
            vsock_cid: default_vsock_cid(),
            vsock_port: default_vsock_port(),
            uds_path: default_uds_path(),
        }
    }
}

// Default value functions
fn default_grpc_mode() -> String {
    "uds".to_string()
}

fn default_vsock_cid() -> u32 {
    3
}

fn default_vsock_port() -> u32 {
    60051
}

fn default_uds_path() -> String {
    "/tmp/nxcc_enclave.sock".to_string()
}

fn default_config_path() -> PathBuf {
    PathBuf::from("config.toml")
}

impl EnclaveConfig {
    /// Loads configuration with the following priority (highest first):
    /// 1. Command-line arguments
    /// 2. Environment variables (prefixed with `NXCC_`)
    /// 3. Configuration file
    /// 4. Default values
    pub fn load() -> Result<Self, figment::Error> {
        // Parse command-line arguments first to get potential config path
        let cli_args = Self::parse();

        // Determine the config file path: use CLI arg or default
        let config_file_path = cli_args
            .config_path
            .clone()
            .unwrap_or_else(default_config_path);

        let mut figment = Figment::new()
            // Start with defaults
            .merge(Serialized::defaults(Self::default()))
            // Add config file if it exists
            .merge(Toml::file(config_file_path))
            // Add environment variables
            .merge(Env::prefixed("NXCC_"));

        // Only merge CLI arguments that were explicitly provided
        // This prevents default CLI values from overriding config file values
        if cli_args.config_path.is_some() {
            figment = figment.merge(("config_path", cli_args.config_path));
        }
        if cli_args.verbose {
            figment = figment.merge(("verbose", cli_args.verbose));
        }
        // Only merge grpc config if any grpc arguments were provided
        figment = figment.merge(("grpc", cli_args.grpc));

        #[cfg(feature = "test-attestation")]
        {
            // Check if --enable-test-providers was explicitly provided
            use clap::Parser;
            let args: Vec<String> = std::env::args().collect();
            if args.iter().any(|arg| arg == "--enable-test-providers") {
                figment = figment.merge(("enable_test_providers", true));
            }
        }

        // Extract the final configuration
        figment.extract()
    }
}
