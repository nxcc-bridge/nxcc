use std::{collections::HashMap, sync::Arc};

use nxcc_interface::{
    proto::vm::{TrustedConfig, UntrustedConfig},
    types::{PolicyExecutionReport, PolicyExecutionRequest, VmAddress},
};
#[cfg(any(test, feature = "test"))]
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
    #[error("Serialization error: {0}")]
    Serialization(String),
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
    #[cfg(any(test, feature = "test"))]
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
            #[cfg(any(test, feature = "test"))]
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
            #[cfg(any(test, feature = "test"))]
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
            #[cfg(any(test, feature = "test"))]
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
#[cfg(any(test, feature = "test"))]
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

    #[cfg(any(test, feature = "test"))]
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

        // Serialize contexts for the VM payload
        let mut payload = Vec::new();
        ciborium::into_writer(&contexts, &mut payload)
            .map_err(|e| RunnerError::Serialization(e.to_string()))?;

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
        let results: Vec<bool> = ciborium::from_reader(result_payload.as_slice())
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
                    i, context.env_report.node_id
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
                    i, context.env_report.node_id
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nxcc_interface::{
        proto::vm::WorkerStatus,
        types::{AttestationReport, ConsumerInfo, EnvReport, PolicyExecutionRequest, SecretId},
    };
    use nxcc_vm_base::client::{
        VmClient as _,
        mock::{MockExecutionBehavior, MockVmServiceClient},
    };

    use super::*; // Import items from the outer module (RunnerService, RunnerError, etc.)
    use crate::secrets::Secrets; // Assuming secrets.rs is in the same crate src dir

    // Helper function to create a default SecretId for tests
    fn test_secret_id(id: u64) -> SecretId {
        SecretId {
            chain_id: 1,
            identity_address: format!("0x{:040x}", id).parse().unwrap(),
            identity_id: alloy_primitives::Uint::from_limbs_slice(&[id]),
        }
    }

    // Helper function to create a default PolicyExecutionRequest for tests
    fn test_policy_request(node_id: &str, secret_ids: Vec<SecretId>) -> PolicyExecutionRequest {
        PolicyExecutionRequest {
            secret_ids,
            consumer: ConsumerInfo {
                code_hash: vec![1; 32],
                signature: vec![2; 64],
            },
            env_report: EnvReport {
                attestation: AttestationReport {
                    ephemeral_public_key: vec![3; 32], // Needs to be 32 bytes for Secrets mock
                    block_hashes: vec![vec![4, 5], vec![6, 7]],
                    user_data: vec![8, 9],
                },
                operator_signature: vec![10; 64],
                node_id: node_id.to_string(),
            },
        }
    }

    // Helper setup function
    fn setup() -> (Arc<Secrets>, RunnerService, MockVmServiceClient) {
        let secrets = Secrets::new();
        let runner_service = RunnerService::new(secrets.clone());
        let mock_client = MockVmServiceClient::new();
        (secrets, runner_service, mock_client)
    }

    // Helper to manually "attach" a mock VM
    async fn attach_mock_vm(
        runner_service: &RunnerService,
        vm_id: &str,
        client: MockVmServiceClient,
    ) {
        let mut vms_guard = runner_service.vms.write().await;
        vms_guard.insert(vm_id.to_string(), client.into());
    }

    // Helper to manually add a worker mapping
    async fn add_worker_mapping(runner_service: &RunnerService, worker_id: &str, vm_id: &str) {
        let mut worker_map_guard = runner_service.worker_map.write().await;
        worker_map_guard.insert(worker_id.to_string(), vm_id.to_string());
    }

    #[tokio::test]
    async fn test_new_runner_service() {
        let (secrets, runner_service, _) = setup();
        assert!(runner_service.vms.read().await.is_empty());
        assert!(runner_service.worker_map.read().await.is_empty());
        // Check if the secrets Arc points to the same allocation
        assert!(Arc::ptr_eq(&runner_service.secrets, &secrets));
    }

    // Note: We don't test the real attach_vm due to network/TLS complexity.
    // We test the state changes via manual insertion and detach_vm.

    #[tokio::test]
    async fn test_detach_vm_exists() {
        let (_secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-1";
        let worker_id_1 = "worker-on-vm1-1";
        let worker_id_2 = "worker-on-vm1-2";
        let worker_id_other = "worker-on-vm2";

        attach_mock_vm(&runner_service, vm_id, mock_client).await;
        add_worker_mapping(&runner_service, worker_id_1, vm_id).await;
        add_worker_mapping(&runner_service, worker_id_2, vm_id).await;
        add_worker_mapping(&runner_service, worker_id_other, "vm-2").await; // Belongs to another VM

        assert!(runner_service.vms.read().await.contains_key(vm_id));
        assert!(
            runner_service
                .worker_map
                .read()
                .await
                .contains_key(worker_id_1)
        );
        assert!(
            runner_service
                .worker_map
                .read()
                .await
                .contains_key(worker_id_2)
        );
        assert!(
            runner_service
                .worker_map
                .read()
                .await
                .contains_key(worker_id_other)
        );

        let result = runner_service.detach_vm(vm_id.to_string()).await;
        result.unwrap(); // Expect Ok

        assert!(!runner_service.vms.read().await.contains_key(vm_id));
        // Check workers associated with vm_id are removed
        assert!(
            !runner_service
                .worker_map
                .read()
                .await
                .contains_key(worker_id_1)
        );
        assert!(
            !runner_service
                .worker_map
                .read()
                .await
                .contains_key(worker_id_2)
        );
        // Check worker on other VM remains
        assert!(
            runner_service
                .worker_map
                .read()
                .await
                .contains_key(worker_id_other)
        );
    }

    #[tokio::test]
    async fn test_detach_vm_not_exists() {
        let (_secrets, runner_service, _mock_client) = setup();
        let vm_id = "vm-nonexistent";

        assert!(!runner_service.vms.read().await.contains_key(vm_id));

        // Detaching a non-existent VM should be Ok (idempotent)
        let result = runner_service.detach_vm(vm_id.to_string()).await;
        result.unwrap();

        assert!(!runner_service.vms.read().await.contains_key(vm_id));
        assert!(runner_service.worker_map.read().await.is_empty());
    }

    #[tokio::test]
    async fn test_run_worker_success() {
        let (_secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-run";
        let worker_code = vec![1, 2, 3];
        let manifest = vec![4, 5];
        let expected_instance_id = "instance-policy-worker-1"; // Default mock ID format

        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await; // Clone needed if we inspect mock later

        let result = runner_service
            .run_worker(vm_id.to_string(), worker_code.clone(), manifest.clone())
            .await;

        let instance_id = result.unwrap();
        assert_eq!(instance_id, expected_instance_id);

        // Verify worker map
        let worker_map = runner_service.worker_map.read().await;
        assert_eq!(
            worker_map.get(expected_instance_id),
            Some(&vm_id.to_string())
        );

        // Verify mock client state (optional but good)
        let (status, code) = mock_client.get_worker(expected_instance_id).unwrap();
        assert_eq!(status, WorkerStatus::Running);
        assert_eq!(code, worker_code);
    }

    #[tokio::test]
    async fn test_run_worker_vm_not_attached() {
        let (_secrets, runner_service, _mock_client) = setup();
        let vm_id = "vm-not-here";
        let worker_code = vec![1, 2, 3];
        let manifest = vec![4, 5];

        let result = runner_service
            .run_worker(vm_id.to_string(), worker_code.clone(), manifest.clone())
            .await;

        assert!(matches!(result, Err(RunnerError::VmNotAttached(id)) if id == vm_id));
        assert!(runner_service.worker_map.read().await.is_empty());
    }

    #[tokio::test]
    async fn test_run_worker_start_fails_in_vm() {
        let (_secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-fail-start";
        let worker_code = vec![1, 2, 3];
        let manifest = vec![4, 5];
        let error_msg = "VM resource limit exceeded";

        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        mock_client.fail_next_operation(error_msg); // Configure mock to fail start_worker

        let result = runner_service
            .run_worker(vm_id.to_string(), worker_code.clone(), manifest.clone())
            .await;

        assert!(matches!(result, Err(RunnerError::WorkerStartFailed(msg)) if msg == error_msg));
        assert!(runner_service.worker_map.read().await.is_empty()); // Should not be added to map
    }

    #[tokio::test]
    async fn test_terminate_worker_success() {
        let (_secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-term";
        let worker_id = "worker-to-term";

        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        add_worker_mapping(&runner_service, worker_id, vm_id).await;
        // Add worker to mock so stop_worker doesn't fail with NotFound initially
        mock_client.add_worker(
            worker_id.to_string(),
            vec![],
            WorkerStatus::Running,
            "".to_string(),
            Default::default(),
            Default::default(),
        );

        assert!(
            runner_service
                .worker_map
                .read()
                .await
                .contains_key(worker_id)
        );
        assert!(mock_client.get_worker(worker_id).is_some());

        let result = runner_service.terminate_worker(worker_id.to_string()).await;
        result.unwrap();

        // Verify removed from map
        assert!(
            !runner_service
                .worker_map
                .read()
                .await
                .contains_key(worker_id)
        );
        // Verify removed from mock VM state
        assert!(mock_client.get_worker(worker_id).is_none());
    }

    #[tokio::test]
    async fn test_terminate_worker_not_found_in_vm() {
        let (_secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-term-nf-vm";
        let worker_id = "worker-nf-vm";

        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        add_worker_mapping(&runner_service, worker_id, vm_id).await;
        // Do NOT add worker to mock client, so stop_worker will return NotFound

        assert!(
            runner_service
                .worker_map
                .read()
                .await
                .contains_key(worker_id)
        );

        let result = runner_service.terminate_worker(worker_id.to_string()).await;
        result.unwrap(); // Should still be Ok(()) as per code logic

        // Verify removed from map even if VM reported NotFound
        assert!(
            !runner_service
                .worker_map
                .read()
                .await
                .contains_key(worker_id)
        );
    }

    #[tokio::test]
    async fn test_terminate_worker_not_found_locally() {
        let (_secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-term-nf-local";
        let worker_id = "worker-nf-local";

        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        // Do NOT add worker mapping

        assert!(
            !runner_service
                .worker_map
                .read()
                .await
                .contains_key(worker_id)
        );

        let result = runner_service.terminate_worker(worker_id.to_string()).await;

        assert!(matches!(result, Err(RunnerError::WorkerNotFound(id)) if id == worker_id));
    }

    #[tokio::test]
    async fn test_terminate_worker_vm_detached_consistency_issue() {
        let (_secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-term-detached";
        let worker_id = "worker-vm-detached";

        // Attach, add mapping, then detach VM *before* terminating worker
        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        add_worker_mapping(&runner_service, worker_id, vm_id).await;
        runner_service.vms.write().await.remove(vm_id); // Simulate detachment

        assert!(
            runner_service
                .worker_map
                .read()
                .await
                .contains_key(worker_id)
        ); // Mapping still exists
        assert!(runner_service.vms.read().await.get(vm_id).is_none()); // VM is gone

        let result = runner_service.terminate_worker(worker_id.to_string()).await;

        // It finds the worker mapping, tries to get the VM client, fails.
        assert!(matches!(result, Err(RunnerError::VmNotAttached(id)) if id == vm_id));
    }

    #[tokio::test]
    async fn test_terminate_worker_fails_in_vm() {
        let (_secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-term-fail";
        let worker_id = "worker-term-fail";
        let error_msg = "VM internal error during stop";

        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        add_worker_mapping(&runner_service, worker_id, vm_id).await;
        mock_client.add_worker(
            // Add worker so stop doesn't cause NotFound
            worker_id.to_string(),
            vec![],
            WorkerStatus::Running,
            "".into(),
            Default::default(),
            Default::default(),
        );
        mock_client.fail_next_operation(error_msg); // Configure mock to fail stop_worker

        assert!(
            runner_service
                .worker_map
                .read()
                .await
                .contains_key(worker_id)
        );

        let result = runner_service.terminate_worker(worker_id.to_string()).await;

        assert!(
            matches!(result, Err(RunnerError::VmConnection(ClientError::Grpc(status))) if status.message() == error_msg)
        );
        // Verify *not* removed from map on general failure
        assert!(
            runner_service
                .worker_map
                .read()
                .await
                .contains_key(worker_id)
        );
    }

    #[tokio::test]
    async fn test_invoke_worker_success() {
        let (_secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-invoke";
        let worker_id = "worker-invoke";
        let payload = vec![10, 20, 30];
        let expected_response = vec![40, 50, 60];

        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        add_worker_mapping(&runner_service, worker_id, vm_id).await;
        mock_client.add_worker(
            // Add worker so invoke doesn't fail with NotFound
            worker_id.to_string(),
            vec![],
            WorkerStatus::Running,
            "".into(),
            Default::default(),
            Default::default(),
        );
        // Configure mock response
        mock_client.set_worker_execution_behavior(
            worker_id,
            MockExecutionBehavior::Fixed(expected_response.clone()),
        );

        let result = runner_service
            .invoke_worker(worker_id.to_string(), payload.clone())
            .await;

        let response = result.unwrap();
        assert_eq!(response, expected_response);
    }

    #[tokio::test]
    async fn test_invoke_worker_not_found_locally() {
        let (_secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-invoke-nf-local";
        let worker_id = "worker-nf-local";
        let payload = vec![10, 20, 30];

        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        // Do NOT add worker mapping

        let result = runner_service
            .invoke_worker(worker_id.to_string(), payload.clone())
            .await;

        assert!(matches!(result, Err(RunnerError::WorkerNotFound(id)) if id == worker_id));
    }

    #[tokio::test]
    async fn test_invoke_worker_fails_in_vm() {
        let (_secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-invoke-fail";
        let worker_id = "worker-invoke-fail";
        let payload = vec![10, 20, 30];
        let error_msg = "Worker execution panicked";

        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        add_worker_mapping(&runner_service, worker_id, vm_id).await;
        mock_client.add_worker(
            // Add worker so invoke doesn't fail with NotFound
            worker_id.to_string(),
            vec![],
            WorkerStatus::Running,
            "".into(),
            Default::default(),
            Default::default(),
        );
        // Configure mock to return error
        mock_client.set_worker_execution_behavior(
            worker_id,
            MockExecutionBehavior::Error(error_msg.to_string()),
        );

        let result = runner_service
            .invoke_worker(worker_id.to_string(), payload.clone())
            .await;

        assert!(
            matches!(result, Err(RunnerError::VmConnection(ClientError::Grpc(status))) if status.message() == error_msg)
        );
    }

    #[tokio::test]
    async fn test_invoke_worker_vm_detached_consistency_issue() {
        let (_secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-invoke-detached";
        let worker_id = "worker-vm-detached";
        let payload = vec![10, 20, 30];

        // Attach, add mapping, then detach VM *before* invoking worker
        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        add_worker_mapping(&runner_service, worker_id, vm_id).await;
        runner_service.vms.write().await.remove(vm_id); // Simulate detachment

        assert!(
            runner_service
                .worker_map
                .read()
                .await
                .contains_key(worker_id)
        ); // Mapping still exists
        assert!(runner_service.vms.read().await.get(vm_id).is_none()); // VM is gone

        let result = runner_service
            .invoke_worker(worker_id.to_string(), payload.clone())
            .await;

        // It finds the worker mapping, tries to get the VM client, fails.
        assert!(matches!(result, Err(RunnerError::VmNotAttached(id)) if id == vm_id));
    }

    #[tokio::test]
    async fn test_execute_policy_success_some_satisfied() {
        let (secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-policy";
        let worker_id = "policy-worker-1";
        let node_id_1 = "node-1";
        let node_id_2 = "node-2";
        let secret_id_1 = test_secret_id(101);
        let secret_id_2 = test_secret_id(102);
        let secret_id_3 = test_secret_id(103);

        let context1 = test_policy_request(node_id_1, vec![secret_id_1.clone()]);
        let context2 =
            test_policy_request(node_id_2, vec![secret_id_2.clone(), secret_id_3.clone()]);
        let contexts = vec![context1.clone(), context2.clone()];

        // Expected VM response: context1=true, context2=false
        let vm_response_bools = vec![true, false];
        let mut vm_response_payload = Vec::new();
        ciborium::into_writer(&vm_response_bools, &mut vm_response_payload).unwrap();

        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        add_worker_mapping(&runner_service, worker_id, vm_id).await;
        mock_client.add_worker(
            // Add worker so invoke doesn't fail with NotFound
            worker_id.to_string(),
            vec![],
            WorkerStatus::Running,
            "".into(),
            Default::default(),
            Default::default(),
        );
        mock_client.set_worker_execution_behavior(
            worker_id,
            MockExecutionBehavior::Fixed(vm_response_payload.clone()),
        );

        // Check initial authorization state
        assert!(!secrets.check_authorization(node_id_1, &secret_id_1));
        assert!(!secrets.check_authorization(node_id_2, &secret_id_2));
        assert!(!secrets.check_authorization(node_id_2, &secret_id_3));

        let result = runner_service
            .execute_policy(worker_id.to_string(), contexts.clone())
            .await;

        let satisfied_contexts = result.unwrap();

        // Verify only context1 is returned
        assert_eq!(satisfied_contexts.len(), 1);
        // Deep comparison might be needed if PolicyExecutionRequest doesn't impl PartialEq well
        assert_eq!(
            satisfied_contexts[0].env_report.node_id,
            context1.env_report.node_id
        );
        assert_eq!(satisfied_contexts[0].secret_ids, context1.secret_ids);

        // Verify authorization stored only for satisfied context
        assert!(secrets.check_authorization(node_id_1, &secret_id_1));
        assert!(!secrets.check_authorization(node_id_2, &secret_id_2)); // context2 failed
        assert!(!secrets.check_authorization(node_id_2, &secret_id_3)); // context2 failed
    }

    #[tokio::test]
    async fn test_execute_policy_success_all_satisfied() {
        let (secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-policy-all";
        let worker_id = "policy-worker-all";
        let node_id_1 = "node-all-1";
        let secret_id_1 = test_secret_id(201);

        let context1 = test_policy_request(node_id_1, vec![secret_id_1.clone()]);
        let contexts = vec![context1.clone()];

        // Expected VM response: context1=true
        let vm_response_bools = vec![true];
        let mut vm_response_payload = Vec::new();
        ciborium::into_writer(&vm_response_bools, &mut vm_response_payload).unwrap();

        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        add_worker_mapping(&runner_service, worker_id, vm_id).await;
        mock_client.add_worker(
            worker_id.to_string(),
            vec![],
            WorkerStatus::Running,
            "".into(),
            Default::default(),
            Default::default(),
        );
        mock_client.set_worker_execution_behavior(
            worker_id,
            MockExecutionBehavior::Fixed(vm_response_payload),
        );

        assert!(!secrets.check_authorization(node_id_1, &secret_id_1));

        let result = runner_service
            .execute_policy(worker_id.to_string(), contexts.clone())
            .await;
        let satisfied_contexts = result.unwrap();

        assert_eq!(satisfied_contexts.len(), 1);
        assert_eq!(
            satisfied_contexts[0].env_report.node_id,
            context1.env_report.node_id
        );
        assert!(secrets.check_authorization(node_id_1, &secret_id_1));
    }

    #[tokio::test]
    async fn test_execute_policy_success_none_satisfied() {
        let (secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-policy-none";
        let worker_id = "policy-worker-none";
        let node_id_1 = "node-none-1";
        let secret_id_1 = test_secret_id(301);

        let context1 = test_policy_request(node_id_1, vec![secret_id_1.clone()]);
        let contexts = vec![context1.clone()];

        // Expected VM response: context1=false
        let vm_response_bools = vec![false];
        let mut vm_response_payload = Vec::new();
        ciborium::into_writer(&vm_response_bools, &mut vm_response_payload).unwrap();

        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        add_worker_mapping(&runner_service, worker_id, vm_id).await;
        mock_client.add_worker(
            worker_id.to_string(),
            vec![],
            WorkerStatus::Running,
            "".into(),
            Default::default(),
            Default::default(),
        );
        mock_client.set_worker_execution_behavior(
            worker_id,
            MockExecutionBehavior::Fixed(vm_response_payload),
        );

        assert!(!secrets.check_authorization(node_id_1, &secret_id_1));

        let result = runner_service
            .execute_policy(worker_id.to_string(), contexts.clone())
            .await;
        let satisfied_contexts = result.unwrap();

        assert!(satisfied_contexts.is_empty());
        assert!(!secrets.check_authorization(node_id_1, &secret_id_1)); // Still not authorized
    }

    #[tokio::test]
    async fn test_execute_policy_vm_invocation_fails() {
        let (secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-policy-fail";
        let worker_id = "policy-worker-fail";
        let node_id_1 = "node-fail-1";
        let secret_id_1 = test_secret_id(401);
        let error_msg = "Policy worker crashed";

        let context1 = test_policy_request(node_id_1, vec![secret_id_1.clone()]);
        let contexts = vec![context1.clone()];

        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        add_worker_mapping(&runner_service, worker_id, vm_id).await;
        mock_client.add_worker(
            worker_id.to_string(),
            vec![],
            WorkerStatus::Running,
            "".into(),
            Default::default(),
            Default::default(),
        );
        mock_client.set_worker_execution_behavior(
            worker_id,
            MockExecutionBehavior::Error(error_msg.to_string()),
        );

        let result = runner_service
            .execute_policy(worker_id.to_string(), contexts.clone())
            .await;

        assert!(
            matches!(result, Err(RunnerError::VmConnection(ClientError::Grpc(status))) if status.message() == error_msg)
        );
        assert!(!secrets.check_authorization(node_id_1, &secret_id_1)); // No authorization granted
    }

    #[tokio::test]
    async fn test_execute_policy_deserialization_error() {
        let (secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-policy-deser";
        let worker_id = "policy-worker-deser";
        let node_id_1 = "node-deser-1";
        let secret_id_1 = test_secret_id(501);

        let context1 = test_policy_request(node_id_1, vec![secret_id_1.clone()]);
        let contexts = vec![context1.clone()];

        // VM returns invalid CBOR data (not a Vec<bool>)
        let vm_response_payload = b"invalid cbor data".to_vec();

        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        add_worker_mapping(&runner_service, worker_id, vm_id).await;
        mock_client.add_worker(
            worker_id.to_string(),
            vec![],
            WorkerStatus::Running,
            "".into(),
            Default::default(),
            Default::default(),
        );
        mock_client.set_worker_execution_behavior(
            worker_id,
            MockExecutionBehavior::Fixed(vm_response_payload.clone()),
        );

        let result = runner_service
            .execute_policy(worker_id.to_string(), contexts.clone())
            .await;

        assert!(matches!(result, Err(RunnerError::Deserialization(_))));
        assert!(!secrets.check_authorization(node_id_1, &secret_id_1)); // No authorization granted
    }

    #[tokio::test]
    async fn test_execute_policy_mismatched_result_count() {
        let (secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-policy-mismatch";
        let worker_id = "policy-worker-mismatch";
        let node_id_1 = "node-mismatch-1";
        let secret_id_1 = test_secret_id(601);

        let context1 = test_policy_request(node_id_1, vec![secret_id_1.clone()]);
        let contexts = vec![context1.clone()]; // Requesting 1 context

        // VM returns results for 2 contexts (incorrect)
        let vm_response_bools = vec![true, false];
        let mut vm_response_payload = Vec::new();
        ciborium::into_writer(&vm_response_bools, &mut vm_response_payload).unwrap();

        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        add_worker_mapping(&runner_service, worker_id, vm_id).await;
        mock_client.add_worker(
            worker_id.to_string(),
            vec![],
            WorkerStatus::Running,
            "".into(),
            Default::default(),
            Default::default(),
        );
        mock_client.set_worker_execution_behavior(
            worker_id,
            MockExecutionBehavior::Fixed(vm_response_payload.clone()),
        );

        let result = runner_service
            .execute_policy(worker_id.to_string(), contexts.clone())
            .await;

        assert!(
            matches!(result, Err(RunnerError::PolicyExecutionFailed(msg)) if msg == "Mismatched result count")
        );
        assert!(!secrets.check_authorization(node_id_1, &secret_id_1)); // No authorization granted
    }

    #[tokio::test]
    async fn test_execute_policy_worker_not_found_locally() {
        let (secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-policy-nf-local";
        let worker_id = "policy-worker-nf-local";
        let node_id_1 = "node-nf-local-1";
        let secret_id_1 = test_secret_id(701);

        let context1 = test_policy_request(node_id_1, vec![secret_id_1.clone()]);
        let contexts = vec![context1.clone()];

        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        // Do NOT add worker mapping

        let result = runner_service
            .execute_policy(worker_id.to_string(), contexts.clone())
            .await;

        assert!(matches!(result, Err(RunnerError::WorkerNotFound(id)) if id == worker_id));
        assert!(!secrets.check_authorization(node_id_1, &secret_id_1)); // No authorization granted
    }

    #[tokio::test]
    async fn test_execute_policy_vm_detached_consistency_issue() {
        let (secrets, runner_service, mock_client) = setup();
        let vm_id = "vm-policy-detached";
        let worker_id = "policy-worker-detached";
        let node_id_1 = "node-detached-1";
        let secret_id_1 = test_secret_id(801);

        let context1 = test_policy_request(node_id_1, vec![secret_id_1.clone()]);
        let contexts = vec![context1.clone()];

        // Attach, add mapping, then detach VM *before* executing policy
        attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
        add_worker_mapping(&runner_service, worker_id, vm_id).await;
        runner_service.vms.write().await.remove(vm_id); // Simulate detachment

        assert!(
            runner_service
                .worker_map
                .read()
                .await
                .contains_key(worker_id)
        ); // Mapping still exists
        assert!(runner_service.vms.read().await.get(vm_id).is_none()); // VM is gone

        let result = runner_service
            .execute_policy(worker_id.to_string(), contexts.clone())
            .await;

        // It finds the worker mapping, tries to get the VM client, fails.
        assert!(matches!(result, Err(RunnerError::VmNotAttached(id)) if id == vm_id));
        assert!(!secrets.check_authorization(node_id_1, &secret_id_1)); // No authorization granted
    }
}
