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
    let secrets = Secrets::new();
    let runner_svc = RunnerService::new(secrets.clone());

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
                .add_service(EnclaveSecretsServer::new(EnclaveSecretsService::new(
                    secrets,
                )))
                .add_service(RunnerServer::new(EnclaveRunnerService::new(runner_svc)))
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
                    .add_service(EnclaveSecretsServer::new(EnclaveSecretsService::new(
                        secrets,
                    )))
                    .add_service(RunnerServer::new(EnclaveRunnerService::new(runner_svc)))
                    .serve_with_incoming(incoming)
                    .await?;
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
