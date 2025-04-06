use std::{collections::HashMap, sync::Arc};

use ethers::types::{Address, U256};
use interface::{
    Secret as DomainSecret, SecretId, SecretRequest, SecretRequesterInfo, SecretsBox,
    proto::daemon::{
        GetSecretsRequest, GetSecretsResponse, SecretIdentifier,
        SecretRequest as ProtoSecretRequest, SecretRequests, SecretsBox as ProtoSecretsBox,
        secrets_server::Secrets,
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

        let Some(requester_info) = req.requester_info else {
            return Err(Status::invalid_argument("Missing requester info"));
        };
        let requester_info = SecretRequesterInfo {
            report: requester_info.report.clone(),
            public_key: requester_info.public_key,
        };

        // Convert secret requests map
        let mut secret_requests = HashMap::new();
        for requests in req.secret_requests {
            let id_proto = requests
                .id
                .ok_or_else(|| Status::invalid_argument("Missing SecretIdentifier in request"))?;

            let address = id_proto
                .identity_address
                .parse::<Address>()
                .map_err(|_| Status::invalid_argument("Invalid identity_address"))?;

            let ident = id_proto
                .identity_id
                .parse::<U256>()
                .map_err(|e| Status::invalid_argument(format!("Invalid identity_id: {e}")))?;

            let id = SecretId {
                chain_id: id_proto.chain_id,
                identity_address: address,
                identity_id: ident,
            };

            let requests: Vec<SecretRequest> = requests
                .requests
                .into_iter()
                .map(|r| SecretRequest {
                    consumer: r.consumer,
                })
                .collect();

            secret_requests.insert(id, requests);
        }

        // Call the service
        let secrets_box = self
            .secrets_service
            .get_secrets(secret_requests, requester_info)
            .await
            .map_err(|e| Status::internal(format!("{:?}", e)))?;

        // Convert SecretsBox to proto format
        let proto_box = ProtoSecretsBox {
            alg: secrets_box.alg,
            nonce: secrets_box.nonce,
            sender_public_key: secrets_box.sender_public_key,
            payload: secrets_box.encrypted_payload,
            signature: secrets_box.signature,
        };

        let response = GetSecretsResponse {
            secrets_box: Some(proto_box),
        };

        Ok(Response::new(response))
    }
}
