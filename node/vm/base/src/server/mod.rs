#[cfg(test)]
mod tests;

use std::{error::Error, fmt, sync::Arc};

use thiserror::Error;
use tonic::{Request, Response, Status, transport::Server};
use tracing::{debug, error, info, warn};

use crate::binding::{BoundClient, ClientBindingLayer};

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
#[tonic::async_trait]
pub trait VmRuntime: Send + Sync + 'static {
    /// Starts a new vm instance returning the ID.
    async fn start_worker(
        &self,
        worker_code: Vec<u8>,
        untrusted_config: nxcc_interface::proto::vm::UntrustedConfig,
        trusted_config: nxcc_interface::proto::vm::TrustedConfig,
    ) -> Result<String, VmError>;

    /// Stops a running worker instance.
    async fn stop_worker(&self, id: String) -> Result<(), VmError>;

    /// Invokes a function or sends data to a worker instance.
    async fn invoke_worker(
        &self,
        id: String,
        handler_name: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, VmError>;

    /// Invokes a worker with an HTTP request.
    async fn invoke_http(
        &self,
        id: String,
        request: nxcc_interface::proto::vm::HttpRequest,
    ) -> Result<nxcc_interface::proto::vm::HttpResponse, VmError>;

    /// Retrieves an attestation report from the execution environment.
    async fn get_attestation(
        &self,
        user_data: Vec<u8>,
    ) -> Result<nxcc_interface::types::AttestationReport, VmError>;

    /// Retrieves the status of a specific worker instance.
    async fn get_worker_status(
        &self,
        id: String,
    ) -> Result<nxcc_interface::proto::vm::WorkerStatus, VmError>;

    /// Retrieves the IDs of all currently running worker instances.
    async fn list_running_workers(&self) -> Result<Vec<String>, VmError>;

    /// Retrieves debug logs from a specific worker instance.
    async fn get_worker_logs(&self, id: String) -> Result<String, VmError>;
}

use nxcc_interface::proto::vm::{
    GetAttestationRequest, GetAttestationResponse, GetWorkerLogsRequest, GetWorkerLogsResponse,
    GetWorkerStatusRequest, GetWorkerStatusResponse, HttpRequest as ProtoHttpRequest,
    HttpResponse as ProtoHttpResponse, InvokeHttpRequest, InvokeHttpResponse, InvokeWorkerRequest,
    InvokeWorkerResponse, ListRunningWorkersRequest, ListRunningWorkersResponse,
    StartWorkerRequest, StartWorkerResponse, StopWorkerRequest, StopWorkerResponse, WorkerStatus,
    vm_server::{Vm, VmServer},
};

pub struct VmServiceGrpc<T: VmRuntime> {
    runtime: Arc<T>,
}

impl<T: VmRuntime> VmServiceGrpc<T> {
    pub fn new(runtime: Arc<T>) -> Self {
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
            "gRPC StartWorker request for worker_id '{}', code size {}",
            req.worker_id,
            req.worker_code.len(),
            // TODO: Maybe log config sizes or keys if needed, be mindful of sensitive data
        );

        // Extract configs, handling potential None cases (though protobuf3 shouldn't send None for messages)
        let untrusted_config = req.untrusted_config.unwrap_or_default();
        let trusted_config = req.trusted_config.unwrap_or_default();

