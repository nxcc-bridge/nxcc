use std::{collections::HashMap, sync::Arc};

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD},
};
use nxcc_interface::types::{
    ConsumerInfo,
    DSSE_WORK_ORDER_PAYLOAD_TYPE,
    DsseEnvelope,
    SecretId,
    SecretRequest,
    Web3Log, // Added Web3Log for potential Launch event structure
    WorkOrderPayload,
    WorkerBundle,
    WorkerEvent,
    WorkerEventKind,
    WorkerManifest,
};
use sha2::{Digest, Sha256};
use tokio::sync::{RwLock, broadcast};
use tracing::{debug, error, info, warn};

use crate::{
    config::Config,
    error::AppError,
    grpc::enclave_client::EnclaveClient,
    policy::PolicyManager,
    // Assuming daemon_event_tx is passed in main.rs or similar
    services::secrets::SecretsService,
    web3::{gateways::GatewayManager, listener::start_web3_event_listener},
};

struct ActiveWorkOrder {
    payload: WorkOrderPayload,
    enclave_worker_id: String, // Should be set once worker is running
    dsse_hash_b64url: String,  // The unique ID for this work order instance
                               // status: WorkOrderStatus, // Future enhancement
}

/// Manages the lifecycle of work orders, including running workers and setting up event listeners.
pub struct WorkOrderOrchestrator {
    enclave_client: EnclaveClient,
    secrets_service: Arc<SecretsService>,
    runner_service: Arc<super::runner::RunnerService>, // Added runner_service
    policy_manager: Arc<PolicyManager>,
    gateway_manager: Arc<GatewayManager>,
    config: Arc<Config>,
    active_work_orders: RwLock<HashMap<String, ActiveWorkOrder>>,
    /// Maps mount path segment (hash of work order) to enclave_worker_id for HTTP routing.
    http_mounts: Arc<RwLock<HashMap<String, String>>>,
    daemon_event_tx: tokio::sync::mpsc::Sender<nxcc_interface::proto::enclave::EventDelivery>,
    daemon_shutdown_rx: broadcast::Receiver<()>, // Used for web3 listeners
}

impl WorkOrderOrchestrator {
    pub fn new(
        enclave_client: EnclaveClient,
        secrets_service: Arc<SecretsService>,
        runner_service: Arc<super::runner::RunnerService>,
        policy_manager: Arc<PolicyManager>,
        gateway_manager: Arc<GatewayManager>,
        config: Arc<Config>,
        http_mounts: Arc<RwLock<HashMap<String, String>>>,
        daemon_event_tx: tokio::sync::mpsc::Sender<nxcc_interface::proto::enclave::EventDelivery>,
        daemon_shutdown_rx: broadcast::Receiver<()>,
    ) -> Arc<Self> {
        Arc::new(Self {
            enclave_client,
            secrets_service,
            runner_service,
            policy_manager,
            gateway_manager,
            config,
            active_work_orders: RwLock::new(HashMap::new()),
            http_mounts,
            daemon_event_tx,
            daemon_shutdown_rx,
        })
    }

