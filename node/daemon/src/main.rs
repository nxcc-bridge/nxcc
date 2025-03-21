mod config;
mod error;
mod grpc;
mod identity;
mod network;
mod services;

use tracing::{Level, info};
use tracing_subscriber::{EnvFilter, fmt::Subscriber};

use crate::{
    config::Config,
    identity::{create_ephemeral_identity, get_or_create_identity},
    network::NetworkManager,
    services::{ServiceManager, secrets::SecretsService},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // ----- 1. Load config -----
    let config = Config::load()?;
    let log_level = if config.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };

    // ----- 2. Initialize logging -----
    // By default, let's raise libp2p_mdns to WARN to reduce "No route to host" spam
    let base_filter = format!("{}={},libp2p_mdns=warn", env!("CARGO_PKG_NAME"), log_level);
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(base_filter));
    let subscriber = Subscriber::builder().with_env_filter(env_filter).finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting daemon...");

    // ----- 3. Load or create identity key -----
    let local_key = match &config.identity_path {
        Some(path) => get_or_create_identity(path)?,
        None => create_ephemeral_identity(),
    };

    let local_peer_id = local_key.public().to_peer_id();
    info!("Local peer id: {local_peer_id}");

    // ----- 4. Create the network manager first -----
    // Create a temporary SecretsService with a dummy channel that will be replaced
    let (dummy_tx, _) = futures::channel::mpsc::channel(1);
    let temp_secrets_service = SecretsService::new(dummy_tx);

    let mut network = NetworkManager::new(local_key, config.clone(), temp_secrets_service).await?;
    network.start().await?;

    // ----- 5. Now create the real services with the actual network channels -----
    let notifier_tx = network
        .notifier_sender
        .clone()
        .expect("No notifier_sender available");
    let secrets_tx = network
        .secrets_sender
        .clone()
        .expect("No secrets_sender available");

    let service_manager = ServiceManager::new(notifier_tx, secrets_tx);
    let secrets_service = service_manager.secrets_service();

    // Update the network manager with the real secrets service
    network.secrets_service = secrets_service.clone();

    // ----- 6. Start gRPC server in background task -----
    {
        let grpc_config = config.grpc.clone();
        tokio::spawn(async move {
            if let Err(e) =
                crate::grpc::start_grpc_server(&grpc_config, secrets_service, shutdown_rx).await
            {
                tracing::error!("gRPC server error: {e}");
            }
        });
    }

    // ----- 7. Wait for Ctrl-C -----
    tokio::signal::ctrl_c().await?;
    tracing::info!("Received Ctrl-C, shutting down...");
    let _ = shutdown_tx.send(());
    Ok(())
}
