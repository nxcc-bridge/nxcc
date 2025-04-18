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
    /// Starts a new worker instance.
    async fn start_worker(
        &self,
        worker_id: String,
        worker_code: Vec<u8>,
        config: Vec<u8>,
    ) -> Result<String, VmError>;

    /// Stops a running worker instance.
    async fn stop_worker(&self, instance_id: String) -> Result<(), VmError>;

    /// Invokes a function or sends data to a worker instance.
    async fn invoke_worker(
        &self,
        instance_id: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, VmError>;

    /// Retrieves an attestation report from the execution environment.
    async fn get_attestation(
        &self,
        user_data: Vec<u8>,
    ) -> Result<nxcc_interface::types::AttestationReport, VmError>;
}

use nxcc_interface::{
    proto::vm::{
        GetAttestationRequest, GetAttestationResponse, InvokeWorkerRequest, InvokeWorkerResponse,
        StartWorkerRequest, StartWorkerResponse, StopWorkerRequest, StopWorkerResponse,
        vm_server::{Vm, VmServer},
    },
    types::IntoProto as _,
};

/// Internal struct that wraps a `VmRuntime` implementation and handles gRPC calls.
pub(crate) struct VmServiceGrpc<T: VmRuntime> {
    runtime: Arc<T>,
}