        match self
            .runtime
            .start_worker(req.worker_code, untrusted_config, trusted_config)
            .await
        {
            Ok(id) => {
                info!("Successfully started worker instance {}", id);
                Ok(Response::new(StartWorkerResponse {
                    id,
                    success: true,
                    error_message: String::new(),
                }))
            }
            Err(e) => {
                error!("Failed to start worker: {}", e);
                Ok(Response::new(StartWorkerResponse {
                    id: String::new(),
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
        debug!("gRPC StopWorker request for id '{}'", req.id);

        match self.runtime.stop_worker(req.id).await {
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
            "gRPC InvokeWorker request for id '{}', handler '{}', payload size {}",
            req.id,
            req.handler_name,
            req.payload.len()
        );

        match self
            .runtime
            .invoke_worker(req.id, req.handler_name, req.payload)
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

    async fn invoke_http(
        &self,
        request: Request<InvokeHttpRequest>,
    ) -> Result<Response<InvokeHttpResponse>, Status> {
        let req_inner = request.into_inner();
        let worker_id = req_inner.worker_id;
        let http_request_proto = req_inner
            .request
            .ok_or_else(|| Status::invalid_argument("Missing HttpRequest in InvokeHttpRequest"))?;

        debug!(
            "gRPC InvokeHttp request for worker_id '{}', uri '{}', method '{}'",
            worker_id, http_request_proto.uri, http_request_proto.method
        );

        match self
            .runtime
            .invoke_http(worker_id.clone(), http_request_proto)
            .await
        {
            Ok(http_response_proto) => {
                debug!("Successfully invoked HTTP worker {}", worker_id);
                Ok(Response::new(InvokeHttpResponse {
                    response: Some(http_response_proto),
                }))
            }
            Err(e) => {
                error!("Failed to invoke HTTP worker {}: {}", worker_id, e);
                Err(Status::internal(format!("HTTP invocation failed: {}", e)))
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
                let proto_report: nxcc_interface::proto::interface::AttestationReport =
                    report.into();
                Ok(Response::new(GetAttestationResponse {
                    report: Some(proto_report),
                }))
            }
            Err(e) => {
                error!("Failed to get attestation: {}", e);
                Err(Status::internal(format!(
                    "Failed to get attestation: {}",
                    e
                )))
            }
        }
    }

    async fn get_worker_status(
        &self,
        request: Request<GetWorkerStatusRequest>,
    ) -> Result<Response<GetWorkerStatusResponse>, Status> {
        let req = request.into_inner();
        debug!("gRPC GetWorkerStatus request for id '{}'", req.id);

        match self.runtime.get_worker_status(req.id.clone()).await {
            Ok(status) => {
                debug!("Successfully retrieved status for worker {}", req.id);
                Ok(Response::new(GetWorkerStatusResponse {
                    id: req.id,
                    status: status.into(), // Convert enum to i32
                    success: true,
                    error_message: String::new(),
                }))
            }
            Err(e) => {
                error!("Failed to get worker status for id {}: {}", req.id, e);
                Ok(Response::new(GetWorkerStatusResponse {
                    id: req.id,
                    status: WorkerStatus::Unspecified.into(),
                    success: false,
                    error_message: e.to_string(),
                }))
            }
        }
    }

    async fn list_running_workers(
        &self,
        _request: Request<ListRunningWorkersRequest>,
    ) -> Result<Response<ListRunningWorkersResponse>, Status> {
        debug!("gRPC ListRunningWorkers request");

        match self.runtime.list_running_workers().await {
            Ok(ids) => {
                debug!("Successfully listed {} running workers", ids.len());
                Ok(Response::new(ListRunningWorkersResponse { ids }))
            }
            Err(e) => {
                error!("Failed to list running workers: {}", e);
                // This is likely an internal error
                Err(Status::internal(format!(
                    "Failed to list running workers: {}",
                    e
                )))
            }
        }
    }

    async fn get_worker_logs(
        &self,
        request: Request<GetWorkerLogsRequest>,
    ) -> Result<Response<GetWorkerLogsResponse>, Status> {
        let req = request.into_inner();
        debug!("gRPC GetWorkerLogs request for id '{}'", req.id);

        match self.runtime.get_worker_logs(req.id.clone()).await {
            Ok(logs) => {
                debug!(
                    "Successfully retrieved logs for worker {}, size {}",
                    req.id,
                    logs.len()
                );
                Ok(Response::new(GetWorkerLogsResponse {
                    logs,
                    success: true,
                    error_message: String::new(),
                }))
            }
            Err(e) => {
                error!("Failed to get worker logs for id {}: {}", req.id, e);
                Ok(Response::new(GetWorkerLogsResponse {
                    logs: String::new(),
                    success: false,
                    error_message: e.to_string(),
                }))
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
    /// Listen on TCP at the specified address
    #[cfg(feature = "tcp")]
    Tcp { addr: std::net::SocketAddr },
}

/// Starts the gRPC server for the VM service with mTLS.
///
/// # Arguments
/// * `config` - The server listening configuration (UDS, VSOCK, or TCP).
/// * `runtime` - An Arc-wrapped implementation of the `VmRuntime` trait.
/// * `server_tls_config` - TLS configuration for the server.
///
/// # Errors
/// Returns an error if the server fails to start or encounters a runtime error.
pub async fn run_vm_server<T: VmRuntime>(
    config: ServerConfig,
    runtime: Arc<T>,
    server_tls_config: tonic::transport::ServerTlsConfig,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let bound_client = BoundClient::new();
    let client_binding_layer = ClientBindingLayer::new(bound_client);
    let grpc_service = VmServiceGrpc::new(runtime);

    let server_builder = Server::builder().add_service(VmServer::new(grpc_service));

    match config {
        #[cfg(feature = "uds")]
        ServerConfig::Uds { path } => {
            info!("VM gRPC Server listening on UDS: {}", path);
            let path = std::path::Path::new(&path);
            if path.exists() {
                warn!("Removing existing UDS file: {}", path.display());
                std::fs::remove_file(path)?;
            }

            let listener = tokio::net::UnixListener::bind(path)?;
            let incoming = tokio_stream::wrappers::UnixListenerStream::new(listener);
            info!("UDS Server started.");
            server_builder.serve_with_incoming(incoming).await?;
            info!("UDS Server stopped. Cleaning up socket file.");
            let _ = std::fs::remove_file(path);
        }
        #[cfg(feature = "vsock")]
        ServerConfig::Vsock { cid, port } => {
            info!(
                "VM gRPC Server listening on vsock: CID={}, port={}",
                cid, port
            );
            let listener =
                tokio_vsock::VsockListener::bind(tokio_vsock::VsockAddr::new(cid, port))?;
            info!("VSOCK Server started.");
            server_builder
                .serve_with_incoming(listener.incoming())
                .await?;
            info!("VSOCK Server stopped.");
        }
        #[cfg(feature = "tcp")]
        ServerConfig::Tcp { addr } => {
            info!("VM gRPC Server listening on TCP: {}", addr);
            info!("TCP Server started.");
            server_builder.serve(addr).await?;
            info!("TCP Server stopped.");
        }
        #[cfg(not(any(feature = "uds", feature = "vsock", feature = "tcp")))]
        _ => {
            return Err("No server transport feature (uds, vsock, or tcp) enabled.".into());
        }
    }

    Ok(())
}
