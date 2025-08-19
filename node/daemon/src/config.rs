use std::path::PathBuf;

use clap::Parser;
use serde::{Deserialize, Serialize};

/// Main configuration struct, shared by file/env/CLI
#[derive(Parser, Debug, Clone, Serialize, Deserialize, Default)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    /// Path to a config file (TOML). Overridden by CLI args and env vars.
    /// This field is skipped in serialization so we don't rewrite the same info into the file.
    #[arg(short, long)]
    #[serde(skip)]
    pub config_path: Option<PathBuf>,

    /// If provided, load or create a keypair at this path. Otherwise use ephemeral mode.
    #[arg(long)]
    pub identity_path: Option<PathBuf>,

    /// Print the Peer ID derived from the identity and exit.
    #[arg(long, default_value_t = false)]
    pub print_peer_id: bool,

    /// Enable verbose logging
    #[arg(short, long, default_value_t = false)]
    pub verbose: bool,

    /// Directory to cache downloaded policies. Defaults to a system temp directory.
    #[arg(long)]
    pub policy_cache_dir: Option<PathBuf>,

    /// Network configuration
    #[serde(default)]
    #[clap(flatten)]
    pub network: NetworkConfig,

    #[serde(default)]
    #[clap(flatten)]
    pub grpc: GrpcConfig,

    #[serde(default)]
    #[clap(flatten)]
    pub enclave: EnclaveConfig,

    #[serde(default)]
    #[clap(flatten)]
    pub http: HttpConfig,

    #[serde(default)]
    #[clap(flatten)]
    pub scheduler: SchedulerConfig,

    #[serde(default)]
    #[clap(flatten)]
    pub attestation: AttestationConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, clap::Args)]
pub struct NetworkConfig {
    /// Comma-separated list of listen addresses.
    /// Defaults to "/ip4/0.0.0.0/tcp/0" (random free port).
    #[clap(
        long,
        value_delimiter = ',',
        default_value = "/ip4/0.0.0.0/tcp/0",
        help = "Listen addresses for the node"
    )]
    #[serde(default = "default_listen_addresses")]
    pub listen_addresses: Vec<String>,

    /// Comma-separated list of bootstrap peers.
    #[clap(
        long,
        value_delimiter = ',',
        help = "Bootstrap peers for the network (comma-separated)"
    )]
    #[serde(default)]
    pub bootstrap_peers: Vec<String>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            listen_addresses: default_listen_addresses(),
            bootstrap_peers: vec![],
        }
    }
}

fn default_listen_addresses() -> Vec<String> {
    vec!["/ip4/0.0.0.0/tcp/0".to_string()]
}

/// Configuration for the local gRPC interface.
#[derive(Debug, Clone, Serialize, Deserialize, clap::Args)]
pub struct GrpcConfig {
    /// Mode for the local gRPC interface: "vsock", "uds", or "tcp"
    #[clap(long, default_value = "uds")]
    #[serde(default = "default_grpc_mode")]
    pub mode: String,

    /// When using vsock: the vsock port to listen on
    #[clap(long, default_value_t = 50051)]
    #[serde(default = "default_vsock_port")]
    pub vsock_port: u32,

    /// When using vsock: the vsock CID (e.g. guest CID)
    #[clap(long, default_value_t = 3)]
    #[serde(default = "default_vsock_cid")]
    pub vsock_cid: u32,

    /// When using UDS: the path to the Unix Domain Socket
    #[clap(long, default_value = "/tmp/daemon_grpc.sock")]
    #[serde(default = "default_uds_path")]
    pub uds_path: String,

    /// When using TCP: the address to listen on (e.g., "0.0.0.0:50051")
    #[clap(long, default_value = "0.0.0.0:50051")]
    #[serde(default = "default_tcp_addr")]
    pub tcp_addr: String,
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            mode: default_grpc_mode(),
            vsock_port: default_vsock_port(),
            vsock_cid: default_vsock_cid(),
            uds_path: default_uds_path(),
            tcp_addr: default_tcp_addr(),
        }
    }
}

fn default_grpc_mode() -> String {
    "uds".to_string()
}

fn default_vsock_port() -> u32 {
    50051
}

fn default_vsock_cid() -> u32 {
    3
}

