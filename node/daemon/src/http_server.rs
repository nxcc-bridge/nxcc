use std::{
    collections::{HashMap, HashSet},
    future::Future,
    net::SocketAddr,
    sync::Arc,
};

use axum::{
    body::{Body, Bytes},
    extract::{Json, Path, Query, State},
    http::{self, HeaderMap, HeaderValue, Request, StatusCode},
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{Router, any, get, post},
};
use nxcc_interface::types::worker::DsseEnvelope;
use serde::Deserialize;
use tokio::sync::RwLock;
use tokio_stream::StreamExt;
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info, instrument, warn};

use crate::{
    config::HttpConfig,
    error::AppError,
    grpc::enclave_client::EnclaveClient,
    services::{secrets::SecretsService, work_order_orchestrator::WorkOrderOrchestrator},
};

/// Registry for tracking attached VMs
#[derive(Debug, Clone, Default)]
pub struct VmRegistry {
    /// Set of attached VM IDs
    attached_vms: Arc<RwLock<HashSet<String>>>,
}

/// Registry for tracking connected peers
#[derive(Debug, Clone, Default)]
pub struct PeerRegistry {
    /// Map of connected peer IDs to their set of multiaddrs
    connected_peers: Arc<RwLock<HashMap<String, HashSet<String>>>>,
}

