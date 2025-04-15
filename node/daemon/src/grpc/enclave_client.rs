use hyper_util::rt::TokioIo;
use interface::{
    proto::enclave::{
        CheckSecretsRequest, DeliverEventRequest, DeliverEventResponse, GetReportRequest,
        GetSecretsEnclaveRequest, PutSecretsRequest, PutSecretsResponse, RunWorkerRequest,
        SecretEnclaveRequest, SecretsBundle as ProtoSecretsBundle,
        enclave_secrets_client::EnclaveSecretsClient, runner_client::RunnerClient,
    },
    types::{AttestationReport, EnvReport, FromProto as _, IntoProto as _, SecretId, SecretsBox},
};
use tokio::net::UnixStream;
use tonic::{
    codegen::http::Uri as HttpUri,
    transport::{Channel, Endpoint, Uri},
};
use tower::service_fn;

use crate::error::AppError;

/// A single client struct for both the secrets and runner services in the enclave.
#[derive(Clone)]
pub struct EnclaveClient {
    secrets_client: EnclaveSecretsClient<Channel>,
    runner_client: RunnerClient<Channel>,
}

impl EnclaveClient {
    pub async fn connect_uds(path: impl Into<String>) -> Result<Self, AppError> {
        let path = path.into();
        let channel = Endpoint::try_from("http://[::]:50051")
            .map_err(|e| AppError::Service(format!("Invalid endpoint: {e}")))?
            .connect_with_connector(service_fn(move |_: Uri| {
                let p = path.clone();
                async move {
                    let stream = UnixStream::connect(p).await?;
                    Ok::<_, std::io::Error>(TokioIo::new(stream))
                }
            }))
            .await
            .map_err(|e| AppError::Service(format!("UDS connect error: {e}")))?;

        Ok(Self {
            secrets_client: EnclaveSecretsClient::new(channel.clone()),
            runner_client: RunnerClient::new(channel),
        })
    }

    #[allow(unused)]
    pub async fn connect_vsock(_cid: u32, _port: u32) -> Result<Self, Box<dyn std::error::Error>> {
        Err("Vsock connect not implemented".into())
    }

    // Secrets interface calls

    pub async fn get_report(&self, user_data: Vec<u8>) -> Result<AttestationReport, String> {
        let mut client = self.secrets_client.clone();
        let req = GetReportRequest { user_data };
        let resp = client.get_report(req).await.map_err(|e| e.to_string())?;
        Ok(AttestationReport::from_proto(resp.into_inner()))
    }

    pub async fn put_secrets(
        &self,
        bundles_with_reports: Vec<(SecretsBox, EnvReport)>,
    ) -> Result<bool, String> {
        let mut bundles = Vec::new();
        for (sb, env_report) in bundles_with_reports {
            bundles.push(ProtoSecretsBundle {
                secrets_box: Some(sb.to_proto()),
                env_report: Some(env_report.to_proto()),
            });
        }
        let req = PutSecretsRequest {
            secrets_bundles: bundles,
        };
        let mut client = self.secrets_client.clone();
        let resp = client.put_secrets(req).await.map_err(|e| e.to_string())?;
        Ok(resp.into_inner().success)
    }

    pub async fn get_secrets(
        &self,
        secret_ids: Vec<SecretId>,
        env_report: EnvReport,
    ) -> Result<SecretsBox, String> {
        let mut requests = Vec::new();
        for sid in secret_ids {
            requests.push(SecretEnclaveRequest {
                id: Some(sid.to_proto()),
            });
        }
        let req = GetSecretsEnclaveRequest {
            requests,
            requester_env_report: Some(env_report.to_proto()),
            policy_reports: vec![], // not used yet
        };
        let mut client = self.secrets_client.clone();
        let resp = client.get_secrets(req).await.map_err(|e| e.to_string())?;
        let out = resp.into_inner();
        if let Some(box_proto) = out.secrets_box {
            Ok(SecretsBox::from_proto(box_proto))
        } else {
            Err("Enclave returned no SecretsBox".to_string())
        }
    }

    pub async fn check_secrets(
        &self,
        ids: Vec<SecretId>,
    ) -> Result<Vec<(SecretId, bool, u64)>, String> {
        let mut proto_ids = Vec::new();
        for sid in ids.iter() {
            proto_ids.push(sid.to_proto());
        }
        let req = CheckSecretsRequest { ids: proto_ids };
        let mut client = self.secrets_client.clone();
        let resp = client.check_secrets(req).await.map_err(|e| e.to_string())?;
        let statuses = resp.into_inner().statuses;
        let mut out = Vec::new();
        for st in statuses {
            if let Some(proto_id) = st.id {
                let sid = SecretId::from_proto(proto_id);
                out.push((sid, st.found, st.expiry));
            }
        }
        Ok(out)
    }

    // Runner interface calls

    pub async fn run_worker(&self, worker_binary: Vec<u8>) -> Result<(), String> {
        let req = RunWorkerRequest { worker_binary };
        let mut client = self.runner_client.clone();
        client.run_worker(req).await.map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn deliver_event(&self, worker_id: String, payload: Vec<u8>) -> Result<(), String> {
        let req = DeliverEventRequest {
            worker_id,
            event_payload: payload,
        };
        let mut client = self.runner_client.clone();
        let _resp: DeliverEventResponse = client
            .deliver_event(req)
            .await
            .map_err(|e| e.to_string())?
            .into_inner();
        Ok(())
    }
}
