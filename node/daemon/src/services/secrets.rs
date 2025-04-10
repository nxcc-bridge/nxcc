use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use futures::channel::mpsc;
use interface::types::{Secret, SecretId, SecretRequest, SecretRequesterInfo, SecretsBox};
use tokio::sync::{Mutex, RwLock, oneshot};
use tracing::{debug, error, warn};

use crate::{
    error::AppError, grpc::enclave_client::EnclaveClient, network::SecretsMessage,
    policy::ManifestChecker, web3::gateways::GatewayManager,
};

/// Each inbound secrets request from the gRPC layer is mapped to these domain types
pub struct SecretsService {
    p2p_secrets_sender: mpsc::Sender<SecretsMessage>,

    // A local ephemeral store of secrets we have "outside" of the real enclave. In reality,
    // we want to rely entirely on the enclave for storing secrets. We'll keep a small map
    // for quick checks. But the real approach is: check `enclave_client.check_secrets`.
    local_cache: RwLock<HashMap<SecretId, Secret>>,

    pending: Mutex<HashMap<u64, PendingRequest>>,
    request_counter: Mutex<u64>,

    gateway_manager: GatewayManager,
    manifest_checker: ManifestChecker,

    // Our handle to the local enclave. In a real deployment, you'd create it in main
    // (e.g. via UDS or vsock) and pass it here.
    enclave_client: tokio::sync::Mutex<EnclaveClient>,
}

struct PendingRequest {
    requested_ids: HashMap<SecretId, Vec<SecretRequest>>,
    requester_info: SecretRequesterInfo,
    threshold: usize,
    collected: Vec<SecretsBox>,
    responder: oneshot::Sender<Result<SecretsBox, AppError>>,
}

impl SecretsService {
    pub async fn new(p2p_secrets_sender: mpsc::Sender<SecretsMessage>) -> Arc<Self> {
        // TODO: In real code, pass a connected EnclaveClient. Here we do a lazy approach:
        // we'll connect in get_secrets() the first time.
        Arc::new(Self {
            p2p_secrets_sender,
            local_cache: RwLock::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            request_counter: Mutex::new(0),
            gateway_manager: GatewayManager::new(),
            manifest_checker: ManifestChecker,
            enclave_client: tokio::sync::Mutex::new(
                // We'll replace with real connect in get_secrets() if needed
                EnclaveClient::connect_uds("/tmp/enclave_grpc.sock".to_string())
                    .await
                    .unwrap_or_else(|_| {
                        panic!(
                            "Failed to create ephemeral EnclaveClient. Make sure the enclave is \
                             running."
                        )
                    }),
            ),
        })
    }

    /// Called by the gRPC `get_secrets` method in daemon/src/grpc/secrets.rs
    /// to retrieve secrets for the caller's request.
    pub async fn get_secrets(
        self: &Arc<Self>,
        secret_requests: HashMap<SecretId, Vec<SecretRequest>>,
        requester_info: SecretRequesterInfo,
    ) -> Result<SecretsBox, AppError> {
        // 1. Attempt to retrieve from local enclave (or local_cache).
        let (local, missing) = self.get_local_secrets(&secret_requests).await?;

        if missing.is_empty() {
            debug!("All secrets found locally, returning immediately");
            return self.encrypt_for_requester(local, &requester_info).await;
        }

        // 2. Some secrets are missing, so we broadcast a request on the P2P network
        let request_id = {
            let mut rc = self.request_counter.lock().await;
            *rc += 1;
            *rc
        };
        let threshold = 1; // we only need 1 peer's response in this demo

        let (tx, rx) = oneshot::channel();

        {
            let mut pending_guard = self.pending.lock().await;
            pending_guard.insert(
                request_id,
                PendingRequest {
                    requested_ids: missing.clone(),
                    requester_info: requester_info.clone(),
                    threshold,
                    collected: Vec::new(),
                    responder: tx,
                },
            );
        }

        // Send the request out
        self.p2p_secrets_sender
            .clone()
            .try_send(SecretsMessage::PublishSecretsRequest {
                request_id,
                secret_requests: BTreeMap::from_iter(missing.clone().into_iter()),
                requester_info: requester_info.clone(),
            })?;

        // Wait for the response
        let p2p_box = match rx.await {
            Ok(Ok(sb)) => sb,
            Ok(Err(e)) => return Err(e),
            Err(e) => {
                return Err(AppError::Service(format!("Pending request dropped: {}", e)));
            }
        };

        // 3. We got a SecretsBox from the peer. Now we store it in our local enclave using put_secrets
        {
            let mut encl = self.enclave_client.lock().await;
            let ar = encl.get_report(Vec::new()).await.map_err(|e| {
                AppError::Service(format!("Failed to get local encl report: {}", e))
            })?;

            // We call put_secrets with a single secrets bundle
            let success = encl
                .put_secrets(vec![(p2p_box.clone(), ar)])
                .await
                .map_err(|e| AppError::Service(format!("put_secrets failed: {}", e)))?;
            if !success {
                return Err(AppError::Service(
                    "Enclave put_secrets returned false".into(),
                ));
            }
        }

        // 4. Re-check or retrieve from the local enclave in an encrypted form for the final response
        let mut all_ids = Vec::new();
        all_ids.extend(local.clone());
        all_ids.extend(missing.keys().cloned());

        // Combine local + newly acquired secrets into a final SecretsBox for the caller
        self.encrypt_for_requester(all_ids, &requester_info).await
    }

