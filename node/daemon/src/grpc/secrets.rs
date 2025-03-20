use tokio::time::{Duration, sleep};
use tonic::{Request, Response, Status};
use tracing::info;

use crate::services::secrets as internal_secrets;

pub mod proto {
    tonic::include_proto!("secrets");
}

#[derive(Default)]
pub struct SecretsService {}

#[tonic::async_trait]
impl proto::secrets_server::Secrets for SecretsService {
    #[tracing::instrument(skip(self))]
    async fn get_secret(
        &self,
        request: Request<proto::GetSecretRequest>,
    ) -> Result<Response<proto::GetSecretResponse>, Status> {
        let req = request.into_inner();
        info!(
            "Received GetSecret request: chain_id={}, contract_address={}, secret_id={}",
            req.chain_id, req.contract_address, req.secret_id
        );
        // Simulate long-running secret share collection
        sleep(Duration::from_secs(3)).await;
        let internal_resp = internal_secrets::process_get_secret_request(req.payload)
            .await
            .map_err(|e| Status::internal(format!("Error processing secret: {}", e)))?;
        let grpc_secrets = internal_resp
            .secrets
            .into_iter()
            .map(|s| proto::EncryptedSecret {
                data: s.data,
                metadata: s.metadata,
            })
            .collect();
        let response = proto::GetSecretResponse {
            secrets: grpc_secrets,
        };
        Ok(Response::new(response))
    }
}
