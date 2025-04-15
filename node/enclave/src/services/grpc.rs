use std::sync::Arc;
use tonic::{Request, Response, Status};

use crate::services::runner::RunnerService;
use crate::services::secrets::Secrets;
use interface::{
    proto::enclave::{
        CheckSecretsRequest, CheckSecretsResponse, DeliverEventRequest, DeliverEventResponse,
        GetReportRequest, GetSecretsEnclaveRequest, GetSecretsEnclaveResponse, PutSecretsRequest,
        PutSecretsResponse, RunWorkerRequest, RunWorkerResponse, SecretStatus as ProtoSecretStatus,
        enclave_secrets_server::EnclaveSecrets, runner_server::Runner,
    },
    types::{AttestationReport, SecretsBox},
};

pub struct EnclaveSecretsService {
    secrets: Arc<Secrets>,
}

impl EnclaveSecretsService {
    pub fn new(secrets: Arc<Secrets>) -> Self {
        Self { secrets }
    }
}

#[tonic::async_trait]
impl EnclaveSecrets for EnclaveSecretsService {
    async fn get_report(
        &self,
        request: Request<GetReportRequest>,
    ) -> Result<Response<interface::proto::interface::AttestationReport>, Status> {
        let req = request.into_inner();
        let ar = self.secrets.get_report(req.user_data);
        Ok(Response::new(ar.to_proto()))
    }

    async fn put_secrets(
        &self,
        request: Request<PutSecretsRequest>,
    ) -> Result<Response<PutSecretsResponse>, Status> {
        let req = request.into_inner();
        let mut bundles = Vec::new();
        for b in req.secrets_bundles {
            let proto_sb = b
                .secrets_box
                .ok_or_else(|| Status::invalid_argument("Missing secrets_box"))?;
            let proto_att = b
                .attestation_report
                .ok_or_else(|| Status::invalid_argument("Missing att_report"))?;
            let sb = SecretsBox::from_proto(proto_sb);
            let att = AttestationReport::from_proto(proto_att);
            bundles.push((sb, att));
        }
        let success = self.secrets.put_secrets(bundles);
        Ok(Response::new(PutSecretsResponse { success }))
    }

    async fn get_secrets(
        &self,
        request: Request<GetSecretsEnclaveRequest>,
    ) -> Result<Response<GetSecretsEnclaveResponse>, Status> {
        let req = request.into_inner();
        let mut ids = Vec::new();
        for sreq in req.requests {
            if let Some(si) = sreq.id {
                ids.push(interface::types::SecretId::from_proto(si));
            }
        }
        let proto_att = req
            .requester_attestation
            .ok_or_else(|| Status::invalid_argument("Missing requester_attestation"))?;
        let att = AttestationReport::from_proto(proto_att);
        let sb = self.secrets.get_secrets(ids, vec![], att);
        let resp = GetSecretsEnclaveResponse {
            secrets_box: Some(sb.to_proto()),
        };
        Ok(Response::new(resp))
    }

    async fn check_secrets(
        &self,
        request: Request<CheckSecretsRequest>,
    ) -> Result<Response<CheckSecretsResponse>, Status> {
        let req = request.into_inner();
        let mut ids = Vec::new();
        for pid in req.ids {
            ids.push(interface::types::SecretId::from_proto(pid));
        }
        let results = self.secrets.check_secrets(ids);
        let mut statuses = Vec::new();
        for (id, found, expiry) in results {
            let mut st = ProtoSecretStatus::default();
            st.id = Some(id.to_proto());
            st.found = found;
            st.expiry = expiry;
            statuses.push(st);
        }
        Ok(Response::new(CheckSecretsResponse { statuses }))
    }
}

pub struct EnclaveRunnerService {
    runner: RunnerService,
}

impl EnclaveRunnerService {
    pub fn new(runner: RunnerService) -> Self {
        Self { runner }
    }
}

#[tonic::async_trait]
impl Runner for EnclaveRunnerService {
    async fn run_worker(
        &self,
        request: Request<RunWorkerRequest>,
    ) -> Result<Response<RunWorkerResponse>, Status> {
        let r = request.into_inner();
        match self.runner.run_worker(r.worker_binary).await {
            Ok(_) => Ok(Response::new(RunWorkerResponse { accepted: true })),
            Err(e) => Err(Status::internal(format!("run_worker failed: {e}"))),
        }
    }

    async fn deliver_event(
        &self,
        request: Request<DeliverEventRequest>,
    ) -> Result<Response<DeliverEventResponse>, Status> {
        let r = request.into_inner();
        match self
            .runner
            .deliver_event(r.worker_id, r.event_payload)
            .await
        {
            Ok(_) => Ok(Response::new(DeliverEventResponse { delivered: true })),
            Err(e) => Err(Status::internal(format!("deliver_event failed: {e}"))),
        }
    }
}
