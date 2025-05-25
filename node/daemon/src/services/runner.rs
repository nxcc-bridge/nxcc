use nxcc_interface::{
    proto::enclave::{
        ExecutePolicyRequest as ProtoExecutePolicyRequest, RunWorkerRequest,
        TerminateWorkerRequest, runner_client::RunnerClient,
    },
    types::{
        ConsumerInfo, EnvReport, FullPolicyPackage, PolicyExecutionRequest, SecretId, WorkerBundle,
        WorkerManifest,
    },
};
use tonic::transport::Channel;
use tracing::{debug, info, warn};

use crate::{config::EnclaveConfig, error::AppError};

/// Service responsible for managing worker execution via the enclave's Runner service.
#[derive(Clone)]
pub struct RunnerService {
    client: RunnerClient<Channel>,
    enclave_config: EnclaveConfig, // Needed for default_vm_id
}

impl RunnerService {
    pub fn new(client: RunnerClient<Channel>, enclave_config: EnclaveConfig) -> Self {
        Self {
            client,
            enclave_config,
        }
    }

    /// Attaches the configured default VM to the enclave runner.
    pub async fn attach_default_vm(&self) -> Result<bool, AppError> {
        info!(
            "Attaching default VM ({}) to enclave runner at UDS path {}...",
            self.enclave_config.default_vm_id, self.enclave_config.default_vm_uds_path
        );
        let address = nxcc_interface::proto::enclave::VmAddress {
            address_type: Some(
                nxcc_interface::proto::enclave::vm_address::AddressType::Uds(
                    nxcc_interface::proto::enclave::UdsAddress {
                        path: self.enclave_config.default_vm_uds_path.clone(),
                    },
                ),
            ),
        };
        let req = nxcc_interface::proto::enclave::AttachVmRequest {
            vm_id: self.enclave_config.default_vm_id.clone(),
            address: Some(address),
        };
        let mut client = self.client.clone();
        let resp = client
            .attach_vm(req)
            .await
            .map_err(|e| AppError::Service(format!("Failed to attach VM: {}", e)))?;
        let attached = resp.into_inner().attached;
        if attached {
            info!("Successfully attached default VM.");
        } else {
            warn!("Failed to attach default VM (enclave reported not attached).");
        }
        Ok(attached)
    }

    /// Runs a worker (typically a policy worker) in the pre-configured default VM.
    async fn run_worker_in_default_vm(
        &self,
        manifest: &WorkerManifest,
        bundle: &WorkerBundle,
    ) -> Result<String, AppError> {
        let manifest_bytes = serde_json::to_vec(manifest).unwrap();
        let bundle_bytes = bundle.0.clone();
        // todo!("add work order to run worker request for runner enclave to verify");
        let req = RunWorkerRequest {
            vm_id: self.enclave_config.default_vm_id.clone(), // TODO: extract this from manifest. check if VM is attached. if so, run. o/w error
            worker_manifest_bytes: manifest_bytes,
            worker_bundle_bytes: bundle_bytes,
        };
        let mut client = self.client.clone();
        let resp = client
            .run_worker(req)
            .await
            .map_err(|e| AppError::Service(format!("Enclave run_worker failed: {}", e)))?;
        let inner = resp.into_inner();
        if inner.success {
            debug!(
                "Successfully started worker {} in default VM for policy execution",
                inner.worker_id
            );
            Ok(inner.worker_id)
        } else {
            Err(AppError::Service(format!(
                "Enclave runner failed to start worker in default VM: {}",
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
        policy_package: FullPolicyPackage,
        env_report: &EnvReport,
        secret_id: &SecretId,
        consumer_info: &ConsumerInfo,
    ) -> Result<bool, AppError> {
        info!(
            "Executing policy check for secret {:?} against node {} for consumer bundle_hash {:?}",
            secret_id, env_report.node_id, consumer_info.bundle_hash
        );
        let FullPolicyPackage { manifest, bundle } = policy_package;

        // 1. Start Worker
        let worker_id = self.run_worker_in_default_vm(&manifest, &bundle).await?;

        info!(
            "Started policy worker {} for node {}",
            worker_id, env_report.node_id
        );

        // 2. Execute Policy
        let policy_request = PolicyExecutionRequest {
            secret_ids: vec![secret_id.clone()],
            consumer: consumer_info.clone(),
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
