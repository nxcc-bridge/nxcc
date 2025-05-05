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
use nxcc_interface::{
    policy::PolicyBundle,
    types::{EnvReport, PolicyExecutionRequest, SecretId, SecretRequest, SecretsBox},
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

        // 2. Fetch and validate policies for missing secrets using PolicyManager
        let mut policies = HashMap::new();
        let missing_ids: HashSet<SecretId> = missing_requests.keys().cloned().collect();
        for secret_id in &missing_ids {
            match self.policy_manager.get_policy(secret_id).await {
                Ok(policy) => {
                    // PolicyManager already checked the manifest internally
                    debug!("Policy validated for secret {:?}", secret_id);
                    policies.insert(secret_id.clone(), policy);
                }
                Err(e) => {
                    error!(
                        "Failed to get or validate policy for secret {:?}: {}",
                        secret_id, e
                    );
                    return Err(e); // Propagate policy fetch/validation error
                }
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
                secret_requests: BTreeMap::from_iter(missing_requests.clone().into_iter()),
                env_report: env_report.clone(), // Send the original requester's EnvReport
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
                let policy = self.policy_manager.get_policy(secret_id).await?;
                match self
                    .runner_service
                    .check_policy_for_env(policy, &env_report, secret_id)
                    .await
                {
                    Ok(false) => {
                        warn!(
                            "Policy check failed for secret {:?} from responder node {}",
                            secret_id, env_report.node_id
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
        secret_requests: BTreeMap<SecretId, Vec<SecretRequest>>,
        requester_env_report: EnvReport, // The EnvReport of the node asking for secrets
    ) -> Option<(SecretsBox, EnvReport)> {
        // Return tuple including our EnvReport
        let found_ids = self.gather_local(&secret_requests).await;
        if found_ids.is_empty() {
            debug!("No local secrets found for incoming request.");
            return None;
        }
        debug!(
            "Found {} potentially matching local secrets for incoming request.",
            found_ids.len()
        );

        // 1. Policy check: Verify the requester is allowed to get these secrets
        let mut authorized_ids = Vec::new();
        for secret_id in found_ids {
            let policy = match self.policy_manager.get_policy(&secret_id).await {
                Ok(p) => p,
                Err(e) => {
                    error!("Failed to get policy for {:?}: {}", secret_id, e);
                    continue; // Skip this secret if policy fetch fails
                }
            };
            match self
                .runner_service
                .check_policy_for_env(policy, &requester_env_report, &secret_id)
                .await
            {
                Ok(true) => authorized_ids.push(secret_id),
                Ok(false) => warn!(
                    // Policy denied is not an error, just a denial
                    "Policy denied for requester {} for secret {:?}",
                    requester_env_report.node_id, secret_id
                ),
                Err(e) => error!(
                    "Policy execution failed for requester {}: {}",
                    requester_env_report.node_id, e
                ),
            }
        }

        if authorized_ids.is_empty() {
            info!(
                "No secrets authorized for requester {}",
                requester_env_report.node_id
            );
            return None;
        }

        // 2. Get the authorized secrets from the enclave
        match self
            .enclave_client
            .get_secrets(authorized_ids, requester_env_report) // Pass requester's report
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

        // Attempt generation for all originally missing IDs
        let ids_to_generate: Vec<SecretId> = p.requested_ids.iter().cloned().collect();
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

        // 7. Store valid secrets in the enclave using put_secrets with EnvReports
        info!(
            "Calling put_secrets for request {} with {} bundles.",
            request_id,
            p.collected_bundles.len()
        );
        // Pass the collected (SecretsBox, EnvReport) tuples directly
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
                    "Failed final check_secrets for request {}: {}",
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
    async fn generate_secrets_flow(&self, ids: Vec<SecretId>) -> Result<(), AppError> {
        info!("Starting generation flow for {} secrets", ids.len());
        if ids.is_empty() {
            return Ok(());
        }

        // For simplicity, we execute policy for each ID individually.
        // Could be batched if the policy worker supports it.
        for secret_id in &ids {
            // 1. Construct self EnvReport (hash doesn't matter for self-auth check)
            let self_env_report = self.get_own_env_report(vec![0u8; 32]).await?;

            // 1.5 Get Policy
            let policy = self.policy_manager.get_policy(secret_id).await?;

            // 2. Execute policy for self-authorization
            match self
                .runner_service
                .check_policy_for_env(policy, &self_env_report, secret_id)
                .await
            {
                Ok(true) => {
                    info!("Self-authorization successful for secret {:?}", secret_id);
                    // Authorization is stored implicitly by execute_policy_for_env via enclave
                }
                Ok(false) => {
                    warn!("Self-authorization denied for secret {:?}", secret_id);
                    // Continue to try others, but maybe return partial failure?
                }
                Err(e) => {
                    error!(
                        "Policy execution failed during self-authorization for secret {:?}: {}",
                        secret_id, e
                    );
                    // Propagate the error for now
                    return Err(e);
                }
            }
        }

        // 3. Call enclave's generate_secrets (which checks internal auth store)
        self.enclave_client
            .generate_secrets(ids)
            .await
            .map_err(|e| AppError::Service(format!("Enclave generate_secrets failed: {}", e)))
    }

    /// Constructs the EnvReport for the current node.
    async fn get_own_env_report(&self, user_data_hash: Vec<u8>) -> Result<EnvReport, AppError> {
        let attestation = self
            .enclave_client
            .get_report(user_data_hash)
            .await
            .map_err(|e| AppError::Service(format!("Failed to get attestation report: {}", e)))?;

        // TODO: Implement operator signing
        let operator_signature = vec![0u8; 64]; // Placeholder
        // TODO: Get node ID from config or identity
        let node_id = self.local_peer_id.to_base58(); // Use the actual PeerId
        Ok(EnvReport {
            attestation,
            operator_signature,
            node_id,
        })
    }
}

fn current_unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
