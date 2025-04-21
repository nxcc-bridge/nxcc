use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use http_body_util::Full;
use hyper::{Method, StatusCode, Uri};
use hyper_util::client::legacy::Client;
use hyperlocal::{UnixClientExt, UnixConnector};
use nxcc_interface::{
    proto::vm::{TrustedConfig, UntrustedConfig, WorkerStatus},
    types::AttestationReport,
};
use nxcc_vm_base::server::{VmError, VmRuntime};
use tempfile::{TempDir, tempdir};
use tokio::{
    io::AsyncWriteExt,
    process::{Child, Command},
    sync::Mutex,
};
use tracing::{debug, error, info, warn};

use crate::{
    config_builder::{CodeType, build_config, detect_code_type},
    errors::WorkerdVmError,
};

const WORKERD_BINARY_PATH: &str = "workerd"; // TODO: Make this configurable or discoverable
const DEFAULT_COMPATIBILITY_DATE: &str = "2024-07-01"; // Example date
const STATUS_RETENTION_DURATION: Duration = Duration::from_secs(60); // Keep dead worker status for 1 min
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10); // Max time to wait for UDS

/// Information about a managed workerd instance.
#[derive(Debug)]
struct WorkerInfo {
    instance_id: String,
    process: Child,
    status: WorkerStatus,
    uds_path: PathBuf,
    config_path: PathBuf,
    temp_dir: Arc<TempDir>, // Keep temp dir alive until worker is dropped
    exit_status: Option<ExitStatus>,
    last_seen: Instant, // For cleaning up old ERROR/STOPPED states
    code_type: CodeType,
}

/// Manages workerd processes based on the VmRuntime trait.
#[derive(Clone)]
pub struct WorkerdVmm {
    workers: Arc<Mutex<HashMap<String, Arc<Mutex<WorkerInfo>>>>>,
}

