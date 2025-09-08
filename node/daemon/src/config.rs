use std::path::PathBuf;

use clap::Parser;
use serde::{Deserialize, Serialize};

/// Main configuration struct for the NXCC daemon
#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    /// If provided, load or create a keypair at this path. Otherwise use ephemeral mode.
    #[arg(long, env = "NXCC_DAEMON_IDENTITY_PATH")]
    pub identity_path: Option<PathBuf>,

    /// Print the Peer ID derived from the identity and exit.
    #[arg(long, default_value_t = false, env = "NXCC_DAEMON_PRINT_PEER_ID")]
    pub print_peer_id: bool,

    /// Enable verbose logging
    #[arg(short, long, default_value_t = false, env = "NXCC_DAEMON_VERBOSE")]
    pub verbose: bool,

    /// Directory to cache downloaded policies. Defaults to a system temp directory.
    #[arg(long, env = "NXCC_DAEMON_POLICY_CACHE_DIR")]
    pub policy_cache_dir: Option<PathBuf>,

    /// Dump configuration as JSON and exit
    #[arg(long, default_value_t = false, env = "NXCC_DAEMON_DUMP_CONFIG")]
    #[serde(skip)]
    pub dump_config: bool,

    /// Network configuration
    #[clap(flatten)]
    pub network: NetworkConfig,

    #[clap(flatten)]
    pub grpc: GrpcConfig,

    #[clap(flatten)]
    pub enclave: EnclaveConfig,

    #[clap(flatten)]
    pub http: HttpConfig,

    #[clap(flatten)]
    pub scheduler: SchedulerConfig,

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
        env = "NXCC_DAEMON_LISTEN_ADDRESSES",
        help = "Listen addresses for the node"
    )]
    pub listen_addresses: Vec<String>,

    /// Comma-separated list of bootstrap peers.
    #[clap(
        long,
        value_delimiter = ',',
        env = "NXCC_DAEMON_BOOTSTRAP_PEERS",
        help = "Bootstrap peers for the network (comma-separated)"
    )]
    pub bootstrap_peers: Vec<String>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        // Create a temporary parser wrapper to get clap defaults
        #[derive(clap::Parser)]
        struct TempWrapper {
            #[clap(flatten)]
            network: NetworkConfig,
        }

        TempWrapper::try_parse_from(&[""]).unwrap().network
    }
}

/// Configuration for the local gRPC interface.
#[derive(Debug, Clone, Serialize, Deserialize, clap::Args)]
pub struct GrpcConfig {
    /// Mode for the local gRPC interface: "vsock", "uds", or "tcp"
    #[clap(long, default_value = "uds", env = "NXCC_DAEMON_MODE")]
    pub mode: String,

    /// When using vsock: the vsock port to listen on
    #[clap(long, default_value_t = 50051, env = "NXCC_DAEMON_VSOCK_PORT")]
    pub vsock_port: u32,

    /// When using vsock: the vsock CID (e.g. guest CID)
    #[clap(long, default_value_t = 3, env = "NXCC_DAEMON_VSOCK_CID")]
    pub vsock_cid: u32,

    /// When using UDS: the path to the Unix Domain Socket
    #[clap(
        long,
        default_value = "/tmp/nxcc/daemon.sock",
        env = "NXCC_DAEMON_UDS_PATH"
    )]
    pub uds_path: String,

    /// When using TCP: the address to listen on (e.g., "0.0.0.0:50051")
    #[clap(long, default_value = "127.0.0.1:50051", env = "NXCC_DAEMON_TCP_ADDR")]
    pub tcp_addr: String,
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

/// Configuration related to the connected enclave and its associated VM.
#[derive(Debug, Clone, Serialize, Deserialize, clap::Args)]
pub struct EnclaveConfig {
    /// The UDS path for the main enclave gRPC service.
    #[clap(
        long,
        default_value = "/tmp/nxcc/enclave.sock",
        env = "NXCC_DAEMON_ENCLAVE_UDS_PATH"
    )]
    pub enclave_uds_path: String, // Keep existing name for consistency

    /// The identifier for the VM instance attached to the enclave for policy execution.
    /// This ID is used in RunWorker requests to the enclave.
    #[clap(
        long,
        default_value = "nxcc/workerd",
        env = "NXCC_DAEMON_DEFAULT_VM_ID"
    )]
    pub default_vm_id: String,

    /// The UDS path for the VM gRPC service that the enclave should connect to.
    /// The daemon tells the enclave this path via AttachVm.
    #[clap(
        long,
        default_value = "/tmp/nxcc/workerd-vmm.sock",
        env = "NXCC_DAEMON_DEFAULT_VM_UDS_PATH"
    )]
    pub default_vm_uds_path: String,
}

