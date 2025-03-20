use tonic::transport::Server;
use tracing::info;

use crate::config::EnclaveConfig;
use crate::services::{runner::RunnerService, secrets::SecretsService};
use interface::proto::enclave::{runner_server::RunnerServer, secrets_server::SecretsServer};

pub async fn start_grpc_server(config: &EnclaveConfig) -> Result<(), Box<dyn std::error::Error>> {
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
                .add_service(SecretsServer::new(SecretsService::default()))
                .add_service(RunnerServer::new(RunnerService::default()))
                .serve_with_incoming(listener.incoming())
                .await?;
        }
        "uds" => {
            info!("Enclave gRPC listening on UDS: {}", config.uds_path);
            #[cfg(unix)]
            {
                use std::path::Path;
                use tokio::net::UnixListener;
                use tokio_stream::wrappers::UnixListenerStream;

                let path = Path::new(config.uds_path);
                if path.exists() {
                    let _ = std::fs::remove_file(path);
                }

                let uds_listener = UnixListener::bind(path)?;
                let incoming = UnixListenerStream::new(uds_listener);

                Server::builder()
                    .add_service(SecretsServer::new(SecretsService::default()))
                    .add_service(RunnerServer::new(RunnerService::default()))
                    .serve_with_incoming(incoming)
                    .await?;
            }
            #[cfg(not(unix))]
            {
                unimplemented!("UDS is not supported on non-UNIX platforms");
            }
        }
        other => {
            return Err(format!("Invalid enclave gRPC mode: {}", other).into());
        }
    }

    Ok(())
}
