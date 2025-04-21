use std::{error::Error, net::SocketAddr, path::Path};

use hyper_util::rt::TokioIo;
use nxcc_interface::{
    proto::vm::{
        GetAttestationRequest, GetWorkerLogsRequest, GetWorkerStatusRequest, InvokeWorkerRequest,
        ListRunningWorkersRequest, StartWorkerRequest, StopWorkerRequest, TrustedConfig,
        UntrustedConfig, WorkerStatus,
    },
    types::{AttestationReport, FromProto as _},
};
use thiserror::Error;
#[cfg(feature = "uds")]
use tokio::net::UnixStream;
#[cfg(feature = "vsock")]
use tokio_vsock::VsockStream;
use tonic::{
    Status,
    transport::{Channel, Endpoint, Uri},
};
use tracing::debug;

/// Errors that can occur during client operations
#[derive(Error, Debug)]
pub enum ClientError {
    #[error("Transport error: {0}")]
    Transport(#[from] tonic::transport::Error),

    #[error("gRPC error: {0}")]
    Grpc(#[from] Status),

    #[error("TLS configuration error: {0}")]
    TlsConfig(#[from] Box<dyn Error + Send + Sync>),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to connect to VM service: {0}")]
    Connect(String),

    #[error("Invalid URI: {0}")]
    Uri(String),
}

/// Client for communicating with a VM service
pub struct VmServiceClient {
    inner: nxcc_interface::proto::vm::vm_client::VmClient<Channel>,
}

impl VmServiceClient {
    /// Connect to a VM service over TCP with TLS
    pub async fn connect(
        addr: SocketAddr,
        client_cert_pem: String,
        client_key_pem: String,
        ca_cert_pem: String,
        domain_name: String,
    ) -> Result<Self, ClientError> {
        let tls_config = crate::tls::create_client_tls_config(
            client_cert_pem,
            client_key_pem,
            ca_cert_pem,
            &domain_name,
        )
        .map_err(ClientError::TlsConfig)?;

        let channel = Channel::from_shared(format!("https://{}", addr))
            .map_err(|e| ClientError::Uri(e.to_string()))?
            .tls_config(tls_config)?
            .connect()
            .await?;

        Ok(Self {
            inner: nxcc_interface::proto::vm::vm_client::VmClient::new(channel),
        })
    }

    /// Connect to a VM service over a Unix Domain Socket with TLS
    #[cfg(feature = "uds")]
    pub async fn connect_uds<P: AsRef<Path>>(
        path: P,
        client_cert_pem: String,
        client_key_pem: String,
        ca_cert_pem: String,
        domain_name: String,
    ) -> Result<Self, ClientError> {
        let tls_config = crate::tls::create_client_tls_config(
            client_cert_pem,
            client_key_pem,
            ca_cert_pem,
            &domain_name,
        )
        .map_err(ClientError::TlsConfig)?;

        let path_str = path.as_ref().to_string_lossy().to_string();
        debug!("Connecting to UDS at {}", path_str);

        let endpoint = Endpoint::try_from("http://[::]:50051")
            .map_err(|e| ClientError::Uri(e.to_string()))?
            .tls_config(tls_config)?;

        let channel = endpoint
            .connect_with_connector(tower::service_fn(move |_: Uri| {
                let path = path_str.clone();
                async move {
                    let stream = UnixStream::connect(&path).await?;
                    Ok::<_, std::io::Error>(TokioIo::new(stream))
                }
            }))
            .await?;

        Ok(Self {
            inner: nxcc_interface::proto::vm::vm_client::VmClient::new(channel),
        })
    }

    /// Connect to a VM service over VSOCK with TLS
    #[cfg(feature = "vsock")]
    pub async fn connect_vsock(
        cid: u32,
        port: u32,
        client_cert_pem: String,
        client_key_pem: String,
        ca_cert_pem: String,
        domain_name: String,
    ) -> Result<Self, ClientError> {
        let tls_config = crate::tls::create_client_tls_config(
            client_cert_pem,
            client_key_pem,
            ca_cert_pem,
            &domain_name,
        )
        .map_err(ClientError::TlsConfig)?;

        debug!("Connecting to VSOCK at CID {} port {}", cid, port);

        let endpoint = Endpoint::try_from("http://[::]:50051")
            .map_err(|e| ClientError::Uri(e.to_string()))?
            .tls_config(tls_config)?;

        let channel = endpoint
            .connect_with_connector(tower::service_fn(move |_: Uri| {
                let cid = cid;
                let port = port;
                async move {
                    let stream =
                        VsockStream::connect(tokio_vsock::VsockAddr::new(cid, port)).await?;
                    Ok::<_, std::io::Error>(TokioIo::new(stream))
                }
            }))
            .await?;

        Ok(Self {
            inner: nxcc_interface::proto::vm::vm_client::VmClient::new(channel),
        })
    }

    /// Start a new worker with the provided code and configuration
    pub async fn start_worker(
        &mut self,
        worker_id: String,
        worker_code: Vec<u8>,
        untrusted_config: UntrustedConfig,
        trusted_config: TrustedConfig,
    ) -> Result<String, ClientError> {
        let request = StartWorkerRequest {
            worker_id,
            worker_code,
            untrusted_config: Some(untrusted_config),
            trusted_config: Some(trusted_config),
        };

        let response = self.inner.start_worker(request).await?.into_inner();

        if response.success {
            Ok(response.id)
        } else {
            Err(ClientError::Grpc(Status::unknown(
                response.error_message.clone(),
            )))
        }
    }

    /// Stop a running worker instance
    pub async fn stop_worker(&mut self, id: String) -> Result<(), ClientError> {
        let request = StopWorkerRequest { id };

        let response = self.inner.stop_worker(request).await?.into_inner();

        if response.success {
            Ok(())
        } else {
            Err(ClientError::Grpc(Status::unknown(
                response.error_message.clone(),
            )))
        }
    }

    /// Invoke a worker with the provided payload
    pub async fn invoke_worker(
        &mut self,
        id: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, ClientError> {
        let request = InvokeWorkerRequest { id, payload };

        let response = self.inner.invoke_worker(request).await?.into_inner();

        if response.success {
            Ok(response.result)
        } else {
            Err(ClientError::Grpc(Status::unknown(
                response.error_message.clone(),
            )))
        }
    }

    /// Get an attestation report from the VM service
    pub async fn get_attestation(
        &mut self,
        user_data: Vec<u8>,
    ) -> Result<AttestationReport, ClientError> {
        let request = GetAttestationRequest { user_data };

        let response = self.inner.get_attestation(request).await?.into_inner();

        match response.report {
            Some(report) => Ok(AttestationReport::from_proto(report)),
            None => Err(ClientError::Grpc(Status::internal(
                "No attestation report received",
            ))),
        }
    }

    /// Get the status of a worker instance
    pub async fn get_worker_status(&mut self, id: String) -> Result<WorkerStatus, ClientError> {
        let request = GetWorkerStatusRequest { id };

        let response = self.inner.get_worker_status(request).await?.into_inner();

        if response.success {
            WorkerStatus::try_from(response.status)
                .map_err(|_| ClientError::Grpc(Status::internal("Invalid worker status received")))
        } else {
            Err(ClientError::Grpc(Status::unknown(
                response.error_message.clone(),
            )))
        }
    }

    /// Get a list of all running worker IDs
    pub async fn list_running_workers(&mut self) -> Result<Vec<String>, ClientError> {
        let request = ListRunningWorkersRequest {};

        let response = self.inner.list_running_workers(request).await?.into_inner();

        Ok(response.ids)
    }

    /// Get logs from a worker instance
    pub async fn get_worker_logs(&mut self, id: String) -> Result<String, ClientError> {
        let request = GetWorkerLogsRequest { id };

        let response = self.inner.get_worker_logs(request).await?.into_inner();

        if response.success {
            Ok(response.logs)
        } else {
            Err(ClientError::Grpc(Status::unknown(
                response.error_message.clone(),
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use nxcc_interface::proto::vm::{TrustedConfig, UntrustedConfig, WorkerStatus};
    use tokio::sync::Mutex;
    use tonic::{Request, Response, Status};

    use super::*;
    use crate::tls::{generate_ca_cert, generate_signed_cert};

    // Mock implementation for the VM client
    #[derive(Clone)]
    struct MockVmService {
        status: WorkerStatus,
        workers: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    }

    impl MockVmService {
        fn new(status: WorkerStatus) -> Self {
            Self {
                status,
                workers: Arc::new(Mutex::new(HashMap::new())),
            }
        }
    }

    #[tonic::async_trait]
    impl nxcc_interface::proto::vm::vm_server::Vm for MockVmService {
        async fn start_worker(
            &self,
            request: Request<StartWorkerRequest>,
        ) -> Result<Response<nxcc_interface::proto::vm::StartWorkerResponse>, Status> {
            let req = request.into_inner();
            let worker_id = req.worker_id.clone();
            let instance_id = format!("instance-{}", worker_id);

            let mut workers = self.workers.lock().await;
            workers.insert(instance_id.clone(), req.worker_code);

            Ok(Response::new(
                nxcc_interface::proto::vm::StartWorkerResponse {
                    id: instance_id,
                    success: true,
                    error_message: String::new(),
                },
            ))
        }

        async fn stop_worker(
            &self,
            request: Request<StopWorkerRequest>,
        ) -> Result<Response<nxcc_interface::proto::vm::StopWorkerResponse>, Status> {
            let req = request.into_inner();
            let mut workers = self.workers.lock().await;

            if workers.remove(&req.id).is_some() {
                Ok(Response::new(
                    nxcc_interface::proto::vm::StopWorkerResponse {
                        success: true,
                        error_message: String::new(),
                    },
                ))
            } else {
                Ok(Response::new(
                    nxcc_interface::proto::vm::StopWorkerResponse {
                        success: false,
                        error_message: "Worker not found".to_string(),
                    },
                ))
            }
        }

        async fn invoke_worker(
            &self,
            request: Request<InvokeWorkerRequest>,
        ) -> Result<Response<nxcc_interface::proto::vm::InvokeWorkerResponse>, Status> {
            let req = request.into_inner();
            let workers = self.workers.lock().await;

            if workers.contains_key(&req.id) {
                Ok(Response::new(
                    nxcc_interface::proto::vm::InvokeWorkerResponse {
                        result: req.payload,
                        success: true,
                        error_message: String::new(),
                    },
                ))
            } else {
                Ok(Response::new(
                    nxcc_interface::proto::vm::InvokeWorkerResponse {
                        result: vec![],
                        success: false,
                        error_message: "Worker not found".to_string(),
                    },
                ))
            }
        }

        async fn get_attestation(
            &self,
            request: Request<GetAttestationRequest>,
        ) -> Result<Response<nxcc_interface::proto::vm::GetAttestationResponse>, Status> {
            let req = request.into_inner();

            Ok(Response::new(
                nxcc_interface::proto::vm::GetAttestationResponse {
                    report: Some(nxcc_interface::proto::interface::AttestationReport {
                        ephemeral_public_key: vec![1, 2, 3, 4],
                        block_hashes: vec![vec![5, 6, 7, 8]],
                        user_data: req.user_data,
                    }),
                },
            ))
        }

        async fn get_worker_status(
            &self,
            request: Request<GetWorkerStatusRequest>,
        ) -> Result<Response<nxcc_interface::proto::vm::GetWorkerStatusResponse>, Status> {
            let req = request.into_inner();
            let workers = self.workers.lock().await;

            if workers.contains_key(&req.id) {
                Ok(Response::new(
                    nxcc_interface::proto::vm::GetWorkerStatusResponse {
                        id: req.id,
                        status: self.status as i32,
                        success: true,
                        error_message: String::new(),
                    },
                ))
            } else {
                Ok(Response::new(
                    nxcc_interface::proto::vm::GetWorkerStatusResponse {
                        id: req.id,
                        status: WorkerStatus::Unspecified as i32,
                        success: false,
                        error_message: "Worker not found".to_string(),
                    },
                ))
            }
        }

        async fn list_running_workers(
            &self,
            _request: Request<ListRunningWorkersRequest>,
        ) -> Result<Response<nxcc_interface::proto::vm::ListRunningWorkersResponse>, Status>
        {
            let workers = self.workers.lock().await;

            Ok(Response::new(
                nxcc_interface::proto::vm::ListRunningWorkersResponse {
                    ids: workers.keys().cloned().collect(),
                },
            ))
        }

        async fn get_worker_logs(
            &self,
            request: Request<GetWorkerLogsRequest>,
        ) -> Result<Response<nxcc_interface::proto::vm::GetWorkerLogsResponse>, Status> {
            let req = request.into_inner();
            let workers = self.workers.lock().await;

            if workers.contains_key(&req.id) {
                Ok(Response::new(
                    nxcc_interface::proto::vm::GetWorkerLogsResponse {
                        logs: format!("Mock logs for {}", req.id),
                        success: true,
                        error_message: String::new(),
                    },
                ))
            } else {
                Ok(Response::new(
                    nxcc_interface::proto::vm::GetWorkerLogsResponse {
                        logs: String::new(),
                        success: false,
                        error_message: "Worker not found".to_string(),
                    },
                ))
            }
        }
    }

    #[tokio::test]
    async fn test_client_operations() -> Result<(), Box<dyn Error>> {
        // Generate certificates
        let (ca_cert, ca_key) = generate_ca_cert().unwrap();
        let ca_cert_pem = ca_cert.pem();
        let (server_cert, server_key) =
            generate_signed_cert("localhost", &ca_cert, &ca_key).unwrap();
        let (client_cert, client_key) = generate_signed_cert("client", &ca_cert, &ca_key).unwrap();

        // Set up server
        let server_tls_config = crate::tls::create_server_tls_config(
            server_cert.clone(),
            server_key.clone(),
            ca_cert_pem.clone(),
        )
        .unwrap();

        let mock_service = MockVmService::new(WorkerStatus::Running);

        let addr: std::net::SocketAddr = "127.0.0.1:0".parse()?;
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let server_addr = listener.local_addr()?;

        let server = tonic::transport::Server::builder()
            .tls_config(server_tls_config)?
            .add_service(nxcc_interface::proto::vm::vm_server::VmServer::new(
                mock_service.clone(),
            ))
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                async {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                },
            );

        let server_handle = tokio::spawn(server);

        // Wait for server to start
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Create client
        let mut client = VmServiceClient::connect(
            server_addr,
            client_cert,
            client_key,
            ca_cert_pem.clone(),
            "localhost".to_string(),
        )
        .await?;

        // Start worker
        let untrusted_config = UntrustedConfig {
            userdata_json: "{\"test\":true}".to_string(),
            advanced_vm_config: HashMap::new(),
        };

        let trusted_config = TrustedConfig {
            crypto_keys: vec![vec![1, 2, 3]],
            limits: None,
        };

        let worker_id = client
            .start_worker(
                "test-worker".to_string(),
                vec![4, 5, 6],
                untrusted_config,
                trusted_config,
            )
            .await?;

        assert_eq!(worker_id, "instance-test-worker");

        // Get worker status
        let status = client.get_worker_status(worker_id.clone()).await?;
        assert_eq!(status, WorkerStatus::Running);

        // List workers
        let workers = client.list_running_workers().await?;
        assert_eq!(workers.len(), 1);
        assert_eq!(workers[0], worker_id);

        // Invoke worker
        let result = client
            .invoke_worker(worker_id.clone(), vec![7, 8, 9])
            .await?;
        assert_eq!(result, vec![7, 8, 9]);

        // Get worker logs
        let logs = client.get_worker_logs(worker_id.clone()).await?;
        assert_eq!(logs, format!("Mock logs for {}", worker_id));

        // Get attestation
        let user_data = vec![10, 11, 12];
        let attestation = client.get_attestation(user_data.clone()).await?;
        assert_eq!(attestation.user_data, user_data);
        assert_eq!(attestation.ephemeral_public_key, vec![1, 2, 3, 4]);

        // Stop worker
        client.stop_worker(worker_id).await?;

        // List workers again - should be empty
        let workers = client.list_running_workers().await?;
        assert_eq!(workers.len(), 0);

        // Clean up
        server_handle.abort();

        Ok(())
    }

    #[tokio::test]
    async fn test_client_error_handling() {
        // Generate certificates
        let (ca_cert, ca_key) = generate_ca_cert().unwrap();
        let ca_cert_pem = ca_cert.pem();
        let (server_cert, server_key) =
            generate_signed_cert("localhost", &ca_cert, &ca_key).unwrap();
        let (client_cert, client_key) = generate_signed_cert("client", &ca_cert, &ca_key).unwrap();

        // Set up server
        let server_tls_config = crate::tls::create_server_tls_config(
            server_cert.clone(),
            server_key.clone(),
            ca_cert_pem.clone(),
        )
        .unwrap();

        let mock_service = MockVmService::new(WorkerStatus::Running);

        let addr: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        let server_addr = listener.local_addr().unwrap();

        let server = tonic::transport::Server::builder()
            .tls_config(server_tls_config)
            .unwrap()
            .add_service(nxcc_interface::proto::vm::vm_server::VmServer::new(
                mock_service.clone(),
            ))
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                async {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                },
            );

        let server_handle = tokio::spawn(server);

        // Wait for server to start
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Create client
        let mut client = VmServiceClient::connect(
            server_addr,
            client_cert,
            client_key,
            ca_cert_pem.clone(),
            "localhost".to_string(),
        )
        .await
        .unwrap();

        // Try to stop non-existent worker
        let result = client.stop_worker("non-existent".to_string()).await;
        assert!(result.is_err());

        // Try to get status of non-existent worker
        let result = client.get_worker_status("non-existent".to_string()).await;
        assert!(result.is_err());

        // Try to invoke non-existent worker
        let result = client
            .invoke_worker("non-existent".to_string(), vec![])
            .await;
        assert!(result.is_err());

        // Try to get logs of non-existent worker
        let result = client.get_worker_logs("non-existent".to_string()).await;
        assert!(result.is_err());

        // Clean up
        server_handle.abort();
    }
}
