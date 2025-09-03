#![allow(warnings)]

use tokio::time::{Duration, interval, sleep};
mod config;
mod error;
mod grpc;
mod http_server;
mod identity;
mod network;
mod policy;
mod services;
mod web3;

use std::{collections::HashMap, sync::Arc};

use grpc::enclave_client;
use nxcc_interface::proto::enclave as enclave_proto;
use tokio::sync::RwLock;
use tracing::{Level, error, info, warn};
use tracing_subscriber::{EnvFilter, FmtSubscriber as Subscriber, fmt::format::FmtSpan};

use crate::{
    config::{Config, EnclaveConfig},
    http_server::{PeerRegistry, VmRegistry, start_http_server},
    identity::{create_ephemeral_identity, get_or_create_identity},
    network::NetworkManager,
    policy::PolicyManager,
    services::{runner::RunnerService, scheduler::SchedulerHandle, secrets::SecretsService},
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
    let subscriber = Subscriber::builder()
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
        .with_env_filter(env_filter)
        .finish();
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

    // Connect to the enclave with retry logic
    info!(
        "connecting to enclave over UDS {}",
        config.enclave.enclave_uds_path.clone()
    );

    let enclave_client = {
        let mut attempts = 0;
        const MAX_ATTEMPTS: u32 = 10;
        loop {
            attempts += 1;
            match grpc::enclave_client::EnclaveClient::connect_uds(
                config.enclave.enclave_uds_path.clone(),
            )
            .await
            {
                Ok(client) => break client,
                Err(e) => {
                    if attempts >= MAX_ATTEMPTS {
                        panic!(
                            "Failed to create EnclaveClient after {} attempts: {}. Ensure the \
                             enclave is running on {}.",
                            MAX_ATTEMPTS, e, config.enclave.enclave_uds_path,
                        );
                    }
                    warn!(
                        "Failed to connect to enclave (attempt {}/{}): {}. Retrying in {}ms...",
                        attempts,
                        MAX_ATTEMPTS,
                        e,
                        attempts * 100
                    );
                    sleep(Duration::from_millis(attempts as u64 * 100)).await;
                }
            }
        }
    };

    // Create VM registry for tracking attached VMs
    let vm_registry = VmRegistry::new();

    // Create peer registry for tracking connected peers
    let peer_registry = PeerRegistry::new();

    // Instantiate services
    let runner_service = Arc::new(RunnerService::new(
        enclave_client.runner(),
        config.enclave.clone(),
        vm_registry.clone(),
    ));
    let gateway_manager = Arc::new(GatewayManager::new());
    let policy_manager = Arc::new(PolicyManager::new(gateway_manager.clone(), &config).await?);

    // Create scheduler handle
    let scheduler_handle = Arc::new(SchedulerHandle::new(config.scheduler.clone()));

    // Attach the default VM to the enclave's runner service
    runner_service
        .attach_default_vm()
        .await
        .expect("Failed to attach default VM to enclave runner");

    let secrets_service = SecretsService::new(
        secrets_tx.clone(),
        enclave_client.clone(), // SecretsService still needs the combined client for secrets calls
        policy_manager.clone(),
        runner_service.clone(),   // Inject RunnerService
        local_key.clone(),        // Pass the local keypair
        Arc::new(config.clone()), // Pass config
    );

    let mut network = NetworkManager::new(
        local_key.clone(),
        config.clone(),
        secrets_service.clone(),
        secrets_rx,
        peer_registry.clone(),
    )
    .await?;

    network.start(shutdown_tx.subscribe()).await?;

    let http_mounts = Arc::new(RwLock::new(HashMap::<String, String>::new()));
    let work_order_orchestrator =
        crate::services::work_order_orchestrator::WorkOrderOrchestrator::new(
            enclave_client.clone(),
            secrets_service.clone(),
            runner_service.clone(),
            policy_manager.clone(),
            gateway_manager.clone(),
            scheduler_handle.clone(),
            Arc::new(config.clone()),
            http_mounts.clone(),
            daemon_event_tx.clone(),
            shutdown_rx.resubscribe(),
        );

    {
        let grpc_config = config.grpc.clone();
        let secrets_service_clone = secrets_service.clone();
        let work_order_orchestrator_for_grpc = work_order_orchestrator.clone();
        let enclave_client_for_grpc_server = enclave_client.clone();
        let vm_registry_for_grpc = vm_registry.clone();
        tokio::spawn(async move {
            if let Err(e) = grpc::start_grpc_server(
                &grpc_config,
                secrets_service_clone,
                work_order_orchestrator_for_grpc,
                enclave_client_for_grpc_server,
                vm_registry_for_grpc,
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
    let work_order_orchestrator_for_http = work_order_orchestrator.clone();
    let local_key_for_http = local_key.clone();
    let vm_registry_for_http = vm_registry.clone();
    let peer_registry_for_http = peer_registry.clone();
    let secrets_service_for_http = secrets_service.clone();

    tokio::spawn(async move {
        if let Err(e) = start_http_server(
            &http_config,
            http_mounts_for_server,
            enclave_client_for_http,
            work_order_orchestrator_for_http,
            local_key_for_http,
            vm_registry_for_http,
            peer_registry_for_http,
            secrets_service_for_http,
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
        let mut ticker = interval(Duration::from_millis(10)); // Send batch every 10ms or if full

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

    // Task to handle scheduled events
    let scheduler_handle_for_events = scheduler_handle.clone();
    let daemon_event_tx_for_scheduler = daemon_event_tx.clone();
    let mut shutdown_rx_for_scheduler = shutdown_tx.subscribe();
    tokio::spawn(async move {
        let mut event_rx = match scheduler_handle_for_events.take_event_receiver().await {
            Some(rx) => rx,
            None => {
                error!("Failed to get scheduler event receiver");
                return;
            }
        };

        loop {
            tokio::select! {
                biased;
                _ = shutdown_rx_for_scheduler.recv() => {
                    info!("Scheduled event handler shutting down.");
                    break;
                }
                event_opt = event_rx.recv() => {
                    if let Some(scheduled_event) = event_opt {
                        info!(
                            "Firing scheduled event for work_order {} handler {}",
                            scheduled_event.work_order_id, scheduled_event.handler
                        );

                        let scheduled_event_payload_proto = nxcc_interface::proto::interface::EventPayload {
                            payload: Some(nxcc_interface::proto::interface::event_payload::Payload::ScheduledEvent(
                                ()
                            )),
                        };

                        // Find the corresponding active work order to get the enclave_worker_id
                        let active_work_orders = work_order_orchestrator.active_work_orders.read().await;
                        if let Some(active_wo) = active_work_orders.get(&scheduled_event.work_order_id) {
                            let event_delivery = nxcc_interface::proto::enclave::EventDelivery {
                                worker_id: active_wo.enclave_worker_id.clone(),
                                handler_name: scheduled_event.handler,
                                event_payload: Some(scheduled_event_payload_proto),
                            };

                            if let Err(e) = daemon_event_tx_for_scheduler.send(event_delivery).await {
                                error!(
                                    "Failed to send scheduled event to daemon queue for work_order {}: {}",
                                    scheduled_event.work_order_id, e
                                );
                            }
                        } else {
                            warn!(
                                "Scheduled event fired for unknown work_order: {}",
                                scheduled_event.work_order_id
                            );
                        }
                    } else {
                        // Channel closed
                        break;
                    }
                }
            }
        }
    });

    tokio::signal::ctrl_c().await?;
    tracing::info!("Received Ctrl-C, shutting down...");
    let _ = shutdown_tx.send(());
    Ok(())
}
