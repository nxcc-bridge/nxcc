use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use futures::channel::mpsc;
use interface::types::{
    AttestationReport, SecretId, SecretRequest, SecretRequesterInfo, SecretsBox,
};
use tokio::sync::{Mutex, oneshot};
use tracing::{debug, error, warn};

use crate::{
    error::AppError, grpc::enclave_client::EnclaveClient, network::SecretsMessage,
    policy::ManifestChecker, web3::gateways::GatewayManager,
};

/// Each inbound secrets request from the gRPC layer is mapped to these domain types.
pub struct SecretsService {
    p2p_secrets_sender: mpsc::Sender<SecretsMessage>,
    pending: Mutex<HashMap<u64, PendingRequest>>,
    request_counter: Mutex<u64>,
    gateway_manager: GatewayManager,
    manifest_checker: ManifestChecker,
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
        Arc::new(Self {
            p2p_secrets_sender,
            pending: Mutex::new(HashMap::new()),
            request_counter: Mutex::new(0),
            gateway_manager: GatewayManager::new(),
            manifest_checker: ManifestChecker,
            enclave_client: tokio::sync::Mutex::new(
                EnclaveClient::connect_uds("/tmp/enclave_grpc.sock".to_string())
                    .await
                    .unwrap_or_else(|_| {
                        panic!(
                            "Failed to create EnclaveClient. Ensure the enclave is running on \
                             /tmp/enclave_grpc.sock."
                        )
                    }),
            ),
        })
    }

    /// Invoked by the daemon's gRPC `get_secrets` method to fetch and return secrets.
    /// The daemon simply relays requests and responses; all encryption is in the enclave.
    pub async fn get_secrets(
        self: &Arc<Self>,
        secret_requests: HashMap<SecretId, Vec<SecretRequest>>,
        requester_info: SecretRequesterInfo,
    ) -> Result<SecretsBox, AppError> {
        let (local, missing) = self.get_local_secrets(&secret_requests).await?;

        // If all secrets are stored in the local enclave, retrieve them immediately
        if missing.is_empty() {
            debug!("All requested secrets found locally.");
            let att = attestation_report_from_requester(&requester_info);
            let mut encl = self.enclave_client.lock().await;
            return encl.get_secrets(local, vec![], att).await.map_err(|e| {
                AppError::Service(format!("Failed to fetch secrets from enclave: {e}"))
            });
        }

        // Some or all secrets are missing, so request them from peers
        let request_id = {
            let mut rc = self.request_counter.lock().await;
            *rc += 1;
            *rc
        };
        let threshold = 1;
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

        self.p2p_secrets_sender
            .clone()
            .try_send(SecretsMessage::PublishSecretsRequest {
                request_id,
                secret_requests: BTreeMap::from_iter(missing.into_iter()),
                requester_info: requester_info.clone(),
            })?;

        let p2p_box = match rx.await {
            Ok(Ok(sb)) => sb,
            Ok(Err(e)) => return Err(e),
            Err(e) => {
                return Err(AppError::Service(format!("Pending request dropped: {}", e)));
            }
        };

        // Store the newly acquired secrets in our local enclave
        {
            let mut encl = self.enclave_client.lock().await;
            let local_att_report = encl
                .get_report(Vec::new())
                .await
                .map_err(|e| AppError::Service(format!("Failed to get enclave report: {e}")))?;

            let success = encl
                .put_secrets(vec![(p2p_box.clone(), local_att_report)])
                .await
                .map_err(|e| AppError::Service(format!("put_secrets failed: {e}")))?;
            if !success {
                return Err(AppError::Service(
                    "Enclave put_secrets returned false".into(),
                ));
            }
        }

        // Now that we've stored the newly received secrets, retrieve all of them for the caller
        let mut all_ids = Vec::new();
        all_ids.extend(local);
        // The above `local` is a Vec<SecretId> of items found in local enclave
        // The remote items were just put, so we can retrieve them as well
        let att = attestation_report_from_requester(&requester_info);
        let mut encl = self.enclave_client.lock().await;
        encl.get_secrets(all_ids, vec![], att)
            .await
            .map_err(|e| AppError::Service(format!("Failed to get secrets from enclave: {e}")))
    }

    /// Called when we receive a P2P response from a peer containing a SecretsBox.
    pub async fn handle_incoming_secret_batch_response(
        &self,
        request_id: u64,
        secrets_box: SecretsBox,
    ) -> Result<(), AppError> {
        let finalize_data = {
            let mut lock = self.pending.lock().await;
            if let Some(pend) = lock.get_mut(&request_id) {
                pend.collected.push(secrets_box);

                if pend.collected.len() >= pend.threshold {
                    let boxes = pend.collected.clone();
                    let responder =
                        std::mem::replace(&mut pend.responder, tokio::sync::oneshot::channel().0);
                    Some((boxes, responder, request_id))
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some((boxes, responder, req_id)) = finalize_data {
            let merged = self.merge_secrets_boxes(boxes).map_err(AppError::Service)?;

            {
                let mut lock = self.pending.lock().await;
                lock.remove(&req_id);
            }

            let _ = responder.send(Ok(merged));
        }

        Ok(())
    }

    /// Called when we receive a P2P secrets request from another peer.
    /// We gather secrets from our local enclave and return them in a SecretsBox.
    pub async fn handle_incoming_secret_batch_request(
        &self,
        request_id: u64,
        secret_requests: BTreeMap<SecretId, Vec<SecretRequest>>,
        requester_info: SecretRequesterInfo,
    ) -> SecretsBox {
        debug!(
            "Handling secrets request {} with {} items",
            request_id,
            secret_requests.len()
        );
        let found_secrets = self.gather_secrets_for_peer(secret_requests).await;
        if found_secrets.is_empty() {
            debug!("No local secrets found for request {request_id}");
            return SecretsBox::new_empty();
        }

        // Create an attestation with ephemeral public key for the local enclave
        let mut encl = self.enclave_client.lock().await;
        let local_report = match encl.get_report(Vec::new()).await {
            Ok(r) => r,
            Err(_) => AttestationReport {
                ephemeral_public_key: vec![],
                block_hashes: vec![],
                user_data: vec![],
            },
        };

        // Forward the request to the local enclave for encryption to the requester's ephemeral key
        let pol_reports = vec![];
        encl.get_secrets(
            found_secrets,
            pol_reports,
            attestation_report_from_requester(&requester_info),
        )
        .await
        .unwrap_or_else(|e| {
            warn!("Failed to retrieve secrets from enclave: {e}");
            SecretsBox::new_empty()
        })
    }

    /// Called if a request times out or is canceled.
    pub async fn timeout_pending_request(&self, request_id: u64) -> Result<(), AppError> {
        let mut lock = self.pending.lock().await;
        if let Some(pend) = lock.remove(&request_id) {
            if pend.collected.is_empty() {
                let _ = pend
                    .responder
                    .send(Err(AppError::Service("No responses received".into())));
            } else if pend.collected.len() < pend.threshold {
                let _ = pend
                    .responder
                    .send(Err(AppError::Service("Threshold not reached".into())));
            } else {
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

    /// Checks which secrets are already in the local enclave. Returns (found, missing).
    async fn get_local_secrets(
        &self,
        secret_requests: &HashMap<SecretId, Vec<SecretRequest>>,
    ) -> Result<(Vec<SecretId>, HashMap<SecretId, Vec<SecretRequest>>), AppError> {
        let requested_ids: Vec<SecretId> = secret_requests.keys().cloned().collect();
        if requested_ids.is_empty() {
            return Ok((Vec::new(), HashMap::new()));
        }

        let mut encl = self.enclave_client.lock().await;
        let check_results = encl
            .check_secrets(requested_ids.clone())
            .await
            .map_err(|e| AppError::Service(format!("Error checking secrets in enclave: {}", e)))?;

        let mut local_found_ids = HashSet::new();
        for (id, found, _expiry) in check_results {
            if found {
                local_found_ids.insert(id);
            }
        }

        let mut local_found_vec = Vec::new();
        let mut missing = HashMap::new();

        for (id, reqs) in secret_requests {
            if local_found_ids.contains(id) {
                local_found_vec.push(id.clone());
            } else {
                missing.insert(id.clone(), reqs.clone());
            }
        }

        Ok((local_found_vec, missing))
    }

    /// Gathers secrets from our local enclave for a peer's request.
    async fn gather_secrets_for_peer(
        &self,
        requests: BTreeMap<SecretId, Vec<SecretRequest>>,
    ) -> Vec<SecretId> {
        let requested_ids: Vec<SecretId> = requests.keys().cloned().collect();
        if requested_ids.is_empty() {
            return Vec::new();
        }

        let mut encl = self.enclave_client.lock().await;
        match encl.check_secrets(requested_ids).await {
            Ok(results) => results
                .into_iter()
                .filter_map(|(id, found, _)| if found { Some(id) } else { None })
                .collect(),
            Err(e) => {
                error!("Failed to check secrets in enclave for peer request: {e}");
                Vec::new()
            }
        }
    }

    /// Merges multiple SecretsBoxes. This sample merges by taking the first non-empty box.
    /// A real threshold scheme could be more complex.
    fn merge_secrets_boxes(&self, boxes: Vec<SecretsBox>) -> Result<SecretsBox, String> {
        for b in boxes {
            if !b.encrypted_payload.is_empty() {
                return Ok(b);
            }
        }
        Ok(SecretsBox::new_empty())
    }
}

/// Converts a requester's info into an AttestationReport so we can pass ephemeral key/user data
/// to the enclave. This does not perform cryptographic verification here.
fn attestation_report_from_requester(info: &SecretRequesterInfo) -> AttestationReport {
    AttestationReport {
        ephemeral_public_key: info.public_key.clone(),
        block_hashes: vec![],
        user_data: info.report.clone(),
    }
}