    pub async fn submit_work_order(
        self: Arc<Self>,
        work_order_dsse_bytes: Vec<u8>,
    ) -> Result<(String, String), AppError> {
        // Calculate SHA256 hash of the raw DSSE envelope bytes for unique ID and mount path.
        let mut hasher = Sha256::new();
        hasher.update(&work_order_dsse_bytes);
        let work_order_hash_bytes = hasher.finalize();
        let work_order_hash_b64url = URL_SAFE_NO_PAD.encode(work_order_hash_bytes);

        // 1. Deserialize DsseEnvelope
        let dsse_envelope: DsseEnvelope =
            serde_json::from_slice(&work_order_dsse_bytes).map_err(|e| {
                AppError::Service(format!("Failed to parse WorkOrder DSSE envelope: {}", e))
            })?;

        // TODO: DSSE signature validation would happen here, likely by calling out to a policy.

        // Check for duplicate work order submission
        if self
            .active_work_orders
            .read()
            .await
            .contains_key(&work_order_hash_b64url)
        {
            let msg = format!(
                "Work order with hash {} already exists and is active.",
                work_order_hash_b64url
            );
            warn!("{}", msg);
            // It's not an error per se, more like a duplicate submission.
            // The gRPC handler expects Ok(SubmitWorkOrderResponse) even for app-level "failures".
            return Ok((
                work_order_hash_b64url,
                "Work order already active.".to_string(),
            ));
        }
        // For now, we assume it's valid if it parses.
        if dsse_envelope.payload_type != DSSE_WORK_ORDER_PAYLOAD_TYPE {
            return Err(AppError::Service(format!(
                "Invalid WorkOrder DSSE payloadType: expected '{}', got '{}'",
                DSSE_WORK_ORDER_PAYLOAD_TYPE, dsse_envelope.payload_type
            )));
        }

        debug!("WorkOrder DSSE payload type validated.");

        // 2. Decode base64 payload
        let payload_bytes = BASE64_STANDARD
            .decode(&dsse_envelope.payload)
            .map_err(|e| {
                AppError::Service(format!("Failed to base64 decode WorkOrder payload: {}", e))
            })?;

        // 3. Deserialize WorkOrderPayload
        let wo_payload: WorkOrderPayload = serde_json::from_slice(&payload_bytes)
            .map_err(|e| AppError::Service(format!("Failed to parse WorkOrderPayload: {}", e)))?;
        info!("Received work order: {}", wo_payload.id);

        let worker_manifest = wo_payload.worker.clone();

        // 4. Fetch the WorkerBundle for the work order
        let actual_worker_bundle = self
            .policy_manager
            .fetch_worker_bundle(
                &worker_manifest.bundle,
                "work-order-context", // manifest_url_for_context
                &SecretId::default(), // secret_id_for_log (using Default impl)
            )
            .await?;
        debug!(
            "Fetched worker bundle for work order (hash: {})",
            work_order_hash_b64url
        );

        // 5. Deserialize WorkerBundlePayload (payload() method handles DSSE_WORKER_BUNDLE_PAYLOAD_TYPE check)
        let actual_bundle_payload = actual_worker_bundle.payload();

        // 6. Check VM ID
        if actual_bundle_payload.vm != self.config.enclave.default_vm_id {
            let msg = format!(
                "Work order (hash: {}) requests VM '{}', but only default VM '{}' is supported.",
                work_order_hash_b64url, actual_bundle_payload.vm, self.config.enclave.default_vm_id
            );
            error!("{}", msg);
            return Err(AppError::Validation(msg));
        }

        // 7. Self-Authorization for secrets via policy execution
        info!(
            "Executing policies for self-authorization for work order {}",
            wo_payload.id
        );
        let daemon_env_report_for_self_auth =
            self.secrets_service.get_own_env_report(vec![]).await?; // TODO: This should be the enclave's report, not daemon's
        let worker_consumer_info_for_self_auth = ConsumerInfo {
            bundle_hash: actual_worker_bundle.hash_signed_payload(),
            signature: actual_worker_bundle.get_dsse_signature(),
        };

        for (secret_id, _name) in &worker_manifest.identities {
            info!(
                "Performing self-authorization for secret {:?} for work order {}",
                secret_id, work_order_hash_b64url
            );
            let policy_package = self.policy_manager.get_policy(secret_id).await?;

            let authorized = self
                .runner_service
                .check_policy_for_env(
                    policy_package,
                    &daemon_env_report_for_self_auth,
                    secret_id,
                    &worker_consumer_info_for_self_auth,
                )
                .await
                .map_err(|e| {
                    AppError::Service(format!(
                        "Policy execution for self-auth of secret {:?} failed: {}",
                        secret_id.identity_id, e
                    ))
                })?;
            if !authorized {
                let msg = format!(
                    "Self-authorization policy denied for secret {:?} for work order (hash: {})",
                    secret_id, work_order_hash_b64url
                );
                error!("{}", msg);
                return Err(AppError::Authorization(msg));
            }
            debug!(
                "Self-authorization policy approved for secret {:?} for work order (hash: {})",
                secret_id, work_order_hash_b64url
            );
        }
        debug!(
            "All secrets self-authorized via policy execution for work order (hash: {})",
            work_order_hash_b64url
        );

        // 8. Ensure secrets are present in the enclave
        let needed_secret_ids_with_names = worker_manifest.identities.clone();
        if !needed_secret_ids_with_names.is_empty() {
            info!(
                "Checking for {} secrets needed by work order {}",
                needed_secret_ids_with_names.len(), // Using payload.id for logging clarity
                work_order_hash_b64url
            );
            let needed_ids: Vec<_> = needed_secret_ids_with_names
                .iter()
                .map(|(id, _)| id.clone())
                .collect();
            let statuses = self
                .enclave_client
                .check_secrets(needed_ids.clone())
                .await
                .map_err(|e| {
                    AppError::Service(format!(
                        "Failed to check secrets for work order {}: {}",
                        work_order_hash_b64url, e
                    ))
                })?;

            let current_time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let mut missing_secret_requests = HashMap::new();
            let worker_consumer_info = ConsumerInfo {
                bundle_hash: actual_worker_bundle.hash_signed_payload(),
                signature: actual_worker_bundle.get_dsse_signature(),
            };

            for (id, found, expiry) in statuses {
                if !found || (expiry != 0 && expiry <= current_time) {
                    debug!(
                        "Secret {:?} for work order {} is missing or expired. Will attempt to \
                         fetch/generate.",
                        id, work_order_hash_b64url
                    );
                    missing_secret_requests.insert(
                        id.clone(),
                        vec![SecretRequest {
                            secret_id: id,
                            consumer: worker_consumer_info.clone(),
                        }],
                    );
                }
            }

            if !missing_secret_requests.is_empty() {
                info!(
                    "Fetching/generating {} missing secrets for work order {}",
                    missing_secret_requests.len(), // Using payload.id for logging clarity
                    work_order_hash_b64url
                );
                let daemon_env_report = self.secrets_service.get_own_env_report(vec![]).await?;
                self.secrets_service
                    .clone() // Arc clone
                    .get_secrets(missing_secret_requests, daemon_env_report)
                    .await
                    .map_err(|e| {
                        AppError::Service(format!(
                            "Failed to get secrets for work order {}: {:?}",
                            work_order_hash_b64url, e
                        ))
                    })?;
                debug!(
                    "Missing secrets processed for work order (hash: {})",
                    work_order_hash_b64url
                );
            }
        }

        // 9. Run Worker
        info!(
            "Requesting enclave to run worker for work order (hash: {})",
            work_order_hash_b64url
        );
        let manifest_bytes = serde_json::to_vec(&worker_manifest).map_err(|e| {
            AppError::Internal(format!("Failed to serialize worker manifest: {}", e))
        })?;
        let bundle_bytes = actual_worker_bundle.0.clone(); // DSSE envelope bytes

        let enclave_worker_id = self
            .enclave_client
            .run_worker(
                self.config.enclave.default_vm_id.clone(),
                manifest_bytes,
                bundle_bytes,
            )
            .await
            .map_err(|e| {
                AppError::Service(format!(
                    "Failed to run worker in enclave for {}: {}",
                    work_order_hash_b64url, e
                ))
            })?;
        info!(
            "Enclave started worker {} for work order (hash: {})",
            enclave_worker_id, work_order_hash_b64url
        );

        // 10. Store active work order
        let active_wo = ActiveWorkOrder {
            payload: wo_payload.clone(), // Keep original payload for reference
            enclave_worker_id: enclave_worker_id.clone(),
            dsse_hash_b64url: work_order_hash_b64url.clone(),
        };
        self.active_work_orders // Keyed by the unique hash
            .write()
            .await
            .insert(work_order_hash_b64url.clone(), active_wo);

        // 11. Handle Event Subscriptions (Launch, Web3, HTTP mounting)
        let enclave_worker_id_clone_for_listeners = enclave_worker_id.clone();
        let wo_id_clone_for_listeners = wo_payload.id.clone();

        let mut launch_event_defined = false;
        let mut launch_event_queued_successfully = false;
        let mut web3_event_configs = Vec::new();

        let mut http_requested = false;
        for event_config in &wo_payload.events {
            match &event_config.kind {
                WorkerEventKind::Launch => {
                    launch_event_defined = true;
                    info!("Queueing Launch event for worker {}", enclave_worker_id);
                    let launch_event_payload_proto = nxcc_interface::proto::interface::EventPayload {
                        payload: Some(nxcc_interface::proto::interface::event_payload::Payload::LaunchEvent(
                                         ()
                        )),
                    };
                    let event_delivery = nxcc_interface::proto::enclave::EventDelivery {
                        worker_id: enclave_worker_id.clone(),
                        handler_name: event_config.handler.clone(),
                        event_payload: Some(launch_event_payload_proto),
                    };
                    // Send to the daemon's central event queue
                    if let Err(e) = self.daemon_event_tx.send(event_delivery).await {
                        error!(
                            "Failed to send Launch event to daemon queue for work_order {}: {}",
                            work_order_hash_b64url, e
                        );
                        // launch_event_queued_successfully remains false
                    } else {
                        launch_event_queued_successfully = true;
                    }
                }
                WorkerEventKind::Web3Event(web3_config) => {
                    web3_event_configs.push(web3_config.clone());
                }
                WorkerEventKind::HttpRequestTrigger => {
                    http_requested = true;
                    info!(
                        "Work order (hash: {}) requests HTTP capability.",
                        work_order_hash_b64url
                    );
                }
                e => {
                    warn!("Received unknown worker event: {e:?}");
                }
            }
        }

        // Determine if Web3 listeners should be set up.
        // If a Launch event was defined, it must have been successfully queued.
        // If no Launch event was defined, proceed directly.
        let should_setup_web3_listeners = if launch_event_defined {
            launch_event_queued_successfully
        } else {
            true // No launch event, so proceed with Web3 listeners
        };

        if http_requested {
            // Mount the worker for HTTP requests
            // The key for http_mounts is the base64url encoded hash of the work order DSSE bytes
            self.http_mounts
                .write()
                .await
                .insert(work_order_hash_b64url.clone(), enclave_worker_id.clone());
            info!(
                "Mounted HTTP worker at segment: {} (enclave_worker_id: {})",
                work_order_hash_b64url, enclave_worker_id
            );
        }

        if should_setup_web3_listeners {
            for web3_config in web3_event_configs {
                // Find the original event_config to get the handler name
                let original_event_config = wo_payload
                    .events
                    .iter()
                    .find(|e| {
                        if let WorkerEventKind::Web3Event(cfg) = &e.kind {
                            // This comparison might need to be more robust if Web3Event can have identical configs but different handlers
                            cfg == &web3_config
                        } else {
                            false
                        }
                    })
                    .ok_or_else(|| {
                        AppError::Internal("Could not find original Web3Event config".to_string())
                    })?;

                info!(
                    "Work order {} requests Web3 event listener for chain {}: Address: {:?}, \
                     Topics: {:?}, Handler: {} (for work_order_hash: {})",
                    wo_id_clone_for_listeners, // payload.id for logging
                    web3_config.chain,
                    web3_config
                        .address
                        .iter()
                        .map(|a| format!("{a:#x}"))
                        .collect::<Vec<_>>(),
                    web3_config.topics,
                    original_event_config.handler,
                    work_order_hash_b64url // actual unique ID
                );

                let wo_id_listener = wo_id_clone_for_listeners.clone();
                let enclave_worker_id_listener = enclave_worker_id_clone_for_listeners.clone();
                let handler_name_listener = original_event_config.handler.clone();
                let gateway_manager_clone = self.gateway_manager.clone();
                let shutdown_rx_clone = self.daemon_shutdown_rx.resubscribe();
                let daemon_event_tx_clone = self.daemon_event_tx.clone();

                let wo_hash_b64 = work_order_hash_b64url.clone();
                tokio::spawn(async move {
                    start_web3_event_listener(
                        wo_hash_b64, // Use the unique hash for listener identification
                        enclave_worker_id_listener,
                        handler_name_listener,
                        web3_config,
                        gateway_manager_clone,
                        shutdown_rx_clone,
                        daemon_event_tx_clone,
                    )
                    .await;
                });
            }
        } else {
            // This implies a Launch event was defined but failed to be queued.
            warn!(
                "Skipping Web3 event listener setup for work order {} due to Launch event failure.",
                work_order_hash_b64url
            );
        }

        Ok((
            work_order_hash_b64url, // Return the unique hash as the ID
            format!("Work order submitted and processed. ID: {}.", wo_payload.id),
        ))
    }
}
