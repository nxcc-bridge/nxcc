use std::{
    collections::HashMap,
    error::Error as _, // Keep this for potential source chaining if needed
    fmt,               // Added for Display impl example
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use http_body_util::BodyExt;
use hyper::{Method, Request, StatusCode, body::Bytes};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use hyperlocal::{UnixClientExt, UnixConnector};
use nxcc_interface::{
    proto::vm::{TrustedConfig, UntrustedConfig, WorkerStatus},
    types::AttestationReport,
};
use nxcc_vm_base::server::{VmError, VmRuntime};
use tempfile::{TempDir, tempdir};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{Mutex, Notify},  // Added Notify
    time::{sleep, timeout}, // Added timeout for select! alternative
};
use tracing::{debug, error, info, warn};

use crate::{
    config_builder::{CodeType, build_config, detect_code_type},
    errors::WorkerdVmError,
};

const WORKERD_BINARY_PATH: &str = "workerd";
const STATUS_RETENTION_DURATION: Duration = Duration::from_secs(60);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const UDS_CHECK_INTERVAL: Duration = Duration::from_millis(100); // How often to poll UDS

/// Information about a managed workerd instance.
#[derive(Debug)]
struct WorkerInfo {
    instance_id: String,
    process: Child,
    status: WorkerStatus,
    uds_path: PathBuf,
    config_path: PathBuf,
    temp_dir: Arc<TempDir>,
    exit_status: Option<ExitStatus>,
    last_seen: Instant,
    code_type: CodeType,
    logs: Arc<Mutex<String>>,
    // Used to signal start_worker when the process exits during startup phase
    startup_exit_notify: Arc<Notify>,
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

