use std::sync::Arc;

use nxcc_interface::{
    proto::enclave::{
        CheckSecretsRequest, CheckSecretsResponse, DeliverEventRequest, DeliverEventResponse,
        GetReportRequest, GetSecretsEnclaveRequest, GetSecretsResponse, PutSecretsRequest,
        PutSecretsResponse, RunWorkerRequest, RunWorkerResponse, SecretStatus,
        enclave_secrets_server::{EnclaveSecrets, EnclaveSecretsServer},
        runner_server::{Runner, RunnerServer},
    },
    types::{EnvReport, FromProto, IntoProto, SecretId, SecretsBox},
};
use tonic::{Request, Response, Status, transport::Server};
use tracing::{debug, error, info};

use crate::{config::EnclaveConfig, runner::RunnerService, secrets::Secrets};

// --- Secrets Service Implementation ---

pub struct EnclaveSecretsService {
    secrets: Arc<Secrets>,
}

impl EnclaveSecretsService {
    pub fn new(secrets: Arc<Secrets>) -> Self {
        Self { secrets }
    }
}

#[tonic::async_trait]
impl EnclaveSecrets for EnclaveSecretsService {
    async fn get_report(
        &self,
        request: Request<GetReportRequest>,
    ) -> Result<Response<nxcc_interface::proto::interface::AttestationReport>, Status> {
        let user_data = request.into_inner().user_data;
        debug!(
            "gRPC GetReport request with user_data size {}",
            user_data.len()
        );
        match self.secrets.get_report(user_data) {
            Ok(report) => Ok(Response::new(report.to_proto())),
            Err(e) => {
                error!("GetReport failed: {}", e);
                Err(Status::internal(format!("Failed to get report: {e}")))
            }
        }
    }

    async fn put_secrets(
        &self,
        request: Request<PutSecretsRequest>,
    ) -> Result<Response<PutSecretsResponse>, Status> {
        let proto_req = request.into_inner();
        debug!(
            "gRPC PutSecrets request with {} bundles",
            proto_req.secrets_bundles.len()
        );
        let mut bundles = Vec::new();
        for bundle_proto in proto_req.secrets_bundles {
            let secrets_box = bundle_proto
                .secrets_box
                .map(SecretsBox::from_proto)
                .ok_or_else(|| Status::invalid_argument("Missing SecretsBox in bundle"))?;
            let env_report = bundle_proto
                .env_report
                .map(EnvReport::from_proto)
                .ok_or_else(|| Status::invalid_argument("Missing EnvReport in bundle"))?;
            bundles.push((secrets_box, env_report));
        }

        match self.secrets.put_secrets(bundles) {
            Ok(success) => Ok(Response::new(PutSecretsResponse { success })),
            Err(e) => {
                error!("PutSecrets failed: {}", e);
                Err(Status::internal(format!("Failed to put secrets: {e}")))
            }
        }
    }

    async fn get_secrets(
        &self,
        request: Request<GetSecretsEnclaveRequest>,
    ) -> Result<Response<GetSecretsResponse>, Status> {
        let proto_req = request.into_inner();
        debug!(
            "gRPC GetSecrets request for {} secret IDs",
            proto_req.requests.len()
        );

        let secret_ids: Vec<SecretId> = proto_req
            .requests
            .into_iter()
            .filter_map(|r| r.id.map(SecretId::from_proto))
            .collect();

        let requester_env_report = proto_req
            .requester_env_report // This field was missing in the proto def! Added it.
            .map(EnvReport::from_proto)
            .ok_or_else(|| Status::invalid_argument("Missing requester_env_report"))?;

        // Policy reports are currently ignored per instructions, local auth store is checked
        let policy_reports = Vec::new(); // Placeholder

        match self
            .secrets
            .get_secrets(secret_ids, requester_env_report, policy_reports)
        {
            Ok(secrets_box) => Ok(Response::new(GetSecretsResponse {
                secrets_box: Some(secrets_box.to_proto()),
            })),
            Err(e) => {
                error!("GetSecrets failed: {}", e);
                Err(Status::internal(format!("Failed to get secrets: {e}")))
            }
        }
    }

    async fn check_secrets(
        &self,
        request: Request<CheckSecretsRequest>,
    ) -> Result<Response<CheckSecretsResponse>, Status> {
        let proto_req = request.into_inner();
        debug!("gRPC CheckSecrets request for {} IDs", proto_req.ids.len());
        let ids: Vec<SecretId> = proto_req
            .ids
            .into_iter()
            .map(SecretId::from_proto)
            .collect();

        match self.secrets.check_secrets(ids) {
            Ok(statuses) => {
                let proto_statuses = statuses
                    .into_iter()
                    .map(|(id, found, expiry)| SecretStatus {
                        id: Some(id.to_proto()),
                        found,
                        expiry,
                    })
                    .collect();
                Ok(Response::new(CheckSecretsResponse {
                    statuses: proto_statuses,
                }))
            }
            Err(e) => {
                error!("CheckSecrets failed: {}", e);
                Err(Status::internal(format!("Failed to check secrets: {e}")))
            }
        }
    }
}

// --- Runner Service Implementation ---

pub struct EnclaveRunnerService {
    runner: Arc<RunnerService>,
}

impl EnclaveRunnerService {
    pub fn new(runner: Arc<RunnerService>) -> Self {
        Self { runner }
    }
}

#[tonic::async_trait]
impl Runner for EnclaveRunnerService {
    async fn run_worker(
        &self,
        request: Request<RunWorkerRequest>,
    ) -> Result<Response<RunWorkerResponse>, Status> {
        let req = request.into_inner();
        debug!(
            "gRPC RunWorker request with binary size {}",
            req.worker_binary.len()
        );
        match self.runner.run_worker(req.worker_binary).await {
            Ok(_) => Ok(Response::new(RunWorkerResponse { accepted: true })),
            Err(e) => {
                error!("RunWorker failed: {}", e);
                Err(Status::internal(format!("Failed to run worker: {e}")))
            }
        }
    }

    async fn deliver_event(
        &self,
        request: Request<DeliverEventRequest>,
    ) -> Result<Response<DeliverEventResponse>, Status> {
        let req = request.into_inner();
        debug!(
            "gRPC DeliverEvent request for worker '{}', payload size {}",
            req.worker_id,
            req.event_payload.len()
        );
        match self
            .runner
            .deliver_event(req.worker_id, req.event_payload)
            .await
        {
            Ok(_) => Ok(Response::new(DeliverEventResponse { delivered: true })),
            Err(e) => {
                error!("DeliverEvent failed: {}", e);
                Err(Status::internal(format!("Failed to deliver event: {e}")))
            }
        }
    }
}

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
