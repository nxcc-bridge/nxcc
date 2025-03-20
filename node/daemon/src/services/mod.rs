pub mod notifier;
pub mod secrets;

use futures::channel::mpsc;

use crate::network::{NotifierMessage, SecretsMessage};

pub struct ServiceManager {
    notifier_sender: mpsc::Sender<NotifierMessage>,
    secrets_sender: mpsc::Sender<SecretsMessage>,
}

impl ServiceManager {
    pub fn new(
        notifier_sender: mpsc::Sender<NotifierMessage>,
        secrets_sender: mpsc::Sender<SecretsMessage>,
    ) -> Self {
        let notifier_sender_clone = notifier_sender.clone();
        let secrets_sender_clone = secrets_sender.clone();

        tokio::spawn(async move {
            notifier::start_service(notifier_sender_clone).await;
        });

        tokio::spawn(async move {
            secrets::start_service(secrets_sender_clone).await;
        });

        Self {
            notifier_sender,
            secrets_sender,
        }
    }
}
