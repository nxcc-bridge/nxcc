pub mod debug;
pub mod enclave_client;
pub mod work_orders;

use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures_util::StreamExt;
use nxcc_interface::proto::daemon::{
    debug_server::DebugServer, work_order_server::WorkOrderServer,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tonic::transport::{Server, server::Connected};
use tracing::info;

use crate::{
    config::GrpcConfig,
    error::AppError,
    grpc::{debug::DebugGrpc, enclave_client::EnclaveClient, work_orders::WorkOrderGrpcService},
    http_server::VmRegistry,
    services::{secrets::SecretsService, work_order_orchestrator::WorkOrderOrchestrator},
};

// Wrapper to implement tonic 0.14 Connected trait for VsockStream
struct VsockStreamWrapper(tokio_vsock::VsockStream);

impl Connected for VsockStreamWrapper {
    type ConnectInfo = ();

    fn connect_info(&self) -> Self::ConnectInfo {
        ()
    }
}

impl AsyncRead for VsockStreamWrapper {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for VsockStreamWrapper {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

pub async fn start_grpc_server(
    config: &GrpcConfig,
    secrets_service: std::sync::Arc<SecretsService>,
    work_order_orchestrator: Arc<WorkOrderOrchestrator>,
    enclave_client: EnclaveClient,
    vm_registry: VmRegistry,
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
            let incoming = listener
                .incoming()
                .map(|stream_result| stream_result.map(|stream| VsockStreamWrapper(stream)));

            Server::builder()
                .add_service(DebugServer::new(DebugGrpc::new(
                    enclave_client.clone(),
                    work_order_orchestrator.clone(),
                    vm_registry.clone(),
                )))
                .add_service(WorkOrderServer::new(WorkOrderGrpcService::new(
                    work_order_orchestrator,
                    enclave_client.clone(),
                )))
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
                let svc = DebugServer::new(DebugGrpc::new(
                    enclave_client.clone(),
                    work_order_orchestrator.clone(),
                    vm_registry.clone(),
                ));
                let wo_svc = WorkOrderServer::new(WorkOrderGrpcService::new(
                    work_order_orchestrator,
                    enclave_client.clone(),
                ));

                Server::builder()
                    .add_service(svc)
                    .add_service(wo_svc)
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
        "tcp" => {
            let addr = config
                .tcp_addr
                .parse()
                .map_err(|e| AppError::Service(format!("Invalid TCP address: {}", e)))?;
            info!("Starting gRPC server on TCP at {}", addr);
            Server::builder()
                .add_service(DebugServer::new(DebugGrpc::new(
                    enclave_client.clone(),
                    work_order_orchestrator.clone(),
                    vm_registry.clone(),
                )))
                .add_service(WorkOrderServer::new(WorkOrderGrpcService::new(
                    work_order_orchestrator,
                    enclave_client.clone(),
                )))
                .serve_with_shutdown(addr, async {
                    let _ = shutdown.recv().await;
                })
                .await
                .map_err(|e| AppError::Service(format!("gRPC server error: {}", e)))?;
        }
        other => {
            return Err(AppError::Service(format!("Invalid gRPC mode: {}", other)));
        }
    }
    Ok(())
}
