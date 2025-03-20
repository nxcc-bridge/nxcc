use ethers::types::{Address, H256};
use tonic::{Request, Response, Status};
use tracing::{debug, info};

use crate::{network::SecretsMessage, services::secrets as internal_secrets};

pub mod proto {
    tonic::include_proto!("secrets");
}

pub struct SecretsService {
    pub secrets_sender: futures::channel::mpsc::Sender<SecretsMessage>,
}

impl SecretsService {
    pub fn new(secrets_sender: futures::channel::mpsc::Sender<SecretsMessage>) -> Self {
        Self { secrets_sender }
    }
}

#[tonic::async_trait]
impl proto::secrets_server::Secrets for SecretsService {
    async fn get_secrets_batch(
        &self,
        request: Request<proto::GetSecretsBatchRequest>,
    ) -> Result<Response<proto::GetSecretsBatchResponse>, Status> {
        let req = request.into_inner();

        debug!(
            "Received gRPC call to GetSecretsBatch with {} secrets",
            req.secrets.len()
        );

        let mut parsed_identifiers = Vec::with_capacity(req.secrets.len());
        for sid in req.secrets {
            let chain_id = sid.chain_id;
            let identity_address = sid
                .identity_address
                .parse::<Address>()
                .map_err(|_| Status::invalid_argument("Invalid identity_address"))?;
            let identity_id = sid
                .identity_id
                .parse::<H256>()
                .map_err(|_| Status::invalid_argument("Invalid identity_id"))?;
            parsed_identifiers.push(internal_secrets::SecretIdentifier {
                chain_id,
                identity_address,
                identity_id,
            });
        }

        let internal_resp = internal_secrets::process_get_secrets_batch_request(
            parsed_identifiers,
            req.payload,
            self.secrets_sender.clone(),
        )
        .await
        .map_err(|e| Status::internal(format!("Error processing batch request: {}", e)))?;

        debug!(
            "Returning {} secrets in GetSecretsBatch response",
            internal_resp.secrets.len()
        );

        let mut grpc_secrets = Vec::with_capacity(internal_resp.secrets.len());
        for s in internal_resp.secrets {
            grpc_secrets.push(proto::EncryptedSecret {
                data: s.data,
                metadata: s.metadata,
                chain_id: s.chain_id,
                identity_address: format!("{:#x}", s.identity_address),
                identity_id: format!("{:#x}", s.identity_id),
            });
        }

        let response = proto::GetSecretsBatchResponse {
            secrets: grpc_secrets,
        };

        Ok(Response::new(response))
    }
}