fn default_uds_path() -> String {
    "/tmp/daemon_grpc.sock".to_string()
}

fn default_tcp_addr() -> String {
    "0.0.0.0:50051".to_string()
}

/// Configuration related to the connected enclave and its associated VM.
#[derive(Debug, Clone, Serialize, Deserialize, clap::Args)]
pub struct EnclaveConfig {
    /// The UDS path for the main enclave gRPC service.
    #[clap(long, default_value = "/tmp/nxcc_enclave.sock")]
    #[serde(default = "default_enclave_uds_path")]
    pub enclave_uds_path: String, // Keep existing name for consistency

    /// The identifier for the VM instance attached to the enclave for policy execution.
    /// This ID is used in RunWorker requests to the enclave.
    #[clap(long, default_value = "nxcc/workerd")]
    #[serde(default = "default_default_vm_id")]
    pub default_vm_id: String,

    /// The UDS path for the VM gRPC service that the enclave should connect to.
    /// The daemon tells the enclave this path via AttachVm.
    #[clap(long, default_value = "/tmp/nxcc_workerd_vm.sock")]
    #[serde(default = "default_default_vm_uds_path")]
    pub default_vm_uds_path: String,
}

impl Default for EnclaveConfig {
    fn default() -> Self {
        Self {
            enclave_uds_path: default_enclave_uds_path(),
            default_vm_id: default_default_vm_id(),
            default_vm_uds_path: default_default_vm_uds_path(),
        }
    }
}

fn default_enclave_uds_path() -> String {
    "/tmp/nxcc_enclave.sock".to_string()
}
fn default_default_vm_id() -> String {
    "nxcc/workerd".to_string()
}
fn default_default_vm_uds_path() -> String {
    "/tmp/nxcc-workerd-vmm.sock".to_string()
}

/// Configuration for the daemon's HTTP server.
#[derive(Debug, Clone, Serialize, Deserialize, clap::Args)]
pub struct HttpConfig {
    /// The base path under which workers will be mounted.
    #[clap(long, default_value = "/w")]
    #[serde(default = "default_http_base_mount_path")]
    pub base_mount_path: String,

    /// The listen address for the HTTP server.
    #[clap(long, default_value = "127.0.0.1:6922")]
    #[serde(default = "default_http_listen_addr")]
    pub http_listen_addr: String,

    /// Enable the public HTTP API (e.g., for submitting work orders).
    #[clap(long, env = "NXCC_HTTP_API_ENABLED", default_value = "true")]
    #[serde(default = "default_api_enabled")]
    pub api_enabled: bool,

    /// Allowed origins for CORS on the public HTTP API.
    /// Use "*" for a wildcard. An empty list disables CORS.
    #[clap(
        long,
        value_delimiter = ',',
        env = "NXCC_HTTP_API_CORS_ALLOWED_ORIGINS"
    )]
    #[serde(default = "default_cors_allowed_origins")]
    pub api_cors_allowed_origins: Vec<String>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            base_mount_path: default_http_base_mount_path(),
            http_listen_addr: default_http_listen_addr(),
            api_enabled: true,
            api_cors_allowed_origins: default_cors_allowed_origins(),
        }
    }
}

fn default_api_enabled() -> bool {
    true
}

fn default_http_base_mount_path() -> String {
    "/w".to_string()
}
fn default_http_listen_addr() -> String {
    "127.0.0.1:6922".to_string()
}

fn default_cors_allowed_origins() -> Vec<String> {
    vec![]
}

/// Configuration for the daemon's scheduler service.
#[derive(Debug, Clone, Serialize, Deserialize, clap::Args)]
pub struct SchedulerConfig {
    /// Minimum allowed schedule interval in milliseconds.
    /// Work orders with scheduled events faster than this will be rejected.
    #[clap(long, default_value_t = 10)]
    #[serde(default = "default_min_schedule_interval_ms")]
    pub min_schedule_interval_ms: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            min_schedule_interval_ms: default_min_schedule_interval_ms(),
        }
    }
}

fn default_min_schedule_interval_ms() -> u64 {
    10
}

/// Configuration for attestation providers and verification
#[derive(Debug, Clone, Serialize, Deserialize, clap::Args)]
pub struct AttestationConfig {
    /// Enable TDX attestation support
    #[clap(long, default_value_t = true)]
    #[serde(default = "default_tdx_enabled")]
    pub tdx_enabled: bool,

