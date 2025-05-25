#![allow(warnings)]

use tokio::time::{Duration, interval};
mod config;
mod error;
mod grpc;
mod http_server; // New module for Axum server
mod identity;
mod network;
mod policy;
mod services;
mod web3;

use std::{collections::HashMap, sync::Arc};

use grpc::enclave_client;
use nxcc_interface::proto::enclave as enclave_proto;
use tokio::sync::RwLock;
use tracing::{Level, error, info};
use tracing_subscriber::{EnvFilter, fmt::Subscriber};

use crate::{
    config::{Config, EnclaveConfig},
    http_server::start_http_server,
    identity::{create_ephemeral_identity, get_or_create_identity},
    network::NetworkManager,
    policy::PolicyManager,
    services::{runner::RunnerService, secrets::SecretsService},
    web3::gateways::GatewayManager,
};

const DAEMON_EVENT_QUEUE_CAPACITY: usize = 1024; // Example capacity
const EVENT_BATCH_SIZE: usize = 100;

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

    // Central event queue for daemon to send events to enclave
    let (daemon_event_tx, mut daemon_event_rx) =
        tokio::sync::mpsc::channel::<enclave_proto::EventDelivery>(DAEMON_EVENT_QUEUE_CAPACITY);

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
                    e, config.enclave.enclave_uds_path,
                )
            });

    // Instantiate services
    let runner_service = Arc::new(RunnerService::new(
        enclave_client.runner(),
        config.enclave.clone(),
    ));
    let gateway_manager = Arc::new(GatewayManager::new());
    let policy_manager = Arc::new(PolicyManager::new(gateway_manager.clone(), &config).await?);

    // Attach the default VM to the enclave's runner service
    runner_service
        .attach_default_vm()
        .await
        .expect("Failed to attach default VM to enclave runner");

    let secrets_service = SecretsService::new(
        secrets_tx.clone(),
        enclave_client.clone(), // SecretsService still needs the combined client for secrets calls
        policy_manager.clone(),
        runner_service.clone(), // Inject RunnerService
        local_key.clone(),      // Pass the local keypair
    );

    let mut network = NetworkManager::new(
        local_key,
        config.clone(),
        secrets_service.clone(),
        secrets_rx,
    )
    .await?;

    network.start(shutdown_tx.subscribe()).await?;

    let http_mounts = Arc::new(RwLock::new(HashMap::<String, String>::new()));

    {
        let grpc_config = config.grpc.clone();
        let secrets_service_clone = secrets_service.clone();
        let enclave_client_clone = enclave_client.clone();
        let http_mounts_clone_for_wo = http_mounts.clone();
        let work_order_orchestrator =
            crate::services::work_order_orchestrator::WorkOrderOrchestrator::new(
                enclave_client.clone(),
                secrets_service.clone(),
                runner_service.clone(),
                policy_manager.clone(),
                gateway_manager.clone(),
                Arc::new(config.clone()),
                http_mounts_clone_for_wo,
                daemon_event_tx.clone(),
                shutdown_rx.resubscribe(),
            );

        let enclave_client_for_grpc_server = enclave_client.clone();
        tokio::spawn(async move {
            if let Err(e) = grpc::start_grpc_server(
                &grpc_config,
                secrets_service_clone,
                work_order_orchestrator,
                enclave_client_for_grpc_server,
                shutdown_rx,
            )
            .await
            {
                tracing::error!("gRPC server error: {e}");
            }
        });
    }

    // Start HTTP server for worker requests
    let http_config = config.http.clone();
    let enclave_client_for_http = enclave_client.clone();
    let mut shutdown_rx_for_http = shutdown_tx.subscribe();
    let http_mounts_for_server = http_mounts.clone();

    tokio::spawn(async move {
        if let Err(e) = start_http_server(
            &http_config,
            http_mounts_for_server,
            enclave_client_for_http,
            async move {
                shutdown_rx_for_http.recv().await.ok();
            },
        )
        .await
        {
            error!("HTTP server error: {}", e);
        }
    });

    // Task to batch and send events from daemon_event_rx to enclave
    let enclave_client_for_event_delivery = enclave_client.clone();
    let mut shutdown_rx_for_event_delivery = shutdown_tx.subscribe();
    tokio::spawn(async move {
        let mut batch = Vec::with_capacity(EVENT_BATCH_SIZE);
        let mut ticker = interval(Duration::from_millis(100)); // Send batch every 100ms or if full

        loop {
            tokio::select! {
                biased;
                _ = shutdown_rx_for_event_delivery.recv() => {
                    info!("Event delivery task shutting down.");
                    // Try to send any remaining events in the batch
                    if !batch.is_empty() {
                        if let Err(e) = enclave_client_for_event_delivery.deliver_batch_events(std::mem::take(&mut batch)).await {
                            error!("Failed to send final batch of events to enclave: {}", e);
                        }
                    }
                    break;
                }
                event_opt = daemon_event_rx.recv() => {
                    if let Some(event) = event_opt {
                        batch.push(event);
                    } else { // Channel closed
                        break;
                    }
                }
                _ = ticker.tick(), if !batch.is_empty() => { /* Timer ticked and batch is not empty, send below */ }
            }

            if batch.len() >= EVENT_BATCH_SIZE || (!batch.is_empty() && daemon_event_rx.is_empty())
            {
                // Send if batch full or channel empty
                if let Err(e) = enclave_client_for_event_delivery
                    .deliver_batch_events(std::mem::take(&mut batch))
                    .await
                {
                    error!("Failed to send batch of events to enclave: {}", e);
                    // TODO: Add retry logic or dead-letter queue for batch
                }
            }
        }
    });

    tokio::signal::ctrl_c().await?;
    tracing::info!("Received Ctrl-C, shutting down...");
    let _ = shutdown_tx.send(());
    Ok(())
}
