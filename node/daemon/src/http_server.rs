use std::{collections::HashMap, future::Future, net::SocketAddr, sync::Arc};

use axum::{
    body::{Body, Bytes},
    extract::{Json, State},
    http::{self, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, post, Router},
};
use nxcc_interface::types::DsseEnvelope;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info, instrument, warn};

use crate::{
    config::HttpConfig, error::AppError, grpc::enclave_client::EnclaveClient,
    services::work_order_orchestrator::WorkOrderOrchestrator,
};

/// Shared application state available to all handlers.
struct AppState {
    enclave_client: EnclaveClient,
    /// Maps a URL path segment to a worker ID.
    /// e.g., "my-worker" -> "enclave_worker_id_123"
    http_mounts: Arc<RwLock<HashMap<String, String>>>,
    work_order_orchestrator: Arc<WorkOrderOrchestrator>,
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

/// A wrapper for `AppError` to provide an `IntoResponse` implementation for the API layer.
/// This allows handlers to return `Result<_, AppError>` and have errors automatically
/// converted into a user-facing JSON response.
struct ApiError(AppError);

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
                    warn!(
                        "Failed to parse header from worker: key='{}'",
                        header.key
                    );
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

/// Configures and starts the HTTP server.
pub async fn start_http_server(
    config: &HttpConfig,
    http_mounts: Arc<RwLock<HashMap<String, String>>>,
    enclave_client: EnclaveClient,
    work_order_orchestrator: Arc<WorkOrderOrchestrator>,
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
    });

    // --- Router Configuration ---

    let mut app = Router::new();

    // Conditionally add the `/api` routes
    if config.api_enabled {
        let mut api_router =
            Router::new().route("/work-orders", post(submit_work_order_handler));

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
        app.nest(worker_base_path, worker_router)
    };

    // Apply the shared state to the final, composed application.
    let app = app.with_state(app_state);

    let addr: SocketAddr = config.listen_addr.parse()?;
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
