use std::sync::Arc;

use nxcc_interface::proto::daemon::{
    SubmitWorkOrderRequest, SubmitWorkOrderResponse, work_order_server::WorkOrder,
};
use tonic::{Request, Response, Status};
use tracing::info;

use crate::services::work_order_orchestrator::WorkOrderOrchestrator;

pub struct WorkOrderGrpcService {
    orchestrator: Arc<WorkOrderOrchestrator>,
}

impl WorkOrderGrpcService {
    pub fn new(orchestrator: Arc<WorkOrderOrchestrator>) -> Self {
        Self { orchestrator }
    }
}

#[tonic::async_trait]
impl WorkOrder for WorkOrderGrpcService {
    async fn submit_work_order(
        &self,
        request: Request<SubmitWorkOrderRequest>,
    ) -> Result<Response<SubmitWorkOrderResponse>, Status> {
        let req = request.into_inner();
        info!(
            "Received gRPC SubmitWorkOrder request with DSSE bytes length: {}",
            req.work_order_dsse_bytes.len()
        );

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
}
