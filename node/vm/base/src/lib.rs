use nxcc_interface::{
    proto::{
        interface as proto_interface,
        vm::{
            GetAttestationRequest, GetAttestationResponse, InvokeWorkerRequest,
            InvokeWorkerResponse, StartWorkerRequest, StartWorkerResponse, StopWorkerRequest,
            StopWorkerResponse,
            vm_server::{Vm, VmServer},
        },
    },
    types::{AttestationReport, IntoProto as _},
};
use std::{error::Error, fmt, path::Path, sync::Arc};
use thiserror::Error;
use tonic::{Request, Response, Status, transport::Server};
use tracing::{debug, error, info};

#[cfg(feature = "uds")]
use std::io::ErrorKind;
#[cfg(feature = "uds")]
use tokio::net::{UnixListener, UnixStream};
#[cfg(feature = "uds")]
use tokio_stream::wrappers::UnixListenerStream;

#[cfg(feature = "vsock")]
use tokio_vsock::{VsockAddr, VsockListener};

/// Error type returned by `VmRuntime` implementations.
#[derive(Error, Debug)]
pub struct VmError {
    message: String,
    /// Optional underlying error source.
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl VmError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    pub fn with_source(
        message: impl Into<String>,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(source) = &self.source {
            write!(f, ": {}", source)?;
        }
        Ok(())
    }
}

/// Trait defining the interface for a specific VM/runtime implementation.
/// Implementers of this trait handle the actual logic of managing and interacting
/// with worker instances.
#[tonic::async_trait]
pub trait VmRuntime: Send + Sync + 'static {
    /// Starts a new worker instance.
    ///
    /// # Arguments
    /// * `worker_id` - An identifier for the type of worker.
    /// * `worker_code` - The code for the worker.
    /// * `config` - Runtime-specific configuration data.
    ///
    /// # Returns
    /// A unique `instance_id` for the started worker on success.
    async fn start_worker(
        &self,
        worker_id: String,
        worker_code: Vec<u8>,
        config: Vec<u8>,
    ) -> Result<String, VmError>;

    /// Stops a running worker instance.
    ///
    /// # Arguments
    /// * `instance_id` - The unique identifier of the worker to stop.
    async fn stop_worker(&self, instance_id: String) -> Result<(), VmError>;

    /// Invokes a function or sends data to a worker instance.
    ///
    /// # Arguments
    /// * `instance_id` - The unique identifier of the worker to invoke.
    /// * `payload` - The data/payload to send to the worker.
    ///
    /// # Returns
    /// The result/output from the worker instance on success.
    async fn invoke_worker(
        &self,
        instance_id: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, VmError>;

    /// Retrieves an attestation report from the execution environment.
    ///
    /// # Arguments
    /// * `user_data` - Optional data to include in the attestation report.
    ///
    /// # Returns
    /// An `AttestationReport` on success.
    async fn get_attestation(&self, user_data: Vec<u8>) -> Result<AttestationReport, VmError>;
}

/// Internal struct that wraps a `VmRuntime` implementation and handles gRPC calls.
struct VmServiceGrpc<T: VmRuntime> {
    runtime: Arc<T>,
}

impl<T: VmRuntime> VmServiceGrpc<T> {
    fn new(runtime: Arc<T>) -> Self {
        Self { runtime }
    }
}

#[tonic::async_trait]
impl<T: VmRuntime> Vm for VmServiceGrpc<T> {
    async fn start_worker(
        &self,
        request: Request<StartWorkerRequest>,
    ) -> Result<Response<StartWorkerResponse>, Status> {
        let req = request.into_inner();
        debug!(
            "gRPC StartWorker request for worker_id '{}', code size {}, config size {}",
            req.worker_id,
            req.worker_code.len(),
            req.config.len()
        );

        match self
            .runtime
            .start_worker(req.worker_id, req.worker_code, req.config)
            .await
        {
            Ok(instance_id) => {
                info!("Successfully started worker instance {}", instance_id);
                Ok(Response::new(StartWorkerResponse {
                    instance_id,
                    success: true,
                    error_message: String::new(),
                }))
            }
            Err(e) => {
                error!("Failed to start worker: {}", e);
                Ok(Response::new(StartWorkerResponse {
                    instance_id: String::new(),
                    success: false,
                    error_message: e.to_string(),
                }))
            }
        }
    }

