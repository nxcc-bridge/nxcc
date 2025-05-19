use std::{collections::HashMap, sync::Arc};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use nxcc_interface::types::{
    ConsumerInfo, DsseEnvelope, SecretId, SecretRequest, WorkOrderPayload, WorkerBundle,
    WorkerEventKind, WorkerManifest,
};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::{
    config::Config, error::AppError, grpc::enclave_client::EnclaveClient, policy::PolicyManager,
    services::secrets::SecretsService,
};

struct ActiveWorkOrder {
    payload: WorkOrderPayload,
    enclave_worker_id: Option<String>,
    // status: WorkOrderStatus, // Future enhancement
}

pub struct WorkOrderOrchestrator {
    enclave_client: EnclaveClient,
    secrets_service: Arc<SecretsService>,
    policy_manager: Arc<PolicyManager>,
    config: Arc<Config>,
    active_work_orders: RwLock<HashMap<String, ActiveWorkOrder>>,
}

impl WorkOrderOrchestrator {
    pub fn new(
        enclave_client: EnclaveClient,
        secrets_service: Arc<SecretsService>,
        policy_manager: Arc<PolicyManager>,
        config: Arc<Config>,
    ) -> Arc<Self> {
        Arc::new(Self {
            enclave_client,
            secrets_service,
            policy_manager,
            config,
            active_work_orders: RwLock::new(HashMap::new()),
        })
    }

    pub async fn submit_work_order(
        self: Arc<Self>, // Changed to Arc<Self> to allow spawning tasks if needed later
        work_order_dsse_bytes: Vec<u8>,
    ) -> Result<(String, String), AppError> {
        // 1. Deserialize DsseEnvelope
        let dsse_envelope: DsseEnvelope =
            serde_json::from_slice(&work_order_dsse_bytes).map_err(|e| {
                AppError::Service(format!("Failed to parse WorkOrder DSSE envelope: {}", e))
            })?;

        // TODO: DSSE signature validation would happen here, likely by calling out to a policy.
        // For now, we assume it's valid if it parses.

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
        debug!("Fetched worker bundle for work order {}", wo_payload.id);

        // 5. Deserialize WorkerBundlePayload (payload() method handles DSSE_WORKER_BUNDLE_PAYLOAD_TYPE check)
        let actual_bundle_payload = actual_worker_bundle.payload();

        // 6. Check VM ID
        if actual_bundle_payload.vm != self.config.enclave.default_vm_id {
            let msg = format!(
                "Work order {} requests VM '{}', but only default VM '{}' is supported.",
                wo_payload.id, actual_bundle_payload.vm, self.config.enclave.default_vm_id
            );
            error!("{}", msg);
            return Err(AppError::Service(msg));
        }

        // 7. Self-Authorization for secrets via policy execution
        info!(
            "Executing policies for self-authorization for work order {}",
            wo_payload.id
        );
        let daemon_env_report_for_self_auth =
            self.secrets_service.get_own_env_report(vec![]).await?;
        let worker_consumer_info_for_self_auth = ConsumerInfo {
            bundle_hash: actual_worker_bundle.hash_signed_payload(),
            signature: actual_worker_bundle.get_dsse_signature(),
        };

        for (secret_id, _name) in &worker_manifest.identities {
            let policy_package = self.policy_manager.get_policy(secret_id).await?;

            let policy_manifest_bytes =
                serde_json::to_vec(&policy_package.manifest).map_err(|e| {
                    AppError::Internal(format!("Failed to serialize policy manifest: {}", e))
                })?;
            let policy_bundle_bytes = policy_package.bundle.0.clone();
            let policy_worker_instance_id = self
                .enclave_client
                .run_worker(
                    self.config.enclave.default_vm_id.clone(),
                    policy_manifest_bytes,
                    policy_bundle_bytes,
                )
                .await
                .map_err(|e| {
                    AppError::Service(format!(
                        "Failed to run policy worker for self-auth {}: {}",
                        secret_id.identity_id, e
                    ))
                })?;

            let policy_request = nxcc_interface::types::PolicyExecutionRequest {
                secret_ids: vec![secret_id.clone()],
                consumer: worker_consumer_info_for_self_auth.clone(),
                env_report: daemon_env_report_for_self_auth.clone(),
            };

            let satisfied_contexts = self
                .enclave_client
                .execute_policy(policy_worker_instance_id.clone(), vec![policy_request])
                .await
                .map_err(|e| {
                    AppError::Service(format!(
                        "Failed to execute policy for self-auth {}: {}",
                        secret_id.identity_id, e
                    ))
                })?;

            if let Err(e) = self
                .enclave_client
                .terminate_worker(policy_worker_instance_id.clone())
                .await
            {
                warn!(
                    "Failed to terminate policy worker {} for self-auth: {}",
                    policy_worker_instance_id, e
                );
            }

            if satisfied_contexts.is_empty() {
                let msg = format!(
                    "Self-authorization policy denied for secret {:?} for work order {}",
                    secret_id, wo_payload.id
                );
                error!("{}", msg);
                return Err(AppError::Service(msg));
            }
            debug!(
                "Self-authorization policy approved for secret {:?} for work order {}",
                secret_id, wo_payload.id
            );
        }
        debug!(
            "All secrets self-authorized via policy execution for work order {}",
            wo_payload.id
        );

        // 8. Ensure secrets are present in the enclave
        let needed_secret_ids_with_names = worker_manifest.identities.clone();
        if !needed_secret_ids_with_names.is_empty() {
            info!(
                "Checking for {} secrets needed by work order {}",
                needed_secret_ids_with_names.len(),
                wo_payload.id
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
                        wo_payload.id, e
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
                        id, wo_payload.id
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
                    missing_secret_requests.len(),
                    wo_payload.id
                );
                let daemon_env_report = self.secrets_service.get_own_env_report(vec![]).await?;
                self.secrets_service
                    .clone() // Arc clone
                    .get_secrets(missing_secret_requests, daemon_env_report)
                    .await
                    .map_err(|e| {
                        AppError::Service(format!(
                            "Failed to get secrets for work order {}: {:?}",
                            wo_payload.id, e
                        ))
                    })?;
                debug!("Missing secrets processed for work order {}", wo_payload.id);
            }
        }

