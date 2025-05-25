use std::{sync::Arc, time::Duration};

use alloy_primitives::B256;
use alloy_provider::Provider;
use alloy_rpc_types::{Filter, FilterSet, Log, Topic};
use futures_util::StreamExt;
use nxcc_interface::{
    proto::enclave as enclave_proto,
    types::{EventPayload, Web3Event as Web3EventConfig, Web3Log as RustWeb3Log},
};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::web3::gateways::GatewayManager;

const RECONNECT_DELAY: Duration = Duration::from_secs(5);

pub fn web3_event_config_to_alloy_filter(config: &Web3EventConfig) -> Filter {
    let address_filter_set = FilterSet::from_iter(config.address.clone());

    let mut topics_array: [Topic; 4] = Default::default();

    for (i, topic_values) in config.topics.iter().enumerate() {
        if i < 4 {
            if topic_values.is_empty() {
                topics_array[i] = FilterSet::default();
            } else {
                topics_array[i] = FilterSet::from_iter(topic_values.clone());
            }
        } else {
            // This warning will lack work_order_id, but it's better than nothing.
            // The caller can log the full config for better context if needed.
            warn!("Web3EventConfig has more than 4 topic groups, ignoring extras.");
            break;
        }
    }

    Filter {
        block_option: Default::default(),
        address: address_filter_set,
        topics: topics_array,
    }
}

pub async fn start_web3_event_listener(
    work_order_id: String,
    enclave_worker_id: String,
    handler_name: String,
    config: Web3EventConfig,
    gateway_manager: Arc<GatewayManager>,
    mut shutdown_rx: broadcast::Receiver<()>,
    daemon_event_tx: tokio::sync::mpsc::Sender<enclave_proto::EventDelivery>,
) {
    info!(
        "Starting Web3 event listener for work_order_id: {}, enclave_worker_id: {}, config: {:?}",
        work_order_id,
        enclave_worker_id,
        config // Use enclave_worker_id for logging
    );

    let filter = web3_event_config_to_alloy_filter(&config);
    debug!(
        "Constructed Alloy Filter for work_order_id: {}: {:?}",
        work_order_id, filter
    );

    loop {
        let provider = match gateway_manager.get_provider(config.chain).await {
            Ok(p) => p,
            Err(e) => {
                error!(
                    "Failed to get provider for work_order_id: {}, chain_id {}: {}. Retrying \
                     after delay.",
                    work_order_id, config.chain, e
                );
                tokio::select! { biased; _ = tokio::time::sleep(RECONNECT_DELAY) => continue, Ok(()) = shutdown_rx.recv() => { info!("Shutdown (work_order_id: {}). Terminating.", work_order_id); break; } }
            }
        };

        info!(
            "Attempting to subscribe to logs for work_order_id: {} on chain_id: {}",
            work_order_id, config.chain
        );
        match provider.subscribe_logs(&filter).await {
            Ok(subscription) => {
                info!(
                    "Successfully subscribed to logs for work_order_id: {}",
                    work_order_id
                );
                let mut log_stream = subscription.into_stream();
                loop {
                    tokio::select! {
                        biased;
                        Ok(()) = shutdown_rx.recv() => {
                            info!("Shutdown during log streaming (work_order_id: {}). Terminating.", work_order_id);
                            return;
                        }
                        log_option = log_stream.next() => {
                            match log_option {
                                Some(log) => {
                                    info!("Received log for work_order_id: {}: {:?}", work_order_id, log);
                                    let rust_web3_log = RustWeb3Log::from(log);
                                    let event_payload_proto = nxcc_interface::proto::interface::EventPayload {
                                        payload: Some(nxcc_interface::proto::interface::event_payload::Payload::Web3Log(rust_web3_log.into())),
                                    };
                                    let event_delivery = enclave_proto::EventDelivery {
                                        worker_id: enclave_worker_id.clone(),
                                        event_payload: Some(event_payload_proto),
                                        handler_name: handler_name.clone()
                                    };
                                    if let Err(e) = daemon_event_tx.send(event_delivery).await {
                                        error!("Failed to send Web3 event to daemon queue for work_order_id {}: {}", work_order_id, e);
                                        // If sending to queue fails, this listener might be orphaned or the daemon is shutting down.
                                        // Consider breaking the loop or specific error handling.
                                    }
                                }
                                None => {
                                    warn!("Log stream ended for work_order_id: {}. Will attempt to reconnect.", work_order_id);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                error!(
                    "Failed to subscribe to logs for work_order_id: {}: {}. Retrying after delay.",
                    work_order_id, e
                );
                tokio::select! { biased; _ = tokio::time::sleep(RECONNECT_DELAY) => continue, Ok(()) = shutdown_rx.recv() => { info!("Shutdown while waiting to resubscribe (work_order_id: {}). Terminating.", work_order_id); break; } }
            }
        }
        // If log_stream ended (None) or subscription failed and we didn't 'continue'
        warn!(
            "Log stream for work_order_id: {} requires reconnection. Delaying.",
            work_order_id
        );
        tokio::select! { biased; _ = tokio::time::sleep(RECONNECT_DELAY) => {}, Ok(()) = shutdown_rx.recv() => { info!("Shutdown while delaying reconnect (work_order_id: {}). Terminating.", work_order_id); break; } }
    }
    info!(
        "Web3 event listener stopped for work_order_id: {}, worker_id: {}",
        work_order_id, enclave_worker_id
    );
}
