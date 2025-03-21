use std::time::Duration;

use futures::channel::mpsc;
use tokio::time::sleep;
use tracing::{debug, error, info};

use crate::network::NotifierMessage;

pub async fn start_service(mut sender: mpsc::Sender<NotifierMessage>) {
    info!("Starting notifier service");

    // Periodically send notifications
    tokio::spawn(async move {
        let mut counter = 0;
        loop {
            sleep(Duration::from_secs(60)).await;
            counter += 1;

            let notification = format!("Periodic notification #{}", counter);
            debug!("Sending notification: {}", notification);

            if let Err(e) = sender.try_send(NotifierMessage::Notification(notification)) {
                error!("Failed to send notification: {}", e);
            }

            // Simulate a response after a short delay
            sleep(Duration::from_secs(2)).await;
            let response = format!("Response to notification #{}", counter);

            if let Err(e) = sender.try_send(NotifierMessage::Response(response)) {
                error!("Failed to send notification response: {}", e);
            }
        }
    });
}