impl<T: VmRuntime> VmServiceGrpc<T> {
    pub(crate) fn new(runtime: Arc<T>) -> Self {
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
                // Return Ok with success=false for application-level errors
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
                // Return Ok with success=false for application-level errors
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
                // Return Ok with success=false for application-level errors
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
                let proto_report = report.to_proto();
                Ok(Response::new(GetAttestationResponse {
                    report: Some(proto_report),
                }))
            }
            Err(e) => {
                error!("Failed to get attestation: {}", e);
                // Return Err for internal/unrecoverable errors
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
    // Create bound client state for client binding
    let bound_client = BoundClient::new();

    // Create the client binding layer
    let client_binding_layer = ClientBindingLayer::new(bound_client);

    // Create the gRPC service
    let grpc_service = VmServiceGrpc::new(runtime);

    // Build the server with TLS and the client binding layer
    let server_builder = Server::builder()
        .tls_config(server_tls_config)?
        .layer(client_binding_layer)
        .add_service(VmServer::new(grpc_service));

    // Start the server based on configuration
    match config {
        #[cfg(feature = "uds")]
        ServerConfig::Uds { path } => {
            info!("VM gRPC Server listening on UDS: {}", path);
            let path = std::path::Path::new(&path);
            // Clean up existing socket file if necessary
            if path.exists() {
                warn!("Removing existing UDS file: {}", path.display());
                std::fs::remove_file(path)?;
            }

            let listener = tokio::net::UnixListener::bind(path)?;
            let incoming = tokio_stream::wrappers::UnixListenerStream::new(listener);
            info!("UDS Server started.");
            server_builder.serve_with_incoming(incoming).await?;
            info!("UDS Server stopped. Cleaning up socket file.");
            // Attempt cleanup on graceful shutdown. Crashes might leave the file.
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use nxcc_interface::proto::vm::{
        GetAttestationRequest, InvokeWorkerRequest, StartWorkerRequest, StopWorkerRequest,
    };

    use super::*;

    // Mock implementation of VmRuntime for testing
    #[derive(Default)]
    struct MockVmRuntime {
        start_worker_count: AtomicUsize,
        stop_worker_count: AtomicUsize,
        invoke_worker_count: AtomicUsize,
        force_attestation_error: AtomicBool,
    }

    #[tonic::async_trait]
    impl VmRuntime for MockVmRuntime {
        async fn start_worker(
            &self,
            worker_id: String,
            _worker_code: Vec<u8>,
            _config: Vec<u8>,
        ) -> Result<String, VmError> {
            self.start_worker_count.fetch_add(1, Ordering::SeqCst);
            Ok(format!("instance-{}", worker_id))
        }

        async fn stop_worker(&self, instance_id: String) -> Result<(), VmError> {
            self.stop_worker_count.fetch_add(1, Ordering::SeqCst);
            if instance_id.starts_with("instance-") {
                Ok(())
            } else {
                Err(VmError::new("Invalid instance ID"))
            }
        }

        async fn invoke_worker(
            &self,
            instance_id: String,
            payload: Vec<u8>,
        ) -> Result<Vec<u8>, VmError> {
            self.invoke_worker_count.fetch_add(1, Ordering::SeqCst);
            if !instance_id.starts_with("instance-") {
                return Err(VmError::new("Invalid instance ID"));
            }
            Ok(payload.iter().map(|b| b.wrapping_add(1)).collect())
        }

        async fn get_attestation(
            &self,
            user_data: Vec<u8>,
        ) -> Result<nxcc_interface::types::AttestationReport, VmError> {
            if self.force_attestation_error.load(Ordering::SeqCst) {
                return Err(VmError::new("Forced attestation error"));
            }

            // Just return a simple mock attestation report
            Ok(nxcc_interface::types::AttestationReport {
                ephemeral_public_key: vec![],
                block_hashes: vec![],
                user_data, // Echo user data back
            })
        }
    }

    // Test the VmServiceGrpc implementation
    #[tokio::test]
    async fn test_vm_service_grpc_start_worker() {
        let runtime = Arc::new(MockVmRuntime::default());
        let service = VmServiceGrpc::new(runtime.clone());

        // Test start_worker
        let request = Request::new(StartWorkerRequest {
            worker_id: "test-worker".to_string(),
            worker_code: vec![1, 2, 3],
            config: vec![4, 5, 6],
        });

        let response = service.start_worker(request).await.unwrap();
        let response = response.into_inner();
        assert!(response.success);
        assert_eq!(response.instance_id, "instance-test-worker");
        assert_eq!(runtime.start_worker_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_vm_service_grpc_stop_invoke_attestation() {
        let runtime = Arc::new(MockVmRuntime::default());
        let service = VmServiceGrpc::new(runtime.clone());

        // Test stop_worker (happy path)
        let request = Request::new(StopWorkerRequest {
            instance_id: "instance-123".to_string(),
        });
        let response = service.stop_worker(request).await.unwrap().into_inner();
        assert!(response.success);
        assert_eq!(runtime.stop_worker_count.load(Ordering::SeqCst), 1);

        // Test invoke_worker (happy path)
        let request = Request::new(InvokeWorkerRequest {
            instance_id: "instance-456".to_string(),
            payload: vec![10, 20],
        });
        let response = service.invoke_worker(request).await.unwrap().into_inner();
        assert!(response.success);
        assert_eq!(response.result, vec![11, 21]); // Mock adds 1
        assert_eq!(runtime.invoke_worker_count.load(Ordering::SeqCst), 1);

        // Test get_attestation (happy path)
        let request = Request::new(GetAttestationRequest {
            user_data: vec![7, 8],
        });
        let response = service.get_attestation(request).await.unwrap().into_inner();
        assert!(response.report.is_some());
        assert_eq!(response.report.unwrap().user_data, vec![7, 8]);
    }

    #[tokio::test]
    async fn test_vm_service_grpc_errors() {
        let runtime = Arc::new(MockVmRuntime::default());
        let service = VmServiceGrpc::new(runtime.clone());

        // Test stop_worker (error path - invalid ID)
        let request = Request::new(StopWorkerRequest {
            instance_id: "invalid-id".to_string(), // Mock expects "instance-" prefix
        });
        let response = service.stop_worker(request).await.unwrap().into_inner();
        assert!(!response.success);
        assert!(response.error_message.contains("Invalid instance ID"));

        // Test invoke_worker (error path - invalid ID)
        let request = Request::new(InvokeWorkerRequest {
            instance_id: "invalid-id".to_string(),
            payload: vec![],
        });
        let response = service.invoke_worker(request).await.unwrap().into_inner();
        assert!(!response.success);
        assert!(response.error_message.contains("Invalid instance ID"));

        // Test get_attestation (error path - forced error)
        runtime
            .force_attestation_error
            .store(true, Ordering::SeqCst);
        let request = Request::new(GetAttestationRequest { user_data: vec![] });
        let result = service.get_attestation(request).await;
        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), Status::internal("").code()); // Check code only
        assert!(status.message().contains("Forced attestation error"));
    }
}
