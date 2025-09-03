use hyper_util::rt::TokioIo;
use nxcc_interface::{
    proto::enclave::{
        AttachVmRequest, CheckSecretsRequest, CheckWorkerStatusRequest,
        DeliverBatchEventsRequest as ProtoDeliverBatchEventsRequest, DetachVmRequest,
        ExecutePolicyRequest as ProtoExecutePolicyRequest,
        ExecutePolicyResponse as ProtoExecutePolicyResponse, GenerateSecretsRequest,
        GetReportRequest, GetSecretsRequest, GetSecretsResponse,
        InvokeHttpWorkerRequest as ProtoInvokeHttpWorkerRequest,
        InvokeHttpWorkerResponse as ProtoInvokeHttpWorkerResponse, PutSecretsRequest,
        PutSecretsResponse, RunWorkerRequest, SecretsBundle as ProtoSecretsBundle,
        StreamWorkerLogsRequest, TerminateWorkerRequest, VmAddress as ProtoVmAddress,
        runner_client::RunnerClient, secrets_client::SecretsClient,
    },
    types::{
        attestation::{AttestationBundle, EnvReport},
        secrets::{ConsumerInfo, SecretId, SecretsBox},
    },
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
    secrets_client: SecretsClient<Channel>,
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
            secrets_client: SecretsClient::new(channel.clone()),
            runner_client: RunnerClient::new(channel),
        })
    }

    /// Returns the underlying secrets client.
    pub fn secrets(&self) -> SecretsClient<Channel> {
        self.secrets_client.clone()
    }

    /// Returns the underlying runner client.
    pub fn runner(&self) -> RunnerClient<Channel> {
        self.runner_client.clone()
    }

    #[allow(unused)]
    pub async fn connect_vsock(_cid: u32, _port: u32) -> Result<Self, Box<dyn std::error::Error>> {
        Err("Vsock connect not implemented".into())
    }

    // Secrets interface calls

    pub async fn get_report(&self) -> Result<AttestationBundle, String> {
        let mut client = self.secrets();
        let req = GetReportRequest {};
        let resp = client.get_report(req).await.map_err(|e| e.to_string())?;
        Ok(AttestationBundle::from(resp.into_inner()))
    }

    pub async fn put_secrets(
        &self,
        bundles_with_reports_and_consumers: Vec<(SecretsBox, EnvReport, ConsumerInfo)>,
    ) -> Result<bool, String> {
        let mut bundles_proto = Vec::new();
        for (sb, env_report, consumer_info) in bundles_with_reports_and_consumers {
            bundles_proto.push(ProtoSecretsBundle {
                secrets_box: Some(sb.into()),
                env_report: Some(env_report.into()),
                consumer_info: Some(consumer_info.into()),
            });
        }
        let req = PutSecretsRequest {
            secrets_bundles: bundles_proto,
        };
        let mut client = self.secrets();
        let resp = client.put_secrets(req).await.map_err(|e| e.to_string())?;
        Ok(resp.into_inner().success)
    }

    pub async fn get_secrets(
        &self,
        secret_requests_with_consumer: Vec<(SecretId, ConsumerInfo)>,
        env_report: EnvReport,
    ) -> Result<SecretsBox, String> {
        let mut proto_requests = Vec::new();
        for (sid, ci) in secret_requests_with_consumer {
            proto_requests.push(nxcc_interface::proto::interface::SecretRequest {
                secret_id: Some(sid.into()),
                consumer: Some(ci.into()),
            });
        }
        let req = GetSecretsRequest {
            requests: proto_requests,
            requester_env_report: Some(env_report.into()),
        };
        let mut client = self.secrets();
        let resp = client.get_secrets(req).await.map_err(|e| e.to_string())?;
        let out = resp.into_inner();
        if let Some(box_proto) = out.secrets_box {
            Ok(SecretsBox::try_from(box_proto)
                .map_err(|e| format!("Invalid SecretsBox from enclave: {}", e))?)
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
            proto_ids.push((*sid).clone().into());
        }
        let req = CheckSecretsRequest { ids: proto_ids };
        let mut client = self.secrets();
        let resp = client.check_secrets(req).await.map_err(|e| e.to_string())?;
        let statuses = resp.into_inner().statuses;
        let mut out = Vec::new();
        for st in statuses {
            if let Some(proto_id) = st.id {
                let sid = SecretId::try_from(proto_id)
                    .map_err(|e| format!("Invalid SecretIdentifier from enclave: {}", e))?;
                out.push((sid, st.found, st.expiry));
            }
        }
        Ok(out)
    }

    pub async fn generate_secrets(
        &self,
        requests_with_consumer: Vec<(SecretId, ConsumerInfo)>,
    ) -> Result<(), String> {
        let proto_requests = requests_with_consumer
            .into_iter()
            .map(
                |(sid, ci)| nxcc_interface::proto::interface::SecretRequest {
                    secret_id: Some(sid.into()),
                    consumer: Some(ci.into()),
                },
            )
            .collect();
        let req = GenerateSecretsRequest {
            requests: proto_requests,
        };
        let mut client = self.secrets();
        client
            .generate_secrets(req)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // Runner interface calls

    pub async fn attach_vm(&self, vm_id: String, vm_uds_path: String) -> Result<bool, String> {
        let address = ProtoVmAddress {
            address_type: Some(
                nxcc_interface::proto::enclave::vm_address::AddressType::Uds(
                    nxcc_interface::proto::enclave::UdsAddress { path: vm_uds_path },
                ),
            ),
        };
        let req = AttachVmRequest {
            vm_id,
            address: Some(address),
        };
        let mut client = self.runner();
        let resp = client.attach_vm(req).await.map_err(|e| e.to_string())?;
        Ok(resp.into_inner().attached)
    }

    pub async fn detach_vm(&self, vm_id: String) -> Result<bool, String> {
        let req = DetachVmRequest { vm_id };
        let mut client = self.runner();
        // TODO: DetachVmResponse doesn't have an `attached` field. Assuming Empty means success.
        // Need to update proto or adjust logic here based on actual enclave behavior.
        client.detach_vm(req).await.map_err(|e| e.to_string())?;
        Ok(true) // Placeholder: Assume success if no error
    }

    pub async fn run_worker(
        &self,
        vm_id: String,
        worker_manifest_bytes: Vec<u8>,
        worker_bundle_bytes: Vec<u8>,
    ) -> Result<String, String> {
        let req = RunWorkerRequest {
            vm_id,
            worker_manifest_bytes,
            worker_bundle_bytes,
        };
        let mut client = self.runner();
        let resp = client.run_worker(req).await.map_err(|e| e.to_string())?;
        let inner = resp.into_inner();
        if inner.success || !inner.worker_id.is_empty() {
            // Consider worker_id presence as success too
            Ok(inner.worker_id)
        } else {
            Err(format!(
                "Enclave runner failed to start worker: {}",
                inner.error_message
            ))
        }
    }

    pub async fn terminate_worker(&self, worker_id: String) -> Result<(), String> {
        let req = TerminateWorkerRequest { worker_id };
        let mut client = self.runner();
        client
            .terminate_worker(req)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn execute_policy(
        &self,
        worker_id: String,
        contexts: Vec<nxcc_interface::types::policy::PolicyExecutionRequest>,
    ) -> Result<Vec<nxcc_interface::types::policy::PolicyExecutionRequest>, String> {
        let proto_contexts = contexts.iter().cloned().map(Into::into).collect();
        let req = ProtoExecutePolicyRequest {
            worker_id,
            contexts: proto_contexts,
        };
        let mut client = self.runner();
        let resp: ProtoExecutePolicyResponse = client
            .execute_policy(req)
            .await
            .map_err(|e| e.to_string())?
            .into_inner();
        let satisfied = resp
            .satisfied_contexts
            .into_iter()
            .map(|p| {
                nxcc_interface::types::policy::PolicyExecutionRequest::try_from(p)
                    .map_err(|e| format!("Invalid PolicyExecutionRequest from enclave: {}", e))
            })
            .collect::<Result<_, _>>()?;
        Ok(satisfied)
    }

    pub async fn deliver_batch_events(
        &self,
        events: Vec<nxcc_interface::proto::enclave::EventDelivery>,
    ) -> Result<bool, String> {
        let req = ProtoDeliverBatchEventsRequest { events };
        let mut client = self.runner();
        let resp = client
            .deliver_batch_events(req)
            .await
            .map_err(|e| e.to_string())?;
        let inner = resp.into_inner();
        if inner.success {
            Ok(true)
        } else {
            Err(format!(
                "Enclave runner failed to deliver batch events: {}",
                inner.message
            ))
        }
    }

    pub async fn invoke_http_worker(
        &self,
        worker_id: String,
        http_request: nxcc_interface::proto::vm::HttpRequest,
    ) -> Result<nxcc_interface::proto::vm::HttpResponse, String> {
        let req = ProtoInvokeHttpWorkerRequest {
            worker_id,
            request: Some(http_request),
        };
        let mut client = self.runner();
        let resp = client
            .invoke_http_worker(req)
            .await
            .map_err(|e| e.to_string())?;
        resp.into_inner()
            .response
            .ok_or_else(|| "Enclave returned no HttpResponse".to_string())
    }

    pub async fn check_worker_status(
        &self,
        worker_id: String,
    ) -> Result<(nxcc_interface::proto::vm::WorkerStatus, String), String> {
        let req = CheckWorkerStatusRequest { worker_id };
        let mut client = self.runner();
        let resp = client
            .check_worker_status(req)
            .await
            .map_err(|e| e.to_string())?
            .into_inner();
        let status =
            nxcc_interface::proto::vm::WorkerStatus::try_from(resp.status).map_err(|_| {
                format!(
                    "Invalid worker status enum value from enclave: {}",
                    resp.status
                )
            })?;
        Ok((status, resp.status_message))
    }

    pub async fn stream_worker_logs(
        &self,
        worker_id: String,
        tail_lines: u32,
        follow: bool,
    ) -> Result<
        tokio_stream::wrappers::ReceiverStream<
            Result<nxcc_interface::proto::vm::StreamWorkerLogsResponse, tonic::Status>,
        >,
        String,
    > {
        let req = StreamWorkerLogsRequest {
            worker_id: worker_id.clone(),
            tail_lines,
            follow,
        };
        let mut client = self.runner();
        let response_stream = client
            .stream_worker_logs(req)
            .await
            .map_err(|e| e.to_string())?
            .into_inner();

        // Convert tonic streaming response to tokio receiver stream
        let (tx, rx) = tokio::sync::mpsc::channel(1000);

        tokio::spawn(async move {
            use futures::StreamExt;
            let mut stream = response_stream;
            while let Some(result) = stream.next().await {
                if let Err(_) = tx.send(result).await {
                    break; // Receiver dropped
                }
            }
        });

        Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
    }
}
