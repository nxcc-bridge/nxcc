#![cfg(test)]

use std::{
    collections::HashMap,
    hash::{Hash, Hasher}, // Added Hasher
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use alloy_primitives::{Address, U256};
use futures::{StreamExt, channel::mpsc};
use hyper_util::rt::TokioIo; // Added
use libp2p::{
    PeerId,
    Transport,
    core::{
        multiaddr::{Multiaddr, Protocol},
        transport::{Boxed, MemoryTransport}, // Added Boxed
        upgrade,                             // Added
    },
    identity,
    noise,                                        // Added noise
    swarm::{NetworkBehaviour, Swarm, SwarmEvent}, // Added SwarmEvent
    yamux,                                        // Added yamux
};
use nxcc_interface::{
    policy::{PolicyBundle, PolicyManifest},
    proto::{
        daemon::{GetSecretsRequest, secrets_client::SecretsClient as DaemonSecretsClient},
        enclave::{
            AttachVmRequest, ExecutePolicyRequest as ProtoExecutePolicyRequest,
            GenerateSecretsRequest, GetReportRequest,
            GetSecretsRequest as EnclaveGetSecretsRequest, PutSecretsRequest, RunWorkerRequest,
            SecretRequest as EnclaveSecretRequest, SecretsBundle, TerminateWorkerRequest,
            runner_server::Runner as _, secrets_server::Secrets as _,
        },
        interface::{EnvReport as ProtoEnvReport, SecretIdentifier, SecretRequest},
        vm::{TrustedConfig, UntrustedConfig, WorkerStatus}, // Added vm types
    },
    types::{
        AttestationReport, ConsumerInfo, EnvReport, FromProto as _, IntoProto as _,
        PolicyExecutionReport, PolicyExecutionRequest, SecretId, SecretsBox, VmAddress,
    },
};
// Use the crate name directly from Cargo.toml dev-dependencies
use nxcc_platform_enclave::{
    config::EnclaveConfig as PlatformEnclaveConfig, grpc::EnclaveRunnerGrpcService,
    grpc::SecretsGrpcService as EnclaveSecretsGrpcService, runner::RunnerService as EnclaveRunner,
    secrets::Secrets as EnclaveSecrets,
};
use nxcc_vm_base::{
    client::{
        VmClient as _,
        mock::{MockExecutionBehavior, MockVmServiceClient},
    },
    server::{ServerConfig as VmServerConfig, VmError, VmRuntime}, // Added VmError, VmRuntime
};
// Use the crate name directly from Cargo.toml dev-dependencies
// use nxcc_workerd_vm::vmm::WorkerdVmm; // Not needed if VM is mocked
use tempfile::TempDir;
use tokio::{
    net::UnixListener, // Removed TcpListener, duplex, AsyncReadExt, AsyncWriteExt
    sync::{Mutex, broadcast},
    time::timeout,
};
use tonic::transport::{Channel, Endpoint, Server as TonicServer, Uri};
use tower::service_fn;
use tracing::{debug, error, info, trace}; // Added trace
use tracing_test::traced_test;

use crate::{
    config::{Config, EnclaveConfig, GrpcConfig, NetworkConfig},
    grpc::{
        enclave_client::EnclaveClient,
        secrets::SecretsDebugGrpc, // Import the gRPC service implementation
    },
    identity::create_ephemeral_identity,
    network::{AppBehaviour, AppEvent, NetworkManager}, // Removed SecretsMessage
    policy::PolicyManager,
    services::{runner::RunnerService, secrets::SecretsService},
    web3::gateways::GatewayManager,
};

// --- Constants ---
const TEST_TIMEOUT: Duration = Duration::from_secs(120); // Generous timeout for CI
const P2P_TIMEOUT_BUFFER: Duration = Duration::from_secs(5); // Buffer beyond daemon's P2P timeout
const MOCK_POLICY_URL: &str = "mock://policy.example.com/test-policy";

// --- Helper Structs ---

struct TestNode {
    config: Config,
    _temp_dir: TempDir,
    local_key: identity::Keypair,
    peer_id: PeerId,
    enclave_client: EnclaveClient,
    daemon_grpc_client: DaemonSecretsClient<Channel>,
    // NetworkManager is now managed internally by its task
    // network_manager: Arc<Mutex<NetworkManager>>,
    secrets_service: Arc<SecretsService>,
    runner_service: Arc<RunnerService>,
    policy_manager: Arc<PolicyManager>,
    mock_vm_client: MockVmServiceClient, // Keep handle to configure VM mock
    shutdown_tx: broadcast::Sender<()>,
    // Handles to background tasks
    _enclave_task: tokio::task::JoinHandle<()>,
    _vm_task: tokio::task::JoinHandle<()>,
    _daemon_grpc_task: tokio::task::JoinHandle<()>,
    _network_task: tokio::task::JoinHandle<()>,
}

impl Drop for TestNode {
    fn drop(&mut self) {
        info!("Shutting down test node for peer {}", self.peer_id);
        let _ = self.shutdown_tx.send(());
        // Dropping the TempDir handles cleanup
    }
}

// --- Mock Implementations ---

/// Mock VmRuntime for the VM Server side of the test setup
#[derive(Default)]
struct MockVmRuntime;

#[tonic::async_trait]
impl VmRuntime for MockVmRuntime {
    async fn start_worker(
        &self,
        _worker_code: Vec<u8>,
        _untrusted_config: UntrustedConfig,
        _trusted_config: TrustedConfig,
    ) -> Result<String, VmError> {
        // The mock client attached to the enclave runner handles the actual mock logic.
        // This server-side mock just needs to acknowledge.
        Ok("mock-instance-id-from-server".to_string())
    }

    async fn stop_worker(&self, _id: String) -> Result<(), VmError> {
        Ok(())
    }

    async fn invoke_worker(&self, _id: String, payload: Vec<u8>) -> Result<Vec<u8>, VmError> {
        // This shouldn't really be called if the enclave runner uses the mock client directly.
        // If it were called, echo behavior might be useful.
        Ok(payload)
    }

    async fn get_attestation(&self, user_data: Vec<u8>) -> Result<AttestationReport, VmError> {
        Ok(AttestationReport {
            ephemeral_public_key: vec![0u8; 32], // Dummy key
            block_hashes: vec![],
            user_data,
        })
    }

    async fn get_worker_status(&self, _id: String) -> Result<WorkerStatus, VmError> {
        Ok(WorkerStatus::Running) // Assume running
    }

    async fn list_running_workers(&self) -> Result<Vec<String>, VmError> {
        Ok(vec![])
    }

    async fn get_worker_logs(&self, _id: String) -> Result<String, VmError> {
        Ok("Mock VM logs".to_string())
    }
}

// --- Helper Functions ---

fn test_secret_id(id_num: u64) -> SecretId {
    SecretId {
        chain_id: 0, // Use 0 for mock chain
        identity_address: Address::random(),
        identity_id: U256::from(id_num),
    }
}

// Creates a simple JS policy worker that approves any request
fn mock_policy_bundle() -> PolicyBundle {
    let manifest = PolicyManifest {
        version: "1.0".to_string(),
        name: "Mock Allow All Policy".to_string(),
        description: "Approves any request".to_string(),
        allowed_consumers: vec![],
        execution_constraints: nxcc_interface::policy::ExecutionConstraints {
            max_memory_mb: 128,
            max_execution_time_ms: 1000,
            allowed_network_calls: false,
        },
    };
    // JS worker that returns `[true]` for any input array of contexts
    let executable = r#"
        export default {
            async fetch(request) {
                // Assume input is CBOR list of contexts, return CBOR list of booleans
                // For mock, always return true for the first context
                // Simple CBOR for [true]: 0x81 0xf5
                return new Response(new Uint8Array([0x81, 0xf5]), {
                    headers: { 'Content-Type': 'application/cbor' }
                });
            }
        }
    "#
    .as_bytes()
    .to_vec();
    PolicyBundle {
        manifest,
        executable,
    }
}

// Mock GatewayManager that returns a fixed policy URL
#[derive(Clone)]
struct MockGatewayManager;

impl MockGatewayManager {
    fn new() -> GatewayManager {
        // We don't actually need the Arc/RwLock state for the mock
        GatewayManager::new()
    }

    async fn get_policy_url(
        &self,
        _chain_id: u64,
        _identity_address: Address,
        _identity_id: U256,
    ) -> Result<String, crate::error::AppError> {
        Ok(MOCK_POLICY_URL.to_string())
    }
}

// Override the GatewayManager's get_policy_url for PolicyManager testing
// This requires modifying PolicyManager slightly or using a trait object,
// but for simplicity here, we'll rely on the mock URL prefix handling.
// A better approach would be dependency injection for the gateway fetcher.

async fn setup_test_node(name: &str) -> TestNode {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let base_path = temp_dir.path();

    let local_key = create_ephemeral_identity();
    let peer_id = local_key.public().to_peer_id();
    info!("Setting up node '{}' with PeerId: {}", name, peer_id);

    // --- Configure UDS paths ---
    let enclave_uds_path = base_path.join(format!("{}_enclave.sock", name));
    let vm_uds_path = base_path.join(format!("{}_vm.sock", name));
    let daemon_grpc_uds_path = base_path.join(format!("{}_daemon_grpc.sock", name));

    // --- Start Mock VM Server ---
    // We still need a VM *server* for the enclave runner to connect to,
    // even though the enclave runner uses a mock *client* internally for tests.
    // The server uses the MockVmRuntime.
    let mock_vm_runtime = Arc::new(MockVmRuntime);
    let vm_config = VmServerConfig::Uds {
        path: vm_uds_path.to_str().unwrap().to_string(),
    };
    let (vm_shutdown_tx, mut vm_shutdown_rx) = broadcast::channel(1);
    let vm_task = tokio::spawn(async move {
        let certs = nxcc_vm_base::tls::MtlsCertificates::new().unwrap();
        let tls_config = certs.server_tls_config().unwrap();
        tokio::select! {
            // Use the actual run_vm_server from vm_base, but with MockVmRuntime
            res = nxcc_vm_base::server::run_vm_server(vm_config, mock_vm_runtime, tls_config) => {
                 if let Err(e) = res {
                    error!("Mock VM server failed: {}", e);
                }
            },
            _ = vm_shutdown_rx.recv() => {
                info!("Mock VM server shutting down.");
            }
        }
    });
    // Wait briefly for VM server to bind UDS
    sleep_ms(1000).await; // Use renamed helper

    // --- Start Enclave Server ---
    let enclave_config = PlatformEnclaveConfig {
        config_path: None,
        verbose: true,
        grpc: nxcc_platform_enclave::config::GrpcConfig {
            // Use the enclave's GrpcConfig equivalent
            mode: "uds".to_string(),
            uds_path: enclave_uds_path.to_str().unwrap().to_string(),
            ..Default::default()
        },
    };
    let enclave_secrets_service = EnclaveSecrets::new();
    let enclave_runner_service = Arc::new(EnclaveRunner::new(enclave_secrets_service.clone()));
    // Create the mock VM *client* handle
    let mock_vm_client = MockVmServiceClient::new();
    // Attach the *mock* VM client to the *real* enclave runner service
    enclave_runner_service
        .attach_mock_client("policy-vm-0".to_string(), mock_vm_client.clone())
        .await;

    let enclave_secrets_grpc = EnclaveSecretsGrpcService::new(enclave_secrets_service.clone());
    let enclave_runner_grpc = EnclaveRunnerGrpcService::new(enclave_runner_service.clone());
    let (enclave_shutdown_tx, mut enclave_shutdown_rx) = broadcast::channel(1);
    let enclave_uds_path_clone = enclave_uds_path.clone();
    let enclave_task = tokio::spawn(async move {
        let listener = UnixListener::bind(&enclave_uds_path_clone)
            .expect("Failed to bind enclave UDS listener");
        let incoming = tokio_stream::wrappers::UnixListenerStream::new(listener);
        info!(
            "Starting in-memory enclave gRPC server at {}",
            enclave_uds_path_clone.display()
        );
        TonicServer::builder()
            .add_service(
                nxcc_interface::proto::enclave::secrets_server::SecretsServer::new(
                    enclave_secrets_grpc,
                ),
            )
            .add_service(
                nxcc_interface::proto::enclave::runner_server::RunnerServer::new(
                    enclave_runner_grpc,
                ),
            )
            .serve_with_incoming_shutdown(incoming, async {
                let _ = enclave_shutdown_rx.recv().await;
                info!("In-memory enclave gRPC server shutting down.");
            })
            .await
            .expect("Enclave server failed");
        let _ = std::fs::remove_file(&enclave_uds_path_clone);
    });
    // Wait briefly for enclave server to bind UDS
    sleep_ms(1000).await; // Use renamed helper

    // --- Configure Daemon ---
    let config = Config {
        config: None,
        identity_path: None, // Use ephemeral
        verbose: true,
        policy_cache_dir: Some(base_path.join("policy_cache")),
        network: NetworkConfig {
            listen_addresses: vec!["/memory/0".to_string()], // Use memory transport
            bootstrap_peers: vec![],
        },
        grpc: GrpcConfig {
            mode: "uds".to_string(),
            uds_path: daemon_grpc_uds_path.to_str().unwrap().to_string(),
            ..Default::default()
        },
        enclave: EnclaveConfig {
            enclave_uds_path: enclave_uds_path.to_str().unwrap().to_string(),
            policy_vm_id: "policy-vm-0".to_string(), // Matches the ID used in attach_mock_client
            policy_vm_uds_path: vm_uds_path.to_str().unwrap().to_string(),
        },
    };

    // Create EnclaveClient connected to the in-memory enclave server
    let enclave_client = EnclaveClient::connect_uds(config.enclave.enclave_uds_path.clone())
        .await
        .expect("Failed to connect EnclaveClient");

    // --- Setup Daemon Services ---
    let gateway_manager = MockGatewayManager::new(); // Use the mock gateway
    let policy_manager = Arc::new(
        PolicyManager::new(gateway_manager, &config)
            .await
            .expect("Failed to create PolicyManager"),
    );
    let runner_service = Arc::new(RunnerService::new(
        enclave_client.runner(),
        config.enclave.clone(),
    ));

    let (secrets_p2p_tx, secrets_p2p_rx) = mpsc::channel(64);
    let (_notifier_tx, notifier_rx) = mpsc::channel(64); // Not used in this test

    let secrets_service = SecretsService::new(
        secrets_p2p_tx.clone(),
        enclave_client.clone(),
        policy_manager.clone(),
        runner_service.clone(),
    );

    // --- Start Daemon gRPC Server ---
    let (daemon_grpc_shutdown_tx, mut daemon_grpc_shutdown_rx) = broadcast::channel(1);
    let daemon_grpc_uds_path_clone = daemon_grpc_uds_path.clone();
    let secrets_service_clone = secrets_service.clone();
    let enclave_client_clone = enclave_client.clone();
    let daemon_grpc_task = tokio::spawn(async move {
        let listener = UnixListener::bind(&daemon_grpc_uds_path_clone)
            .expect("Failed to bind daemon gRPC UDS listener");
        let incoming = tokio_stream::wrappers::UnixListenerStream::new(listener);
        info!(
            "Starting in-memory daemon gRPC server at {}",
            daemon_grpc_uds_path_clone.display()
        );
        TonicServer::builder()
            .add_service(
                nxcc_interface::proto::daemon::secrets_server::SecretsServer::new(
                    SecretsDebugGrpc::new(secrets_service_clone, enclave_client_clone),
                ),
            )
            .serve_with_incoming_shutdown(incoming, async {
                let _ = daemon_grpc_shutdown_rx.recv().await;
                info!("In-memory daemon gRPC server shutting down.");
            })
            .await
            .expect("Daemon gRPC server failed");
        let _ = std::fs::remove_file(&daemon_grpc_uds_path_clone);
    });
    // Wait briefly for daemon gRPC server to bind UDS
    sleep_ms(1000).await; // Use renamed helper

    // --- Create Daemon gRPC Client ---
    let daemon_grpc_client = {
        let uds_path = config.grpc.uds_path.clone();
        let channel = Endpoint::try_from("http://[::]:50051") // Dummy URI
            .unwrap()
            .connect_with_connector(service_fn(move |_: Uri| {
                let path = uds_path.clone();
                async move {
                    let stream = tokio::net::UnixStream::connect(path).await?;
                    Ok::<_, std::io::Error>(TokioIo::new(stream))
                }
            }))
            .await
            .expect("Failed to connect daemon gRPC client");
        DaemonSecretsClient::new(channel)
    };

    // --- Setup Network Manager ---
    // NetworkManager holds its own Arc<SecretsService>
    let network_manager_secrets_service = SecretsService::new(
        secrets_p2p_tx.clone(), // Need another sender for the NM's service instance
        enclave_client.clone(),
        policy_manager.clone(),
        runner_service.clone(),
    );
    let mut network_manager = NetworkManager::new(
        local_key.clone(),
        config.clone(),
        network_manager_secrets_service, // Pass the NM's service instance
        notifier_rx,                     // Consumed
        secrets_p2p_rx,                  // Consumed
    )
    .await
    .expect("Failed to create NetworkManager");

    // --- Start Network Loop ---
    let (network_shutdown_tx, network_shutdown_rx) = broadcast::channel(1);
    let network_task = tokio::spawn(async move {
        // network_manager is moved into the task here
        if let Err(e) = network_manager.start_memory(network_shutdown_rx).await {
            error!("NetworkManager failed to start: {}", e);
        }
    });

    // --- Combine Shutdown Signals ---
    let shutdown_tx = broadcast::channel(1).0;
    let s_tx1 = shutdown_tx.clone();
    let s_tx2 = shutdown_tx.clone();
    let s_tx3 = shutdown_tx.clone();
    let s_tx4 = shutdown_tx.clone();
    tokio::spawn(async move {
        let _ = s_tx1.subscribe().recv().await;
        let _ = vm_shutdown_tx.send(());
    });
    tokio::spawn(async move {
        let _ = s_tx2.subscribe().recv().await;
        let _ = enclave_shutdown_tx.send(());
    });
    tokio::spawn(async move {
        let _ = s_tx3.subscribe().recv().await;
        let _ = daemon_grpc_shutdown_tx.send(());
    });
    tokio::spawn(async move {
        let _ = s_tx4.subscribe().recv().await;
        let _ = network_shutdown_tx.send(());
    });

    TestNode {
        config,
        _temp_dir: temp_dir,
        local_key,
        peer_id,
        enclave_client,
        daemon_grpc_client,
        // network_manager, // Removed as it's moved into task
        secrets_service, // Keep the original handle for direct interaction if needed
        runner_service,
        policy_manager,
        mock_vm_client,
        shutdown_tx,
        _enclave_task: enclave_task,
        _vm_task: vm_task,
        _daemon_grpc_task: daemon_grpc_task,
        _network_task: network_task,
    }
}

// Add a method to NetworkManager to start with MemoryTransport
impl NetworkManager {
    pub async fn start_memory(
        &mut self,
        mut shutdown: broadcast::Receiver<()>,
    ) -> Result<(), crate::error::AppError> {
        let peer_id = self.local_key.public().to_peer_id();

        // Build the swarm with MemoryTransport
        let swarm = self.build_swarm_memory(peer_id)?; // Call the method

        // Listen on memory transport
        let listen_addr: Multiaddr = "/memory/0".parse().unwrap();
        let mut swarm = swarm; // Shadow to make mutable
        swarm.listen_on(listen_addr)?;

        // Extract the actual listening address
        let mut listening = false;
        while !listening {
            // Use select to also check for shutdown signal while waiting
            tokio::select! {
                event = swarm.select_next_some() => {
                    if let SwarmEvent::NewListenAddr { address, .. } = event {
                        info!("Node {} listening on {}", peer_id, address);
                        listening = true;
                    } else {
                        // Handle other events if necessary, or ignore
                        trace!("Node {} ignoring event: {:?}", peer_id, event);
                    }
                },
                _ = shutdown.recv() => {
                     info!("Shutdown received while waiting for listener for peer {}", peer_id);
                     return Ok(()); // Exit gracefully if shutdown occurs
                }
            }
        }

        let secrets_service = Arc::clone(&self.secrets_service);
        let notifier_rx = std::mem::replace(&mut self.notifier_receiver, mpsc::channel(1).1);
        let secrets_rx = std::mem::replace(&mut self.secrets_receiver, mpsc::channel(1).1);
        let secrets_topic = libp2p::gossipsub::IdentTopic::new("secrets");

        // Run swarm loop
        crate::network::run_network_loop(
            swarm, // Pass the swarm
            notifier_rx,
            secrets_rx,
            secrets_service,
            secrets_topic,
            shutdown, // Pass the receiver
        )
        .await;

        Ok(())
    }

    // Make this a method on NetworkManager
    fn build_swarm_memory(
        &self, // Add &self
        peer_id: PeerId,
    ) -> Result<Swarm<AppBehaviour>, crate::error::AppError> {
        use libp2p::{gossipsub, identify, kad, mdns, ping}; // Removed noise, yamux as they are part of stack

        // Build the transport stack for memory
        let transport = libp2p::core::transport::MemoryTransport::default()
            .upgrade(libp2p::core::upgrade::Version::V1Lazy)
            .authenticate(libp2p::noise::Config::new(&self.local_key)?) // Use self.local_key
            .multiplex(libp2p::yamux::Config::default())
            .boxed();

        let message_id_fn = |message: &gossipsub::Message| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            message.data.hash(&mut hasher); // Requires Hash trait in scope
            gossipsub::MessageId::from(hasher.finish().to_string()) // Requires Hasher trait in scope
        };
        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(1)) // Faster heartbeat for tests
            .validation_mode(gossipsub::ValidationMode::Strict)
            .message_id_fn(message_id_fn)
            .build()
            .map_err(|e| crate::error::AppError::Network(format!("Gossipsub config: {e}")))?;
        let ping_behavior = ping::Behaviour::new(ping::Config::new());
        let identify_config = identify::Config::new("/p2p/1.0.0".into(), self.local_key.public()); // Use self.local_key
        let identify_behavior = identify::Behaviour::new(identify_config);
        let store = kad::store::MemoryStore::new(peer_id);
        let mut kad_config = kad::Config::default();
        kad_config.set_query_timeout(Duration::from_secs(5)); // Shorter timeout for tests
        let kad_behavior = kad::Behaviour::with_config(peer_id, store, kad_config);

        let mut behaviour = AppBehaviour {
            kad: kad_behavior,
            identify: identify_behavior,
            mdns: mdns::tokio::Behaviour::new(mdns::Config::default(), peer_id).unwrap(), // Keep field, but won't discover
            gossipsub: gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(self.local_key.clone()), // Use self.local_key
                gossipsub_config,
            )
            .expect("Gossipsub init error"),
            ping: ping_behavior,
        };
        behaviour.kad.set_mode(Some(kad::Mode::Server));
        let secrets_topic = gossipsub::IdentTopic::new("secrets");
        behaviour.gossipsub.subscribe(&secrets_topic)?;

        let swarm = libp2p::SwarmBuilder::with_existing_identity(self.local_key.clone()) // Use self.local_key
            .with_tokio()
            .with_other_transport(|_| transport) // Use the built memory transport stack
            .expect("Memory transport build failed") // Use expect here as it shouldn't fail
            .with_behaviour(|_| behaviour)
            .expect("Valid behaviour")
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(10))) // Shorter idle timeout
            .build();
        Ok(swarm)
    }
}

