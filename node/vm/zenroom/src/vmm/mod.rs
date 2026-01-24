use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::{
        Arc,
        atomic::{AtomicI32, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::Utc;
use dashmap::DashMap;
use hkdf::Hkdf;
use nxcc_interface::{
    proto::vm::{
        Header as ProtoHeader, HttpRequest as ProtoHttpRequest, HttpResponse as ProtoHttpResponse,
        Limits, StreamWorkerLogsResponse, TrustedConfig, UntrustedConfig, WorkerStatus,
    },
    types::{
        attestation::AttestationBundle,
        worker::events::{EventPayload, Web3Log},
    },
};
use nxcc_vm_base::{
    logging::VmmLogManager,
    server::{VmError, VmRuntime},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::mpsc,
    time::sleep,
};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{instrument, warn};
use url::Url;
use uuid::Uuid;

use crate::{config::ZenroomConfig, errors::ZenroomVmError};

const VM_ID: &str = "nxcc/zenroom";
const DEFAULT_HANDLER_NAME: &str = "fetch";

#[derive(Clone)]
pub struct ZenroomVmm {
    workers: Arc<DashMap<String, Arc<WorkerInstance>>>,
    config: ZenroomConfig,
    log_manager: Arc<VmmLogManager>,
    http_client: reqwest::Client,
}

impl ZenroomVmm {
    pub fn new(config: ZenroomConfig) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.postback_timeout_ms))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            workers: Arc::new(DashMap::new()),
            config,
            log_manager: VmmLogManager::new(),
            http_client,
        }
    }

    async fn build_invocation_envelope<'a>(
        &self,
        worker: &WorkerInstance,
        invocation: InvocationInput<'a>,
    ) -> InvocationEnvelope {
        let start_time = Instant::now();

        let nxcc = match invocation {
            InvocationInput::Event { handler, payload } => {
                build_nxcc_event(worker, &handler, &payload)
            }
            InvocationInput::Http { request } => build_nxcc_http(worker, &request),
        };

        let data_value = merge_data(
            worker.config.inputs.data.clone(),
            nxcc,
            worker.config.inputs.merge_strategy,
        );

        let mut keys_value = worker.config.inputs.keys.clone();
        let mut total_secret_bytes = 0usize;
        if let Err(err) = inject_secrets(
            &mut keys_value,
            &worker.trusted_secrets,
            &self.config,
            &mut total_secret_bytes,
        ) {
            return InvocationEnvelope::error(
                ErrorInfo::new("secret_injection_failed", err),
                start_time.elapsed(),
            );
        }

        let conf = match build_conf(&worker.config.conf, &self.config) {
            Ok(conf) => conf,
            Err(err) => {
                return InvocationEnvelope::error(
                    ErrorInfo::new("invalid_conf", err),
                    start_time.elapsed(),
                );
            }
        };

        let exec_path = match worker.config.mode {
            ZenroomMode::Zencode => self.config.zencode_exec_path.clone(),
            ZenroomMode::Lua => self.config.lua_exec_path.clone(),
        };

        let mut timeout_ms = self.config.exec_timeout_ms;
        if let Some(limits) = worker.limits.as_ref()
            && limits.max_runtime_seconds > 0
        {
            timeout_ms = timeout_ms.min(limits.max_runtime_seconds.saturating_mul(1000));
        }

        let exec_outcome = execute_zenroom(
            &exec_path,
            &conf,
            &worker.script,
            &keys_value,
            &data_value,
            &worker.config.inputs.extra,
            &worker.config.inputs.context,
            timeout_ms,
            self.config.max_stdout_bytes,
            self.config.max_stderr_bytes,
        )
        .await;

        let mut envelope = build_envelope_from_exec(
            exec_outcome.output,
            exec_outcome.error,
            start_time.elapsed(),
            worker.config.output.format,
        );

        if !envelope.ok {
            return envelope;
        }

        let selected_output = match compute_selected_output(
            envelope.zenroom.stdout_json.as_ref(),
            &worker.config.output,
            worker.config.postback.required,
        ) {
            Ok(selected) => selected,
            Err(err) => {
                envelope.ok = false;
                envelope.error = Some(err);
                return envelope;
            }
        };

        envelope.selected_output = selected_output.clone();

        if worker.config.postback.enabled {
            let postback_results = match self
                .run_postbacks(&worker.config.postback, &envelope.zenroom, &selected_output)
                .await
            {
                Ok(results) => results,
                Err(err) => {
                    envelope.ok = false;
                    envelope.error = Some(err);
                    return envelope;
                }
            };

            let required_failed =
                worker.config.postback.required && postback_results.iter().any(|result| !result.ok);
            if required_failed {
                envelope.ok = false;
                envelope.error = Some(ErrorInfo::new(
                    "postback_required_failed",
                    "Required postback target failed",
                ));
            }
            envelope.postbacks = postback_results;
        }

        envelope
    }

    async fn run_postbacks(
        &self,
        postback: &PostbackConfig,
        zenroom: &ZenroomResult,
        selected_output: &Option<Value>,
    ) -> Result<Vec<PostbackResult>, ErrorInfo> {
        if !self.config.postback_enabled {
            return Err(ErrorInfo::new(
                "postback_disabled",
                "Postback is disabled by operator configuration",
            ));
        }

        if postback.targets.is_empty() {
            if postback.required {
                return Err(ErrorInfo::new(
                    "postback_required_missing",
                    "Postback required but no targets configured",
                ));
            }
            return Ok(Vec::new());
        }

        let mut results = Vec::with_capacity(postback.targets.len());
        for (index, target) in postback.targets.iter().enumerate() {
            let result = match target {
                PostbackTarget::Http(target) => {
                    self.execute_http_postback(
                        index,
                        target,
                        zenroom,
                        selected_output,
                        &postback.retries,
                    )
                    .await
                }
                PostbackTarget::EvmJsonRpc(target) => {
                    self.execute_evm_postback(
                        index,
                        target,
                        zenroom,
                        selected_output,
                        &postback.retries,
                    )
                    .await
                }
            };
            results.push(result);
        }

        Ok(results)
    }

    async fn execute_http_postback(
        &self,
        index: usize,
        target: &HttpPostbackTarget,
        zenroom: &ZenroomResult,
        selected_output: &Option<Value>,
        retries: &PostbackRetryConfig,
    ) -> PostbackResult {
        let start = Instant::now();
        let url = match Url::parse(&target.url) {
            Ok(url) => url,
            Err(err) => {
                return PostbackResult::failed(
                    index,
                    "http",
                    start.elapsed(),
                    ErrorInfo::new("invalid_url", err.to_string()),
                );
            }
        };

        if let Err(err) = validate_postback_url(&url, &self.config).await {
            return PostbackResult::failed(index, "http", start.elapsed(), err);
        }

        let body_selection =
            match select_postback_body(target.body.as_ref(), zenroom, selected_output) {
                Ok(body) => body,
                Err(err) => {
                    return PostbackResult::failed(index, "http", start.elapsed(), err);
                }
            };

        let method = target.method.clone().unwrap_or_else(|| "POST".to_string());
        let max_attempts = retries.max.saturating_add(1);
        let mut attempt = 0;

        loop {
            let mut request = self
                .http_client
                .request(method.parse().unwrap_or(reqwest::Method::POST), url.clone());

            if let Some(headers) = &target.headers {
                for (key, value) in headers {
                    request = request.header(key, value);
                }
            }

            match &body_selection {
                SelectedBody::Json(value) => {
                    request = request.json(value);
                }
                SelectedBody::Raw(bytes) => {
                    request = request.body(bytes.clone());
                }
            }

            match request.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let ok = status.is_success();
                    let status_code = status.as_u16();
                    let body_bytes = resp.bytes().await.unwrap_or_default();
                    let snippet = truncate_bytes(&body_bytes, 512);
                    return PostbackResult {
                        target_index: index,
                        kind: "http".to_string(),
                        ok,
                        status: Some(status_code),
                        duration_ms: start.elapsed().as_millis() as u64,
                        response_body_truncated: snippet,
                        error: None,
                    };
                }
                Err(err) => {
                    attempt += 1;
                    if attempt >= max_attempts {
                        return PostbackResult::failed(
                            index,
                            "http",
                            start.elapsed(),
                            ErrorInfo::new("postback_failed", err.to_string()),
                        );
                    }
                    sleep(Duration::from_millis(retries.backoff_ms)).await;
                }
            }
        }
    }

    async fn execute_evm_postback(
        &self,
        index: usize,
        target: &EvmJsonRpcTarget,
        zenroom: &ZenroomResult,
        selected_output: &Option<Value>,
        retries: &PostbackRetryConfig,
    ) -> PostbackResult {
        let start = Instant::now();
        let url = match Url::parse(&target.url) {
            Ok(url) => url,
            Err(err) => {
                return PostbackResult::failed(
                    index,
                    "evm_jsonrpc",
                    start.elapsed(),
                    ErrorInfo::new("invalid_url", err.to_string()),
                );
            }
        };

        if let Err(err) = validate_postback_url(&url, &self.config).await {
            return PostbackResult::failed(index, "evm_jsonrpc", start.elapsed(), err);
        }

        let mut params = Vec::new();
        for param in &target.params {
            match select_postback_body(Some(param), zenroom, selected_output) {
                Ok(SelectedBody::Json(value)) => params.push(value),
                Ok(SelectedBody::Raw(bytes)) => {
                    params.push(Value::String(String::from_utf8_lossy(&bytes).to_string()))
                }
                Err(err) => {
                    return PostbackResult::failed(index, "evm_jsonrpc", start.elapsed(), err);
                }
            }
        }

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": target.method,
            "params": params,
        });

        let max_attempts = retries.max.saturating_add(1);
        let mut attempt = 0;

        loop {
            match self
                .http_client
                .post(url.clone())
                .json(&payload)
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    let ok = status.is_success();
                    let status_code = status.as_u16();
                    let body_bytes = resp.bytes().await.unwrap_or_default();
                    let snippet = truncate_bytes(&body_bytes, 512);
                    return PostbackResult {
                        target_index: index,
                        kind: "evm_jsonrpc".to_string(),
                        ok,
                        status: Some(status_code),
                        duration_ms: start.elapsed().as_millis() as u64,
                        response_body_truncated: snippet,
                        error: None,
                    };
                }
                Err(err) => {
                    attempt += 1;
                    if attempt >= max_attempts {
                        return PostbackResult::failed(
                            index,
                            "evm_jsonrpc",
                            start.elapsed(),
                            ErrorInfo::new("postback_failed", err.to_string()),
                        );
                    }
                    sleep(Duration::from_millis(retries.backoff_ms)).await;
                }
            }
        }
    }

    fn append_log(&self, worker: &WorkerInstance, line: &str) {
        worker.log_buffer.write_log(line.to_string());
    }
}

