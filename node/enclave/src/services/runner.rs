use crate::services::secrets::Secrets;
use ciborium::de::from_reader;
use interface::types::{PolicyExecutionReport, PolicyExecutionRequest};
use std::sync::Arc;

/// Orchestrates policy worker logic. In reality, it would dispatch to a WASM or execution enclave.
pub struct RunnerService {
    secrets: Arc<Secrets>,
}

impl RunnerService {
    pub fn new(secrets: Arc<Secrets>) -> Self {
        Self { secrets }
    }

    /// Stub for spawning or reusing a policy worker.
    pub async fn run_worker(&self, _worker_binary: Vec<u8>) -> Result<(), String> {
        Ok(())
    }

    /// Delivers a batch of PolicyExecutionRequest objects. We do not use `bincode` or `chrono`.
    pub async fn deliver_event(
        &self,
        _worker_id: String,
        event_payload: Vec<u8>,
    ) -> Result<(), String> {
        let requests: Vec<PolicyExecutionRequest> = from_reader(event_payload.as_slice())
            .map_err(|e| format!("Failed to parse requests: {e}"))?;

        for req in requests {
            // In a real scenario, evaluate policy. For now, always true.
            let dec = true;
            if dec {
                let rep = PolicyExecutionReport {
                    request: req.clone(),
                    decision: true,
                    timestamp: current_unix_time(),
                };
                self.secrets.store_authorization(rep);
            }
        }
        Ok(())
    }
}

fn current_unix_time() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(_) => 0,
    }
}
