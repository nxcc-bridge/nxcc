use std::sync::Arc;

use interface::proto::enclave::{
    DeliverEventRequest, RunWorkerRequest, runner_client::RunnerClient,
};

/// Merged client for the runner portion of the enclave
#[derive(Clone)]
pub struct RunnerService {
    client: RunnerClient<tonic::transport::Channel>,
}

impl RunnerService {
    pub fn new(client: RunnerClient<tonic::transport::Channel>) -> Self {
        Self { client }
    }

    pub async fn run_worker(&self, worker_binary: Vec<u8>) -> Result<(), String> {
        let mut client = self.client.clone();
        let req = RunWorkerRequest { worker_binary };
        client.run_worker(req).await.map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Delivers a batch of policy requests to the runner as an event payload
    pub async fn deliver_event(&self, worker_id: String, payload: Vec<u8>) -> Result<(), String> {
        let mut client = self.client.clone();
        let req = DeliverEventRequest {
            worker_id,
            event_payload: payload,
        };
        client.deliver_event(req).await.map_err(|e| e.to_string())?;
        Ok(())
    }
}
