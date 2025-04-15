use std::sync::Arc;

use interface::proto::enclave::{
    DeliverEventRequest, RunWorkerRequest, runner_client::RunnerClient,
};
use tokio::sync::Mutex;

/// Merged client for the runner portion of the enclave
pub struct RunnerService {
    inner: Arc<Mutex<RunnerClient<tonic::transport::Channel>>>,
}

impl RunnerService {
    pub fn new(client: RunnerClient<tonic::transport::Channel>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(client)),
        }
    }

    pub async fn run_worker(&self, worker_binary: Vec<u8>) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        let req = RunWorkerRequest { worker_binary };
        guard.run_worker(req).await.map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Delivers a batch of policy requests to the runner as an event payload
    pub async fn deliver_event(&self, worker_id: String, payload: Vec<u8>) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        let req = DeliverEventRequest {
            worker_id,
            event_payload: payload,
        };
        guard.deliver_event(req).await.map_err(|e| e.to_string())?;
        Ok(())
    }
}
