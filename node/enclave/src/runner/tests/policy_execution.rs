use nxcc_interface::proto::vm::WorkerStatus;
use nxcc_vm_base::client::mock::MockExecutionBehavior;

use super::common::*;
use crate::runner::{ClientError, RunnerError};

#[tokio::test]
async fn test_execute_policy_success_some_satisfied() {
    let (secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-policy";
    let worker_id = "policy-worker-1";
    let node_id_1 = "node-1";
    let node_id_2 = "node-2";
    let secret_id_1 = test_secret_id(101);
    let secret_id_2 = test_secret_id(102);
    let secret_id_3 = test_secret_id(103);

    let context1 = test_policy_request(node_id_1, vec![secret_id_1.clone()]);
    let context2 = test_policy_request(node_id_2, vec![secret_id_2.clone(), secret_id_3.clone()]);
    let contexts = vec![context1.clone(), context2.clone()];

    // Expected VM response: context1=true, context2=false
    let vm_response_bools = vec![true, false];
    let vm_response_payload = serde_json::to_vec(&vm_response_bools).unwrap();

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
    mock_client.set_worker_execution_behavior(
        worker_id,
        MockExecutionBehavior::Fixed(vm_response_payload.clone()),
    );

    // Check initial authorization state
    assert!(!secrets.check_authorization(
        &context1.env_report.attestation,
        &secret_id_1,
        &context1.consumer
    ));
    assert!(!secrets.check_authorization(
        &context2.env_report.attestation,
        &secret_id_2,
        &context2.consumer
    ));
    assert!(!secrets.check_authorization(
        &context2.env_report.attestation,
        &secret_id_3,
        &context2.consumer
    ));

    let result = runner_service
        .execute_policy(worker_id.to_string(), contexts.clone())
        .await;

    let satisfied_contexts = result.unwrap();

    // Verify only context1 is returned
    assert_eq!(satisfied_contexts.len(), 1);
    // Deep comparison might be needed if PolicyExecutionRequest doesn't impl PartialEq well
    assert_eq!(
        satisfied_contexts[0].env_report.node_id,
        context1.env_report.node_id
    );
    assert_eq!(satisfied_contexts[0].secret_ids, context1.secret_ids);

    // Verify authorization stored only for satisfied context
    assert!(secrets.check_authorization(
        &context1.env_report.attestation,
        &secret_id_1,
        &context1.consumer
    ));
    assert!(!secrets.check_authorization(
        &context2.env_report.attestation,
        &secret_id_2,
        &context2.consumer
    )); // context2 failed
    assert!(!secrets.check_authorization(
        &context2.env_report.attestation,
        &secret_id_3,
        &context2.consumer
    )); // context2 failed
}

#[tokio::test]
async fn test_execute_policy_success_all_satisfied() {
    let (secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-policy-all";
    let worker_id = "policy-worker-all";
    let node_id_1 = "node-all-1";
    let secret_id_1 = test_secret_id(201);

    let context1 = test_policy_request(node_id_1, vec![secret_id_1.clone()]);
    let contexts = vec![context1.clone()];

    // Expected VM response: context1=true
    let vm_response_bools = vec![true];
    let vm_response_payload = serde_json::to_vec(&vm_response_bools).unwrap();

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    add_worker_mapping(&runner_service, worker_id, vm_id).await;
    mock_client.add_worker(
        worker_id.to_string(),
        vec![],
        WorkerStatus::Running,
        "".into(),
        Default::default(),
        Default::default(),
    );
    mock_client.set_worker_execution_behavior(
        worker_id,
        MockExecutionBehavior::Fixed(vm_response_payload),
    );

    assert!(!secrets.check_authorization(
        &context1.env_report.attestation,
        &secret_id_1,
        &context1.consumer
    ));

    let result = runner_service
        .execute_policy(worker_id.to_string(), contexts.clone())
        .await;
    let satisfied_contexts = result.unwrap();

    assert_eq!(satisfied_contexts.len(), 1);
    assert_eq!(
        satisfied_contexts[0].env_report.node_id,
        context1.env_report.node_id
    );
    assert!(secrets.check_authorization(
        &context1.env_report.attestation,
        &secret_id_1,
        &context1.consumer
    ));
}

#[tokio::test]
async fn test_execute_policy_success_none_satisfied() {
    let (secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-policy-none";
    let worker_id = "policy-worker-none";
    let node_id_1 = "node-none-1";
    let secret_id_1 = test_secret_id(301);

    let context1 = test_policy_request(node_id_1, vec![secret_id_1.clone()]);
    let contexts = vec![context1.clone()];

    // Expected VM response: context1=false
    let vm_response_bools = vec![false];
    let vm_response_payload = serde_json::to_vec(&vm_response_bools).unwrap();

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    add_worker_mapping(&runner_service, worker_id, vm_id).await;
    mock_client.add_worker(
        worker_id.to_string(),
        vec![],
        WorkerStatus::Running,
        "".into(),
        Default::default(),
        Default::default(),
    );
    mock_client.set_worker_execution_behavior(
        worker_id,
        MockExecutionBehavior::Fixed(vm_response_payload),
    );

    assert!(!secrets.check_authorization(
        &context1.env_report.attestation,
        &secret_id_1,
        &context1.consumer
    ));

    let result = runner_service
        .execute_policy(worker_id.to_string(), contexts.clone())
        .await;
    let satisfied_contexts = result.unwrap();

    assert!(satisfied_contexts.is_empty());
    assert!(!secrets.check_authorization(
        &context1.env_report.attestation,
        &secret_id_1,
        &context1.consumer
    )); // Still not authorized
}

#[tokio::test]
async fn test_execute_policy_vm_invocation_fails() {
    let (secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-policy-fail";
    let worker_id = "policy-worker-fail";
    let node_id_1 = "node-fail-1";
    let secret_id_1 = test_secret_id(401);
    let error_msg = "Policy worker crashed";

    let context1 = test_policy_request(node_id_1, vec![secret_id_1.clone()]);
    let contexts = vec![context1.clone()];

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    add_worker_mapping(&runner_service, worker_id, vm_id).await;
    mock_client.add_worker(
        worker_id.to_string(),
        vec![],
        WorkerStatus::Running,
        "".into(),
        Default::default(),
        Default::default(),
    );
    mock_client.set_worker_execution_behavior(
        worker_id,
        MockExecutionBehavior::Error(error_msg.to_string()),
    );

    let result = runner_service
        .execute_policy(worker_id.to_string(), contexts.clone())
        .await;

    assert!(
        matches!(result, Err(RunnerError::VmConnection(ClientError::Grpc(status))) if status.message() == error_msg)
    );
    assert!(!secrets.check_authorization(
        &context1.env_report.attestation,
        &secret_id_1,
        &context1.consumer
    )); // No authorization granted
}

#[tokio::test]
async fn test_execute_policy_deserialization_error() {
    let (secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-policy-deser";
    let worker_id = "policy-worker-deser";
    let node_id_1 = "node-deser-1";
    let secret_id_1 = test_secret_id(501);

    let context1 = test_policy_request(node_id_1, vec![secret_id_1.clone()]);
    let contexts = vec![context1.clone()];

    // VM returns invalid CBOR data (not a Vec<bool>)
    let vm_response_payload = b"invalid cbor data".to_vec();

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    add_worker_mapping(&runner_service, worker_id, vm_id).await;
    mock_client.add_worker(
        worker_id.to_string(),
        vec![],
        WorkerStatus::Running,
        "".into(),
        Default::default(),
        Default::default(),
    );
    mock_client.set_worker_execution_behavior(
        worker_id,
        MockExecutionBehavior::Fixed(vm_response_payload.clone()),
    );

    let result = runner_service
        .execute_policy(worker_id.to_string(), contexts.clone())
        .await;

    assert!(matches!(result, Err(RunnerError::Deserialization(_))));
    assert!(!secrets.check_authorization(
        &context1.env_report.attestation,
        &secret_id_1,
        &context1.consumer
    )); // No authorization granted
}

#[tokio::test]
async fn test_execute_policy_mismatched_result_count() {
    let (secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-policy-mismatch";
    let worker_id = "policy-worker-mismatch";
    let node_id_1 = "node-mismatch-1";
    let secret_id_1 = test_secret_id(601);

    let context1 = test_policy_request(node_id_1, vec![secret_id_1.clone()]);
    let contexts = vec![context1.clone()]; // Requesting 1 context

    // VM returns results for 2 contexts (incorrect)
    let vm_response_bools = vec![true, false];
    let vm_response_payload = serde_json::to_vec(&vm_response_bools).unwrap();

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    add_worker_mapping(&runner_service, worker_id, vm_id).await;
    mock_client.add_worker(
        worker_id.to_string(),
        vec![],
        WorkerStatus::Running,
        "".into(),
        Default::default(),
        Default::default(),
    );
    mock_client.set_worker_execution_behavior(
        worker_id,
        MockExecutionBehavior::Fixed(vm_response_payload.clone()),
    );

    let result = runner_service
        .execute_policy(worker_id.to_string(), contexts.clone())
        .await;

    assert!(
        matches!(result, Err(RunnerError::PolicyExecutionFailed(msg)) if msg == "Mismatched result count")
    );
    assert!(!secrets.check_authorization(
        &context1.env_report.attestation,
        &secret_id_1,
        &context1.consumer
    )); // No authorization granted
}

#[tokio::test]
async fn test_execute_policy_worker_not_found_locally() {
    let (secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-policy-nf-local";
    let worker_id = "policy-worker-nf-local";
    let node_id_1 = "node-nf-local-1";
    let secret_id_1 = test_secret_id(701);

    let context1 = test_policy_request(node_id_1, vec![secret_id_1.clone()]);
    let contexts = vec![context1.clone()];

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    // Do NOT add worker mapping

    let result = runner_service
        .execute_policy(worker_id.to_string(), contexts.clone())
        .await;

    assert!(matches!(result, Err(RunnerError::WorkerNotFound(id)) if id == worker_id));
    assert!(!secrets.check_authorization(
        &context1.env_report.attestation,
        &secret_id_1,
        &context1.consumer
    )); // No authorization granted
}

#[tokio::test]
async fn test_execute_policy_vm_detached_consistency_issue() {
    let (secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-policy-detached";
    let worker_id = "policy-worker-detached";
    let node_id_1 = "node-detached-1";
    let secret_id_1 = test_secret_id(801);

    let context1 = test_policy_request(node_id_1, vec![secret_id_1.clone()]);
    let contexts = vec![context1.clone()];

    // Attach, add mapping, then detach VM *before* executing policy
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
        .execute_policy(worker_id.to_string(), contexts.clone())
        .await;

    // It finds the worker mapping, tries to get the VM client, fails.
    assert!(matches!(result, Err(RunnerError::VmNotAttached(id)) if id == vm_id));
    assert!(!secrets.check_authorization(
        &context1.env_report.attestation,
        &secret_id_1,
        &context1.consumer
    )); // No authorization granted
}
