pub mod secrets;

use tracing::info;

use crate::{config::GrpcConfig, error::AppError};

pub async fn start_grpc_server(config: &GrpcConfig) -> Result<(), AppError> {
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
            let svc = secrets::proto::secrets_server::SecretsServer::new(
                secrets::SecretsService::default(),
            );
            tonic::transport::Server::builder()
                .add_service(svc)
                .serve_with_incoming(incoming)
                .await
                .map_err(|e| AppError::Service(format!("gRPC server error: {}", e)))?;
        }
        "uds" => {
            info!("Starting gRPC server on UDS at path {}", config.uds_path);
            #[cfg(unix)]
            {
                use std::{io::ErrorKind, path::Path};

                use tokio::net::{UnixListener, UnixStream};

                let uds_path = &config.uds_path;
                if Path::new(uds_path).exists() {
                    // Try to connect to see if a server is running.
                    match UnixStream::connect(uds_path).await {
                        Ok(_) => {
                            // Connection succeeded, so another server is using the socket.
                            return Err(AppError::Service(format!(
                                "UDS at {} is already in use by a running server.",
                                uds_path
                            )));
                        }
                        Err(e) => {
                            // If the error indicates no server is listening, assume it's stale.
                            if e.kind() == ErrorKind::ConnectionRefused {
                                std::fs::remove_file(uds_path).map_err(|e| {
                                    AppError::Service(format!(
                                        "Failed to remove stale UDS file {}: {}",
                                        uds_path, e
                                    ))
                                })?;
                            } else {
                                // For any other error, propagate it.
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
                let incoming = tokio_stream::wrappers::UnixListenerStream::new(uds_listener);
                let svc = secrets::proto::secrets_server::SecretsServer::new(
                    secrets::SecretsService::default(),
                );

                tonic::transport::Server::builder()
                    .add_service(svc)
                    .serve_with_incoming_shutdown(incoming, async {
                        tokio::signal::ctrl_c()
                            .await
                            .expect("Failed to listen for shutdown signal");
                    })
                    .await
                    .map_err(|e| AppError::Service(format!("gRPC server error: {}", e)))?;

                // Cleanup: remove the UDS file after the server shuts down.
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