// Removed connect_peers helper
// Removed swarm_mut helper

// Helper to create a GetSecrets request for the daemon gRPC
fn create_daemon_get_secrets_request(
    secret_id: &SecretId,
    node_id: &str,      // Node ID of the requester
    _kx_pub_key: &[u8], // Requester's ephemeral KX public key (no longer using x25519)
) -> GetSecretsRequest {
    // Generate random bytes for the ephemeral key for the mock request
    let mut ephemeral_key_bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut ephemeral_key_bytes);

    let env_report = ProtoEnvReport {
        attestation: Some(nxcc_interface::proto::interface::AttestationReport {
            ephemeral_public_key: ephemeral_key_bytes.to_vec(), // Use random bytes
            block_hashes: vec![vec![1]],                        // Dummy data
            user_data: vec![0u8; 32],                           // Dummy data
        }),
        operator_signature: vec![2; 64], // Dummy data
        node_id: node_id.to_string(),
    };
    GetSecretsRequest {
        secret_requests: vec![SecretRequest {
            secret_id: Some(secret_id.to_proto()),
            consumer: Some(nxcc_interface::proto::interface::ConsumerInfo {
                code_hash: vec![3; 32], // Dummy data
                signature: vec![4; 64], // Dummy data
            }),
        }],
        env_report: Some(env_report),
    }
}