impl WorkerdVmm {
    pub fn new() -> Self {
        WorkerdVmm {
            workers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Spawns a background task to monitor a worker process.
    fn monitor_worker(
        worker_info_lock: Arc<Mutex<WorkerInfo>>,
        workers_map_lock: Arc<Mutex<HashMap<String, Arc<Mutex<WorkerInfo>>>>>,
    ) {
        tokio::spawn(async move {
            let instance_id = worker_info_lock.lock().await.instance_id.clone();
            let mut process = {
                // Take ownership of the process handle inside the lock, then release lock
                let mut info = worker_info_lock.lock().await;
                // We need to extract the process handle to wait on it without holding the lock
                std::mem::replace(
                    &mut info.process,
                    // Create a dummy child process that will immediately exit.
                    // This is a workaround because Child doesn't implement Default or Clone.
                    // The original handle is now owned by this task.
                    Command::new("sh")
                        .arg("-c")
                        .arg("exit 0")
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                        .expect("Failed to spawn dummy process"),
                )
            };

            debug!(instance_id, "Monitoring worker process...");

            match process.wait().await {
                Ok(exit_status) => {
                    let mut info = worker_info_lock.lock().await;
                    info.exit_status = Some(exit_status);
                    info.last_seen = Instant::now();
                    if info.status == WorkerStatus::Running || info.status == WorkerStatus::Starting
                    {
                        if exit_status.success() {
                            info.status = WorkerStatus::Stopped;
                            info!(instance_id, "Worker process exited successfully.");
                        } else {
                            info.status = WorkerStatus::Error;
                            warn!(
                                instance_id,
                                "Worker process exited with error: {}", exit_status
                            );
                        }
                    } else {
                        // If it was already STOPPING, mark as STOPPED
                        info.status = WorkerStatus::Stopped;
                        debug!(instance_id, "Worker process exited after stop request.");
                    }
                }
                Err(e) => {
                    error!(instance_id, "Failed to wait for worker process: {}", e);
                    let mut info = worker_info_lock.lock().await;
                    info.status = WorkerStatus::Error;
                    info.exit_status = None; // Unknown exit status
                    info.last_seen = Instant::now();
                }
            }

            // Schedule cleanup after retention period
            let cleanup_workers_map_lock = workers_map_lock.clone();
            tokio::spawn(async move {
                tokio::time::sleep(STATUS_RETENTION_DURATION).await;
                let mut workers = cleanup_workers_map_lock.lock().await;
                // Clone the worker arc before accessing it to avoid borrowing issues
                let worker_arc = match workers.get(&instance_id) {
                    Some(arc) => arc.clone(),
                    None => return, // Worker already removed
                };

                // Now check status outside of the workers lock
                let status = {
                    let info = worker_arc.lock().await;
                    info.status
                };

                // Only remove if it's still in a terminal state
                if matches!(status, WorkerStatus::Stopped | WorkerStatus::Error) {
                    info!(instance_id, "Removing worker state after retention period.");
                    workers.remove(&instance_id);
                }
            });
        });
    }

    /// Checks if the UDS socket is ready for connections.
    async fn check_uds_ready(uds_path: &Path, timeout_duration: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout_duration {
            if tokio::net::UnixStream::connect(uds_path).await.is_ok() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }

    /// Sends an invocation request to the worker via UDS.
    async fn invoke_via_uds(
        &self,
        uds_path: &Path,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, WorkerdVmError> {
        // Use hyperlocal's UnixClientExt to create a client that works with unix sockets
        let client: Client<UnixConnector, Full<hyper::body::Bytes>> = Client::unix();

        // Build the request with the payload
        let req = hyper::Request::builder()
            .method(Method::POST)
            .uri(hyperlocal::Uri::new(uds_path, "/"))
            .header("Content-Type", "application/octet-stream")
            .body(Full::new(hyper::body::Bytes::from(payload)))
            .map_err(|e| WorkerdVmError::UdsCommunicationFailed {
                path: uds_path.to_path_buf(),
                source: Box::new(e),
            })?;

        debug!("Sending invocation to UDS: {}", uds_path.display());

        // Send the request
        let response =
            client
                .request(req)
                .await
                .map_err(|e| WorkerdVmError::UdsCommunicationFailed {
                    path: uds_path.to_path_buf(),
                    source: Box::new(e),
                })?;

        let status = response.status();

        // Use http_body_util to collect the entire response body
        let body_bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .map_err(|e| WorkerdVmError::UdsCommunicationFailed {
                path: uds_path.to_path_buf(),
                source: Box::new(e),
            })?
            .to_bytes();

        if status != StatusCode::OK {
            let body_string = String::from_utf8_lossy(&body_bytes).to_string();
            error!(
                "Worker invocation failed with status {}: {}",
                status, body_string
            );
            return Err(WorkerdVmError::InvocationHttpFailed {
                status: status.as_u16(),
                body: body_string,
            });
        }

        debug!(
            "Received successful invocation response, size: {}",
            body_bytes.len()
        );
        Ok(body_bytes.to_vec())
    }
}

#[async_trait]
impl VmRuntime for WorkerdVmm {
    async fn start_worker(
        &self,
        worker_code: Vec<u8>,
        untrusted_config: UntrustedConfig,
        trusted_config: TrustedConfig,
    ) -> Result<String, VmError> {
        // 1. Detect code type
        let code_type = detect_code_type(&worker_code)?;

        // 2. Prepare for config generation
        let temp_dir = Arc::new(tempdir().map_err(WorkerdVmError::TempDirCreationFailed)?);
        let uds_file_name = format!("worker-{}.sock", rand::random::<u64>());
        let uds_path = temp_dir.path().join(uds_file_name);
        let config_file_name = "config.capnp.bin".to_string();
        let config_path = temp_dir.path().join(&config_file_name);

        // 3. Build Cap'n Proto config
        let config_bytes = build_config(
            &worker_code,
            code_type,
            &untrusted_config,
            &trusted_config,
            &uds_path,
            "main",          // Service name
            "invoke_socket", // Socket name
        )?;

        // 4. Set the instance ID
        let instance_id = uuid::Uuid::new_v4().to_string(); // TODO: get this from the manifest hash

        // 5. Check for existing worker
        let mut workers = self.workers.lock().await;
        if let Some(existing_worker_lock) = workers.get(&instance_id) {
            let mut existing_worker = existing_worker_lock.lock().await;
            match existing_worker.status {
                WorkerStatus::Starting | WorkerStatus::Running => {
                    info!(
                        instance_id,
                        "Reusing existing {} worker instance.",
                        existing_worker.status.as_str_name()
                    );
                    existing_worker.last_seen = Instant::now(); // Mark as recently used
                    return Ok(instance_id);
                }
                WorkerStatus::Stopping | WorkerStatus::Stopped | WorkerStatus::Error => {
                    info!(
                        instance_id,
                        "Found existing worker in {} state, will replace.",
                        existing_worker.status.as_str_name()
                    );
                    // Allow replacement by falling through
                }
                WorkerStatus::Unspecified => {
                    warn!(
                        instance_id,
                        "Found existing worker in unspecified state, attempting to replace."
                    );
                    // Allow replacement
                }
            }
        }

        // 6. Write config to file
        {
            let mut file = tokio::fs::File::create(&config_path)
                .await
                .map_err(WorkerdVmError::ConfigFileWriteFailed)?;
            file.write_all(&config_bytes)
                .await
                .map_err(WorkerdVmError::ConfigFileWriteFailed)?;
            file.flush()
                .await
                .map_err(WorkerdVmError::ConfigFileWriteFailed)?;
            debug!(instance_id, "Wrote config to {}", config_path.display());
        }

        // 7. Spawn workerd process
        // Ensure workerd binary exists and is executable
        // TODO: Add proper path finding/configuration
        if Command::new(WORKERD_BINARY_PATH)
            .arg("--version")
            .output()
            .await
            .is_err()
        {
            return Err(WorkerdVmError::WorkerdBinaryNotFound.into());
        }

        let mut command = Command::new(WORKERD_BINARY_PATH);
        command
            .arg("serve")
            .arg(&config_path)
            // .arg("--verbose") // Optional: for debugging workerd startup
            .kill_on_drop(true) // Ensure process is killed if VMM drops unexpectedly
            .stdin(std::process::Stdio::null()) // Don't inherit stdin
            .stdout(std::process::Stdio::piped()) // Capture stdout/stderr if logging needed
            .stderr(std::process::Stdio::piped());

        info!(
            instance_id,
            "Spawning workerd process with config: {}",
            config_path.display()
        );
        let child_process = command
            .spawn()
            .map_err(WorkerdVmError::ProcessStartFailed)?;

        // TODO: Capture stdout/stderr for GetWorkerLogs if needed

        // 8. Store worker info
        let worker_info = WorkerInfo {
            instance_id: instance_id.clone(),
            process: child_process,
            status: WorkerStatus::Starting,
            uds_path: uds_path.clone(),
            config_path,
            temp_dir,
            exit_status: None,
            last_seen: Instant::now(),
            code_type,
        };
        let worker_info_lock = Arc::new(Mutex::new(worker_info));

        // Insert into map before starting monitor to avoid race conditions
        workers.insert(instance_id.clone(), worker_info_lock.clone());
        // Release map lock before potentially long-running operations
        drop(workers);

        // 9. Monitor process and check startup
        Self::monitor_worker(worker_info_lock.clone(), self.workers.clone());

        // Check if UDS becomes available within timeout
        if Self::check_uds_ready(&uds_path, STARTUP_TIMEOUT).await {
            let mut info = worker_info_lock.lock().await;
            if info.status == WorkerStatus::Starting {
                // Check if it didn't crash immediately
                info.status = WorkerStatus::Running;
                info.last_seen = Instant::now();
                info!(instance_id, "Worker transitioned to RUNNING state.");
            } else {
                warn!(
                    instance_id,
                    "Worker UDS was ready but status was already {:?}, likely crashed.",
                    info.status
                );
            }
        } else {
            error!(
                instance_id,
                "Worker UDS did not become ready within timeout."
            );
            let mut info = worker_info_lock.lock().await;
            // If monitor hasn't already set it to Error, set it now.
            if info.status == WorkerStatus::Starting {
                info.status = WorkerStatus::Error;
                info.last_seen = Instant::now();
            }
            // Attempt to kill the potentially lingering process
            if let Err(e) = info.process.start_kill() {
                error!(
                    instance_id,
                    "Failed to kill unresponsive worker process: {}", e
                );
            }
            return Err(VmError::new(format!(
                "Worker {} failed to start within timeout",
                instance_id
            )));
        }

        Ok(instance_id)
    }

    async fn stop_worker(&self, id: String) -> Result<(), VmError> {
        let workers = self.workers.lock().await;
        let worker_lock = workers
            .get(&id)
            .cloned()
            .ok_or_else(|| WorkerdVmError::WorkerNotFound(id.clone()))?;
        // Release map lock
        drop(workers);

        let mut worker = worker_lock.lock().await;
        match worker.status {
            WorkerStatus::Starting | WorkerStatus::Running => {
                info!(instance_id = id, "Sending termination signal to worker.");
                worker.status = WorkerStatus::Stopping;
                worker.last_seen = Instant::now();
                // SIGTERM is the default for kill()
                if let Err(e) = worker.process.start_kill() {
                    error!(instance_id = id, "Failed to send kill signal: {}", e);
                    // Mark as error if kill fails, monitor task will handle final state
                    worker.status = WorkerStatus::Error;
                    return Err(WorkerdVmError::Internal(format!(
                        "Failed to signal worker {}: {}",
                        id, e
                    ))
                    .into());
                }
                // Monitor task will update to STOPPED when process exits
                Ok(())
            }
            WorkerStatus::Stopping | WorkerStatus::Stopped | WorkerStatus::Error => {
                warn!(
                    instance_id = id,
                    "StopWorker called on worker in state {:?}, ignoring.", worker.status
                );
                Ok(()) // Idempotent stop
            }
            WorkerStatus::Unspecified => {
                warn!(
                    instance_id = id,
                    "StopWorker called on worker in unspecified state."
                );
                Err(WorkerdVmError::WorkerNotRunnable(worker.status).into())
            }
        }
    }

    async fn invoke_worker(&self, id: String, payload: Vec<u8>) -> Result<Vec<u8>, VmError> {
        let workers = self.workers.lock().await;
        let worker_lock = workers
            .get(&id)
            .cloned()
            .ok_or_else(|| WorkerdVmError::WorkerNotFound(id.clone()))?;
        // Release map lock
        drop(workers);

        let uds_path = {
            // Lock worker briefly to get path and check status
            let worker = worker_lock.lock().await;
            if worker.status != WorkerStatus::Running {
                error!(
                    instance_id = id,
                    "Attempted to invoke worker in non-running state: {:?}", worker.status
                );
                return Err(WorkerdVmError::WorkerNotRunnable(worker.status).into());
            }
            worker.uds_path.clone()
        }; // Release worker lock

        self.invoke_via_uds(&uds_path, payload).await.map_err(|e| {
            // Add context if the error is UdsCommunicationFailed without a path
            if let WorkerdVmError::UdsCommunicationFailed { path, source } = e {
                if path == PathBuf::from("unknown") {
                    WorkerdVmError::UdsCommunicationFailed {
                        path: uds_path,
                        source,
                    }
                    .into()
                } else {
                    WorkerdVmError::UdsCommunicationFailed { path, source }.into()
                }
            } else {
                e.into()
            }
        })
    }

    async fn get_attestation(&self, _user_data: Vec<u8>) -> Result<AttestationReport, VmError> {
        // workerd does not run in a TEE or provide standard attestation reports.
        Err(WorkerdVmError::AttestationNotSupported.into())
    }

    async fn get_worker_status(&self, id: String) -> Result<WorkerStatus, VmError> {
        let workers = self.workers.lock().await;
        if let Some(worker_lock) = workers.get(&id) {
            // Clone the lock to avoid borrowing issues
            let worker_lock = worker_lock.clone();
            drop(workers); // Release the workers map lock

            let mut worker = worker_lock.lock().await;
            worker.last_seen = Instant::now(); // Mark as recently checked
            let current_status = worker.status;

            // Now we can check for cleanup and return status
            Ok(current_status)
        } else {
            // Check if it *was* present but got cleaned up (implies Stopped or Error)
            // This requires more state tracking (e.g., a separate "recently_cleaned" set)
            // For now, just return not found if not in the main map.
            Err(WorkerdVmError::WorkerNotFound(id).into())
        }
    }

    async fn list_running_workers(&self) -> Result<Vec<String>, VmError> {
        let workers = self.workers.lock().await;
        let mut running_ids = Vec::new();
        for (id, worker_lock) in workers.iter() {
            let worker = worker_lock.lock().await;
            if worker.status == WorkerStatus::Running {
                running_ids.push(id.clone());
            }
        }
        Ok(running_ids)
    }

    async fn get_worker_logs(&self, id: String) -> Result<String, VmError> {
        // TODO: Implement log capture from child process stdout/stderr.
        // This requires handling the piped streams in the monitor task or elsewhere.
        // For now, return not implemented or empty.
        warn!(instance_id = id, "GetWorkerLogs is not fully implemented.");
        let workers = self.workers.lock().await;
        if workers.contains_key(&id) {
            Ok(format!(
                "Logs for worker {} are not currently captured.",
                id
            ))
        } else {
            Err(WorkerdVmError::WorkerNotFound(id).into())
        }
    }
}