    /// This is called by `handle_incoming_secret_batch_response` when we get a new secrets box from a peer.
    pub async fn handle_incoming_secret_batch_response(
        &self,
        request_id: u64,
        secrets_box: SecretsBox,
    ) -> Result<(), AppError> {
        // First, check if we need to finalize and prepare data outside the lock
        let finalize_data = {
            let mut lock = self.pending.lock().await;
            if let Some(pend) = lock.get_mut(&request_id) {
                pend.collected.push(secrets_box);

                // If we've collected enough for threshold, prepare for finalization
                if pend.collected.len() >= pend.threshold {
                    // Clone the data we need outside the lock
                    let boxes = pend.collected.clone();
                    let responder = std::mem::replace(
                        &mut pend.responder,
                        // Replace with a dummy sender that will be dropped
                        tokio::sync::oneshot::channel().0,
                    );

                    Some((boxes, responder, request_id))
                } else {
                    None
                }
            } else {
                None
            }
        };

        // If we need to finalize, do it outside the lock
        if let Some((boxes, responder, request_id)) = finalize_data {
            // Merge the secrets boxes
            let merged = self.merge_secrets_boxes(boxes).expect("TODO: handle");

            // Remove the pending request
            {
                let mut lock = self.pending.lock().await;
                lock.remove(&request_id);
            }

            // Send the merged secrets box
            let _ = responder.send(Ok(merged));
        }

        Ok(())
    }

    pub async fn handle_incoming_secret_batch_request(
        &self,
        request_id: u64,
        secret_requests: BTreeMap<SecretId, Vec<SecretRequest>>,
        requester_info: SecretRequesterInfo,
    ) -> SecretsBox {
        debug!(
            "Incoming secret request {} with {} items",
            request_id,
            secret_requests.len()
        );

        // For each requested secret, see if we have it in our local enclave
        // If we have it, we gather them into one box. If not, we skip it.
        let found_secrets = self.gather_secrets_for_peer(secret_requests).await;

        // If none found, return empty
        if found_secrets.is_empty() {
            debug!(
                "No secrets found locally for request {}, returning empty box",
                request_id
            );
            return SecretsBox::new_empty();
        }

        // Now we ask our local enclave to produce an AEAD-encrypted box for the requester's ephemeral key
        let mut encl = self.enclave_client.lock().await;
        let proto_ar = encl.get_report(Vec::new()).await.unwrap_or_else(|_| {
            // Fallback
            interface::AttestationReport {
                ephemeral_public_key: vec![],
                block_hashes: vec![],
                user_data: vec![],
            }
        });

        // We need a domain-level "policy reports" for each secret; not implementing for now.
        let pol_reports = vec![];

        encl.get_secrets(
            found_secrets,
            pol_reports,
            attestation_report_from_requester(&requester_info),
        )
        .await
        .unwrap_or_else(|e| {
            warn!("Failed to get secrets from enclave: {}", e);
            SecretsBox::new_empty()
        })
    }

