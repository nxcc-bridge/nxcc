use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicI32, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use dashmap::DashMap;
use http_body_util::BodyExt;
use hyper::{
    Method, Request, Response as HyperResponse, StatusCode,
    body::Bytes,
    header,
    header::{HeaderName, HeaderValue},
};
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use hyperlocal::UnixConnector;
use nxcc_interface::{
    proto::vm::{
        Header as ProtoHeader, HttpRequest as ProtoHttpRequest, HttpResponse as ProtoHttpResponse,
        TrustedConfig, UntrustedConfig, WorkerStatus,
    },
    types::{AttestationReport, EventPayload},
};
use nxcc_vm_base::server::{VmError, VmRuntime};
use serde::Deserialize;
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
    event_payload: EventPayload<'a>,
}

/// Holds information about a single worker process.
/// The structure is designed to minimize locking.
/// - `status` is atomic for lock-free reads.
/// - `process` is behind a Mutex because `Child` methods require `&mut self`.
/// - Other fields are immutable after creation and can be read freely.
#[derive(Debug)]
struct WorkerData {
    instance_id: String,
    process: Mutex<Child>,
    pid: u32,
    status: AtomicI32,
    uds_path: PathBuf,
    config_path: PathBuf,
    temp_dir: Arc<TempDir>,
    code_type: CodeType,
    logs: Arc<Mutex<String>>,
}

impl WorkerData {
    fn get_status(&self) -> WorkerStatus {
        let status_val = self.status.load(Ordering::SeqCst);
        // Gracefully handle if the value is somehow not a valid enum variant.
        WorkerStatus::try_from(status_val).unwrap_or(WorkerStatus::Unspecified)
    }

    fn set_status(&self, new_status: WorkerStatus) {
        self.status.store(new_status as i32, Ordering::SeqCst);
    }
}

#[derive(Clone)]
pub struct WorkerdVmm {
    // Use a highly concurrent DashMap instead of a Mutex-guarded HashMap.
    // This allows simultaneous access to different workers.
    workers: Arc<DashMap<String, Arc<WorkerData>>>,
    config: WorkerdConfig,
}

