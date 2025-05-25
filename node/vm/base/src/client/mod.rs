#[cfg(feature = "test")]
pub mod mock;
#[cfg(test)]
mod tests;

use std::{future::Future, net::SocketAddr, path::Path};

use hyper_util::rt::TokioIo;
use nxcc_interface::{
    proto::vm::{
        GetAttestationRequest, GetWorkerLogsRequest, GetWorkerStatusRequest,
        HttpRequest as ProtoHttpRequest, HttpResponse as ProtoHttpResponse, InvokeHttpRequest,
        InvokeHttpResponse, InvokeWorkerRequest, ListRunningWorkersRequest, StartWorkerRequest,
        StopWorkerRequest, TrustedConfig, UntrustedConfig, WorkerStatus,
    },
    types::AttestationReport,
};
use thiserror::Error;
#[cfg(feature = "uds")]
use tokio::net::UnixStream;
#[cfg(feature = "vsock")]
use tokio_vsock::VsockStream;
use tonic::{
    Status,
    transport::{Channel, ClientTlsConfig, Endpoint, Uri},
};
use tracing::debug;

pub use crate::tls::MtlsCertificates; // Expose the main struct if needed by callers

/// Errors that can occur during client operations
#[derive(Error, Debug)]
pub enum ClientError {
    #[error("Transport error: {0}")]
    Transport(#[from] tonic::transport::Error),

    #[error("gRPC error: {0}")]
    Grpc(#[from] Status),

    #[error("TLS configuration error: {0}")]
    TlsConfig(#[from] crate::tls::TlsError), // Use concrete TlsError

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to connect to VM service: {0}")]
    Connect(String), // Keep this for specific connection logic errors if any

    #[error("Invalid URI: {0}")]
    Uri(String), // Keep for URI parsing errors
}

/// Trait defining the interface for a VM service client
pub trait VmClient {
    /// Start a new worker with the provided code and configuration
    fn start_worker(
        &mut self,
        worker_id: String,
        worker_code: Vec<u8>,
        untrusted_config: UntrustedConfig,
        trusted_config: TrustedConfig,
    ) -> impl Future<Output = Result<String, ClientError>> + Send;

    /// Stop a running worker instance
    fn stop_worker(&mut self, id: String) -> impl Future<Output = Result<(), ClientError>> + Send;

    /// Invoke a worker with the provided payload
    fn invoke_worker(
        &mut self,
        id: String,
        handler_name: String,
        payload: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, ClientError>> + Send;

    /// Invoke an HTTP request on a worker
    fn invoke_http(
        &mut self,
        id: String,
        request: ProtoHttpRequest,
    ) -> impl Future<Output = Result<ProtoHttpResponse, ClientError>> + Send;

    /// Get an attestation report from the VM service
    fn get_attestation(
        &mut self,
        user_data: Vec<u8>,
    ) -> impl Future<Output = Result<AttestationReport, ClientError>>;

    /// Get the status of a worker instance
    fn get_worker_status(
        &mut self,
        id: String,
    ) -> impl Future<Output = Result<WorkerStatus, ClientError>> + Send;

    /// Get a list of all running worker IDs
    fn list_running_workers(
        &mut self,
    ) -> impl Future<Output = Result<Vec<String>, ClientError>> + Send;

    /// Get logs from a worker instance
    fn get_worker_logs(
        &mut self,
        id: String,
    ) -> impl Future<Output = Result<String, ClientError>> + Send;
}

/// Client for communicating with a VM service
#[derive(Clone)]
pub struct VmServiceClient {
    inner: nxcc_interface::proto::vm::vm_client::VmClient<Channel>,
}

impl VmServiceClient {
    /// Connect to a VM service over TCP with TLS
    pub async fn connect(
        addr: SocketAddr,
        tls_config: ClientTlsConfig,
    ) -> Result<Self, ClientError> {
        let endpoint = Channel::from_shared(format!("https://{}", addr))
            .map_err(|e| ClientError::Uri(e.to_string()))?;

        let channel = endpoint.connect().await?;

        Ok(Self {
            inner: nxcc_interface::proto::vm::vm_client::VmClient::new(channel),
        })
    }

    /// Connect to a VM service over a Unix Domain Socket with TLS
    #[cfg(feature = "uds")]
    pub async fn connect_uds<P: AsRef<Path>>(
        path: P,
        tls_config: ClientTlsConfig,
    ) -> Result<Self, ClientError> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        debug!("Connecting to UDS at {}", path_str);

        // Use a dummy URI, the connector overrides it. Domain name comes from tls_config.
        let endpoint = Endpoint::try_from("http://[::]:50051") // Dummy URI
            .map_err(|e| ClientError::Uri(e.to_string()))?
            .tls_config(tls_config)?;

        let channel = endpoint
            .connect_with_connector(tower::service_fn(move |_: Uri| {
                let path = path_str.clone();
                async move {
                    let stream = UnixStream::connect(&path).await?;
                    Ok::<_, std::io::Error>(TokioIo::new(stream))
                }
            }))
            .await?;

        Ok(Self {
            inner: nxcc_interface::proto::vm::vm_client::VmClient::new(channel),
        })
    }

    /// Connect to a VM service over VSOCK with TLS
    #[cfg(feature = "vsock")]
    pub async fn connect_vsock(
        cid: u32,
        port: u32,
        tls_config: ClientTlsConfig,
    ) -> Result<Self, ClientError> {
        debug!("Connecting to VSOCK at CID {} port {}", cid, port);

        // Use a dummy URI, the connector overrides it. Domain name comes from tls_config.
        let endpoint = Endpoint::try_from("http://[::]:50051") // Dummy URI
            .map_err(|e| ClientError::Uri(e.to_string()))?
            .tls_config(tls_config)?;

        let channel = endpoint
            .connect_with_connector(tower::service_fn(move |_: Uri| {
                let cid = cid;
                let port = port;
                async move {
                    let stream =
                        VsockStream::connect(tokio_vsock::VsockAddr::new(cid, port)).await?;
                    Ok::<_, std::io::Error>(TokioIo::new(stream))
                }
            }))
            .await?;

        Ok(Self {
            inner: nxcc_interface::proto::vm::vm_client::VmClient::new(channel),
        })
    }
}

impl VmClient for VmServiceClient {
    async fn start_worker(
        &mut self,
        worker_id: String,
        worker_code: Vec<u8>,
        untrusted_config: UntrustedConfig,
        trusted_config: TrustedConfig,
    ) -> Result<String, ClientError> {
        let request = StartWorkerRequest {
            worker_id,
            worker_code,
            untrusted_config: Some(untrusted_config),
            trusted_config: Some(trusted_config),
        };

        let response = self.inner.start_worker(request).await?.into_inner();

        if response.success {
            Ok(response.id)
        } else {
            Err(ClientError::Grpc(Status::unknown(
                response.error_message.clone(),
            )))
        }
    }

    async fn stop_worker(&mut self, id: String) -> Result<(), ClientError> {
        let request = StopWorkerRequest { id };

        let response = self.inner.stop_worker(request).await?.into_inner();

        if response.success {
            Ok(())
        } else {
            Err(ClientError::Grpc(Status::unknown(
                response.error_message.clone(),
            )))
        }
    }

    async fn invoke_worker(
        &mut self,
        id: String,
        handler_name: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, ClientError> {
        let request = InvokeWorkerRequest {
            id,
            payload,
            handler_name,
        };

        let response = self.inner.invoke_worker(request).await?.into_inner();

        if response.success {
            Ok(response.result)
        } else {
            Err(ClientError::Grpc(Status::unknown(
                response.error_message.clone(),
            )))
        }
    }

    async fn invoke_http(
        &mut self,
        id: String,
        request: ProtoHttpRequest,
    ) -> Result<ProtoHttpResponse, ClientError> {
        let req = InvokeHttpRequest {
            worker_id: id,
            request: Some(request),
        };

        let response = self.inner.invoke_http(req).await?.into_inner();

        // Assuming InvokeHttpResponse directly contains HttpResponse or an error.
        // If success is indicated by gRPC status, this is simpler.
        response
            .response
            .ok_or_else(|| ClientError::Grpc(Status::internal("No HttpResponse received from VM")))
    }

    async fn get_attestation(
        &mut self,
        user_data: Vec<u8>,
    ) -> Result<AttestationReport, ClientError> {
        let request = GetAttestationRequest { user_data };

        let response = self.inner.get_attestation(request).await?.into_inner();

        match response.report {
            Some(report) => Ok(AttestationReport::from(report)),
            None => Err(ClientError::Grpc(Status::internal(
                "No attestation report received",
            ))),
        }
    }

    async fn get_worker_status(&mut self, id: String) -> Result<WorkerStatus, ClientError> {
        let request = GetWorkerStatusRequest { id };

        let response = self.inner.get_worker_status(request).await?.into_inner();

        if response.success {
            WorkerStatus::try_from(response.status)
                .map_err(|_| ClientError::Grpc(Status::internal("Invalid worker status received")))
        } else {
            Err(ClientError::Grpc(Status::unknown(
                response.error_message.clone(),
            )))
        }
    }

    async fn list_running_workers(&mut self) -> Result<Vec<String>, ClientError> {
        let request = ListRunningWorkersRequest {};

        let response = self.inner.list_running_workers(request).await?.into_inner();

        Ok(response.ids)
    }

    async fn get_worker_logs(&mut self, id: String) -> Result<String, ClientError> {
        let request = GetWorkerLogsRequest { id };

        let response = self.inner.get_worker_logs(request).await?.into_inner();

        if response.success {
            Ok(response.logs)
        } else {
            Err(ClientError::Grpc(Status::unknown(
                response.error_message.clone(),
            )))
        }
    }
}
