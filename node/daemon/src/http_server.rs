use std::{collections::HashMap, future::Future, net::SocketAddr, sync::Arc};

use axum::{
    Router,
    body::Body,
    extract::{Json, State},
    http::{Request, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::post,
};
use nxcc_interface::types::DsseEnvelope;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info, warn};

use crate::{
    config::HttpConfig, error::AppError, grpc::enclave_client::EnclaveClient,
    services::work_order_orchestrator::WorkOrderOrchestrator,
};

struct AppState {
    enclave_client: EnclaveClient,
    http_mounts: Arc<RwLock<HashMap<String, String>>>, // mount_segment (hash) -> enclave_worker_id
    base_mount_path: String,
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

async fn universal_http_handler(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
) -> impl IntoResponse {
    let path = request.uri().path().to_string();
    let path = path.trim_start_matches('/');
    let mut segments = path.splitn(2, '/');
    let mount_segment = segments.next().unwrap_or("").to_string();
    let worker_path_segment = segments.next().unwrap_or("");

    if mount_segment.is_empty() {
        debug!("Missing mount segment in path_after_base: {}", path);
        return (StatusCode::NOT_FOUND, "Missing mount segment").into_response();
    }

    let enclave_worker_id = {
        let mounts = state.http_mounts.read().await;
        mounts.get(&mount_segment).cloned()
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

    let mut worker_uri_str = format!("/{}", worker_path_segment.trim_start_matches('/'));
    if let Some(query) = request.uri().query() {
        worker_uri_str.push('?');
        worker_uri_str.push_str(query);
    }
    // Ensure worker_uri_str always starts with a single slash if not empty
    if worker_uri_str != "/" && !worker_uri_str.starts_with('/') && !worker_uri_str.is_empty() {
        worker_uri_str = format!("/{}", worker_uri_str);
    } else if worker_uri_str.is_empty() {
        worker_uri_str = "/".to_string();
    }

    let method = request.method().to_string();
    let headers = request.headers().clone();
    let body_bytes = match axum::body::to_bytes(request.into_body(), usize::MAX).await {
        Ok(bytes) => bytes,
        Err(e) => {
            error!("Failed to read request body: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to read request body",
            )
                .into_response();
        }
    };

    let mut proto_headers = Vec::new();
    for (key, value) in headers.iter() {
        proto_headers.push(nxcc_interface::proto::vm::Header {
            key: key.as_str().to_string(),
            value: value.as_bytes().to_vec(),
        });
    }

    let vm_http_request = nxcc_interface::proto::vm::HttpRequest {
        method,
        uri: worker_uri_str.clone(),
        headers: proto_headers,
        body: body_bytes.to_vec(),
    };

    debug!(
        "Forwarding HTTP request (method: {}, uri: {}) to enclave_worker_id: {}, segment: {}",
        vm_http_request.method, worker_uri_str, enclave_worker_id, mount_segment
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

            for header_proto in vm_http_response.headers {
                if let (Ok(name), Ok(value)) = (
                    axum::http::header::HeaderName::from_bytes(header_proto.key.as_bytes()),
                    axum::http::header::HeaderValue::from_bytes(&header_proto.value),
                ) {
                    response_builder = response_builder.header(name, value);
                } else {
                    warn!(
                        "Failed to parse header from worker: {}={:?}",
                        header_proto.key, header_proto.value
                    );
                }
            }

            response_builder
                .body(axum::body::Body::from(vm_http_response.body))
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

async fn submit_work_order_handler(
    State(state): State<Arc<AppState>>,
    Json(dsse_envelope): Json<DsseEnvelope>,
) -> impl IntoResponse {
    info!("Received HTTP SubmitWorkOrder request");
    let work_order_dsse_bytes = match serde_json::to_vec(&dsse_envelope) {
        Ok(bytes) => bytes,
        Err(e) => {
            error!("Failed to serialize incoming DSSE envelope: {}", e);
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResponse {
                    error: "Failed to process DSSE envelope".to_string(),
                }),
            )
                .into_response();
        }
    };

    match state
        .work_order_orchestrator
        .clone()
        .submit_work_order(work_order_dsse_bytes)
        .await
    {
        Ok((work_order_id, message)) => (
            StatusCode::ACCEPTED,
            Json(SubmitWorkOrderSuccessResponse {
                work_order_id,
                message,
            }),
        )
            .into_response(),
        Err(e) => {
            error!("SubmitWorkOrder failed via HTTP: {:?}", e);
            let (status, error_message) = match e {
                AppError::Validation(_) => (StatusCode::BAD_REQUEST, e.to_string()),
                AppError::Authorization(_) => (StatusCode::FORBIDDEN, e.to_string()),
                _ => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "An internal error occurred".to_string(),
                ),
            };
            (
                status,
                Json(ApiErrorResponse {
                    error: error_message,
                }),
            )
                .into_response()
        }
    }
}

pub async fn start_http_server(
    config: &HttpConfig,
    http_mounts: Arc<RwLock<HashMap<String, String>>>,
    enclave_client: EnclaveClient,
    work_order_orchestrator: Arc<WorkOrderOrchestrator>,
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
) -> Result<(), anyhow::Error> {
    let worker_base_path = config.base_mount_path.trim_end_matches('/').to_string();
    if !worker_base_path.starts_with('/') && !worker_base_path.is_empty() {
        anyhow::bail!("HTTP base_mount_path must start with '/' or be empty for root");
    }

    let app_state_base_path = if worker_base_path.is_empty() {
        "/".to_string()
    } else {
        worker_base_path.clone()
    };

    let app_state = Arc::new(AppState {
        enclave_client,
        http_mounts,
        base_mount_path: app_state_base_path,
        work_order_orchestrator,
    });

    let mut app = Router::new();

    if config.api_enabled {
        let mut api_router = Router::new()
            .route("/work-orders", post(submit_work_order_handler))
            .with_state(app_state.clone());

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

    let worker_router = Router::new()
        .fallback_service(axum::routing::any(universal_http_handler).with_state(app_state));

    let app = if worker_base_path.is_empty() || worker_base_path == "/" {
        app.fallback_service(worker_router)
    } else {
        app.nest(&worker_base_path, worker_router)
    };

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
