use std::sync::Arc;

use nxcc_interface::{
    proto::{
        enclave::{
            AttachVmRequest, AttachVmResponse, CheckSecretsRequest, CheckSecretsResponse,
            DeliverBatchEventsRequest, DeliverBatchEventsResponse, DetachVmRequest,
            ExecutePolicyRequest, ExecutePolicyResponse, GenerateSecretsRequest, GetReportRequest,
            GetSecretsRequest, GetSecretsResponse, InvokeHttpWorkerRequest,
            InvokeHttpWorkerResponse, PutSecretsRequest, PutSecretsResponse, RunWorkerRequest,
            RunWorkerResponse, SecretStatus, TerminateWorkerRequest,
            runner_server::{Runner, RunnerServer},
            secrets_server::{Secrets as SecretsServerTrait, SecretsServer},
        },
        interface,
    },
    types::{
        ConsumerInfo, EnvReport, EventPayload, PolicyExecutionRequest, SecretId, SecretRequest,
        SecretsBox, VmAddress, WorkerBundle, WorkerManifest,
    },
};
use nxcc_vm_base::client::ClientError;
use tonic::{Request, Response, Status, transport::Server};
use tracing::{debug, error, info};

use crate::{
    config::EnclaveConfig,
    runner::{RunnerError, RunnerService},
    secrets::Secrets,
};

// --- Secrets Service Implementation ---

pub struct SecretsGrpcService {
    secrets: Arc<Secrets>,
}

impl SecretsGrpcService {
    pub fn new(secrets: Arc<Secrets>) -> Self {
        Self { secrets }
    }
}

