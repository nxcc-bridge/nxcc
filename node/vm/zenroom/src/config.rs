use clap::{Args, Parser};
use serde::{Deserialize, Serialize};

/// Configuration for the Zenroom VM service.
#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
#[command(name = "ZenroomConfig", author, version, about, long_about = None)]
pub struct Config {
    /// Inherited base VM configuration (verbose, server listener settings).
    #[clap(flatten)]
    pub base: nxcc_vm_base::config::Config,

    /// Zenroom-specific configuration.
    #[clap(flatten)]
    pub zenroom: ZenroomConfig,
}

/// Zenroom-specific configuration options.
#[derive(Args, Debug, Clone, Serialize, Deserialize)]
pub struct ZenroomConfig {
    /// Path to the `zencode-exec` binary executable.
    #[clap(
        long = "zencode-exec-path",
        default_value = "zencode-exec",
        env = "NXCC_ZENROOM_VM_ZENCODE_EXEC_PATH"
    )]
    pub zencode_exec_path: String,

    /// Path to the `lua-exec` binary executable (for Lua mode).
    #[clap(
        long = "lua-exec-path",
        default_value = "lua-exec",
        env = "NXCC_ZENROOM_VM_LUA_EXEC_PATH"
    )]
    pub lua_exec_path: String,

    /// Hard timeout per invocation, in milliseconds.
    #[clap(
        long = "exec-timeout-ms",
        default_value_t = 10_000,
        env = "NXCC_ZENROOM_VM_EXEC_TIMEOUT_MS"
    )]
    pub exec_timeout_ms: u64,

    /// Enable postback execution.
    #[clap(
        long = "postback-enabled",
        default_value_t = false,
        env = "NXCC_ZENROOM_VM_POSTBACK_ENABLED"
    )]
    pub postback_enabled: bool,

    /// Comma-separated list of allowed host suffixes.
    #[clap(
        long = "postback-allowed-host-suffixes",
        value_delimiter = ',',
        env = "NXCC_ZENROOM_VM_POSTBACK_ALLOWED_HOST_SUFFIXES"
    )]
    pub postback_allowed_host_suffixes: Vec<String>,

    /// Comma-separated list of allowed URL schemes.
    #[clap(
        long = "postback-allowed-schemes",
        value_delimiter = ',',
        default_value = "https",
        env = "NXCC_ZENROOM_VM_POSTBACK_ALLOWED_SCHEMES"
    )]
    pub postback_allowed_schemes: Vec<String>,

    /// Comma-separated list of allowed ports.
    #[clap(
        long = "postback-allowed-ports",
        value_delimiter = ',',
        default_value = "443",
        env = "NXCC_ZENROOM_VM_POSTBACK_ALLOWED_PORTS"
    )]
    pub postback_allowed_ports: Vec<u16>,

    /// Block private or non-global IP destinations.
    #[clap(
        long = "postback-block-private-ips",
        default_value_t = true,
        env = "NXCC_ZENROOM_VM_POSTBACK_BLOCK_PRIVATE_IPS"
    )]
    pub postback_block_private_ips: bool,

    /// Timeout for postback requests, in milliseconds.
    #[clap(
        long = "postback-timeout-ms",
        default_value_t = 5_000,
        env = "NXCC_ZENROOM_VM_POSTBACK_TIMEOUT_MS"
    )]
    pub postback_timeout_ms: u64,

    /// Maximum stdout bytes captured per invocation.
    #[clap(
        long = "max-stdout-bytes",
        default_value_t = 256 * 1024,
        env = "NXCC_ZENROOM_VM_MAX_STDOUT_BYTES"
    )]
    pub max_stdout_bytes: usize,

    /// Maximum stderr bytes captured per invocation.
    #[clap(
        long = "max-stderr-bytes",
        default_value_t = 256 * 1024,
        env = "NXCC_ZENROOM_VM_MAX_STDERR_BYTES"
    )]
    pub max_stderr_bytes: usize,

    /// Maximum script size in bytes.
    #[clap(
        long = "max-script-bytes",
        default_value_t = 512 * 1024,
        env = "NXCC_ZENROOM_VM_MAX_SCRIPT_BYTES"
    )]
    pub max_script_bytes: usize,

    /// Maximum total size of secrets (raw or derived) in bytes.
    #[clap(
        long = "max-total-secrets-bytes",
        default_value_t = 64 * 1024,
        env = "NXCC_ZENROOM_VM_MAX_TOTAL_SECRETS_BYTES"
    )]
    pub max_total_secrets_bytes: usize,

    /// Maximum derived key length.
    #[clap(
        long = "max-derived-key-len",
        default_value_t = 64,
        env = "NXCC_ZENROOM_VM_MAX_DERIVED_KEY_LEN"
    )]
    pub max_derived_key_len: usize,

    /// Maximum size of the Zenroom conf string.
    #[clap(
        long = "max-conf-bytes",
        default_value_t = 4096,
        env = "NXCC_ZENROOM_VM_MAX_CONF_BYTES"
    )]
    pub max_conf_bytes: usize,

    /// Allow non-zero debug settings in the Zenroom conf string.
    #[clap(
        long = "allow-debug-conf",
        default_value_t = false,
        env = "NXCC_ZENROOM_VM_ALLOW_DEBUG_CONF"
    )]
    pub allow_debug_conf: bool,

    /// Require HKDF when injecting secrets.
    #[clap(
        long = "require-kdf",
        default_value_t = false,
        env = "NXCC_ZENROOM_VM_REQUIRE_KDF"
    )]
    pub require_kdf: bool,
}

impl Default for ZenroomConfig {
    fn default() -> Self {
        #[derive(clap::Parser)]
        struct TempWrapper {
            #[clap(flatten)]
            zenroom: ZenroomConfig,
        }

        TempWrapper::try_parse_from([""]).unwrap().zenroom
    }
}

impl Default for Config {
    fn default() -> Self {
        use clap::Parser;
        Config::try_parse_from([""]).unwrap()
    }
}

impl Config {
    /// Load configuration from environment variables and CLI arguments using clap.
    pub fn load() -> Self {
        Config::parse()
    }
}