        // 9. Store active work order (basic for now)
        let active_wo = ActiveWorkOrder {
            payload: wo_payload.clone(),
            enclave_worker_id: None,
        };
        self.active_work_orders
            .write()
            .await
            .insert(wo_payload.id.clone(), active_wo);

        // 10. Run Worker
        info!(
            "Requesting enclave to run worker for work order {}",
            wo_payload.id
        );
        let manifest_bytes = serde_json::to_vec(&worker_manifest).map_err(|e| {
            AppError::Internal(format!("Failed to serialize worker manifest: {}", e))
        })?;
        let bundle_bytes = actual_worker_bundle.0.clone();

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
                    wo_payload.id, e
                ))
            })?;
        info!(
            "Enclave started worker {} for work order {}",
            enclave_worker_id, wo_payload.id
        );

        // Update active work order with enclave_worker_id
        if let Some(ao) = self
            .active_work_orders
            .write()
            .await
            .get_mut(&wo_payload.id)
        {
            ao.enclave_worker_id = Some(enclave_worker_id.clone());
        }

        // 11. Handle Launch Event
        for event in &wo_payload.events {
            if let WorkerEventKind::Launch = event.kind {
                info!(
                    "Executing Launch event for work order {}, worker {}",
                    wo_payload.id, enclave_worker_id
                );
                // Using an empty payload for launch, adjust if specific payload needed.
                self.enclave_client
                    .invoke_worker(enclave_worker_id.clone(), Vec::new()) // Empty payload for launch
                    .await
                    .map_err(|e| {
                        AppError::Service(format!(
                            "Launch event failed for work order {}: {}",
                            wo_payload.id, e
                        ))
                    })?;
                debug!("Launch event processed for work order {}", wo_payload.id);
            }
        }

        Ok((
            wo_payload.id,
            "Work order submitted and processed.".to_string(),
        ))
    }
}
