use std::{collections::HashMap, sync::Arc};

use ethers::types::{Address, U256};
use interface::{
    proto::daemon::{
        GetSecretsRequest, GetSecretsResponse,
        secrets_server::Secrets,
        SecretRequests,
    },
    types::{
        SecretId, SecretRequest, SecretRequesterInfo, SecretsBox,
    },
};
use tonic::{Request, Response, Status};
use tracing::debug;

use crate::services::secrets::SecretsService;

pub struct SecretsDebugGrpc {
    secrets_service: Arc<SecretsService>,
}

impl SecretsDebugGrpc {
    pub fn new(secrets_service: Arc<SecretsService>) -> Self {
        Self { secrets_service }
    }
}

#[tonic::async_trait]
impl Secrets for SecretsDebugGrpc {
    async fn get_secrets(
        &self,
        request: Request<GetSecretsRequest>,
    ) -> Result<Response<GetSecretsResponse>, Status> {
        let req = request.into_inner();
        debug!(
            "Received gRPC get_secrets with {} items",
            req.secret_requests.len()
        );

        let Some(requester_info_proto) = req.requester_info else {
            return Err(Status::invalid_argument("Missing requester info"));
        };
        let requester_info = SecretRequesterInfo::from_proto(requester_info_proto);

        let mut secret_requests = HashMap::new();
        for sr in req.secret_requests {
            let id_proto = sr.id.ok_or_else(|| Status::invalid_argument("Missing SecretIdentifier"))?;
            let domain_id = SecretId::from_proto(id_proto);

            let requests: Vec<SecretRequest> = sr
                .requests
                .into_iter()
                .map(|r_proto| SecretRequest::from_proto(r_proto))
                .collect();

            secret_requests.insert(domain_id, requests);
        }

        let secrets_box: SecretsBox = self
            .secrets_service
            .get_secrets(secret_requests, requester_info)
            .await
            .map_err(|e| Status::internal(format!("{:?}", e)))?;

        let proto_box = secrets_box.to_proto();

        let response = GetSecretsResponse {
            secrets_box: Some(proto_box),
        };

        Ok(Response::new(response))
    }
}