    /// Google Cloud Project ID for GCS attestation verification
    #[clap(long, env = "NXCC_GCS_PROJECT_ID")]
    pub gcs_project_id: Option<String>,

    /// Google Cloud Service Account JSON key file path
    #[clap(long, env = "NXCC_GCS_SERVICE_ACCOUNT_KEY")]
    pub gcs_service_account_key: Option<PathBuf>,

    /// Prefer local verification over remote when available
    #[clap(long, default_value_t = true)]
    #[serde(default = "default_prefer_local_verification")]
    pub prefer_local_verification: bool,

    /// Maximum age of block hashes for freshness proofs (seconds)
    #[clap(long, default_value_t = 300)]
    #[serde(default = "default_max_block_age")]
    pub max_block_age: u64,

    /// Minimum number of chains required for freshness proof
    #[clap(long, default_value_t = 2)]
    #[serde(default = "default_min_chain_count")]
    pub min_chain_count: usize,

    /// Chain IDs to use for freshness proofs (comma-separated)
    #[clap(long, value_delimiter = ',')]
    #[serde(default = "default_freshness_chain_ids")]
    pub freshness_chain_ids: Vec<u64>,
}

impl Default for AttestationConfig {
    fn default() -> Self {
        Self {
            tdx_enabled: default_tdx_enabled(),
            gcs_project_id: None,
            gcs_service_account_key: None,
            prefer_local_verification: default_prefer_local_verification(),
            max_block_age: default_max_block_age(),
            min_chain_count: default_min_chain_count(),
            freshness_chain_ids: default_freshness_chain_ids(),
        }
    }
}

fn default_tdx_enabled() -> bool {
    true
}

fn default_prefer_local_verification() -> bool {
    true
}

fn default_max_block_age() -> u64 {
    300 // 5 minutes
}

fn default_min_chain_count() -> usize {
    2
}

fn default_freshness_chain_ids() -> Vec<u64> {
    vec![1, 137, 56, 10] // Ethereum, Polygon, BSC, Optimism
}

impl Config {
    /// Load from a combination of:
    /// 1. A default struct,
    /// 2. A TOML file (if found),
    /// 3. Environment variables (prefixed with `NXCC_`),
    /// 4. CLI arguments (parsed by `clap`).
    pub fn load() -> Result<Self, figment::Error> {
        use figment::{
            Figment,
            providers::{Env, Format, Serialized, Toml},
        };

        let cli = Config::parse();

        // Fall back to "config.toml" if `--config` was not provided
        let config_path = cli
            .config_path
            .clone()
            .unwrap_or_else(|| "config.toml".into());

        // Load config with figment doing automatic merging
        let config: Config = Figment::new()
            .merge(Serialized::defaults(Config::default()))
            .merge(Toml::file(config_path))
            .merge(Env::prefixed("NXCC_"))
            .merge(Serialized::defaults(cli)) // CLI args have highest priority
            .extract()?;

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr};

    use super::*;

    #[test]
    fn test_config_merging_nested_structures() {
        // Create a base config with some values
        let mut base_config = Config {
            verbose: false,
            identity_path: Some("/base/identity".into()),
            network: NetworkConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/tcp/8000".to_string()],
                bootstrap_peers: vec![],
                ..Default::default()
            },
            grpc: GrpcConfig {
                uds_path: "/base/grpc.sock".to_string(),
                tcp_addr: "127.0.0.1:9000".to_string(),
                ..Default::default()
            },
            enclave: EnclaveConfig {
                enclave_uds_path: "/base/enclave.sock".to_string(),
                default_vm_id: "base-vm".to_string(),
                ..Default::default()
            },
            http: HttpConfig {
                http_listen_addr: "127.0.0.1:8080".to_string(),
                api_enabled: false,
                api_cors_allowed_origins: vec!["http://localhost:3000".to_string()],
                ..Default::default()
            },
            scheduler: SchedulerConfig {
                min_schedule_interval_ms: 1000,
                ..Default::default()
            },
            attestation: AttestationConfig {
                tdx_enabled: false,
                gcs_project_id: Some("base-project".to_string()),
                prefer_local_verification: true,
                max_block_age: 100,
                min_chain_count: 2,
                freshness_chain_ids: vec![1, 2],
                ..Default::default()
            },
            ..Default::default()
        };

