use std::collections::HashMap;

use interface::{
    proto::daemon::{GetSecretsRequest, GetSecretsResponse, secrets_server::Secrets},
    types::{EnvReport, SecretId, SecretRequest, SecretsBox},
};
use tonic::{Request, Response, Status};
use tracing::debug;

use crate::services::secrets::SecretsService;

pub struct SecretsDebugGrpc {
    secrets_service: std::sync::Arc<SecretsService>,
}

impl SecretsDebugGrpc {
    pub fn new(secrets_service: std::sync::Arc<SecretsService>) -> Self {
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
            "Received gRPC get_secrets with {} request items",
            req.secret_requests.len()
        );

        let env_proto = req
            .env_report
            .ok_or_else(|| Status::invalid_argument("Missing EnvReport"))?;
        let env_report = EnvReport::from_proto(env_proto);

        let mut grouped = HashMap::new();
        for proto_req in req.secret_requests {
            let sr = SecretRequest::from_proto(proto_req);
            grouped
                .entry(sr.secret_id.clone())
                .or_insert_with(Vec::new)
                .push(sr);
        }

        let secrets_box: SecretsBox = self
            .secrets_service
            .get_secrets(grouped, env_report)
            .await
            .map_err(|e| Status::internal(format!("SecretsService error: {:?}", e)))?;

        let resp = GetSecretsResponse {
            secrets_box: Some(secrets_box.to_proto()),
        };
        Ok(Response::new(resp))
    }
}
