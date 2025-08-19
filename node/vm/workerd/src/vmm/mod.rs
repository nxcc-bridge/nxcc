use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicI32, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
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
        StreamWorkerLogsResponse, TrustedConfig, UntrustedConfig, WorkerStatus,
    },
    types::{AttestationReport, EventPayload},
};
use nxcc_vm_base::{
    logging::{LogEntry, VmmLogManager},
    server::{VmError, VmRuntime},
};
use serde::Deserialize;
use tempfile::TempDir;
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{Mutex, mpsc},
    time::sleep,
};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{Instrument as _, debug, error, info, instrument, warn};

use crate::{
    config::WorkerdConfig,
    config_builder::{CodeType, build_config, detect_code_type},
    errors::WorkerdVmError,
};

#[derive(Deserialize)]
struct VmEventInvocationRequest<'a> {
    handler: String,
    #[serde(borrow)]
    event_payload: EventPayload<'a>,
}

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
    logs: Arc<Mutex<String>>, // Legacy logs field for compatibility
    log_buffer: Arc<nxcc_vm_base::logging::LogBuffer>,
}

impl WorkerData {
    fn get_status(&self) -> WorkerStatus {
        let status_val = self.status.load(Ordering::SeqCst);
        WorkerStatus::try_from(status_val).unwrap_or(WorkerStatus::Unspecified)
    }

    fn set_status(&self, new_status: WorkerStatus) {
        self.status.store(new_status as i32, Ordering::SeqCst);
    }
}

#[derive(Clone)]
pub struct WorkerdVmm {
    workers: Arc<DashMap<String, Arc<WorkerData>>>,
    config: WorkerdConfig,
    client: Client<UnixConnector, http_body_util::Full<Bytes>>,
    log_manager: Arc<VmmLogManager>,
}

impl WorkerdVmm {
    pub fn new(config: WorkerdConfig) -> Self {
        let client = Client::builder(TokioExecutor::new()).build(UnixConnector);

        WorkerdVmm {
            workers: Arc::new(DashMap::new()),
            config,
            client,
            log_manager: VmmLogManager::new(),
        }
    }

    async fn log_stream<R>(
        mut reader: BufReader<R>,
        instance_id: String,
        output_type: &str,
        logs: Arc<Mutex<String>>,
        log_buffer: Arc<nxcc_vm_base::logging::LogBuffer>,
    ) where
        R: AsyncReadExt + Unpin,
    {
        let mut line = String::new();
        while let Ok(bytes_read) = reader.read_line(&mut line).await {
            if bytes_read == 0 {
                break;
            }

            let formatted_line = format!("{}: {}", output_type, line);

            // Write to legacy logs for compatibility
            {
                let mut logs_guard = logs.lock().await;
                logs_guard.push_str(&formatted_line);
            }

            // Write to new log buffer system
            log_buffer.write_log(formatted_line.trim_end().to_string());

            match output_type {
                "stdout" => info!(?instance_id, "stdout: {}", line.trim()),
                "stderr" => error!(?instance_id, "stderr: {}", line.trim()),
                _ => debug!(?instance_id, "{} = {}", output_type, line.trim()),
            }
            line.clear();
        }
    }

