use std::{collections::HashMap, sync::Arc};

use nxcc_interface::{
    proto::daemon::{
        AttachVmRequest, AttachVmResponse, GetSecretsRequest, GetSecretsResponse,
        secrets_server::Secrets,
    },
    types::{
        AttestationReport, EnvReport, FromProto as _, IntoProto as _, SecretId, SecretRequest,
        SecretsBox,
    },
};
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};

use crate::{grpc::enclave_client::EnclaveClient, services::secrets::SecretsService};

pub struct SecretsDebugGrpc {
    secrets_service: Arc<SecretsService>,
    enclave_client: EnclaveClient,
}

impl SecretsDebugGrpc {
    pub fn new(secrets_service: Arc<SecretsService>, enclave_client: EnclaveClient) -> Self {
        Self {
            secrets_service,
            enclave_client,
        }
    }
}

#[tonic::async_trait]
impl Secrets for SecretsDebugGrpc {
    async fn get_secrets(
        &self,
        request: Request<GetSecretsRequest>,
    ) -> Result<Response<GetSecretsResponse>, Status> {
        let req = request.into_inner();
        info!(
            "Received gRPC get_secrets request with {} secret requests",
            req.secret_requests.len()
        );

        let mut grouped_requests = HashMap::new();
        let mut all_secret_ids = Vec::new(); // Collect all requested IDs for the final fetch
        for proto_req in req.secret_requests {
            let sr = SecretRequest::from_proto(proto_req);
            all_secret_ids.push(sr.secret_id.clone());
            grouped_requests
                .entry(sr.secret_id.clone())
                .or_insert_with(Vec::new)
                .push(sr);
        }
        all_secret_ids.dedup(); // Ensure unique IDs

        // --- Generate Daemon's Own EnvReport ---
        let user_data_hash = Default::default();

        let attestation = self
            .enclave_client
            .get_report(user_data_hash)
            .await
            .map_err(|e| {
                Status::internal(format!("Failed to get own attestation report: {}", e))
            })?;

        // Construct the daemon's own EnvReport
        let operator_signature = vec![0u8; 64]; // Placeholder
        let env_report = EnvReport {
            attestation,
            operator_signature,
            node_id: "@self".into(), // Use placeholder consistent with SecretsService::get_own_env_report
        };
        // --- End EnvReport Generation ---

        // Call the internal service method. This ensures secrets are fetched/stored.
        // It returns Ok(()) on success, Err otherwise.
        match self
            .secrets_service
            .clone()
            .get_secrets(grouped_requests, env_report.clone())
            .await
        {
            Ok(()) => {
                info!(
                    "Internal get_secrets succeeded. Fetching final SecretsBox from enclave for \
                     {} secrets.",
                    all_secret_ids.len()
                );
                match self
                    .enclave_client
                    .get_secrets(all_secret_ids, env_report)
                    .await
                {
                    Ok(secrets_box) => {
                        info!("Successfully retrieved final SecretsBox from enclave.");
                        let resp = GetSecretsResponse {
                            secrets_box: Some(secrets_box.to_proto()),
                        };
                        Ok(Response::new(resp))
                    }
                    Err(e) => {
                        error!(
                            "Internal get_secrets succeeded, but final enclave get_secrets \
                             failed: {}",
                            e
                        );
                        Err(Status::internal(format!(
                            "Failed to retrieve secrets from enclave after fetch: {}",
                            e
                        )))
                    }
                }
            }
            Err(e) => {
                error!("Internal get_secrets failed: {:?}", e);
                Err(Status::internal(format!(
                    "SecretsService failed to get secrets: {:?}",
                    e
                )))
            }
        }
    }

    async fn attach_vm(
        &self,
        request: Request<AttachVmRequest>,
    ) -> Result<Response<AttachVmResponse>, Status> {
        let req = request.into_inner();

        let vm_id = if req.vm_id.is_empty() {
            req.uds_path.clone()
        } else {
            req.vm_id
        };

        let uds_path = req.uds_path;

        tracing::info!("AttachVm debug request: vm_id='{vm_id}', uds_path='{uds_path}'");

        match self.enclave_client.attach_vm(vm_id, uds_path).await {
            Ok(attached) => Ok(Response::new(AttachVmResponse { success: true })),
            Err(e) => {
                tracing::error!("AttachVm failed: {e}");
                Err(Status::internal(e))
            }
        }
    }
}
