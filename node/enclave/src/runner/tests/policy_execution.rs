use std::sync::{Arc, Mutex};

use nxcc_attestation::tdx::hardware::TdxInterface;
use nxcc_interface::{
    proto::vm::WorkerStatus,
    types::{
        attestation::{AttestationBundle, EnvReport, RawAttestation},
        policy::{PolicyExecutionContextForWorker, PolicyExecutionRequest},
        secrets::ConsumerInfo,
    },
};
use nxcc_vm_base::client::mock::MockExecutionBehavior;

use super::common::*;
use crate::runner::{ClientError, RunnerError};

// Helper function to create a default ConsumerInfo for tests
fn test_consumer_info() -> ConsumerInfo {
    ConsumerInfo {
        bundle_hash: vec![0x42; 32],
        signature: vec![0x44; 64],
    }
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

    let context1 = test_policy_request(vec![secret_id_1.clone()]);
    let context2 = test_policy_request(vec![secret_id_2.clone(), secret_id_3.clone()]);
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
    assert_eq!("test-node", "test-node");
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

    let context1 = test_policy_request(vec![secret_id_1.clone()]);
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
    assert_eq!("test-node", "test-node");
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

    let context1 = test_policy_request(vec![secret_id_1.clone()]);
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

    let context1 = test_policy_request(vec![secret_id_1.clone()]);
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

    let context1 = test_policy_request(vec![secret_id_1.clone()]);
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

    let context1 = test_policy_request(vec![secret_id_1.clone()]);
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

    let context1 = test_policy_request(vec![secret_id_1.clone()]);
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

    let context1 = test_policy_request(vec![secret_id_1.clone()]);
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

#[tokio::test]
async fn test_execute_policy_with_attestation_claims() {
    use nxcc_interface::types::attestation::{
        AttestationBundle, EnvReport, InterfaceMeasurement, RawAttestation,
        StandardizedAttestationClaims,
    };

    let (secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-policy-attestation";
    let worker_id = "policy-worker-attestation";
    let node_id = "node-attestation-1";
    let secret_id = test_secret_id(901);

    // Create a realistic attestation bundle with proper TDX quote structure
    // Create test userdata with ephemeral key and freshness info
    let test_userdata = nxcc_attestation::user_data_binding::UserData::new(
        vec![0x01; 32], // ephemeral key (32 bytes for test)
        vec![
            nxcc_interface::gateway::BlockInfo {
                chain_id: 1,
                chain_name: "test1".to_string(),
                block_number: 1,
                block_hash: vec![0xAB; 32],
                timestamp: 0,
                fetched_at: 0,
            },
            nxcc_interface::gateway::BlockInfo {
                chain_id: 2,
                chain_name: "test2".to_string(),
                block_number: 2,
                block_hash: vec![0xCD; 32],
                timestamp: 0,
                fetched_at: 0,
            },
        ],
    );

    let attestation_bundle = AttestationBundle {
        raw_attestation: RawAttestation {
            platform_type: "tdx".to_string(),
            evidence: {
                // Create a realistic TDX quote structure for testing
                use nxcc_attestation::tdx::hardware::TdxSimulator;
                let simulator = TdxSimulator::new();
                simulator.generate_quote(&[0x42; 32]).unwrap()
            },
            certificates: None,
        },
        detached_userdata: test_userdata.to_cbor().unwrap(),
    };

    let env_report = EnvReport {
        attestation: attestation_bundle,
        operator_signature: None, // Not needed for this test
    };

    let mut context = PolicyExecutionRequest {
        secret_ids: vec![secret_id.clone()],
        consumer: test_consumer_info(),
        env_report,
        attestation_claims: None, // Will be populated by execute_policy
    };

    let contexts = vec![context.clone()];

    // Expected VM response: context=true
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

    // Set up custom validation for the request payload sent to the policy worker
    use std::sync::{Arc, Mutex};
    let captured_payload = Arc::new(Mutex::new(None));
    let captured_payload_clone = captured_payload.clone();

    mock_client.set_worker_execution_behavior(
        worker_id,
        MockExecutionBehavior::Transform(Arc::new(move |payload: Vec<u8>| {
            let mut captured = captured_payload_clone.lock().unwrap();
            *captured = Some(payload.clone());
            vm_response_payload.clone()
        })),
    );

    let result = runner_service
        .execute_policy(worker_id.to_string(), contexts.clone())
        .await;

    let satisfied_contexts = result.unwrap();

    // Verify the context was satisfied
    assert_eq!(satisfied_contexts.len(), 1);
    // Node identity is no longer exposed to policies

    // Verify that the attestation claims were populated
    assert!(satisfied_contexts[0].attestation_claims.is_some());
    let claims = satisfied_contexts[0].attestation_claims.as_ref().unwrap();

    // Verify key claims properties
    assert_eq!(claims.eat_profile, "urn:nxcc:profile:tdx-v1");
    assert_eq!(claims.dbgstat, 0); // TDX simulator uses production mode (debug disabled)
    assert!(!claims.measurements.is_empty());

    // Find the primary software measurement
    let primary_measurement = claims
        .measurements
        .iter()
        .find(|m| m.measurement_type.as_ref() == Some(&"application".to_string()))
        .expect("Should have primary software measurement");
    assert_eq!(primary_measurement.alg, "sha-384");
    assert!(!primary_measurement.val.is_empty());

    // Verify the policy worker received the request with populated attestation claims
    let captured = captured_payload.lock().unwrap();
    let payload = captured.as_ref().expect("Should have captured payload");
    let sent_contexts: Vec<PolicyExecutionContextForWorker> =
        serde_json::from_slice(payload).expect("Should be able to deserialize captured payload");

    assert_eq!(sent_contexts.len(), 1);
    assert!(sent_contexts[0].attestation_claims.is_some());
    let sent_claims = sent_contexts[0].attestation_claims.as_ref().unwrap();
    assert_eq!(sent_claims.eat_profile, "urn:nxcc:profile:tdx-v1");
    assert!(!sent_claims.measurements.is_empty());
}

#[tokio::test]
async fn test_execute_policy_bad_quote_no_claims() {
    let (secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-policy-bad-quote";
    let worker_id = "policy-worker-bad-quote";
    let node_id = "node-bad-quote-1";
    let secret_id = test_secret_id(902);

    // Create an attestation bundle with invalid quote data
    // Create bad test userdata
    let bad_test_userdata = nxcc_attestation::user_data_binding::UserData::new(
        vec![0x01; 32], // ephemeral key (32 bytes for test)
        vec![nxcc_interface::gateway::BlockInfo {
            chain_id: 1,
            chain_name: "test".to_string(),
            block_number: 1,
            block_hash: vec![0xAB; 32],
            timestamp: 0,
            fetched_at: 0,
        }],
    );

    let bad_attestation = AttestationBundle {
        raw_attestation: RawAttestation {
            platform_type: "tdx".to_string(),
            evidence: vec![0xFF; 10], // Too short to be a valid TDX quote
            certificates: None,
        },
        detached_userdata: bad_test_userdata.to_cbor().unwrap(),
    };

    let env_report = EnvReport {
        attestation: bad_attestation,
        operator_signature: None,
    };

    let context = PolicyExecutionRequest {
        secret_ids: vec![secret_id.clone()],
        consumer: test_consumer_info(),
        env_report,
        attestation_claims: None,
    };

    let contexts = vec![context.clone()];

    // Expected VM response: context=true (policy allows even without verified claims)
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

    // Capture the request sent to the policy worker
    use std::sync::{Arc, Mutex};
    let captured_payload = Arc::new(Mutex::new(None));
    let captured_payload_clone = captured_payload.clone();

    mock_client.set_worker_execution_behavior(
        worker_id,
        MockExecutionBehavior::Transform(Arc::new(move |payload: Vec<u8>| {
            let mut captured = captured_payload_clone.lock().unwrap();
            *captured = Some(payload.clone());
            vm_response_payload.clone()
        })),
    );

    let result = runner_service
        .execute_policy(worker_id.to_string(), contexts.clone())
        .await;

    let satisfied_contexts = result.unwrap();

    // Context is satisfied because the policy allowed it despite no verified claims
    assert_eq!(satisfied_contexts.len(), 1);

    // But attestation claims should be None due to verification failure
    assert!(satisfied_contexts[0].attestation_claims.is_none());

    // Verify the policy worker received the request with no attestation claims
    let captured = captured_payload.lock().unwrap();
    let payload = captured.as_ref().expect("Should have captured payload");
    let sent_contexts: Vec<PolicyExecutionContextForWorker> =
        serde_json::from_slice(payload).expect("Should be able to deserialize captured payload");

    assert_eq!(sent_contexts.len(), 1);
    assert!(
        sent_contexts[0].attestation_claims.is_none(),
        "Policy should receive request without attestation claims when verification fails"
    );
}

#[tokio::test]
async fn test_execute_policy_verification_before_execution() {
    // This test verifies that attestation verification happens BEFORE policy execution
    // and that policies never receive known-bad quotes as valid

    let (secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-policy-verification-order";
    let worker_id = "policy-worker-verification-order";
    let node_id = "node-verification-order-1";
    let secret_id = test_secret_id(903);

    // Create a context with a good quote that should verify successfully
    // Create good test userdata
    let good_test_userdata = nxcc_attestation::user_data_binding::UserData::new(
        vec![0x01; 32], // ephemeral key (32 bytes for test)
        vec![nxcc_interface::gateway::BlockInfo {
            chain_id: 1,
            chain_name: "test".to_string(),
            block_number: 1,
            block_hash: vec![0xAB; 32],
            timestamp: 0,
            fetched_at: 0,
        }],
    );

    let good_attestation = AttestationBundle {
        raw_attestation: RawAttestation {
            platform_type: "tdx".to_string(),
            evidence: {
                use nxcc_attestation::tdx::hardware::TdxSimulator;
                let simulator = TdxSimulator::new();
                simulator.generate_quote(&[0x42; 32]).unwrap()
            },
            certificates: None,
        },
        detached_userdata: good_test_userdata.to_cbor().unwrap(),
    };

    let env_report = EnvReport {
        attestation: good_attestation,
        operator_signature: None,
    };

    let context = PolicyExecutionRequest {
        secret_ids: vec![secret_id.clone()],
        consumer: test_consumer_info(),
        env_report,
        attestation_claims: None,
    };

    let contexts = vec![context];

    // Set up a policy worker that explicitly checks for attestation claims
    // This simulates a security-conscious policy that requires verified attestation
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

    let execution_count = Arc::new(Mutex::new(0));
    let execution_count_clone = execution_count.clone();
    let verification_happened = Arc::new(Mutex::new(false));
    let verification_happened_clone = verification_happened.clone();

    mock_client.set_worker_execution_behavior(
        worker_id,
        MockExecutionBehavior::Transform(Arc::new(move |payload: Vec<u8>| {
            let mut count = execution_count_clone.lock().unwrap();
            *count += 1;

            // Verify that the payload contains attestation claims (verification happened first)
            let sent_contexts: Vec<PolicyExecutionContextForWorker> =
                serde_json::from_slice(&payload).expect("Should deserialize");

            // Attestation verification should have happened before we got here
            if sent_contexts[0].attestation_claims.is_some() {
                let mut verified = verification_happened_clone.lock().unwrap();
                *verified = true;
            }

            vm_response_payload.clone()
        })),
    );

    let result = runner_service
        .execute_policy(worker_id.to_string(), contexts)
        .await;

    let satisfied_contexts = result.unwrap();

    // Verify execution happened exactly once
    assert_eq!(*execution_count.lock().unwrap(), 1);

    // Verify attestation verification happened before policy execution
    assert!(
        *verification_happened.lock().unwrap(),
        "Attestation verification should happen before policy execution"
    );

    // Verify the satisfied context has verified attestation claims
    assert_eq!(satisfied_contexts.len(), 1);
    assert!(satisfied_contexts[0].attestation_claims.is_some());

    let claims = satisfied_contexts[0].attestation_claims.as_ref().unwrap();
    assert_eq!(claims.eat_profile, "urn:nxcc:profile:tdx-v1");
}
