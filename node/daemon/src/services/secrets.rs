use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use futures::channel::mpsc;
use interface::{
    policy::{PolicyBundle, PolicyManifest},
    types::{AttestationReport, EnvReport, SecretId, SecretRequest, SecretsBox},
};
use tokio::sync::{Mutex, RwLock, oneshot};
use tracing::{debug, error, info, warn};

use crate::{
    error::AppError, grpc::enclave_client::EnclaveClient, network::SecretsMessage,
    policy::ManifestChecker, web3::gateways::GatewayManager,
};

// Timeout for waiting for P2P responses
const P2P_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
// Threshold for number of valid responses needed per secret (currently request-level)
const RESPONSE_THRESHOLD: usize = 1;

pub struct SecretsService {
    p2p_secrets_sender: mpsc::Sender<SecretsMessage>,
    enclave_client: EnclaveClient,
    gateway_manager: GatewayManager,
    manifest_checker: ManifestChecker,
    policy_cache: RwLock<HashMap<SecretId, PolicyBundle>>,
    pending: Mutex<HashMap<u64, PendingRequest>>,
    request_counter: AtomicU64,
}

struct PendingRequest {
    // All secret IDs originally requested that were missing locally
    requested_ids: HashSet<SecretId>,
    // Bundles received from peers
    collected_bundles: Vec<(SecretsBox, AttestationReport)>,
    // Count of responses received
    response_count: usize,
    // Threshold for number of responses needed for the request
    threshold: usize,
    // Channel to notify the original caller
    responder: oneshot::Sender<Result<(), AppError>>,
}

impl SecretsService {
    pub fn new(
        p2p_secrets_sender: mpsc::Sender<SecretsMessage>,
        enclave_client: EnclaveClient,
        gateway_manager: GatewayManager,
        manifest_checker: ManifestChecker,
    ) -> Arc<Self> {
        Arc::new(Self {
            p2p_secrets_sender,
            enclave_client,
            gateway_manager,
            manifest_checker,
            policy_cache: RwLock::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            request_counter: AtomicU64::new(0),
        })
    }

