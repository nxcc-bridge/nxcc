use std::{collections::HashMap, time::Instant};

use nxcc_interface::{
    proto::vm::{TrustedConfig, UntrustedConfig},
    types::{
        secrets::ConsumerInfo,
        vm::VmAddress,
        worker::{WorkerBundle, WorkerManifest},
    },
};
#[cfg(test)]
use nxcc_vm_base::client::mock::MockVmServiceClient;
use nxcc_vm_base::{
    client::{ClientError, VmServiceClient},
    tls::MtlsCertificates,
};
use tracing::{error, info, warn};

use super::{RunnerError, RunnerService};

impl RunnerService {
    /// Establishes a connection to a VM and stores it.
    pub async fn attach_vm(&self, vm_id: String, address: VmAddress) -> Result<bool, RunnerError> {
        info!("Attempting to attach VM '{}' at {:?}", vm_id, address);

        // Generate ephemeral mTLS certs for this connection attempt
        let certs = MtlsCertificates::new()?;
        let client_tls_config = certs.client_tls_config()?;

        let client_result = match address {
            VmAddress::Tcp(tcp) => {
                let addr_str = format!("{}:{}", tcp.host, tcp.port);
                let socket_addr: std::net::SocketAddr = addr_str
                    .parse()
                    .map_err(|e| RunnerError::Internal(format!("Invalid TCP address: {e}")))?;
                VmServiceClient::connect(socket_addr, client_tls_config).await
            }
            #[cfg(feature = "uds")]
            VmAddress::Uds(uds) => VmServiceClient::connect_uds(uds.path, client_tls_config).await,
            #[cfg(not(feature = "uds"))]
            VmAddress::Uds(uds) => {
                return Err(RunnerError::UnsupportedVmAddress(format!(
                    "UDS address type not supported in this build: {}",
                    uds.path
                )));
            }
            #[cfg(feature = "vsock")]
            VmAddress::Vsock(vsock) => {
                VmServiceClient::connect_vsock(vsock.cid, vsock.port, client_tls_config).await
            }
            #[cfg(not(feature = "vsock"))]
            VmAddress::Vsock(vsock) => {
                return Err(RunnerError::UnsupportedVmAddress(format!(
                    "VSOCK address type not supported in this build: CID={}, Port={}",
                    vsock.cid, vsock.port
                )));
            }
        };

        match client_result {
            Ok(client) => {
                let mut vms_guard = self.vms.write().await;
                // Use the From implementation to convert to VmClient::Real
                vms_guard.insert(vm_id.clone(), client.into());
                info!("Successfully attached VM '{}'", vm_id);
                Ok(true)
            }
            Err(e) => {
                error!("Failed to connect and attach VM '{}': {}", vm_id, e);
                Err(e.into())
            }
        }
    }

    #[cfg(test)]
    pub async fn attach_mock_client(&self, vm_id: String, mock_client: MockVmServiceClient) {
        let mut vms_guard = self.vms.write().await;
        vms_guard.insert(vm_id, mock_client.into());
    }

    /// Removes a VM connection.
    pub async fn detach_vm(&self, vm_id: String) -> Result<(), RunnerError> {
        info!("Detaching VM '{}'", vm_id);
        let mut vms_guard = self.vms.write().await;
        if vms_guard.remove(&vm_id).is_some() {
            // Also remove any workers associated with this VM
            let mut worker_map_guard = self.worker_map.write().await;
            worker_map_guard.retain(|_worker_id, mapped_vm_id| *mapped_vm_id != vm_id);
            // TODO: attempt to kill the workers
            info!("Successfully detached VM '{}'", vm_id);
            Ok(())
        } else {
            warn!(
                "Attempted to detach VM '{}', but it was not attached.",
                vm_id
            );
            Ok(())
        }
    }

