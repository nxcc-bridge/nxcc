use std::{
    error::Error,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use tokio::sync::Mutex;
use tonic::{
    Status,
    body::Body,
    codegen::http::{self, Request, Response},
};
use tower::{Layer, Service};
use tracing::{debug, error, info, warn};

/// Bounded client state - stores the DER certificate of the first client that connects
#[derive(Clone)]
pub struct BoundClient {
    inner: Arc<Mutex<Option<Vec<u8>>>>,
}

impl Default for BoundClient {
    fn default() -> Self {
        Self::new()
    }
}

impl BoundClient {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    /// Try to bind this client as the exclusive client. Returns true if binding succeeded.
    pub async fn bind_client(&self, cert_der: Vec<u8>) -> bool {
        let mut state = self.inner.lock().await;
        if state.is_none() {
            debug!("Binding new client with cert of length {}", cert_der.len());
            *state = Some(cert_der);
            true
        } else {
            false
        }
    }

    /// Check if the provided cert matches the bound client's cert
    pub async fn is_bound_client(&self, cert_der: &[u8]) -> bool {
        let state = self.inner.lock().await;
        state
            .as_ref()
            .is_some_and(|bound_cert| bound_cert == cert_der)
    }

    /// Returns true if a client is already bound
    pub async fn has_bound_client(&self) -> bool {
        let state = self.inner.lock().await;
        state.is_some()
    }
}

/// ClientBindingLayer enforces binding to the first client that connects
#[derive(Clone)]
pub struct ClientBindingLayer {
    bound_client: BoundClient,
}

impl ClientBindingLayer {
    pub fn new(bound_client: BoundClient) -> Self {
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
pub struct ClientBindingService<S> {
    inner: S,
    bound_client: BoundClient,
}

impl<S> Service<Request<Body>> for ClientBindingService<S>
where
    S: Service<Request<Body>, Response = Response<Body>> + Send + 'static + Clone,
    S::Future: Send + 'static,
    S::Error: Into<Box<dyn Error + Send + Sync>> + Send,
{
    type Response = S::Response;
    type Error = Box<dyn Error + Send + Sync>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        // Extract the peer certificate DER bytes
        let client_cert_der = extract_client_cert(&req);

        let bound_client = self.bound_client.clone();
        let inner = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, inner);

        Box::pin(async move {
            match client_cert_der {
                Some(cert_bytes) => {
                    // Check if this client is already the bound client
                    if bound_client.is_bound_client(&cert_bytes).await {
                        // This is the bound client, let the request proceed
                        debug!("Request from bound client accepted");
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
                            Ok(create_error_response(http::StatusCode::FORBIDDEN, status))
                        }
                    }
                }
                None => {
                    // No client certificate, reject
                    error!("Rejected request with no client certificate");
                    let status = Status::unauthenticated("Client certificate required");
                    Ok(create_error_response(
                        http::StatusCode::UNAUTHORIZED,
                        status,
                    ))
                }
            }
        })
    }
}

/// Helper function to extract client certificate from the request
fn extract_client_cert(req: &Request<Body>) -> Option<Vec<u8>> {
    req.extensions()
        .get::<tonic::transport::server::TlsConnectInfo<tonic::transport::server::TcpConnectInfo>>()
        .and_then(|tls_info| tls_info.peer_certs())
        .and_then(|certs| certs.first().cloned())
        .map(|cert| cert.as_ref().to_vec())
}

/// Helper function to create an error response with appropriate gRPC status headers
fn create_error_response(http_status: http::StatusCode, status: Status) -> Response<Body> {
    http::Response::builder()
        .status(http_status)
        .header("content-type", "application/grpc")
        .header("grpc-status", status.code().to_string())
        .header("grpc-message", status.message())
        .body(Body::default())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bound_client_binding() {
        let client = BoundClient::new();

        // Initially, no client is bound
        assert!(!(client.has_bound_client().await));

        // First client can bind
        let cert1 = vec![1, 2, 3];
        assert!(client.bind_client(cert1.clone()).await);

        // After binding, has_bound_client should return true
        assert!(client.has_bound_client().await);

        // Second client with different cert cannot bind
        let cert2 = vec![4, 5, 6];
        assert!(!(client.bind_client(cert2.clone()).await));

        // First client is recognized
        assert!(client.is_bound_client(&cert1).await);

        // Second client is not recognized
        assert!(!(client.is_bound_client(&cert2).await));
    }

    // More tests would be added for the layer and service implementations
    // using mock requests and services
}
