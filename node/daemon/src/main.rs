#![allow(unused)]

mod config;
mod error;
mod grpc;
mod identity;
mod network;
mod policy;
mod services;
mod web3;

use tracing::{Level, info};
use tracing_subscriber::{EnvFilter, fmt::Subscriber};

use crate::{
    config::Config,
    identity::{create_ephemeral_identity, get_or_create_identity},
    network::NetworkManager,
    services::secrets::SecretsService,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);

    let config = Config::load()?;
    let log_level = if config.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };

    let base_filter = format!("{}={}", env!("CARGO_PKG_NAME"), log_level);
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(base_filter));
    let subscriber = Subscriber::builder().with_env_filter(env_filter).finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting daemon...");

    let local_key = match &config.identity_path {
        Some(path) => get_or_create_identity(path)?,
        None => create_ephemeral_identity(),
    };

    let local_peer_id = local_key.public().to_peer_id();
    info!("Local peer id: {local_peer_id}");

    let (secrets_tx, secrets_rx) = futures::channel::mpsc::channel(64);
    let (notifier_tx, notifier_rx) = futures::channel::mpsc::channel(64);

    let secrets_service = SecretsService::new(secrets_tx.clone()).await;

    {
        let notifier_tx_clone = notifier_tx.clone();
        tokio::spawn(async move {
            services::notifier::start_service(notifier_tx_clone).await;
        });
    }

    let mut network = NetworkManager::new(
        local_key,
        config.clone(),
        secrets_service.clone(),
        notifier_rx,
        secrets_rx,
    )
    .await?;

    network.start(shutdown_tx.subscribe()).await?;

    {
        let grpc_config = config.grpc.clone();
        let secrets_service_clone = secrets_service.clone();
        tokio::spawn(async move {
            if let Err(e) =
                crate::grpc::start_grpc_server(&grpc_config, secrets_service_clone, shutdown_rx)
                    .await
            {
                tracing::error!("gRPC server error: {e}");
            }
        });
    }

    tokio::signal::ctrl_c().await?;
    tracing::info!("Received Ctrl-C, shutting down...");
    let _ = shutdown_tx.send(());
    Ok(())
}
