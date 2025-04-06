use interface::{
    AttestationReport, Secret as DomainSecret, SecretId, SecretsBox,
    proto::enclave::{
        AttestationReport as ProtoAttestationReport, CheckSecretsRequest, GetReportRequest,
        GetSecretsEnclaveRequest, PolicyExecutionReport, PutSecretsRequest, PutSecretsResponse,
        SecretEnclaveRequest, SecretIdentifier as ProtoSecretId, SecretsBox as ProtoSecretsBox,
        SecretsBundle as ProtoSecretsBundle,
        enclave_secrets_client::EnclaveSecretsClient as ProtoEnclaveSecretsClient,
    },
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
        Ok(proto_attestation_to_domain(resp.into_inner()))
    }

    pub async fn put_secrets(
        &mut self,
        boxes: Vec<(SecretsBox, AttestationReport)>,
    ) -> Result<bool, String> {
        let mut bundles = Vec::new();
        for (sb, att) in boxes {
            bundles.push(ProtoSecretsBundle {
                secrets_box: Some(domain_secrets_box_to_proto(&sb)),
                attestation_report: Some(domain_attestation_to_proto(&att)),
            });
        }

        let req = PutSecretsRequest {
            secrets_bundles: bundles,
        };
        let resp = self
            .inner
            .put_secrets(req)
            .await
            .map_err(|e| e.to_string())?;
        Ok(resp.into_inner().success)
    }

    /// get_secrets for the provided secrets. The policy reports are ignored in this demo.
    pub async fn get_secrets(
        &mut self,
        ids: Vec<SecretId>,
        policy_reports: Vec<(Vec<u8>, Vec<u8>)>,
        requester_report: AttestationReport,
    ) -> Result<SecretsBox, String> {
        let mut requests = Vec::new();
        for id in ids {
            requests.push(SecretEnclaveRequest {
                id: Some(ProtoSecretId {
                    chain_id: id.chain_id,
                    identity_address: format!("{:x}", id.identity_address),
                    identity_id: format!("{:x}", id.identity_id),
                }),
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
            requester_attestation: Some(domain_attestation_to_proto(&requester_report)),
        };

        let resp = self
            .inner
            .get_secrets(req)
            .await
            .map_err(|e| e.to_string())?;
        let r = resp.into_inner();
        match r.secrets_box {
            Some(pb) => Ok(proto_secrets_box_to_domain(&pb)),
            None => Err("Enclave returned no secrets_box".to_string()),
        }
    }

    pub async fn check_secrets(
        &mut self,
        ids: Vec<SecretId>,
    ) -> Result<Vec<(SecretId, bool, u64)>, String> {
        let proto_ids = ids
            .iter()
            .map(|id| ProtoSecretId {
                chain_id: id.chain_id,
                identity_address: format!("{:x}", id.identity_address),
                identity_id: format!("{:x}", id.identity_id),
            })
            .collect();

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
                out.push((
                    SecretId {
                        chain_id: pid.chain_id,
                        identity_address: pid.identity_address.parse().expect("TODO"),
                        identity_id: pid.identity_id.parse().expect("TODO"),
                    },
                    st.found,
                    st.expiry,
                ));
            }
        }
        Ok(out)
    }
}

// -- Helper conversions --

fn domain_attestation_to_proto(a: &AttestationReport) -> ProtoAttestationReport {
    ProtoAttestationReport {
        ephemeral_public_key: a.ephemeral_public_key.clone(),
        block_hashes: a.block_hashes.clone(),
        user_data: a.user_data.clone(),
    }
}

fn proto_attestation_to_domain(a: ProtoAttestationReport) -> AttestationReport {
    AttestationReport {
        ephemeral_public_key: a.ephemeral_public_key,
        block_hashes: a.block_hashes,
        user_data: a.user_data,
    }
}

fn domain_secrets_box_to_proto(sb: &SecretsBox) -> ProtoSecretsBox {
    ProtoSecretsBox {
        encrypted_payload: sb.encrypted_payload.clone(),
        nonce: sb.nonce.clone(),
        sender_public_key: sb.sender_public_key.clone(),
        signature: sb.signature.clone(),
        alg: sb.alg.clone(),
    }
}

fn proto_secrets_box_to_domain(psb: &ProtoSecretsBox) -> SecretsBox {
    SecretsBox {
        encrypted_payload: psb.encrypted_payload.clone(),
        nonce: psb.nonce.clone(),
        sender_public_key: psb.sender_public_key.clone(),
        signature: psb.signature.clone(),
        alg: psb.alg.clone(),
    }
}