    /// Fetches secrets, ensuring all requested secrets (local or remote) are available.
    /// Returns Ok(()) on success, Err otherwise. The actual secrets box for the caller
    /// needs to be retrieved separately via the gRPC handler after this returns Ok.
    pub async fn get_secrets(
        self: Arc<Self>,
        secret_requests: HashMap<SecretId, Vec<SecretRequest>>,
        env_report: EnvReport,
    ) -> Result<(), AppError> {
        info!(
            "get_secrets called with {} unique secret IDs",
            secret_requests.len()
        );

        // 1. Determine local/missing secrets
        let (local_ids, missing_requests) = self.check_local(&secret_requests).await?;
        debug!(
            "Local secrets: {}, Missing secrets: {}",
            local_ids.len(),
            missing_requests.len()
        );

        if missing_requests.is_empty() {
            info!("All requested secrets are available locally.");
            // All secrets are local, nothing more to do here.
            // The gRPC handler will call enclave's get_secrets later.
            return Ok(());
        }

        // 2. Fetch and cache policies for missing secrets
        // 3. Check policy manifests
        let mut policies = HashMap::new();
        let missing_ids: HashSet<SecretId> = missing_requests.keys().cloned().collect();
        for secret_id in &missing_ids {
            match self.get_policy(secret_id).await {
                Ok(policy) => {
                    // Check manifest
                    if let Err(e) = self.manifest_checker.check_manifest(&policy.manifest) {
                        error!(
                            "Manifest check failed for policy of secret {:?}: {}",
                            secret_id, e
                        );
                        return Err(AppError::Service(format!(
                            "Policy manifest check failed for {:?}: {}",
                            secret_id, e
                        )));
                    }
                    debug!("Manifest check passed for policy of secret {:?}", secret_id);
                    policies.insert(secret_id.clone(), policy);
                }
                Err(e) => {
                    error!("Failed to get policy for secret {:?}: {}", secret_id, e);
                    return Err(e); // Propagate policy fetch/parse error
                }
            }
        }
        info!(
            "Successfully fetched and checked manifests for {} policies.",
            policies.len()
        );

        // 4. Request missing secrets from P2P network
        let request_id = self.request_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();

        {
            let mut guard = self.pending.lock().await;
            guard.insert(
                request_id,
                PendingRequest {
                    requested_ids: missing_ids.clone(), // Store only the IDs
                    collected_bundles: Vec::new(),
                    response_count: 0,
                    threshold: RESPONSE_THRESHOLD, // Use constant for now
                    responder: tx,
                },
            );
        }
        info!(
            "Created pending request {} for {} missing secrets.",
            request_id,
            missing_ids.len()
        );

        // Spawn timeout task
        let self_clone = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(P2P_RESPONSE_TIMEOUT).await;
            info!("Timeout reached for request {}", request_id);
            if let Err(e) = self_clone.timeout_pending_request(request_id).await {
                error!(
                    "Error during timeout finalization for request {}: {}",
                    request_id, e
                );
            }
        });

        // Publish request to P2P network
        self.p2p_secrets_sender
            .clone()
            .try_send(SecretsMessage::PublishSecretsRequest {
                request_id,
                secret_requests: BTreeMap::from_iter(missing_requests.clone().into_iter()),
                env_report: env_report.clone(),
            })
            .map_err(|e| AppError::Service(format!("Failed to publish secrets request: {e}")))?;

        info!(
            "Published P2P secrets request {} for {} secrets.",
            request_id,
            missing_ids.len()
        );

        // 5. Wait for P2P process completion (threshold met or timeout)
        match rx.await {
            Ok(Ok(())) => {
                info!("Successfully processed request {}", request_id);
                Ok(())
            }
            Ok(Err(e)) => {
                error!("Error processing request {}: {}", request_id, e);
                Err(e)
            }
            Err(e) => {
                error!("Responder channel closed for request {}: {}", request_id, e);
                Err(AppError::Service(format!(
                    "Internal channel error for request {}: {}",
                    request_id, e
                )))
            }
        }
    }

    /// Fetches policy from cache or network.
    async fn get_policy(&self, secret_id: &SecretId) -> Result<PolicyBundle, AppError> {
        // Check cache first
        if let Some(policy) = self.policy_cache.read().await.get(secret_id) {
            debug!("Policy cache hit for secret {:?}", secret_id);
            return Ok(policy.clone());
        }
        debug!("Policy cache miss for secret {:?}", secret_id);

        // Fetch from network
        let policy_url = self
            .gateway_manager
            .get_policy_url(
                secret_id.chain_id,
                secret_id.identity_address,
                secret_id.identity_id,
            )
            .await?;

        info!(
            "Fetching policy for secret {:?} from URL: {}",
            secret_id, policy_url
        );

        // Handle mock URLs for testing/dev
        if policy_url.starts_with("mock://") {
            warn!("Using mock policy for secret {:?}", secret_id);
            // Create a dummy policy bundle
            let mock_manifest = PolicyManifest {
                version: "1.0".to_string(),
                name: format!("Mock Policy for {:?}", secret_id),
                description: "A dummy policy for testing".to_string(),
                allowed_consumers: vec![], // Adjust as needed
                execution_constraints: interface::policy::ExecutionConstraints {
                    max_memory_mb: 128,
                    max_execution_time_ms: 1000,
                    allowed_network_calls: false,
                },
            };
            let mock_policy = PolicyBundle {
                manifest: mock_manifest,
                executable: b"mock_executable_code".to_vec(),
            };
            // Cache the mock policy
            self.policy_cache
                .write()
                .await
                .insert(secret_id.clone(), mock_policy.clone());
            return Ok(mock_policy);
        }

        // Fetch the actual policy content
        let client = reqwest::Client::new();
        let response = client.get(&policy_url).send().await.map_err(|e| {
            AppError::Network(format!("Failed to fetch policy from {}: {}", policy_url, e))
        })?;

        if !response.status().is_success() {
            return Err(AppError::Network(format!(
                "Failed to fetch policy from {}: Status {}",
                policy_url,
                response.status()
            )));
        }

        let policy: PolicyBundle = response.json().await.map_err(|e| {
            AppError::Service(format!(
                "Failed to parse policy JSON from {}: {}",
                policy_url, e
            ))
        })?;

        // Cache the fetched policy
        self.policy_cache
            .write()
            .await
            .insert(secret_id.clone(), policy.clone());
        info!("Successfully fetched and cached policy for {:?}", secret_id);

        Ok(policy)
    }

    /// Called when we receive a p2p secrets response.
    pub async fn handle_incoming_secret_batch_response(
        self: &Arc<Self>,
        request_id: u64,
        secrets_box: SecretsBox,
        attestation_report: AttestationReport,
    ) -> Result<(), AppError> {
        let mut lock = self.pending.lock().await;
        if let Some(p) = lock.get_mut(&request_id) {
            debug!(
                "Received response for pending request {}, current count: {}, threshold: {}",
                request_id, p.response_count, p.threshold
            );

            // 6. Run policy check against responder (TODO)
            // This requires:
            // - The policy for the secret(s) inside the box (we don't know which ones yet).
            // - The responder's EnvReport (we only have AttestationReport currently).
            // - Integration with the RunnerService/EnclaveClient runner part.
            // For now, we assume the response is valid if received.
            let is_valid_response = true; // Placeholder

            if is_valid_response {
                p.collected_bundles.push((secrets_box, attestation_report));
                p.response_count += 1;
                info!(
                    "Added valid response bundle to request {}. New count: {}",
                    request_id, p.response_count
                );

                // Check if threshold is met
                if p.response_count >= p.threshold {
                    info!(
                        "Threshold met for request {}. Triggering finalization.",
                        request_id
                    );
                    // Drop the lock before calling finalize_request to avoid deadlock
                    drop(lock);
                    // Use spawn to avoid blocking the network handler
                    let self_clone = Arc::clone(self);
                    tokio::spawn(async move {
                        if let Err(e) = self_clone.finalize_request(request_id).await {
                            error!(
                                "Error during threshold finalization for request {}: {}",
                                request_id, e
                            );
                        }
                    });
                }
            } else {
                warn!(
                    "Ignoring invalid response for request {} (policy check failed - simulated)",
                    request_id
                );
            }
        } else {
            debug!(
                "Received response for unknown or already finalized request {}",
                request_id
            );
        }
        Ok(())
    }

    /// Called when we receive a p2p secrets request. Return a SecretsBox if local secrets are found.
    pub async fn handle_incoming_secret_batch_request(
        &self,
        _request_id: u64,
        secret_requests: BTreeMap<SecretId, Vec<SecretRequest>>,
        env_report: EnvReport,
    ) -> SecretsBox {
        let found_ids = self.gather_local(&secret_requests).await;
        if found_ids.is_empty() {
            debug!("No local secrets found for incoming request.");
            return SecretsBox::new_empty();
        }
        debug!(
            "Found {} local secrets for incoming request.",
            found_ids.len()
        );

        // TODO: Policy check - Should we check if the *requester* (env_report) is allowed
        // by the policy to receive these secrets before sending? This might require
        // fetching policies here as well. For now, assume allowed if found locally.

        let att = env_report.attestation.clone();
        match self
            .enclave_client
            .get_secrets(found_ids, vec![], att)
            .await
        {
            Ok(sb) => {
                info!("Returning secrets box for incoming request.");
                sb
            }
            Err(e) => {
                error!(
                    "Failed to get secrets from enclave for incoming request: {}",
                    e
                );
                SecretsBox::new_empty()
            }
        }
    }

    /// Finalizes a request after timeout.
    pub async fn timeout_pending_request(self: Arc<Self>, request_id: u64) -> Result<(), AppError> {
        // Check if the request still exists (might have been finalized by threshold)
        if self.pending.lock().await.contains_key(&request_id) {
            info!("Finalizing request {} due to timeout.", request_id);
            self.finalize_request(request_id).await
        } else {
            debug!(
                "Timeout occurred for request {}, but it was already finalized.",
                request_id
            );
            Ok(())
        }
    }

    /// Processes collected bundles, puts them into the enclave, checks final status, and responds.
    async fn finalize_request(self: Arc<Self>, request_id: u64) -> Result<(), AppError> {
        let pending_req = {
            let mut lock = self.pending.lock().await;
            // Remove the request to prevent duplicate finalization
            lock.remove(&request_id)
        };

        if let Some(p) = pending_req {
            info!(
                "Finalizing request {}: received {} responses (threshold {})",
                request_id, p.response_count, p.threshold
            );

            if p.response_count < p.threshold {
                warn!(
                    "Request {} failed: Threshold not met ({} < {})",
                    request_id, p.response_count, p.threshold
                );
                let _ = p.responder.send(Err(AppError::Service(format!(
                    "Threshold not met for request {}",
                    request_id
                ))));
                return Ok(()); // Return Ok here as the finalization itself didn't fail
            }

            if p.collected_bundles.is_empty() {
                warn!(
                    "Request {} failed: Threshold met but no bundles collected (this shouldn't \
                     happen)",
                    request_id
                );
                let _ = p.responder.send(Err(AppError::Service(format!(
                    "No bundles collected for request {}",
                    request_id
                ))));
                return Ok(());
            }

            // 7. Store valid secrets in the enclave
            info!(
                "Calling put_secrets for request {} with {} bundles.",
                request_id,
                p.collected_bundles.len()
            );
            match self.enclave_client.put_secrets(p.collected_bundles).await {
                Ok(success) => {
                    if !success {
                        warn!(
                            "Enclave put_secrets returned false for request {}",
                            request_id
                        );
                        // Treat as failure even if enclave call succeeded
                        let _ = p.responder.send(Err(AppError::Service(format!(
                            "Enclave failed to store secrets for request {}",
                            request_id
                        ))));
                        return Ok(());
                    }
                    info!("Enclave put_secrets succeeded for request {}", request_id);

                    // Verify that all requested secrets are now present
                    let final_check_ids: Vec<SecretId> = p.requested_ids.iter().cloned().collect();
                    match self.enclave_client.check_secrets(final_check_ids).await {
                        Ok(statuses) => {
                            let now = current_unix_time();
                            let mut all_found = true;
                            let found_set: HashSet<SecretId> = statuses
                                .into_iter()
                                .filter_map(|(sid, found, expiry)| {
                                    if found && (expiry == 0 || expiry > now) {
                                        Some(sid)
                                    } else {
                                        None
                                    }
                                })
                                .collect();

                            for requested_id in &p.requested_ids {
                                if !found_set.contains(requested_id) {
                                    all_found = false;
                                    error!(
                                        "Request {} failed: Secret {:?} not found in enclave \
                                         after put_secrets.",
                                        request_id, requested_id
                                    );
                                    break;
                                }
                            }

                            if all_found {
                                info!(
                                    "All {} requested secrets confirmed in enclave for request {}.",
                                    p.requested_ids.len(),
                                    request_id
                                );
                                let _ = p.responder.send(Ok(()));
                            } else {
                                let _ = p.responder.send(Err(AppError::Service(format!(
                                    "Failed to confirm all secrets in enclave for request {}",
                                    request_id
                                ))));
                            }
                        }
                        Err(e) => {
                            error!(
                                "Failed final check_secrets for request {}: {}",
                                request_id, e
                            );
                            let _ = p.responder.send(Err(AppError::Service(format!(
                                "Failed final check_secrets for request {}: {}",
                                request_id, e
                            ))));
                        }
                    }
                }
                Err(e) => {
                    error!(
                        "Enclave put_secrets failed for request {}: {}",
                        request_id, e
                    );
                    let _ = p.responder.send(Err(AppError::Service(format!(
                        "Enclave put_secrets failed for request {}: {}",
                        request_id, e
                    ))));
                }
            }
        } else {
            debug!(
                "Attempted to finalize request {}, but it was not found in pending map.",
                request_id
            );
        }
        Ok(())
    }

    /// Check which secrets are locally available/unexpired in the enclave.
    async fn check_local(
        &self,
        requests: &HashMap<SecretId, Vec<SecretRequest>>,
    ) -> Result<(Vec<SecretId>, HashMap<SecretId, Vec<SecretRequest>>), AppError> {
        let all_ids: Vec<SecretId> = requests.keys().cloned().collect();
        if all_ids.is_empty() {
            return Ok((Vec::new(), HashMap::new()));
        }

        let statuses = self
            .enclave_client
            .check_secrets(all_ids.clone())
            .await
            .map_err(|e| AppError::Service(format!("check_secrets failed: {e}")))?;

        let now = current_unix_time();
        let local_set: HashSet<SecretId> = statuses
            .into_iter()
            .filter_map(|(sid, found, expiry)| {
                if found && (expiry == 0 || expiry > now) {
                    Some(sid)
                } else {
                    None
                }
            })
            .collect();

        let mut local_ids = Vec::new();
        let mut missing = HashMap::new();
        for (sid, reqs) in requests {
            if local_set.contains(sid) {
                local_ids.push(sid.clone());
            } else {
                missing.insert(sid.clone(), reqs.clone());
            }
        }
        Ok((local_ids, missing))
    }

    /// Gather local secrets for an inbound request from a peer.
    async fn gather_local(
        &self,
        requests: &BTreeMap<SecretId, Vec<SecretRequest>>,
    ) -> Vec<SecretId> {
        let all_ids: Vec<SecretId> = requests.keys().cloned().collect();
        if all_ids.is_empty() {
            return Vec::new();
        }

        match self.enclave_client.check_secrets(all_ids.clone()).await {
            Ok(statuses) => {
                let now = current_unix_time();
                statuses
                    .into_iter()
                    .filter_map(|(sid, found, expiry)| {
                        if found && (expiry == 0 || expiry > now) {
                            Some(sid)
                        } else {
                            None
                        }
                    })
                    .collect()
            }
            Err(e) => {
                error!("check_secrets failed during gather_local: {}", e);
                Vec::new()
            }
        }
    }
}

fn current_unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
