mod config;
mod error;
mod grpc;
mod identity;
mod network;
mod services;

use futures::channel::mpsc;
use tracing::{Level, info};
use tracing_subscriber::EnvFilter;

use crate::{
    config::Config,
    network::{NetworkManager, SecretsMessage},
    services::ServiceManager,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::load()?;

    let log_level = if config.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(log_level.into())
                .from_env()
                .unwrap(),
        )
        .init();

    info!("Starting P2P service...");

    let identity_path = config.identity_path.clone();

    let local_key = match identity_path {
        Some(path) => identity::get_or_create_identity(&path)?,
        None => {
            info!("No identity path specified; using ephemeral identity.");
            identity::create_ephemeral_identity()
        }
    };

    let local_peer_id = local_key.public().to_peer_id();
    info!("Local peer id: {}", local_peer_id);

    let mut network = NetworkManager::new(local_key, config.clone()).await?;
    network.start().await?;

    // Grab the secrets sender from the network
    let secrets_sender = network
        .secrets_sender
        .clone()
        .expect("Secrets sender missing");

    let _service_manager = ServiceManager::new(
        network
            .notifier_sender
            .clone()
            .expect("Notifier sender missing"),
        secrets_sender.clone(),
    );

    // Start the gRPC server
    tokio::spawn(async move {
        if let Err(e) = grpc::start_grpc_server(&config.grpc, secrets_sender).await {
            tracing::error!("gRPC server error: {}", e);
        }
    });

    tokio::signal::ctrl_c().await?;
    info!("Shutting down...");
    Ok(())
}