    #[instrument(
        level = "info",
        skip(self, proto_http_request),
        fields(instance_id = %instance_id, uri = %proto_http_request.uri),
        err
    )]
    async fn invoke_via_uds(
        &self,
        instance_id: &str,
        uds_path: &Path,
        proto_http_request: ProtoHttpRequest,
    ) -> Result<ProtoHttpResponse, WorkerdVmError> {
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

        let response = self.client.request(hyper_request).await.map_err(|e| {
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

    #[instrument(level = "info", skip(self), fields(id = %id), err)]
    async fn probe_worker_internal(
        &self,
        id: &str,
    ) -> Result<(WorkerStatus, String), WorkerdVmError> {
        let worker = self
            .workers
            .get(id)
            .ok_or_else(|| WorkerdVmError::WorkerNotFound(id.to_string()))?
            .clone();

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
    #[instrument(
        level = "info",
        skip(self, worker_code, untrusted_config, trusted_config),
        err
    )]
    async fn start_worker(
        &self,
        worker_code: Vec<u8>,
        untrusted_config: UntrustedConfig,
        trusted_config: TrustedConfig,
    ) -> Result<String, VmError> {
        let code_type = {
            let _span = tracing::info_span!("detect_code_type").entered();
            detect_code_type(&worker_code)?
        };

        let temp_dir_handle = {
            let _span = tracing::info_span!("create_temp_dir").entered();
            Arc::new(
                tempfile::Builder::new()
                    .prefix(&self.config.temp_dir_prefix)
                    .rand_bytes(5)
                    .tempdir()
                    .map_err(WorkerdVmError::TempDirCreationFailed)?,
            )
        };
        let base_path = temp_dir_handle.path();
        let instance_id = uuid::Uuid::new_v4().to_string();
        tracing::Span::current().record("instance_id", &instance_id);

        let short_id = &instance_id[..8];
        let uds_file_name = format!("w-{}.sock", short_id);
        let uds_path = base_path.join(&uds_file_name);
        let config_file_name = "config.capnp.bin";
        let config_path = base_path.join(config_file_name);

        info!(
            uds = %uds_path.display(),
            config = %config_path.display(),
            "Preparing worker"
        );

        let config_bytes = {
            let _span = tracing::info_span!("build_config").entered();
            build_config(
                &worker_code,
                code_type,
                &untrusted_config,
                &trusted_config,
                &uds_path,
                "main",
                "invoke_socket",
            )?
        };

        let span = tracing::info_span!("write_config_file");
        let cfg_path = &config_path;
        async move {
            let mut file = File::create(cfg_path)
                .await
                .map_err(WorkerdVmError::ConfigFileWriteFailed)?;
            file.write_all(&config_bytes)
                .await
                .map_err(WorkerdVmError::ConfigFileWriteFailed)?;
            file.flush()
                .await
                .map_err(WorkerdVmError::ConfigFileWriteFailed)?;
            Ok::<_, WorkerdVmError>(())
        }
        .instrument(span)
        .await?;

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

        info!("Spawning workerd process...");
        let mut child_process = {
            let _span = tracing::info_span!("spawn_process").entered();
            command
                .spawn()
                .map_err(WorkerdVmError::ProcessStartFailed)?
        };
        let child_pid = child_process.id().unwrap_or(0);
        info!(pid = ?child_pid, "Workerd process spawned.");

        let logs_arc = Arc::new(Mutex::new(String::new()));
        let log_buffer = self.log_manager.register_worker(instance_id.clone());

        if let Some(stdout) = child_process.stdout.take() {
            tokio::spawn(WorkerdVmm::log_stream(
                BufReader::new(stdout),
                instance_id.clone(),
                "stdout",
                logs_arc.clone(),
                log_buffer.clone(),
            ));
        }
        if let Some(stderr) = child_process.stderr.take() {
            tokio::spawn(WorkerdVmm::log_stream(
                BufReader::new(stderr),
                instance_id.clone(),
                "stderr",
                logs_arc.clone(),
                log_buffer.clone(),
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
            log_buffer: log_buffer.clone(),
        });
        self.workers.insert(instance_id.clone(), worker_data);

        let span = tracing::info_span!("wait_for_uds_ready");
        let start_time = Instant::now();
        let startup_timeout = Duration::from_secs(self.config.startup_timeout_secs);
        let uds_check_interval = Duration::from_millis(self.config.uds_check_interval_ms);
        async move {
        loop {
            if let Some(worker) = self.workers.get(&instance_id) {
                let mut process = worker.process.lock().await;
                if let Ok(Some(exit_status)) = process.try_wait() {
                    worker.set_status(WorkerStatus::Error);
                    let logs_content = worker.logs.lock().await.clone();
                    error!(
                        ?exit_status,
                        "Workerd process exited prematurely during startup check"
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

            // Check if socket file exists (workerd creates it when ready)
            if uds_path.exists() {
                // Give workerd additional time to fully initialize
                sleep(Duration::from_millis(200)).await;

                if let Some(worker) = self.workers.get(&instance_id) {
                    let mut process = worker.process.lock().await;
                    if let Ok(Some(exit_status)) = process.try_wait() {
                        worker.set_status(WorkerStatus::Error);
                        let logs_content = worker.logs.lock().await.clone();
                        error!(
                            ?exit_status,
                            "Workerd process exited just before marking as Running"
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
                        info!("UDS ready, worker is now Running.");
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
    }.instrument(span).await
    }

    #[instrument(level = "info", skip(self), fields(id = %id), err)]
    async fn stop_worker(&self, id: String) -> Result<(), VmError> {
        let worker = self
            .workers
            .get(&id)
            .ok_or_else(|| WorkerdVmError::WorkerNotFound(id.clone()))?
            .clone();

        match worker.get_status() {
            WorkerStatus::Starting | WorkerStatus::Running => {
                info!(instance_id = ?id, "Stopping worker.");
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

                // Move worker logs to dead storage
                self.log_manager.terminate_worker(&id);

                Ok(())
            }
            WorkerStatus::Stopped | WorkerStatus::Error => Ok(()),
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

    #[instrument(
        level = "info",
        skip(self, payload),
        fields(id = %id, handler = %handler_name),
        err
    )]
    async fn invoke_worker(
        &self,
        id: String,
        handler_name: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, VmError> {
        let worker = self
            .workers
            .get(&id)
            .ok_or_else(|| WorkerdVmError::WorkerNotFound(id.clone()))?
            .clone();

        let status = worker.get_status();
        if status != WorkerStatus::Running {
            error!(
                instance_id = ?id,
                "Attempted to invoke worker in non-running state: {:?}", status
            );
            return Err(WorkerdVmError::WorkerNotRunnable(status).into());
        }

        let uds_path = worker.uds_path.clone();

        let http_handler_path = if handler_name.starts_with('/') {
            handler_name
        } else {
            format!("/{}", handler_name)
        };

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

    #[instrument(
        level = "info",
        skip(self, request),
        fields(id = %id, uri = %request.uri, method = %request.method),
        err
    )]
    async fn invoke_http(
        &self,
        id: String,
        request: ProtoHttpRequest,
    ) -> Result<ProtoHttpResponse, VmError> {
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

        self.invoke_via_uds(&id, &uds_path, request)
            .await
            .map_err(|e| {
                error!(instance_id = ?id, "HTTP Invocation via UDS failed: {}", e);
                e.into()
            })
    }

    #[instrument(level = "info", skip(self, _user_data), err)]
    async fn get_attestation(&self, _user_data: Vec<u8>) -> Result<AttestationReport, VmError> {
        Err(WorkerdVmError::AttestationNotSupported.into())
    }

    #[instrument(level = "info", skip(self), fields(id = %id), err)]
    async fn get_worker_status(&self, id: String) -> Result<WorkerStatus, VmError> {
        let worker = self
            .workers
            .get(&id)
            .ok_or_else(|| WorkerdVmError::WorkerNotFound(id))?;
        Ok(worker.get_status())
    }

    #[instrument(level = "info", skip(self), err)]
    async fn list_running_workers(&self) -> Result<Vec<String>, VmError> {
        let running_ids = self
            .workers
            .iter()
            .filter(|entry| entry.value().get_status() == WorkerStatus::Running)
            .map(|entry| entry.key().clone())
            .collect();
        Ok(running_ids)
    }

    #[instrument(level = "info", skip(self), fields(id = %id), err)]
    async fn get_worker_logs(&self, id: String) -> Result<String, VmError> {
        let worker = self
            .workers
            .get(&id)
            .ok_or_else(|| WorkerdVmError::WorkerNotFound(id))?;

        let logs_content = worker.logs.lock().await.clone();
        Ok(logs_content)
    }

    async fn stream_worker_logs(
        &self,
        id: String,
        tail_lines: u32,
        follow: bool,
    ) -> Result<ReceiverStream<Result<StreamWorkerLogsResponse, tonic::Status>>, VmError> {
        debug!(
            "Streaming logs for worker {}, tail_lines: {}, follow: {}",
            id, tail_lines, follow
        );

        let (tx, rx) = mpsc::channel::<Result<StreamWorkerLogsResponse, tonic::Status>>(1000);

        // Get historical logs if requested
        let tail_lines_opt = if tail_lines > 0 {
            Some(tail_lines as usize)
        } else {
            None
        };
        if let Some(logs) = self.log_manager.get_worker_logs(&id, tail_lines_opt) {
            for entry in logs {
                // Convert Instant to Unix timestamp milliseconds
                let timestamp_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;

                let response = StreamWorkerLogsResponse {
                    log_line: entry.line,
                    timestamp_ms,
                    is_historical: true,
                };
                if let Err(_) = tx.send(Ok(response)).await {
                    warn!("Log stream receiver dropped during historical logs");
                    break;
                }
            }
        } else {
            // Worker not found
            return Err(WorkerdVmError::WorkerNotFound(id).into());
        }

        // If follow is true and worker is active, stream new logs
        if follow {
            if let Some(mut streamer) = self.log_manager.create_log_streamer(&id) {
                tokio::spawn(async move {
                    while let Some(entry) = streamer.next_log().await {
                        let timestamp_ms = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;

                        let response = StreamWorkerLogsResponse {
                            log_line: entry.line,
                            timestamp_ms,
                            is_historical: false,
                        };
                        if let Err(_) = tx.send(Ok(response)).await {
                            debug!("Log stream receiver dropped, stopping streaming");
                            break;
                        }
                    }
                });
            }
        }

        Ok(ReceiverStream::new(rx))
    }

    #[instrument(level = "info", skip(self), fields(id = %id), err)]
    async fn probe_worker(&self, id: String) -> Result<(WorkerStatus, String), VmError> {
        self.probe_worker_internal(&id).await.map_err(|e| e.into())
    }
}

#[cfg(test)]
mod tests;