impl PeerRegistry {
    pub fn new() -> Self {
        Self {
            connected_peers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a connected peer with its multiaddr
    pub async fn add_peer(&self, peer_id: String, multiaddr: String) {
        let mut peers = self.connected_peers.write().await;
        peers
            .entry(peer_id)
            .or_insert_with(HashSet::new)
            .insert(multiaddr);
    }

    /// Remove a specific multiaddr for a peer
    pub async fn remove_peer_addr(&self, peer_id: &str, multiaddr: &str) {
        let mut peers = self.connected_peers.write().await;
        if let Some(addrs) = peers.get_mut(peer_id) {
            addrs.remove(multiaddr);
            if addrs.is_empty() {
                peers.remove(peer_id);
            }
        }
    }

    /// Remove all addresses for a peer
    pub async fn remove_peer(&self, peer_id: &str) {
        let mut peers = self.connected_peers.write().await;
        peers.remove(peer_id);
    }

    /// Get a map of all connected peers and their multiaddrs
    pub async fn get_connected_peers(&self) -> HashMap<String, HashSet<String>> {
        let peers = self.connected_peers.read().await;
        peers.clone()
    }
}

impl VmRegistry {
    pub fn new() -> Self {
        Self {
            attached_vms: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// Add a VM ID to the registry
    pub async fn add_vm(&self, vm_id: String) {
        let mut vms = self.attached_vms.write().await;
        vms.insert(vm_id);
    }

    /// Remove a VM ID from the registry
    pub async fn remove_vm(&self, vm_id: &str) {
        let mut vms = self.attached_vms.write().await;
        vms.remove(vm_id);
    }

    /// Get a list of all attached VM IDs
    pub async fn list_vms(&self) -> Vec<String> {
        let vms = self.attached_vms.read().await;
        vms.iter().cloned().collect()
    }
}

/// Shared application state available to all handlers.
struct AppState {
    enclave_client: EnclaveClient,
    /// Maps a URL path segment to a worker ID.
    /// e.g., "my-worker" -> "enclave_worker_id_123"
    http_mounts: Arc<RwLock<HashMap<String, String>>>,
    work_order_orchestrator: Arc<WorkOrderOrchestrator>,
    /// Local peer identity for P2P networking
    local_key: libp2p::identity::Keypair,
    /// Registry of attached VMs
    vm_registry: VmRegistry,
    /// Registry of connected peers
    peer_registry: PeerRegistry,
    /// Secrets service for env report generation
    secrets_service: Arc<SecretsService>,
}

#[derive(serde::Serialize)]
struct ApiErrorResponse {
    error: String,
}

#[derive(serde::Serialize)]
struct SubmitWorkOrderSuccessResponse {
    work_order_id: String,
    message: String,
}

#[derive(serde::Serialize)]
struct StatusResponse {
    health: String,
    peer_id: String,
    connected_peers: HashMap<String, HashSet<String>>,
    vm_ids: Vec<String>,
}

/// A wrapper for `AppError` to provide an `IntoResponse` implementation for the API layer.
/// This allows handlers to return `Result<_, AppError>` and have errors automatically
/// converted into a user-facing JSON response.
struct ApiError(AppError);

impl From<AppError> for ApiError {
    fn from(err: AppError) -> Self {
        ApiError(err)
    }
}

impl From<String> for ApiError {
    fn from(err: String) -> Self {
        ApiError(AppError::Internal(err))
    }
}

impl From<&str> for ApiError {
    fn from(err: &str) -> Self {
        ApiError(AppError::Internal(err.to_string()))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let app_error = self.0;
        let (status, error_message) = match &app_error {
            AppError::Validation(_) => (StatusCode::BAD_REQUEST, app_error.to_string()),
            AppError::Authorization(_) => (StatusCode::FORBIDDEN, app_error.to_string()),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred".to_string(),
            ),
        };

        error!("API Error: {} - {}", status, app_error);

        let body = Json(ApiErrorResponse {
            error: error_message,
        });

        (status, body).into_response()
    }
}

// --- API Handlers ---

/// Handles forwarding arbitrary HTTP requests to a mounted worker enclave.
/// The first path segment determines the worker, and the rest of the path is forwarded.
/// e.g., a request to `/my-worker/some/path` will be forwarded to the worker mounted at "my-worker".
#[instrument(skip_all, fields(uri = %request.uri()))]
async fn universal_http_handler(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
) -> Response {
    let path = request.uri().path().trim_start_matches('/');
    let (mount_segment, worker_path) = path.split_once('/').unwrap_or((path, ""));

    if mount_segment.is_empty() {
        debug!("Request is missing mount segment: {}", path);
        return (StatusCode::NOT_FOUND, "Not Found").into_response();
    }

    let enclave_worker_id = {
        let mounts = state.http_mounts.read().await;
        // Look up the worker ID using the mount segment.
        mounts.get(mount_segment).cloned()
    };

    let enclave_worker_id = match enclave_worker_id {
        Some(id) => id,
        None => {
            debug!("No worker mounted at segment: {}", mount_segment);
            return (
                StatusCode::NOT_FOUND,
                format!("No worker mounted at segment: {}", mount_segment),
            )
                .into_response();
        }
    };

    // Reconstruct the URI to be sent to the worker, preserving the query string.
    let mut worker_uri = format!("/{}", worker_path);
    if let Some(query) = request.uri().query() {
        worker_uri.push('?');
        worker_uri.push_str(query);
    }

    let method = request.method().clone();
    let headers = request.headers().clone();
    let mount_segment = mount_segment.to_string(); // Release the borrow on request, but preserve the mount segment for debugging.
    let body = match axum::body::to_bytes(request.into_body(), usize::MAX).await {
        Ok(bytes) => bytes.to_vec(),
        Err(e) => {
            error!("Failed to read request body: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to read request body",
            )
                .into_response();
        }
    };

    // Convert HTTP headers to the protobuf format expected by the enclave client.
    let proto_headers = headers
        .iter()
        .map(|(key, value)| nxcc_interface::proto::vm::Header {
            key: key.as_str().to_string(),
            value: value.as_bytes().to_vec(),
        })
        .collect();

    let vm_http_request = nxcc_interface::proto::vm::HttpRequest {
        method: method.to_string(),
        uri: worker_uri.clone(),
        headers: proto_headers,
        body,
    };

    debug!(
        "Forwarding HTTP request (method: {}, uri: {}) to enclave_worker_id: {}",
        method, worker_uri, enclave_worker_id
    );

    match state
        .enclave_client
        .invoke_http_worker(enclave_worker_id.clone(), vm_http_request)
        .await
    {
        Ok(vm_http_response) => {
            let mut response_builder = Response::builder().status(
                StatusCode::from_u16(vm_http_response.status_code as u16)
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            );

            for header in vm_http_response.headers {
                if let (Ok(name), Ok(value)) = (
                    http::header::HeaderName::from_bytes(header.key.as_bytes()),
                    http::header::HeaderValue::from_bytes(&header.value),
                ) {
                    response_builder = response_builder.header(name, value);
                } else {
                    warn!("Failed to parse header from worker: key='{}'", header.key);
                }
            }

            response_builder
                .body(Body::from(vm_http_response.body))
                .unwrap_or_else(|e| {
                    error!("Failed to construct response: {}", e);
                    (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error").into_response()
                })
        }
        Err(e) => {
            error!(
                "Error invoking HTTP worker {} (segment {}): {}",
                enclave_worker_id, mount_segment, e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Error processing request: {}", e),
            )
                .into_response()
        }
    }
}

/// Handles submission of a new work order.
/// It accepts a DSSE envelope in the request body.
async fn submit_work_order_handler(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    info!("Received HTTP SubmitWorkOrder request");

    let (work_order_id, message) = state
        .work_order_orchestrator
        .clone()
        .submit_work_order(body.to_vec())
        .await
        .map_err(ApiError)?; // Use the ApiError wrapper for automatic response conversion

    Ok((
        StatusCode::ACCEPTED,
        Json(SubmitWorkOrderSuccessResponse {
            work_order_id,
            message,
        }),
    ))
}

/// Handles status requests, returning node health and P2P information.
async fn status_handler(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, ApiError> {
    info!("Received status request");

    // Get peer ID from local key
    let peer_id = state.local_key.public().to_peer_id().to_string();

    // Get VM list from local registry
    let vm_ids = state.vm_registry.list_vms().await;

    // Get connected peers from peer registry
    let connected_peers = state.peer_registry.get_connected_peers().await;

    Ok(Json(StatusResponse {
        health: "ok".to_string(),
        peer_id,
        connected_peers,
        vm_ids,
    }))
}

#[derive(serde::Serialize)]
struct EnvReportResponse {
    attestation: serde_json::Value,
    operator_signature: Option<serde_json::Value>,
}

/// Handles env report requests, returning the node's environment report including attestation and operator signature.
async fn env_report_handler(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, ApiError> {
    info!("Received env report request");

    // Get the env report from the secrets service
    let env_report = state
        .secrets_service
        .get_own_env_report()
        .await
        .map_err(|e| ApiError::from(format!("Failed to get env report: {}", e)))?;

    // Convert to JSON for response
    let attestation_json = serde_json::to_value(&env_report.attestation)
        .map_err(|e| ApiError::from(format!("Failed to serialize attestation: {}", e)))?;

    let operator_signature_json = env_report
        .operator_signature
        .map(|sig| serde_json::to_value(&sig))
        .transpose()
        .map_err(|e| ApiError::from(format!("Failed to serialize operator signature: {}", e)))?;

    Ok(Json(EnvReportResponse {
        attestation: attestation_json,
        operator_signature: operator_signature_json,
    }))
}

/// Query parameters for worker logs API
#[derive(Deserialize, serde::Serialize)]
struct WorkerLogsQuery {
    /// Number of lines to tail (optional)
    #[serde(rename = "tail")]
    tail_lines: Option<u32>,
    /// Whether to follow/stream logs (optional, defaults to false)
    #[serde(default)]
    follow: bool,
}

/// Handles worker log streaming via Server-Sent Events
async fn worker_logs_handler(
    State(state): State<Arc<AppState>>,
    Path(worker_id): Path<String>,
    Query(params): Query<WorkerLogsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    info!(
        "Received worker logs request for worker_id: {} with params: tail={:?}, follow={}",
        worker_id, params.tail_lines, params.follow
    );

    // If follow is false, return static logs
    if !params.follow {
        // For non-streaming requests, we would typically get logs and return them as JSON
        // But since we want to support streaming, let's redirect to SSE even for static logs
        return Err(ApiError::from(
            "Non-streaming logs not implemented yet. Use follow=true for streaming.",
        ));
    }

    // Create a stream from the enclave
    let log_stream = match state
        .enclave_client
        .stream_worker_logs(
            worker_id.clone(),
            params.tail_lines.unwrap_or(0),
            params.follow,
        )
        .await
    {
        Ok(stream) => stream,
        Err(e) => {
            error!("Failed to start log stream for worker {}: {}", worker_id, e);
            return Err(ApiError::from(format!("Failed to start log stream: {}", e)));
        }
    };

    // Convert the gRPC stream to SSE events
    let sse_stream = log_stream.map(|result| match result {
        Ok(log_response) => {
            Ok::<Event, axum::BoxError>(Event::default().data(log_response.log_line).event("log"))
        }
        Err(e) => Ok::<Event, axum::BoxError>(
            Event::default()
                .data(format!("Error: {}", e))
                .event("error"),
        ),
    });

    Ok(Sse::new(sse_stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

/// Configures and starts the HTTP server.
pub async fn start_http_server(
    config: &HttpConfig,
    http_mounts: Arc<RwLock<HashMap<String, String>>>,
    enclave_client: EnclaveClient,
    work_order_orchestrator: Arc<WorkOrderOrchestrator>,
    local_key: libp2p::identity::Keypair,
    vm_registry: VmRegistry,
    peer_registry: PeerRegistry,
    secrets_service: Arc<SecretsService>,
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
) -> Result<(), anyhow::Error> {
    let worker_base_path = config.base_mount_path.trim_end_matches('/');
    if !worker_base_path.starts_with('/') && !worker_base_path.is_empty() {
        anyhow::bail!("HTTP base_mount_path must start with '/' or be empty for root");
    }

    let app_state = Arc::new(AppState {
        enclave_client,
        http_mounts,
        work_order_orchestrator,
        local_key,
        vm_registry,
        peer_registry,
        secrets_service,
    });

    // --- Router Configuration ---

    let mut app = Router::new();

    // Conditionally add the `/api` routes
    if config.api_enabled {
        tracing::info!("Setting up API routes including /workers/{{worker_id}}/logs");
        let mut api_router = Router::new()
            .route("/work-orders", post(submit_work_order_handler))
            .route("/status", get(status_handler))
            .route("/env-report", get(env_report_handler))
            .route("/workers/{worker_id}/logs", get(worker_logs_handler));

        // Conditionally add CORS layer to the API router
        if !config.api_cors_allowed_origins.is_empty() {
            let origins = config
                .api_cors_allowed_origins
                .iter()
                .map(|s| s.parse().expect("Invalid CORS origin"))
                .collect::<Vec<_>>();

            let cors = CorsLayer::new()
                .allow_origin(origins)
                .allow_methods(Any)
                .allow_headers(Any);
            api_router = api_router.layer(cors);
        }
        app = app.nest("/api", api_router);
    }

    // The worker router acts as a fallback for any request under its mount point.
    let worker_router = Router::new().fallback(any(universal_http_handler));

    // Combine the routers. The worker router is either nested under a base path
    // or used as a fallback for the entire application if the base path is root.
    let app = if worker_base_path.is_empty() {
        app.fallback_service(worker_router.with_state(app_state.clone()))
    } else {
        app.nest(
            worker_base_path,
            worker_router.with_state(app_state.clone()),
        )
    };

    // Apply the shared state to the final, composed application.
    let app = app.with_state(app_state);

    let addr: SocketAddr = config.http_listen_addr.parse()?;
    tracing::info!(
        "HTTP server listening on {} for worker path '{}' and API path '/api' (enabled: {})",
        addr,
        config.base_mount_path,
        config.api_enabled
    );
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app)
        .with_graceful_shutdown(shutdown_signal)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_worker_logs_query_parsing() {
        // Test default values
        let query_str = "";
        let parsed: WorkerLogsQuery = serde_urlencoded::from_str(query_str).unwrap();
        assert_eq!(parsed.tail_lines, None);
        assert_eq!(parsed.follow, false);

        // Test with follow=true
        let query_str = "follow=true";
        let parsed: WorkerLogsQuery = serde_urlencoded::from_str(query_str).unwrap();
        assert_eq!(parsed.tail_lines, None);
        assert_eq!(parsed.follow, true);

        // Test with tail parameter
        let query_str = "tail=10&follow=true";
        let parsed: WorkerLogsQuery = serde_urlencoded::from_str(query_str).unwrap();
        assert_eq!(parsed.tail_lines, Some(10));
        assert_eq!(parsed.follow, true);

        // Test with both parameters
        let query_str = "tail=25&follow=false";
        let parsed: WorkerLogsQuery = serde_urlencoded::from_str(query_str).unwrap();
        assert_eq!(parsed.tail_lines, Some(25));
        assert_eq!(parsed.follow, false);

        // Test edge cases
        let query_str = "tail=0&follow=true";
        let parsed: WorkerLogsQuery = serde_urlencoded::from_str(query_str).unwrap();
        assert_eq!(parsed.tail_lines, Some(0));
        assert_eq!(parsed.follow, true);
    }

    #[tokio::test]
    async fn test_worker_logs_query_invalid_parsing() {
        // Test invalid tail parameter
        let query_str = "tail=invalid&follow=true";
        let result: Result<WorkerLogsQuery, _> = serde_urlencoded::from_str(query_str);
        assert!(
            result.is_err(),
            "Should fail to parse invalid tail parameter"
        );

        // Test negative tail parameter
        let query_str = "tail=-5&follow=true";
        let result: Result<WorkerLogsQuery, _> = serde_urlencoded::from_str(query_str);
        assert!(
            result.is_err(),
            "Should fail to parse negative tail parameter"
        );
    }

    #[tokio::test]
    async fn test_api_error_conversion() {
        // Test String to ApiError conversion
        let error = ApiError::from("Test error message");
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        // Test &str to ApiError conversion
        let error = ApiError::from("Another error");
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        // Test AppError to ApiError conversion
        let app_error = AppError::Internal("Internal error".to_string());
        let api_error = ApiError::from(app_error);
        let response = api_error.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_api_error_response_format() {
        let error = ApiError::from("Test error");
        let response = error.into_response();

        // Check that response has correct content type
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/json"
        );
    }

    #[tokio::test]
    async fn test_vm_registry_basic_operations() {
        let registry = VmRegistry::new();

        // Initially empty
        assert_eq!(registry.list_vms().await.len(), 0);

        // Add VMs
        registry.add_vm("vm-1".to_string()).await;
        registry.add_vm("vm-2".to_string()).await;

        let vms = registry.list_vms().await;
        assert_eq!(vms.len(), 2);
        assert!(vms.contains(&"vm-1".to_string()));
        assert!(vms.contains(&"vm-2".to_string()));

        // Remove VM
        registry.remove_vm("vm-1").await;
        let vms = registry.list_vms().await;
        assert_eq!(vms.len(), 1);
        assert!(vms.contains(&"vm-2".to_string()));
        assert!(!vms.contains(&"vm-1".to_string()));

        // Remove non-existent VM (should not error)
        registry.remove_vm("non-existent").await;
        let vms = registry.list_vms().await;
        assert_eq!(vms.len(), 1);
    }

    #[tokio::test]
    async fn test_vm_registry_duplicate_vms() {
        let registry = VmRegistry::new();

        // Add same VM twice
        registry.add_vm("vm-1".to_string()).await;
        registry.add_vm("vm-1".to_string()).await;

        // Should only contain one instance
        let vms = registry.list_vms().await;
        assert_eq!(vms.len(), 1);
        assert!(vms.contains(&"vm-1".to_string()));
    }

    #[test]
    fn test_worker_logs_query_serde_attributes() {
        // Test that serde rename works correctly
        let query = WorkerLogsQuery {
            tail_lines: Some(10),
            follow: true,
        };

        // Serialize to query string
        let serialized = serde_urlencoded::to_string(&query).unwrap();

        // Should use 'tail' instead of 'tail_lines'
        assert!(serialized.contains("tail=10"));
        assert!(serialized.contains("follow=true"));
    }
}
