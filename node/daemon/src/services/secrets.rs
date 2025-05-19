use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use futures::channel::mpsc;
use libp2p::PeerId; // Import PeerId
use libp2p::identity::Keypair; // Import Keypair
use nxcc_interface::types::{
    AttestationReport, ConsumerInfo, EnvReport, PolicyExecutionRequest, SecretId, SecretRequest,
    SecretsBox, WorkerBundle, WorkerManifest,
};
use tokio::sync::{Mutex, RwLock, oneshot};
use tracing::{debug, error, info, warn};

use super::runner;
use crate::config::Config; // Import Config
use crate::{
    error::AppError, grpc::enclave_client::EnclaveClient, network::SecretsMessage,
    policy::PolicyManager, services::runner::RunnerService,
};

// Timeout for waiting for P2P responses
#[cfg(not(debug_assertions))]
const P2P_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(debug_assertions)]
const P2P_RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);
// Threshold for number of valid responses needed per secret (currently request-level)
const RESPONSE_THRESHOLD: usize = 1;

pub struct SecretsService {
    p2p_secrets_sender: mpsc::Sender<SecretsMessage>,
    enclave_client: EnclaveClient,
    policy_manager: Arc<PolicyManager>,
    pending: Mutex<HashMap<u64, PendingRequest>>,
    runner_service: Arc<RunnerService>,
    request_counter: AtomicU64,
    local_peer_id: PeerId, // Store the local PeerId
                           // config: Arc<Config>, // Store config if needed for other things
}

