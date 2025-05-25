use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use nxcc_interface::proto::vm::{
    GetAttestationRequest, GetWorkerLogsRequest, GetWorkerStatusRequest, Header as ProtoHeader,
    HttpRequest as ProtoHttpRequest, HttpResponse as ProtoHttpResponse, InvokeHttpRequest,
    InvokeHttpResponse, InvokeWorkerRequest, ListRunningWorkersRequest, StartWorkerRequest,
    StopWorkerRequest, TrustedConfig, UntrustedConfig, WorkerStatus,
};

use super::*;

// Mock implementation of VmRuntime for testing
#[derive(Default)]
struct MockVmRuntime {
    start_worker_count: AtomicUsize,
    stop_worker_count: AtomicUsize,
    invoke_worker_count: AtomicUsize,
    get_status_count: AtomicUsize,
    list_workers_count: AtomicUsize,
    get_logs_count: AtomicUsize,
    invoke_http_count: AtomicUsize,
    force_attestation_error: AtomicBool,
    workers: Mutex<HashMap<String, WorkerStatus>>, // Simulate worker state
}

#[tonic::async_trait]
impl VmRuntime for MockVmRuntime {
    async fn start_worker(
        &self,
        _worker_code: Vec<u8>,
        _untrusted_config: UntrustedConfig,
        _trusted_config: TrustedConfig,
    ) -> Result<String, VmError> {
        self.start_worker_count.fetch_add(1, Ordering::SeqCst);
        let id = "instance-test-worker".to_string();
        let mut workers = self.workers.lock().unwrap();
        workers.insert(id.clone(), WorkerStatus::Running);
        Ok(id)
    }

    async fn stop_worker(&self, id: String) -> Result<(), VmError> {
        self.stop_worker_count.fetch_add(1, Ordering::SeqCst);
        let mut workers = self.workers.lock().unwrap();
        if workers.remove(&id).is_some() {
            Ok(())
        } else {
            Err(VmError::new(format!("Worker instance not found: {}", id)))
        }
    }

