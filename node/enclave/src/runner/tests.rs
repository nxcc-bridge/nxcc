use std::sync::Arc;

use nxcc_interface::{
    proto::vm::WorkerStatus,
    types::{
        AttestationReport, ConsumerInfo, DSSE_WORKER_BUNDLE_PAYLOAD_TYPE, DsseEnvelope,
        DsseSignatureEntry, EnvReport, PolicyExecutionRequest, SecretId, WorkerBundle,
        WorkerBundlePayload, WorkerBundlePointer, WorkerManifest,
    },
};
use nxcc_vm_base::client::{
    VmClient as _,
    mock::{MockExecutionBehavior, MockVmServiceClient},
};

use super::*; // Import items from the outer module (RunnerService, RunnerError, etc.)
use crate::secrets::Secrets; // Assuming secrets.rs is in the same crate src dir

// Helper function to create a default SecretId for tests
fn test_secret_id(id: u64) -> SecretId {
    SecretId {
        chain_id: 1,
        identity_address: format!("0x{:040x}", id).parse().unwrap(),
        identity_id: alloy_primitives::Uint::from_limbs_slice(&[id]),
    }
}

// Helper function to create a default PolicyExecutionRequest for tests
fn test_policy_request(node_id: &str, secret_ids: Vec<SecretId>) -> PolicyExecutionRequest {
    PolicyExecutionRequest {
        secret_ids,
        consumer: ConsumerInfo {
            bundle_hash: vec![1; 32], // Changed from code_hash
            signature: vec![2; 64],   // Assuming signature remains
        },
        env_report: EnvReport {
            attestation: AttestationReport {
                measurement: vec![0u8; 32],
                ephemeral_public_key: vec![3; 32], // Needs to be 32 bytes for Secrets mock
                block_hashes: vec![vec![4, 5], vec![6, 7]],
                user_data: vec![8, 9],
            },
            operator_signature: vec![10; 64],
            node_id: node_id.to_string(),
        },
    }
}

// Helper setup function
fn setup() -> (Arc<Secrets>, RunnerService, MockVmServiceClient) {
    let secrets = Secrets::new();
    let runner_service = RunnerService::new(secrets.clone());
    let mock_client = MockVmServiceClient::new();
    (secrets, runner_service, mock_client)
}

// Helper to manually "attach" a mock VM
async fn attach_mock_vm(runner_service: &RunnerService, vm_id: &str, client: MockVmServiceClient) {
    let mut vms_guard = runner_service.vms.write().await;
    vms_guard.insert(vm_id.to_string(), client.into());
}

// Helper to manually add a worker mapping
async fn add_worker_mapping(runner_service: &RunnerService, worker_id: &str, vm_id: &str) {
    let mut worker_map_guard = runner_service.worker_map.write().await;
    worker_map_guard.insert(worker_id.to_string(), vm_id.to_string());
}

// Helper to create a default WorkerManifest for tests
fn test_worker_manifest() -> WorkerManifest {
    WorkerManifest {
        bundle: WorkerBundlePointer {
            source: "file:dummy.js".parse().unwrap(),
            hash: None,
        },
        identities: vec![],
        userdata: Default::default(),
    }
}

// Helper to create a default WorkerBundle for tests
fn test_worker_bundle(executable_code: Vec<u8>) -> WorkerBundle {
    let payload_struct = WorkerBundlePayload {
        vm: "test-vm".to_string(),
        executable: executable_code,
        metadata: Default::default(),
    };
    let json_payload_bytes = serde_json::to_vec(&payload_struct).unwrap();

    let dsse_envelope = DsseEnvelope {
        payload: base64::encode(&json_payload_bytes),
        payload_type: DSSE_WORKER_BUNDLE_PAYLOAD_TYPE.to_string(),
        signatures: vec![DsseSignatureEntry {
            key_id: Some("test_key_id".to_string()),
            // Using a valid base64 string for the mock signature
            sig: base64::encode(b"mock_signature_bytes_longer_than_32_for_base64"),
        }],
    };
    WorkerBundle(serde_json::to_vec(&dsse_envelope).unwrap())
}

#[tokio::test]
async fn test_new_runner_service() {
    let (secrets, runner_service, _) = setup();
    assert!(runner_service.vms.read().await.is_empty());
    assert!(runner_service.worker_map.read().await.is_empty());
    // Check if the secrets Arc points to the same allocation
    assert!(Arc::ptr_eq(&runner_service.secrets, &secrets));
}

// Note: We don't test the real attach_vm due to network/TLS complexity.
// We test the state changes via manual insertion and detach_vm.

#[tokio::test]
async fn test_detach_vm_exists() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-1";
    let worker_id_1 = "worker-on-vm1-1";
    let worker_id_2 = "worker-on-vm1-2";
    let worker_id_other = "worker-on-vm2";

    attach_mock_vm(&runner_service, vm_id, mock_client).await;
    add_worker_mapping(&runner_service, worker_id_1, vm_id).await;
    add_worker_mapping(&runner_service, worker_id_2, vm_id).await;
    add_worker_mapping(&runner_service, worker_id_other, "vm-2").await; // Belongs to another VM

    assert!(runner_service.vms.read().await.contains_key(vm_id));
    assert!(
        runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id_1)
    );
    assert!(
        runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id_2)
    );
    assert!(
        runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id_other)
    );

    let result = runner_service.detach_vm(vm_id.to_string()).await;
    result.unwrap(); // Expect Ok

    assert!(!runner_service.vms.read().await.contains_key(vm_id));
    // Check workers associated with vm_id are removed
    assert!(
        !runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id_1)
    );
    assert!(
        !runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id_2)
    );
    // Check worker on other VM remains
    assert!(
        runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id_other)
    );
}

#[tokio::test]
async fn test_detach_vm_not_exists() {
    let (_secrets, runner_service, _mock_client) = setup();
    let vm_id = "vm-nonexistent";

    assert!(!runner_service.vms.read().await.contains_key(vm_id));

    // Detaching a non-existent VM should be Ok (idempotent)
    let result = runner_service.detach_vm(vm_id.to_string()).await;
    result.unwrap();

    assert!(!runner_service.vms.read().await.contains_key(vm_id));
    assert!(runner_service.worker_map.read().await.is_empty());
}

#[tokio::test]
async fn test_run_worker_success() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-run";
    let manifest_obj = test_worker_manifest();
    let bundle_code = vec![1, 2, 3];
    let bundle_obj = test_worker_bundle(bundle_code.clone());
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