    /// Starts a worker in a specified VM.
    pub async fn run_worker(
        &self,
        worker_id: String,
        vm_id: String,
        worker_manifest: WorkerManifest,
        worker_bundle: WorkerBundle,
    ) -> Result<String, RunnerError> {
        let payload = worker_bundle
            .payload()
            .map_err(|e| RunnerError::Deserialization(e.to_string()))?;
        info!(
            "Requesting to run worker in VM '{}' (manifest user_data: {:?}, bundle payload size: \
             {})",
            vm_id,
            worker_manifest.userdata,
            payload.executable.len()
        );

        let mut worker_secrets_for_vm = HashMap::new();
        if !worker_manifest.identities.is_empty() {
            let bundle_payload_hash = worker_bundle
                .hash_signed_payload()
                .map_err(|e| RunnerError::Deserialization(e.to_string()))?;
            let dsse_signature = worker_bundle
                .get_dsse_signature()
                .map_err(|e| RunnerError::Deserialization(e.to_string()))?;

            let worker_consumer_info = ConsumerInfo {
                bundle_hash: bundle_payload_hash,
                signature: dsse_signature,
            };

            match self
                .secrets
                .get_secrets_for_local_worker(
                    worker_manifest.identities.clone(), // Vec<(SecretId, String)>
                    worker_consumer_info,
                )
                .await
            {
                Ok(secrets_map) => {
                    worker_secrets_for_vm = secrets_map;
                    info!(
                        "Retrieved {} secrets for local worker",
                        worker_secrets_for_vm.len()
                    );
                }
                Err(e) => {
                    error!("Failed to get secrets for local worker: {}", e);
                    return Err(RunnerError::Internal(format!(
                        "Failed to get secrets for worker: {}",
                        e
                    )));
                }
            }
        }

        let untrusted_config = UntrustedConfig {
            userdata_json: serde_json::to_string(&worker_manifest.userdata).unwrap_or_default(),
            ..Default::default()
        };
        let trusted_config = TrustedConfig {
            secrets: worker_secrets_for_vm,
            limits: None,
        };

        let mut vms_guard = self.vms.write().await; // Use write lock as we need a mutable client
        let client = vms_guard
            .get_mut(&vm_id)
            .ok_or_else(|| RunnerError::VmNotAttached(vm_id.clone()))?;
        let worker_type_id = "policy-worker".to_string(); // TODO: placeholder

        match client
            .start_worker(
                worker_id.clone(), // Use the work order hash as worker ID
                payload.executable,
                untrusted_config,
                trusted_config,
            )
            .await
        {
            Ok(instance_id) => {
                info!(
                    "Successfully started worker instance '{}' in VM '{}'.",
                    instance_id, vm_id
                );
                let mut worker_map_guard = self.worker_map.write().await;
                worker_map_guard.insert(instance_id.clone(), vm_id.clone());
                drop(worker_map_guard);
                Ok(instance_id)
            }
            Err(e) => {
                error!("Failed to start worker in VM '{}': {}", vm_id, e);
                // Map specific client errors if needed
                match e {
                    ClientError::Grpc(status) => {
                        Err(RunnerError::WorkerStartFailed(status.message().to_string()))
                    }
                    _ => Err(RunnerError::VmConnection(e)),
                }
            }
        }
    }

    /// Terminates a specific worker instance.
    pub async fn terminate_worker(&self, worker_id: String) -> Result<(), RunnerError> {
        info!("Requesting to terminate worker '{}'", worker_id);

        let vm_id = {
            let worker_map_guard = self.worker_map.read().await;
            worker_map_guard
                .get(&worker_id)
                .cloned()
                .ok_or_else(|| RunnerError::WorkerNotFound(worker_id.clone()))?
        };

        let mut vms_guard = self.vms.write().await; // Write lock for mutable client
        let client = vms_guard
            .get_mut(&vm_id)
            .ok_or_else(|| RunnerError::VmNotAttached(vm_id.clone()))?; // Should not happen if worker_map is consistent

        match client.stop_worker(worker_id.clone()).await {
            Ok(()) => {
                info!(
                    "Successfully requested termination for worker '{}'",
                    worker_id
                );
                // Move from active worker map to dead worker map
                let mut worker_map_guard = self.worker_map.write().await;
                worker_map_guard.remove(&worker_id);
                drop(worker_map_guard);

                let mut dead_worker_map_guard = self.dead_worker_map.write().await;
                dead_worker_map_guard.insert(worker_id.clone(), (vm_id.clone(), Instant::now()));
                Ok(())
            }
            Err(ClientError::Grpc(status)) if status.code() == tonic::Code::NotFound => {
                warn!(
                    "Worker '{}' not found in VM '{}' during termination request.",
                    worker_id, vm_id
                );
                // Move from active worker map to dead worker map if it exists locally
                let mut worker_map_guard = self.worker_map.write().await;
                worker_map_guard.remove(&worker_id);
                drop(worker_map_guard);

                let mut dead_worker_map_guard = self.dead_worker_map.write().await;
                dead_worker_map_guard.insert(worker_id.clone(), (vm_id.clone(), Instant::now()));
                Ok(())
            }
            Err(e) => {
                error!(
                    "Failed to request termination for worker '{}' in VM '{}': {}",
                    worker_id, vm_id, e
                );
                Err(e.into())
            }
        }
    }

