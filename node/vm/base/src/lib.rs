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
use rcgen::{
    Certificate as RcgenCertificate, CertificateParams, DistinguishedName, DnType, KeyPair,
    date_time_ymd,
};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use std::{error::Error, fmt, path::Path, sync::Arc};
use thiserror::Error;
use tokio::sync::Mutex;
use tonic::body::BoxBody;
use tonic::codegen::http;
use tonic::{
    Request, Response, Status,
    transport::{Certificate, ClientTlsConfig, Identity, Server, ServerTlsConfig},
};
use tower::{Layer, Service};
use tracing::{debug, error, info, warn};

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

/// Bounded client state - stores the DER certificate of the first client that connects
#[derive(Clone)]
struct BoundClient {
    inner: Arc<Mutex<Option<Vec<u8>>>>,
}

impl BoundClient {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    async fn bind_client(&self, cert_der: Vec<u8>) -> bool {
        let mut state = self.inner.lock().await;
        if state.is_none() {
            *state = Some(cert_der);
            true
        } else {
            false
        }
    }

    async fn is_bound_client(&self, cert_der: &[u8]) -> bool {
        let state = self.inner.lock().await;
        if let Some(bound_cert) = &*state {
            bound_cert == cert_der
        } else {
            // If no client is bound yet, any client can potentially bind
            // The binding logic is handled in bind_client
            // Here, we just check if the *current* request matches the *already* bound client
            // If nothing is bound, this specific client isn't the bound one (yet).
            false
        }
    }
}

/// ClientBindingLayer enforces binding to the first client that connects
#[derive(Clone)]
struct ClientBindingLayer {
    bound_client: BoundClient,
}

impl ClientBindingLayer {
    fn new(bound_client: BoundClient) -> Self {
        Self { bound_client }
    }
}

impl<S> Layer<S> for ClientBindingLayer {
    type Service = ClientBindingService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ClientBindingService {
            inner,
            bound_client: self.bound_client.clone(),
        }
    }
}

#[derive(Clone)]
struct ClientBindingService<S> {
    inner: S,
    bound_client: BoundClient,
}

impl<S> Service<http::Request<BoxBody>> for ClientBindingService<S>
where
    S: Service<http::Request<BoxBody>, Response = http::Response<BoxBody>> + Send + 'static + Clone,
    S::Future: Send + 'static,
    S::Error: Into<Box<dyn Error + Send + Sync>> + Send,
{
    type Response = S::Response;
    type Error = Box<dyn Error + Send + Sync>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: http::Request<BoxBody>) -> Self::Future {
        // Correctly extract the peer certificate DER bytes
        let client_cert_der = req
            .extensions()
            .get::<tonic::transport::server::TlsConnectInfo<tonic::transport::server::TcpConnectInfo>>() // Use TcpConnectInfo or appropriate type
            .and_then(|tls_info| tls_info.peer_certs())
            .and_then(|certs| certs.first().cloned())
            .map(|cert| cert.as_ref().to_vec()); // Convert CertificateDer<'a> to Vec<u8>

        let bound_client = self.bound_client.clone();
        let inner = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, inner);

        Box::pin(async move {
            match client_cert_der {
                Some(cert_bytes) => {
                    // Check if this client is already the bound client
                    if bound_client.is_bound_client(&cert_bytes).await {
                        // This is the bound client, let the request proceed
                        inner.call(req).await.map_err(Into::into)
                    } else {
                        // Attempt to bind this client (only succeeds if no client is bound yet)
                        if bound_client.bind_client(cert_bytes).await {
                            // This was the first client, now bound
                            info!("First client connected and bound to service");
                            inner.call(req).await.map_err(Into::into)
                        } else {
                            // A *different* client is already bound, reject this one
                            warn!("Rejected request from non-bound client");
                            let status =
                                Status::permission_denied("Service is bound to another client");
                            // Create a valid HTTP response for gRPC status
                            let response = http::Response::builder()
                                .status(http::StatusCode::FORBIDDEN) // Or appropriate HTTP status
                                .header("content-type", "application/grpc")
                                .header("grpc-status", status.code().to_string())
                                .header("grpc-message", status.message())
                                .body(BoxBody::default())
                                .unwrap(); // Handle error appropriately
                            Ok(response) // Return Ok(response) for Layer errors that map to gRPC status
                        }
                    }
                }
                None => {
                    // No client certificate, reject
                    error!("Rejected request with no client certificate");
                    let status = Status::unauthenticated("Client certificate required");
                    // Create a valid HTTP response for gRPC status
                    let response = http::Response::builder()
                        .status(http::StatusCode::UNAUTHORIZED) // Or appropriate HTTP status
                        .header("content-type", "application/grpc")
                        .header("grpc-status", status.code().to_string())
                        .header("grpc-message", status.message())
                        .body(BoxBody::default())
                        .unwrap(); // Handle error appropriately
                    Ok(response) // Return Ok(response) for Layer errors that map to gRPC status
                }
            }
        })
    }
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
                let proto_report: proto_interface::AttestationReport = report.to_proto();
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

