#[cfg(test)]
mod tests;

mod error;
mod execution;
mod management;
mod vm_client;

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use nxcc_interface::types::worker::events::EventPayload;
use serde::Serialize;
use tokio::sync::{RwLock, mpsc};
use tracing::{Instrument as _, debug, debug_span, error, info, info_span};

pub use self::error::RunnerError;
use self::vm_client::VmClient;
use crate::secrets::Secrets;

/// Manages attached VM clients and worker mappings.
pub struct RunnerService {
    /// Stores active VM clients, keyed by the vm_id assigned during attach.
    vms: Arc<RwLock<HashMap<String, VmClient>>>,
    /// Maps running worker_id (returned by VM) back to the vm_id it runs on.
    worker_map: Arc<RwLock<HashMap<String, String>>>,
    /// Maps dead worker_id to (vm_id, death_time) for TTL-based log access.
    dead_worker_map: Arc<RwLock<HashMap<String, (String, Instant)>>>,
    /// Shared secrets service for storing authorizations.
    secrets: Arc<Secrets>,
    /// Sender for the internal event queue: (worker_id, handler_name, serialized_vm_invocation_payload)
    event_tx: mpsc::UnboundedSender<(String, String, Vec<u8>)>,
}

/// Structure passed as payload to VmClient::invoke_worker for event delivery.
/// The VM (e.g., workerd VMM) will deserialize this and use the handler_name.
#[derive(Serialize)] // Keep Serialize
pub(crate) struct VmEventInvocation<'a> {
    // Owned version
    pub(crate) handler: String,
    #[serde(borrow)]
    pub(crate) event_payload: EventPayload<'a>,
}

impl RunnerService {
    pub fn new(secrets: Arc<Secrets>) -> Self {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<(String, String, Vec<u8>)>();

        let vms_clone = Arc::new(RwLock::new(HashMap::<String, VmClient>::new()));
        let worker_map_clone = Arc::new(RwLock::new(HashMap::<String, String>::new()));
        let dead_worker_map_clone =
            Arc::new(RwLock::new(HashMap::<String, (String, Instant)>::new()));

        let vms_for_task = vms_clone.clone();
        let worker_map_for_task = worker_map_clone.clone();

        // Start cleanup task for expired dead worker mappings
        let dead_worker_map_for_cleanup = dead_worker_map_clone.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;

                let mut dead_map = dead_worker_map_for_cleanup.write().await;
                let now = Instant::now();
                let initial_count = dead_map.len();

                dead_map.retain(|worker_id, (_vm_id, death_time)| {
                    let should_retain = now.duration_since(*death_time) < Duration::from_secs(300); // 5 minutes
                    if !should_retain {
                        debug!("Cleaning up expired dead worker mapping: {}", worker_id);
                    }
                    should_retain
                });

                let cleaned_count = initial_count - dead_map.len();
                if cleaned_count > 0 {
                    debug!("Cleaned up {} expired dead worker mappings", cleaned_count);
                }
            }
        });

        tokio::spawn(
            async move {
                info!("Enclave event processing task started.");

                loop {
                    // Span specifically for the receive operation to track blocking time
                    let receive_result = async { event_rx.recv().await }
                        .instrument(debug_span!("event_receive"))
                        .await;

                    let Some((worker_id, handler_name, vm_invocation_payload_bytes)) =
                        receive_result
                    else {
                        break;
                    };

                    // Span for processing each event
                    async {
                        debug!(
                            "Processing event for worker_id: {}, handler: {}, payload_size: {}",
                            worker_id,
                            handler_name,
                            vm_invocation_payload_bytes.len()
                        );

                        let (vm_id, client_clone) = async {
                            let worker_map_guard = worker_map_for_task.read().await;
                            let vm_id = worker_map_guard.get(&worker_id).cloned();
                            if let Some(ref vm_id_str) = vm_id {
                                let vms_guard = vms_for_task.read().await;
                                let client_clone = vms_guard.get(vm_id_str).cloned();
                                (vm_id, client_clone)
                            } else {
                                (vm_id, None)
                            }
                        }
                        .instrument(debug_span!("lookup_vm_and_client"))
                        .await;

                        if let (Some(vm_id), Some(mut client)) = (vm_id, client_clone) {
                            let invoke_result = client
                                .invoke_worker(
                                    worker_id.clone(),
                                    handler_name.clone(),
                                    vm_invocation_payload_bytes,
                                )
                                .instrument(debug_span!(
                                    "invoke_worker",
                                    worker_id = %worker_id,
                                    handler = %handler_name
                                ))
                                .await;

                            match invoke_result {
                                Ok(response) => {
                                    debug!(
                                        "Worker {} handler {} invocation successful, response_size: {}",
                                        worker_id,
                                        handler_name,
                                        response.len()
                                    );
                                    // TODO: Handle worker response if necessary
                                }
                                Err(e) => {
                                    error!(
                                        "Failed to invoke worker {} handler {} in VM {}: {}",
                                        worker_id, handler_name, vm_id, e
                                    );
                                }
                            }
                        } else {
                            error!(
                                "Worker {} not found in map during event processing.",
                                worker_id
                            );
                        }
                    }
                    .instrument(
                        info_span!("process_event", worker_id = %worker_id, handler = %handler_name),
                    )
                    .await;
                }

                info!("Enclave event processing task stopped.");
            }
            .instrument(info_span!("enclave_event_processing_task")),
        );

        Self {
            vms: vms_clone,
            worker_map: worker_map_clone,
            dead_worker_map: dead_worker_map_clone,
            secrets,
            event_tx,
        }
    }
}

#[cfg(test)]
impl RunnerService {
    pub(crate) async fn set_worker_vm_mapping(&self, worker_id: String, vm_id: String) {
        self.worker_map.write().await.insert(worker_id, vm_id);
    }
}
