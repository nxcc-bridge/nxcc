use std::sync::Arc;
use tonic::transport::Server;
use tracing::info;

use crate::config::EnclaveConfig;
use crate::services::{
    grpc::{EnclaveRunnerService, EnclaveSecretsService},
    runner::RunnerService,
    secrets::Secrets,
};

use interface::proto::enclave::{
    enclave_secrets_server::EnclaveSecretsServer, runner_server::RunnerServer,
};

pub async fn start_grpc_server(config: &EnclaveConfig) -> Result<(), Box<dyn std::error::Error>> {
    // Instantiate shared services
    let secrets_service = Secrets::new(); // Arc<Secrets>
    let runner_service = Arc::new(RunnerService::new(secrets_service.clone())); // Arc<RunnerService>

    // Instantiate gRPC service wrappers
    let secrets_grpc = EnclaveSecretsService::new(secrets_service); // Takes Arc<Secrets>
    let runner_grpc = EnclaveRunnerService::new(runner_service); // Takes Arc<RunnerService>

    match config.mode {
        "vsock" => {
            info!(
                "Enclave gRPC listening on vsock: CID={}, port={}",
                config.vsock_cid, config.vsock_port
            );
            let listener = tokio_vsock::VsockListener::bind(tokio_vsock::VsockAddr::new(
                config.vsock_cid,
                config.vsock_port,
            ))?;

            Server::builder()
                .add_service(EnclaveSecretsServer::new(secrets_grpc))
                .add_service(RunnerServer::new(runner_grpc))
                .serve_with_incoming(listener.incoming())
                .await?;
        }
        "uds" => {
            info!("Enclave gRPC listening on UDS: {}", config.uds_path);
            #[cfg(unix)]
            {
                use std::{io::ErrorKind, path::Path};
                use tokio::net::{UnixListener, UnixStream};
                use tokio_stream::wrappers::UnixListenerStream;

                let path = Path::new(config.uds_path);
                if path.exists() {
                    // Attempt to connect to check if it's active
                    match UnixStream::connect(path).await {
                        Ok(_) => {
                            return Err(format!(
                                "Enclave UDS {} already in use by a running server.",
                                config.uds_path
                            )
                            .into());
                        }
                        Err(e) if e.kind() == ErrorKind::ConnectionRefused => {
                            // Socket exists but server isn't running, remove it
                            info!("Removing stale UDS file: {}", config.uds_path);
                            std::fs::remove_file(path)?;
                        }
                        Err(e) => {
                            return Err(format!(
                                "Error checking existing UDS {}: {}",
                                config.uds_path, e
                            )
                            .into());
                        }
                    }
                }

                let uds_listener = UnixListener::bind(path)?;
                let incoming = UnixListenerStream::new(uds_listener);

                Server::builder()
                    .add_service(EnclaveSecretsServer::new(secrets_grpc))
                    .add_service(RunnerServer::new(runner_grpc))
                    .serve_with_incoming(incoming)
                    .await?;

                // Clean up UDS file on shutdown (best effort)
                let _ = std::fs::remove_file(path);
            }
            #[cfg(not(unix))]
            {
                unimplemented!("UDS not supported on this platform");
            }
        }
        other => {
            return Err(format!("Invalid enclave gRPC mode: {}", other).into());
        }
    }
    Ok(())
}