    async fn invoke_worker(
        &self,
        id: String,
        _handler_name: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, VmError> {
        self.invoke_worker_count.fetch_add(1, Ordering::SeqCst);
        let workers = self.workers.lock().unwrap();
        if !workers.contains_key(&id) {
            return Err(VmError::new(format!("Worker instance not found: {}", id)));
        }
        // Simulate some work
        Ok(payload.iter().map(|b| b.wrapping_add(1)).collect())
    }

    async fn invoke_http(
        &self,
        id: String,
        request: ProtoHttpRequest,
    ) -> Result<ProtoHttpResponse, VmError> {
        self.invoke_http_count.fetch_add(1, Ordering::SeqCst);
        let workers = self.workers.lock().unwrap();
        if !workers.contains_key(&id) {
            return Err(VmError::new(format!("Worker instance not found: {}", id)));
        }
        // Simulate an HTTP response
        Ok(ProtoHttpResponse {
            status_code: 200,
            headers: request.headers, // Echo back headers
            body: request.body,       // Echo back body
        })
    }

    async fn get_attestation(
        &self,
        user_data: Vec<u8>,
    ) -> Result<nxcc_interface::types::AttestationReport, VmError> {
        if self.force_attestation_error.load(Ordering::SeqCst) {
            return Err(VmError::new("Forced attestation error"));
        }
        Ok(nxcc_interface::types::AttestationReport {
            measurement: vec![0u8; 32],
            ephemeral_public_key: vec![],
            block_hashes: vec![],
            user_data,
        })
    }

    async fn get_worker_status(&self, id: String) -> Result<WorkerStatus, VmError> {
        self.get_status_count.fetch_add(1, Ordering::SeqCst);
        let workers = self.workers.lock().unwrap();
        workers
            .get(&id)
            .cloned()
            .ok_or_else(|| VmError::new(format!("Worker instance not found: {}", id)))
    }

    async fn list_running_workers(&self) -> Result<Vec<String>, VmError> {
        self.list_workers_count.fetch_add(1, Ordering::SeqCst);
        let workers = self.workers.lock().unwrap();
        Ok(workers.keys().cloned().collect())
    }

    async fn get_worker_logs(&self, id: String) -> Result<String, VmError> {
        self.get_logs_count.fetch_add(1, Ordering::SeqCst);
        let workers = self.workers.lock().unwrap();
        if !workers.contains_key(&id) {
            return Err(VmError::new(format!("Worker instance not found: {}", id)));
        }
        Ok(format!("Log entry 1 for {}\nLog entry 2 for {}", id, id))
    }
}

#[tokio::test]
async fn test_vm_service_grpc_start_worker() {
    let runtime = Arc::new(MockVmRuntime::default());
    let service = VmServiceGrpc::new(runtime.clone());

    let request = Request::new(StartWorkerRequest {
        worker_id: "test-worker".to_string(),
        worker_code: vec![1, 2, 3],
        untrusted_config: Some(UntrustedConfig {
            userdata_json: "{}".to_string(),
            ..Default::default()
        }),
        trusted_config: Some(TrustedConfig::default()),
    });

    let response = service.start_worker(request).await.unwrap();
    let response = response.into_inner();
    assert!(response.success);
    assert!(response.id.starts_with("instance-test-worker"));
    assert_eq!(runtime.start_worker_count.load(Ordering::SeqCst), 1);
    assert!(
        runtime
            .workers
            .lock()
            .unwrap()
            .contains_key("instance-test-worker")
    );
}

#[tokio::test]
async fn test_vm_service_grpc_stop_invoke_attestation() {
    let runtime = Arc::new(MockVmRuntime::default());
    // Pre-populate a worker
    runtime
        .workers
        .lock()
        .unwrap()
        .insert("instance-123".to_string(), WorkerStatus::Running);
    runtime
        .workers
        .lock()
        .unwrap()
        .insert("instance-456".to_string(), WorkerStatus::Running);

    let service = VmServiceGrpc::new(runtime.clone());

    // Test stop_worker (happy path)
    let request = Request::new(StopWorkerRequest {
        id: "instance-123".to_string(),
    });
    let response = service.stop_worker(request).await.unwrap().into_inner();
    assert!(response.success);
    assert_eq!(runtime.stop_worker_count.load(Ordering::SeqCst), 1);
    assert!(!runtime.workers.lock().unwrap().contains_key("instance-123"));

    // Test invoke_worker (happy path)
    let request = Request::new(InvokeWorkerRequest {
        id: "instance-456".to_string(),
        handler_name: "default_handler".to_string(),
        payload: vec![10, 20],
    });
    let response = service.invoke_worker(request).await.unwrap().into_inner();
    assert!(response.success);
    assert_eq!(response.result, vec![11, 21]); // Mock adds 1
    assert_eq!(runtime.invoke_worker_count.load(Ordering::SeqCst), 1);

    // Test get_attestation (happy path)
    let request = Request::new(GetAttestationRequest {
        user_data: vec![7, 8],
    });
    let response = service.get_attestation(request).await.unwrap().into_inner();
    assert!(response.report.is_some());
    assert_eq!(response.report.unwrap().user_data, vec![7, 8]);
}

#[tokio::test]
async fn test_vm_service_grpc_invoke_http() {
    let runtime = Arc::new(MockVmRuntime::default());
    runtime
        .workers
        .lock()
        .unwrap()
        .insert("http-worker-1".to_string(), WorkerStatus::Running);
    let service = VmServiceGrpc::new(runtime.clone());

    let http_request_proto = ProtoHttpRequest {
        method: "GET".to_string(),
        uri: "/test/path".to_string(),
        headers: vec![ProtoHeader {
            key: "X-Custom".to_string(),
            value: b"value".to_vec(),
        }],
        body: b"request body".to_vec(),
    };
    let request = Request::new(InvokeHttpRequest {
        worker_id: "http-worker-1".to_string(),
        request: Some(http_request_proto.clone()),
    });

    let response = service.invoke_http(request).await.unwrap().into_inner();
    assert!(response.response.is_some());
    let http_response = response.response.unwrap();
    assert_eq!(http_response.status_code, 200);
    assert_eq!(http_response.body, http_request_proto.body);
    assert_eq!(runtime.invoke_http_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_vm_service_grpc_status_list_logs() {
    let runtime = Arc::new(MockVmRuntime::default());
    // Pre-populate workers
    runtime
        .workers
        .lock()
        .unwrap()
        .insert("instance-abc".to_string(), WorkerStatus::Running);
    runtime
        .workers
        .lock()
        .unwrap()
        .insert("instance-xyz".to_string(), WorkerStatus::Error);

    let service = VmServiceGrpc::new(runtime.clone());

    // Test get_worker_status (happy path - running)
    let request = Request::new(GetWorkerStatusRequest {
        id: "instance-abc".to_string(),
    });
    let response = service
        .get_worker_status(request)
        .await
        .unwrap()
        .into_inner();
    assert!(response.success);
    assert_eq!(response.id, "instance-abc");
    assert_eq!(
        WorkerStatus::try_from(response.status).unwrap(),
        WorkerStatus::Running
    );
    assert_eq!(runtime.get_status_count.load(Ordering::SeqCst), 1);

    // Test get_worker_status (happy path - error state)
    let request = Request::new(GetWorkerStatusRequest {
        id: "instance-xyz".to_string(),
    });
    let response = service
        .get_worker_status(request)
        .await
        .unwrap()
        .into_inner();
    assert!(response.success);
    assert_eq!(response.id, "instance-xyz");
    assert_eq!(
        WorkerStatus::try_from(response.status).unwrap(),
        WorkerStatus::Error
    );
    assert_eq!(runtime.get_status_count.load(Ordering::SeqCst), 2);

    // Test list_running_workers
    let request = Request::new(ListRunningWorkersRequest {});
    let response = service
        .list_running_workers(request)
        .await
        .unwrap()
        .into_inner();
    assert_eq!(response.ids.len(), 2);
    assert!(response.ids.contains(&"instance-abc".to_string()));
    assert!(response.ids.contains(&"instance-xyz".to_string()));
    assert_eq!(runtime.list_workers_count.load(Ordering::SeqCst), 1);

    // Test get_worker_logs (happy path)
    let request = Request::new(GetWorkerLogsRequest {
        id: "instance-abc".to_string(),
    });
    let response = service.get_worker_logs(request).await.unwrap().into_inner();
    assert!(response.success);
    assert!(response.logs.contains("Log entry 1 for instance-abc"));
    assert_eq!(runtime.get_logs_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_vm_service_grpc_errors() {
    let runtime = Arc::new(MockVmRuntime::default());
    // No workers started initially
    let service = VmServiceGrpc::new(runtime.clone());

    // Test stop_worker (error path - not found)
    let request = Request::new(StopWorkerRequest {
        id: "invalid-id".to_string(),
    });
    let response = service.stop_worker(request).await.unwrap().into_inner();
    assert!(!response.success);
    assert!(response.error_message.contains("Worker instance not found"));

    // Test invoke_worker (error path - not found)
    let request = Request::new(InvokeWorkerRequest {
        id: "invalid-id".to_string(),
        handler_name: "default_handler".to_string(),
        payload: vec![],
    });
    let response = service.invoke_worker(request).await.unwrap().into_inner();
    assert!(!response.success);
    assert!(response.error_message.contains("Worker instance not found"));

    // Test get_worker_status (error path - not found)
    let request = Request::new(GetWorkerStatusRequest {
        id: "invalid-id".to_string(),
    });
    let response = service
        .get_worker_status(request)
        .await
        .unwrap()
        .into_inner();
    assert!(!response.success);
    assert!(response.error_message.contains("Worker instance not found"));

    // Test get_worker_logs (error path - not found)
    let request = Request::new(GetWorkerLogsRequest {
        id: "invalid-id".to_string(),
    });
    let response = service.get_worker_logs(request).await.unwrap().into_inner();
    assert!(!response.success);
    assert!(response.error_message.contains("Worker instance not found"));

    // Test get_attestation (error path - forced error)
    runtime
        .force_attestation_error
        .store(true, Ordering::SeqCst);
    let request = Request::new(GetAttestationRequest { user_data: vec![] });
    let result = service.get_attestation(request).await;
    assert!(result.is_err());
    let status = result.unwrap_err();
    assert_eq!(status.code(), Status::internal("").code());
    assert!(status.message().contains("Forced attestation error"));
}
