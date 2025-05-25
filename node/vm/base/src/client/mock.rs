use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use nxcc_interface::{
    proto::vm::{
        Header as ProtoHeader, HttpRequest as ProtoHttpRequest, HttpResponse as ProtoHttpResponse,
        TrustedConfig, UntrustedConfig, WorkerStatus,
    },
    types::AttestationReport,
};
use tonic::Status;
use tracing::{debug, info, warn};

use super::{ClientError, VmClient};

/// Worker information stored in the mock
#[derive(Debug, Clone)]
struct WorkerInfo {
    id: String,
    code: Vec<u8>,
    status: WorkerStatus,
    logs: String,
    trusted_config: TrustedConfig,
    untrusted_config: UntrustedConfig,
}

/// Mock VM Service client for testing
#[derive(Clone)]
pub struct MockVmServiceClient {
    workers: Arc<Mutex<HashMap<String, WorkerInfo>>>,
    next_instance_id: Arc<Mutex<u64>>,
    fail_next_operation: Arc<Mutex<Option<String>>>,
    attestation_behavior: Arc<Mutex<MockAttestationBehavior>>,
    execution_behavior: Arc<Mutex<HashMap<String, MockExecutionBehavior>>>,
    default_execution_behavior: Arc<Mutex<MockExecutionBehavior>>,
    invocations: Arc<Mutex<HashMap<String, Vec<Vec<u8>>>>>, // worker_id -> Vec<payload>
    http_invocations: Arc<Mutex<HashMap<String, Vec<ProtoHttpRequest>>>>, // worker_id -> Vec<HttpRequest>
}

/// Configure mock attestation behavior
#[derive(Debug, Clone)]
pub enum MockAttestationBehavior {
    /// Return a standard attestation report
    Standard,
    /// Return a custom attestation report
    Custom(AttestationReport),
    /// Return an error with the specified message
    Error(String),
}

/// Configure mock execution behavior
#[derive(Clone)]
pub enum MockExecutionBehavior {
    /// Echo back the input payload (default behavior)
    Echo,
    /// Return a fixed response regardless of input
    Fixed(Vec<u8>),
    /// Return a response based on a transformation function
    Transform(Arc<dyn Fn(Vec<u8>) -> Vec<u8> + Send + Sync>),
    /// Return an error with the specified message
    Error(String),
    /// Simulate an HTTP response
    HttpResponse(ProtoHttpResponse),
}

impl Default for MockAttestationBehavior {
    fn default() -> Self {
        Self::Standard
    }
}

impl Default for MockExecutionBehavior {
    fn default() -> Self {
        Self::Echo
    }
}

impl Default for MockVmServiceClient {
    fn default() -> Self {
        Self::new()
    }
}

