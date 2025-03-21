use std::sync::Arc;

use ethers::types::{Address, H256};
use interface::proto::daemon::{
    EncryptedSecret, GetSecretsRequest, GetSecretsResponse, secrets_server::Secrets,
};
use tonic::{Request, Response, Status};
use tracing::debug;

use crate::{
    error::AppError,
    services::secrets::{Secret, SecretId, SecretsService},
};

pub struct SecretsDebugGrpc {
    secrets_service: Arc<SecretsService>,
}

impl SecretsDebugGrpc {
    pub fn new(secrets_service: Arc<SecretsService>) -> Self {
        Self { secrets_service }
    }
}

impl SecretsDebugGrpc {
    async fn transform_and_get(
        &self,
        secret_ids: Vec<SecretId>,
        payload: Vec<u8>,
    ) -> Result<Vec<Secret>, AppError> {
        self.secrets_service.get_secrets(secret_ids, payload).await
    }
}

#[tonic::async_trait]
impl Secrets for SecretsDebugGrpc {
    async fn get_secrets(
        &self,
        request: Request<GetSecretsRequest>,
    ) -> Result<Response<GetSecretsResponse>, Status> {
        let req = request.into_inner();
        debug!("Received gRPC get_secrets with {} items", req.secrets.len());

        let mut ids = Vec::with_capacity(req.secrets.len());
        for sid in req.secrets {
            let address = sid
                .identity_address
                .parse::<Address>()
                .map_err(|_| Status::invalid_argument("Invalid identity_address"))?;
            let ident = sid
                .identity_id
                .parse::<H256>()
                .map_err(|_| Status::invalid_argument("Invalid identity_id"))?;
            ids.push(SecretId {
                chain_id: sid.chain_id,
                identity_address: address,
                identity_id: ident,
            });
        }

        let secrets = self
            .transform_and_get(ids, req.payload)
            .await
            .map_err(|e| Status::internal(format!("{:?}", e)))?;

        let response = GetSecretsResponse {
            secrets: secrets
                .into_iter()
                .map(|s| EncryptedSecret {
                    data: s.data,
                    metadata: s.metadata,
                    chain_id: s.id.chain_id,
                    identity_address: format!("{:#x}", s.id.identity_address),
                    identity_id: format!("{:#x}", s.id.identity_id),
                })
                .collect(),
        };

        Ok(Response::new(response))
    }
}
