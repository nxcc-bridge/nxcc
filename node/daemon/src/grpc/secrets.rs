use std::{collections::HashMap, sync::Arc};

use interface::{
    proto::daemon::{GetSecretsRequest, GetSecretsResponse, secrets_server::Secrets},
    types::{EnvReport, SecretId, SecretRequest, SecretsBox},
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

        let env_proto = req
            .env_report
            .ok_or_else(|| Status::invalid_argument("Missing EnvReport"))?;
        // Keep the original EnvReport for the final enclave call
        let original_env_report = EnvReport::from_proto(env_proto);

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

        // Call the internal service method. This ensures secrets are fetched/stored.
        // It returns Ok(()) on success, Err otherwise.
        match self
            .secrets_service
            .clone()
            .get_secrets(grouped_requests, original_env_report.clone())
            .await
        {
            Ok(()) => {
                info!(
                    "Internal get_secrets succeeded. Fetching final SecretsBox from enclave for \
                     {} secrets.",
                    all_secret_ids.len()
                );
                // If successful, call the enclave's get_secrets to get the actual box
                // for the original caller, passing the original EnvReport.
                match self
                    .enclave_client
                    .get_secrets(all_secret_ids, original_env_report)
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
}
