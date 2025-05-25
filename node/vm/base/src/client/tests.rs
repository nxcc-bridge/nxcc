use std::{collections::HashMap, error::Error, sync::Arc};

use nxcc_interface::proto::vm::{
    Header as ProtoHeader, HttpRequest as ProtoHttpRequest, HttpResponse as ProtoHttpResponse,
    InvokeHttpRequest, InvokeHttpResponse, TrustedConfig, UntrustedConfig, WorkerStatus,
};
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};

use super::*;
use crate::tls::MtlsCertificates; // Use the new struct

// Mock implementation for the VM client (remains the same)
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

    async fn invoke_http(
        &self,
        request: Request<InvokeHttpRequest>,
    ) -> Result<Response<InvokeHttpResponse>, Status> {
        let req_inner = request.into_inner();
        let workers = self.workers.lock().await;

        if workers.contains_key(&req_inner.worker_id) {
            // Simple echo-like behavior for HTTP mock
            let proto_req = req_inner.request.unwrap_or_default();
            Ok(Response::new(InvokeHttpResponse {
                response: Some(ProtoHttpResponse {
                    status_code: 200,
                    headers: proto_req.headers, // Echo headers
                    body: proto_req.body,       // Echo body
                }),
            }))
        } else {
            Ok(Response::new(InvokeHttpResponse {
                response: Some(ProtoHttpResponse {
                    status_code: 404,
                    headers: vec![],
                    body: b"Worker not found".to_vec(),
                }),
            }))
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
                    measurement: vec![0u8; 32],
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
    ) -> Result<Response<nxcc_interface::proto::vm::ListRunningWorkersResponse>, Status> {
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
    // Generate certificates using the simplified API
    let certs = MtlsCertificates::new()?;
    let server_tls_config = certs.server_tls_config()?;
    let client_tls_config = certs.client_tls_config()?;

    // Set up server
    let mock_service = MockVmService::new(WorkerStatus::Running);

    let addr: std::net::SocketAddr = "127.0.0.1:0".parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let server_addr = listener.local_addr()?;

    let server = tonic::transport::Server::builder()
        .tls_config(server_tls_config)? // Use the generated config
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

    // Create client using the simplified connect and generated config
    let mut client = VmServiceClient::connect(server_addr, client_tls_config).await?;

    // Start worker
    let untrusted_config = UntrustedConfig {
        userdata_json: "{\"test\":true}".to_string(),
        advanced_vm_config: HashMap::new(),
    };

    let mut secrets = HashMap::new();
    secrets.insert("CLIENT_SECRET".to_string(), vec![1, 2, 3]);
    let trusted_config = TrustedConfig {
        secrets,
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

    assert!(worker_id.starts_with("instance-")); // Mock service generates ID

    // Get worker status
    let status = client.get_worker_status(worker_id.clone()).await?;
    assert_eq!(status, WorkerStatus::Running);

    // List workers
    let workers = client.list_running_workers().await?;
    assert!(!workers.is_empty()); // Mock service returns fixed list

    // Invoke worker
    let result = client
        .invoke_worker(
            worker_id.clone(),
            "default_handler".to_string(),
            vec![7, 8, 9],
        )
        .await?;
    assert_eq!(result, vec![7, 8, 9]); // Mock service echoes payload

    // Invoke HTTP
    let http_req_payload = ProtoHttpRequest {
        method: "POST".to_string(),
        uri: "/test".to_string(),
        headers: vec![ProtoHeader {
            key: "X-Test".to_string(),
            value: b"true".to_vec(),
        }],
        body: b"http body".to_vec(),
    };
    let http_resp = client
        .invoke_http(worker_id.clone(), http_req_payload.clone())
        .await?;
    assert_eq!(http_resp.status_code, 200);
    assert_eq!(http_resp.body, http_req_payload.body);
    assert_eq!(http_resp.headers.len(), 1);
    assert_eq!(http_resp.headers[0].key, "X-Test");

    // Get worker logs
    let logs = client.get_worker_logs(worker_id.clone()).await?;
    assert!(logs.contains(&worker_id)); // Mock service includes ID in logs

    // Get attestation
    let user_data = vec![10, 11, 12];
    let attestation = client.get_attestation(user_data.clone()).await?;
    assert_eq!(attestation.user_data, user_data);
    assert_eq!(attestation.ephemeral_public_key, vec![1, 2, 3, 4]); // Mock data

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
    let certs = MtlsCertificates::new().unwrap();
    let server_tls_config = certs.server_tls_config().unwrap();
    let client_tls_config = certs.client_tls_config().unwrap();

    // Set up server
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
    let mut client = VmServiceClient::connect(server_addr, client_tls_config)
        .await
        .unwrap();

    // Try to stop non-existent worker
    let result = client.stop_worker("non-existent".to_string()).await;
    assert!(result.is_err());
    match result.err().unwrap() {
        ClientError::Grpc(status) => assert!(status.message().contains("Worker not found")),
        e => panic!("Expected Grpc error, got {:?}", e),
    }

    // Try to get status of non-existent worker
    let result = client.get_worker_status("non-existent".to_string()).await;
    assert!(result.is_err());
    match result.err().unwrap() {
        ClientError::Grpc(status) => assert!(status.message().contains("Worker not found")),
        e => panic!("Expected Grpc error, got {:?}", e),
    }

    // Try to invoke non-existent worker
    let result = client
        .invoke_worker(
            "non-existent".to_string(),
            "default_handler".to_string(),
            vec![],
        )
        .await;
    assert!(result.is_err());
    match result.err().unwrap() {
        ClientError::Grpc(status) => assert!(status.message().contains("Worker not found")),
        e => panic!("Expected Grpc error, got {:?}", e),
    }

    // Try to invoke HTTP on non-existent worker
    let http_req_payload = ProtoHttpRequest {
        method: "GET".to_string(),
        uri: "/".to_string(),
        headers: vec![],
        body: vec![],
    };
    let result = client
        .invoke_http("non-existent".to_string(), http_req_payload)
        .await;
    assert!(result.is_err()); // MockVmServiceClient's invoke_http returns error for not found
    match result.err().unwrap() {
        ClientError::Grpc(status) => assert!(status.message().contains("Worker not found")),
        e => panic!("Expected Grpc error, got {:?}", e),
    }
    // Try to get logs of non-existent worker
    let result = client.get_worker_logs("non-existent".to_string()).await;
    assert!(result.is_err());
    match result.err().unwrap() {
        ClientError::Grpc(status) => assert!(status.message().contains("Worker not found")),
        e => panic!("Expected Grpc error, got {:?}", e),
    }

    // Clean up
    server_handle.abort();
}

// Test the mock implementation when feature flag is active
#[cfg(feature = "test")]
#[tokio::test]
async fn test_mock_client() {
    use nxcc_interface::proto::vm::{
        Header as ProtoHeader, HttpRequest as ProtoHttpRequest, HttpResponse as ProtoHttpResponse,
    };

    use super::mock::{MockAttestationBehavior, MockVmServiceClient};
    // Create a new mock client
    let mut client = MockVmServiceClient::new();

    // Start a worker
    let untrusted_config = UntrustedConfig {
        userdata_json: "{\"test\":true}".to_string(),
        advanced_vm_config: HashMap::new(),
    };

    let mut secrets = HashMap::new();
    secrets.insert("CLIENT_SECRET".to_string(), vec![1, 2, 3]);
    let trusted_config = TrustedConfig {
        secrets,
        limits: None,
    };

    let worker_id = client
        .start_worker(
            "test-worker".to_string(),
            vec![4, 5, 6],
            untrusted_config,
            trusted_config,
        )
        .await
        .unwrap();

    assert!(worker_id.starts_with("instance-test-worker-"));

    // Get worker status
    let status = client.get_worker_status(worker_id.clone()).await.unwrap();
    assert_eq!(status, WorkerStatus::Running);

    // List workers
    let workers = client.list_running_workers().await.unwrap();
    assert_eq!(workers.len(), 1);
    assert_eq!(workers[0], worker_id);

    // Invoke worker
    let payload = vec![7, 8, 9];
    let result = client
        .invoke_worker(
            worker_id.clone(),
            "default_handler".to_string(),
            payload.clone(),
        )
        .await
        .unwrap();
    assert_eq!(result, payload); // Mock echoes back the payload

    // Invoke HTTP on mock
    let http_req_payload = ProtoHttpRequest {
        method: "PUT".to_string(),
        uri: "/mock/resource".to_string(),
        headers: vec![ProtoHeader {
            key: "X-Mock".to_string(),
            value: b"mock-val".to_vec(),
        }],
        body: b"mock http body".to_vec(),
    };
    // Set specific HTTP response behavior
    client.set_worker_execution_behavior(
        &worker_id,
        crate::client::mock::MockExecutionBehavior::HttpResponse(ProtoHttpResponse {
            status_code: 201,
            headers: vec![ProtoHeader {
                key: "Location".to_string(),
                value: b"/mock/resource/1".to_vec(),
            }],
            body: b"Created".to_vec(),
        }),
    );

    let http_resp = client
        .invoke_http(worker_id.clone(), http_req_payload.clone())
        .await
        .unwrap();
    assert_eq!(http_resp.status_code, 201);
    assert_eq!(http_resp.body, b"Created".to_vec());
    assert_eq!(http_resp.headers[0].key, "Location");

    // Get worker logs
    let logs = client.get_worker_logs(worker_id.clone()).await.unwrap();
    assert!(logs.contains("started"));
    assert!(logs.contains("Invoked with payload"));

    // Test attestation with standard behavior
    let user_data = vec![10, 11, 12];
    let attestation = client.get_attestation(user_data.clone()).await.unwrap();
    assert_eq!(attestation.user_data, user_data);
    assert_eq!(attestation.ephemeral_public_key, vec![1, 2, 3, 4, 5]);

    // Test attestation with custom behavior
    client.set_attestation_behavior(MockAttestationBehavior::Custom(AttestationReport {
        user_data: vec![],
        measurement: vec![0u8; 32],
        ephemeral_public_key: vec![9, 8, 7],
        block_hashes: vec![vec![1, 1, 1]],
    }));

    let user_data2 = vec![20, 21, 22];
    let attestation = client.get_attestation(user_data2.clone()).await.unwrap();
    assert_eq!(attestation.user_data, user_data2); // User data gets replaced
    assert_eq!(attestation.ephemeral_public_key, vec![9, 8, 7]); // From custom attestation
    assert_eq!(attestation.block_hashes, vec![vec![1, 1, 1]]); // From custom attestation

    // Test attestation error behavior
    client.set_attestation_behavior(MockAttestationBehavior::Error(
        "Attestation failed".to_string(),
    ));
    let result = client.get_attestation(vec![]).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ClientError::Grpc(status) => assert_eq!(status.message(), "Attestation failed"),
        e => panic!("Expected Grpc error, got {:?}", e),
    }

    // Test fail_next_operation functionality
    client.fail_next_operation("Forced failure");
    let result = client.get_worker_status(worker_id.clone()).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ClientError::Grpc(status) => assert_eq!(status.message(), "Forced failure"),
        e => panic!("Expected Grpc error, got {:?}", e),
    }

    // Error state should be reset after one failure
    let status = client.get_worker_status(worker_id.clone()).await.unwrap();
    assert_eq!(status, WorkerStatus::Running);

    // Add a pre-existing worker for testing
    client.add_worker(
        "pre-existing-1".to_string(),
        vec![1, 2, 3],
        WorkerStatus::Error,
        "Error logs".to_string(),
        TrustedConfig::default(),
        UntrustedConfig::default(),
    );

    // Check that pre-existing worker is accessible
    let status = client
        .get_worker_status("pre-existing-1".to_string())
        .await
        .unwrap();
    assert_eq!(status, WorkerStatus::Error);

    // Update worker status
    client
        .set_worker_status("pre-existing-1", WorkerStatus::Running)
        .unwrap();
    let status = client
        .get_worker_status("pre-existing-1".to_string())
        .await
        .unwrap();
    assert_eq!(status, WorkerStatus::Running);

    // Append logs to worker
    client
        .append_logs("pre-existing-1", "\nMore logs\n")
        .unwrap();
    let logs = client
        .get_worker_logs("pre-existing-1".to_string())
        .await
        .unwrap();
    assert_eq!(logs, "Error logs\nMore logs\n");

    // Try to get non-existent worker
    let result = client.get_worker_status("non-existent".to_string()).await;
    assert!(result.is_err());

    // Stop worker
    let result = client.stop_worker(worker_id.clone()).await;
    assert!(result.is_ok());

    // Worker should be gone
    let result = client.get_worker_status(worker_id).await;
    assert!(result.is_err());

    // Clear all workers
    client.clear_workers();
    let workers = client.list_running_workers().await.unwrap();
    assert_eq!(workers.len(), 0);
}
