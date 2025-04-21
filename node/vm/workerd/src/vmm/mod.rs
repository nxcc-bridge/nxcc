use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use http_body_util::BodyExt;
use hyper::{Method, Request, StatusCode, body::Bytes};
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use hyperlocal::UnixConnector;
use nxcc_interface::{
    proto::vm::{TrustedConfig, UntrustedConfig, WorkerStatus},
    types::AttestationReport,
};
use nxcc_vm_base::server::{VmError, VmRuntime};
use tempfile::TempDir;
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::Mutex,
    time::sleep,
};
use tracing::{debug, error, info, warn};

use crate::{
    config_builder::{CodeType, build_config, detect_code_type},
    errors::WorkerdVmError,
};

const WORKERD_BINARY_PATH: &str = "workerd";
/// How long we wait for the worker's UDS to become connectable before timing out.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
/// Poll interval for checking UDS readiness during startup.
const UDS_CHECK_INTERVAL: Duration = Duration::from_millis(100);

/// Holds information about a single worker process.
#[derive(Debug)]
struct WorkerInfo {
    instance_id: String,
    process: Child,
    pid: u32,
    status: WorkerStatus,
    uds_path: PathBuf,
    config_path: PathBuf,
    temp_dir: Arc<TempDir>,
    code_type: CodeType,
    logs: Arc<Mutex<String>>,
}

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

    /// Reads lines from `reader` and appends them to the shared `logs` buffer,
    /// also logging them via `tracing`.
    async fn log_stream<R>(
        mut reader: BufReader<R>,
        instance_id: String,
        output_type: &str,
        logs: Arc<Mutex<String>>,
    ) where
        R: AsyncReadExt + Unpin,
    {
        let mut line = String::new();
        while let Ok(bytes_read) = reader.read_line(&mut line).await {
            if bytes_read == 0 {
                break;
            }
            {
                let mut logs_guard = logs.lock().await;
                logs_guard.push_str(&format!("{}: {}", output_type, line));
            }
            match output_type {
                "stdout" => info!(?instance_id, "stdout: {}", line.trim()),
                "stderr" => error!(?instance_id, "stderr: {}", line.trim()),
                _ => debug!(?instance_id, "{} = {}", output_type, line.trim()),
            }
            line.clear();
        }
    }

    /// Makes an HTTP invocation via a Unix-domain socket.
    async fn invoke_via_uds(
        &self,
        instance_id: &str, // <-- New parameter
        uds_path: &Path,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, WorkerdVmError> {
        let client: Client<UnixConnector, http_body_util::Full<Bytes>> =
            Client::builder(TokioExecutor::new()).build(UnixConnector);

        let req = Request::builder()
            .method(Method::POST)
            .uri(hyperlocal::Uri::new(uds_path, "/"))
            .header("Content-Type", "application/octet-stream")
            .body(http_body_util::Full::new(Bytes::from(payload)))
            .map_err(|e| WorkerdVmError::UdsCommunicationFailed {
                path: uds_path.to_path_buf(),
                source: Box::new(e),
            })?;

        debug!("Sending invocation to UDS: {}", uds_path.display());

        let response =
            client
                .request(req)
                .await
                .map_err(|e| WorkerdVmError::UdsCommunicationFailed {
                    path: uds_path.to_path_buf(),
                    source: Box::new(e),
                })?;

        let status = response.status();
        let body_bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|e| WorkerdVmError::UdsCommunicationFailed {
                path: uds_path.to_path_buf(),
                source: Box::new(e),
            })?
            .to_bytes();

        if status != StatusCode::OK {
            let body_string = String::from_utf8_lossy(&body_bytes).to_string();

            // 1) Grab the status (and optionally logs) for this worker
            let worker_status = match self.get_worker_status(instance_id.to_string()).await {
                Ok(s) => s,
                Err(_) => WorkerStatus::Unspecified,
            };

            // 2) Optionally fetch logs as well (if desired)
            let worker_logs = match self.get_worker_logs(instance_id.to_string()).await {
                Ok(logs) => logs,
                Err(_) => "<Could not retrieve logs>".to_string(),
            };

            error!(
                ?instance_id,
                ?worker_status,
                "Worker invocation failed with status {}: {}\nWorker logs:\n{}",
                status,
                body_string,
                worker_logs
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
        let code_type = detect_code_type(&worker_code)?;
        let temp_dir_handle = Arc::new(
            tempfile::Builder::new()
                .prefix("nxcc-workerd-")
                .rand_bytes(5)
                .tempdir()
                .map_err(WorkerdVmError::TempDirCreationFailed)?,
        );
        let base_path = temp_dir_handle.path();
        let instance_id = uuid::Uuid::new_v4().to_string();
        let short_id = &instance_id[..8];
        let uds_file_name = format!("w-{}.sock", short_id);
        let uds_path = base_path.join(&uds_file_name);
        let config_file_name = "config.capnp.bin";
        let config_path = base_path.join(config_file_name);

        info!(
            instance_id = ?instance_id,
            uds = %uds_path.display(),
            config = %config_path.display(),
            "Preparing worker"
        );

        let config_bytes = build_config(
            &worker_code,
            code_type,
            &untrusted_config,
            &trusted_config,
            &uds_path,
            "main",
            "invoke_socket",
        )?;

        // Write the config
        {
            let mut file = File::create(&config_path)
                .await
                .map_err(WorkerdVmError::ConfigFileWriteFailed)?;
            file.write_all(&config_bytes)
                .await
                .map_err(WorkerdVmError::ConfigFileWriteFailed)?;
            file.flush()
                .await
                .map_err(WorkerdVmError::ConfigFileWriteFailed)?;
        }

        // Sanity check for the workerd binary
        if Command::new(WORKERD_BINARY_PATH)
            .arg("--version")
            .output()
            .await
            .is_err()
        {
            error!(
                "workerd binary not found or failed to execute at path: {}",
                WORKERD_BINARY_PATH
            );
            return Err(WorkerdVmError::WorkerdBinaryNotFound.into());
        }

        let mut command = Command::new(WORKERD_BINARY_PATH);
        command
            .arg("serve")
            .arg("-b")
            .arg(&config_path)
            .arg("--verbose")
            .kill_on_drop(true)
            .current_dir(base_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        info!(instance_id = ?instance_id, "Spawning workerd process...");
        let mut child_process = command
            .spawn()
            .map_err(WorkerdVmError::ProcessStartFailed)?;
        let child_pid = child_process.id().unwrap_or(0);
        info!(instance_id = ?instance_id, pid = ?child_pid, "Workerd process spawned.");

        // Capture logs from stdout/stderr in background tasks
        let logs_arc = Arc::new(Mutex::new(String::new()));
        if let Some(stdout) = child_process.stdout.take() {
            let id_clone = instance_id.clone();
            let logs_clone = logs_arc.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                WorkerdVmm::log_stream(reader, id_clone, "stdout", logs_clone).await;
            });
        }
        if let Some(stderr) = child_process.stderr.take() {
            let id_clone = instance_id.clone();
            let logs_clone = logs_arc.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                WorkerdVmm::log_stream(reader, id_clone, "stderr", logs_clone).await;
            });
        }

        // Insert the worker in "Starting" status into the map
        let worker_info = WorkerInfo {
            instance_id: instance_id.clone(),
            process: child_process,
            pid: child_pid,
            status: WorkerStatus::Starting,
            uds_path: uds_path.clone(),
            config_path,
            temp_dir: temp_dir_handle,
            code_type,
            logs: logs_arc.clone(),
        };
        {
            let mut workers_map = self.workers.lock().await;
            workers_map.insert(instance_id.clone(), Arc::new(Mutex::new(worker_info)));
        }

        // Poll for UDS readiness, or time out
        let start_time = Instant::now();
        loop {
            match tokio::net::UnixStream::connect(&uds_path).await {
                Ok(_) => {
                    // Mark the worker as Running and return success
                    let workers_map = self.workers.lock().await;
                    if let Some(worker_lock) = workers_map.get(&instance_id) {
                        let mut worker = worker_lock.lock().await;
                        if worker.status == WorkerStatus::Starting {
                            worker.status = WorkerStatus::Running;
                            info!(instance_id = ?instance_id, "UDS ready, worker is now Running.");
                        }
                    }
                    return Ok(instance_id);
                }
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::NotFound
                        && e.kind() != std::io::ErrorKind::ConnectionRefused
                    {
                        warn!(
                            ?instance_id,
                            uds=%uds_path.display(),
                            "Unexpected error checking UDS: {}. Will retry.", e
                        );
                    }
                }
            }

            if start_time.elapsed() > STARTUP_TIMEOUT {
                // Timed out waiting for UDS. Mark the worker as Error and kill it.
                let workers_map = self.workers.lock().await;
                if let Some(worker_lock) = workers_map.get(&instance_id) {
                    let mut worker = worker_lock.lock().await;
                    if worker.status == WorkerStatus::Starting {
                        worker.status = WorkerStatus::Error;
                        warn!(
                            instance_id = ?worker.instance_id,
                            "Timeout waiting for UDS. Killing process."
                        );
                        let _ = worker.process.kill(); // best-effort kill
                    }
                    let logs_content = worker.logs.lock().await.clone();
                    return Err(WorkerdVmError::StartupTimeout {
                        instance_id: instance_id.clone(),
                        timeout: STARTUP_TIMEOUT,
                        logs: logs_content,
                    }
                    .into());
                }
                // If it's not in the map for some reason, just fail
                return Err(WorkerdVmError::StartupTimeout {
                    instance_id,
                    timeout: STARTUP_TIMEOUT,
                    logs: "[No logs captured]".to_string(),
                }
                .into());
            }

            sleep(UDS_CHECK_INTERVAL).await;
        }
    }

    async fn stop_worker(&self, id: String) -> Result<(), VmError> {
        let worker_lock = {
            let workers_map = self.workers.lock().await;
            workers_map.get(&id).cloned()
        }
        .ok_or_else(|| WorkerdVmError::WorkerNotFound(id.clone()))?;

        let mut worker = worker_lock.lock().await;
        match worker.status {
            WorkerStatus::Starting | WorkerStatus::Running => {
                info!(instance_id = ?id, "Stopping worker.");
                if let Err(e) = worker.process.kill().await {
                    // If the process is already gone, that's not a fatal error
                    if e.kind() != std::io::ErrorKind::InvalidInput {
                        error!(?id, "Failed to kill worker process: {}", e);
                        return Err(WorkerdVmError::Internal(format!(
                            "Failed to kill worker {}: {}",
                            id, e
                        ))
                        .into());
                    }
                }
                // Mark as Stopped
                worker.status = WorkerStatus::Stopped;
                Ok(())
            }
            WorkerStatus::Stopped | WorkerStatus::Error => {
                // Already terminal, do nothing
                debug!(
                    instance_id = ?id,
                    "stop_worker called but worker is already in {:?} state, ignoring.", worker.status
                );
                Ok(())
            }
            // We won't remove Unspecified from the proto, but treat it similarly as an error
            WorkerStatus::Unspecified => {
                error!(
                    ?id,
                    "stop_worker called on a worker with Unspecified status."
                );
                Err(WorkerdVmError::WorkerNotRunnable(WorkerStatus::Unspecified).into())
            }
            WorkerStatus::Stopping => unimplemented!(),
        }
    }

    async fn invoke_worker(&self, id: String, payload: Vec<u8>) -> Result<Vec<u8>, VmError> {
        let worker_lock = {
            let workers = self.workers.lock().await;
            workers.get(&id).cloned()
        }
        .ok_or_else(|| WorkerdVmError::WorkerNotFound(id.clone()))?;

        // Check status and get UDS path
        let uds_path = {
            let worker = worker_lock.lock().await;
            if worker.status != WorkerStatus::Running {
                error!(
                    instance_id = ?id,
                    "Attempted to invoke worker in non-running state: {:?}", worker.status
                );
                return Err(WorkerdVmError::WorkerNotRunnable(worker.status).into());
            }
            worker.uds_path.clone()
        };

        self.invoke_via_uds(&id, &uds_path, payload)
            .await
            .map_err(|e| {
                error!(instance_id = ?id, "Invocation via UDS failed: {}", e);
                e.into()
            })
    }

    async fn get_attestation(&self, _user_data: Vec<u8>) -> Result<AttestationReport, VmError> {
        // Not supported in this sample
        Err(WorkerdVmError::AttestationNotSupported.into())
    }

    async fn get_worker_status(&self, id: String) -> Result<WorkerStatus, VmError> {
        let worker_lock = {
            let workers = self.workers.lock().await;
            workers.get(&id).cloned()
        }
        .ok_or_else(|| WorkerdVmError::WorkerNotFound(id))?;

        let worker = worker_lock.lock().await;
        Ok(worker.status)
    }

    async fn list_running_workers(&self) -> Result<Vec<String>, VmError> {
        let workers_map = self.workers.lock().await;
        let mut running_ids = Vec::with_capacity(workers_map.len());
        for (id, worker_lock) in workers_map.iter() {
            let worker = worker_lock.lock().await;
            if worker.status == WorkerStatus::Running {
                running_ids.push(id.clone());
            }
        }
        Ok(running_ids)
    }

    async fn get_worker_logs(&self, id: String) -> Result<String, VmError> {
        let worker_lock = {
            let workers = self.workers.lock().await;
            workers.get(&id).cloned()
        }
        .ok_or_else(|| WorkerdVmError::WorkerNotFound(id))?;

        let worker = worker_lock.lock().await;
        // Grab a clone of the logs
        let logs_content = worker.logs.lock().await.clone();
        Ok(logs_content)
    }
}

#[cfg(test)]
mod tests;
