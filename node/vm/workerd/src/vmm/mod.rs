use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait; // Already present
use http_body_util::BodyExt;
use hyper::{
    Method, Request, Response as HyperResponse, StatusCode,
    body::Bytes,
    header,
    header::{HeaderName, HeaderValue},
};
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use hyperlocal::UnixConnector;
use nxcc_interface::types::EventPayload; // For deserializing VmEventInvocation
use nxcc_interface::{
    proto::vm::{
        Header as ProtoHeader, HttpRequest as ProtoHttpRequest, HttpResponse as ProtoHttpResponse,
        TrustedConfig, UntrustedConfig, WorkerStatus,
    },
    types::AttestationReport,
};
use nxcc_vm_base::server::{VmError, VmRuntime};
use serde::Deserialize; // For VmEventInvocation
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
    config::WorkerdConfig,
    config_builder::{CodeType, build_config, detect_code_type},
    errors::WorkerdVmError,
};

/// Expected structure of the payload received by `WorkerdVmm::invoke_worker`
/// when the invocation is for an event.
#[derive(Deserialize)]
struct VmEventInvocationRequest<'a> {
    handler: String,
    #[serde(borrow)]
    event_payload: EventPayload<'a>, // Assuming EventPayload can be deserialized with lifetime if needed, or owned
}

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
    config: WorkerdConfig,
}