        // Create CLI args that should override some nested values
        let cli = Config {
            verbose: true,
            identity_path: Some("/cli/identity".into()),
            network: NetworkConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/tcp/9000".to_string()],
                bootstrap_peers: vec!["/ip4/127.0.0.1/tcp/8001".to_string()],
                ..Default::default()
            },
            grpc: GrpcConfig {
                uds_path: "/cli/grpc.sock".to_string(),
                tcp_addr: "127.0.0.1:9001".to_string(),
                ..Default::default()
            },
            enclave: EnclaveConfig {
                enclave_uds_path: "/base/enclave.sock".to_string(), // Explicitly set to preserve base value
                default_vm_id: "cli-vm".to_string(),
                ..Default::default()
            },
            http: HttpConfig {
                http_listen_addr: "127.0.0.1:8080".to_string(), // Explicitly preserve base value
                api_enabled: false, // Set to false (different from default true) to trigger override
                api_cors_allowed_origins: vec!["http://localhost:3000".to_string()], // Explicitly preserve base value
                ..Default::default()
            },
            scheduler: SchedulerConfig {
                min_schedule_interval_ms: 2000,
                ..Default::default()
            },
            attestation: AttestationConfig {
                tdx_enabled: false, // Set to false (different from default true) to trigger override
                gcs_project_id: Some("cli-project".to_string()),
                prefer_local_verification: true, // Explicitly preserve base value
                max_block_age: 200,
                min_chain_count: 2,              // Explicitly preserve base value
                freshness_chain_ids: vec![1, 2], // Explicitly preserve base value
                ..Default::default()
            },
            ..Default::default()
        };

        // Use figment to merge base config with CLI overrides (same as the actual implementation)
        use figment::{Figment, providers::Serialized};

        let merged_config: Config = Figment::new()
            .merge(Serialized::defaults(base_config))
            .merge(Serialized::defaults(cli))
            .extract()
            .unwrap();

        let base_config = merged_config;

        // Verify the expected merged results
        assert_eq!(base_config.verbose, true); // Overridden by CLI
        assert_eq!(
            base_config
                .identity_path
                .as_ref()
                .unwrap()
                .to_str()
                .unwrap(),
            "/cli/identity"
        ); // Overridden by CLI

        // Network: CLI values should override
        assert_eq!(
            base_config.network.listen_addresses,
            vec!["/ip4/127.0.0.1/tcp/9000"]
        );
        assert_eq!(
            base_config.network.bootstrap_peers,
            vec!["/ip4/127.0.0.1/tcp/8001"]
        );

        // gRPC: CLI values should override
        assert_eq!(base_config.grpc.uds_path, "/cli/grpc.sock");
        assert_eq!(base_config.grpc.tcp_addr, "127.0.0.1:9001");

        // Enclave: Only default_vm_id should be overridden, enclave_uds_path should remain from base
        assert_eq!(base_config.enclave.default_vm_id, "cli-vm");
        assert_eq!(base_config.enclave.enclave_uds_path, "/base/enclave.sock"); // Should remain from base

        // HTTP: Only api_enabled should be overridden
        assert_eq!(base_config.http.api_enabled, false);
        assert_eq!(base_config.http.http_listen_addr, "127.0.0.1:8080"); // Should remain from base
        assert_eq!(
            base_config.http.api_cors_allowed_origins,
            vec!["http://localhost:3000"]
        ); // Should remain from base

        // Scheduler: Should be overridden
        assert_eq!(base_config.scheduler.min_schedule_interval_ms, 2000);

        // Attestation: Some fields overridden, others preserved
        assert_eq!(base_config.attestation.tdx_enabled, false); // Overridden
        assert_eq!(
            base_config.attestation.gcs_project_id.as_ref().unwrap(),
            "cli-project"
        ); // Overridden
        assert_eq!(base_config.attestation.max_block_age, 200); // Overridden
        assert_eq!(base_config.attestation.prefer_local_verification, true); // Should remain from base
        assert_eq!(base_config.attestation.min_chain_count, 2); // Should remain from base
        assert_eq!(base_config.attestation.freshness_chain_ids, vec![1, 2]); // Should remain from base
    }
}