struct PendingRequest {
    // All secret IDs originally requested that were missing locally
    requested_ids: HashSet<SecretId>,
    // Store the ConsumerInfo for each originally requested SecretId
    consumers_by_id: HashMap<SecretId, ConsumerInfo>,
    // Bundles received from peers, along with their EnvReport
    collected_bundles: Vec<(SecretsBox, EnvReport)>,
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
        policy_manager: Arc<PolicyManager>,
        runner_service: Arc<RunnerService>,
        local_key: Keypair, // Pass the keypair
                            // config: Arc<Config>, // Pass config
    ) -> Arc<Self> {
        Arc::new(Self {
            p2p_secrets_sender,
            enclave_client,
            policy_manager,
            runner_service,
            pending: Mutex::new(HashMap::new()),
            request_counter: AtomicU64::new(0),
            local_peer_id: local_key.public().to_peer_id(), // Derive PeerId
                                                            // config,
        })
    }

    /// Fetches secrets, ensuring all requested secrets (local or remote) are available.
    /// Returns Ok(()) on success, Err otherwise. The actual secrets box for the caller
    /// needs to be retrieved separately via the gRPC handler after this returns Ok.
    pub async fn get_secrets(
        self: Arc<Self>,
        secret_requests_map: HashMap<SecretId, Vec<SecretRequest>>,
        caller_env_report: EnvReport,
    ) -> Result<(), AppError> {
        info!(
            "get_secrets called with {} unique secret IDs",
            secret_requests_map.len()
        );

        // 1. Determine local/missing secrets
        let (local_ids, missing_requests_map) = self.check_local(&secret_requests_map).await?;
        debug!(
            "Local secrets: {}, Missing secrets: {}",
            local_ids.len(),
            missing_requests_map.len()
        );

        if missing_requests_map.is_empty() {
            info!("All requested secrets are available locally.");
            // All secrets are local, nothing more to do here.
            // The gRPC handler will call enclave's get_secrets later.
            return Ok(());
        }

        // 2. Fetch and validate policies for missing secrets using PolicyManager
        let mut policies = HashMap::new();
        let missing_ids: HashSet<SecretId> = missing_requests_map.keys().cloned().collect();

        // Extract a representative ConsumerInfo for each missing SecretId
        // This assumes all SecretRequest in the Vec for a given SecretId have the same ConsumerInfo,
        // or that the first one is representative.
        let mut missing_consumers_by_id = HashMap::new();
        for (id, req_vec) in &missing_requests_map {
            if let Some(first_req) = req_vec.first() {
                missing_consumers_by_id.insert(id.clone(), first_req.consumer.clone());
            } else {
                // This case should ideally not be reached if missing_requests_map is well-formed
                error!("Empty request vector for missing secret ID: {:?}", id);
                return Err(AppError::Internal(format!(
                    "No consumer info found for secret ID: {:?}",
                    id
                )));
            }
        }

        let self_env_report = self.get_own_env_report(vec![]).await?;

        for secret_id in &missing_ids {
            let policy_package = match self.policy_manager.get_policy(secret_id).await {
                Ok(policy_package) => {
                    // PolicyManager.get_policy now handles manifest validation (no identities)
                    debug!("Policy validated for secret {:?}", secret_id);
                    policy_package
                }
                Err(e) => {
                    error!(
                        "Failed to get or validate policy for secret {:?}: {}",
                        secret_id, e
                    );
                    return Err(e); // Propagate policy fetch/validation error
                }
            };
            policies.insert(secret_id.clone(), policy_package.clone());

            let consumer_info = missing_consumers_by_id.get(secret_id).ok_or_else(|| {
                AppError::Internal(format!("ConsumerInfo missing for secret {:?}", secret_id))
            })?;

            // TODO: This policy check is for the *daemon* to be able to *request* the secret
            // for a given consumer. The enclave will perform its own checks later.
            // The EnvReport here is the daemon's own.
            match self
                .runner_service
                .check_policy_for_env(
                    policy_package,
                    &self_env_report, // Our node's env report
                    secret_id,
                    consumer_info, // The consumer specified in the original request
                )
                .await
            {
                Ok(true) => { /* Allowed to proceed with P2P request/generation */ }
                Ok(false) => {
                    return Err(AppError::Service(format!(
                        "Policy denied for self to request secret {:?} for consumer bundle_hash \
                         {:?}",
                        secret_id, consumer_info.bundle_hash
                    )));
                }
                Err(e) => return Err(e),
            }
        }
        info!(
            "Successfully fetched and validated policies for {} missing secrets.",
            policies.len()
        );

        // 3. Request missing secrets from P2P network
        let request_id = self.request_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();

        {
            let mut guard = self.pending.lock().await;
            guard.insert(
                request_id,
                PendingRequest {
                    requested_ids: missing_ids.clone(), // Store only the IDs
                    consumers_by_id: missing_consumers_by_id.clone(),
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
            let mut lock = self_clone.pending.lock().await; // Acquire lock here
            if let Some(p) = lock.get_mut(&request_id) {
                // Check if threshold was already met
                if p.response_count < p.threshold {
                    // Threshold not met, remove and finalize for timeout/generation
                    let pending_req = lock.remove(&request_id);
                    drop(lock); // Drop lock before calling finalize
                    if let Some(p_inner) = pending_req {
                        if let Err(e) = self_clone
                            .finalize_request_timeout(request_id, p_inner)
                            .await
                        {
                            error!(
                                "Error during timeout finalization for request {}: {}",
                                request_id, e
                            );
                        }
                    }
                } else {
                    // Threshold met, request already finalized or being finalized
                    drop(lock);
                    debug!(
                        "Timeout occurred for request {}, but threshold was already met.",
                        request_id
                    );
                }
            } else {
                // Request already removed (finalized by threshold or timeout)
                drop(lock);
                debug!(
                    "Timeout occurred for request {}, but it was already finalized.",
                    request_id
                );
            }
        });

        // Publish request to P2P network
        self.p2p_secrets_sender
            .clone()
            .try_send(SecretsMessage::PublishSecretsRequest {
                request_id,
                secret_requests: BTreeMap::from_iter(missing_requests_map.clone().into_iter()),
                env_report: caller_env_report.clone(),
            })
            .map_err(|e| AppError::Service(format!("Failed to publish secrets request: {e}")))?;

        info!(
            "Published P2P secrets request {} for {} secrets.",
            request_id,
            missing_ids.len()
        );

        // 4. Wait for P2P process completion (threshold met or timeout)
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

    /// Called when we receive a p2p secrets response.
    pub async fn handle_incoming_secret_batch_response(
        self: &Arc<Self>,
        request_id: u64,
        secrets_box: SecretsBox,
        env_report: EnvReport, // Now receives EnvReport
    ) -> Result<(), AppError> {
        let mut lock = self.pending.lock().await;
        if let Some(p) = lock.get_mut(&request_id) {
            debug!(
                "Received response for pending request {}, current count: {}, threshold: {}",
                request_id, p.response_count, p.threshold
            );

            // 1. Run policy check against the responder's EnvReport
            let mut is_valid_response = true;
            for secret_id in &secrets_box.contained_secret_ids {
                // Fetch policy needed for the check
                let policy_package = self.policy_manager.get_policy(secret_id).await?;
                let consumer_info = p.consumers_by_id.get(secret_id).ok_or_else(|| {
                    AppError::Internal(format!(
                        "ConsumerInfo missing for secret {:?} in pending request {}",
                        secret_id, request_id
                    ))
                })?;

                match self
                    .runner_service
                    .check_policy_for_env(policy_package, &env_report, secret_id, consumer_info)
                    .await
                {
                    Ok(false) => {
                        warn!(
                            "Policy check failed for secret {:?} from responder node {} for \
                             consumer bundle_hash {:?}",
                            secret_id, env_report.node_id, consumer_info.bundle_hash
                        );
                        is_valid_response = false;
                        break; // One failure invalidates the whole bundle for now
                    }
                    Ok(true) => { /* Policy satisfied, continue */ }
                    Err(e) => {
                        error!(
                            "Policy execution failed for responder node {}: {}",
                            env_report.node_id, e
                        );
                        is_valid_response = false;
                        break;
                    }
                }
            }

            if is_valid_response {
                // Store the EnvReport along with the SecretsBox
                p.collected_bundles.push((secrets_box, env_report));
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
                    // Remove from map *before* dropping lock to prevent timeout finalization race
                    let pending_req = lock.remove(&request_id);
                    // Drop the lock before calling finalize_request to avoid deadlock
                    drop(lock);
                    // Use spawn to avoid blocking the network handler
                    if let Some(p_inner) = pending_req {
                        let self_clone = Arc::clone(self);
                        tokio::spawn(async move {
                            if let Err(e) = self_clone
                                .finalize_request_threshold(request_id, p_inner)
                                .await
                            {
                                error!(
                                    "Error during threshold finalization for request {}: {}",
                                    request_id, e
                                );
                                // TODO: How to signal failure back if responder already gone? Log is best effort.
                            }
                        });
                    } else {
                        warn!(
                            "Threshold met for request {}, but it was already removed from \
                             pending map.",
                            request_id
                        );
                    }
                }
            } else {
                warn!(
                    "Ignoring invalid response for request {} (policy check failed)",
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
        secret_requests_from_peer: BTreeMap<SecretId, Vec<SecretRequest>>,
        requester_env_report: EnvReport,
    ) -> Option<(SecretsBox, EnvReport)> {
        // Return tuple including our EnvReport
        let found_ids = self.gather_local(&secret_requests_from_peer).await;
        if found_ids.is_empty() {
            debug!("No local secrets found for incoming request.");
            return None;
        }
        debug!(
            "Found {} potentially matching local secrets for incoming request.",
            found_ids.len()
        );

        // 1. Policy check: Verify the requester is allowed to get these secrets
        let mut authorized_ids_and_consumers = Vec::new();
        for secret_id in found_ids {
            // Extract the ConsumerInfo from the peer's request for this specific secret_id
            let consumer_info_for_peer_worker = match secret_requests_from_peer
                .get(&secret_id)
                .and_then(|req_vec| req_vec.first().map(|req| req.consumer.clone()))
            {
                Some(ci) => ci,
                None => {
                    error!(
                        "ConsumerInfo missing in peer's request for secret {:?}, skipping \
                         authorization check.",
                        secret_id
                    );
                    continue;
                }
            };

            let policy_package = match self.policy_manager.get_policy(&secret_id).await {
                Ok(p) => p,
                Err(e) => {
                    error!("Failed to get policy for {:?}: {}", secret_id, e);
                    continue; // Skip this secret if policy fetch fails
                }
            };
            match self
                .runner_service
                .check_policy_for_env(
                    policy_package,
                    &requester_env_report,
                    &secret_id,
                    &consumer_info_for_peer_worker,
                )
                .await
            {
                Ok(true) => {
                    authorized_ids_and_consumers.push((secret_id, consumer_info_for_peer_worker))
                }
                Ok(false) => warn!(
                    // Policy denied is not an error, just a denial
                    "Policy denied for requester {} for secret {:?} with consumer bundle_hash {:?}",
                    requester_env_report.node_id,
                    secret_id,
                    consumer_info_for_peer_worker.bundle_hash
                ),
                Err(e) => error!(
                    "Policy execution failed for requester {}: {}",
                    requester_env_report.node_id, e
                ),
            }
        }

        if authorized_ids_and_consumers.is_empty() {
            info!(
                "No secrets authorized for requester {}",
                requester_env_report.node_id
            );
            return None;
        }

        // 2. Get the authorized secrets from the enclave
        match self
            .enclave_client
            .get_secrets(authorized_ids_and_consumers, requester_env_report) // Pass requester's report
            .await
        {
            Ok(sb) => {
                info!(
                    "Returning secrets box with {} authorized secrets.",
                    sb.contained_secret_ids.len()
                );
                // 3. Generate *our* EnvReport to send back with the secrets
                match self
                    .get_own_env_report(sb.calculate_binding_hash().to_vec())
                    .await
                {
                    Ok(our_env_report) => Some((sb, our_env_report)),
                    Err(e) => {
                        error!("Failed to generate own EnvReport for response: {}", e);
                        None
                    }
                }
            }
            Err(e) => {
                error!(
                    "Failed to get secrets from enclave for incoming request: {}",
                    e
                );
                None
            }
        }
    }

    /// Finalizes a request after timeout, attempting generation.
    /// Assumes the request was removed from the map by the caller.
    async fn finalize_request_timeout(
        self: Arc<Self>,
        request_id: u64,
        p: PendingRequest,
    ) -> Result<(), AppError> {
        warn!(
            "Request {} timed out with {} responses (threshold {}). Attempting generation.",
            request_id, p.response_count, p.threshold
        );

        let mut ids_to_generate = Vec::new();
        for id_to_generate_sid in p.requested_ids.iter() {
            if let Some(consumer) = p.consumers_by_id.get(id_to_generate_sid) {
                ids_to_generate.push((id_to_generate_sid.clone(), consumer.clone()));
            } else {
                error!(
                    "ConsumerInfo missing for secret {:?} in pending request {} during timeout \
                     finalization",
                    id_to_generate_sid, request_id
                );
            }
        }

        if ids_to_generate.is_empty() && !p.requested_ids.is_empty() {
            warn!(
                "No valid (secret, consumer) pairs for generation in request {}",
                request_id
            );
            let _ = p.responder.send(Err(AppError::Internal(
                "No valid consumers for generation".into(),
            )));
            return Ok(());
        }

        match self.generate_secrets_flow(ids_to_generate).await {
            Ok(()) => {
                info!("Secret generation successful for request {}", request_id);
                // Check if secrets are now present and respond
                self.check_and_respond(request_id, p.requested_ids, p.responder)
                    .await?;
            }
            Err(e) => {
                error!("Secret generation failed for request {}: {}", request_id, e);
                let _ = p.responder.send(Err(e));
            }
        }
        Ok(())
    }

    /// Processes collected bundles, puts them into the enclave, checks final status, and responds.
    /// Assumes the request was removed from the map by the caller.
    async fn finalize_request_threshold(
        self: Arc<Self>,
        request_id: u64,
        p: PendingRequest,
    ) -> Result<(), AppError> {
        info!(
            "Finalizing request {}: received {} responses (threshold {})",
            request_id, p.response_count, p.threshold
        );

        // Threshold check is implicitly done by the fact this function is called

        if p.collected_bundles.is_empty() {
            warn!(
                "Request {} failed: Threshold met but no bundles collected (this shouldn't happen)",
                request_id
            );
            let _ = p.responder.send(Err(AppError::Service(format!(
                "No bundles collected for request {}",
                request_id
            ))));
            return Ok(());
        }

        // 7. Store valid secrets in the enclave using put_secrets with EnvReports and ConsumerInfo
        info!(
            "Preparing to call put_secrets for request {} with {} bundles.",
            request_id,
            p.collected_bundles.len()
        );

        let mut bundles_for_enclave = Vec::new();
        for (secrets_box, env_report) in p.collected_bundles {
            // TODO: This is a simplification. If a SecretsBox contains secrets originally
            // requested for *different* local consumers, this logic is insufficient.
            // The `PutSecretsRequest::SecretsBundle` in proto associates one `ConsumerInfo`
            // with one `SecretsBox`. This implies all secrets in that box are for that consumer.
            // For now, we pick the ConsumerInfo associated with the *first* secret ID in the box.
            // A more robust solution might involve splitting the SecretsBox or having the enclave
            // handle multiple consumers per box if the proto is updated.
            if let Some(first_sid) = secrets_box.contained_secret_ids.first() {
                if let Some(consumer_info) = p.consumers_by_id.get(first_sid) {
                    bundles_for_enclave.push((secrets_box, env_report, consumer_info.clone()));
                } else {
                    warn!(
                        "ConsumerInfo not found for primary secret {:?} in SecretsBox from peer, \
                         request_id {}. Skipping bundle.",
                        first_sid, request_id
                    );
                }
            } else {
                warn!(
                    "Received empty SecretsBox from peer for request_id {}. Skipping bundle.",
                    request_id
                );
            }
        }

        match self.enclave_client.put_secrets(bundles_for_enclave).await {
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

                self.check_and_respond(request_id, p.requested_ids, p.responder)
                    .await?;
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
        Ok(())
    }

    /// Checks if all requested secrets are present in the enclave and sends the final response.
    async fn check_and_respond(
        &self,
        request_id: u64,
        requested_ids: HashSet<SecretId>,
        responder: oneshot::Sender<Result<(), AppError>>,
    ) -> Result<(), AppError> {
        let final_check_ids: Vec<SecretId> = requested_ids.iter().cloned().collect();
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

                for requested_id in &requested_ids {
                    if !found_set.contains(requested_id) {
                        all_found = false;
                        error!(
                            "Request {} failed: Secret {:?} not found in enclave after process.",
                            request_id, requested_id
                        );
                        break;
                    }
                }

                if all_found {
                    info!(
                        "All {} requested secrets confirmed in enclave for request {}.",
                        requested_ids.len(),
                        request_id
                    );
                    let _ = responder.send(Ok(()));
                } else {
                    let _ = responder.send(Err(AppError::Service(format!(
                        "Failed to confirm all secrets in enclave for request {}",
                        request_id
                    ))));
                }
            }
            Err(e) => {
                error!(
                    "Failed to final check_secrets for request {}: {}",
                    request_id, e
                );
                let _ = responder.send(Err(AppError::Service(format!(
                    "Failed final check_secrets for request {}: {}",
                    request_id, e
                ))));
            }
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

    /// Orchestrates the secret generation flow for a list of IDs.
    async fn generate_secrets_flow(
        &self,
        requests_with_consumer: Vec<(SecretId, ConsumerInfo)>,
    ) -> Result<(), AppError> {
        info!(
            "Starting generation flow for {} secret-consumer pairs",
            requests_with_consumer.len()
        );
        if requests_with_consumer.is_empty() {
            return Ok(());
        }

        let self_env_report = self.get_own_env_report(vec![]).await?;

        for (secret_id, consumer_info) in &requests_with_consumer {
            let policy_package = self.policy_manager.get_policy(secret_id).await?;

            // 2. Execute policy for self-authorization
            match self
                .runner_service
                .check_policy_for_env(policy_package, &self_env_report, secret_id, consumer_info)
                .await
            {
                Ok(true) => {
                    info!(
                        "Self-authorization successful for secret {:?} and consumer bundle_hash \
                         {:?}",
                        secret_id, consumer_info.bundle_hash
                    );
                    // Authorization is stored implicitly by execute_policy_for_env via enclave
                }
                Ok(false) => {
                    warn!(
                        "Self-authorization denied for secret {:?} and consumer bundle_hash {:?}",
                        secret_id, consumer_info.bundle_hash
                    );
                    // Continue to try others, but maybe return partial failure?
                }
                Err(e) => {
                    error!(
                        "Policy execution failed during self-authorization for secret {:?} and \
                         consumer bundle_hash {:?}: {}",
                        secret_id, consumer_info.bundle_hash, e
                    );
                    // Propagate the error for now
                    return Err(e);
                }
            }
        }

        // 3. Call enclave's generate_secrets (which checks internal auth store)
        self.enclave_client
            .generate_secrets(requests_with_consumer)
            .await
            .map_err(|e| AppError::Service(format!("Enclave generate_secrets failed: {}", e)))
    }

    /// Constructs the EnvReport for the current node.
    pub(crate) async fn get_own_env_report(
        &self,
        user_data_hash: Vec<u8>,
    ) -> Result<EnvReport, AppError> {
        let attestation = self
            .enclave_client
            .get_report(user_data_hash)
            .await
            .map_err(|e| AppError::Service(format!("Failed to get attestation report: {}", e)))?;

        // TODO: Implement operator signing
        let operator_signature = vec![0u8; 64]; // Placeholder
        Ok(EnvReport {
            attestation,
            operator_signature,
            node_id: "@self".into(), // This node_id is for daemon's internal use/logging; enclave uses attestation.
        })
    }
}

fn current_unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
