#[cfg(test)]
mod tests;

use std::{collections::HashMap, sync::Arc};

use nxcc_interface::{
    proto::vm::{TrustedConfig, UntrustedConfig},
    types::{PolicyExecutionReport, PolicyExecutionRequest, VmAddress},
};
#[cfg(test)]
use nxcc_vm_base::client::mock::MockVmServiceClient;
use nxcc_vm_base::{
    client::{ClientError, VmClient as _, VmServiceClient},
    tls::MtlsCertificates,
};
use thiserror::Error;
use tokio::sync::RwLock;
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
    #[error("VM address type not supported by client")]
    UnsupportedVmAddress,
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
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, ClientError> {
        match self {
            VmClient::Real(client) => client.invoke_worker(worker_id, payload).await,
            #[cfg(test)]
            VmClient::Mock(client) => client.invoke_worker(worker_id, payload).await,
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
    vms: RwLock<HashMap<String, VmClient>>,
    /// Maps running worker_id (returned by VM) back to the vm_id it runs on.
    worker_map: RwLock<HashMap<String, String>>,
    /// Shared secrets service for storing authorizations.
    secrets: Arc<Secrets>,
}

impl RunnerService {
    pub fn new(secrets: Arc<Secrets>) -> Self {
        Self {
            vms: RwLock::new(HashMap::new()),
            worker_map: RwLock::new(HashMap::new()),
            secrets,
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
            VmAddress::Uds(_) => return Err(RunnerError::UnsupportedVmAddress),
            #[cfg(feature = "vsock")]
            VmAddress::Vsock(vsock) => {
                VmServiceClient::connect_vsock(vsock.cid, vsock.port, client_tls_config).await
            }
            #[cfg(not(feature = "vsock"))]
            VmAddress::Vsock(_) => return Err(RunnerError::UnsupportedVmAddress),
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
        worker_code: Vec<u8>,
        manifest: Vec<u8>,
    ) -> Result<String, RunnerError> {
        info!(
            "Requesting to run worker in VM '{}' (code size: {}, manifest size: {})",
            vm_id,
            worker_code.len(),
            manifest.len()
        );

        // TODO: Parse manifest to potentially extract UntrustedConfig and VM to run in.
        // For now, use defaults or pass manifest bytes directly if the VM expects it.
        let untrusted_config = UntrustedConfig {
            userdata_json: String::from_utf8_lossy(&manifest).to_string(),
            ..Default::default()
        };
        let trusted_config = TrustedConfig::default(); // TODO: example

        let mut vms_guard = self.vms.write().await; // Use write lock as we need a mutable client
        let client = vms_guard
            .get_mut(&vm_id)
            .ok_or_else(|| RunnerError::VmNotAttached(vm_id.clone()))?;

        // The worker_id in the request here is more like a type/template ID.
        // The VM returns the actual instance ID. Let's use a placeholder or derive from manifest.
        let worker_type_id = "policy-worker".to_string(); // TODO: placeholder

        match client
            .start_worker(
                worker_type_id, // This is the 'type' id for the VM
                worker_code,
                untrusted_config,
                trusted_config,
            )
            .await
        {
            Ok(instance_id) => {
                info!(
                    "Successfully started worker instance '{}' in VM '{}'",
                    instance_id, vm_id
                );
                // Store mapping from instance_id -> vm_id
                let mut worker_map_guard = self.worker_map.write().await;
                worker_map_guard.insert(instance_id.clone(), vm_id);
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

    /// Invokes a generic worker with a payload.
    pub async fn invoke_worker(
        &self,
        worker_id: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, RunnerError> {
        debug!(
            "Invoking worker '{}' with payload size {}",
            worker_id,
            payload.len()
        );

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
            .ok_or_else(|| RunnerError::VmNotAttached(vm_id.clone()))?;

        client
            .invoke_worker(worker_id.clone(), payload)
            .await
            .map_err(|e| {
                error!(
                    "Failed to invoke worker '{}' in VM '{}': {}",
                    worker_id, vm_id, e
                );
                e.into()
            })
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

        let result_payload = client
            .invoke_worker(worker_id.clone(), payload)
            .await
            .map_err(|e| {
                error!(
                    "Policy execution invocation failed for worker '{}' in VM '{}': {}",
                    worker_id, vm_id, e
                );
                RunnerError::VmConnection(e) // Or map specific errors
            })?;

        // Deserialize the result payload from the VM
        // TODO: we assume VM returns Vec<bool> indicating success for each context index
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
}
