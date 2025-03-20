use tonic::{Request, Response, Status};

use interface::proto::enclave::{
    EncryptedSecret, GetSecretsRequest,
    GetSecretsResponse, secrets_server::Secrets,
};

#[derive(Default)]
pub struct SecretsService {}

#[tonic::async_trait]
impl Secrets for SecretsService {
    async fn get_secrets(
        &self,
        _request: Request<GetSecretsRequest>,
    ) -> Result<Response<GetSecretsResponse>, Status> {
        // For debugging/testing only:
        // This might eventually be replaced by internal calls from the runner to the secrets logic
        // (rather than being a user-facing gRPC endpoint).
        let resp = GetSecretsResponse {
            secrets: vec![EncryptedSecret {
                data: b"fake_secret_data".to_vec(),
                metadata: b"debug_metadata".to_vec(),
                chain_id: 1,
                identity_address: "0x00".to_string(),
                identity_id: "0x00".to_string(),
            }],
        };
        Ok(Response::new(resp))
    }
}
