use std::path::PathBuf;

use clap::{Args, Parser};
use serde::{Deserialize, Serialize};

// --- Configuration Structs ---

/// Configuration for the Workerd VM service.
#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
#[command(name = "WorkerdConfig", author, version, about, long_about = None)]
pub struct Config {
    /// Inherited base VM configuration (verbose, server listener settings).
    #[clap(flatten)] // Embed base CLI args directly
    pub base: nxcc_vm_base::config::Config,

    /// Workerd-specific configuration.
    #[clap(flatten)]
    pub workerd: WorkerdConfig,
}

/// Workerd-specific configuration options.
#[derive(Args, Debug, Clone, Serialize, Deserialize)]
pub struct WorkerdConfig {
    /// Path to the `workerd` binary executable.
    #[clap(
        long = "workerd-path",
        default_value = "workerd",
        env = "NXCC_WORKERD_BINARY_PATH"
    )]
    pub binary_path: String,

    /// Timeout in seconds for worker startup.
    #[clap(
        long = "startup-timeout",
        default_value_t = 10,
        env = "NXCC_WORKERD_STARTUP_TIMEOUT"
    )]
    pub startup_timeout_secs: u64,

    /// Interval in milliseconds for checking UDS readiness during startup.
    #[clap(
        long = "uds-check-interval",
        default_value_t = 100,
        env = "NXCC_WORKERD_UDS_CHECK_INTERVAL"
    )]
    pub uds_check_interval_ms: u64,

    /// Prefix for temporary directories created for workers.
    #[clap(
        long = "temp-dir-prefix",
        default_value = "nxcc-workerd-",
        env = "NXCC_WORKERD_TEMP_DIR_PREFIX"
    )]
    pub temp_dir_prefix: String,
}

impl Default for WorkerdConfig {
    fn default() -> Self {
        // Create a temporary parser wrapper to get clap defaults
        #[derive(clap::Parser)]
        struct TempWrapper {
            #[clap(flatten)]
            workerd: WorkerdConfig,
        }

        TempWrapper::try_parse_from(&[""]).unwrap().workerd
    }
}

impl Default for Config {
    fn default() -> Self {
        use clap::Parser;
        // Parse with empty arguments to get clap's default values
        Config::try_parse_from(&[""]).unwrap()
    }
}

impl Config {
    /// Load configuration from environment variables and CLI arguments using clap.
    /// Environment variables use the NXCC_WORKERD_ prefix for workerd-specific settings
    /// and NXCC_VM_ prefix for base VM settings.
    pub fn load() -> Self {
        Config::parse()
    }
}