impl WorkerdVmm {
    pub fn new(config: WorkerdConfig) -> Self {
        WorkerdVmm {
            workers: Arc::new(Mutex::new(HashMap::new())),
            config,
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
        proto_http_request: ProtoHttpRequest,
    ) -> Result<ProtoHttpResponse, WorkerdVmError> {
        let client: Client<UnixConnector, http_body_util::Full<Bytes>> =
            Client::builder(TokioExecutor::new()).build(UnixConnector);

        let method = Method::from_bytes(proto_http_request.method.as_bytes())
            .map_err(|e| WorkerdVmError::InvalidHttpRequest(format!("Invalid method: {}", e)))?;

        let mut hyper_request_builder = Request::builder()
            .method(method)
            .uri(hyperlocal::Uri::new(uds_path, &proto_http_request.uri));

        for header_proto in proto_http_request.headers {
            let header_name = HeaderName::from_bytes(header_proto.key.as_bytes()).map_err(|e| {
                WorkerdVmError::InvalidHttpRequest(format!("Invalid header name: {}", e))
            })?;
            let header_value = HeaderValue::from_bytes(&header_proto.value).map_err(|e| {
                WorkerdVmError::InvalidHttpRequest(format!("Invalid header value: {}", e))
            })?;
            hyper_request_builder = hyper_request_builder.header(header_name, header_value);
        }

        let hyper_request = hyper_request_builder
            .body(http_body_util::Full::new(Bytes::from(
                proto_http_request.body,
            )))
            .map_err(|e| WorkerdVmError::UdsCommunicationFailed {
                path: uds_path.to_path_buf(),
                source: Box::new(e),
            })?;

        debug!(
            "Sending invocation to UDS: {} at path {}",
            uds_path.display(),
            proto_http_request.uri
        );

        let response = client.request(hyper_request).await.map_err(|e| {
            WorkerdVmError::UdsCommunicationFailed {
                path: uds_path.to_path_buf(),
                source: Box::new(e),
            }
        })?;

        let mut proto_response_headers = Vec::new();
        for (name, value) in response.headers().iter() {
            proto_response_headers.push(ProtoHeader {
                key: name.as_str().to_string(),
                value: value.as_bytes().to_vec(),
            });
        }

        let status = response.status();
        let response_body_bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|e| WorkerdVmError::UdsCommunicationFailed {
                path: uds_path.to_path_buf(),
                source: Box::new(e),
            })?
            .to_bytes();

        Ok(ProtoHttpResponse {
            status_code: status.as_u16() as u32,
            headers: proto_response_headers,
            body: response_body_bytes.to_vec(),
        })

        // The original error handling for non-OK status is now implicitly handled
        // by the caller of invoke_http, as the full HttpResponse is returned.
        // The daemon will decide what to do based on the status code.
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
                .prefix(&self.config.temp_dir_prefix)
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
            "main",          // Default service name for the socket
            "invoke_socket", // Name of the socket binding in config
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
        if Command::new(&self.config.binary_path)
            .arg("--version")
            .output()
            .await
            .is_err()
        {
            error!(
                "workerd binary not found or failed to execute at path: {}",
                self.config.binary_path
            );
            return Err(WorkerdVmError::WorkerdBinaryNotFound.into());
        }

        let mut command = Command::new(&self.config.binary_path);
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
        let startup_timeout = Duration::from_secs(self.config.startup_timeout_secs);
        let uds_check_interval = Duration::from_millis(self.config.uds_check_interval_ms);
        loop {
            // Check if the process exited prematurely
            {
                let workers_map = self.workers.lock().await;
                if let Some(worker_lock) = workers_map.get(&instance_id) {
                    let mut worker = worker_lock.lock().await;
                    // Check if the process handle still exists and try_wait
                    match worker.process.try_wait() {
                        Ok(Some(exit_status)) => {
                            // Process exited!
                            worker.status = WorkerStatus::Error; // Or determine based on exit_status
                            let logs_content = worker.logs.lock().await.clone();
                            error!(
                                instance_id = ?instance_id,
                                ?exit_status,
                                "Workerd process exited prematurely during startup check. Logs:\n{}",
                                logs_content
                            );
                            return Err(WorkerdVmError::StartupFailedPrematureExit {
                                instance_id: instance_id.clone(),
                                final_status: worker.status, // Use the updated status
                                logs: logs_content,
                            }
                            .into());
                        }
                        Ok(None) => {
                            // Still running, continue check
                        }
                        Err(e) => {
                            // Error checking status, log it but might proceed cautiously
                            warn!(instance_id = ?instance_id, "Error checking workerd process status during startup: {}", e);
                        }
                    }
                } else {
                    // Worker somehow removed from map during startup? Should not happen.
                    return Err(WorkerdVmError::Internal(format!(
                        "Worker {} disappeared from map during startup",
                        instance_id
                    ))
                    .into());
                }
            }

            match tokio::net::UnixStream::connect(&uds_path).await {
                Ok(_) => {
                    // Socket file exists and is connectable.
                    sleep(Duration::from_millis(100)).await; // Adjust duration if needed (50-200ms range)

                    // Re-check process status one last time before declaring success
                    // (Optional but safer: prevents declaring success if it crashed *during* the sleep)
                    {
                        let workers_map = self.workers.lock().await;
                        if let Some(worker_lock) = workers_map.get(&instance_id) {
                            let mut worker = worker_lock.lock().await;
                            if let Ok(Some(exit_status)) = worker.process.try_wait() {
                                worker.status = WorkerStatus::Error;
                                let logs_content = worker.logs.lock().await.clone();
                                error!(
                                    instance_id = ?instance_id,
                                    ?exit_status,
                                    "Workerd process exited just before marking as Running. Logs:\n{}",
                                    logs_content
                                );
                                return Err(WorkerdVmError::StartupFailedPrematureExit {
                                    instance_id: instance_id.clone(),
                                    final_status: worker.status,
                                    logs: logs_content,
                                }
                                .into());
                            }
                            // If still running or error checking status, proceed to mark Running
                            if worker.status == WorkerStatus::Starting {
                                worker.status = WorkerStatus::Running;
                                info!(instance_id = ?instance_id, "UDS ready, worker is now Running.");
                            }
                        } else {
                            // Should not happen
                            return Err(WorkerdVmError::Internal(format!(
                                "Worker {} disappeared from map just before marking Running",
                                instance_id
                            ))
                            .into());
                        }
                    }
                    return Ok(instance_id);
                }
                Err(e) => {
                    // Log unexpected errors, but connection refused/not found are expected during startup
                    if e.kind() != std::io::ErrorKind::NotFound
                        && e.kind() != std::io::ErrorKind::ConnectionRefused
                    {
                        warn!(
                            ?instance_id,
                            uds=%uds_path.display(),
                            "Unexpected error checking UDS: {}. Will retry.", e
                        );
                    }
                    // No need to sleep here, the main loop sleep handles polling interval
                }
            }

            if start_time.elapsed() > startup_timeout {
                // Timed out waiting for UDS. Mark the worker as Error and kill it.
                let workers_map = self.workers.lock().await;
                if let Some(worker_lock) = workers_map.get(&instance_id) {
                    let mut worker = worker_lock.lock().await;
                    // Check if it exited on its own before we kill it
                    let final_status = match worker.process.try_wait() {
                        Ok(Some(status)) => {
                            error!(instance_id=?worker.instance_id, ?status, "Workerd exited on its own before startup timeout.");
                            WorkerStatus::Error // Mark as error if it exited
                        }
                        _ => WorkerStatus::Error, // Still mark as Error due to timeout
                    };

                    if worker.status == WorkerStatus::Starting {
                        worker.status = final_status; // Use determined status
                        warn!(
                            instance_id = ?worker.instance_id,
                            timeout = ?startup_timeout,
                            "Timeout waiting for UDS. Killing process (if running)."
                        );
                        // Use kill() directly on the Child object, no need for pid lookup
                        if let Err(e) = worker.process.kill().await {
                            // Log error if kill fails, but proceed with timeout error
                            // Ignore InvalidInput error which means process already exited
                            if e.kind() != std::io::ErrorKind::InvalidInput {
                                error!(instance_id = ?worker.instance_id, "Error killing process on startup timeout: {}", e);
                            }
                        }
                    }
                    let logs_content = worker.logs.lock().await.clone();
                    return Err(WorkerdVmError::StartupTimeout {
                        instance_id: instance_id.clone(),
                        timeout: startup_timeout,
                        logs: logs_content,
                    }
                    .into());
                }
                // If it's not in the map for some reason, just fail
                return Err(WorkerdVmError::StartupTimeout {
                    instance_id,
                    timeout: startup_timeout,
                    logs: "[No logs captured - worker not found in map]".to_string(),
                }
                .into());
            }

            sleep(uds_check_interval).await;
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

    async fn invoke_worker(
        &self,
        id: String,
        handler_name: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, VmError> {
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

        // The `payload` here is the one constructed by `RunnerService`, which is
        // `VmEventInvocation` serialized to JSON (or whatever RunnerService chose for events),
        // or the direct policy context payload.
        // The `handler_name` parameter to *this* function (`WorkerdVmm::invoke_worker`)
        // is the one extracted by `RunnerService` from `VmEventInvocation.handler` for events,
        // or a fixed name like "_policy" for policy execution.

        // The actual bytes to send to the worker's HTTP endpoint are in `payload`.
        // The `handler_name` is used to construct the HTTP path.

        let http_handler_path = if handler_name.starts_with('/') {
            handler_name.clone()
        } else {
            format!("/{}", handler_name)
        };

        debug!(instance_id = ?id, %http_handler_path, "Invoking worker via UDS");

        // For invoke_worker, we still use the old path, assuming it's a POST with octet-stream
        let proto_http_request = ProtoHttpRequest {
            method: "POST".to_string(),
            uri: http_handler_path,
            headers: vec![ProtoHeader {
                key: header::CONTENT_TYPE.to_string(),
                value: "application/octet-stream".as_bytes().to_vec(),
            }],
            body: payload,
        };

        self.invoke_via_uds(&id, &uds_path, proto_http_request)
            .await
            .map_err(|e| {
                error!(instance_id = ?id, "Invocation via UDS failed: {}", e);
                e.into()
            })
            .map(|resp| resp.body) // Return only the body for invoke_worker
    }

    async fn invoke_http(
        &self,
        id: String,
        request: ProtoHttpRequest,
    ) -> Result<ProtoHttpResponse, VmError> {
        let worker_lock = {
            let workers = self.workers.lock().await;
            workers.get(&id).cloned()
        }
        .ok_or_else(|| WorkerdVmError::WorkerNotFound(id.clone()))?;

        let uds_path = {
            let worker = worker_lock.lock().await;
            if worker.status != WorkerStatus::Running {
                error!(
                    instance_id = ?id,
                    "Attempted to HTTP invoke worker in non-running state: {:?}", worker.status
                );
                return Err(WorkerdVmError::WorkerNotRunnable(worker.status).into());
            }
            worker.uds_path.clone()
        };

        debug!(instance_id = ?id, uri = %request.uri, "Invoking worker HTTP endpoint via UDS");

        self.invoke_via_uds(&id, &uds_path, request)
            .await
            .map_err(|e| {
                error!(instance_id = ?id, "HTTP Invocation via UDS failed: {}", e);
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
