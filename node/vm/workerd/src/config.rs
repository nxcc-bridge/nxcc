use std::path::PathBuf;

use clap::{Args, Parser};
use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};

// --- Default value functions ---
fn default_config_path() -> PathBuf {
    PathBuf::from("workerd-vm-config.toml") // Specific default for this binary
}

fn default_workerd_binary_path() -> String {
    "workerd".to_string()
}

fn default_startup_timeout() -> u64 {
    10 // 10 seconds
}

fn default_uds_check_interval() -> u64 {
    100 // 100 milliseconds
}

fn default_temp_dir_prefix() -> String {
    "nxcc-workerd-".to_string()
}

// --- Configuration Structs ---

/// Configuration for the Workerd VM service.
#[derive(Parser, Debug, Clone, Serialize, Deserialize, Default)]
#[command(name = "WorkerdConfig", author, version, about, long_about = None)]
pub struct Config {
    /// Path to the TOML configuration file. Defaults to "workerd-vm-config.toml".
    #[arg(short, long, env = "NXCC_CONFIG_PATH")]
    #[serde(skip)] // Don't serialize the path into the config file itself
    pub config_path: Option<PathBuf>,

    /// Inherited base VM configuration (verbose, server listener settings).
    #[serde(flatten)] // Embed base config fields directly in TOML/JSON
    #[clap(flatten)] // Embed base CLI args directly
    pub base: nxcc_vm_base::config::Config,

    /// Workerd-specific configuration.
    #[serde(default)]
    #[clap(flatten)]
    pub workerd: WorkerdConfig,
}

/// Workerd-specific configuration options.
#[derive(Args, Debug, Clone, Serialize, Deserialize)]
pub struct WorkerdConfig {
    /// Path to the `workerd` binary executable.
    #[clap(long = "workerd-path", default_value_t = default_workerd_binary_path(), env = "NXCC_WORKERD_PATH")]
    #[serde(default = "default_workerd_binary_path")]
    pub binary_path: String,

    /// Timeout in seconds for worker startup.
    #[clap(long = "startup-timeout", default_value_t = default_startup_timeout(), env = "NXCC_STARTUP_TIMEOUT")]
    #[serde(default = "default_startup_timeout")]
    pub startup_timeout_secs: u64,

    /// Interval in milliseconds for checking UDS readiness during startup.
    #[clap(long = "uds-check-interval", default_value_t = default_uds_check_interval(), env = "NXCC_UDS_CHECK_INTERVAL")]
    #[serde(default = "default_uds_check_interval")]
    pub uds_check_interval_ms: u64,

    /// Prefix for temporary directories created for workers.
    #[clap(long = "temp-dir-prefix", default_value_t = default_temp_dir_prefix(), env = "NXCC_TEMP_DIR_PREFIX")]
    #[serde(default = "default_temp_dir_prefix")]
    pub temp_dir_prefix: String,
}

impl Default for WorkerdConfig {
    fn default() -> Self {
        Self {
            binary_path: default_workerd_binary_path(),
            startup_timeout_secs: default_startup_timeout(),
            uds_check_interval_ms: default_uds_check_interval(),
            temp_dir_prefix: default_temp_dir_prefix(),
        }
    }
}

impl Config {
    /// Loads configuration with the following priority (highest first):
    /// 1. Command-line arguments
    /// 2. Environment variables (prefixed with `NXCC_`)
    /// 3. Configuration file (path from CLI/env or default)
    /// 4. Default values
    pub fn load() -> Result<Self, figment::Error> {
        // Parse command-line arguments first to potentially get config path and other overrides
        let cli_args = Self::parse();

        // Determine the config file path: use CLI arg > env var > default
        let config_file_path = cli_args
            .config_path
            .clone()
            .or_else(|| std::env::var("NXCC_CONFIG_PATH").ok().map(PathBuf::from))
            .unwrap_or_else(default_config_path);

        Figment::new()
            // Start with defaults derived from the struct (includes base defaults)
            .merge(Serialized::defaults(Self::default()))
            // Add config file if it exists and is specified/defaulted
            .merge(Toml::file(config_file_path))
            // Add environment variables (NXCC_ prefix)
            .merge(Env::prefixed("NXCC_").map(|key| {
                // Map env vars like NXCC_SERVER_MODE to base.server.mode
                // Map env vars like NXCC_WORKERD_PATH to workerd.binary_path
                let key_str = key.as_str();
                if key_str.starts_with("SERVER_") {
                    format!("base.server.{}", key_str.strip_prefix("SERVER_").unwrap()).into()
                } else if key_str == "VERBOSE" {
                    "base.verbose".into()
                } else if key_str == "WORKERD_PATH" {
                    "workerd.binary_path".into()
                } else if key_str == "STARTUP_TIMEOUT" {
                    "workerd.startup_timeout_secs".into()
                } else if key_str == "UDS_CHECK_INTERVAL" {
                    "workerd.uds_check_interval_ms".into()
                } else if key_str == "TEMP_DIR_PREFIX" {
                    "workerd.temp_dir_prefix".into()
                } else {
                    key.into() // Keep others as is (though unlikely to match top-level fields)
                }
            }))
            // Add CLI arguments (highest priority)
            .merge(Serialized::defaults(cli_args))
            .extract()
    }
}