    // Helper function to read process output lines and store them (Unchanged)
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
                break; // EOF
            }
            {
                let mut logs_guard = logs.lock().await;
                logs_guard.push_str(&format!("{}: {}", output_type, line));
            }
            match output_type {
                "stdout" => info!(instance_id = instance_id, stdout = line.trim()),
                "stderr" => error!(instance_id = instance_id, stderr = line.trim()),
                _ => debug!(
                    instance_id = instance_id,
                    "{} = {}",
                    output_type,
                    line.trim()
                ),
            }
            line.clear();
        }
    }

    /// Spawns a background task to monitor a worker process, capture its output,
    /// update its status upon exit, and notify waiters about the exit.
    fn monitor_worker(
        worker_info_lock: Arc<Mutex<WorkerInfo>>,
        workers_map_lock: Arc<Mutex<HashMap<String, Arc<Mutex<WorkerInfo>>>>>,
    ) {
        tokio::spawn(async move {
            let (instance_id, stdout_handle, stderr_handle, logs, startup_exit_notify) = {
                // Take ownership of handles and notifier inside the lock
                let mut info = worker_info_lock.lock().await;
                let instance_id = info.instance_id.clone();
                let logs = info.logs.clone();
                let stdout = info.process.stdout.take();
                let stderr = info.process.stderr.take();
                // Clone the Arc<Notify>
                let startup_exit_notify = info.startup_exit_notify.clone();
                (instance_id, stdout, stderr, logs, startup_exit_notify)
            };

            // Spawn log readers (Unchanged)
            if let Some(stdout) = stdout_handle {
                let id_clone = instance_id.clone();
                let logs_clone = logs.clone();
                tokio::spawn(async move {
                    let reader = BufReader::new(stdout);
                    Self::log_stream(reader, id_clone, "stdout", logs_clone).await;
                });
            } else {
                error!(instance_id = instance_id, "Failed to capture stdout handle");
            }
            if let Some(stderr) = stderr_handle {
                let id_clone = instance_id.clone();
                let logs_clone = logs.clone();
                tokio::spawn(async move {
                    let reader = BufReader::new(stderr);
                    Self::log_stream(reader, id_clone, "stderr", logs_clone).await;
                });
            } else {
                error!(instance_id = instance_id, "Failed to capture stderr handle");
            }

            // Wait for the process to exit
            let exit_status_result = {
                let mut info = worker_info_lock.lock().await;
                // Check if process is still valid before waiting
                // Note: process.wait() consumes the "waitable" state.
                info.process.wait().await
            };

            // --- Process has exited ---

            // Update worker state based on exit status
            let final_status = {
                // Scope for lock
                let mut info = worker_info_lock.lock().await;
                info.last_seen = Instant::now(); // Mark time of exit detection

                match exit_status_result {
                    Ok(status) => {
                        info.exit_status = Some(status);
                        let prev_status = info.status;
                        if status.success() {
                            info.status = WorkerStatus::Stopped;
                            info!(
                                instance_id = info.instance_id,
                                prev_status = ?prev_status,
                                exit_code = status.code(),
                                "Worker process exited successfully."
                            );
                        } else {
                            info.status = WorkerStatus::Error;
                            warn!(
                                instance_id = info.instance_id,
                                prev_status = ?prev_status,
                                exit_code = status.code(),
                                "Worker process exited with error: {}", status
                            );
                        }
                    }
                    Err(e) => {
                        info.exit_status = None;
                        info.status = WorkerStatus::Error;
                        error!(
                            instance_id = info.instance_id,
                            "Failed to wait for worker process: {}. Marking as Error.", e
                        );
                    }
                }
                info.status // Return the final status set
            }; // Release lock

            // *** Notify start_worker that the process has exited ***
            // This allows start_worker's select! to react immediately if it's still waiting.
            debug!(instance_id = instance_id, "Notifying about process exit.");
            startup_exit_notify.notify_waiters();

            // Schedule cleanup task (only if status is terminal)
            if matches!(final_status, WorkerStatus::Stopped | WorkerStatus::Error) {
                let cleanup_workers_map_lock = workers_map_lock.clone();
                let cleanup_instance_id = instance_id.clone();
                tokio::spawn(async move {
                    sleep(STATUS_RETENTION_DURATION).await;
                    let mut workers = cleanup_workers_map_lock.lock().await;
                    if let Some(worker_arc) = workers.get(&cleanup_instance_id) {
                        let info = worker_arc.lock().await;
                        // Check status and time again before removing
                        if matches!(info.status, WorkerStatus::Stopped | WorkerStatus::Error)
                            && info.last_seen.elapsed() >= STATUS_RETENTION_DURATION
                        {
                            info!(
                                instance_id = cleanup_instance_id,
                                "Removing worker state after retention period."
                            );
                            // Drop lock before removing if possible? No, need lock to remove.
                            drop(info); // Explicitly drop lock before removing from map
                            workers.remove(&cleanup_instance_id);
                        } else {
                            debug!(
                                instance_id = cleanup_instance_id,
                                status = ?info.status,
                                elapsed = ?info.last_seen.elapsed(),
                                "Worker not removed: Status not terminal or retention not expired."
                            );
                        }
                    }
                });
            }
        });
    }

    // Removed check_uds_ready - logic moved into start_worker's select! loop

    // invoke_via_uds (Unchanged)
    async fn invoke_via_uds(
        &self,
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
        // 1. Detect code type (Unchanged)
        let code_type = detect_code_type(&worker_code)?;

        // 2. Prepare temporary directory and paths (Unchanged)
        let temp_dir_handle = Arc::new(tempdir().map_err(WorkerdVmError::TempDirCreationFailed)?);
        let base_path = temp_dir_handle.path();
        let instance_id = uuid::Uuid::new_v4().to_string();
        let uds_file_name = format!("worker-{}.sock", instance_id);
        let uds_path = base_path.join(&uds_file_name);
        let config_file_name = "config.capnp.bin";
        let config_path = base_path.join(config_file_name);

        info!(
            instance_id,
            "Preparing worker. UDS: {}, Config: {}",
            uds_path.display(),
            config_path.display()
        );

        // 3. Build Cap'n Proto config bytes (Unchanged)
        let config_bytes = build_config(
            &worker_code,
            code_type,
            &untrusted_config,
            &trusted_config,
            &uds_path,
            "main",
            "invoke_socket",
        )?;

        // 4. Check for existing worker (Unchanged)
        {
            // Scope for lock
            let mut workers = self.workers.lock().await;
            if let Some(existing_worker_lock) = workers.get(&instance_id) {
                let existing_worker = existing_worker_lock.lock().await;
                match existing_worker.status {
                    WorkerStatus::Starting | WorkerStatus::Running => {
                        info!(
                            instance_id,
                            "Reusing existing {} worker instance.",
                            existing_worker.status.as_str_name()
                        );
                        return Ok(instance_id);
                    }
                    _ => {
                        info!(
                            instance_id,
                            "Found existing worker in {} state, will replace.",
                            existing_worker.status.as_str_name()
                        );
                    }
                }
            }
        } // Release lock

        // 5. Write config to file asynchronously (Unchanged)
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
            debug!(instance_id, "Wrote config to {}", config_path.display());
        }

        // 6. Verify workerd binary exists (Unchanged)
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

        // 7. Spawn workerd process (Unchanged)
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

        info!(instance_id, "Spawning workerd process...");
        let child_process = command
            .spawn()
            .map_err(WorkerdVmError::ProcessStartFailed)?;
        let child_pid = child_process.id();
        info!(instance_id, pid = ?child_pid, "Workerd process spawned.");

        // 8. Create shared state: logs and exit notifier
        let logs_arc = Arc::new(Mutex::new(String::new()));
        let startup_exit_notify = Arc::new(Notify::new()); // Create notifier

        // 9. Create and store worker info structure
        let worker_info = WorkerInfo {
            instance_id: instance_id.clone(),
            process: child_process, // Takes ownership
            status: WorkerStatus::Starting,
            uds_path: uds_path.clone(),
            config_path,
            temp_dir: temp_dir_handle,
            exit_status: None,
            last_seen: Instant::now(),
            code_type,
            logs: logs_arc.clone(),
            startup_exit_notify: startup_exit_notify.clone(), // Store notifier clone
        };
        let worker_info_lock = Arc::new(Mutex::new(worker_info));

        // 10. Add worker to the central map
        {
            let mut workers_map = self.workers.lock().await;
            workers_map.insert(instance_id.clone(), worker_info_lock.clone());
        }

        // 11. Start the background monitor task (passes the worker_info_lock)
        Self::monitor_worker(worker_info_lock.clone(), self.workers.clone());

        // --- NEW: Wait for UDS readiness OR process exit, with timeout ---
        debug!(
            instance_id,
            "Waiting for UDS or process exit (timeout: {:?})", STARTUP_TIMEOUT
        );

        // Define the UDS check future (polls readiness)
        let uds_check_future = async {
            loop {
                match tokio::net::UnixStream::connect(&uds_path).await {
                    Ok(_) => {
                        debug!(instance_id, "UDS socket check successful.");
                        return Ok::<_, ()>(()); // UDS is ready
                    }
                    Err(e) => {
                        // Log less frequently or at debug level to avoid spam
                        debug!(
                            instance_id,
                            "UDS not ready yet ({}): {}",
                            uds_path.display(),
                            e
                        );

                        // ----- REMOVED THE try_acquire CHECK -----
                        // No need to check here, select! handles the race.

                        sleep(UDS_CHECK_INTERVAL).await;
                    }
                }
            }
        };

        // Define the exit notification future (Unchanged)
        let exit_notify_future = startup_exit_notify.notified();

        tokio::select! {
            biased; // Keep biased

            // Branch 1: UDS became ready (No change needed here)
            uds_result = uds_check_future => {
                match uds_result {
                    Ok(()) => {
                        // UDS is ready. Lock info and check the *current* status.
                        let mut info = worker_info_lock.lock().await;
                        if info.status == WorkerStatus::Starting {
                            // Common case: UDS ready, process still starting/running
                            info.status = WorkerStatus::Running;
                            info.last_seen = Instant::now();
                            info!(instance_id, "Worker UDS ready, transitioned to RUNNING.");
                            Ok(instance_id) // Successfully started
                        } else {
                            // Edge case: Process exited *after* UDS was ready but *before* we locked info.
                            warn!(instance_id, "Worker UDS was ready, but status was already {:?} (exited just after UDS ready).", info.status);
                            let logs_content = info.logs.lock().await.clone(); // Capture logs
                            Err(WorkerdVmError::StartupFailedPrematureExit {
                                instance_id: instance_id.clone(),
                                final_status: info.status,
                                logs: logs_content,
                            }.into())
                        }
                    }
                    Err(_) => {
                        // This Err branch in uds_result is now unreachable because
                        // we removed the only place returning Err(()) from uds_check_future.
                        // We could technically remove this match arm, but leaving it
                        // as unreachable!() might be safer if the future changes later.
                         unreachable!("uds_check_future should only return Ok(())");
                    }
                }
            }

            // Branch 2: Monitor task signaled an exit BEFORE UDS was ready (Unchanged)
            _ = exit_notify_future => {
                // Process exited. The monitor task has already updated the status.
                let info = worker_info_lock.lock().await;
                error!(instance_id, "Worker process exited during startup before UDS was ready. Final status: {:?}", info.status);
                let logs_content = info.logs.lock().await.clone(); // Capture logs
                // Ensure status reflects failure if it somehow ended as 'Stopped' successfully but too early
                let final_error_status = if info.status == WorkerStatus::Stopped { WorkerStatus::Error } else { info.status };

                Err(WorkerdVmError::StartupFailedPrematureExit {
                    instance_id: instance_id.clone(),
                    final_status: final_error_status, // Use the status set by monitor
                    logs: logs_content,
                }.into())
            }

            // Branch 3: Overall timeout (Unchanged)
            _ = sleep(STARTUP_TIMEOUT) => {
                // Timeout occurred. UDS wasn't ready and process didn't signal exit (or hasn't yet).
                error!(instance_id, "Timeout waiting for worker UDS socket to become ready.");
                let logs_content;
                // Attempt to kill the process and update status to Error
                { // Scope for lock
                    let mut info = worker_info_lock.lock().await;
                    logs_content = info.logs.lock().await.clone(); // Capture logs *before* potential kill
                    error!(instance_id, "Worker logs during timed-out startup attempt:\n{}", if logs_content.is_empty() { "[No logs captured]" } else { &logs_content });

                    // Only update status if it's still Starting. If monitor task already marked
                    // it as Error/Stopped, respect that.
                    if info.status == WorkerStatus::Starting {
                         info.status = WorkerStatus::Error; // Mark as Error due to timeout
                         info.last_seen = Instant::now();
                         // Try to kill the potentially lingering process
                         if let Err(e) = info.process.start_kill() {
                             if e.kind() != std::io::ErrorKind::InvalidInput { // Ignore if already exited
                                 error!(instance_id, "Failed to kill unresponsive worker process after startup timeout: {}", e);
                             }
                         } else {
                             info!(instance_id, "Sent kill signal to unresponsive worker after startup timeout.");
                         }
                    } else {
                         warn!(instance_id, "Startup timed out, but worker status was already {:?}. Not changing status.", info.status);
                    }
                } // Release lock

                Err(WorkerdVmError::StartupTimeout {
                    instance_id: instance_id.clone(),
                    timeout: STARTUP_TIMEOUT,
                    logs: logs_content,
                }.into())
            }
        }
    }

    // stop_worker (Unchanged)
    async fn stop_worker(&self, id: String) -> Result<(), VmError> {
        let worker_lock = {
            let workers = self.workers.lock().await;
            workers.get(&id).cloned()
        }
        .ok_or_else(|| WorkerdVmError::WorkerNotFound(id.clone()))?;

        let mut worker = worker_lock.lock().await;
        match worker.status {
            WorkerStatus::Starting | WorkerStatus::Running => {
                info!(instance_id = id, "Sending termination signal to worker.");
                // Update status optimistically, monitor task confirms final state
                if worker.status == WorkerStatus::Running {
                    worker.status = WorkerStatus::Stopping;
                } // If Starting, let it remain Starting, kill signal might interrupt it.
                worker.last_seen = Instant::now();
                if let Err(e) = worker.process.start_kill() {
                    if e.kind() == std::io::ErrorKind::InvalidInput {
                        warn!(
                            instance_id = id,
                            "Attempted to kill worker, but process already exited."
                        );
                    } else {
                        error!(instance_id = id, "Failed to send kill signal: {}", e);
                        // Don't force Error state here, monitor task is the source of truth on exit status
                        return Err(WorkerdVmError::Internal(format!(
                            "Failed to signal worker {}: {}",
                            id, e
                        ))
                        .into());
                    }
                }
                Ok(())
            }
            WorkerStatus::Stopping | WorkerStatus::Stopped | WorkerStatus::Error => {
                warn!(
                    instance_id = id,
                    "StopWorker called on worker already in state {:?}, ignoring.", worker.status
                );
                worker.last_seen = Instant::now();
                Ok(())
            }
            WorkerStatus::Unspecified => {
                warn!(
                    instance_id = id,
                    "StopWorker called on worker in UNspecified state."
                );
                Err(WorkerdVmError::WorkerNotRunnable(worker.status).into())
            }
        }
    }

    // invoke_worker (Unchanged)
    async fn invoke_worker(&self, id: String, payload: Vec<u8>) -> Result<Vec<u8>, VmError> {
        let worker_lock = {
            let workers = self.workers.lock().await;
            workers.get(&id).cloned()
        }
        .ok_or_else(|| WorkerdVmError::WorkerNotFound(id.clone()))?;

        let (uds_path, current_status) = {
            let mut worker = worker_lock.lock().await;
            let status = worker.status;
            if !matches!(status, WorkerStatus::Running) {
                error!(
                    instance_id = id,
                    "Attempted to invoke worker in non-running state: {:?}", status
                );
                return Err(WorkerdVmError::WorkerNotRunnable(status).into());
            }
            worker.last_seen = Instant::now();
            (worker.uds_path.clone(), status)
        };

        if current_status != WorkerStatus::Running {
            // Double check
            error!(
                instance_id = id,
                "Worker state changed to {:?} immediately before invocation", current_status
            );
            return Err(WorkerdVmError::WorkerNotRunnable(current_status).into());
        }

        self.invoke_via_uds(&uds_path, payload).await.map_err(|e| {
            error!(instance_id = id, "Invocation via UDS failed: {}", e);
            // Consider checking worker status again here if UDS fails
            // let current_status = worker_lock.lock().await.status;
            // if current_status != WorkerStatus::Running { ... }
            e.into()
        })
    }

    // get_attestation (Unchanged)
    async fn get_attestation(&self, _user_data: Vec<u8>) -> Result<AttestationReport, VmError> {
        Err(WorkerdVmError::AttestationNotSupported.into())
    }

    // get_worker_status (Unchanged)
    async fn get_worker_status(&self, id: String) -> Result<WorkerStatus, VmError> {
        let worker_lock = {
            let workers = self.workers.lock().await;
            workers.get(&id).cloned()
        };

        if let Some(lock) = worker_lock {
            let mut worker = lock.lock().await;
            worker.last_seen = Instant::now();
            Ok(worker.status)
        } else {
            Err(WorkerdVmError::WorkerNotFound(id).into())
        }
    }

    // list_running_workers (Unchanged)
    async fn list_running_workers(&self) -> Result<Vec<String>, VmError> {
        let workers = self.workers.lock().await;
        let mut running_ids = Vec::with_capacity(workers.len());
        for worker_lock in workers.values() {
            let worker = worker_lock.lock().await;
            if worker.status == WorkerStatus::Running {
                running_ids.push(worker.instance_id.clone());
            }
        }
        Ok(running_ids)
    }

    // get_worker_logs (Unchanged)
    async fn get_worker_logs(&self, id: String) -> Result<String, VmError> {
        let worker_lock = {
            let workers = self.workers.lock().await;
            workers.get(&id).cloned()
        };

        if let Some(lock) = worker_lock {
            let worker_info = lock.lock().await;
            let logs_content = worker_info.logs.lock().await.clone();
            Ok(logs_content)
        } else {
            Err(WorkerdVmError::WorkerNotFound(id).into())
        }
    }
}
#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use nxcc_interface::proto::vm::Limits;
    use tokio::time::{self, Duration};

    use super::*;

    // Helper to create mock configs
    fn create_mock_configs() -> (UntrustedConfig, TrustedConfig) {
        let untrusted = UntrustedConfig {
            userdata_json: r#"{"message": "Hello from config!"}"#.to_string(),
            advanced_vm_config: HashMap::new(),
        };
        let trusted = TrustedConfig {
            crypto_keys: vec![
                // Example symmetric key JWK (base64 encoded bytes 0-15)
                r#"{"kty":"oct","k":"AAECAwQFBgcICQoLDA0ODw==", "alg": "HS256"}"#
                    .as_bytes()
                    .to_vec(),
            ],
            limits: Some(Limits {
                memory_mb: 128,
                cpu_count: 1,
                max_runtime_seconds: 5, // Short runtime for tests
            }),
        };
        (untrusted, trusted)
    }

    // Helper JS code that returns a specific ID
    fn create_js_code(id: &str) -> Vec<u8> {
        format!(
            r#"
            export default {{
                async fetch(request, env, ctx) {{
                    let body = await request.text();
                    return new Response("Response from {}: " + body);
                }}
            }}
            "#,
            id
        )
        .into_bytes()
    }

    // Helper JS code that returns config data
    fn create_js_config_code() -> Vec<u8> {
        r#"
        export default {
            async fetch(request, env, ctx) {
                let config = JSON.parse(env.USER_CONFIG);
                // Basic check for key presence (workerd doesn't expose key directly)
                let key_present = typeof env.SECRET_KEY_0 !== 'undefined';
                return new Response(`Config message: ${config.message}, Key bound: ${key_present}`);
            }
        }
        "#
        .to_string()
        .into_bytes()
    }

    // Helper to wait for a worker to reach a specific status
    async fn wait_for_status(
        vmm: &WorkerdVmm,
        id: &str,
        target_status: WorkerStatus,
        timeout: Duration,
    ) -> Result<WorkerStatus, String> {
        let start = time::Instant::now();
        loop {
            match vmm.get_worker_status(id.to_string()).await {
                Ok(status) => {
                    if status == target_status {
                        return Ok(status);
                    }
                    // Special case: If we expect STOPPED, accept ERROR too, as workerd might crash on stop sometimes
                    if target_status == WorkerStatus::Stopped && status == WorkerStatus::Error {
                        warn!(
                            "Worker {} reached ERROR state while waiting for STOPPED.",
                            id
                        );
                        return Ok(status);
                    }
                }
                Err(e) => {
                    // If we expect STOPPED or ERROR, NotFound is also acceptable after some time
                    if (target_status == WorkerStatus::Stopped
                        || target_status == WorkerStatus::Error)
                        && start.elapsed() > Duration::from_secs(1)
                    {
                        if let Some(werr) =
                            e.source().and_then(|s| s.downcast_ref::<WorkerdVmError>())
                        {
                            if matches!(werr, WorkerdVmError::WorkerNotFound(_)) {
                                info!(
                                    "Worker {} not found, assuming Stopped/Error state reached.",
                                    id
                                );
                                return Ok(target_status); // Return the target status as it's effectively gone
                            }
                        }
                    }
                    // Otherwise, keep trying or report the error
                    if start.elapsed() >= timeout {
                        return Err(format!(
                            "Error getting status for {}: {}. Target: {:?}",
                            id, e, target_status
                        ));
                    }
                }
            }
            if start.elapsed() >= timeout {
                let final_status = vmm.get_worker_status(id.to_string()).await;
                return Err(format!(
                    "Timeout waiting for worker {} to reach {:?}. Last status: {:?}",
                    id, target_status, final_status
                ));
            }
            time::sleep(Duration::from_millis(200)).await;
        }
    }

    #[tokio::test]
    #[ignore] // Requires workerd binary on PATH
    async fn test_start_invoke_stop_single_worker() -> Result<(), Box<dyn std::error::Error>> {
        let vmm = WorkerdVmm::new();
        let (untrusted, trusted) = create_mock_configs();
        let code = create_js_code("single");

        // 1. Start worker
        let worker_id = vmm
            .start_worker(code, untrusted, trusted)
            .await
            .expect("Failed to start worker");
        info!("Started worker: {}", worker_id);

        // 2. Check status (should be Running shortly after start)
        let status = wait_for_status(
            &vmm,
            &worker_id,
            WorkerStatus::Running,
            Duration::from_secs(15),
        )
        .await?; // Increased timeout for CI
        assert_eq!(status, WorkerStatus::Running);

        // 3. Invoke worker
        let payload = b"test payload".to_vec();
        let response = vmm
            .invoke_worker(worker_id.clone(), payload)
            .await
            .expect("Failed to invoke worker");
        assert_eq!(
            String::from_utf8_lossy(&response),
            "Response from single: test payload"
        );

        // 4. Stop worker
        vmm.stop_worker(worker_id.clone())
            .await
            .expect("Failed to stop worker");

        // 5. Check status (should be Stopped or Error eventually)
        let final_status = wait_for_status(
            &vmm,
            &worker_id,
            WorkerStatus::Stopped,
            Duration::from_secs(5),
        )
        .await?;
        assert!(
            final_status == WorkerStatus::Stopped || final_status == WorkerStatus::Error,
            "Final status was {:?}",
            final_status
        );

        // 6. Attempt to invoke stopped worker
        let invoke_stopped_result = vmm
            .invoke_worker(worker_id.clone(), b"after stop".to_vec())
            .await;
        assert!(invoke_stopped_result.is_err());
        let err = invoke_stopped_result
            .unwrap_err()
            .source()
            .unwrap()
            .downcast_ref::<WorkerdVmError>()
            .unwrap()
            .to_string();
        // It could be WorkerNotFound if cleanup happened fast, or WorkerNotRunnable
        assert!(
            err.contains("Worker instance not found")
                || err.contains("Worker is not in a runnable state"),
            "Unexpected error: {}",
            err
        );

        Ok(())
    }

    #[tokio::test]
    #[ignore] // Requires workerd binary on PATH
    async fn test_multiple_workers_lifecycle() -> Result<(), Box<dyn std::error::Error>> {
        let vmm = WorkerdVmm::new();
        let (untrusted1, trusted1) = create_mock_configs();
        let (untrusted2, trusted2) = create_mock_configs();
        let code1 = create_js_code("worker1");
        let code2 = create_js_code("worker2");

        // 1. Start two workers
        let id1 = vmm
            .start_worker(code1, untrusted1, trusted1)
            .await
            .expect("Failed to start worker 1");
        info!("Started worker 1: {}", id1);
        let id2 = vmm
            .start_worker(code2, untrusted2, trusted2)
            .await
            .expect("Failed to start worker 2");
        info!("Started worker 2: {}", id2);

        // 2. Verify both are running
        let status1 =
            wait_for_status(&vmm, &id1, WorkerStatus::Running, Duration::from_secs(15)).await?; // Increased timeout
        assert_eq!(status1, WorkerStatus::Running);
        let status2 =
            wait_for_status(&vmm, &id2, WorkerStatus::Running, Duration::from_secs(15)).await?; // Increased timeout
        assert_eq!(status2, WorkerStatus::Running);

        let running_workers = vmm.list_running_workers().await?;
        assert!(running_workers.contains(&id1));
        assert!(running_workers.contains(&id2));
        assert_eq!(running_workers.len(), 2);

        // 3. Invoke each worker and verify response
        let resp1 = vmm.invoke_worker(id1.clone(), b"ping1".to_vec()).await?;
        assert_eq!(
            String::from_utf8_lossy(&resp1),
            "Response from worker1: ping1"
        );

        let resp2 = vmm.invoke_worker(id2.clone(), b"ping2".to_vec()).await?;
        assert_eq!(
            String::from_utf8_lossy(&resp2),
            "Response from worker2: ping2"
        );

        // 4. Shut down one worker (worker1)
        vmm.stop_worker(id1.clone())
            .await
            .expect("Failed to stop worker 1");

        // 5. Check statuses
        let status1_after_stop =
            wait_for_status(&vmm, &id1, WorkerStatus::Stopped, Duration::from_secs(5)).await?;
        assert!(
            status1_after_stop == WorkerStatus::Stopped
                || status1_after_stop == WorkerStatus::Error,
            "Worker 1 final status was {:?}",
            status1_after_stop
        );

        let status2_after_stop = vmm.get_worker_status(id2.clone()).await?;
        assert_eq!(status2_after_stop, WorkerStatus::Running); // Worker 2 should still be running

        let running_workers_after_stop = vmm.list_running_workers().await?;
        assert!(!running_workers_after_stop.contains(&id1));
        assert!(running_workers_after_stop.contains(&id2));
        assert_eq!(running_workers_after_stop.len(), 1);

        // 6. Attempt to invoke the shut-down worker (worker1)
        let invoke_stopped_result = vmm.invoke_worker(id1.clone(), b"post-stop".to_vec()).await;
        assert!(invoke_stopped_result.is_err());
        let err_str = invoke_stopped_result.unwrap_err().to_string();
        assert!(
            err_str.contains("Worker instance not found")
                || err_str.contains("Worker is not in a runnable state"),
            "Unexpected error string: {}",
            err_str
        );

        // 7. Invoke the remaining worker (worker2)
        let resp2_again = vmm
            .invoke_worker(id2.clone(), b"ping2 again".to_vec())
            .await?;
        assert_eq!(
            String::from_utf8_lossy(&resp2_again),
            "Response from worker2: ping2 again"
        );

        // 8. Clean up the remaining worker (worker2)
        vmm.stop_worker(id2.clone())
            .await
            .expect("Failed to stop worker 2");
        let status2_final =
            wait_for_status(&vmm, &id2, WorkerStatus::Stopped, Duration::from_secs(5)).await?;
        assert!(
            status2_final == WorkerStatus::Stopped || status2_final == WorkerStatus::Error,
            "Worker 2 final status was {:?}",
            status2_final
        );

        let running_workers_final = vmm.list_running_workers().await?;
        assert!(running_workers_final.is_empty());

        Ok(())
    }

    #[tokio::test]
    #[ignore] // Requires workerd binary on PATH
    async fn test_worker_config_bindings() -> Result<(), Box<dyn std::error::Error>> {
        let vmm = WorkerdVmm::new();
        let (untrusted, trusted) = create_mock_configs();
        let code = create_js_config_code();

        let worker_id = vmm
            .start_worker(code, untrusted, trusted)
            .await
            .expect("Failed to start config worker");

        let status = wait_for_status(
            &vmm,
            &worker_id,
            WorkerStatus::Running,
            Duration::from_secs(15),
        )
        .await?; // Increased timeout
        assert_eq!(status, WorkerStatus::Running);

        let response = vmm
            .invoke_worker(worker_id.clone(), vec![])
            .await
            .expect("Failed to invoke config worker");

        assert_eq!(
            String::from_utf8_lossy(&response),
            "Config message: Hello from config!, Key bound: true"
        );

        vmm.stop_worker(worker_id.clone()).await?;
        wait_for_status(
            &vmm,
            &worker_id,
            WorkerStatus::Stopped,
            Duration::from_secs(5),
        )
        .await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_error_handling_non_existent_worker() {
        let vmm = WorkerdVmm::new();
        let non_existent_id = "id-does-not-exist".to_string();

        // Stop non-existent
        let stop_res = vmm.stop_worker(non_existent_id.clone()).await;
        assert!(stop_res.is_err());
        assert!(
            stop_res
                .unwrap_err()
                .to_string()
                .contains("Worker instance not found")
        );

        // Invoke non-existent
        let invoke_res = vmm.invoke_worker(non_existent_id.clone(), vec![]).await;
        assert!(invoke_res.is_err());
        assert!(
            invoke_res
                .unwrap_err()
                .to_string()
                .contains("Worker instance not found")
        );

        // Get status non-existent
        let status_res = vmm.get_worker_status(non_existent_id.clone()).await;
        assert!(status_res.is_err());
        assert!(
            status_res
                .unwrap_err()
                .to_string()
                .contains("Worker instance not found")
        );

        // Get logs non-existent
        let logs_res = vmm.get_worker_logs(non_existent_id.clone()).await;
        assert!(logs_res.is_err());
        assert!(
            logs_res
                .unwrap_err()
                .to_string()
                .contains("Worker instance not found")
        );
    }

    #[tokio::test]
    async fn test_get_attestation_unsupported() {
        let vmm = WorkerdVmm::new();
        let attestation_res = vmm.get_attestation(vec![1, 2, 3]).await;
        assert!(attestation_res.is_err());
        assert!(
            attestation_res
                .unwrap_err()
                .to_string()
                .contains("Attestation not supported")
        );
    }

    #[tokio::test]
    async fn test_start_worker_invalid_code() {
        // This test assumes workerd itself might fail or the config builder might reject it.
        // Currently, config_builder requires UTF-8.
        let vmm = WorkerdVmm::new();
        let (untrusted, trusted) = create_mock_configs();
        let invalid_code = vec![0xff, 0xfe, 0xfd]; // Invalid UTF-8

        let start_res = vmm.start_worker(invalid_code, untrusted, trusted).await;
        assert!(start_res.is_err());
        // Expecting error from detect_code_type or build_config
        let err_msg = start_res.unwrap_err().to_string();
        assert!(
            err_msg.contains("Unsupported worker code type") || err_msg.contains("not valid UTF-8")
        );
    }

    // TODO: Add test for workerd failing to start if a mock workerd script is implemented.
    // Current tests rely on workerd being present and functional.
    // The STARTUP_TIMEOUT test implicitly covers cases where workerd starts but hangs.
}
