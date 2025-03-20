use std::time::Duration;

use futures::channel::mpsc;
use tokio::time::sleep;
use tracing::{error, info};

use crate::{error::AppError, network::SecretsMessage};

pub async fn start_service(mut sender: mpsc::Sender<SecretsMessage>) {
    info!("Starting secrets service");

    // // Periodically send secret requests and responses
    // tokio::spawn(async move {
    //     let mut counter = 0;
    //     loop {
    //         sleep(Duration::from_secs(45)).await;
    //         counter += 1;

    //         let request = format!("Secret request #{}", counter);
    //         info!("Sending secret request: {}", request);

    //         if let Err(e) = sender.try_send(SecretsMessage::Request(request)) {
    //             error!("Failed to send secret request: {}", e);
    //         }

    //         // Simulate a response after a short delay
    //         sleep(Duration::from_secs(3)).await;
    //         let response = format!("Secret response #{}", counter);

    //         if let Err(e) = sender.try_send(SecretsMessage::Response(response)) {
    //             error!("Failed to send secret response: {}", e);
    //         }
    //     }
    // });
}

/// Dummy types for internal secret processing.
pub struct EncryptedSecretData {
    pub data: Vec<u8>,
    pub metadata: String,
}

pub struct EncryptedSecrets {
    pub secrets: Vec<EncryptedSecretData>,
}

/// Process a get_secret request. Simulate a long-running operation (e.g. querying peers)
/// and then return a dummy “encrypted secret” response.
pub async fn process_get_secret_request(payload: Vec<u8>) -> Result<EncryptedSecrets, AppError> {
    info!(
        "Processing get_secret request with payload of {} bytes",
        payload.len()
    );
    sleep(Duration::from_secs(3)).await;
    Ok(EncryptedSecrets {
        secrets: vec![EncryptedSecretData {
            data: payload,
            metadata: "dummy encrypted secret".into(),
        }],
    })
}
