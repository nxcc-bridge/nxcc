use nxcc_interface::{
    policy::PolicyBundle,
    proto::enclave::{
        ExecutePolicyRequest as ProtoExecutePolicyRequest, RunWorkerRequest,
        TerminateWorkerRequest, runner_client::RunnerClient,
    },
    types::{EnvReport, PolicyExecutionRequest, SecretId},
};
use tonic::transport::Channel;
use tracing::{debug, info, warn};

use crate::{config::EnclaveConfig, error::AppError};

/// Service responsible for managing worker execution via the enclave's Runner service.
#[derive(Clone)]
pub struct RunnerService {
    client: RunnerClient<Channel>,
    enclave_config: EnclaveConfig, // Needed for policy_vm_id
}

impl RunnerService {
    pub fn new(client: RunnerClient<Channel>, enclave_config: EnclaveConfig) -> Self {
        Self {
            client,
            enclave_config,
        }
    }

    /// Attaches the configured policy VM to the enclave runner.
    pub async fn attach_policy_vm(&self) -> Result<bool, AppError> {
        info!(
            "Attaching policy VM ({}) to enclave runner at UDS path {}...",
            self.enclave_config.policy_vm_id, self.enclave_config.policy_vm_uds_path
        );
        let address = nxcc_interface::proto::enclave::VmAddress {
            address_type: Some(
                nxcc_interface::proto::enclave::vm_address::AddressType::Uds(
                    nxcc_interface::proto::enclave::UdsAddress {
                        path: self.enclave_config.policy_vm_uds_path.clone(),
                    },
                ),
            ),
        };
        let req = nxcc_interface::proto::enclave::AttachVmRequest {
            vm_id: self.enclave_config.policy_vm_id.clone(),
            address: Some(address),
        };
        let mut client = self.client.clone();
        let resp = client
            .attach_vm(req)
            .await
            .map_err(|e| AppError::Service(format!("Failed to attach VM: {}", e)))?;
        let attached = resp.into_inner().attached;
        if attached {
            info!("Successfully attached policy VM.");
        } else {
            warn!("Failed to attach policy VM (enclave reported not attached).");
        }
        Ok(attached)
    }

    /// Runs a policy worker in the pre-configured policy VM.
    async fn run_policy_worker(&self, policy: PolicyBundle) -> Result<String, AppError> {
        let req = RunWorkerRequest {
            vm_id: self.enclave_config.policy_vm_id.clone(),
            worker_code: policy.executable,
            manifest: serde_json::to_vec(&policy.manifest).unwrap(), // Panic on internal error
        };
        let mut client = self.client.clone();
        let resp = client
            .run_worker(req)
            .await
            .map_err(|e| AppError::Service(format!("Enclave run_worker failed: {}", e)))?;
        let inner = resp.into_inner();
        if inner.success {
            debug!("Successfully started policy worker {}", inner.worker_id);
            Ok(inner.worker_id)
        } else {
            Err(AppError::Service(format!(
                "Enclave runner failed to start policy worker: {}",
                inner.error_message
            )))
        }
    }

    /// Executes a policy worker against a batch of contexts.
    async fn execute_policy(
        &self,
        worker_id: String,
        contexts: Vec<PolicyExecutionRequest>,
    ) -> Result<Vec<PolicyExecutionRequest>, AppError> {
        let proto_contexts = contexts.iter().cloned().map(Into::into).collect();
        let req = ProtoExecutePolicyRequest {
            worker_id,
            contexts: proto_contexts,
        };
        let mut client = self.client.clone();
        let resp = client
            .execute_policy(req)
            .await
            .map_err(|e| AppError::Service(format!("Enclave execute_policy failed: {}", e)))?
            .into_inner();
        let satisfied = resp
            .satisfied_contexts
            .into_iter()
            .map(PolicyExecutionRequest::from)
            .collect();
        Ok(satisfied)
    }

    /// Terminates a specific worker instance.
    async fn terminate_worker(&self, worker_id: String) -> Result<(), AppError> {
        let req = TerminateWorkerRequest {
            worker_id: worker_id.clone(),
        };
        let mut client = self.client.clone();
        client
            .terminate_worker(req)
            .await
            .map_err(|e| AppError::Service(format!("Enclave terminate_worker failed: {}", e)))?;
        debug!("Successfully terminated worker {}", worker_id);
        Ok(())
    }

    /// Executes the policy for a given secret against a provided EnvReport.
    /// Handles starting, executing, and terminating the policy worker.
    /// Returns Ok(true) if satisfied, Ok(false) if denied, Err if execution failed.
    pub async fn check_policy_for_env(
        &self,
        policy: PolicyBundle,
        env_report: &EnvReport,
        secret_id: &SecretId,
    ) -> Result<bool, AppError> {
        info!(
            "Executing policy check for secret {:?} against node {}",
            secret_id, env_report.node_id
        );

        // 1. Start Worker
        let worker_id = self.run_policy_worker(policy).await?;
        info!(
            "Started policy worker {} for node {}",
            worker_id, env_report.node_id
        );

        // 2. Execute Policy
        let policy_request = PolicyExecutionRequest {
            secret_ids: vec![secret_id.clone()],
            consumer: nxcc_interface::types::ConsumerInfo {
                // TODO: Populate consumer info if needed/available
                code_hash: vec![],
                signature: vec![],
            },
            env_report: env_report.clone(),
        };

        let satisfied_contexts_result = self
            .execute_policy(worker_id.clone(), vec![policy_request])
            .await;

        // 3. Terminate Worker (best effort, even if execution failed)
        if let Err(e) = self.terminate_worker(worker_id.clone()).await {
            warn!("Failed to terminate policy worker {}: {}", worker_id, e);
        } else {
            info!("Terminated policy worker {}", worker_id);
        }

        // 4. Check Result
        let satisfied_contexts = satisfied_contexts_result?; // Propagate execution error
        Ok(!satisfied_contexts.is_empty()) // True if the context was returned as satisfied
    }
}
