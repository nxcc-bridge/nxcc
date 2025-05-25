use nxcc_interface::proto::vm::WorkerStatus;
use nxcc_vm_base::client::mock::MockExecutionBehavior;

use super::common::*;
use crate::runner::{ClientError, RunnerError};

#[tokio::test]
async fn test_run_worker_success() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-run";
    let manifest_obj = test_worker_manifest();
    let bundle_code = vec![1, 2, 3];
    let bundle_obj = test_worker_bundle(bundle_code.clone());
    // launch_payload is no longer passed directly to run_worker
    let expected_instance_id = "instance-policy-worker-1"; // Default mock ID format

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await; // Clone needed if we inspect mock later

    let result = runner_service
        .run_worker(vm_id.to_string(), manifest_obj.clone(), bundle_obj.clone())
        .await;

    let instance_id = result.unwrap();
    assert_eq!(instance_id, expected_instance_id);

    // Verify worker map
    let worker_map = runner_service.worker_map.read().await;
    assert_eq!(
        worker_map.get(expected_instance_id),
        Some(&vm_id.to_string())
    );

    // Verify mock client state (optional but good)
    let (status, code) = mock_client.get_worker(expected_instance_id).unwrap();
    assert_eq!(status, WorkerStatus::Running);
    assert_eq!(code, bundle_code);

    // Verify no invocations happened directly within run_worker
    let invocations = mock_client.get_invocations(expected_instance_id);
    assert_eq!(
        invocations.len(),
        0,
        "Expected zero invocations directly from run_worker"
    );

    mock_client.clear_invocations(expected_instance_id); // Clear for next tests
}

#[tokio::test]
async fn test_run_worker_vm_not_attached() {
    let (_secrets, runner_service, _mock_client) = setup();
    let vm_id = "vm-not-here";
    let manifest_obj = test_worker_manifest();
    let bundle_obj = test_worker_bundle(vec![1, 2, 3]);

    let result = runner_service
        .run_worker(vm_id.to_string(), manifest_obj.clone(), bundle_obj.clone())
        .await;

    assert!(matches!(result, Err(RunnerError::VmNotAttached(id)) if id == vm_id));
    assert!(runner_service.worker_map.read().await.is_empty());
}

#[tokio::test]
async fn test_run_worker_start_fails_in_vm() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-fail-start";
    let manifest_obj = test_worker_manifest();
    let bundle_obj = test_worker_bundle(vec![1, 2, 3]);
    let error_msg = "VM resource limit exceeded";

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    mock_client.fail_next_operation(error_msg); // Configure mock to fail start_worker

    let result = runner_service
        .run_worker(vm_id.to_string(), manifest_obj.clone(), bundle_obj.clone())
        .await;

    assert!(matches!(result, Err(RunnerError::WorkerStartFailed(msg)) if msg == error_msg));
    assert!(runner_service.worker_map.read().await.is_empty()); // Should not be added to map
}

#[tokio::test]
async fn test_terminate_worker_success() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-term";
    let worker_id = "worker-to-term";

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    add_worker_mapping(&runner_service, worker_id, vm_id).await;
    // Add worker to mock so stop_worker doesn't fail with NotFound initially
    mock_client.add_worker(
        worker_id.to_string(),
        vec![],
        WorkerStatus::Running,
        "".to_string(),
        Default::default(),
        Default::default(),
    );

    assert!(
        runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id)
    );
    assert!(mock_client.get_worker(worker_id).is_some());

    let result = runner_service.terminate_worker(worker_id.to_string()).await;
    result.unwrap();

    // Verify removed from map
    assert!(
        !runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id)
    );
    // Verify removed from mock VM state
    assert!(mock_client.get_worker(worker_id).is_none());
}

#[tokio::test]
async fn test_terminate_worker_not_found_in_vm() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-term-nf-vm";
    let worker_id = "worker-nf-vm";

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    add_worker_mapping(&runner_service, worker_id, vm_id).await;
    // Do NOT add worker to mock client, so stop_worker will return NotFound

    assert!(
        runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id)
    );

    let result = runner_service.terminate_worker(worker_id.to_string()).await;
    result.unwrap(); // Should still be Ok(()) as per code logic

    // Verify removed from map even if VM reported NotFound
    assert!(
        !runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id)
    );
}

#[tokio::test]
async fn test_terminate_worker_not_found_locally() {
    let (_secrets, runner_service, _mock_client) = setup();
    let vm_id = "vm-term-nf-local";
    let worker_id = "worker-nf-local";

    attach_mock_vm(&runner_service, vm_id, _mock_client).await;
    // Do NOT add worker mapping

    assert!(
        !runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id)
    );

    let result = runner_service.terminate_worker(worker_id.to_string()).await;

    assert!(matches!(result, Err(RunnerError::WorkerNotFound(id)) if id == worker_id));
}

#[tokio::test]
async fn test_terminate_worker_vm_detached_consistency_issue() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-term-detached";
    let worker_id = "worker-vm-detached";

    // Attach, add mapping, then detach VM *before* terminating worker
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

    let result = runner_service.terminate_worker(worker_id.to_string()).await;

    // It finds the worker mapping, tries to get the VM client, fails.
    assert!(matches!(result, Err(RunnerError::VmNotAttached(id)) if id == vm_id));
}

#[tokio::test]
async fn test_terminate_worker_fails_in_vm() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-term-fail";
    let worker_id = "worker-term-fail";
    let error_msg = "VM internal error during stop";

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    add_worker_mapping(&runner_service, worker_id, vm_id).await;
    mock_client.add_worker(
        // Add worker so stop doesn't cause NotFound
        worker_id.to_string(),
        vec![],
        WorkerStatus::Running,
        "".into(),
        Default::default(),
        Default::default(),
    );
    mock_client.fail_next_operation(error_msg); // Configure mock to fail stop_worker

    assert!(
        runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id)
    );

    let result = runner_service.terminate_worker(worker_id.to_string()).await;

    assert!(
        matches!(result, Err(RunnerError::VmConnection(ClientError::Grpc(status))) if status.message() == error_msg)
    );
    // Verify *not* removed from map on general failure
    assert!(
        runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id)
    );
}
