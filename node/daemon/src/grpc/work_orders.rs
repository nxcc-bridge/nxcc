use std::sync::Arc;

use nxcc_interface::proto::daemon::{
    AttachVmRequest, AttachVmResponse, CheckWorkerStatusRequest, CheckWorkerStatusResponse,
    SubmitWorkOrderRequest, SubmitWorkOrderResponse, work_order_server::WorkOrder,
};
use tonic::{Request, Response, Status};
use tracing::info;

use crate::{
    grpc::enclave_client::EnclaveClient, services::work_order_orchestrator::WorkOrderOrchestrator,
};

pub struct WorkOrderGrpcService {
    orchestrator: Arc<WorkOrderOrchestrator>,
    enclave_client: EnclaveClient,
}

impl WorkOrderGrpcService {
    pub fn new(orchestrator: Arc<WorkOrderOrchestrator>, enclave_client: EnclaveClient) -> Self {
        Self {
            enclave_client,
            orchestrator,
        }
    }
}

#[tonic::async_trait]
impl WorkOrder for WorkOrderGrpcService {
    async fn submit_work_order(
        &self,
        request: Request<SubmitWorkOrderRequest>,
    ) -> Result<Response<SubmitWorkOrderResponse>, Status> {
        let req = request.into_inner();
        info!("Received gRPC SubmitWorkOrder request");

        match self
            .orchestrator
            .clone()
            .submit_work_order(req.work_order_dsse_bytes)
            .await
        {
            Ok((work_order_id, message)) => Ok(Response::new(SubmitWorkOrderResponse {
                work_order_id,
                success: true,
                message,
            })),
            Err(e) => {
                tracing::error!("SubmitWorkOrder failed: {:?}", e);
                // Return success=false in the payload for application-level errors
                Ok(Response::new(SubmitWorkOrderResponse {
                    work_order_id: String::new(),
                    success: false,
                    message: e.to_string(),
                }))
            }
        }
    }

    async fn check_worker_status(
        &self,
        request: Request<CheckWorkerStatusRequest>,
    ) -> Result<Response<CheckWorkerStatusResponse>, Status> {
        let req = request.into_inner();
        let work_order_id = req.work_order_id;

        info!("CheckWorkerStatus debug request: work_order_id='{work_order_id}'");

        let active_orders = self.orchestrator.active_work_orders.read().await;
        let active_order = match active_orders.get(&work_order_id) {
            Some(order) => order,
            None => {
                return Ok(Response::new(CheckWorkerStatusResponse {
                    found: false,
                    is_running: false,
                    status_message: "Work order not found or not active".to_string(),
                    worker_id: String::new(),
                    vm_id: String::new(),
                }));
            }
        };

        let enclave_worker_id = active_order.enclave_worker_id.clone();
        let vm_id = self.orchestrator.config.enclave.default_vm_id.clone();

        drop(active_orders);

        match self
            .enclave_client
            .check_worker_status(enclave_worker_id.clone())
            .await
        {
            Ok((status, message)) => {
                let is_running = status == nxcc_interface::proto::vm::WorkerStatus::Running;
                Ok(Response::new(CheckWorkerStatusResponse {
                    found: true,
                    is_running,
                    status_message: message,
                    worker_id: enclave_worker_id,
                    vm_id,
                }))
            }
            Err(e) => {
                tracing::error!("CheckWorkerStatus failed: {e}");
                Err(Status::internal(e))
            }
        }
    }
}
