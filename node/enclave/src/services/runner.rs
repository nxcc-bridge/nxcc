use interface::proto::enclave::{
    DeliverEventRequest, DeliverEventResponse, RunWorkerRequest, RunWorkerResponse,
    runner_server::Runner,
};
use tonic::{Request, Response, Status};

#[derive(Default)]
pub struct RunnerService;

#[tonic::async_trait]
impl Runner for RunnerService {
    async fn run_worker(
        &self,
        _request: Request<RunWorkerRequest>,
    ) -> Result<Response<RunWorkerResponse>, Status> {
        todo!("Implement run_worker in enclave's secrets service (or move to runner)");
    }

    async fn deliver_event(
        &self,
        _request: Request<DeliverEventRequest>,
    ) -> Result<Response<DeliverEventResponse>, Status> {
        todo!("Implement deliver_event in enclave's secrets service");
    }
}