impl WorkerdVmm {
    pub fn new(config: WorkerdConfig) -> Self {
        WorkerdVmm {
            workers: Arc::new(DashMap::new()),
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
        instance_id: &str,
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
    }

    async fn probe_worker_internal(
        &self,
        id: &str,
    ) -> Result<(WorkerStatus, String), WorkerdVmError> {
        let worker = self
            .workers
            .get(id)
            .ok_or_else(|| WorkerdVmError::WorkerNotFound(id.to_string()))?
            .clone();

        // Lock only the process to check its exit status
        if let Ok(Some(exit_status)) = worker.process.lock().await.try_wait() {
            let msg = format!("Process exited with status: {}", exit_status);
            error!(instance_id = ?id, "{}", msg);
            if worker.get_status() != WorkerStatus::Stopped {
                worker.set_status(WorkerStatus::Error);
            }
            return Ok((worker.get_status(), msg));
        }

        match tokio::net::UnixStream::connect(&worker.uds_path).await {
            Ok(_) => {
                if worker.get_status() == WorkerStatus::Starting {
                    worker.set_status(WorkerStatus::Running);
                }
                Ok((worker.get_status(), "UDS socket connectable".to_string()))
            }
            Err(e) => {
                let msg = format!("UDS socket not connectable: {}", e);
                warn!(instance_id = ?id, "{}", msg);
                if worker.get_status() != WorkerStatus::Starting {
                    worker.set_status(WorkerStatus::Error);
                }
                Ok((worker.get_status(), msg))
            }
        }
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
            "main",
            "invoke_socket",
        )?;

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

        let logs_arc = Arc::new(Mutex::new(String::new()));
        if let Some(stdout) = child_process.stdout.take() {
            tokio::spawn(WorkerdVmm::log_stream(
                BufReader::new(stdout),
                instance_id.clone(),
                "stdout",
                logs_arc.clone(),
            ));
        }
        if let Some(stderr) = child_process.stderr.take() {
            tokio::spawn(WorkerdVmm::log_stream(
                BufReader::new(stderr),
                instance_id.clone(),
                "stderr",
                logs_arc.clone(),
            ));
        }

        let worker_data = Arc::new(WorkerData {
            instance_id: instance_id.clone(),
            process: Mutex::new(child_process),
            pid: child_pid,
            status: AtomicI32::new(WorkerStatus::Starting as i32),
            uds_path: uds_path.clone(),
            config_path,
            temp_dir: temp_dir_handle,
            code_type,
            logs: logs_arc.clone(),
        });
        // Insert into the DashMap without a global lock.
        self.workers.insert(instance_id.clone(), worker_data);

        let start_time = Instant::now();
        let startup_timeout = Duration::from_secs(self.config.startup_timeout_secs);
        let uds_check_interval = Duration::from_millis(self.config.uds_check_interval_ms);
        loop {
            if let Some(worker) = self.workers.get(&instance_id) {
                // Lock only the process to check its status.
                let mut process = worker.process.lock().await;
                if let Ok(Some(exit_status)) = process.try_wait() {
                    worker.set_status(WorkerStatus::Error);
                    let logs_content = worker.logs.lock().await.clone();
                    error!(
                        instance_id = ?instance_id,
                        ?exit_status,
                        "Workerd process exited prematurely during startup check. Logs:\n{}",
                        logs_content
                    );
                    return Err(WorkerdVmError::StartupFailedPrematureExit {
                        instance_id: instance_id.clone(),
                        final_status: worker.get_status(),
                        logs: logs_content,
                    }
                    .into());
                }
            } else {
                return Err(WorkerdVmError::Internal(format!(
                    "Worker {} disappeared from map during startup",
                    instance_id
                ))
                .into());
            }

            if tokio::net::UnixStream::connect(&uds_path).await.is_ok() {
                sleep(Duration::from_millis(100)).await;

                if let Some(worker) = self.workers.get(&instance_id) {
                    let mut process = worker.process.lock().await;
                    if let Ok(Some(exit_status)) = process.try_wait() {
                        worker.set_status(WorkerStatus::Error);
                        let logs_content = worker.logs.lock().await.clone();
                        error!(
                            instance_id = ?instance_id,
                            ?exit_status,
                            "Workerd process exited just before marking as Running. Logs:\n{}",
                            logs_content
                        );
                        return Err(WorkerdVmError::StartupFailedPrematureExit {
                            instance_id: instance_id.clone(),
                            final_status: worker.get_status(),
                            logs: logs_content,
                        }
                        .into());
                    }
                    if worker.get_status() == WorkerStatus::Starting {
                        worker.set_status(WorkerStatus::Running);
                        info!(instance_id = ?instance_id, "UDS ready, worker is now Running.");
                    }
                } else {
                    return Err(WorkerdVmError::Internal(format!(
                        "Worker {} disappeared from map just before marking Running",
                        instance_id
                    ))
                    .into());
                }
                return Ok(instance_id);
            }

            if start_time.elapsed() > startup_timeout {
                if let Some(worker) = self.workers.get(&instance_id) {
                    let mut process = worker.process.lock().await;
                    let final_status = match process.try_wait() {
                        Ok(Some(status)) => {
                            error!(instance_id=?worker.instance_id, ?status, "Workerd exited on its own before startup timeout.");
                            WorkerStatus::Error
                        }
                        _ => WorkerStatus::Error,
                    };

                    if worker.get_status() == WorkerStatus::Starting {
                        worker.set_status(final_status);
                        warn!(
                            instance_id = ?worker.instance_id,
                            timeout = ?startup_timeout,
                            "Timeout waiting for UDS. Killing process (if running)."
                        );
                        if let Err(e) = process.kill().await {
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
        let worker = self
            .workers
            .get(&id)
            .ok_or_else(|| WorkerdVmError::WorkerNotFound(id.clone()))?
            .clone();

        match worker.get_status() {
            WorkerStatus::Starting | WorkerStatus::Running => {
                info!(instance_id = ?id, "Stopping worker.");
                // Lock only the process to kill it.
                if let Err(e) = worker.process.lock().await.kill().await {
                    if e.kind() != std::io::ErrorKind::InvalidInput {
                        error!(?id, "Failed to kill worker process: {}", e);
                        return Err(WorkerdVmError::Internal(format!(
                            "Failed to kill worker {}: {}",
                            id, e
                        ))
                        .into());
                    }
                }
                worker.set_status(WorkerStatus::Stopped);
                Ok(())
            }
            WorkerStatus::Stopped | WorkerStatus::Error => {
                debug!(
                    instance_id = ?id,
                    "stop_worker called but worker is already in {:?} state, ignoring.", worker.get_status()
                );
                Ok(())
            }
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
        // --- HOT PATH OPTIMIZATION ---
        // 1. Get worker from DashMap (fast, concurrent)
        let worker = self
            .workers
            .get(&id)
            .ok_or_else(|| WorkerdVmError::WorkerNotFound(id.clone()))?
            .clone(); // Clone the Arc, releasing the map's read guard

        // 2. Check status with a lock-free atomic read
        let status = worker.get_status();
        if status != WorkerStatus::Running {
            error!(
                instance_id = ?id,
                "Attempted to invoke worker in non-running state: {:?}", status
            );
            return Err(WorkerdVmError::WorkerNotRunnable(status).into());
        }

        // 3. Get immutable data without a lock
        let uds_path = worker.uds_path.clone();
        // --- End of shared state access for invocation setup ---

        let http_handler_path = if handler_name.starts_with('/') {
            handler_name
        } else {
            format!("/{}", handler_name)
        };

        debug!(instance_id = ?id, %http_handler_path, "Invoking worker via UDS");

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
            .map(|resp| resp.body)
    }

    async fn invoke_http(
        &self,
        id: String,
        request: ProtoHttpRequest,
    ) -> Result<ProtoHttpResponse, VmError> {
        // --- HOT PATH OPTIMIZATION (same as invoke_worker) ---
        let worker = self
            .workers
            .get(&id)
            .ok_or_else(|| WorkerdVmError::WorkerNotFound(id.clone()))?
            .clone();

        let status = worker.get_status();
        if status != WorkerStatus::Running {
            error!(
                instance_id = ?id,
                "Attempted to HTTP invoke worker in non-running state: {:?}", status
            );
            return Err(WorkerdVmError::WorkerNotRunnable(status).into());
        }

        let uds_path = worker.uds_path.clone();

        debug!(instance_id = ?id, uri = %request.uri, "Invoking worker HTTP endpoint via UDS");

        self.invoke_via_uds(&id, &uds_path, request)
            .await
            .map_err(|e| {
                error!(instance_id = ?id, "HTTP Invocation via UDS failed: {}", e);
                e.into()
            })
    }

    async fn get_attestation(&self, _user_data: Vec<u8>) -> Result<AttestationReport, VmError> {
        Err(WorkerdVmError::AttestationNotSupported.into())
    }

    async fn get_worker_status(&self, id: String) -> Result<WorkerStatus, VmError> {
        // Lock-free status check
        let worker = self
            .workers
            .get(&id)
            .ok_or_else(|| WorkerdVmError::WorkerNotFound(id))?;
        Ok(worker.get_status())
    }

    async fn list_running_workers(&self) -> Result<Vec<String>, VmError> {
        // Iterate over the map without a global lock and perform lock-free status checks.
        let running_ids = self
            .workers
            .iter()
            .filter(|entry| entry.value().get_status() == WorkerStatus::Running)
            .map(|entry| entry.key().clone())
            .collect();
        Ok(running_ids)
    }

    async fn get_worker_logs(&self, id: String) -> Result<String, VmError> {
        let worker = self
            .workers
            .get(&id)
            .ok_or_else(|| WorkerdVmError::WorkerNotFound(id))?;

        // The log buffer itself still needs a lock, but this is not on the hot path.
        let logs_content = worker.logs.lock().await.clone();
        Ok(logs_content)
    }

    async fn probe_worker(&self, id: String) -> Result<(WorkerStatus, String), VmError> {
        self.probe_worker_internal(&id).await.map_err(|e| e.into())
    }
}

#[cfg(test)]
mod tests;
