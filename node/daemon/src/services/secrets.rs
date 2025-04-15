use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use futures::channel::mpsc;
use interface::types::{AttestationReport, EnvReport, SecretId, SecretRequest, SecretsBox};
use tokio::sync::{Mutex, oneshot};

use crate::{error::AppError, grpc::enclave_client::EnclaveClient, network::SecretsMessage};

pub struct SecretsService {
    p2p_secrets_sender: mpsc::Sender<SecretsMessage>,
    enclave_client: Mutex<EnclaveClient>,
    pending: Mutex<HashMap<u64, PendingRequest>>,
    request_counter: Mutex<u64>,
}

struct PendingRequest {
    requested_ids: HashMap<SecretId, Vec<SecretRequest>>,
    env_report: EnvReport,
    collected: Vec<SecretsBox>,
    threshold: usize,
    responder: oneshot::Sender<Result<SecretsBox, AppError>>,
}

impl SecretsService {
    pub fn new(
        p2p_secrets_sender: mpsc::Sender<SecretsMessage>,
        enclave_client: EnclaveClient,
    ) -> Arc<Self> {
        Arc::new(Self {
            p2p_secrets_sender,
            enclave_client: Mutex::new(enclave_client),
            pending: Mutex::new(HashMap::new()),
            request_counter: Mutex::new(0),
        })
    }

    /// Main entry point for secret retrieval.
    pub async fn get_secrets(
        &self,
        secret_requests: HashMap<SecretId, Vec<SecretRequest>>,
        env_report: EnvReport,
    ) -> Result<SecretsBox, AppError> {
        let (local_ids, missing) = self.check_local(&secret_requests).await?;
        if missing.is_empty() {
            let att = env_report.attestation.clone();
            let mut encl = self.enclave_client.lock().await;
            let sb = encl
                .get_secrets(local_ids, vec![], att)
                .await
                .map_err(|e| AppError::Service(format!("Enclave get_secrets: {e}")))?;
            return Ok(sb);
        }

        let request_id = {
            let mut rc = self.request_counter.lock().await;
            *rc += 1;
            *rc
        };
        let (tx, rx) = oneshot::channel();

        {
            let mut guard = self.pending.lock().await;
            guard.insert(
                request_id,
                PendingRequest {
                    requested_ids: missing.clone(),
                    env_report: env_report.clone(),
                    collected: Vec::new(),
                    threshold: 1,
                    responder: tx,
                },
            );
        }

        self.p2p_secrets_sender
            .clone()
            .try_send(SecretsMessage::PublishSecretsRequest {
                request_id,
                secret_requests: BTreeMap::from_iter(missing.into_iter()),
                env_report: env_report.clone(),
            })
            .map_err(|e| AppError::Service(format!("Failed to publish secrets request: {e}")))?;

        let p2p_box = match rx.await {
            Ok(Ok(sb)) => sb,
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(AppError::Service(format!("oneshot canceled: {e}"))),
        };

        {
            let mut encl = self.enclave_client.lock().await;
            let local_report = encl
                .get_report(vec![])
                .await
                .map_err(|e| AppError::Service(format!("Enclave get_report: {e}")))?;
            encl.put_secrets(vec![(p2p_box.clone(), local_report)])
                .await
                .map_err(|e| AppError::Service(format!("Enclave put_secrets: {e}")))?;
        }

        let att = env_report.attestation.clone();
        let mut encl = self.enclave_client.lock().await;
        let sb = encl
            .get_secrets(local_ids, vec![], att)
            .await
            .map_err(|e| AppError::Service(format!("Final get_secrets: {e}")))?;
        Ok(sb)
    }

    /// Called when we receive a p2p secrets response.
    pub async fn handle_incoming_secret_batch_response(
        &self,
        request_id: u64,
        secrets_box: SecretsBox,
    ) -> Result<(), AppError> {
        let finalize = {
            let mut lock = self.pending.lock().await;
            if let Some(p) = lock.get_mut(&request_id) {
                p.collected.push(secrets_box);
                if p.collected.len() >= p.threshold {
                    let merged = self.merge_boxes(p.collected.clone())?;
                    let responder = std::mem::replace(&mut p.responder, oneshot::channel().0);
                    Some((merged, responder, request_id))
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some((sb, responder, req_id)) = finalize {
            let mut lock = self.pending.lock().await;
            lock.remove(&req_id);
            let _ = responder.send(Ok(sb));
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
        let found = self.gather_local(&secret_requests).await;
        if found.is_empty() {
            return SecretsBox::new_empty();
        }
        let att = env_report.attestation.clone();
        let mut encl = self.enclave_client.lock().await;
        match encl.get_secrets(found, vec![], att).await {
            Ok(sb) => sb,
            Err(_) => SecretsBox::new_empty(),
        }
    }

    pub async fn timeout_pending_request(&self, request_id: u64) -> Result<(), AppError> {
        let mut lock = self.pending.lock().await;
        if let Some(p) = lock.remove(&request_id) {
            if p.collected.is_empty() {
                let _ = p
                    .responder
                    .send(Err(AppError::Service("No responses received".into())));
            } else if p.collected.len() < p.threshold {
                let _ = p
                    .responder
                    .send(Err(AppError::Service("Threshold not met".into())));
            } else {
                let merged = self.merge_boxes(p.collected)?;
                let _ = p.responder.send(Ok(merged));
            }
        }
        Ok(())
    }

    /// Check which secrets are locally available/unexpired in the enclave.
    async fn check_local(
        &self,
        requests: &HashMap<SecretId, Vec<SecretRequest>>,
    ) -> Result<(Vec<SecretId>, HashMap<SecretId, Vec<SecretRequest>>), AppError> {
        let mut all_ids = Vec::new();
        for sid in requests.keys() {
            all_ids.push(sid.clone());
        }
        if all_ids.is_empty() {
            return Ok((Vec::new(), HashMap::new()));
        }
        let mut encl = self.enclave_client.lock().await;
        let statuses = encl
            .check_secrets(all_ids.clone())
            .await
            .map_err(|e| AppError::Service(format!("check_secrets: {e}")))?;

        let now = current_unix_time();
        let mut local_set = HashSet::new();
        for (sid, found, expiry) in statuses {
            if found && (expiry == 0 || expiry > now) {
                local_set.insert(sid);
            }
        }

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
        let mut all_ids = Vec::new();
        for sid in requests.keys() {
            all_ids.push(sid.clone());
        }
        if all_ids.is_empty() {
            return Vec::new();
        }

        let mut encl = self.enclave_client.lock().await;
        match encl.check_secrets(all_ids.clone()).await {
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
            Err(_) => Vec::new(),
        }
    }

    fn merge_boxes(&self, boxes: Vec<SecretsBox>) -> Result<SecretsBox, AppError> {
        for sb in boxes {
            if !sb.encrypted_payload.is_empty() {
                return Ok(sb);
            }
        }
        Ok(SecretsBox::new_empty())
    }
}

fn current_unix_time() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(_) => 0,
    }
}
