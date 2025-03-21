pub mod notifier;
pub mod secrets;

use std::sync::Arc;

use futures::channel::mpsc;

use crate::{
    network::{NotifierMessage, SecretsMessage},
    services::{notifier::start_service as start_notifier, secrets::SecretsService},
};

pub struct ServiceManager {
    notifier_sender: mpsc::Sender<NotifierMessage>,
    secrets_service: Arc<SecretsService>,
}

impl ServiceManager {
    pub fn new(
        notifier_sender: mpsc::Sender<NotifierMessage>,
        p2p_secrets_sender: mpsc::Sender<SecretsMessage>,
    ) -> Self {
        let secrets_service = SecretsService::new(p2p_secrets_sender);

        {
            let clone = notifier_sender.clone();
            tokio::spawn(async move {
                start_notifier(clone).await;
            });
        }

        Self {
            notifier_sender,
            secrets_service,
        }
    }

    pub fn secrets_service(&self) -> Arc<SecretsService> {
        Arc::clone(&self.secrets_service)
    }
}
