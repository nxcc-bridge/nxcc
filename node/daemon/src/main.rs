#![allow(warnings)]

mod config;
mod error;
mod grpc;
mod identity;
mod network;
mod policy;
mod services;
mod web3;

use std::sync::Arc;

use grpc::enclave_client;
use tracing::{Level, info};
use tracing_subscriber::{EnvFilter, fmt::Subscriber};

use crate::{
    config::{Config, EnclaveConfig},
    identity::{create_ephemeral_identity, get_or_create_identity},
    network::NetworkManager,
    policy::PolicyManager,
    services::{runner::RunnerService, secrets::SecretsService},
    web3::gateways::GatewayManager,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::load()?;

    let log_level = if config.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };
    let base_filter = format!("{}={}", env!("CARGO_PKG_NAME").replace("-", "_"), log_level);
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(base_filter));
    let subscriber = Subscriber::builder().with_env_filter(env_filter).finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let local_key = match &config.identity_path {
        Some(path) => get_or_create_identity(path)?,
        None => create_ephemeral_identity(),
    };

    if config.print_peer_id {
        println!("{}", local_key.public().to_peer_id());
        return Ok(());
    }

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);

    let (secrets_tx, secrets_rx) = futures::channel::mpsc::channel(64);
    let (notifier_tx, notifier_rx) = futures::channel::mpsc::channel(64);

    // Connect to the enclave
    info!(
        "connecting to enclave over UDS {}",
        config.enclave.enclave_uds_path.clone()
    );
    let enclave_client =
        grpc::enclave_client::EnclaveClient::connect_uds(config.enclave.enclave_uds_path.clone())
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "Failed to create EnclaveClient: {}. Ensure the enclave is running on {}.",
                    config.enclave.enclave_uds_path, e
                )
            });

    // Instantiate services
    let runner_service = Arc::new(RunnerService::new(
        enclave_client.runner(),
        config.enclave.clone(),
    ));
    let gateway_manager = GatewayManager::new();
    let policy_manager = Arc::new(PolicyManager::new(gateway_manager, &config).await?);

    // Attach the policy VM to the enclave's runner service
    runner_service
        .attach_policy_vm()
        .await
        .expect("Failed to attach policy VM to enclave runner");

    let secrets_service = SecretsService::new(
        secrets_tx.clone(),
        enclave_client.clone(), // SecretsService still needs the combined client for secrets calls
        policy_manager.clone(),
        runner_service.clone(), // Inject RunnerService
        local_key.clone(),      // Pass the local keypair
                                // Arc::new(config.clone()), // Pass config if needed
    );

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
        // Pass enclave client to gRPC server for the final get_secrets call
        let enclave_client_clone = enclave_client.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::grpc::start_grpc_server(
                &grpc_config,
                secrets_service_clone,
                enclave_client_clone, // Pass enclave client here
                shutdown_rx,
            )
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