// Helper to check enclave secrets via its client
async fn check_enclave_secret(
    enclave_client: &EnclaveClient,
    secret_id: &SecretId,
) -> Result<bool, String> {
    let statuses = enclave_client
        .check_secrets(vec![secret_id.clone()])
        .await?;
    Ok(statuses.first().is_some_and(|s| s.1)) // Check 'found' status
}

// Renamed helper sleep function
async fn sleep_ms(duration_ms: u64) {
    tokio::time::sleep(Duration::from_millis(duration_ms)).await;
}

// --- The Integration Test ---

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[traced_test]
async fn test_daemon_secret_generation_and_sharing_workflow() {
    timeout(TEST_TIMEOUT, async {
        // --- 0. Setup ---
        info!("Setting up Alice node...");
        let alice = setup_test_node("alice").await;
        info!("Setting up Bob node...");
        let bob = setup_test_node("bob").await;

        // Configure Alice's mock VM to succeed policy execution
        let _policy_bundle = mock_policy_bundle(); // Keep for potential future use
        alice.mock_vm_client.set_default_execution_behavior(
            MockExecutionBehavior::Fixed(vec![0x81, 0xf5]), // CBOR for [true]
        );
        // Configure Bob's mock VM similarly
        bob.mock_vm_client.set_default_execution_behavior(
            MockExecutionBehavior::Fixed(vec![0x81, 0xf5]), // CBOR for [true]
        );

        // Peers should connect implicitly via MemoryTransport gossip/kad

        // Define the secret ID
        let secret_id = test_secret_id(999);
        info!("Testing with Secret ID: {:?}", secret_id);

        // --- 1. Alice receives GetSecrets request for non-existent secret ---
        info!("Step 1: Alice receives GetSecrets request");
        // Generate random bytes for the mock requester key
        let mut alice_requester_key_bytes = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut alice_requester_key_bytes);
        let get_secrets_req_alice = create_daemon_get_secrets_request(
            &secret_id,
            "external_requester_alice",
            &alice_requester_key_bytes,
        );

        // Verify Alice doesn't have the secret initially
        assert!(
            !check_enclave_secret(&alice.enclave_client, &secret_id)
                .await
                .unwrap(),
            "Alice should not have the secret initially"
        );

        // --- 2. Alice asks Bob (P2P), Bob doesn't have it ---
        // --- 3. Alice generates the secret ---
        info!("Step 2 & 3: Alice requests from Bob (fails), generates secret");
        let mut alice_client = alice.daemon_grpc_client.clone();
        let alice_get_secrets_future = alice_client.get_secrets(get_secrets_req_alice);

        // This call will block until generation (or timeout)
        // It implicitly covers steps 2 & 3
        let get_secrets_resp_alice = timeout(
            Duration::from_secs(45), // Timeout slightly longer than P2P timeout + buffer
            alice_get_secrets_future,
        )
        .await
        .expect("Alice GetSecrets timed out")
        .expect("Alice GetSecrets call failed");

        // --- 4. Verify Alice generated the secret ---
        info!("Step 4: Verify Alice generated the secret");
        // Add a small delay to allow generation and state update
        sleep_ms(500).await;
        assert!(
            check_enclave_secret(&alice.enclave_client, &secret_id)
                .await
                .unwrap(),
            "Alice should have the secret after generation"
        );

        let secrets_box_alice_proto = get_secrets_resp_alice
            .into_inner()
            .secrets_box
            .expect("Alice's GetSecrets response missing SecretsBox");
        let secrets_box_alice = SecretsBox::from_proto(secrets_box_alice_proto);
        assert!(
            !secrets_box_alice.contained_secret_ids.is_empty(),
            "Alice's response box should contain the generated secret"
        );
        assert!(
            secrets_box_alice.contained_secret_ids.contains(&secret_id),
            "Alice's response box missing correct secret ID"
        );

        info!("Step 5: Alice now has the secret");

        // --- 6. Bob receives GetSecrets request ---
        info!("Step 6: Bob receives GetSecrets request");
        let mut bob_requester_key_bytes = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bob_requester_key_bytes);
        let get_secrets_req_bob = create_daemon_get_secrets_request(
            &secret_id,
            "external_requester_bob",
            &bob_requester_key_bytes,
        );

        // Verify Bob doesn't have the secret initially
        assert!(
            !check_enclave_secret(&bob.enclave_client, &secret_id)
                .await
                .unwrap(),
            "Bob should not have the secret initially"
        );

        // --- 7. Bob asks Alice (P2P), Alice sends secret ---
        // --- 8. Bob receives and stores secret ---
        info!("Step 7 & 8: Bob requests from Alice (succeeds), stores secret");
        let mut bob_client = bob.daemon_grpc_client.clone();
        let bob_get_secrets_future = bob_client.get_secrets(get_secrets_req_bob);

        // This call will block until Bob gets the secret from Alice
        let get_secrets_resp_bob = timeout(
            Duration::from_secs(45), // Allow time for P2P round trip
            bob_get_secrets_future,
        )
        .await
        .expect("Bob GetSecrets timed out")
        .expect("Bob GetSecrets call failed");

        // --- 9. Verify Bob now has the secret ---
        info!("Step 9: Verify Bob now has the secret");
        // Add a small delay to allow Bob's put_secrets to complete via P2P message handling
        sleep_ms(1000).await; // Increased delay

        assert!(
            check_enclave_secret(&bob.enclave_client, &secret_id)
                .await
                .unwrap(),
            "Bob should have the secret after receiving from Alice"
        );

        // Verify Bob's response box also contains the secret
        let secrets_box_bob_proto = get_secrets_resp_bob
            .into_inner()
            .secrets_box
            .expect("Bob's GetSecrets response missing SecretsBox");
        let secrets_box_bob = SecretsBox::from_proto(secrets_box_bob_proto);
        assert!(
            !secrets_box_bob.contained_secret_ids.is_empty(),
            "Bob's response box should contain the secret"
        );
        assert!(
            secrets_box_bob.contained_secret_ids.contains(&secret_id),
            "Bob's response box missing correct secret ID"
        );

        info!("Workflow completed successfully!");
    })
    .await
    .expect("Test timed out");
}