    async fn stop_worker(
        &self,
        request: Request<StopWorkerRequest>,
    ) -> Result<Response<StopWorkerResponse>, Status> {
        let req = request.into_inner();
        debug!(
            "gRPC StopWorker request for instance_id '{}'",
            req.instance_id
        );

        match self.runtime.stop_worker(req.instance_id).await {
            Ok(()) => {
                info!("Successfully stopped worker instance");
                Ok(Response::new(StopWorkerResponse {
                    success: true,
                    error_message: String::new(),
                }))
            }
            Err(e) => {
                error!("Failed to stop worker: {}", e);
                Ok(Response::new(StopWorkerResponse {
                    success: false,
                    error_message: e.to_string(),
                }))
            }
        }
    }

    async fn invoke_worker(
        &self,
        request: Request<InvokeWorkerRequest>,
    ) -> Result<Response<InvokeWorkerResponse>, Status> {
        let req = request.into_inner();
        debug!(
            "gRPC InvokeWorker request for instance_id '{}', payload size {}",
            req.instance_id,
            req.payload.len()
        );

        match self
            .runtime
            .invoke_worker(req.instance_id, req.payload)
            .await
        {
            Ok(result) => {
                debug!("Successfully invoked worker, result size {}", result.len());
                Ok(Response::new(InvokeWorkerResponse {
                    result,
                    success: true,
                    error_message: String::new(),
                }))
            }
            Err(e) => {
                error!("Failed to invoke worker: {}", e);
                Ok(Response::new(InvokeWorkerResponse {
                    result: Vec::new(),
                    success: false,
                    error_message: e.to_string(),
                }))
            }
        }
    }

    async fn get_attestation(
        &self,
        request: Request<GetAttestationRequest>,
    ) -> Result<Response<GetAttestationResponse>, Status> {
        let req = request.into_inner();
        debug!(
            "gRPC GetAttestation request with user_data size {}",
            req.user_data.len()
        );

        match self.runtime.get_attestation(req.user_data).await {
            Ok(report) => {
                debug!("Successfully retrieved attestation report");
                let proto_report: proto_interface::AttestationReport = report.to_proto();
                Ok(Response::new(GetAttestationResponse {
                    report: Some(proto_report),
                }))
            }
            Err(e) => {
                error!("Failed to get attestation: {}", e);
                // Unlike other methods, attestation failure might be more critical,
                // so return an internal error status.
                Err(Status::internal(format!(
                    "Failed to get attestation: {}",
                    e
                )))
            }
        }
    }
}

/// Configuration for the VM gRPC server.
#[derive(Clone, Debug)]
pub enum ServerConfig {
    /// Listen on a Unix Domain Socket at the specified path.
    #[cfg(feature = "uds")]
    Uds { path: String },
    /// Listen on VSOCK at the specified CID and port.
    #[cfg(feature = "vsock")]
    Vsock { cid: u32, port: u32 },
}

