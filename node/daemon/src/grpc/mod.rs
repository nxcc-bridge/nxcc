pub mod enclave_client;
pub mod secrets;

use interface::proto::daemon::secrets_server::SecretsServer;
use tonic::transport::Server;
use tracing::info;

use crate::{
    config::GrpcConfig, error::AppError, grpc::secrets::SecretsDebugGrpc,
    services::secrets::SecretsService,
};

pub async fn start_grpc_server(
    config: &GrpcConfig,
    secrets_service: std::sync::Arc<SecretsService>,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) -> Result<(), AppError> {
    match config.mode.as_str() {
        "vsock" => {
            info!(
                "Starting gRPC server on vsock at CID {} port {}",
                config.vsock_cid, config.vsock_port
            );
            let listener = tokio_vsock::VsockListener::bind(tokio_vsock::VsockAddr::new(
                config.vsock_cid,
                config.vsock_port,
            ))
            .map_err(|e| AppError::Service(format!("Failed to bind vsock: {}", e)))?;
            let incoming = listener.incoming();

            Server::builder()
                .add_service(SecretsServer::new(SecretsDebugGrpc::new(secrets_service)))
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = shutdown.recv().await;
                })
                .await
                .map_err(|e| AppError::Service(format!("gRPC server error: {}", e)))?;
        }
        "uds" => {
            info!("Starting gRPC server on UDS at path {}", config.uds_path);
            #[cfg(unix)]
            {
                use std::{io::ErrorKind, path::Path};

                use tokio::net::{UnixListener, UnixStream};
                use tokio_stream::wrappers::UnixListenerStream;

                let uds_path = &config.uds_path;
                if Path::new(uds_path).exists() {
                    match UnixStream::connect(uds_path).await {
                        Ok(_) => {
                            return Err(AppError::Service(format!(
                                "UDS at {} is already in use by a running server.",
                                uds_path
                            )));
                        }
                        Err(e) => {
                            if e.kind() == ErrorKind::ConnectionRefused {
                                std::fs::remove_file(uds_path).map_err(|e| {
                                    AppError::Service(format!(
                                        "Failed to remove stale UDS file {}: {}",
                                        uds_path, e
                                    ))
                                })?;
                            } else {
                                return Err(AppError::Service(format!(
                                    "Error checking existing UDS at {}: {}",
                                    uds_path, e
                                )));
                            }
                        }
                    }
                }

                let uds_listener = UnixListener::bind(uds_path)
                    .map_err(|e| AppError::Service(format!("Failed to bind UDS: {}", e)))?;
                let incoming = UnixListenerStream::new(uds_listener);
                let svc = SecretsServer::new(SecretsDebugGrpc::new(secrets_service));

                Server::builder()
                    .add_service(svc)
                    .serve_with_incoming_shutdown(incoming, async {
                        let _ = shutdown.recv().await;
                    })
                    .await
                    .map_err(|e| AppError::Service(format!("gRPC server error: {}", e)))?;

                if let Err(e) = std::fs::remove_file(uds_path) {
                    tracing::warn!("Failed to remove UDS file on shutdown: {}", e);
                }
            }
            #[cfg(not(unix))]
            {
                return Err(AppError::Service(
                    "UDS not supported on this platform".into(),
                ));
            }
        }
        other => {
            return Err(AppError::Service(format!("Invalid gRPC mode: {}", other)));
        }
    }
    Ok(())
}
