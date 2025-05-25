#[cfg(test)]
mod tests;

use std::{collections::HashMap, sync::Arc};

use nxcc_interface::{
    proto::vm::{
        HttpRequest as ProtoHttpRequest, HttpResponse as ProtoHttpResponse,
        InvokeWorkerRequest as ProtoInvokeWorkerRequest, TrustedConfig, UntrustedConfig,
    },
    types::{
        ConsumerInfo, EventPayload, PolicyExecutionReport, PolicyExecutionRequest, VmAddress,
        Web3Log, WorkerBundle, WorkerManifest,
    },
};
#[cfg(test)]
use nxcc_vm_base::client::mock::MockVmServiceClient;
use nxcc_vm_base::{
    client::{ClientError, VmClient as _, VmServiceClient},
    tls::MtlsCertificates,
};
use serde::Serialize;
use thiserror::Error;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error, info, warn};

use crate::secrets::Secrets;

#[derive(Error, Debug)]
pub enum RunnerError {
    #[error("VM with ID '{0}' not attached")]
    VmNotAttached(String),
    #[error("Worker with ID '{0}' not found")]
    WorkerNotFound(String),
    #[error("VM connection error: {0}")]
    VmConnection(#[from] ClientError),
    #[error("Deserialization error: {0}")]
    Deserialization(String),
    #[error("Policy execution failed in VM: {0}")]
    PolicyExecutionFailed(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("TLS configuration error: {0}")]
    TlsConfig(#[from] nxcc_vm_base::tls::TlsError),
    #[error("Failed to start worker in VM: {0}")]
    WorkerStartFailed(String),
    #[error("Unsupported VM address type: {0}")]
    UnsupportedVmAddress(String),
    #[error("Event delivery channel send error: {0}")]
    EventSendError(String),
}

// Define an enum to hold different VM client implementations
// Make it cfg-gated so the mock variant only exists in test builds
/// Enum for different VM client implementations
/// The mock variant is only available during tests
#[derive(Clone)]
pub enum VmClient {
    // Real VM service client
    Real(VmServiceClient),

    // Mock client only exists during tests
    #[cfg(test)]
    Mock(MockVmServiceClient),
}

// Implement the necessary methods to delegate to the inner client
impl VmClient {
    // Async function to start a worker
    pub async fn start_worker(
        &mut self,
        worker_type_id: String,
        worker_code: Vec<u8>,
        untrusted_config: UntrustedConfig,
        trusted_config: TrustedConfig,
    ) -> Result<String, ClientError> {
        match self {
            VmClient::Real(client) => {
                client
                    .start_worker(
                        worker_type_id,
                        worker_code,
                        untrusted_config,
                        trusted_config,
                    )
                    .await
            }
            #[cfg(test)]
            VmClient::Mock(client) => {
                client
                    .start_worker(
                        worker_type_id,
                        worker_code,
                        untrusted_config,
                        trusted_config,
                    )
                    .await
            }
        }
    }

    // Async function to stop a worker
    pub async fn stop_worker(&mut self, worker_id: String) -> Result<(), ClientError> {
        match self {
            VmClient::Real(client) => client.stop_worker(worker_id).await,
            #[cfg(test)]
            VmClient::Mock(client) => client.stop_worker(worker_id).await,
        }
    }

    // Async function to invoke a worker
    pub async fn invoke_worker(
        &mut self,
        worker_id: String,
        handler_name: String, // Added handler_name
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, ClientError> {
        match self {
            VmClient::Real(client) => client.invoke_worker(worker_id, handler_name, payload).await,
            #[cfg(test)]
            VmClient::Mock(client) => client.invoke_worker(worker_id, handler_name, payload).await,
        }
    }

    // Async function to invoke an HTTP request on a worker
    pub async fn invoke_http(
        &mut self,
        worker_id: String,
        request: ProtoHttpRequest,
    ) -> Result<ProtoHttpResponse, ClientError> {
        match self {
            VmClient::Real(client) => client.invoke_http(worker_id, request).await,
            #[cfg(test)]
            VmClient::Mock(client) => client.invoke_http(worker_id, request).await,
        }
    }
}

// Create a convenient From implementation for VmServiceClient to make client creation more ergonomic
impl From<VmServiceClient> for VmClient {
    fn from(client: VmServiceClient) -> Self {
        VmClient::Real(client)
    }
}

// Create a convenient From implementation for MockVmServiceClient when in test mode
#[cfg(test)]
impl From<MockVmServiceClient> for VmClient {
    fn from(client: MockVmServiceClient) -> Self {
        VmClient::Mock(client)
    }
}

/// Manages attached VM clients and worker mappings.
pub struct RunnerService {
    /// Stores active VM clients, keyed by the vm_id assigned during attach.
    vms: Arc<RwLock<HashMap<String, VmClient>>>,
    /// Maps running worker_id (returned by VM) back to the vm_id it runs on.
    worker_map: Arc<RwLock<HashMap<String, String>>>,
    /// Shared secrets service for storing authorizations.
    secrets: Arc<Secrets>,
    /// Sender for the internal event queue: (worker_id, handler_name, serialized_vm_invocation_payload)
    event_tx: mpsc::Sender<(String, String, Vec<u8>)>,
}

/// Structure passed as payload to VmClient::invoke_worker for event delivery.
/// The VM (e.g., workerd VMM) will deserialize this and use the handler_name.
#[derive(Serialize)] // Keep Serialize
struct VmEventInvocation<'a> {
    // Owned version
    handler: String,
    #[serde(borrow)]
    event_payload: EventPayload<'a>,
}

impl RunnerService {
    pub fn new(secrets: Arc<Secrets>) -> Self {
        let (event_tx, mut event_rx) = mpsc::channel::<(String, String, Vec<u8>)>(1024); // TODO: Make capacity configurable

        let vms_clone = Arc::new(RwLock::new(HashMap::<String, VmClient>::new()));
        let worker_map_clone = Arc::new(RwLock::new(HashMap::<String, String>::new()));

        let vms_for_task = vms_clone.clone();
        let worker_map_for_task = worker_map_clone.clone();

        tokio::spawn(async move {
            info!("Enclave event processing task started.");
            while let Some((worker_id, handler_name, vm_invocation_payload_bytes)) =
                event_rx.recv().await
            {
                debug!(
                    "Processing event for worker_id: {}, handler: {}, payload_size: {}",
                    worker_id,
                    handler_name,
                    vm_invocation_payload_bytes.len()
                );
                let vm_id_option = worker_map_for_task.read().await.get(&worker_id).cloned();

                if let Some(vm_id) = vm_id_option {
                    let mut vms_guard = vms_for_task.write().await;
                    if let Some(client) = vms_guard.get_mut(&vm_id) {
                        match client
                            .invoke_worker(
                                worker_id.clone(),
                                handler_name.clone(),
                                vm_invocation_payload_bytes,
                            )
                            .await
                        {
                            Ok(response) => {
                                debug!(
                                    "Worker {} handler {} invocation successful, response_size: {}",
                                    worker_id,
                                    handler_name,
                                    response.len()
                                );
                                // TODO: Handle worker response if necessary
                            }
                            Err(e) => {
                                error!(
                                    "Failed to invoke worker {} handler {} in VM {}: {}",
                                    worker_id, handler_name, vm_id, e
                                );
                            }
                        }
                    } else {
                        error!(
                            "VM {} not found for worker {} during event processing.",
                            vm_id, worker_id
                        );
                    }
                } else {
                    error!(
                        "Worker {} not found in map during event processing.",
                        worker_id
                    );
                }
            }
            info!("Enclave event processing task stopped.");
        });

        Self {
            vms: vms_clone,
            worker_map: worker_map_clone,
            secrets,
            event_tx,
        }
    }

    /// Establishes a connection to a VM and stores it.
    pub async fn attach_vm(&self, vm_id: String, address: VmAddress) -> Result<bool, RunnerError> {
        info!("Attempting to attach VM '{}' at {:?}", vm_id, address);

        // Generate ephemeral mTLS certs for this connection attempt
        let certs = MtlsCertificates::new()?;
        let client_tls_config = certs.client_tls_config()?;

        let client_result = match address {
            VmAddress::Tcp(tcp) => {
                let addr_str = format!("{}:{}", tcp.host, tcp.port);
                let socket_addr: std::net::SocketAddr = addr_str
                    .parse()
                    .map_err(|e| RunnerError::Internal(format!("Invalid TCP address: {e}")))?;
                VmServiceClient::connect(socket_addr, client_tls_config).await
            }
            #[cfg(feature = "uds")]
            VmAddress::Uds(uds) => VmServiceClient::connect_uds(uds.path, client_tls_config).await,
            #[cfg(not(feature = "uds"))]
            VmAddress::Uds(uds) => {
                return Err(RunnerError::UnsupportedVmAddress(format!(
                    "UDS address type not supported in this build: {}",
                    uds.path
                )));
            }
            #[cfg(feature = "vsock")]
            VmAddress::Vsock(vsock) => {
                VmServiceClient::connect_vsock(vsock.cid, vsock.port, client_tls_config).await
            }
            #[cfg(not(feature = "vsock"))]
            VmAddress::Vsock(vsock) => {
                return Err(RunnerError::UnsupportedVmAddress(format!(
                    "VSOCK address type not supported in this build: CID={}, Port={}",
                    vsock.cid, vsock.port
                )));
            }
        };

        match client_result {
            Ok(client) => {
                let mut vms_guard = self.vms.write().await;
                // Use the From implementation to convert to VmClient::Real
                vms_guard.insert(vm_id.clone(), client.into());
                info!("Successfully attached VM '{}'", vm_id);
                Ok(true)
            }
            Err(e) => {
                error!("Failed to connect and attach VM '{}': {}", vm_id, e);
                Err(e.into())
            }
        }
    }

    #[cfg(test)]
    pub async fn attach_mock_client(&self, vm_id: String, mock_client: MockVmServiceClient) {
        let mut vms_guard = self.vms.write().await;
        vms_guard.insert(vm_id, mock_client.into());
    }

    /// Removes a VM connection.
    pub async fn detach_vm(&self, vm_id: String) -> Result<(), RunnerError> {
        info!("Detaching VM '{}'", vm_id);
        let mut vms_guard = self.vms.write().await;
        if vms_guard.remove(&vm_id).is_some() {
            // Also remove any workers associated with this VM
            let mut worker_map_guard = self.worker_map.write().await;
            worker_map_guard.retain(|_worker_id, mapped_vm_id| *mapped_vm_id != vm_id);
            // TODO: attempt to kill the workers
            info!("Successfully detached VM '{}'", vm_id);
            Ok(())
        } else {
            warn!(
                "Attempted to detach VM '{}', but it was not attached.",
                vm_id
            );
            Ok(())
        }
    }

    /// Starts a worker in a specified VM.
    pub async fn run_worker(
        &self,
        vm_id: String,
        worker_manifest: WorkerManifest,
        worker_bundle: WorkerBundle,
    ) -> Result<String, RunnerError> {
        info!(
            "Requesting to run worker in VM '{}' (manifest user_data: {:?}, bundle payload size: \
             {})",
            vm_id,
            worker_manifest.userdata,
            worker_bundle.payload().executable.len()
        );

        let mut worker_secrets_for_vm = HashMap::new();
        if !worker_manifest.identities.is_empty() {
            let bundle_payload_hash = worker_bundle.hash_signed_payload();
            let dsse_signature = worker_bundle.get_dsse_signature();

            let worker_consumer_info = ConsumerInfo {
                bundle_hash: bundle_payload_hash,
                signature: dsse_signature,
            };

            match self.secrets.get_secrets_for_local_worker(
                worker_manifest.identities.clone(), // Vec<(SecretId, String)>
                worker_consumer_info,
            ) {
                Ok(secrets_map) => {
                    worker_secrets_for_vm = secrets_map;
                    info!(
                        "Retrieved {} secrets for local worker",
                        worker_secrets_for_vm.len()
                    );
                }
                Err(e) => {
                    error!("Failed to get secrets for local worker: {}", e);
                    return Err(RunnerError::Internal(format!(
                        "Failed to get secrets for worker: {}",
                        e
                    )));
                }
            }
        }

        let untrusted_config = UntrustedConfig {
            userdata_json: serde_json::to_string(&worker_manifest.userdata).unwrap_or_default(),
            ..Default::default()
        };
        let trusted_config = TrustedConfig {
            secrets: worker_secrets_for_vm,
            limits: None,
        };

        let mut vms_guard = self.vms.write().await; // Use write lock as we need a mutable client
        let client = vms_guard
            .get_mut(&vm_id)
            .ok_or_else(|| RunnerError::VmNotAttached(vm_id.clone()))?;
        let worker_type_id = "policy-worker".to_string(); // TODO: placeholder

        match client
            .start_worker(
                worker_type_id, // This is the 'type' id for the VM
                worker_bundle.payload().executable,
                untrusted_config,
                trusted_config,
            )
            .await
        {
            Ok(instance_id) => {
                info!(
                    "Successfully started worker instance '{}' in VM '{}'.",
                    instance_id, vm_id
                );
                let mut worker_map_guard = self.worker_map.write().await;
                worker_map_guard.insert(instance_id.clone(), vm_id.clone());
                drop(worker_map_guard);
                Ok(instance_id)
            }
            Err(e) => {
                error!("Failed to start worker in VM '{}': {}", vm_id, e);
                // Map specific client errors if needed
                match e {
                    ClientError::Grpc(status) => {
                        Err(RunnerError::WorkerStartFailed(status.message().to_string()))
                    }
                    _ => Err(RunnerError::VmConnection(e)),
                }
            }
        }
    }

    /// Terminates a specific worker instance.
    pub async fn terminate_worker(&self, worker_id: String) -> Result<(), RunnerError> {
        info!("Requesting to terminate worker '{}'", worker_id);

        let vm_id = {
            let worker_map_guard = self.worker_map.read().await;
            worker_map_guard
                .get(&worker_id)
                .cloned()
                .ok_or_else(|| RunnerError::WorkerNotFound(worker_id.clone()))?
        };

        let mut vms_guard = self.vms.write().await; // Write lock for mutable client
        let client = vms_guard
            .get_mut(&vm_id)
            .ok_or_else(|| RunnerError::VmNotAttached(vm_id.clone()))?; // Should not happen if worker_map is consistent

        match client.stop_worker(worker_id.clone()).await {
            Ok(()) => {
                info!(
                    "Successfully requested termination for worker '{}'",
                    worker_id
                );
                // Remove from worker map
                let mut worker_map_guard = self.worker_map.write().await;
                worker_map_guard.remove(&worker_id);
                Ok(())
            }
            Err(ClientError::Grpc(status)) if status.code() == tonic::Code::NotFound => {
                warn!(
                    "Worker '{}' not found in VM '{}' during termination request.",
                    worker_id, vm_id
                );
                // Remove from worker map if it exists locally
                let mut worker_map_guard = self.worker_map.write().await;
                worker_map_guard.remove(&worker_id);
                Ok(())
            }
            Err(e) => {
                error!(
                    "Failed to request termination for worker '{}' in VM '{}': {}",
                    worker_id, vm_id, e
                );
                Err(e.into())
            }
        }
    }

    /// Executes a policy worker against multiple contexts.
    pub async fn execute_policy(
        &self,
        worker_id: String,
        contexts: Vec<PolicyExecutionRequest>,
    ) -> Result<Vec<PolicyExecutionRequest>, RunnerError> {
        info!(
            "Executing policy worker '{}' for {} contexts",
            worker_id,
            contexts.len()
        );

        let vm_id = {
            let worker_map_guard = self.worker_map.read().await;
            worker_map_guard
                .get(&worker_id)
                .cloned()
                .ok_or_else(|| RunnerError::WorkerNotFound(worker_id.clone()))?
        };

        let payload = serde_json::to_vec(&contexts).unwrap();

        let mut vms_guard = self.vms.write().await; // Write lock for mutable client
        let client = vms_guard
            .get_mut(&vm_id)
            .ok_or_else(|| RunnerError::VmNotAttached(vm_id.clone()))?;

        const POLICY_HANDLER_NAME: &str = "_policy"; // Default handler for policy execution
        let result_payload = client
            .invoke_worker(worker_id.clone(), POLICY_HANDLER_NAME.to_string(), payload)
            .await
            .map_err(|e| {
                error!(
                    "Policy execution (handler: {}) invocation failed for worker '{}' in VM '{}': \
                     {}",
                    POLICY_HANDLER_NAME, worker_id, vm_id, e
                );
                RunnerError::VmConnection(e) // Or map specific errors
            })?;

        // Deserialize the result payload from the VM
        // For policy execution, the VM returns Vec<bool> indicating success for each context index.
        // The handler_name for policy execution is fixed (e.g., "_policy").
        let results: Vec<bool> = serde_json::from_slice(result_payload.as_slice())
            .map_err(|e| RunnerError::Deserialization(e.to_string()))?;

        if results.len() != contexts.len() {
            error!(
                "Mismatched number of results ({}) and contexts ({}) from policy worker '{}'",
                results.len(),
                contexts.len(),
                worker_id
            );
            return Err(RunnerError::PolicyExecutionFailed(
                "Mismatched result count".to_string(),
            ));
        }

        let current_time = chrono::Utc::now().timestamp() as u64;
        let mut satisfied_contexts = Vec::new();

        for (i, context) in contexts.into_iter().enumerate() {
            if results[i] {
                // Policy satisfied for this context
                debug!(
                    "Policy satisfied for context {} (Node ID: {})",
                    i,
                    context.env_report.node_id // node_id used for logging only
                );
                let report = PolicyExecutionReport {
                    request: context.clone(),
                    decision: true,
                    timestamp: current_time,
                };
                // Store authorization in the secrets service
                self.secrets.store_authorization(report);
                satisfied_contexts.push(context);
            } else {
                debug!(
                    "Policy denied for context {} (Node ID: {})",
                    i,
                    context.env_report.node_id // node_id used for logging only
                );
            }
        }

        info!(
            "Policy execution complete for worker '{}'. {}/{} contexts satisfied.",
            worker_id,
            satisfied_contexts.len(),
            results.len()
        );
        Ok(satisfied_contexts)
    }

    /// Delivers a batch of asynchronous events to appropriate workers.
    pub async fn deliver_batch_events(
        &self,
        events: Vec<(String, String, EventPayload<'static>)>, // (worker_id, handler_name, event_payload)
    ) -> Result<(), RunnerError> {
        info!("Received batch of {} events for delivery.", events.len());
        for (worker_id, handler_name, event_payload) in events {
            // 1. Verification (stub)
            debug!(
                "Stub verification for event to worker_id: {}, handler: {}",
                worker_id, handler_name
            );

            // 2. Serialize VmEventInvocation for VM
            let vm_invocation = VmEventInvocation {
                handler: handler_name.clone(), // Clone handler_name
                event_payload,                 // Move event_payload
            };
            let vm_invocation_payload_bytes = serde_json::to_vec(&vm_invocation).map_err(|e| {
                RunnerError::Internal(format!("Failed to serialize VmEventInvocation: {}", e))
            })?;

            // 3. Send to internal queue
            if let Err(e) = self
                .event_tx
                .send((
                    worker_id.clone(),
                    handler_name.clone(),
                    vm_invocation_payload_bytes,
                ))
                .await
            {
                error!(
                    "Failed to send event (handler: {}) to internal queue for worker {}: {}",
                    handler_name, worker_id, e
                );
                return Err(RunnerError::EventSendError(e.to_string()));
            }
        }
        Ok(())
    }

    /// Invokes a worker with an HTTP request.
    pub async fn invoke_http_worker(
        &self,
        worker_id: String,
        http_request: ProtoHttpRequest,
    ) -> Result<ProtoHttpResponse, RunnerError> {
        info!(
            "Invoking HTTP worker '{}' with URI: {}",
            worker_id, http_request.uri
        );

        let vm_id = {
            let worker_map_guard = self.worker_map.read().await;
            worker_map_guard
                .get(&worker_id)
                .cloned()
                .ok_or_else(|| RunnerError::WorkerNotFound(worker_id.clone()))?
        };

        let mut vms_guard = self.vms.write().await;
        let client = vms_guard
            .get_mut(&vm_id)
            .ok_or_else(|| RunnerError::VmNotAttached(vm_id.clone()))?;

        client
            .invoke_http(worker_id.clone(), http_request)
            .await
            .map_err(|e| {
                error!(
                    "HTTP invocation failed for worker '{}' in VM '{}': {}",
                    worker_id, vm_id, e
                );
                RunnerError::VmConnection(e)
            })
    }
}

#[cfg(test)]
impl RunnerService {
    pub(crate) async fn set_worker_vm_mapping(&self, worker_id: String, vm_id: String) {
        self.worker_map.write().await.insert(worker_id, vm_id);
    }
}