#[derive(Debug)]
struct WorkerInstance {
    instance_id: String,
    worker_id: String,
    script: String,
    config: WorkerConfig,
    trusted_secrets: HashMap<String, Vec<u8>>,
    limits: Option<Limits>,
    status: AtomicI32,
    log_buffer: Arc<nxcc_vm_base::logging::LogBuffer>,
}

impl WorkerInstance {
    fn get_status(&self) -> WorkerStatus {
        let status_val = self.status.load(Ordering::SeqCst);
        WorkerStatus::try_from(status_val).unwrap_or(WorkerStatus::Unspecified)
    }

    fn set_status(&self, new_status: WorkerStatus) {
        self.status.store(new_status as i32, Ordering::SeqCst);
    }
}

#[derive(Debug, Deserialize, Default, Clone)]
struct UserdataRoot {
    #[serde(default)]
    zenroom: Option<ZenroomUserdata>,
    #[serde(default)]
    postback: Option<PostbackUserdata>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct ZenroomUserdata {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    conf: Option<String>,
    #[serde(default)]
    inputs: Option<ZenroomInputs>,
    #[serde(default)]
    output: Option<ZenroomOutput>,
    #[serde(default)]
    http: Option<ZenroomHttp>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct ZenroomInputs {
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    keys: Option<Value>,
    #[serde(default)]
    extra: Option<Value>,
    #[serde(default)]
    context: Option<Value>,
    #[serde(default)]
    merge_strategy: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct ZenroomOutput {
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    json_pointer: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct ZenroomHttp {
    #[serde(default)]
    response_mode: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct PostbackUserdata {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    required: Option<bool>,
    #[serde(default)]
    retries: Option<PostbackRetries>,
    #[serde(default)]
    targets: Option<Vec<PostbackTarget>>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct PostbackRetries {
    #[serde(default)]
    max: Option<u32>,
    #[serde(default)]
    backoff_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PostbackTarget {
    Http(HttpPostbackTarget),
    #[serde(rename = "evm_jsonrpc")]
    EvmJsonRpc(EvmJsonRpcTarget),
}

#[derive(Debug, Deserialize, Clone)]
struct HttpPostbackTarget {
    url: String,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    headers: Option<HashMap<String, String>>,
    #[serde(default)]
    body: Option<PostbackBodySelector>,
}

#[derive(Debug, Deserialize, Clone)]
struct EvmJsonRpcTarget {
    url: String,
    method: String,
    #[serde(default)]
    params: Vec<PostbackBodySelector>,
}

#[derive(Debug, Deserialize, Clone)]
struct PostbackBodySelector {
    from: String,
    #[serde(default)]
    json_pointer: Option<String>,
}

#[derive(Debug, Clone)]
struct WorkerConfig {
    mode: ZenroomMode,
    conf: String,
    inputs: NormalizedInputs,
    output: NormalizedOutput,
    http: NormalizedHttp,
    postback: PostbackConfig,
}

#[derive(Debug, Clone)]
struct NormalizedInputs {
    data: Value,
    keys: Value,
    extra: Value,
    context: Value,
    merge_strategy: MergeStrategy,
}

#[derive(Debug, Clone)]
struct NormalizedOutput {
    format: OutputFormat,
    json_pointer: Option<String>,
}

#[derive(Debug, Clone)]
struct NormalizedHttp {
    response_mode: HttpResponseMode,
}

#[derive(Debug, Clone)]
struct PostbackConfig {
    enabled: bool,
    required: bool,
    retries: PostbackRetryConfig,
    targets: Vec<PostbackTarget>,
}

#[derive(Debug, Clone)]
struct PostbackRetryConfig {
    max: u32,
    backoff_ms: u64,
}

#[derive(Debug, Clone, Copy)]
enum ZenroomMode {
    Zencode,
    Lua,
}

#[derive(Debug, Clone, Copy)]
enum MergeStrategy {
    Merge,
    Wrap,
}

#[derive(Debug, Clone, Copy)]
enum OutputFormat {
    Json,
    Raw,
}

#[derive(Debug, Clone, Copy)]
enum HttpResponseMode {
    Envelope,
    Stdout,
    SelectedOutput,
}

#[derive(Debug)]
enum InvocationInput<'a> {
    Event {
        handler: String,
        payload: EventPayload<'a>,
    },
    Http {
        request: ProtoHttpRequest,
    },
}

#[derive(Debug, Serialize)]
struct InvocationEnvelope {
    ok: bool,
    error: Option<ErrorInfo>,
    zenroom: ZenroomResult,
    selected_output: Option<Value>,
    postbacks: Vec<PostbackResult>,
    timing: TimingInfo,
}

impl InvocationEnvelope {
    fn error(err: ErrorInfo, duration: Duration) -> Self {
        Self {
            ok: false,
            error: Some(err),
            zenroom: ZenroomResult::empty(),
            selected_output: None,
            postbacks: Vec::new(),
            timing: TimingInfo::from_duration(duration),
        }
    }
}

#[derive(Debug, Serialize, Clone)]
struct ZenroomResult {
    exit_code: i32,
    stdout: String,
    stderr: String,
    stdout_json: Option<Value>,
    stdout_overflowed: bool,
    stderr_overflowed: bool,
}

impl ZenroomResult {
    fn empty() -> Self {
        Self {
            exit_code: -1,
            stdout: String::new(),
            stderr: String::new(),
            stdout_json: None,
            stdout_overflowed: false,
            stderr_overflowed: false,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
struct PostbackResult {
    target_index: usize,
    kind: String,
    ok: bool,
    status: Option<u16>,
    duration_ms: u64,
    response_body_truncated: Option<String>,
    error: Option<ErrorInfo>,
}

impl PostbackResult {
    fn failed(index: usize, kind: &str, duration: Duration, error: ErrorInfo) -> Self {
        Self {
            target_index: index,
            kind: kind.to_string(),
            ok: false,
            status: None,
            duration_ms: duration.as_millis() as u64,
            response_body_truncated: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Serialize, Clone)]
struct TimingInfo {
    total_ms: u64,
}

impl TimingInfo {
    fn from_duration(duration: Duration) -> Self {
        Self {
            total_ms: duration.as_millis() as u64,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
struct ErrorInfo {
    code: String,
    message: String,
}

impl ErrorInfo {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug)]
struct ExecOutput {
    exit_code: i32,
    stdout: String,
    stderr: String,
    stdout_json: Option<Value>,
    stdout_overflowed: bool,
    stderr_overflowed: bool,
}

#[derive(Debug)]
struct ExecOutcome {
    output: ExecOutput,
    error: Option<ErrorInfo>,
}

#[derive(Debug)]
enum OutputReadResult {
    Ok(Vec<u8>),
    Overflow(Vec<u8>),
    Err(String),
}

#[derive(Debug)]
enum SelectedBody {
    Json(Value),
    Raw(Vec<u8>),
}

#[async_trait]
impl VmRuntime for ZenroomVmm {
    #[instrument(level = "info", skip(self, worker_code, untrusted_config, trusted_config), fields(worker_id = %worker_id))]
    async fn start_worker(
        &self,
        worker_id: String,
        worker_code: Vec<u8>,
        untrusted_config: UntrustedConfig,
        trusted_config: TrustedConfig,
    ) -> Result<String, VmError> {
        if worker_code.len() > self.config.max_script_bytes {
            return Err(VmError::new(format!(
                "Script size {} exceeds max {}",
                worker_code.len(),
                self.config.max_script_bytes
            )));
        }

        let script = String::from_utf8(worker_code)
            .map_err(|e| VmError::new(format!("Worker code must be UTF-8: {e}")))?;

        let userdata_json = untrusted_config.userdata_json.trim();
        let userdata: UserdataRoot = if userdata_json.is_empty() {
            UserdataRoot::default()
        } else {
            serde_json::from_str(userdata_json)
                .map_err(|e| VmError::new(format!("Invalid userdata JSON: {e}")))?
        };

        let config = normalize_worker_config(&userdata)?;

        let secrets = trusted_config.secrets.clone();
        let total_secret_bytes: usize = secrets.values().map(|v| v.len()).sum();
        if total_secret_bytes > self.config.max_total_secrets_bytes {
            return Err(VmError::new(format!(
                "Total secrets size {} exceeds max {}",
                total_secret_bytes, self.config.max_total_secrets_bytes
            )));
        }

        let instance_id = Uuid::new_v4().to_string();
        let log_buffer = self.log_manager.register_worker(instance_id.clone());

        let worker = WorkerInstance {
            instance_id: instance_id.clone(),
            worker_id,
            script,
            config,
            trusted_secrets: secrets,
            limits: trusted_config.limits,
            status: AtomicI32::new(WorkerStatus::Running as i32),
            log_buffer,
        };

        self.workers.insert(instance_id.clone(), Arc::new(worker));

        Ok(instance_id)
    }

    #[instrument(level = "info", skip(self), fields(id = %id), err)]
    async fn stop_worker(&self, id: String) -> Result<(), VmError> {
        if let Some((_, worker)) = self.workers.remove(&id) {
            worker.set_status(WorkerStatus::Stopped);
            self.log_manager.terminate_worker(&id);
            Ok(())
        } else {
            Err(ZenroomVmError::WorkerNotFound(id).into())
        }
    }

    #[instrument(level = "info", skip(self, payload), fields(id = %id, handler = %handler_name))]
    async fn invoke_worker(
        &self,
        id: String,
        handler_name: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, VmError> {
        let worker = self
            .workers
            .get(&id)
            .ok_or_else(|| ZenroomVmError::WorkerNotFound(id.clone()))?
            .clone();

        let status = worker.get_status();
        if status != WorkerStatus::Running {
            return Err(ZenroomVmError::WorkerNotRunnable(status).into());
        }

        let invocation_request: VmEventInvocationRequest<'_> =
            match serde_json::from_slice(&payload) {
                Ok(req) => req,
                Err(err) => {
                    let envelope = InvocationEnvelope::error(
                        ErrorInfo::new("invalid_event_payload", err.to_string()),
                        Duration::from_millis(0),
                    );
                    return Ok(serde_json::to_vec(&envelope).unwrap());
                }
            };

        let VmEventInvocationRequest {
            handler: payload_handler,
            event_payload,
        } = invocation_request;
        if payload_handler != handler_name {
            warn!(
                expected = %handler_name,
                payload_handler = %payload_handler,
                "Invocation handler mismatch"
            );
        }

        let envelope = self
            .build_invocation_envelope(
                &worker,
                InvocationInput::Event {
                    handler: handler_name,
                    payload: event_payload,
                },
            )
            .await;

        if !envelope.zenroom.stdout.is_empty() {
            self.append_log(&worker, &format!("stdout: {}", envelope.zenroom.stdout));
        }
        if !envelope.zenroom.stderr.is_empty() {
            self.append_log(&worker, &format!("stderr: {}", envelope.zenroom.stderr));
        }
        for result in &envelope.postbacks {
            let line = if result.ok {
                format!(
                    "postback[{}]: {} status={} duration_ms={}",
                    result.target_index,
                    result.kind,
                    result.status.unwrap_or(0),
                    result.duration_ms
                )
            } else {
                format!(
                    "postback[{}]: {} failed ({})",
                    result.target_index,
                    result.kind,
                    result
                        .error
                        .as_ref()
                        .map(|e| e.message.clone())
                        .unwrap_or_else(|| "unknown error".to_string())
                )
            };
            self.append_log(&worker, &line);
        }

        Ok(serde_json::to_vec(&envelope).unwrap())
    }

    #[instrument(level = "info", skip(self, request), fields(id = %id, uri = %request.uri))]
    async fn invoke_http(
        &self,
        id: String,
        request: ProtoHttpRequest,
    ) -> Result<ProtoHttpResponse, VmError> {
        let worker = self
            .workers
            .get(&id)
            .ok_or_else(|| ZenroomVmError::WorkerNotFound(id.clone()))?
            .clone();

        let status = worker.get_status();
        if status != WorkerStatus::Running {
            return Err(ZenroomVmError::WorkerNotRunnable(status).into());
        }

        let envelope = self
            .build_invocation_envelope(
                &worker,
                InvocationInput::Http {
                    request: request.clone(),
                },
            )
            .await;

        let (status_code, body, content_type) = if envelope.ok {
            match worker.config.http.response_mode {
                HttpResponseMode::Envelope => (
                    200,
                    serde_json::to_vec(&envelope).unwrap(),
                    "application/json".to_string(),
                ),
                HttpResponseMode::Stdout => {
                    let stdout = envelope.zenroom.stdout.clone();
                    let parsed = serde_json::from_str::<Value>(&stdout).ok();
                    let content_type = if parsed.is_some() {
                        "application/json".to_string()
                    } else {
                        "text/plain".to_string()
                    };
                    (200, stdout.into_bytes(), content_type)
                }
                HttpResponseMode::SelectedOutput => {
                    let output = envelope
                        .selected_output
                        .clone()
                        .or_else(|| envelope.zenroom.stdout_json.clone())
                        .unwrap_or(Value::Null);
                    (
                        200,
                        serde_json::to_vec(&output).unwrap(),
                        "application/json".to_string(),
                    )
                }
            }
        } else {
            (
                500,
                serde_json::to_vec(&envelope).unwrap(),
                "application/json".to_string(),
            )
        };

        Ok(ProtoHttpResponse {
            status_code,
            headers: vec![ProtoHeader {
                key: "content-type".to_string(),
                value: content_type.as_bytes().to_vec(),
            }],
            body,
        })
    }

    #[instrument(level = "info", skip(self, _user_data), err)]
    async fn get_attestation(&self, _user_data: Vec<u8>) -> Result<AttestationBundle, VmError> {
        Err(ZenroomVmError::AttestationNotSupported.into())
    }

    #[instrument(level = "info", skip(self), fields(id = %id), err)]
    async fn get_worker_status(&self, id: String) -> Result<WorkerStatus, VmError> {
        let worker = self
            .workers
            .get(&id)
            .ok_or_else(|| ZenroomVmError::WorkerNotFound(id))?;
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
        if let Some(entries) = self.log_manager.get_worker_logs(&id, None) {
            let joined = entries
                .iter()
                .map(|entry| entry.line.clone())
                .collect::<Vec<_>>()
                .join("\n");
            return Ok(joined);
        }
        Err(ZenroomVmError::WorkerNotFound(id).into())
    }

    async fn stream_worker_logs(
        &self,
        id: String,
        tail_lines: u32,
        follow: bool,
    ) -> Result<ReceiverStream<Result<StreamWorkerLogsResponse, tonic::Status>>, VmError> {
        let (tx, rx) = mpsc::channel::<Result<StreamWorkerLogsResponse, tonic::Status>>(1000);

        let tail_lines_opt = if tail_lines > 0 {
            Some(tail_lines as usize)
        } else {
            None
        };
        if let Some(logs) = self.log_manager.get_worker_logs(&id, tail_lines_opt) {
            for entry in logs {
                let timestamp_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let response = StreamWorkerLogsResponse {
                    log_line: entry.line,
                    timestamp_ms,
                    is_historical: true,
                };
                if tx.send(Ok(response)).await.is_err() {
                    break;
                }
            }
        } else {
            return Err(ZenroomVmError::WorkerNotFound(id).into());
        }

        if follow && let Some(mut streamer) = self.log_manager.create_log_streamer(&id) {
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
                    if tx.send(Ok(response)).await.is_err() {
                        break;
                    }
                }
            });
        }

        Ok(ReceiverStream::new(rx))
    }

    #[instrument(level = "info", skip(self), fields(id = %id), err)]
    async fn probe_worker(&self, id: String) -> Result<(WorkerStatus, String), VmError> {
        let worker = self
            .workers
            .get(&id)
            .ok_or_else(|| ZenroomVmError::WorkerNotFound(id.clone()))?;

        let status = worker.get_status();
        Ok((status, format!("Worker status: {:?}", status)))
    }
}

#[derive(Debug, Deserialize)]
struct VmEventInvocationRequest<'a> {
    handler: String,
    #[serde(borrow)]
    event_payload: EventPayload<'a>,
}

fn normalize_worker_config(userdata: &UserdataRoot) -> Result<WorkerConfig, VmError> {
    let zenroom = userdata.zenroom.clone().unwrap_or_default();
    let postback = userdata.postback.clone().unwrap_or_default();

    let mode = match zenroom
        .mode
        .as_deref()
        .unwrap_or("zencode")
        .to_lowercase()
        .as_str()
    {
        "zencode" => ZenroomMode::Zencode,
        "lua" => ZenroomMode::Lua,
        other => return Err(VmError::new(format!("Unsupported zenroom.mode '{other}'"))),
    };

    let inputs = zenroom.inputs.unwrap_or_default();
    let merge_strategy = match inputs
        .merge_strategy
        .as_deref()
        .unwrap_or("merge")
        .to_lowercase()
        .as_str()
    {
        "merge" => MergeStrategy::Merge,
        "wrap" => MergeStrategy::Wrap,
        other => {
            return Err(VmError::new(format!(
                "Unsupported merge_strategy '{other}'"
            )));
        }
    };

    let output = zenroom.output.unwrap_or_default();
    let format = match output
        .format
        .as_deref()
        .unwrap_or("json")
        .to_lowercase()
        .as_str()
    {
        "json" => OutputFormat::Json,
        "raw" => OutputFormat::Raw,
        other => return Err(VmError::new(format!("Unsupported output.format '{other}'"))),
    };

    let http = zenroom.http.unwrap_or_default();
    let response_mode = match http
        .response_mode
        .as_deref()
        .unwrap_or("envelope")
        .to_lowercase()
        .as_str()
    {
        "envelope" => HttpResponseMode::Envelope,
        "stdout" => HttpResponseMode::Stdout,
        "selected_output" => HttpResponseMode::SelectedOutput,
        other => {
            return Err(VmError::new(format!(
                "Unsupported http.response_mode '{other}'"
            )));
        }
    };

    let conf = zenroom.conf.unwrap_or_else(|| "debug=0".to_string());

    let normalized_inputs = NormalizedInputs {
        data: inputs.data.unwrap_or_else(|| json!({})),
        keys: inputs.keys.unwrap_or_else(|| json!({})),
        extra: inputs.extra.unwrap_or_else(|| json!({})),
        context: inputs.context.unwrap_or_else(|| json!({})),
        merge_strategy,
    };

    let retry_config = postback.retries.unwrap_or_default();
    let postback_config = PostbackConfig {
        enabled: postback.enabled.unwrap_or(false),
        required: postback.required.unwrap_or(false),
        retries: PostbackRetryConfig {
            max: retry_config.max.unwrap_or(0),
            backoff_ms: retry_config.backoff_ms.unwrap_or(250),
        },
        targets: postback.targets.unwrap_or_default(),
    };

    Ok(WorkerConfig {
        mode,
        conf,
        inputs: normalized_inputs,
        output: NormalizedOutput {
            format,
            json_pointer: output.json_pointer,
        },
        http: NormalizedHttp { response_mode },
        postback: postback_config,
    })
}

fn build_conf(conf: &str, config: &ZenroomConfig) -> Result<String, String> {
    let adjusted = if config.allow_debug_conf {
        conf.to_string()
    } else {
        let mut entries = Vec::new();
        for part in conf.split(',') {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                continue;
            }
            let mut iter = trimmed.splitn(2, '=');
            let key = iter.next().unwrap_or("");
            if key == "debug" {
                continue;
            }
            entries.push(trimmed.to_string());
        }
        entries.push("debug=0".to_string());
        entries.join(",")
    };

    if adjusted.len() > config.max_conf_bytes {
        return Err(format!(
            "Zenroom conf length {} exceeds max {}",
            adjusted.len(),
            config.max_conf_bytes
        ));
    }

    Ok(adjusted)
}

fn merge_data(user_data: Value, nxcc: Value, strategy: MergeStrategy) -> Value {
    match strategy {
        MergeStrategy::Wrap => json!({ "input": user_data, "nxcc": nxcc }),
        MergeStrategy::Merge => {
            if let Value::Object(mut map) = user_data {
                if map.contains_key("nxcc") {
                    json!({ "input": Value::Object(map), "nxcc": nxcc })
                } else {
                    map.insert("nxcc".to_string(), nxcc);
                    Value::Object(map)
                }
            } else {
                json!({ "input": user_data, "nxcc": nxcc })
            }
        }
    }
}

fn build_nxcc_event(worker: &WorkerInstance, handler: &str, payload: &EventPayload<'_>) -> Value {
    let (kind, payload_value) = match payload {
        EventPayload::Web3Log(log) => ("web3_log", web3_log_to_json(log)),
        EventPayload::Launch => ("launch", json!({})),
        EventPayload::Scheduled => ("scheduled", json!({})),
        EventPayload::HttpRequest => ("unknown", json!({})),
        EventPayload::_Phantom(_) => ("unknown", json!({})),
    };

    json!({
        "worker": {
            "worker_id": worker.worker_id.clone(),
            "instance_id": worker.instance_id.clone(),
            "vm": VM_ID,
        },
        "invocation": {
            "type": "event",
            "handler": handler,
            "received_at": Utc::now().to_rfc3339(),
        },
        "event": {
            "kind": kind,
            "payload": payload_value,
        }
    })
}

fn build_nxcc_http(worker: &WorkerInstance, request: &ProtoHttpRequest) -> Value {
    let mut headers_map: HashMap<String, Vec<String>> = HashMap::new();
    for header in &request.headers {
        let key = header.key.to_ascii_lowercase();
        let value = String::from_utf8_lossy(&header.value).to_string();
        headers_map.entry(key).or_default().push(value);
    }

    let mut normalized_headers = HashMap::new();
    for (key, values) in headers_map.iter() {
        normalized_headers.insert(key.clone(), values.join(", "));
    }

    let scheme = headers_map
        .get("x-forwarded-proto")
        .and_then(|values| values.first())
        .map(|value| value.split(',').next().unwrap_or(value).to_string())
        .unwrap_or_else(|| "http".to_string());

    let host = headers_map
        .get("x-forwarded-host")
        .and_then(|values| values.first())
        .or_else(|| headers_map.get("host").and_then(|values| values.first()))
        .cloned()
        .unwrap_or_else(|| "localhost".to_string());

    let url = format!("{}://{}{}", scheme, host, request.uri.as_str());
    let body_b64 = BASE64_STANDARD.encode(&request.body);

    json!({
        "worker": {
            "worker_id": worker.worker_id.clone(),
            "instance_id": worker.instance_id.clone(),
            "vm": VM_ID,
        },
        "invocation": {
            "type": "http",
            "handler": DEFAULT_HANDLER_NAME,
            "received_at": Utc::now().to_rfc3339(),
        },
        "http": {
            "method": request.method.clone(),
            "url": url,
            "headers": normalized_headers,
            "body_base64": body_b64,
        }
    })
}

fn web3_log_to_json(log: &Web3Log) -> Value {
    json!({
        "address": format!("{:#x}", log.address),
        "topics": log.topics.iter().map(|t| format!("{:#x}", t)).collect::<Vec<_>>(),
        "data": format!("0x{}", hex::encode(&log.data)),
        "block_hash": log.block_hash.map(|h| format!("{:#x}", h)),
        "block_number": log.block_number,
        "transaction_hash": log.transaction_hash.map(|h| format!("{:#x}", h)),
        "transaction_index": log.transaction_index,
        "log_index": log.log_index,
        "removed": log.removed,
    })
}

fn inject_secrets(
    value: &mut Value,
    secrets: &HashMap<String, Vec<u8>>,
    config: &ZenroomConfig,
    total_bytes: &mut usize,
) -> Result<(), String> {
    match value {
        Value::Array(values) => {
            for entry in values {
                inject_secrets(entry, secrets, config, total_bytes)?;
            }
            Ok(())
        }
        Value::Object(map) => {
            if let Some(secret_name) = map.get("$secret") {
                let secret_name = secret_name
                    .as_str()
                    .ok_or_else(|| "Invalid $secret value".to_string())?;
                let secret = secrets
                    .get(secret_name)
                    .ok_or_else(|| format!("Missing secret '{}'", secret_name))?;

                let encoding = map
                    .get("encoding")
                    .and_then(Value::as_str)
                    .unwrap_or("base64")
                    .to_lowercase();

                let kdf_config = map.get("kdf");
                if config.require_kdf && kdf_config.is_none() {
                    return Err("HKDF is required for secret injection".to_string());
                }

                let derived = if let Some(kdf_value) = kdf_config {
                    derive_secret(secret, kdf_value, config)?
                } else {
                    secret.clone()
                };

                *total_bytes = total_bytes.saturating_add(derived.len());
                if *total_bytes > config.max_total_secrets_bytes {
                    return Err(format!(
                        "Total derived secrets size {} exceeds max {}",
                        total_bytes, config.max_total_secrets_bytes
                    ));
                }

                let encoded = match encoding.as_str() {
                    "hex" => hex::encode(derived),
                    "base64" => BASE64_STANDARD.encode(derived),
                    other => return Err(format!("Unsupported encoding '{other}'")),
                };

                *value = Value::String(encoded);
                return Ok(());
            }

            for entry in map.values_mut() {
                inject_secrets(entry, secrets, config, total_bytes)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn derive_secret(
    secret: &[u8],
    kdf_value: &Value,
    config: &ZenroomConfig,
) -> Result<Vec<u8>, String> {
    let kdf_obj = kdf_value
        .as_object()
        .ok_or_else(|| "kdf must be an object".to_string())?;
    let info = kdf_obj.get("info").and_then(Value::as_str).unwrap_or("");
    let len = kdf_obj.get("len").and_then(Value::as_u64).unwrap_or(32) as usize;
    if len > config.max_derived_key_len {
        return Err(format!(
            "Derived key length {} exceeds max {}",
            len, config.max_derived_key_len
        ));
    }

    let salt = kdf_obj.get("salt").and_then(Value::as_str).unwrap_or("");

    let hk = Hkdf::<Sha256>::new(Some(salt.as_bytes()), secret);
    let mut okm = vec![0u8; len];
    hk.expand(info.as_bytes(), &mut okm)
        .map_err(|_| "HKDF expand failed".to_string())?;
    Ok(okm)
}

#[allow(clippy::too_many_arguments)]
async fn execute_zenroom(
    exec_path: &str,
    conf: &str,
    script: &str,
    keys: &Value,
    data: &Value,
    extra: &Value,
    context: &Value,
    timeout_ms: u64,
    max_stdout: usize,
    max_stderr: usize,
) -> ExecOutcome {
    let keys_str = serde_json::to_string(keys).unwrap_or_default();
    let data_str = serde_json::to_string(data).unwrap_or_default();
    let extra_str = serde_json::to_string(extra).unwrap_or_default();
    let context_str = serde_json::to_string(context).unwrap_or_default();

    let mut input = String::new();
    input.push_str(conf);
    input.push('\n');
    input.push_str(&BASE64_STANDARD.encode(script));
    input.push('\n');
    input.push_str(&BASE64_STANDARD.encode(&keys_str));
    input.push('\n');
    input.push_str(&BASE64_STANDARD.encode(&data_str));
    input.push('\n');
    input.push_str(&BASE64_STANDARD.encode(&extra_str));
    input.push('\n');
    input.push_str(&BASE64_STANDARD.encode(&context_str));
    input.push('\n');

    let mut output = ExecOutput {
        exit_code: -1,
        stdout: String::new(),
        stderr: String::new(),
        stdout_json: None,
        stdout_overflowed: false,
        stderr_overflowed: false,
    };
    let mut error: Option<ErrorInfo> = None;

    let mut child = match Command::new(exec_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            error = Some(ErrorInfo::new("exec_failed", err.to_string()));
            return ExecOutcome { output, error };
        }
    };

    if let Some(mut stdin) = child.stdin.take()
        && let Err(err) = stdin.write_all(input.as_bytes()).await
    {
        error = Some(ErrorInfo::new("exec_failed", err.to_string()));
        let _ = child.kill().await;
        return ExecOutcome { output, error };
    }

    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            error = Some(ErrorInfo::new("exec_failed", "missing stdout"));
            let _ = child.kill().await;
            return ExecOutcome { output, error };
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            error = Some(ErrorInfo::new("exec_failed", "missing stderr"));
            let _ = child.kill().await;
            return ExecOutcome { output, error };
        }
    };

    let mut stdout_handle = tokio::spawn(read_with_limit(stdout, max_stdout));
    let mut stderr_handle = tokio::spawn(read_with_limit(stderr, max_stderr));

    let mut exit_status: Option<std::process::ExitStatus> = None;
    let mut stdout_result: Option<OutputReadResult> = None;
    let mut stderr_result: Option<OutputReadResult> = None;

    let timeout = tokio::time::sleep(Duration::from_millis(timeout_ms));
    tokio::pin!(timeout);
    let mut timed_out = false;
    let mut overflow_error: Option<ErrorInfo> = None;

    loop {
        tokio::select! {
            status = child.wait(), if exit_status.is_none() => {
                match status {
                    Ok(status) => exit_status = Some(status),
                    Err(err) => {
                        error = Some(ErrorInfo::new("exec_failed", err.to_string()));
                        break;
                    }
                }
            }
            stdout = &mut stdout_handle, if stdout_result.is_none() => {
                match stdout {
                    Ok(result) => {
                        if matches!(&result, OutputReadResult::Overflow(_)) && overflow_error.is_none() {
                            overflow_error = Some(ErrorInfo::new(
                                "stdout_limit_exceeded",
                                "stdout exceeded configured limit",
                            ));
                            let _ = child.kill().await;
                        }
                        stdout_result = Some(result);
                    }
                    Err(_) => {
                        error = Some(ErrorInfo::new("exec_failed", "stdout task failed"));
                        let _ = child.kill().await;
                        stdout_result = Some(OutputReadResult::Err("stdout task failed".to_string()));
                    }
                }
            }
            stderr = &mut stderr_handle, if stderr_result.is_none() => {
                match stderr {
                    Ok(result) => {
                        if matches!(&result, OutputReadResult::Overflow(_)) && overflow_error.is_none() {
                            overflow_error = Some(ErrorInfo::new(
                                "stderr_limit_exceeded",
                                "stderr exceeded configured limit",
                            ));
                            let _ = child.kill().await;
                        }
                        stderr_result = Some(result);
                    }
                    Err(_) => {
                        error = Some(ErrorInfo::new("exec_failed", "stderr task failed"));
                        let _ = child.kill().await;
                        stderr_result = Some(OutputReadResult::Err("stderr task failed".to_string()));
                    }
                }
            }
            _ = &mut timeout, if !timed_out => {
                timed_out = true;
                error = Some(ErrorInfo::new("timeout", "Zenroom execution timed out"));
                let _ = child.kill().await;
            }
        }

        if stdout_result.is_some() && stderr_result.is_some() && exit_status.is_some() {
            break;
        }

        if timed_out && stdout_result.is_some() && stderr_result.is_some() {
            break;
        }
    }

    let stdout_result = stdout_result.unwrap_or(OutputReadResult::Ok(Vec::new()));
    let stderr_result = stderr_result.unwrap_or(OutputReadResult::Ok(Vec::new()));

    let stdout_overflowed = matches!(&stdout_result, OutputReadResult::Overflow(_));
    let stderr_overflowed = matches!(&stderr_result, OutputReadResult::Overflow(_));

    let stdout_bytes = match stdout_result {
        OutputReadResult::Ok(bytes) | OutputReadResult::Overflow(bytes) => bytes,
        OutputReadResult::Err(err) => {
            error = Some(ErrorInfo::new("exec_failed", err));
            Vec::new()
        }
    };
    let stderr_bytes = match stderr_result {
        OutputReadResult::Ok(bytes) | OutputReadResult::Overflow(bytes) => bytes,
        OutputReadResult::Err(err) => {
            error = Some(ErrorInfo::new("exec_failed", err));
            Vec::new()
        }
    };

    output.stdout_overflowed = stdout_overflowed;
    output.stderr_overflowed = stderr_overflowed;
    output.stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
    output.stderr = String::from_utf8_lossy(&stderr_bytes).to_string();
    output.exit_code = exit_status.and_then(|status| status.code()).unwrap_or(-1);
    output.stdout_json = serde_json::from_str::<Value>(&output.stdout).ok();

    if error.is_none() {
        error = overflow_error;
    }

    ExecOutcome { output, error }
}

async fn read_with_limit<R: AsyncReadExt + Unpin>(
    mut reader: R,
    max_bytes: usize,
) -> OutputReadResult {
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 4096];

    loop {
        let read = match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(err) => return OutputReadResult::Err(err.to_string()),
        };

        if buf.len() + read > max_bytes {
            let remaining = max_bytes.saturating_sub(buf.len());
            buf.extend_from_slice(&chunk[..remaining]);
            return OutputReadResult::Overflow(buf);
        }
        buf.extend_from_slice(&chunk[..read]);
    }

    OutputReadResult::Ok(buf)
}

fn build_envelope_from_exec(
    mut exec: ExecOutput,
    exec_error: Option<ErrorInfo>,
    duration: Duration,
    output_format: OutputFormat,
) -> InvocationEnvelope {
    if matches!(output_format, OutputFormat::Raw) {
        exec.stdout_json = None;
    }

    let mut ok = exec.exit_code == 0 && exec_error.is_none();

    let zenroom = ZenroomResult {
        exit_code: exec.exit_code,
        stdout: exec.stdout,
        stderr: exec.stderr,
        stdout_json: exec.stdout_json,
        stdout_overflowed: exec.stdout_overflowed,
        stderr_overflowed: exec.stderr_overflowed,
    };

    let mut error = exec_error;

    if error.is_none() && !ok {
        error = Some(ErrorInfo::new(
            "zenroom_exit_nonzero",
            format!("Zenroom exited with code {}", zenroom.exit_code),
        ));
        ok = false;
    }

    InvocationEnvelope {
        ok,
        error,
        zenroom,
        selected_output: None,
        postbacks: Vec::new(),
        timing: TimingInfo::from_duration(duration),
    }
}

fn compute_selected_output(
    stdout_json: Option<&Value>,
    output: &NormalizedOutput,
    postback_required: bool,
) -> Result<Option<Value>, ErrorInfo> {
    let Some(pointer) = output.json_pointer.as_deref() else {
        return Ok(None);
    };

    let stdout_json = match stdout_json {
        Some(value) => value,
        None => {
            if postback_required {
                return Err(ErrorInfo::new(
                    "stdout_json_missing",
                    "stdout JSON missing for selected_output",
                ));
            }
            return Ok(None);
        }
    };

    let selected = stdout_json.pointer(pointer).cloned();
    if selected.is_none() && postback_required {
        return Err(ErrorInfo::new(
            "json_pointer_not_found",
            "json_pointer did not resolve",
        ));
    }

    Ok(selected)
}

fn select_postback_body(
    selector: Option<&PostbackBodySelector>,
    zenroom: &ZenroomResult,
    selected_output: &Option<Value>,
) -> Result<SelectedBody, ErrorInfo> {
    let selector = selector
        .ok_or_else(|| ErrorInfo::new("postback_body_missing", "Postback body selector missing"))?;

    match selector.from.as_str() {
        "stdout" => {
            if let Some(pointer) = selector.json_pointer.as_deref() {
                let stdout_json = zenroom.stdout_json.as_ref().ok_or_else(|| {
                    ErrorInfo::new("stdout_json_missing", "stdout_json not available")
                })?;
                let value = stdout_json.pointer(pointer).cloned().ok_or_else(|| {
                    ErrorInfo::new("json_pointer_not_found", "json_pointer did not resolve")
                })?;
                Ok(SelectedBody::Json(value))
            } else if let Some(stdout_json) = zenroom.stdout_json.as_ref() {
                Ok(SelectedBody::Json(stdout_json.clone()))
            } else {
                Ok(SelectedBody::Raw(zenroom.stdout.as_bytes().to_vec()))
            }
        }
        "selected_output" => {
            let value = selected_output
                .clone()
                .ok_or_else(|| ErrorInfo::new("selected_output_missing", "No selected_output"))?;
            Ok(SelectedBody::Json(value))
        }
        other => Err(ErrorInfo::new(
            "unsupported_postback_source",
            format!("Unsupported postback source '{other}'"),
        )),
    }
}

async fn validate_postback_url(url: &Url, config: &ZenroomConfig) -> Result<(), ErrorInfo> {
    let scheme = url.scheme();
    if !config
        .postback_allowed_schemes
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(scheme))
    {
        return Err(ErrorInfo::new(
            "postback_scheme_denied",
            format!("Scheme '{scheme}' not allowed"),
        ));
    }

    let port = url
        .port_or_known_default()
        .ok_or_else(|| ErrorInfo::new("postback_port_missing", "Missing URL port"))?;
    if !config.postback_allowed_ports.contains(&port) {
        return Err(ErrorInfo::new(
            "postback_port_denied",
            format!("Port {port} not allowed"),
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| ErrorInfo::new("postback_host_missing", "Missing URL host"))?;

    if config.postback_allowed_host_suffixes.is_empty() {
        return Err(ErrorInfo::new(
            "postback_host_denied",
            "No allowed postback host suffixes configured",
        ));
    }

    let host_allowed = config.postback_allowed_host_suffixes.iter().any(|suffix| {
        host.eq_ignore_ascii_case(suffix)
            || host
                .to_ascii_lowercase()
                .ends_with(&format!(".{}", suffix.to_ascii_lowercase()))
    });

    if !host_allowed {
        return Err(ErrorInfo::new(
            "postback_host_denied",
            format!("Host '{host}' not in allowlist"),
        ));
    }

    if config.postback_block_private_ips {
        let ip_addrs = resolve_host_ips(host, port).await?;
        for ip in ip_addrs {
            if !is_public_ip(&ip) {
                return Err(ErrorInfo::new(
                    "postback_ip_denied",
                    format!("IP {ip} is not globally routable"),
                ));
            }
        }
    }

    Ok(())
}

async fn resolve_host_ips(host: &str, port: u16) -> Result<Vec<IpAddr>, ErrorInfo> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![ip]);
    }

    let mut results = Vec::new();
    let lookup = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| ErrorInfo::new("postback_dns_failed", e.to_string()))?;
    for addr in lookup {
        results.push(addr.ip());
    }
    Ok(results)
}

fn is_public_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(addr) => is_public_ipv4(addr),
        IpAddr::V6(addr) => is_public_ipv6(addr),
    }
}

fn is_public_ipv4(addr: &Ipv4Addr) -> bool {
    let ip = u32::from_be_bytes(addr.octets());
    let ranges = [
        (Ipv4Addr::new(0, 0, 0, 0), 8),          // "this network"
        (Ipv4Addr::new(10, 0, 0, 0), 8),         // private
        (Ipv4Addr::new(100, 64, 0, 0), 10),      // shared address space
        (Ipv4Addr::new(127, 0, 0, 0), 8),        // loopback
        (Ipv4Addr::new(169, 254, 0, 0), 16),     // link-local
        (Ipv4Addr::new(172, 16, 0, 0), 12),      // private
        (Ipv4Addr::new(192, 0, 0, 0), 24),       // IETF protocol assignments
        (Ipv4Addr::new(192, 0, 2, 0), 24),       // documentation
        (Ipv4Addr::new(192, 88, 99, 0), 24),     // 6to4 relay (deprecated)
        (Ipv4Addr::new(192, 168, 0, 0), 16),     // private
        (Ipv4Addr::new(198, 18, 0, 0), 15),      // benchmarking
        (Ipv4Addr::new(198, 51, 100, 0), 24),    // documentation
        (Ipv4Addr::new(203, 0, 113, 0), 24),     // documentation
        (Ipv4Addr::new(224, 0, 0, 0), 4),        // multicast
        (Ipv4Addr::new(240, 0, 0, 0), 4),        // reserved for future use
        (Ipv4Addr::new(255, 255, 255, 255), 32), // broadcast
    ];

    !ranges
        .iter()
        .any(|(base, prefix)| ipv4_in_range(ip, *base, *prefix))
}

fn ipv4_in_range(ip: u32, base: Ipv4Addr, prefix: u8) -> bool {
    if prefix == 0 {
        return true;
    }
    let base_ip = u32::from_be_bytes(base.octets());
    let mask = u32::MAX << (32 - prefix);
    (ip & mask) == (base_ip & mask)
}

fn is_public_ipv6(addr: &Ipv6Addr) -> bool {
    if let Some(v4) = addr.to_ipv4_mapped().or_else(|| addr.to_ipv4()) {
        return is_public_ipv4(&v4);
    }

    if addr.is_loopback()
        || addr.is_unspecified()
        || addr.is_multicast()
        || addr.is_unique_local()
        || addr.is_unicast_link_local()
        || is_documentation_ipv6(addr)
    {
        return false;
    }

    true
}

fn is_documentation_ipv6(addr: &Ipv6Addr) -> bool {
    let segments = addr.segments();
    segments[0] == 0x2001 && segments[1] == 0x0db8
}

fn truncate_bytes(bytes: &[u8], max: usize) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let truncated = if bytes.len() > max {
        &bytes[..max]
    } else {
        bytes
    };
    Some(String::from_utf8_lossy(truncated).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_strategy_wrap() {
        let data = json!(["a", "b"]);
        let nxcc = json!({"worker": {"worker_id": "id"}});
        let merged = merge_data(data, nxcc.clone(), MergeStrategy::Wrap);
        assert_eq!(merged["nxcc"], nxcc);
        assert_eq!(merged["input"], json!(["a", "b"]));
    }

    #[test]
    fn test_merge_strategy_merge_object() {
        let data = json!({"foo": "bar"});
        let nxcc = json!({"worker": {"worker_id": "id"}});
        let merged = merge_data(data, nxcc.clone(), MergeStrategy::Merge);
        assert_eq!(merged["foo"], "bar");
        assert_eq!(merged["nxcc"], nxcc);
    }

    #[test]
    fn test_secret_injection_base64() {
        let mut value = json!({"key": {"$secret": "SECRET_A"}});
        let mut secrets = HashMap::new();
        secrets.insert("SECRET_A".to_string(), vec![0x01, 0x02, 0x03]);
        let cfg = ZenroomConfig::default();
        let mut total = 0;
        inject_secrets(&mut value, &secrets, &cfg, &mut total).unwrap();
        assert_eq!(value, json!({"key": "AQID"}));
    }

    #[test]
    fn test_secret_injection_hkdf_hex() {
        let mut value = json!({
            "key": {
                "$secret": "SECRET_B",
                "kdf": {"info": "test", "len": 4},
                "encoding": "hex"
            }
        });
        let mut secrets = HashMap::new();
        secrets.insert("SECRET_B".to_string(), vec![0x00; 32]);
        let cfg = ZenroomConfig::default();
        let mut total = 0;
        inject_secrets(&mut value, &secrets, &cfg, &mut total).unwrap();
        if let Value::Object(map) = value {
            let key = map.get("key").unwrap().as_str().unwrap();
            assert_eq!(key.len(), 8);
        } else {
            panic!("expected object");
        }
    }
}
