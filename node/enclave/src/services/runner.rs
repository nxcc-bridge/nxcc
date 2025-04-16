use crate::services::secrets::Secrets;
use ciborium::de::from_reader;
use nxcc_interface::types::{PolicyExecutionReport, PolicyExecutionRequest};
use std::sync::Arc;
use tracing::{debug, error, info};

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
        // TODO: Implement worker loading/spawning
        info!("RunnerService: run_worker called (stub)");
        Ok(())
    }

    /// Delivers a batch of PolicyExecutionRequest objects, evaluates them (stubbed), and stores authorizations.
    pub async fn deliver_event(
        &self,
        worker_id: String, // TODO: Use worker_id to dispatch if multiple workers exist
        event_payload: Vec<u8>,
    ) -> Result<(), String> {
        info!(
            "RunnerService: deliver_event called for worker '{}' with payload size {}",
            worker_id,
            event_payload.len()
        );
        let requests: Vec<PolicyExecutionRequest> =
            from_reader(event_payload.as_slice()).map_err(|e| {
                error!("Failed to parse requests from event payload: {}", e);
                format!("Failed to parse requests: {e}")
            })?;
        debug!("Parsed {} policy execution requests", requests.len());

        for req in requests {
            // --- Policy Evaluation Simulation ---
            // In a real scenario, load the policy executable associated with req.secret_ids
            // (fetching if necessary, potentially using a PolicyManager similar to the daemon),
            // execute it in a sandboxed environment (e.g., WASM runtime), providing
            // req.consumer and req.env_report as input.
            // For now, we simulate a simple policy: always grant access.
            let decision = true; // Policy evaluation result (simulated)
            debug!(
                "Policy simulation for node '{}', {} secrets -> Decision: {}",
                req.env_report.node_id,
                req.secret_ids.len(),
                decision
            );
            // --- End Simulation ---

            let report = PolicyExecutionReport {
                request: req.clone(), // Clone request details into the report
                decision,
                timestamp: current_unix_time(),
            };

            // Store the authorization result (only if positive) in the Secrets service
            if report.decision {
                self.secrets.store_authorization(report);
            } else {
                info!(
                    "Policy denied access for node '{}' regarding secrets {:?}",
                    req.env_report.node_id, req.secret_ids
                );
            }
        }
        Ok(())
    }
}

fn current_unix_time() -> u64 {
    chrono::Utc::now().timestamp() as u64
}
