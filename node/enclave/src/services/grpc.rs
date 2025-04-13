use super::secrets::Secrets;
use interface::{
    proto::enclave::{
        CheckSecretsRequest, CheckSecretsResponse, GetReportRequest, GetSecretsEnclaveRequest,
        GetSecretsEnclaveResponse, PutSecretsRequest, PutSecretsResponse,
        SecretStatus as ProtoSecretStatus, enclave_secrets_server::EnclaveSecrets,
    },
    proto::interface::AttestationReport as ProtoAttestationReport,
    proto::interface::SecretsBox as ProtoSecretsBox,
    types::{AttestationReport, SecretId, SecretsBox},
};
use std::sync::Arc;
use tonic::{Request, Response, Status};
use tracing::debug;

pub struct EnclaveSecretsService {
    pub enclave: Arc<Secrets>,
}

#[tonic::async_trait]
impl EnclaveSecrets for EnclaveSecretsService {
    async fn get_report(
        &self,
        request: Request<GetReportRequest>,
    ) -> Result<Response<ProtoAttestationReport>, Status> {
        let req = request.into_inner();
        debug!("EnclaveSecrets::get_report called");
        let ar = self.enclave.get_report(req.user_data);
        Ok(Response::new(ar.to_proto()))
    }

    async fn put_secrets(
        &self,
        request: Request<PutSecretsRequest>,
    ) -> Result<Response<PutSecretsResponse>, Status> {
        let req = request.into_inner();
        debug!(
            "EnclaveSecrets::put_secrets received {} bundles",
            req.secrets_bundles.len()
        );
        let mut bundles = Vec::new();
        for sb in req.secrets_bundles {
            let sb_proto = sb
                .secrets_box
                .ok_or_else(|| Status::invalid_argument("missing secrets box"))?;
            let att_proto = sb
                .attestation_report
                .ok_or_else(|| Status::invalid_argument("missing attestation report"))?;
            bundles.push((
                SecretsBox::from_proto(sb_proto),
                AttestationReport::from_proto(att_proto),
            ));
        }
        let success = self.enclave.put_secrets(bundles);
        Ok(Response::new(PutSecretsResponse { success }))
    }

    async fn get_secrets(
        &self,
        request: Request<GetSecretsEnclaveRequest>,
    ) -> Result<Response<GetSecretsEnclaveResponse>, Status> {
        let req = request.into_inner();
        debug!(
            "EnclaveSecrets::get_secrets called with {} requests",
            req.requests.len()
        );
        let mut ids = Vec::new();
        for r in req.requests {
            let i = r.id.ok_or_else(|| Status::invalid_argument("missing id"))?;
            ids.push(SecretId::from_proto(i));
        }
        let mut policy_reports = Vec::new();
        for p in req.policy_reports {
            policy_reports.push((p.content_hash, p.signature));
        }
        let ar = AttestationReport::from_proto(
            req.requester_attestation
                .ok_or_else(|| Status::invalid_argument("missing attestation"))?,
        );
        let sb = self.enclave.get_secrets(ids, policy_reports, ar);
        Ok(Response::new(GetSecretsEnclaveResponse {
            secrets_box: Some(sb.to_proto()),
        }))
    }

    async fn check_secrets(
        &self,
        request: Request<CheckSecretsRequest>,
    ) -> Result<Response<CheckSecretsResponse>, Status> {
        let req = request.into_inner();
        debug!(
            "EnclaveSecrets::check_secrets called with {} IDs",
            req.ids.len()
        );
        let mut ids = Vec::new();
        for i in req.ids {
            ids.push(SecretId::from_proto(i));
        }
        let results = self.enclave.check_secrets(ids);
        let mut statuses = Vec::new();
        for (id, found, expiry) in results {
            statuses.push(ProtoSecretStatus {
                id: Some(id.to_proto()),
                found,
                expiry,
            });
        }
        Ok(Response::new(CheckSecretsResponse { statuses }))
    }
}
