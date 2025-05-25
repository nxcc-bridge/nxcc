use std::{collections::HashMap, sync::Arc};

use nxcc_interface::{
    proto::daemon::{AttachVmRequest, AttachVmResponse, debug_server::Debug},
    types::{AttestationReport, EnvReport, SecretId, SecretsBox},
};
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};

use crate::grpc::enclave_client::EnclaveClient;

pub struct DebugGrpc {
    enclave_client: EnclaveClient,
}

impl DebugGrpc {
    pub fn new(enclave_client: EnclaveClient) -> Self {
        Self { enclave_client }
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

        match self.enclave_client.attach_vm(vm_id, uds_path).await {
            Ok(attached) => Ok(Response::new(AttachVmResponse { success: true })),
            Err(e) => {
                tracing::error!("AttachVm failed: {e}");
                Err(Status::internal(e))
            }
        }
    }
}
