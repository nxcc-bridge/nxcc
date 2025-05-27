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
    /// Mode for the local gRPC interface: "vsock" or "uds"
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
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            mode: default_grpc_mode(),
            vsock_port: default_vsock_port(),
            vsock_cid: default_vsock_cid(),
            uds_path: default_uds_path(),
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

/// Configuration related to the connected enclave and its associated VM.
#[derive(Debug, Clone, Serialize, Deserialize, clap::Args)]
pub struct EnclaveConfig {
    /// The UDS path for the main enclave gRPC service.
    #[clap(long, default_value = "/tmp/enclave_grpc.sock")]
    #[serde(default = "default_enclave_uds_path")]
    pub enclave_uds_path: String, // Keep existing name for consistency

    /// The identifier for the VM instance attached to the enclave for policy execution.
    /// This ID is used in RunWorker requests to the enclave.
    #[clap(long, default_value = "nxcc/workerd")]
    #[serde(default = "default_default_vm_id")]
    pub default_vm_id: String,

    /// The UDS path for the VM gRPC service that the enclave should connect to.
    /// The daemon tells the enclave this path via AttachVm.
    #[clap(long, default_value = "/tmp/nxcc-workerd-vmm.sock")]
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
    "/tmp/enclave_grpc.sock".to_string()
}
fn default_default_vm_id() -> String {
    "nxcc/workerd".to_string()
}
fn default_default_vm_uds_path() -> String {
    "/tmp/nxcc-workerd-vmm.sock".to_string()
}

/// Configuration for the daemon's HTTP listener for workers.
#[derive(Debug, Clone, Serialize, Deserialize, clap::Args)]
pub struct HttpConfig {
    /// The base path under which workers will be mounted.
    #[clap(long, default_value = "/w")]
    #[serde(default = "default_http_base_mount_path")]
    pub base_mount_path: String,

    /// The listen address for the HTTP server
    #[clap(long, default_value = "0.0.0.0:6922")]
    #[serde(default = "default_http_listen_addr")]
    pub listen_addr: String,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            base_mount_path: default_http_base_mount_path(),
            listen_addr: default_http_listen_addr(),
        }
    }
}

fn default_http_base_mount_path() -> String {
    "/w".to_string()
}
fn default_http_listen_addr() -> String {
    "0.0.0.0:6922".to_string()
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

        Figment::new()
            .merge(Serialized::defaults(Config::default()))
            .merge(Toml::file(config_path))
            .merge(Env::prefixed("NXCC_"))
            .merge(Serialized::defaults(cli))
            .extract()
    }
}
