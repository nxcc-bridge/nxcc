use ethers::types::{Address, H256};
pub use interface::proto::daemon::{
    EncryptedSecret, GetSecretsRequest, GetSecretsResponse,
    secrets_server::Secrets,
};
use tonic::{Request, Response, Status};
use tracing::debug;

use crate::{network::SecretsMessage, services::secrets as internal_secrets};

pub struct SecretsService {
    secrets_sender: futures::channel::mpsc::Sender<SecretsMessage>,
}

impl SecretsService {
    pub fn new(secrets_sender: futures::channel::mpsc::Sender<SecretsMessage>) -> Self {
        Self { secrets_sender }
    }
}

#[tonic::async_trait]
impl Secrets for SecretsService {
    async fn get_secrets(
        &self,
        request: Request<GetSecretsRequest>,
    ) -> Result<Response<GetSecretsResponse>, Status> {
        let req = request.into_inner();

        debug!(
            "Received gRPC call to GetSecrets with {} secret(s)",
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

        // Reuse the same internal function that processes the batch request.
        let internal_resp = internal_secrets::process_get_secrets_batch_request(
            parsed_identifiers,
            req.payload,
            self.secrets_sender.clone(),
        )
        .await
        .map_err(|e| Status::internal(format!("Error processing request: {}", e)))?;

        // Map internal secret data to the gRPC response.
        let grpc_secrets = internal_resp
            .secrets
            .into_iter()
            .map(|s| EncryptedSecret {
                data: s.data,
                metadata: s.metadata.into(),
                chain_id: s.chain_id,
                identity_address: format!("{:#x}", s.identity_address),
                identity_id: format!("{:#x}", s.identity_id),
            })
            .collect();

        let response = GetSecretsResponse {
            secrets: grpc_secrets,
        };

        Ok(Response::new(response))
    }
}