impl MockVmServiceClient {
    /// Create a new mock client
    pub fn new() -> Self {
        Self {
            workers: Arc::new(Mutex::new(HashMap::new())),
            next_instance_id: Arc::new(Mutex::new(1)),
            fail_next_operation: Arc::new(Mutex::new(None)),
            attestation_behavior: Arc::new(Mutex::new(MockAttestationBehavior::Standard)),
            execution_behavior: Arc::new(Mutex::new(HashMap::new())),
            default_execution_behavior: Arc::new(Mutex::new(MockExecutionBehavior::Echo)),
            invocations: Arc::new(Mutex::new(HashMap::new())),
            http_invocations: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Force the next operation to fail with the given error message
    pub fn fail_next_operation(&self, error_message: impl Into<String>) {
        let mut fail = self.fail_next_operation.lock().unwrap();
        *fail = Some(error_message.into());
    }

    /// Reset the failure state
    pub fn reset_failures(&self) {
        let mut fail = self.fail_next_operation.lock().unwrap();
        *fail = None;
    }

    /// Set a custom attestation behavior
    pub fn set_attestation_behavior(&self, behavior: MockAttestationBehavior) {
        let mut attestation = self.attestation_behavior.lock().unwrap();
        *attestation = behavior;
    }

    /// Set the default execution behavior for all workers without specific behavior
    pub fn set_default_execution_behavior(&self, behavior: MockExecutionBehavior) {
        let mut default_behavior = self.default_execution_behavior.lock().unwrap();
        *default_behavior = behavior;
    }

    /// Set a custom execution behavior for a specific worker
    pub fn set_worker_execution_behavior(&self, worker_id: &str, behavior: MockExecutionBehavior) {
        let mut behaviors = self.execution_behavior.lock().unwrap();
        behaviors.insert(worker_id.to_string(), behavior);
    }

    /// Clear all custom execution behaviors
    pub fn clear_execution_behaviors(&self) {
        let mut behaviors = self.execution_behavior.lock().unwrap();
        behaviors.clear();

        let mut default_behavior = self.default_execution_behavior.lock().unwrap();
        *default_behavior = MockExecutionBehavior::Echo;
    }

    /// Get all recorded invocations for a worker
    pub fn get_invocations(&self, worker_id: &str) -> Vec<Vec<u8>> {
        let invocations_map = self.invocations.lock().unwrap();
        invocations_map.get(worker_id).cloned().unwrap_or_default()
    }

    /// Clear recorded invocations for a worker
    pub fn clear_invocations(&self, worker_id: &str) {
        let mut invocations_map = self.invocations.lock().unwrap();
        invocations_map.remove(worker_id);
    }
    pub fn clear_all_invocations(&self) {
        self.invocations.lock().unwrap().clear();
    }

    /// Get all recorded HTTP invocations for a worker
    pub fn get_http_invocations(&self, worker_id: &str) -> Vec<ProtoHttpRequest> {
        let invocations_map = self.http_invocations.lock().unwrap();
        invocations_map.get(worker_id).cloned().unwrap_or_default()
    }

    /// Clear recorded HTTP invocations for a worker
    pub fn clear_http_invocations(&self, worker_id: &str) {
        let mut invocations_map = self.http_invocations.lock().unwrap();
        invocations_map.remove(worker_id);
    }

    /// Get the current worker state by ID
    pub fn get_worker(&self, id: &str) -> Option<(WorkerStatus, Vec<u8>)> {
        let workers = self.workers.lock().unwrap();
        workers.get(id).map(|w| (w.status, w.code.clone()))
    }

    /// Add a pre-existing worker (useful for test setup)
    pub fn add_worker(
        &self,
        id: String,
        code: Vec<u8>,
        status: WorkerStatus,
        logs: String,
        trusted_config: TrustedConfig,
        untrusted_config: UntrustedConfig,
    ) {
        let mut workers = self.workers.lock().unwrap();
        workers.insert(
            id.clone(),
            WorkerInfo {
                id,
                code,
                status,
                logs,
                trusted_config,
                untrusted_config,
            },
        );
    }

    /// Append logs to a worker
    pub fn append_logs(&self, id: &str, logs: &str) -> Result<(), ClientError> {
        let mut workers = self.workers.lock().unwrap();
        if let Some(worker) = workers.get_mut(id) {
            worker.logs.push_str(logs);
            Ok(())
        } else {
            Err(ClientError::Grpc(Status::not_found(format!(
                "Worker '{}' not found",
                id
            ))))
        }
    }

    /// Set worker status
    pub fn set_worker_status(&self, id: &str, status: WorkerStatus) -> Result<(), ClientError> {
        let mut workers = self.workers.lock().unwrap();
        if let Some(worker) = workers.get_mut(id) {
            worker.status = status;
            Ok(())
        } else {
            Err(ClientError::Grpc(Status::not_found(format!(
                "Worker '{}' not found",
                id
            ))))
        }
    }

    /// Clear all workers (reset state)
    pub fn clear_workers(&self) {
        let mut workers = self.workers.lock().unwrap();
        workers.clear();
    }

    /// Internal method to check if next operation should fail
    fn check_failure(&self) -> Result<(), ClientError> {
        let mut fail = self.fail_next_operation.lock().unwrap();
        if let Some(error) = fail.take() {
            return Err(ClientError::Grpc(Status::internal(error)));
        }
        Ok(())
    }

    /// Get the configuration details of a worker instance.
    pub fn get_worker_config_details(
        &self,
        instance_id: &str,
    ) -> Result<(WorkerStatus, Vec<u8>, UntrustedConfig, TrustedConfig), ClientError> {
        // Consistent with other getters, include check_failure to allow simulating errors.
        if let Err(e) = self.check_failure() {
            return Err(e);
        }

        debug!(
            "MockVmServiceClient: Attempting to get config details for worker instance '{}'",
            instance_id
        );

        let workers_guard = self.workers.lock().unwrap();
        if let Some(worker_info) = workers_guard.get(instance_id) {
            // Assuming TrustedConfig and UntrustedConfig implement Debug for logging.
            // If not, you might need to adjust the log message.
            info!(
                "MockVmServiceClient: Found config details for worker instance '{}'. Status: \
                 {:?}, Code_len: {}, UntrustedConfig present: {}, TrustedConfig present: {}",
                instance_id,
                worker_info.status,
                worker_info.code.len(),
                true, // Placeholder, or log parts of the config if small
                true  // Placeholder
            );
            Ok((
                worker_info.status.clone(), // Enums generated by Prost are Clone+Copy, but .clone() is safe.
                worker_info.code.clone(),
                worker_info.untrusted_config.clone(),
                worker_info.trusted_config.clone(),
            ))
        } else {
            warn!(
                "MockVmServiceClient: Worker instance '{}' not found when trying to get config \
                 details.",
                instance_id
            );
            Err(ClientError::Grpc(tonic::Status::not_found(format!(
                "Worker instance '{}' not found (for config details)",
                instance_id
            ))))
        }
    }
}

impl VmClient for MockVmServiceClient {
    async fn start_worker(
        &mut self,
        worker_id: String,
        worker_code: Vec<u8>,
        untrusted_config: UntrustedConfig,
        trusted_config: TrustedConfig,
    ) -> Result<String, ClientError> {
        self.check_failure()?;

        debug!("MockVmServiceClient: Starting worker '{}'", worker_id);
        let instance_id = {
            let mut next_id = self.next_instance_id.lock().unwrap();
            let id = format!("instance-{}-{}", worker_id, *next_id);
            *next_id += 1;
            id
        };

        let worker_info = WorkerInfo {
            id: instance_id.clone(),
            code: worker_code,
            status: WorkerStatus::Running,
            logs: format!("Worker '{}' started\n", instance_id),
            trusted_config,
            untrusted_config,
        };

        let mut workers = self.workers.lock().unwrap();
        workers.insert(instance_id.clone(), worker_info);

        info!("MockVmServiceClient: Started worker '{}'", instance_id);
        Ok(instance_id)
    }

    async fn stop_worker(&mut self, id: String) -> Result<(), ClientError> {
        self.check_failure()?;

        debug!("MockVmServiceClient: Stopping worker '{}'", id);
        let mut workers = self.workers.lock().unwrap();

        if workers.remove(&id).is_some() {
            // Also remove any custom execution behavior for this worker
            let mut behaviors = self.execution_behavior.lock().unwrap();
            behaviors.remove(&id);

            info!("MockVmServiceClient: Stopped worker '{}'", id);
            Ok(())
        } else {
            Err(ClientError::Grpc(Status::not_found(format!(
                "Worker '{}' not found",
                id
            ))))
        }
    }

    async fn invoke_worker(
        &mut self,
        id: String,
        _handler_name: String, // Mock doesn't use handler_name for now
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, ClientError> {
        self.check_failure()?;

        debug!("MockVmServiceClient: Invoking worker '{}'", id);
        let mut workers = self.workers.lock().unwrap();

        if let Some(worker) = workers.get_mut(&id) {
            // Only running workers can be invoked
            if worker.status != WorkerStatus::Running {
                return Err(ClientError::Grpc(Status::failed_precondition(format!(
                    "Worker '{}' is not in RUNNING state (current state: {:?})",
                    id, worker.status
                ))));
            }

            // Append to logs
            worker.logs.push_str(&format!(
                "Invoked with payload of {} bytes\n",
                payload.len()
            ));

            // Record the invocation
            {
                let mut invocations_map = self.invocations.lock().unwrap();
                invocations_map
                    .entry(id.clone())
                    .or_default()
                    .push(payload.clone());
            }

            // Get the execution behavior for this worker
            let behaviors = self.execution_behavior.lock().unwrap();
            let default_behavior = self.default_execution_behavior.lock().unwrap();

            let behavior = behaviors.get(&id).unwrap_or(&default_behavior);

            // Process according to the configured behavior
            let result = match behavior {
                MockExecutionBehavior::Echo => {
                    // Echo back the payload (default behavior)
                    Ok(payload)
                }
                MockExecutionBehavior::Fixed(response) => {
                    // Return the fixed response
                    Ok(response.clone())
                }
                MockExecutionBehavior::Transform(transform_fn) => {
                    // Apply the transformation function
                    Ok(transform_fn(payload))
                }
                MockExecutionBehavior::Error(error_msg) => {
                    // Return an error
                    Err(ClientError::Grpc(Status::internal(error_msg.clone())))
                }
                MockExecutionBehavior::HttpResponse(_) => {
                    // This behavior is for invoke_http, not invoke_worker
                    Err(ClientError::Grpc(Status::unimplemented(
                        "HttpResponse behavior is for invoke_http, not invoke_worker".to_string(),
                    )))
                }
            };

            if let Ok(response) = &result {
                info!(
                    "MockVmServiceClient: Successfully invoked worker '{}', returning {} bytes",
                    id,
                    response.len()
                );
            } else {
                info!("MockVmServiceClient: Worker '{}' invocation failed", id);
            }

            result
        } else {
            Err(ClientError::Grpc(Status::not_found(format!(
                "Worker '{}' not found",
                id
            ))))
        }
    }

    async fn invoke_http(
        &mut self,
        id: String,
        request: ProtoHttpRequest,
    ) -> Result<ProtoHttpResponse, ClientError> {
        self.check_failure()?;

        debug!("MockVmServiceClient: Invoking HTTP for worker '{}'", id);
        let mut workers = self.workers.lock().unwrap();

        if let Some(worker) = workers.get_mut(&id) {
            if worker.status != WorkerStatus::Running {
                return Err(ClientError::Grpc(Status::failed_precondition(format!(
                    "Worker '{}' is not in RUNNING state (current state: {:?})",
                    id, worker.status
                ))));
            }

            worker.logs.push_str(&format!(
                "HTTP Invoked with method {} uri {}\n",
                request.method, request.uri
            ));

            {
                let mut http_invocations_map = self.http_invocations.lock().unwrap();
                http_invocations_map
                    .entry(id.clone())
                    .or_default()
                    .push(request.clone());
            }

            let behaviors = self.execution_behavior.lock().unwrap();
            let default_behavior = self.default_execution_behavior.lock().unwrap();
            let behavior = behaviors.get(&id).unwrap_or(&default_behavior);

            match behavior {
                MockExecutionBehavior::HttpResponse(response) => Ok(response.clone()),
                MockExecutionBehavior::Error(error_msg) => {
                    Err(ClientError::Grpc(Status::internal(error_msg.clone())))
                }
                _ => {
                    // Default simple HTTP response for other behaviors or if not specifically HttpResponse
                    Ok(ProtoHttpResponse {
                        status_code: 200,
                        headers: vec![ProtoHeader {
                            key: "content-type".to_string(),
                            value: b"text/plain".to_vec(),
                        }],
                        body: format!("Mock HTTP response for {}", id).into_bytes(),
                    })
                }
            }
        } else {
            Err(ClientError::Grpc(Status::not_found(format!(
                "Worker '{}' not found",
                id
            ))))
        }
    }

    async fn get_attestation(
        &mut self,
        user_data: Vec<u8>,
    ) -> Result<AttestationReport, ClientError> {
        self.check_failure()?;

        debug!("MockVmServiceClient: Getting attestation report");

        match &*self.attestation_behavior.lock().unwrap() {
            MockAttestationBehavior::Standard => {
                // Create a standardized attestation for testing
                Ok(AttestationReport {
                    user_data,
                    ephemeral_public_key: vec![1, 2, 3, 4, 5],
                    measurement: vec![0u8; 32],
                    block_hashes: vec![vec![10, 11, 12], vec![13, 14, 15]],
                })
            }
            MockAttestationBehavior::Custom(report) => {
                // Return the custom report but replace user_data with the input
                let mut custom_report = report.clone();
                custom_report.user_data = user_data;
                Ok(custom_report)
            }
            MockAttestationBehavior::Error(message) => {
                Err(ClientError::Grpc(Status::internal(message.clone())))
            }
        }
    }

    async fn get_worker_status(&mut self, id: String) -> Result<WorkerStatus, ClientError> {
        self.check_failure()?;

        debug!("MockVmServiceClient: Getting status for worker '{}'", id);
        let workers = self.workers.lock().unwrap();

        if let Some(worker) = workers.get(&id) {
            info!(
                "MockVmServiceClient: Worker '{}' status is {:?}",
                id, worker.status
            );
            Ok(worker.status)
        } else {
            Err(ClientError::Grpc(Status::not_found(format!(
                "Worker '{}' not found",
                id
            ))))
        }
    }

    async fn list_running_workers(&mut self) -> Result<Vec<String>, ClientError> {
        self.check_failure()?;

        debug!("MockVmServiceClient: Listing running workers");
        let workers = self.workers.lock().unwrap();

        // Get IDs of all workers
        let ids: Vec<String> = workers
            .iter()
            .filter(|(_, w)| w.status == WorkerStatus::Running)
            .map(|(id, _)| id.clone())
            .collect();

        info!("MockVmServiceClient: Found {} running workers", ids.len());
        Ok(ids)
    }

    async fn get_worker_logs(&mut self, id: String) -> Result<String, ClientError> {
        self.check_failure()?;

        debug!("MockVmServiceClient: Getting logs for worker '{}'", id);
        let workers = self.workers.lock().unwrap();

        if let Some(worker) = workers.get(&id) {
            info!(
                "MockVmServiceClient: Retrieved {} bytes of logs for worker '{}'",
                worker.logs.len(),
                id
            );
            Ok(worker.logs.clone())
        } else {
            Err(ClientError::Grpc(Status::not_found(format!(
                "Worker '{}' not found",
                id
            ))))
        }
    }
}
