use ethers::types::{Address, H256};
use futures::channel::mpsc;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tracing::debug;

use crate::{error::AppError, network::SecretsMessage};

pub async fn start_service(_sender: mpsc::Sender<SecretsMessage>) {
    debug!("Starting secrets service");
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretIdentifier {
    pub chain_id: u64,
    pub identity_address: Address,
    pub identity_id: H256,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchEncryptedSecretData {
    pub chain_id: u64,
    pub identity_address: Address,
    pub identity_id: H256,
    pub data: Vec<u8>,
    pub metadata: String,
}

pub struct EncryptedSecretsBatch {
    pub secrets: Vec<BatchEncryptedSecretData>,
}

pub async fn process_get_secrets_batch_request(
    secrets: Vec<SecretIdentifier>,
    payload: Vec<u8>,
    sender: mpsc::Sender<SecretsMessage>,
) -> Result<EncryptedSecretsBatch, AppError> {
    debug!(
        "Processing batch request for {} secret(s), payload size={}",
        secrets.len(),
        payload.len()
    );

    let (response_sender, response_receiver) =
        oneshot::channel::<Result<Vec<BatchEncryptedSecretData>, AppError>>();

    let threshold = 2;

    debug!(
        "Sending SecretsMessage::GetSecretsBatch with threshold={}",
        threshold
    );

    sender.clone().try_send(SecretsMessage::GetSecretsBatch {
        secrets,
        payload,
        threshold,
        response_sender,
    })?;

    match response_receiver.await {
        Ok(Ok(results)) => {
            debug!("Got {} secrets back from network", results.len());
            Ok(EncryptedSecretsBatch { secrets: results })
        }
        Ok(Err(e)) => {
            debug!("Network responded with an error: {}", e);
            Err(e)
        }
        Err(e) => {
            debug!("Failed to receive batch response: {}", e);
            Err(AppError::Service(format!(
                "Failed to receive batch response: {}",
                e
            )))
        }
    }
}