/// Starts the gRPC server for the VM service.
///
/// This function takes ownership of the runtime implementation and runs the
/// server indefinitely until an error occurs or the process is terminated.
///
/// # Arguments
/// * `config` - The server listening configuration (UDS or VSOCK).
/// * `runtime` - An Arc-wrapped implementation of the `VmRuntime` trait.
///
/// # Errors
/// Returns an error if the server fails to start or encounters a runtime error.
pub async fn run_vm_server<T: VmRuntime>(
    config: ServerConfig,
    runtime: Arc<T>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let grpc_service = VmServiceGrpc::new(runtime);
    let server_builder = Server::builder().add_service(VmServer::new(grpc_service));

    match config {
        #[cfg(feature = "uds")]
        ServerConfig::Uds { path } => {
            info!("VM gRPC Server listening on UDS: {}", path);
            let path = Path::new(&path);
            if path.exists() {
                match UnixStream::connect(path).await {
                    Ok(_) => {
                        return Err(format!(
                            "UDS path {} already in use by a running server.",
                            path.display()
                        )
                        .into());
                    }
                    Err(e) if e.kind() == ErrorKind::ConnectionRefused => {
                        info!("Removing stale UDS file: {}", path.display());
                        std::fs::remove_file(path)?;
                    }
                    Err(e) => {
                        return Err(format!(
                            "Error checking existing UDS {}: {}",
                            path.display(),
                            e
                        )
                        .into());
                    }
                }
            }

            let listener = UnixListener::bind(path)?;
            let incoming = UnixListenerStream::new(listener);

            server_builder.serve_with_incoming(incoming).await?;
            let _ = std::fs::remove_file(path); // Clean up on shutdown
        }
        #[cfg(feature = "vsock")]
        ServerConfig::Vsock { cid, port } => {
            info!(
                "VM gRPC Server listening on vsock: CID={}, port={}",
                cid, port
            );
            let listener = VsockListener::bind(VsockAddr::new(cid, port))?;
            server_builder
                .serve_with_incoming(listener.incoming())
                .await?;
        }
        #[cfg(not(any(feature = "uds", feature = "vsock")))]
        _ => {
            return Err("No server transport feature (uds or vsock) enabled.".into());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nxcc_interface::{
        proto::vm::{
            GetAttestationRequest, InvokeWorkerRequest, StartWorkerRequest, StopWorkerRequest,
            vm_server::Vm,
        },
        types::{AttestationReport, FromProto as _},
    };
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };
    use tonic::Request;

    // ===== Mock runtime (rich variant from suite #1, plus a helper for deterministic ids) =====
    #[derive(Debug, Clone)]
    struct MockWorkerState {
        worker_id: String,
        worker_code: Vec<u8>,
        config: Vec<u8>,
    }

    #[derive(Debug, Default)]
    struct MockVmRuntimeState {
        workers: HashMap<String, MockWorkerState>,
        next_instance_id: u64,
        next_start_worker_result: Option<Result<String, VmError>>,
        next_stop_worker_result: Option<Result<(), VmError>>,
        next_invoke_worker_result: Option<Result<Vec<u8>, VmError>>,
        next_get_attestation_result: Option<Result<AttestationReport, VmError>>,
        start_worker_calls: Vec<(String, Vec<u8>, Vec<u8>)>,
        stop_worker_calls: Vec<String>,
        invoke_worker_calls: Vec<(String, Vec<u8>)>,
        get_attestation_calls: Vec<Vec<u8>>,
    }

    #[derive(Debug, Clone)]
    struct MockVmRuntime {
        state: Arc<Mutex<MockVmRuntimeState>>,
    }

    impl MockVmRuntime {
        fn new() -> Self {
            Self {
                state: Arc::new(Mutex::new(MockVmRuntimeState::default())),
            }
        }

        /* ------------- helpers copied from suite #1 ------------- */
        fn set_next_start_worker_result(&self, r: Result<String, VmError>) {
            self.state.lock().unwrap().next_start_worker_result = Some(r);
        }
        fn set_next_stop_worker_result(&self, r: Result<(), VmError>) {
            self.state.lock().unwrap().next_stop_worker_result = Some(r);
        }
        fn set_next_invoke_worker_result(&self, r: Result<Vec<u8>, VmError>) {
            self.state.lock().unwrap().next_invoke_worker_result = Some(r);
        }
        fn set_next_get_attestation_result(&self, r: Result<AttestationReport, VmError>) {
            self.state.lock().unwrap().next_get_attestation_result = Some(r);
        }

        fn get_start_worker_calls(&self) -> Vec<(String, Vec<u8>, Vec<u8>)> {
            self.state.lock().unwrap().start_worker_calls.clone()
        }
        fn get_stop_worker_calls(&self) -> Vec<String> {
            self.state.lock().unwrap().stop_worker_calls.clone()
        }
        fn get_invoke_worker_calls(&self) -> Vec<(String, Vec<u8>)> {
            self.state.lock().unwrap().invoke_worker_calls.clone()
        }
        fn get_get_attestation_calls(&self) -> Vec<Vec<u8>> {
            self.state.lock().unwrap().get_attestation_calls.clone()
        }
        fn expect_worker_exists(&self, instance_id: &str) -> bool {
            self.state.lock().unwrap().workers.contains_key(instance_id)
        }

        /* ------------- NEW: helper to generate deterministic IDs when desired ------------- */
        fn make_deterministic_id(worker_id: &str, code_len: usize) -> String {
            format!("instance-{worker_id}-{code_len}")
        }
    }

    #[tonic::async_trait]
    impl VmRuntime for MockVmRuntime {
        async fn start_worker(
            &self,
            worker_id: String,
            worker_code: Vec<u8>,
            config: Vec<u8>,
        ) -> Result<String, VmError> {
            let mut st = self.state.lock().unwrap();
            st.start_worker_calls
                .push((worker_id.clone(), worker_code.clone(), config.clone()));

            match st.next_start_worker_result.take() {
                Some(res) => res,
                None => {
                    st.next_instance_id += 1;
                    let id = format!("mock-instance-{}", st.next_instance_id);
                    st.workers.insert(
                        id.clone(),
                        MockWorkerState {
                            worker_id,
                            worker_code,
                            config,
                        },
                    );
                    Ok(id)
                }
            }
        }

        async fn stop_worker(&self, instance_id: String) -> Result<(), VmError> {
            let mut st = self.state.lock().unwrap();
            st.stop_worker_calls.push(instance_id.clone());

            match st.next_stop_worker_result.take() {
                Some(res) => res,
                None => {
                    st.workers
                        .remove(&instance_id)
                        .ok_or_else(|| VmError::new(format!("Worker {instance_id} not found")))?;
                    Ok(())
                }
            }
        }

        async fn invoke_worker(
            &self,
            instance_id: String,
            payload: Vec<u8>,
        ) -> Result<Vec<u8>, VmError> {
            let mut st = self.state.lock().unwrap();
            st.invoke_worker_calls
                .push((instance_id.clone(), payload.clone()));

            // If no pre‑set result and worker missing → error
            if st.next_invoke_worker_result.is_none() && !st.workers.contains_key(&instance_id) {
                return Err(VmError::new(format!(
                    "Worker {instance_id} not found for invoke"
                )));
            }

            match st.next_invoke_worker_result.take() {
                Some(res) => res,
                None => Ok(payload), // default echo
            }
        }

        async fn get_attestation(&self, user_data: Vec<u8>) -> Result<AttestationReport, VmError> {
            let mut st = self.state.lock().unwrap();
            st.get_attestation_calls.push(user_data.clone());

            match st.next_get_attestation_result.take() {
                Some(res) => res,
                None => Ok(AttestationReport {
                    ephemeral_public_key: b"mock_pub_key".to_vec(),
                    block_hashes: vec![b"hash1".to_vec(), b"hash2".to_vec()],
                    user_data,
                }),
            }
        }
    }

    // ===== Helper to build the gRPC façade =====
    fn setup() -> (VmServiceGrpc<MockVmRuntime>, Arc<MockVmRuntime>) {
        let rt = Arc::new(MockVmRuntime::new());
        (VmServiceGrpc::new(rt.clone()), rt)
    }

    // ======= TESTS ================================================================

    /* ---------- start_worker ---------- */
    #[tokio::test]
    async fn test_start_worker_success() {
        let (svc, mock) = setup();
        let req = Request::new(StartWorkerRequest {
            worker_id: "ok-worker".into(),
            worker_code: vec![1, 2, 3, 4],
            config: b"{}".to_vec(),
        });
        let resp = svc.start_worker(req).await.unwrap().into_inner();
        assert!(resp.success);
        assert!(!resp.instance_id.is_empty());
        assert!(mock.expect_worker_exists(&resp.instance_id));
        assert!(resp.error_message.is_empty());
    }

    #[tokio::test]
    async fn test_start_worker_failure() {
        let (svc, mock) = setup();
        let wanted_id = MockVmRuntime::make_deterministic_id("bad", 5 /*code len*/);
        mock.set_next_start_worker_result(Err(VmError::new("simulated failure")));
        let req = Request::new(StartWorkerRequest {
            worker_id: "bad".into(),
            worker_code: vec![0; 5],
            config: vec![],
        });
        let resp = svc.start_worker(req).await.unwrap().into_inner();
        assert!(!resp.success);
        assert!(resp.instance_id.is_empty());
        assert!(resp.error_message.contains("simulated failure"));
        assert!(!mock.expect_worker_exists(&wanted_id));
        // verify call was captured
        assert_eq!(mock.get_start_worker_calls().len(), 1);
    }

    /* ---------- stop_worker ---------- */
    #[tokio::test]
    async fn test_stop_worker_success_and_cannot_invoke_after() {
        let (svc, mock) = setup();
        // pre‑create
        let inst = "instance-to-stop".to_string();
        mock.state.lock().unwrap().workers.insert(
            inst.clone(),
            MockWorkerState {
                worker_id: "w".into(),
                worker_code: vec![],
                config: vec![],
            },
        );
        let stop = Request::new(StopWorkerRequest {
            instance_id: inst.clone(),
        });
        let resp = svc.stop_worker(stop).await.unwrap().into_inner();
        assert!(resp.success && resp.error_message.is_empty());
        assert!(!mock.expect_worker_exists(&inst));

        // try to invoke — expect not found
        let invoke = Request::new(InvokeWorkerRequest {
            instance_id: inst.clone(),
            payload: vec![9],
        });
        let invoke_resp = svc.invoke_worker(invoke).await.unwrap().into_inner();
        assert!(!invoke_resp.success);
        assert!(invoke_resp.error_message.contains("not found"));
    }

    #[tokio::test]
    async fn test_stop_worker_failure_runtime_error() {
        let (svc, mock) = setup();
        let inst = "cant-stop".to_string();
        mock.state.lock().unwrap().workers.insert(
            inst.clone(),
            MockWorkerState {
                worker_id: "x".into(),
                worker_code: vec![],
                config: vec![],
            },
        );
        mock.set_next_stop_worker_result(Err(VmError::new("runtime oops")));
        let resp = svc
            .stop_worker(Request::new(StopWorkerRequest {
                instance_id: inst.clone(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!resp.success);
        assert!(resp.error_message.contains("runtime oops"));
        assert!(mock.expect_worker_exists(&inst));
    }

    #[tokio::test]
    async fn test_stop_worker_failure_not_found() {
        let (svc, _) = setup();
        let missing = "does-not-exist".to_string();
        let resp = svc
            .stop_worker(Request::new(StopWorkerRequest {
                instance_id: missing.clone(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!resp.success);
        assert!(resp.error_message.contains("not found"));
    }

    /* ---------- invoke_worker ---------- */
    #[tokio::test]
    async fn test_invoke_worker_success() {
        let (svc, mock) = setup();
        let inst = "invoke-ok".to_string();
        mock.state.lock().unwrap().workers.insert(
            inst.clone(),
            MockWorkerState {
                worker_id: "i".into(),
                worker_code: vec![],
                config: vec![],
            },
        );
        let expected = vec![42, 42];
        mock.set_next_invoke_worker_result(Ok(expected.clone()));
        let resp = svc
            .invoke_worker(Request::new(InvokeWorkerRequest {
                instance_id: inst.clone(),
                payload: vec![1, 2],
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(resp.success);
        assert_eq!(resp.result, expected);
    }

    #[tokio::test]
    async fn test_invoke_worker_echo_default() {
        let (svc, mock) = setup();
        let inst = "echoer".to_string();
        mock.state.lock().unwrap().workers.insert(
            inst.clone(),
            MockWorkerState {
                worker_id: "e".into(),
                worker_code: vec![],
                config: vec![],
            },
        );
        let payload = b"ping".to_vec();
        let resp = svc
            .invoke_worker(Request::new(InvokeWorkerRequest {
                instance_id: inst,
                payload: payload.clone(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(resp.success);
        assert_eq!(resp.result, payload);
    }

    #[tokio::test]
    async fn test_invoke_worker_failure_not_found() {
        let (svc, _) = setup();
        let resp = svc
            .invoke_worker(Request::new(InvokeWorkerRequest {
                instance_id: "missing".into(),
                payload: vec![],
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!resp.success);
        assert!(resp.error_message.contains("not found"));
    }

    #[tokio::test]
    async fn test_invoke_worker_failure_runtime_error() {
        let (svc, mock) = setup();
        let inst = "invoke-bad".to_string();
        mock.state.lock().unwrap().workers.insert(
            inst.clone(),
            MockWorkerState {
                worker_id: "b".into(),
                worker_code: vec![],
                config: vec![],
            },
        );
        mock.set_next_invoke_worker_result(Err(VmError::new("exec timed out")));
        let resp = svc
            .invoke_worker(Request::new(InvokeWorkerRequest {
                instance_id: inst,
                payload: vec![8],
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!resp.success);
        assert!(resp.error_message.contains("exec timed out"));
    }

    /* ---------- get_attestation ---------- */
    #[tokio::test]
    async fn test_get_attestation_success() {
        let (svc, mock) = setup();
        let expected = AttestationReport {
            ephemeral_public_key: b"keyX".to_vec(),
            block_hashes: vec![b"h1".to_vec()],
            user_data: b"data!".to_vec(),
        };
        mock.set_next_get_attestation_result(Ok(expected.clone()));
        let resp = svc
            .get_attestation(Request::new(GetAttestationRequest {
                user_data: expected.user_data.clone(),
            }))
            .await
            .unwrap()
            .into_inner();
        let got = AttestationReport::from_proto(resp.report.unwrap());
        assert_eq!(got, expected);
        assert_eq!(mock.get_get_attestation_calls(), vec![expected.user_data]);
    }

    #[tokio::test]
    async fn test_get_attestation_failure() {
        let (svc, mock) = setup();
        mock.set_next_get_attestation_result(Err(VmError::new("TEE failure")));
        let err = svc
            .get_attestation(Request::new(GetAttestationRequest { user_data: vec![1] }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::Internal);
        assert!(err.message().contains("TEE failure"));
    }

    /* ---------- Misc. ---------- */
    #[test]
    fn test_vmerror_with_source_sanity() {
        let io_err = std::io::Error::new(std::io::ErrorKind::Other, "inner");
        let vm_err = VmError::with_source("outer", io_err);
        let disp = vm_err.to_string();
        assert!(disp.contains("outer"));
        assert!(disp.contains("inner"));
    }
}
