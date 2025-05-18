use nxcc_interface::proto::vm::WorkerStatus;
use nxcc_vm_base::client::mock::MockExecutionBehavior;

use super::common::*;
use crate::runner::{ClientError, RunnerError};

#[tokio::test]
async fn test_invoke_worker_success() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-invoke";
    let worker_id = "worker-invoke";
    let payload = vec![10, 20, 30];
    let expected_response = vec![40, 50, 60];

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    add_worker_mapping(&runner_service, worker_id, vm_id).await;
    mock_client.add_worker(
        // Add worker so invoke doesn't fail with NotFound
        worker_id.to_string(),
        vec![],
        WorkerStatus::Running,
        "".into(),
        Default::default(),
        Default::default(),
    );
    // Configure mock response
    mock_client.set_worker_execution_behavior(
        worker_id,
        MockExecutionBehavior::Fixed(expected_response.clone()),
    );

    let result = runner_service
        .invoke_worker(worker_id.to_string(), payload.clone())
        .await;

    let response = result.unwrap();
    assert_eq!(response, expected_response);
}

#[tokio::test]
async fn test_invoke_worker_not_found_locally() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-invoke-nf-local";
    let worker_id = "worker-nf-local";
    let payload = vec![10, 20, 30];

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    // Do NOT add worker mapping

    let result = runner_service
        .invoke_worker(worker_id.to_string(), payload.clone())
        .await;

    assert!(matches!(result, Err(RunnerError::WorkerNotFound(id)) if id == worker_id));
}

#[tokio::test]
async fn test_invoke_worker_fails_in_vm() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-invoke-fail";
    let worker_id = "worker-invoke-fail";
    let payload = vec![10, 20, 30];
    let error_msg = "Worker execution panicked";

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    add_worker_mapping(&runner_service, worker_id, vm_id).await;
    mock_client.add_worker(
        // Add worker so invoke doesn't fail with NotFound
        worker_id.to_string(),
        vec![],
        WorkerStatus::Running,
        "".into(),
        Default::default(),
        Default::default(),
    );
    // Configure mock to return error
    mock_client.set_worker_execution_behavior(
        worker_id,
        MockExecutionBehavior::Error(error_msg.to_string()),
    );

    let result = runner_service
        .invoke_worker(worker_id.to_string(), payload.clone())
        .await;

    assert!(
        matches!(result, Err(RunnerError::VmConnection(ClientError::Grpc(status))) if status.message() == error_msg)
    );
}

#[tokio::test]
async fn test_invoke_worker_vm_detached_consistency_issue() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-invoke-detached";
    let worker_id = "worker-vm-detached";
    let payload = vec![10, 20, 30];

    // Attach, add mapping, then detach VM *before* invoking worker
    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    add_worker_mapping(&runner_service, worker_id, vm_id).await;
    runner_service.vms.write().await.remove(vm_id); // Simulate detachment

    assert!(
        runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id)
    ); // Mapping still exists
    assert!(runner_service.vms.read().await.get(vm_id).is_none()); // VM is gone

    let result = runner_service
        .invoke_worker(worker_id.to_string(), payload.clone())
        .await;

    // It finds the worker mapping, tries to get the VM client, fails.
    assert!(matches!(result, Err(RunnerError::VmNotAttached(id)) if id == vm_id));
}
