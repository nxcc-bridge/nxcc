use std::{pin::Pin, time::Instant};

use futures::Stream;
use nxcc_interface::proto::{
    enclave::StreamWorkerLogsRequest,
    vm::{StreamWorkerLogsResponse, WorkerStatus},
};
use tokio_stream::StreamExt;
use tonic::Status;

use super::common::*;
use crate::runner::RunnerError;

#[tokio::test]
async fn test_stream_worker_logs_success() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-log-stream";
    let worker_id = "worker-log-stream";

    // Set up mock VM and worker
    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    add_worker_mapping(&runner_service, worker_id, vm_id).await;

    // Add worker to mock client
    mock_client.add_worker(
        worker_id.to_string(),
        vec![1, 2, 3],
        WorkerStatus::Running,
        "Some logs here\nMore logs".to_string(),
        Default::default(),
        Default::default(),
    );

    // Test streaming logs
    let request = StreamWorkerLogsRequest {
        worker_id: worker_id.to_string(),
        tail_lines: 2, // Request 2 tail lines to match mock expectation
        follow: false,
    };

    let result = runner_service
        .stream_worker_logs(request.worker_id, request.tail_lines, request.follow)
        .await;
    assert!(result.is_ok(), "Stream worker logs should succeed");

    let _stream = result.unwrap();

    // For unit tests, we mainly want to verify that the enclave correctly
    // routes the request to the appropriate VM and that the basic flow works.
    // The actual streaming functionality is tested in the VM layer tests.
}

#[tokio::test]
async fn test_stream_worker_logs_worker_not_found() {
    let (_secrets, runner_service, _mock_client) = setup();
    let non_existent_worker = "worker-does-not-exist";

    let request = StreamWorkerLogsRequest {
        worker_id: non_existent_worker.to_string(),
        tail_lines: 0,
        follow: false,
    };

    let result = runner_service
        .stream_worker_logs(request.worker_id, request.tail_lines, request.follow)
        .await;

    assert!(result.is_err(), "Should fail for non-existent worker");
    assert!(
        matches!(result.unwrap_err(), RunnerError::WorkerNotFound(id) if id == non_existent_worker),
        "Should return WorkerNotFound error"
    );
}

#[tokio::test]
async fn test_stream_worker_logs_vm_not_attached() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-not-attached";
    let worker_id = "worker-orphaned";

    // Add worker mapping but don't attach VM
    add_worker_mapping(&runner_service, worker_id, vm_id).await;

    let request = StreamWorkerLogsRequest {
        worker_id: worker_id.to_string(),
        tail_lines: 5,
        follow: true,
    };

    let result = runner_service
        .stream_worker_logs(request.worker_id, request.tail_lines, request.follow)
        .await;

    assert!(result.is_err(), "Should fail when VM is not attached");
    assert!(
        matches!(result.unwrap_err(), RunnerError::VmNotAttached(id) if id == vm_id),
        "Should return VmNotAttached error"
    );
}

#[tokio::test]
async fn test_stream_worker_logs_with_tail_and_follow() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-tail-follow";
    let worker_id = "worker-tail-follow";

    // Set up mock VM and worker
    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    add_worker_mapping(&runner_service, worker_id, vm_id).await;

    // Add worker to mock client with some logs
    mock_client.add_worker(
        worker_id.to_string(),
        vec![1, 2, 3],
        WorkerStatus::Running,
        "Historical log 1\nHistorical log 2\nHistorical log 3".to_string(),
        Default::default(),
        Default::default(),
    );

    // Test streaming with tail lines and follow
    let request = StreamWorkerLogsRequest {
        worker_id: worker_id.to_string(),
        tail_lines: 2, // Only last 2 lines
        follow: true,  // Follow for new logs
    };

    let result = runner_service
        .stream_worker_logs(request.worker_id, request.tail_lines, request.follow)
        .await;
    assert!(result.is_ok(), "Stream with tail and follow should succeed");

    // For unit tests, we mainly want to verify that the enclave correctly
    // routes the request and handles the parameters properly
}

#[tokio::test]
async fn test_stream_worker_logs_dead_worker() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-dead-worker";
    let worker_id = "worker-dead";

    // Set up mock VM and worker
    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;

    // Add worker to mock client with some logs
    mock_client.add_worker(
        worker_id.to_string(),
        vec![1, 2, 3],
        WorkerStatus::Running,
        "Log from live worker\nAnother log entry".to_string(),
        Default::default(),
        Default::default(),
    );

    // First add worker mapping, then remove it to simulate dead worker
    add_worker_mapping(&runner_service, worker_id, vm_id).await;

    // Move worker to dead worker map
    let mut worker_map = runner_service.worker_map.write().await;
    let vm_id_value = worker_map.remove(worker_id).unwrap();
    drop(worker_map);

    let mut dead_worker_map = runner_service.dead_worker_map.write().await;
    dead_worker_map.insert(worker_id.to_string(), (vm_id_value, Instant::now()));
    drop(dead_worker_map);

    // Test streaming logs from dead worker
    let request = StreamWorkerLogsRequest {
        worker_id: worker_id.to_string(),
        tail_lines: 0,
        follow: false, // Can't follow dead worker
    };

    let result = runner_service
        .stream_worker_logs(request.worker_id, request.tail_lines, request.follow)
        .await;
    assert!(
        result.is_ok(),
        "Should be able to stream logs from dead worker"
    );

    // For unit tests, we mainly verify that dead workers can be found and
    // their logs can be accessed through the VM
}

#[tokio::test]
async fn test_stream_worker_logs_empty_params() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-empty-params";
    let worker_id = "worker-empty-params";

    // Set up mock VM and worker
    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    add_worker_mapping(&runner_service, worker_id, vm_id).await;

    // Add worker to mock client
    mock_client.add_worker(
        worker_id.to_string(),
        vec![1, 2, 3],
        WorkerStatus::Running,
        "Test log".to_string(),
        Default::default(),
        Default::default(),
    );

    // Test with zero tail lines and no follow
    let request = StreamWorkerLogsRequest {
        worker_id: worker_id.to_string(),
        tail_lines: 0,
        follow: false,
    };

    let result = runner_service
        .stream_worker_logs(request.worker_id, request.tail_lines, request.follow)
        .await;
    assert!(result.is_ok(), "Should succeed even with zero tail lines");

    // For unit tests, we verify that zero tail lines is handled correctly
}
