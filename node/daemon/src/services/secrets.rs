use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use ethers::types::{Address, H256};
use futures::channel::mpsc;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock, oneshot};
use tracing::debug;

use crate::{error::AppError, network::SecretsMessage};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretId {
    pub chain_id: u64,
    pub identity_address: Address,
    pub identity_id: H256,
}

#[derive(Debug, Clone)]
#[cfg_attr(debug_assertions, derive(Serialize, Deserialize))]
pub struct Secret {
    pub id: SecretId,
    pub data: Vec<u8>,
    pub metadata: Vec<u8>,
}

pub struct SecretsService {
    p2p_secrets_sender: mpsc::Sender<SecretsMessage>,
    store: RwLock<HashMap<SecretId, Secret>>,
    pending: Mutex<HashMap<u64, PendingRequest>>,
    request_counter: Mutex<u64>,
}

struct PendingRequest {
    requested_ids: Vec<SecretId>,
    threshold: usize,
    fallback_data: Vec<u8>,
    collected: Vec<Secret>,
    responder: oneshot::Sender<Result<Vec<Secret>, AppError>>,
}

impl SecretsService {
    pub fn new(p2p_secrets_sender: mpsc::Sender<SecretsMessage>) -> Arc<Self> {
        Arc::new(Self {
            p2p_secrets_sender,
            store: RwLock::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            request_counter: Mutex::new(0),
        })
    }

    pub async fn probe_secrets(
        &self,
        secret_ids: impl IntoIterator<Item = SecretId>,
    ) -> Result<HashSet<SecretId>, AppError> {
        let snapshot = self.store.read().await;
        let mut found = HashSet::new();
        for id in secret_ids {
            if snapshot.contains_key(&id) {
                found.insert(id);
            }
        }
        Ok(found)
    }

    pub async fn get_secrets(
        self: &Arc<Self>,
        secret_ids: Vec<SecretId>,
        payload: Vec<u8>,
    ) -> Result<Vec<Secret>, AppError> {
        let local_ids = self.probe_secrets(secret_ids.clone()).await?;
        let mut results = Vec::new();

        {
            let store_guard = self.store.read().await;
            for id in &local_ids {
                if let Some(s) = store_guard.get(id) {
                    results.push(s.clone());
                }
            }
        }

        let missing: Vec<_> = secret_ids
            .into_iter()
            .filter(|id| !local_ids.contains(id))
            .collect();

        if missing.is_empty() {
            debug!("All secrets requested are already stored locally");
            return Ok(results);
        }

        let threshold = 2;
        let request_id = {
            let mut counter = self.request_counter.lock().await;
            *counter += 1;
            *counter
        };

        let (tx, rx) = oneshot::channel();

        {
            let mut pending_guard = self.pending.lock().await;
            pending_guard.insert(
                request_id,
                PendingRequest {
                    requested_ids: missing.clone(),
                    threshold,
                    fallback_data: payload.clone(),
                    collected: Vec::new(),
                    responder: tx,
                },
            );
        }

        self.p2p_secrets_sender
            .clone()
            .try_send(SecretsMessage::PublishSecretsRequest {
                request_id,
                secret_ids: missing,
            })?;

        let service = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            service.timeout_pending_request(request_id).await;
        });

        let net_results = rx.await.map_err(|e| AppError::Service(e.to_string()))??;
        self.store_secrets(net_results.clone()).await?;
        results.extend(net_results);
        Ok(results)
    }

    pub async fn store_secrets(&self, secrets: Vec<Secret>) -> Result<(), AppError> {
        let mut guard = self.store.write().await;
        for s in secrets {
            guard.insert(s.id.clone(), s);
        }
        Ok(())
    }

    pub async fn handle_incoming_secret_batch_request(
        self: &Arc<Self>,
        request_id: u64,
        items: Vec<SecretId>,
    ) -> Vec<Secret> {
        let snapshot = self.store.read().await;
        let mut found = Vec::new();
        for i in &items {
            if let Some(sec) = snapshot.get(i) {
                found.push(sec.clone());
            }
        }
        found
    }

    pub async fn handle_incoming_secret_batch_response(
        &self,
        request_id: u64,
        secrets: Vec<Secret>,
    ) {
        let mut pending_guard = self.pending.lock().await;
        if let Some(req) = pending_guard.get_mut(&request_id) {
            req.collected.extend(secrets);
            if req.collected.len() >= req.threshold {
                if let Some(done) = pending_guard.remove(&request_id) {
                    let _ = done.responder.send(Ok(done.collected));
                }
            }
        }
    }

    async fn timeout_pending_request(&self, request_id: u64) {
        let mut pending_guard = self.pending.lock().await;
        if let Some(req) = pending_guard.remove(&request_id) {
            if !req.collected.is_empty() {
                let _ = req.responder.send(Ok(req.collected));
            } else {
                if let Some(first_id) = req.requested_ids.get(0) {
                    let fallback_secret = Secret {
                        id: first_id.clone(),
                        data: req.fallback_data,
                        metadata: b"local_fallback".to_vec(),
                    };
                    let _ = req.responder.send(Ok(vec![fallback_secret]));
                } else {
                    let _ = req.responder.send(Ok(Vec::new()));
                }
            }
        }
    }
}