    /// Called after a timeout, or if the request was canceled, etc.
    pub async fn timeout_pending_request(&self, request_id: u64) -> Result<(), AppError> {
        let mut lock = self.pending.lock().await;
        if let Some(pend) = lock.remove(&request_id) {
            if pend.collected.is_empty() {
                let _ = pend
                    .responder
                    .send(Err(AppError::Service("No responses collected".into())));
            } else if pend.collected.len() < pend.threshold {
                let _ = pend
                    .responder
                    .send(Err(AppError::Service("Threshold not reached".into())));
            } else {
                // Merge them
                match self.merge_secrets_boxes(pend.collected) {
                    Ok(sb) => {
                        let _ = pend.responder.send(Ok(sb));
                    }
                    Err(e) => {
                        let _ = pend.responder.send(Err(AppError::Service(e)));
                    }
                }
            }
        }
        Ok(())
    }

    async fn get_local_secrets(
        &self,
        secret_requests: &HashMap<SecretId, Vec<SecretRequest>>,
    ) -> Result<(Vec<SecretId>, HashMap<SecretId, Vec<SecretRequest>>), AppError> {
        let mut local_found = Vec::new();
        let mut missing = HashMap::new();

        // We'll do a quick local_cache check. Then in a real system, we'd do
        // `enclave_client.check_secrets` instead.
        let cache_guard = self.local_cache.read().await;
        for (id, reqs) in secret_requests {
            if cache_guard.contains_key(id) {
                local_found.push(id.clone());
            } else {
                missing.insert(id.clone(), reqs.clone());
            }
        }
        Ok((local_found, missing))
    }

    /// Merge multiple secrets boxes. For the simple "demo," we just take the first box.
    /// In a real threshold scheme, you'd combine shares here.
    fn merge_secrets_boxes(&self, boxes: Vec<SecretsBox>) -> Result<SecretsBox, String> {
        // We do a trivial approach: just return the first non-empty box.
        for b in boxes {
            if !b.encrypted_payload.is_empty() {
                return Ok(b);
            }
        }
        Ok(SecretsBox::new_empty())
    }

    /// Called from `handle_incoming_secret_batch_request` to gather local secrets for the request.
    async fn gather_secrets_for_peer(
        &self,
        requests: BTreeMap<SecretId, Vec<SecretRequest>>,
    ) -> Vec<SecretId> {
        let cache_guard = self.local_cache.read().await;
        let mut found = Vec::new();
        for (id, _req) in requests.iter() {
            if cache_guard.contains_key(id) {
                found.push(id.clone());
            }
        }
        found
    }

    /// After we've acquired the secrets, produce an AEAD-encrypted `SecretsBox` for the caller.
    /// For demonstration, we do not re-check the policy here; we assume it was handled earlier.
    async fn encrypt_for_requester(
        &self,
        secret_ids: Vec<SecretId>,
        _requester_info: &SecretRequesterInfo,
    ) -> Result<SecretsBox, AppError> {
        let mut encl = self.enclave_client.lock().await;

        // We only do ephemeral local "policy reports" = empty
        let pol_reports = vec![];
        let ar = encl
            .get_report(Vec::new())
            .await
            .map_err(|e| AppError::Service(format!("Enclave get_report failed: {}", e)))?;

        // We call get_secrets in our local enclave. The secrets to get are those in `secret_ids`.
        let secrets_box = encl
            .get_secrets(secret_ids, pol_reports, ar)
            .await
            .map_err(|e| AppError::Service(format!("Enclave get_secrets failed: {}", e)))?;

        Ok(secrets_box)
    }
}

fn attestation_report_from_requester(info: &SecretRequesterInfo) -> interface::AttestationReport {
    // For demonstration, we only fill ephemeral_public_key from `info.public_key`
    interface::AttestationReport {
        ephemeral_public_key: info.public_key.clone(),
        block_hashes: vec![],
        user_data: info.report.clone(),
    }
}