#[tonic::async_trait]
impl SecretsServerTrait for SecretsGrpcService {
    async fn get_report(
        &self,
        request: Request<GetReportRequest>,
    ) -> Result<Response<interface::AttestationReport>, Status> {
        let user_data = request.into_inner().user_data;
        debug!(
            "gRPC GetReport request with user_data size {}",
            user_data.len()
        );
        match self.secrets.get_report(user_data) {
            Ok(report) => Ok(Response::new(report.into())),
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
                .map(SecretsBox::from)
                .ok_or_else(|| Status::invalid_argument("Missing SecretsBox in bundle"))?;
            let env_report = bundle_proto
                .env_report
                .map(EnvReport::from)
                .ok_or_else(|| Status::invalid_argument("Missing EnvReport in bundle"))?;
            let consumer_info = bundle_proto
                .consumer_info
                .map(ConsumerInfo::from)
                .ok_or_else(|| Status::invalid_argument("Missing ConsumerInfo in bundle"))?;
            bundles.push((secrets_box, env_report, consumer_info));
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
        request: Request<GetSecretsRequest>,
    ) -> Result<Response<GetSecretsResponse>, Status> {
        let proto_req = request.into_inner();
        // proto_req.requests is Vec<nxcc_interface::proto::interface::SecretRequest>
        debug!(
            "gRPC GetSecrets request for {} secret requests",
            proto_req.requests.len()
        );

        let internal_requests: Vec<(SecretId, ConsumerInfo)> = proto_req
            .requests
            .into_iter()
            .map(SecretRequest::from) // Convert proto to internal SecretRequest
            .map(|sr| (sr.secret_id, sr.consumer)) // Extract parts
            .collect();

        let requester_env_report = proto_req
            .requester_env_report
            .map(EnvReport::from)
            .ok_or_else(|| Status::invalid_argument("Missing requester_env_report"))?;

        match self
            .secrets
            .get_secrets(internal_requests, requester_env_report)
        {
            Ok(secrets_box) => Ok(Response::new(GetSecretsResponse {
                secrets_box: Some(secrets_box.into()),
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
        let ids: Vec<SecretId> = proto_req.ids.into_iter().map(SecretId::from).collect();

        match self.secrets.check_secrets(ids) {
            Ok(statuses) => {
                let proto_statuses = statuses
                    .into_iter()
                    .map(|(id, found, expiry)| SecretStatus {
                        id: Some(id.into()),
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

    async fn generate_secrets(
        &self,
        request: Request<GenerateSecretsRequest>,
    ) -> Result<Response<()>, Status> {
        let proto_req = request.into_inner();
        // proto_req.requests is Vec<nxcc_interface::proto::interface::SecretRequest>
        debug!(
            "gRPC GenerateSecrets request for {} ID-Consumer pairs",
            proto_req.requests.len()
        );
        let internal_requests: Vec<(SecretId, ConsumerInfo)> = proto_req
            .requests
            .into_iter()
            .map(SecretRequest::from)
            .map(|sr| (sr.secret_id, sr.consumer))
            .collect();
        match self.secrets.generate_secrets(internal_requests) {
            Ok(()) => Ok(Response::new(())),
            Err(e) => {
                error!("GenerateSecrets failed: {}", e);
                // Map specific errors (like duplicate) to appropriate gRPC status codes
                Err(Status::already_exists(format!(
                    "Failed to generate secrets: {}",
                    e
                )))
            }
        }
    }
}

// --- Runner Service Implementation ---

pub struct EnclaveRunnerGrpcService {
    runner: Arc<RunnerService>,
}

impl EnclaveRunnerGrpcService {
    pub fn new(runner: Arc<RunnerService>) -> Self {
        Self { runner }
    }
}

fn map_runner_error(err: RunnerError) -> Status {
    error!("Runner service error: {}", err);
    match err {
        RunnerError::VmNotAttached(id) => {
            Status::failed_precondition(format!("VM not attached: {id}"))
        }
        RunnerError::WorkerNotFound(id) => Status::not_found(format!("Worker not found: {id}")),
        RunnerError::VmConnection(client_err) => {
            Status::internal(format!("VM connection error: {client_err}"))
        }
        RunnerError::Deserialization(s) => Status::internal(format!("Data format error: {s}")),
        RunnerError::PolicyExecutionFailed(s) => {
            Status::internal(format!("Policy execution failed: {s}"))
        }
        RunnerError::Internal(s) => Status::internal(s),
        RunnerError::TlsConfig(tls_err) => Status::internal(format!("TLS config error: {tls_err}")),
        RunnerError::WorkerStartFailed(s) => {
            Status::internal(format!("Failed to start worker: {s}"))
        }
        RunnerError::UnsupportedVmAddress(_) => {
            Status::unimplemented("VM address type not supported by this enclave build")
        }
        RunnerError::EventSendError(s) => Status::internal(format!("Event send error: {s}")),
    }
}

#[tonic::async_trait]
impl Runner for EnclaveRunnerGrpcService {
    async fn attach_vm(
        &self,
        request: Request<AttachVmRequest>,
    ) -> Result<Response<AttachVmResponse>, Status> {
        let req = request.into_inner();
        debug!("gRPC AttachVm request for vm_id '{}'", req.vm_id);
        let address = req
            .address
            .map(VmAddress::from)
            .ok_or_else(|| Status::invalid_argument("Missing VM address"))?;

        match self.runner.attach_vm(req.vm_id, address).await {
            Ok(attached) => Ok(Response::new(AttachVmResponse { attached })),
            Err(e) => Err(map_runner_error(e)),
        }
    }

    async fn detach_vm(&self, request: Request<DetachVmRequest>) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        debug!("gRPC DetachVm request for vm_id '{}'", req.vm_id);
        match self.runner.detach_vm(req.vm_id).await {
            Ok(()) => Ok(Response::new(())),
            Err(e) => Err(map_runner_error(e)), // Although detach doesn't strictly need to comply, report internal errors
        }
    }

    async fn run_worker(
        &self,
        request: Request<RunWorkerRequest>,
    ) -> Result<Response<RunWorkerResponse>, Status> {
        let req = request.into_inner();
        debug!(
            "gRPC RunWorker request for vm_id '{}', code size {}, manifest size {}",
            req.vm_id,
            req.worker_manifest_bytes.len(),
            req.worker_bundle_bytes.len()
        );

        let worker_manifest: WorkerManifest = serde_json::from_slice(&req.worker_manifest_bytes)
            .map_err(|e| {
                Status::invalid_argument(format!("Failed to deserialize WorkerManifest: {}", e))
            })?;

        let worker_bundle = WorkerBundle(req.worker_bundle_bytes);

        match self
            .runner
            .run_worker(req.vm_id, worker_manifest, worker_bundle)
            .await
        {
            Ok(worker_id) => Ok(Response::new(RunWorkerResponse {
                success: true,
                worker_id,
                error_message: String::new(),
            })),
            Err(RunnerError::WorkerStartFailed(msg)) => {
                // Specific handling for start failure reported by VM
                Ok(Response::new(RunWorkerResponse {
                    success: false,
                    worker_id: String::new(),
                    error_message: msg,
                }))
            }
            Err(e) => Err(map_runner_error(e)),
        }
    }

    async fn terminate_worker(
        &self,
        request: Request<TerminateWorkerRequest>,
    ) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        debug!(
            "gRPC TerminateWorker request for worker_id '{}'",
            req.worker_id
        );
        match self.runner.terminate_worker(req.worker_id).await {
            Ok(()) => Ok(Response::new(())),
            Err(RunnerError::WorkerNotFound(_)) => Ok(Response::new(())),
            Err(e) => Err(map_runner_error(e)),
        }
    }

    async fn execute_policy(
        &self,
        request: Request<ExecutePolicyRequest>,
    ) -> Result<Response<ExecutePolicyResponse>, Status> {
        let req = request.into_inner();
        debug!(
            "gRPC ExecutePolicy request for worker '{}', {} contexts",
            req.worker_id,
            req.contexts.len()
        );

        let internal_contexts: Vec<PolicyExecutionRequest> = req
            .contexts
            .into_iter()
            .map(PolicyExecutionRequest::from)
            .collect();

        match self
            .runner
            .execute_policy(req.worker_id, internal_contexts)
            .await
        {
            Ok(satisfied_internal_contexts) => {
                let satisfied_proto_contexts = satisfied_internal_contexts
                    .into_iter()
                    .map(Into::into)
                    .collect();
                Ok(Response::new(ExecutePolicyResponse {
                    satisfied_contexts: satisfied_proto_contexts,
                }))
            }
            Err(e) => Err(map_runner_error(e)),
        }
    }

    async fn deliver_batch_events(
        &self,
        request: Request<DeliverBatchEventsRequest>,
    ) -> Result<Response<DeliverBatchEventsResponse>, Status> {
        let req_inner = request.into_inner();
        debug!(
            "gRPC DeliverBatchEvents request with {} events",
            req_inner.events.len()
        );

        let mut internal_events = Vec::new();
        for proto_event_delivery in req_inner.events {
            let worker_id = proto_event_delivery.worker_id;
            let event_payload_proto = proto_event_delivery
                .event_payload
                .ok_or_else(|| Status::invalid_argument("EventDelivery missing event_payload"))?;
            let rust_event_payload = EventPayload::from(event_payload_proto);
            let handler_name = proto_event_delivery.handler_name;
            internal_events.push((worker_id, handler_name, rust_event_payload));
        }

        match self.runner.deliver_batch_events(internal_events).await {
            Ok(()) => Ok(Response::new(DeliverBatchEventsResponse {
                success: true,
                message: "Batch delivered".to_string(),
            })),
            Err(e) => Err(map_runner_error(e)),
        }
    }

    async fn invoke_http_worker(
        &self,
        request: Request<InvokeHttpWorkerRequest>,
    ) -> Result<Response<InvokeHttpWorkerResponse>, Status> {
        let req_inner = request.into_inner();
        debug!(
            "gRPC InvokeHttpWorker request for worker_id '{}', uri '{}'",
            req_inner.worker_id,
            req_inner.request.as_ref().map_or("N/A", |r| r.uri.as_str())
        );

        let http_request_proto = req_inner.request.ok_or_else(|| {
            Status::invalid_argument("Missing HttpRequest in InvokeHttpWorkerRequest")
        })?;

        match self
            .runner
            .invoke_http_worker(req_inner.worker_id, http_request_proto)
            .await
        {
            Ok(http_response_proto) => Ok(Response::new(InvokeHttpWorkerResponse {
                response: Some(http_response_proto),
            })),
            Err(e) => Err(map_runner_error(e)),
        }
    }
}

// --- Server Setup ---

pub async fn start_grpc_server(config: &EnclaveConfig) -> Result<(), Box<dyn std::error::Error>> {
    // Instantiate shared services
    let secrets_service = Secrets::new();
    let runner_service = Arc::new(RunnerService::new(secrets_service.clone()));

    // Instantiate gRPC service wrappers
    let secrets_grpc = SecretsGrpcService::new(secrets_service);
    let runner_grpc = EnclaveRunnerGrpcService::new(runner_service);

    let builder = Server::builder()
        .add_service(SecretsServer::new(secrets_grpc))
        .add_service(RunnerServer::new(runner_grpc));

    match config.grpc.mode.as_str() {
        "vsock" => {
            info!(
                "Enclave gRPC listening on vsock: CID={}, port={}",
                config.grpc.vsock_cid, config.grpc.vsock_port
            );
            #[cfg(feature = "vsock")]
            {
                use tokio::sync::oneshot;
                use tokio_vsock::VsockListener;

                let (shutdown_tx, shutdown_rx) = oneshot::channel();

                let ctrl_c_task = tokio::spawn(async move {
                    tokio::signal::ctrl_c()
                        .await
                        .expect("Failed to listen for ctrl-c");
                    let _ = shutdown_tx.send(());
                });

                let listener = VsockListener::bind(tokio_vsock::VsockAddr::new(
                    config.grpc.vsock_cid,
                    config.grpc.vsock_port,
                ))?;

                builder
                    .serve_with_incoming_shutdown(listener.incoming(), async {
                        let _ = shutdown_rx.await;
                    })
                    .await?;

                ctrl_c_task.abort();
            }
            #[cfg(not(feature = "vsock"))]
            {
                return Err("VSOCK feature is not enabled in this build.".into());
            }
        }
        "uds" => {
            info!("Enclave gRPC listening on UDS: {}", config.grpc.uds_path);
            #[cfg(all(unix, feature = "uds"))]
            {
                use std::path::Path;

                use tokio::{net::UnixListener, sync::oneshot};
                use tokio_stream::wrappers::UnixListenerStream;

                let path = Path::new(&config.grpc.uds_path);
                // Clean up existing socket file unconditionally for simplicity in dev
                if path.exists() {
                    debug!("Removing existing UDS file: {}", config.grpc.uds_path);
                    std::fs::remove_file(path)?;
                }

                let uds_listener = UnixListener::bind(path)?;
                let incoming = UnixListenerStream::new(uds_listener);

                let (shutdown_tx, shutdown_rx) = oneshot::channel();
                let uds_path_clone = path.to_path_buf();

                let ctrl_c_task = tokio::spawn(async move {
                    tokio::signal::ctrl_c()
                        .await
                        .expect("Failed to listen for ctrl-c");
                    let _ = std::fs::remove_file(&uds_path_clone);
                    let _ = shutdown_tx.send(());
                });

                builder
                    .serve_with_incoming_shutdown(incoming, async {
                        let _ = shutdown_rx.await;
                    })
                    .await?;

                ctrl_c_task.abort();

                let _ = std::fs::remove_file(path);
            }
            #[cfg(not(all(unix, feature = "uds")))]
            {
                return Err("UDS feature is not enabled or platform is not Unix.".into());
            }
        }
        other => {
            return Err(format!("Invalid enclave gRPC mode: {}", other).into());
        }
    }

    Ok(())
}
