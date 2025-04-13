use interface::{
    proto::enclave::{
        CheckSecretsRequest, GetReportRequest, GetSecretsEnclaveRequest, PolicyExecutionReport,
        PutSecretsRequest, PutSecretsResponse, SecretEnclaveRequest,
        SecretsBundle as ProtoSecretsBundle,
        enclave_secrets_client::EnclaveSecretsClient as ProtoEnclaveSecretsClient,
    },
    types::{AttestationReport, SecretId, SecretsBox},
};
use tokio_vsock::VsockStream;
use tonic::{
    codegen::http::Uri as HttpUri,
    transport::{Channel, Endpoint, Uri},
};
use tracing::debug;

#[derive(Clone)]
pub struct EnclaveClient {
    inner: ProtoEnclaveSecretsClient<Channel>,
}

impl EnclaveClient {
    /// Create a client that connects via a Unix domain socket
    pub async fn connect_uds(path: String) -> Result<Self, Box<dyn std::error::Error>> {
        #[cfg(unix)]
        {
            use hyper_util::rt::TokioIo;
            use tokio::net::UnixStream;
            use tonic::transport::{Endpoint, Uri};
            use tower::service_fn;

            // Create an endpoint with a dummy URI (not used for UDS)
            let channel = Endpoint::try_from("http://[::]:50051")?
                .connect_with_connector(service_fn(move |_: Uri| {
                    let path = path.to_string();
                    async move {
                        // Connect to the Unix domain socket
                        let stream = UnixStream::connect(path).await?;
                        Ok::<_, std::io::Error>(TokioIo::new(stream))
                    }
                }))
                .await?;

            Ok(Self {
                inner: ProtoEnclaveSecretsClient::new(channel),
            })
        }

        #[cfg(not(unix))]
        {
            Err("Unix domain sockets are not supported on this platform".into())
        }
    }

    /// Create a client that connects via vsock. Requires a custom connector.
    pub async fn connect_vsock(cid: u32, port: u32) -> Result<Self, Box<dyn std::error::Error>> {
        use hyper_util::rt::TokioIo;
        use tonic::transport::{Endpoint, Uri};
        use tower::service_fn;

        // Create an endpoint with a dummy URI (not used for vsock)
        let channel = Endpoint::try_from("http://[::]:50051")?
            .connect_with_connector(service_fn(move |_: Uri| {
                let cid = cid;
                let port = port;
                async move {
                    // Connect to vsock
                    let addr = tokio_vsock::VsockAddr::new(cid, port);
                    let stream = tokio_vsock::VsockStream::connect(addr).await?;
                    Ok::<_, std::io::Error>(TokioIo::new(stream))
                }
            }))
            .await?;

        Ok(Self {
            inner: ProtoEnclaveSecretsClient::new(channel),
        })
    }

    pub async fn get_report(&mut self, user_data: Vec<u8>) -> Result<AttestationReport, String> {
        let req = GetReportRequest { user_data };
        let resp = self
            .inner
            .get_report(req)
            .await
            .map_err(|e| e.to_string())?;
        Ok(AttestationReport::from_proto(resp.into_inner()))
    }

    pub async fn put_secrets(
        &mut self,
        boxes: Vec<(SecretsBox, AttestationReport)>,
    ) -> Result<bool, String> {
        let mut bundles = Vec::new();
        for (sb, att) in boxes {
            let sb_proto = sb.to_proto();
            let att_proto = att.to_proto();
            bundles.push(ProtoSecretsBundle {
                secrets_box: Some(sb_proto),
                attestation_report: Some(att_proto),
            });
        }

        let req = PutSecretsRequest {
            secrets_bundles: bundles,
        };
        let resp: PutSecretsResponse = self
            .inner
            .put_secrets(req)
            .await
            .map_err(|e| e.to_string())?
            .into_inner();
        Ok(resp.success)
    }

    /// get_secrets for the provided IDs. The policy reports are ignored in this demo.
    pub async fn get_secrets(
        &mut self,
        ids: Vec<SecretId>,
        policy_reports: Vec<(Vec<u8>, Vec<u8>)>,
        requester_report: AttestationReport,
    ) -> Result<SecretsBox, String> {
        let mut requests = Vec::new();
        for id in ids {
            requests.push(SecretEnclaveRequest {
                id: Some(id.to_proto()),
            });
        }
        let pr_proto = policy_reports
            .iter()
            .map(|(h, s)| PolicyExecutionReport {
                content_hash: h.clone(),
                signature: s.clone(),
            })
            .collect();

        let req = GetSecretsEnclaveRequest {
            requests,
            policy_reports: pr_proto,
            requester_attestation: Some(requester_report.to_proto()),
        };

        let resp = self
            .inner
            .get_secrets(req)
            .await
            .map_err(|e| e.to_string())?;
        let r = resp.into_inner();
        match r.secrets_box {
            Some(pb) => Ok(SecretsBox::from_proto(pb)),
            None => Err("Enclave returned no secrets_box".to_string()),
        }
    }

    pub async fn check_secrets(
        &mut self,
        ids: Vec<SecretId>,
    ) -> Result<Vec<(SecretId, bool, u64)>, String> {
        let proto_ids = ids.iter().map(|id| id.to_proto()).collect();

        let req = CheckSecretsRequest { ids: proto_ids };
        let resp = self
            .inner
            .check_secrets(req)
            .await
            .map_err(|e| e.to_string())?;
        let r = resp.into_inner();
        let mut out = Vec::new();
        for st in r.statuses {
            if let Some(pid) = st.id {
                let domain_id = SecretId::from_proto(pid);
                out.push((domain_id, st.found, st.expiry));
            }
        }
        Ok(out)
    }
}
