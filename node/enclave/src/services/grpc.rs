use std::sync::Arc;
use tonic::{Request, Response, Status};
use tracing::debug;

use interface::{
    AttestationReport, SecretId, SecretsBox,
    proto::enclave::{
        AttestationReport as ProtoAttestationReport, CheckSecretsRequest, CheckSecretsResponse,
        GetReportRequest, GetSecretsEnclaveRequest, GetSecretsEnclaveResponse, PutSecretsRequest,
        PutSecretsResponse, SecretIdentifier as ProtoSecretId, SecretStatus as ProtoSecretStatus,
        SecretsBox as ProtoSecretsBox,
        enclave_secrets_server::EnclaveSecrets,
    },
};

use super::secrets::SecretsEnclave;

// Helper: convert from domain AttestationReport to proto
fn attestation_report_to_proto(ar: &AttestationReport) -> ProtoAttestationReport {
    ProtoAttestationReport {
        ephemeral_public_key: ar.ephemeral_public_key.clone(),
        block_hashes: ar.block_hashes.clone(),
        user_data: ar.user_data.clone(),
    }
}

// Helper: convert from proto to domain AttestationReport
fn attestation_report_from_proto(pa: &ProtoAttestationReport) -> AttestationReport {
    AttestationReport {
        ephemeral_public_key: pa.ephemeral_public_key.clone(),
        block_hashes: pa.block_hashes.clone(),
        user_data: pa.user_data.clone(),
    }
}

// Helper: convert from domain SecretsBox to proto
fn secrets_box_to_proto(sb: &SecretsBox) -> ProtoSecretsBox {
    ProtoSecretsBox {
        encrypted_payload: sb.encrypted_payload.clone(),
        nonce: sb.nonce.clone(),
        sender_public_key: sb.sender_public_key.clone(),
        signature: sb.signature.clone(),
        alg: sb.alg.clone(),
    }
}

// Helper: convert from proto to domain SecretsBox
fn secrets_box_from_proto(psb: &ProtoSecretsBox) -> SecretsBox {
    SecretsBox {
        encrypted_payload: psb.encrypted_payload.clone(),
        nonce: psb.nonce.clone(),
        sender_public_key: psb.sender_public_key.clone(),
        signature: psb.signature.clone(),
        alg: psb.alg.clone(),
    }
}

fn proto_secret_id_to_domain(id: &ProtoSecretId) -> SecretId {
    SecretId {
        chain_id: id.chain_id,
        identity_address: id.identity_address.parse().expect("TODO"),
        identity_id: id.identity_id.parse().expect("TODO"),
    }
}

pub struct EnclaveSecretsService {
    pub enclave: Arc<SecretsEnclave>,
}

#[tonic::async_trait]
impl EnclaveSecrets for EnclaveSecretsService {
    async fn get_report(
        &self,
        request: Request<GetReportRequest>,
    ) -> Result<Response<ProtoAttestationReport>, Status> {
        let req = request.into_inner();
        debug!("EnclaveSecrets::get_report called");

        let domain_ar = self.enclave.get_report(req.user_data);
        let proto_ar = attestation_report_to_proto(&domain_ar);
        Ok(Response::new(proto_ar))
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

        let mut domain_bundles = Vec::new();
        for sb in req.secrets_bundles {
            let domain_box = secrets_box_from_proto(
                &sb.secrets_box
                    .ok_or_else(|| Status::invalid_argument("missing secrets box"))?,
            );
            let domain_attestation = attestation_report_from_proto(
                &sb.attestation_report
                    .ok_or_else(|| Status::invalid_argument("missing attestation report"))?,
            );
            domain_bundles.push((domain_box, domain_attestation));
        }

        let success = self.enclave.put_secrets(domain_bundles);
        let resp = PutSecretsResponse { success };
        Ok(Response::new(resp))
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

        let mut domain_ids = Vec::new();
        for r in req.requests {
            domain_ids
                .push(proto_secret_id_to_domain(&r.id.ok_or_else(|| {
                    Status::invalid_argument("missing request id")
                })?));
        }

        // For simplicity, policy_reports are a list of (content_hash, signature).
        // We won't do anything with them here. In a real system, we'd verify them.
        let mut domain_reports = Vec::new();
        for pr in req.policy_reports {
            domain_reports.push((pr.content_hash, pr.signature));
        }

        let requester_ar = attestation_report_from_proto(
            &req.requester_attestation
                .ok_or_else(|| Status::invalid_argument("missing requester attestation"))?,
        );

        let secrets_box = self
            .enclave
            .get_secrets(domain_ids, domain_reports, requester_ar);
        let proto_box = secrets_box_to_proto(&secrets_box);

        let resp = GetSecretsEnclaveResponse {
            secrets_box: Some(proto_box),
        };
        Ok(Response::new(resp))
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

        let mut domain_ids = Vec::new();
        for p in &req.ids {
            domain_ids.push(proto_secret_id_to_domain(p));
        }

        let results = self.enclave.check_secrets(domain_ids);
        let mut statuses = Vec::new();
        for (id, found, expiry) in results {
            statuses.push(ProtoSecretStatus {
                id: Some(ProtoSecretId {
                    chain_id: id.chain_id,
                    identity_address: format!("{:x}", id.identity_address),
                    identity_id: format!("{:x}", id.identity_id),
                }),
                found,
                expiry,
            });
        }
        Ok(Response::new(CheckSecretsResponse { statuses }))
    }
}
