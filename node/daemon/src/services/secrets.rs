use std::time::Duration;

use futures::channel::mpsc;
use tokio::{sync::oneshot, time::sleep};
use tracing::info;

use crate::{error::AppError, network::SecretsMessage};

pub async fn start_service(mut sender: mpsc::Sender<SecretsMessage>) {
    info!("Starting secrets service");
}

#[derive(Debug, Clone)]
pub struct EncryptedSecretData {
    pub data: Vec<u8>,
    pub metadata: String,
}

pub struct EncryptedSecrets {
    pub secrets: Vec<EncryptedSecretData>,
}

/// Process a get_secret request by querying peers on the network
pub async fn process_get_secret_request(
    chain_id: String,
    contract_address: String,
    secret_id: String,
    payload: Vec<u8>,
    sender: mpsc::Sender<SecretsMessage>,
) -> Result<EncryptedSecrets, AppError> {
    info!(
        "Processing get_secret request: chain_id={}, contract_address={}, secret_id={}",
        chain_id, contract_address, secret_id
    );

    // Create a channel for the response
    let (response_sender, response_receiver) = oneshot::channel();

    // For simplicity, use a fixed threshold
    let threshold = 2;

    // Send the request to the network manager
    if let Err(e) = sender.clone().try_send(SecretsMessage::GetSecret {
        chain_id,
        contract_address,
        secret_id,
        payload: payload.clone(),
        threshold,
        response_sender,
    }) {
        return Err(AppError::Service(format!(
            "Failed to send request to network: {}",
            e
        )));
    }

    // Wait for the response
    match response_receiver.await {
        Ok(result) => Ok(EncryptedSecrets { secrets: result? }),
        Err(e) => Err(AppError::Service(format!(
            "Failed to receive response: {}",
            e
        ))),
    }
}