/// Generate a self-signed certificate for mTLS using modern rcgen
fn generate_self_signed_cert() -> Result<(String, String), Box<dyn Error + Send + Sync>> {
    // Create parameters for the certificate
    let mut params = CertificateParams::new(vec!["localhost".to_string()])?;

    // Set validity period (optional, defaults are usually fine)
    // params.not_before = date_time_ymd(2023, 1, 1);
    // params.not_after = date_time_ymd(2033, 12, 31);

    // Set distinguished name (required)
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(DnType::CommonName, "localhost");
    params.distinguished_name = distinguished_name;

    // Generate a key pair
    let key_pair = KeyPair::generate()?; // Or specify algorithm: KeyPair::generate(&rcgen::PKCS_ECDSA_P256_SHA256)?;

    // Sign the certificate using the key pair
    let cert = params.self_signed(&key_pair)?;

    Ok((cert.pem(), key_pair.serialize_pem()))
}

/// Starts the gRPC server for the VM service with mTLS.
///
/// This function takes ownership of the runtime implementation and runs the
/// server indefinitely until an error occurs or the process is terminated.
///
/// # Arguments
/// * `config` - The server listening configuration (UDS, VSOCK, or TCP).
/// * `runtime` - An Arc-wrapped implementation of the `VmRuntime` trait.
///
/// # Errors
/// Returns an error if the server fails to start or encounters a runtime error.
pub async fn run_vm_server<T: VmRuntime>(
    config: ServerConfig,
    runtime: Arc<T>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Generate server's self-signed certificate for mTLS
    let (server_cert_pem, server_key_pem) = generate_self_signed_cert()?;
    let server_identity = Identity::from_pem(server_cert_pem.clone(), server_key_pem);

    // Create bound client state for client binding
    let bound_client = BoundClient::new();

    // Create the client binding layer
    let client_binding_layer = ClientBindingLayer::new(bound_client);

    // Create the gRPC service
    let grpc_service = VmServiceGrpc::new(runtime);

    // Create server CA certificate (using its own cert as CA for self-signed mTLS)
    let server_ca_cert = Certificate::from_pem(server_cert_pem);

    // Configure TLS with client certificate verification
    // client_auth_optional(true) allows the connection initially,
    // but our ClientBindingLayer will enforce the cert presence and binding.
    let server_tls_config = ServerTlsConfig::new()
        .identity(server_identity)
        .client_ca_root(server_ca_cert)
        .client_auth_optional(true); // Layer enforces non-optional

    // Build the server with TLS and the client binding layer
    let server_builder = Server::builder()
        .tls_config(server_tls_config)?
        .layer(client_binding_layer)
        .add_service(VmServer::new(grpc_service));

    match config {
        #[cfg(feature = "uds")]
        ServerConfig::Uds { path } => {
            info!("VM gRPC Server listening on UDS: {}", path);
            let path = Path::new(&path);
            // Clean up existing socket file if necessary
            if path.exists() {
                warn!("Removing existing UDS file: {}", path.display());
                std::fs::remove_file(path)?;
            }

            let listener = UnixListener::bind(path)?;
            let incoming = UnixListenerStream::new(listener);
            info!("UDS Server started.");
            server_builder.serve_with_incoming(incoming).await?;
            info!("UDS Server stopped. Cleaning up socket file.");
            let _ = std::fs::remove_file(path); // Clean up on shutdown
        }
        #[cfg(feature = "vsock")]
        ServerConfig::Vsock { cid, port } => {
            info!(
                "VM gRPC Server listening on vsock: CID={}, port={}",
                cid, port
            );
            let listener = VsockListener::bind(VsockAddr::new(cid, port))?;
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

/// Client configuration for connecting to the VM gRPC service with mTLS.
#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// Server address URI (e.g., "http://localhost:50051", "unix:/path/to/socket").
    pub server_uri: String,
    /// Timeout for connection attempts.
    pub timeout: Duration,
    /// The server's root CA certificate (DER format). In our self-signed case, this is the server's own certificate.
    pub server_ca_cert_der: Vec<u8>,
    /// The client's certificate (DER format).
    pub client_cert_der: Vec<u8>,
    /// The client's private key (DER format).
    pub client_key_der: Vec<u8>,
}

/// Generate client TLS configuration using provided certificates and keys.
pub fn create_client_tls_config(
    server_ca_cert_pem: String,
    client_cert_pem: Vec<u8>,
    client_key_pem: Vec<u8>,
    domain_name: &str, // e.g., "localhost"
) -> Result<ClientTlsConfig, Box<dyn Error + Send + Sync>> {
    // Create client identity
    let client_identity = Identity::from_pem(client_cert_pem, client_key_pem);

    // Create server CA certificate object
    let server_ca_cert = Certificate::from_pem(server_ca_cert_pem);

    // Create client TLS config, validating the server using its cert as CA
    let client_tls_config = ClientTlsConfig::new()
        .identity(client_identity)
        .ca_certificate(server_ca_cert)
        .domain_name(domain_name); // Must match CN or SAN in server cert

    Ok(client_tls_config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nxcc_interface::{
        proto::vm::{
            GetAttestationRequest, InvokeWorkerRequest, StartWorkerRequest, StopWorkerRequest,
            vm_client::VmClient, vm_server::Vm,
        },
        types::{AttestationReport, FromProto as _},
    };
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex as StdMutex},
    };
    use tokio::net::TcpListener;
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::transport::Endpoint;

    // ===== Mock runtime (remains the same) =====
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
        state: Arc<StdMutex<MockVmRuntimeState>>,
    }

    impl MockVmRuntime {
        fn new() -> Self {
            Self {
                state: Arc::new(StdMutex::new(MockVmRuntimeState::default())),
            }
        }

        /* ------------- helpers copied from suite #1 ------------- */
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

    // ===== Test the mTLS and client binding functionality =====
    #[tokio::test]
    #[cfg(feature = "tcp")] // Only run this test if TCP feature is enabled
    async fn test_mtls_client_binding() -> Result<(), Box<dyn Error + Send + Sync>> {
        // Setup server
        let addr = "127.0.0.1:0"; // Use port 0 for OS to pick an available port
        let listener = TcpListener::bind(addr).await?;
        let server_addr = listener.local_addr()?;
        let server_uri = format!("https://{}", server_addr); // Use https for TLS
        let domain = "localhost"; // Domain name for certs and TLS config

        let (_, runtime) = setup();

        // Generate server certificate
        let (server_cert_der, server_key_der) = generate_self_signed_cert()?;
        let server_identity = Identity::from(server_cert_der.clone(), server_key_der.clone())?;

        // Create server CA certificate object (using its own cert)
        let server_ca_cert = Certificate::from_der(server_cert_der.clone())?;

        // Configure server TLS
        let server_tls_config = ServerTlsConfig::new()
            .identity(server_identity)
            .client_ca_root(server_ca_cert.clone()) // Use the server's cert as the CA root
            .client_auth_optional(true); // Layer will enforce non-optional

        // Create bound client state
        let bound_client = BoundClient::new();
        let client_binding_layer = ClientBindingLayer::new(bound_client);

        // Start server in background
        let server_handle = tokio::spawn(async move {
            Server::builder()
                .tls_config(server_tls_config)
                .unwrap()
                .layer(client_binding_layer)
                .add_service(VmServer::new(VmServiceGrpc::new(runtime)))
                .serve_with_incoming(TcpListenerStream::new(listener))
                .await
        });

        // Wait briefly for server to start
        tokio::time::sleep(Duration::from_millis(200)).await;

        // --- Client 1 Setup ---
        let (client1_cert_der, client1_key_der) = generate_self_signed_cert()?;
        let client1_tls_config = create_client_tls_config(
            server_cert_der.clone(), // Server's cert is the CA
            client1_cert_der.clone(),
            client1_key_der.clone(),
            domain,
        )?;
        let client1_endpoint = Endpoint::new(server_uri.clone())?
            .tls_config(client1_tls_config)?
            .connect_timeout(Duration::from_secs(5));

        // --- Client 2 Setup ---
        let (client2_cert_der, client2_key_der) = generate_self_signed_cert()?;
        let client2_tls_config = create_client_tls_config(
            server_cert_der.clone(), // Server's cert is the CA
            client2_cert_der.clone(),
            client2_key_der.clone(),
            domain,
        )?;
        let client2_endpoint = Endpoint::new(server_uri.clone())?
            .tls_config(client2_tls_config)?
            .connect_timeout(Duration::from_secs(5));

        // --- Test Logic ---
        // Connect first client
        let mut client1 = VmClient::connect(client1_endpoint).await?;
        info!("Client 1 connected");

        // First client should be able to make a request (and bind)
        let response1 = client1
            .get_attestation(Request::new(GetAttestationRequest {
                user_data: vec![1, 2, 3],
            }))
            .await;
        info!("Client 1 GetAttestation result: {:?}", response1);
        assert!(
            response1.is_ok(),
            "Client 1 initial request failed: {:?}",
            response1.err()
        );
        assert!(response1.unwrap().into_inner().report.is_some());
        info!("Client 1 GetAttestation successful");

        // Connect second client
        let mut client2 = VmClient::connect(client2_endpoint).await?;
        info!("Client 2 connected");

        // Second client should NOT be able to make requests
        let result2 = client2
            .get_attestation(Request::new(GetAttestationRequest {
                user_data: vec![4, 5, 6],
            }))
            .await;
        info!("Client 2 GetAttestation result: {:?}", result2);
        assert!(result2.is_err(), "Client 2 request unexpectedly succeeded");
        let status = result2.unwrap_err();
        assert_eq!(
            status.code(),
            tonic::Code::PermissionDenied,
            "Client 2 error code mismatch"
        );
        assert!(
            status.message().contains("bound to another client"),
            "Client 2 error message mismatch"
        );
        info!("Client 2 GetAttestation correctly rejected");

        // First client should still be able to make requests
        let response3 = client1
            .start_worker(Request::new(StartWorkerRequest {
                worker_id: "test-worker".to_string(),
                worker_code: vec![10, 20, 30],
                config: vec![],
            }))
            .await;
        info!("Client 1 StartWorker result: {:?}", response3);
        assert!(
            response3.is_ok(),
            "Client 1 second request failed: {:?}",
            response3.err()
        );
        assert!(response3.unwrap().into_inner().success);
        info!("Client 1 StartWorker successful");

        // Cleanup
        server_handle.abort();
        Ok(())
    }

    // ======= Other TESTS (should remain largely the same) ========================

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

    #[tokio::test]
    async fn test_bound_client() {
        let bound_client = BoundClient::new();

        // Bind first client
        let cert1 = vec![1, 2, 3];
        assert!(bound_client.bind_client(cert1.clone()).await);

        // Same client should be recognized
        assert!(bound_client.is_bound_client(&cert1).await);

        // Different client should be rejected
        let cert2 = vec![4, 5, 6];
        assert!(!bound_client.is_bound_client(&cert2).await);

        // Cannot bind second client
        assert!(!bound_client.bind_client(cert2.clone()).await);

        // Original client should still be recognized
        assert!(bound_client.is_bound_client(&cert1).await);
    }

    #[test]
    fn test_generate_self_signed_cert() {
        let result = generate_self_signed_cert();
        assert!(result.is_ok());

        let (cert_pem, key_pem) = result.unwrap();
        assert!(!cert_pem.is_empty());
        assert!(!key_pem.is_empty());

        // Should be able to create identity
        let _identity_result = Identity::from_pem(cert_pem, key_pem);
    }
}
