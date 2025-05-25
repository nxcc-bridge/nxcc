use nxcc_interface::proto::vm::{
    Header as ProtoHeader, HttpRequest as ProtoHttpRequest, HttpResponse as ProtoHttpResponse,
};
use nxcc_vm_base::client::{ClientError, mock::MockExecutionBehavior};
use tonic::Status;

use super::common::*;
use crate::runner::RunnerError;

#[tokio::test]
async fn test_invoke_http_worker_success() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-http-invoke";
    let worker_id = "worker-http-1";

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    add_worker_mapping(&runner_service, worker_id, vm_id).await;
    // Add worker to mock so invoke_http doesn't fail with NotFound initially
    mock_client.add_worker(
        worker_id.to_string(),
        vec![], // dummy code
        nxcc_interface::proto::vm::WorkerStatus::Running,
        "".to_string(), // dummy config path
        Default::default(),
        Default::default(),
    );

    let request_payload = ProtoHttpRequest {
        method: "GET".to_string(),
        uri: "/test".to_string(),
        headers: vec![ProtoHeader {
            key: "X-Test-Header".to_string(),
            value: b"TestValue".to_vec(),
        }],
        body: b"test_request_body".to_vec(),
    };

    let expected_response_payload = ProtoHttpResponse {
        status_code: 200,
        headers: vec![ProtoHeader {
            key: "X-Response-Header".to_string(),
            value: b"ResponseValue".to_vec(),
        }],
        body: b"test_response_body".to_vec(),
    };

    mock_client.set_worker_execution_behavior(
        worker_id,
        MockExecutionBehavior::HttpResponse(expected_response_payload.clone()),
    );

    let result = runner_service
        .invoke_http_worker(worker_id.to_string(), request_payload.clone())
        .await;

    assert!(
        result.is_ok(),
        "invoke_http_worker failed: {:?}",
        result.err()
    );
    let response = result.unwrap();
    assert_eq!(response, expected_response_payload);

    // Verify mock client was called correctly
    let http_invocations = mock_client.get_http_invocations(worker_id);
    assert_eq!(http_invocations.len(), 1);
    assert_eq!(http_invocations[0], request_payload);
}

#[tokio::test]
async fn test_invoke_http_worker_worker_not_found() {
    let (_secrets, runner_service, _mock_client) = setup();
    let worker_id = "worker-does-not-exist";

    let request_payload = ProtoHttpRequest {
        method: "GET".to_string(),
        uri: "/test".to_string(),
        ..Default::default()
    };

    let result = runner_service
        .invoke_http_worker(worker_id.to_string(), request_payload)
        .await;

    assert!(
        matches!(result, Err(RunnerError::WorkerNotFound(id)) if id == worker_id),
        "Expected WorkerNotFound error"
    );
}

#[tokio::test]
async fn test_invoke_http_worker_vm_not_attached() {
    let (_secrets, runner_service, _mock_client) = setup();
    let vm_id = "vm-not-attached";
    let worker_id = "worker-on-detached-vm";

    // Add mapping but don't attach VM
    add_worker_mapping(&runner_service, worker_id, vm_id).await;

    let request_payload = ProtoHttpRequest {
        method: "POST".to_string(),
        uri: "/submit".to_string(),
        ..Default::default()
    };

    let result = runner_service
        .invoke_http_worker(worker_id.to_string(), request_payload)
        .await;

    assert!(
        matches!(result, Err(RunnerError::VmNotAttached(id)) if id == vm_id),
        "Expected VmNotAttached error"
    );
}

#[tokio::test]
async fn test_invoke_http_worker_vm_client_error() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-client-error";
    let worker_id = "worker-vm-error";

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    add_worker_mapping(&runner_service, worker_id, vm_id).await;
    mock_client.add_worker(
        worker_id.to_string(),
        vec![],
        nxcc_interface::proto::vm::WorkerStatus::Running,
        "".to_string(),
        Default::default(),
        Default::default(),
    );

    let request_payload = ProtoHttpRequest {
        method: "DELETE".to_string(),
        uri: "/resource/123".to_string(),
        ..Default::default()
    };

    let expected_vm_error_msg = "VM is down";
    mock_client.set_worker_execution_behavior(
        worker_id,
        MockExecutionBehavior::Error(expected_vm_error_msg.to_string()),
    );

    let result = runner_service
        .invoke_http_worker(worker_id.to_string(), request_payload)
        .await;

    assert!(
        matches!(result, Err(RunnerError::VmConnection(ClientError::Grpc(s))) if s.message() == expected_vm_error_msg),
        "Expected VmConnection error with specific gRPC status"
    );
}
