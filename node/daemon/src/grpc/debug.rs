use std::{collections::HashMap, sync::Arc};

use nxcc_interface::{
    proto::daemon::{
        AttachVmRequest, AttachVmResponse, CheckWorkerStatusRequest, CheckWorkerStatusResponse,
        debug_server::Debug,
    },
    types::{
        attestation::EnvReport,
        secrets::{SecretId, SecretsBox},
    },
};
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};

use crate::{
    grpc::enclave_client::EnclaveClient, http_server::VmRegistry,
    services::work_order_orchestrator::WorkOrderOrchestrator,
};

pub struct DebugGrpc {
    enclave_client: EnclaveClient,
    orchestrator: Arc<WorkOrderOrchestrator>,
    vm_registry: VmRegistry,
}

impl DebugGrpc {
    pub fn new(
        enclave_client: EnclaveClient,
        orchestrator: Arc<WorkOrderOrchestrator>,
        vm_registry: VmRegistry,
    ) -> Self {
        Self {
            enclave_client,
            orchestrator,
            vm_registry,
        }
    }
}

#[tonic::async_trait]
impl Debug for DebugGrpc {
    async fn attach_vm(
        &self,
        request: Request<AttachVmRequest>,
    ) -> Result<Response<AttachVmResponse>, Status> {
        let req = request.into_inner();

        let vm_id = if req.vm_id.is_empty() {
            req.uds_path.clone()
        } else {
            req.vm_id
        };

        let uds_path = req.uds_path;

        tracing::info!("AttachVm debug request: vm_id='{vm_id}', uds_path='{uds_path}'");

        match self.enclave_client.attach_vm(vm_id.clone(), uds_path).await {
            Ok(attached) => {
                if attached {
                    // Register the VM in our local registry
                    self.vm_registry.add_vm(vm_id).await;
                }
                Ok(Response::new(AttachVmResponse { success: attached }))
            }
            Err(e) => {
                tracing::error!("AttachVm failed: {e}");
                Err(Status::internal(e))
            }
        }
    }
}