    pub async fn check_worker_status(
        &self,
        worker_id: String,
    ) -> Result<(nxcc_interface::proto::vm::WorkerStatus, String), RunnerError> {
        info!("Checking status for worker '{}'", worker_id);

        let vm_id = {
            let worker_map_guard = self.worker_map.read().await;
            worker_map_guard
                .get(&worker_id)
                .cloned()
                .ok_or_else(|| RunnerError::WorkerNotFound(worker_id.clone()))?
        };

        let mut vms_guard = self.vms.write().await;
        let client = vms_guard
            .get_mut(&vm_id)
            .ok_or_else(|| RunnerError::VmNotAttached(vm_id.clone()))?;

        client.probe_worker(worker_id.clone()).await.map_err(|e| {
            error!(
                "Probe worker failed for worker '{}' in VM '{}': {}",
                worker_id, vm_id, e
            );
            RunnerError::VmConnection(e)
        })
    }
    /// Gets the VM ID for a worker, checking both active and dead worker maps.
    pub async fn get_worker_vm_id(&self, worker_id: &str) -> Option<String> {
        // Check active workers first
        if let Some(vm_id) = self.worker_map.read().await.get(worker_id) {
            return Some(vm_id.clone());
        }

        // Check dead workers
        if let Some((vm_id, _death_time)) = self.dead_worker_map.read().await.get(worker_id) {
            return Some(vm_id.clone());
        }

        None
    }

    /// Gets static logs from a worker via its VM.
    pub async fn get_worker_logs(&self, worker_id: String) -> Result<String, RunnerError> {
        info!("Requesting to get logs for worker '{}'", worker_id);

        let vm_id = self
            .get_worker_vm_id(&worker_id)
            .await
            .ok_or_else(|| RunnerError::WorkerNotFound(worker_id.clone()))?;

        let mut vms_guard = self.vms.write().await;
        let client = vms_guard
            .get_mut(&vm_id)
            .ok_or_else(|| RunnerError::VmNotAttached(vm_id.clone()))?;

        match client.get_worker_logs(worker_id.clone()).await {
            Ok(logs) => {
                info!("Successfully retrieved logs for worker '{}'", worker_id);
                Ok(logs)
            }
            Err(e) => {
                error!("Failed to get logs for worker '{}': {}", worker_id, e);
                Err(RunnerError::VmConnection(e))
            }
        }
    }

    /// Streams logs from a worker via its VM.
    pub async fn stream_worker_logs(
        &self,
        worker_id: String,
        tail_lines: u32,
        follow: bool,
    ) -> Result<
        tokio_stream::wrappers::ReceiverStream<
            Result<nxcc_interface::proto::vm::StreamWorkerLogsResponse, tonic::Status>,
        >,
        RunnerError,
    > {
        info!("Requesting to stream logs for worker '{}'", worker_id);

        let vm_id = self
            .get_worker_vm_id(&worker_id)
            .await
            .ok_or_else(|| RunnerError::WorkerNotFound(worker_id.clone()))?;

        let mut vms_guard = self.vms.write().await;
        let client = vms_guard
            .get_mut(&vm_id)
            .ok_or_else(|| RunnerError::VmNotAttached(vm_id.clone()))?;

        client
            .stream_worker_logs(worker_id.clone(), tail_lines, follow)
            .await
            .map_err(|e| {
                error!(
                    "Log streaming failed for worker '{}' in VM '{}': {}",
                    worker_id, vm_id, e
                );
                RunnerError::VmConnection(e)
            })
    }
}