impl Default for EnclaveConfig {
    fn default() -> Self {
        // Create a temporary parser wrapper to get clap defaults
        #[derive(clap::Parser)]
        struct TempWrapper {
            #[clap(flatten)]
            enclave: EnclaveConfig,
        }

        TempWrapper::try_parse_from(&[""]).unwrap().enclave
    }
}

/// Configuration for the daemon's HTTP server.
#[derive(Debug, Clone, Serialize, Deserialize, clap::Args)]
pub struct HttpConfig {
    /// The base path under which workers will be mounted.
    #[clap(long, default_value = "/w", env = "NXCC_DAEMON_BASE_MOUNT_PATH")]
    pub base_mount_path: String,

    /// The listen address for the HTTP server.
    #[clap(
        long,
        default_value = "127.0.0.1:6922",
        env = "NXCC_DAEMON_HTTP_LISTEN_ADDR"
    )]
    pub http_listen_addr: String,

    /// Enable the public HTTP API (e.g., for submitting work orders).
    #[clap(long, env = "NXCC_DAEMON_API_ENABLED", default_value = "true")]
    pub api_enabled: bool,

    /// Allowed origins for CORS on the public HTTP API.
    /// Use "*" for a wildcard. An empty list disables CORS.
    #[clap(
        long,
        value_delimiter = ',',
        env = "NXCC_DAEMON_API_CORS_ALLOWED_ORIGINS",
        default_value = "*"
    )]
    pub api_cors_allowed_origins: Vec<String>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        // Create a temporary parser wrapper to get clap defaults
        #[derive(clap::Parser)]
        struct TempWrapper {
            #[clap(flatten)]
            http: HttpConfig,
        }

        TempWrapper::try_parse_from(&[""]).unwrap().http
    }
}

/// Configuration for the daemon's scheduler service.
#[derive(Debug, Clone, Serialize, Deserialize, clap::Args)]
pub struct SchedulerConfig {
    /// Minimum allowed schedule interval in milliseconds.
    /// Work orders with scheduled events faster than this will be rejected.
    #[clap(
        long,
        default_value_t = 10,
        env = "NXCC_DAEMON_MIN_SCHEDULE_INTERVAL_MS"
    )]
    pub min_schedule_interval_ms: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        // Create a temporary parser wrapper to get clap defaults
        #[derive(clap::Parser)]
        struct TempWrapper {
            #[clap(flatten)]
            scheduler: SchedulerConfig,
        }

        TempWrapper::try_parse_from(&[""]).unwrap().scheduler
    }
}

/// Configuration for attestation providers and verification
#[derive(Debug, Clone, Serialize, Deserialize, clap::Args)]
pub struct AttestationConfig {
    /// Enable TDX attestation support
    #[clap(long, default_value_t = true, env = "NXCC_DAEMON_TDX_ENABLED")]
    pub tdx_enabled: bool,

    /// Google Cloud Project ID for GCS attestation verification
    #[clap(long, env = "NXCC_DAEMON_GCS_PROJECT_ID")]
    pub gcs_project_id: Option<String>,

    /// Google Cloud Service Account JSON key file path
    #[clap(long, env = "NXCC_DAEMON_GCS_SERVICE_ACCOUNT_KEY")]
    pub gcs_service_account_key: Option<PathBuf>,

    /// Prefer local verification over remote when available
    #[clap(
        long,
        default_value_t = true,
        env = "NXCC_DAEMON_PREFER_LOCAL_VERIFICATION"
    )]
    pub prefer_local_verification: bool,

    /// Maximum age of block hashes for freshness proofs (seconds)
    #[clap(long, default_value_t = 300, env = "NXCC_DAEMON_MAX_BLOCK_AGE")]
    pub max_block_age: u64,

    /// Minimum number of chains required for freshness proof
    #[clap(long, default_value_t = 2, env = "NXCC_DAEMON_MIN_CHAIN_COUNT")]
    pub min_chain_count: usize,

    /// Chain IDs to use for freshness proofs (comma-separated)
    #[clap(long, value_delimiter = ',', env = "NXCC_DAEMON_FRESHNESS_CHAIN_IDS", default_values_t = [1, 137, 56, 10])]
    pub freshness_chain_ids: Vec<u64>,

    /// Path to the operator signing key file (Ed25519 private key)
    /// If not provided, operator signatures will not be included in attestations
    /// The public key will be automatically derived from the private key
    #[clap(long, env = "NXCC_DAEMON_OPERATOR_SIGNING_KEY_PATH")]
    pub operator_signing_key_path: Option<PathBuf>,
}

impl Default for AttestationConfig {
    fn default() -> Self {
        // Create a temporary parser wrapper to get clap defaults
        #[derive(clap::Parser)]
        struct TempWrapper {
            #[clap(flatten)]
            attestation: AttestationConfig,
        }

        TempWrapper::try_parse_from(&[""]).unwrap().attestation
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
    /// Environment variables use the NXCC_DAEMON_ prefix.
    pub fn load() -> Self {
        Config::parse()
    }
}
