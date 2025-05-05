use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use alloy_primitives::{Address, U256};
use nxcc_interface::{
    proto::enclave::{
        DetachVmRequest, ExecutePolicyRequest as ProtoExecutePolicyRequest, GenerateSecretsRequest,
        GetReportRequest, GetSecretsRequest as ProtoGetSecretsRequest, InvokeWorkerRequest,
        PutSecretsRequest as ProtoPutSecretsRequest, RunWorkerRequest, SecretRequest,
        SecretsBundle, TerminateWorkerRequest, runner_server::Runner as _,
        secrets_server::Secrets as _,
    },
    types::{
        AttestationReport, ConsumerInfo, EnvReport, FromProto as _, IntoProto as _,
        PolicyExecutionReport, PolicyExecutionRequest, SecretId, SecretsBox,
    },
};
use nxcc_vm_base::client::mock::{MockExecutionBehavior, MockVmServiceClient};
use tonic::{Code, Request};
use tracing::info;

use crate::{
    crypto::{KeyExchangeKeyPair, decrypt_secrets_box, encrypt_secrets_box},
    grpc::{EnclaveRunnerGrpcService, SecretsGrpcService},
    runner::RunnerService,
    secrets::Secrets,
};

fn test_secret_id(id_num: u64) -> SecretId {
    SecretId {
        chain_id: 1,
        identity_address: Address::from_slice(&[id_num as u8; 20]),
        identity_id: U256::from(id_num),
    }
}

fn test_consumer_info() -> ConsumerInfo {
    ConsumerInfo {
        code_hash: vec![1; 32],
        signature: vec![2; 64],
    }
}

// Creates an EnvReport suitable for *policy evaluation* (doesn't need specific hash)
fn test_policy_env_report(node_id: &str) -> EnvReport {
    let kx = KeyExchangeKeyPair::generate(); // Need a valid key for attestation structure
    EnvReport {
        attestation: AttestationReport {
            ephemeral_public_key: kx.public_key().as_bytes().to_vec(),
            block_hashes: vec![vec![1, 2]],
            user_data: vec![0u8; 32], // Placeholder hash for policy eval
        },
        operator_signature: vec![3; 64],
        node_id: node_id.to_string(),
    }
}

// Creates an EnvReport suitable for *sending secrets* (needs correct binding hash)
fn test_sending_env_report(
    node_id: &str,
    sender_kx_pk: &[u8], // Sender's KX PubKey used in attestation
    binding_hash: Vec<u8>,
) -> EnvReport {
    EnvReport {
        attestation: AttestationReport {
            ephemeral_public_key: sender_kx_pk.to_vec(),
            block_hashes: vec![vec![4, 5]],
            user_data: binding_hash, // Crucial: hash of the secrets box
        },
        operator_signature: vec![6; 64],
        node_id: node_id.to_string(),
    }
}

// Creates an EnvReport suitable for *requesting secrets* (needs getter's pubkey)
fn test_requesting_env_report(node_id: &str, getter_kx_pk: &[u8]) -> EnvReport {
    EnvReport {
        attestation: AttestationReport {
            ephemeral_public_key: getter_kx_pk.to_vec(), // Getter's KX PubKey
            block_hashes: vec![vec![7, 8]],
            user_data: vec![9u8; 32], // User data content not critical for GetSecrets verification itself
        },
        operator_signature: vec![10; 64],
        node_id: node_id.to_string(),
    }
}

fn test_policy_request(node_id: &str, secret_ids: Vec<SecretId>) -> PolicyExecutionRequest {
    PolicyExecutionRequest {
        secret_ids,
        consumer: test_consumer_info(),
        env_report: test_policy_env_report(node_id),
    }
}

// --- The Integration Test (Refactored) ---

#[tokio::test]
#[tracing_test::traced_test]
async fn test_enclave_workflow() {
    let secrets_service = Secrets::new();
    let runner_service = Arc::new(RunnerService::new(secrets_service.clone()));
    let mock_vm_client = MockVmServiceClient::new();

    let secrets_grpc = SecretsGrpcService::new(secrets_service.clone());
    let runner_grpc = EnclaveRunnerGrpcService::new(runner_service.clone());

    let vm_id = "mock-vm-01";
    let policy_worker_type_id = "policy-worker"; // Used in RunWorker
    let policy_worker_code = b"mock_policy_wasm".to_vec();
    let policy_manifest = b"{}".to_vec();
    let expected_policy_worker_instance_id = format!("instance-{}-1", policy_worker_type_id); // Default mock format

    let secret_id = test_secret_id(12345);
    let secret_data = b"this is the secret data".to_vec();
    let secret_expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600; // Expires in 1 hour

    let putter_node_id = "node-putter";
    let getter_node_id = "node-getter";

    // --- 1. Runner attaches a mock VM ---
    // Use the test-only helper method on RunnerService directly
    runner_service
        .attach_mock_client(vm_id.to_string(), mock_vm_client.clone())
        .await;
    // Verification: Subsequent calls targeting this vm_id should succeed (if mock is configured correctly).
    // We can't directly check the internal map anymore.
    info!("Step 1: Mock VM Attached via test helper");

    // --- 2. Runner starts a mock policy worker ---
    let run_worker_req = Request::new(RunWorkerRequest {
        vm_id: vm_id.to_string(),
        worker_code: policy_worker_code.clone(),
        manifest: policy_manifest.clone(),
    });
    // Configure mock VM to succeed start_worker (default behavior)

    let run_worker_resp = runner_grpc
        .run_worker(run_worker_req)
        .await
        .expect("RunWorker call failed");
    let run_worker_inner = run_worker_resp.into_inner();
    let policy_worker_id = run_worker_inner.worker_id;

    assert!(run_worker_inner.success, "RunWorker should succeed");
    assert_eq!(
        policy_worker_id, expected_policy_worker_instance_id,
        "Unexpected worker ID"
    );
    // Verification: The worker_id is returned and success is true.
    info!(
        "Step 2: Policy worker '{}' started in VM '{}'",
        policy_worker_id, vm_id
    );

    // --- 3. Policy execution fails (wrong context), PutSecret rejected ---
    let policy_req_fail = test_policy_request(putter_node_id, vec![secret_id.clone()]); // Request for putter

    // Configure mock VM to return 'false' for this policy execution
    let vm_response_fail = vec![false];
    let vm_response_payload_fail = serde_json::to_vec(&vm_response_fail).unwrap();
    mock_vm_client.set_worker_execution_behavior(
        &policy_worker_id,
        MockExecutionBehavior::Fixed(vm_response_payload_fail.clone()),
    );

    let execute_req_fail = Request::new(ProtoExecutePolicyRequest {
        worker_id: policy_worker_id.clone(),
        contexts: vec![policy_req_fail.to_proto()],
    });

    let execute_resp_fail = runner_grpc
        .execute_policy(execute_req_fail)
        .await
        .expect("ExecutePolicy (fail) call failed");

    assert!(
        execute_resp_fail.into_inner().satisfied_contexts.is_empty(),
        "Policy should not have been satisfied"
    );
    // Verify no authorization was stored (indirectly, by trying PutSecrets)
    info!("Step 3a: Policy execution failed as expected");

    // Get enclave public key via GetReport
    let report_req = Request::new(GetReportRequest { user_data: vec![] }); // user_data not critical here
    let report_resp = secrets_grpc
        .get_report(report_req)
        .await
        .expect("GetReport failed");
    let enclave_pk_bytes = report_resp.into_inner().ephemeral_public_key;
    let enclave_pk = x25519_dalek::PublicKey::from(
        <[u8; 32]>::try_from(enclave_pk_bytes.as_slice())
            .expect("Invalid pubkey len from GetReport"),
    );

    // Attempt PutSecret - should fail due to missing authorization
    let putter_kx = KeyExchangeKeyPair::generate();
    let secrets_to_send = vec![(secret_id.clone(), secret_data.clone(), secret_expiry)];
    let secrets_box_for_put = encrypt_secrets_box(&putter_kx, &enclave_pk, &secrets_to_send)
        .expect("Failed to encrypt secrets for put");
    let binding_hash = secrets_box_for_put.calculate_binding_hash();
    let putter_env_report = test_sending_env_report(
        putter_node_id,
        putter_kx.public_key().as_bytes(),
        binding_hash.to_vec(),
    );

    let put_secrets_req = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box_for_put.to_proto()),
            env_report: Some(putter_env_report.to_proto()),
        }],
    });

    let put_secrets_resp = secrets_grpc
        .put_secrets(put_secrets_req)
        .await
        .expect("PutSecrets call failed unexpectedly"); // The call itself shouldn't fail, but success should be false

    assert!(
        !put_secrets_resp.into_inner().success,
        "PutSecrets should have been rejected (returned success=false) due to missing \
         authorization"
    );
    // Verify secret not stored using CheckSecrets
    let check_req = Request::new(nxcc_interface::proto::enclave::CheckSecretsRequest {
        ids: vec![secret_id.to_proto()],
    });
    let check_resp = secrets_grpc
        .check_secrets(check_req)
        .await
        .expect("CheckSecrets failed");
    let statuses = check_resp.into_inner().statuses;
    assert_eq!(statuses.len(), 1);
    assert!(
        !statuses[0].found,
        "Secret should not be found after rejected Put"
    );
    info!("Step 3b: PutSecret rejected as expected");

    // --- 4. Policy execution succeeds ---
    let policy_req_ok = test_policy_request(putter_node_id, vec![secret_id.clone()]); // Same request, different VM behavior

    // Configure mock VM to return 'true' for this policy execution
    let vm_response_ok = vec![true];
    let vm_response_payload_ok = serde_json::to_vec(&vm_response_ok).unwrap();
    mock_vm_client.set_worker_execution_behavior(
        &policy_worker_id,
        MockExecutionBehavior::Fixed(vm_response_payload_ok.clone()),
    );

    let execute_req_ok = Request::new(ProtoExecutePolicyRequest {
        worker_id: policy_worker_id.clone(),
        contexts: vec![policy_req_ok.to_proto()],
    });

    let execute_resp_ok = runner_grpc
        .execute_policy(execute_req_ok)
        .await
        .expect("ExecutePolicy (ok) call failed");

    assert_eq!(
        execute_resp_ok.into_inner().satisfied_contexts.len(),
        1,
        "Policy should have been satisfied"
    );
    // Verify authorization WAS stored (indirectly, by trying PutSecrets next)
    info!("Step 4: Policy execution succeeded");

    // --- 5. PutSecret succeeds ---
    // Reuse the SecretsBox and EnvReport from step 3b
    let put_secrets_req_2 = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box_for_put.to_proto()), // Re-use the same box
            env_report: Some(putter_env_report.to_proto()),    // Re-use the same report
        }],
    });

    let put_secrets_resp_2 = secrets_grpc
        .put_secrets(put_secrets_req_2)
        .await
        .expect("PutSecrets (2) call failed");

    assert!(
        put_secrets_resp_2.into_inner().success,
        "PutSecrets should have succeeded now"
    );
    // Verify secret IS stored using CheckSecrets
    let check_req_2 = Request::new(nxcc_interface::proto::enclave::CheckSecretsRequest {
        ids: vec![secret_id.to_proto()],
    });
    let check_resp_2 = secrets_grpc
        .check_secrets(check_req_2)
        .await
        .expect("CheckSecrets (2) failed");
    let statuses_2 = check_resp_2.into_inner().statuses;
    assert_eq!(statuses_2.len(), 1);
    assert!(
        statuses_2[0].found,
        "Secret should be found after successful Put"
    );
    assert_eq!(
        statuses_2[0].expiry, secret_expiry,
        "Stored secret has wrong expiry"
    );
    info!("Step 5: PutSecret succeeded");

    // --- 6. Further PutSecret fails (NO auth consumption), GetSecret fails (no auth yet) ---
    // Attempt PutSecret again - should SUCCEED again because auth wasn't consumed.
    let put_secrets_req_3 = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box_for_put.to_proto()),
            env_report: Some(putter_env_report.to_proto()),
        }],
    });
    let put_secrets_resp_3 = secrets_grpc
        .put_secrets(put_secrets_req_3)
        .await
        .expect("PutSecrets (3) call failed");
    assert!(
        !put_secrets_resp_3.into_inner().success,
        "PutSecrets (3) should return success=false as secret already exists (even though \
         authorized)"
    );
    info!(
        "Step 6a: Further PutSecret processed (auth not consumed) but reported success=false as \
         secret exists"
    );

    // Attempt GetSecret - should fail as getter isn't authorized yet
    let getter_kx = KeyExchangeKeyPair::generate();
    let getter_env_report =
        test_requesting_env_report(getter_node_id, getter_kx.public_key().as_bytes());
    let get_secrets_req_fail = Request::new(ProtoGetSecretsRequest {
        requests: vec![SecretRequest {
            id: Some(secret_id.to_proto()),
        }],
        policy_reports: vec![], // Unused
        requester_env_report: Some(getter_env_report.to_proto()),
    });

    let get_secrets_resp_fail = secrets_grpc
        .get_secrets(get_secrets_req_fail)
        .await
        .expect("GetSecrets (fail) call failed unexpectedly"); // Call ok, but box empty

    let secrets_box_fail =
        SecretsBox::from_proto(get_secrets_resp_fail.into_inner().secrets_box.unwrap());
    assert!(
        secrets_box_fail.contained_secret_ids.is_empty(),
        "GetSecrets should return empty box when unauthorized"
    );
    info!("Step 6b: GetSecret failed (no authorization for getter)");

    // --- 7. Policy invoked again for GetSecret request ---
    let policy_req_get = test_policy_request(getter_node_id, vec![secret_id.clone()]); // Request for getter

    // Configure mock VM to return 'true' for this policy execution
    // Reuse the config from step 4 (returns true)
    mock_vm_client.set_worker_execution_behavior(
        &policy_worker_id,
        MockExecutionBehavior::Fixed(vm_response_payload_ok.clone()), // Ensure it's still set to true
    );

    let execute_req_get = Request::new(ProtoExecutePolicyRequest {
        worker_id: policy_worker_id.clone(),
        contexts: vec![policy_req_get.to_proto()],
    });

    let execute_resp_get = runner_grpc
        .execute_policy(execute_req_get)
        .await
        .expect("ExecutePolicy (get) call failed");

    assert_eq!(
        execute_resp_get.into_inner().satisfied_contexts.len(),
        1,
        "Policy for GetSecret should have been satisfied"
    );
    // Verify authorization WAS stored for the GETTER (indirectly via GetSecrets call)
    info!("Step 7: Policy execution succeeded for GetSecret request");

    // --- 8. GetSecret succeeds ---
    let get_secrets_req_ok = Request::new(ProtoGetSecretsRequest {
        requests: vec![SecretRequest {
            id: Some(secret_id.to_proto()),
        }],
        policy_reports: vec![],                                   // Unused
        requester_env_report: Some(getter_env_report.to_proto()), // Reuse getter's report
    });

    let get_secrets_resp_ok = secrets_grpc
        .get_secrets(get_secrets_req_ok)
        .await
        .expect("GetSecrets (ok) call failed");

    let secrets_box_ok_proto = get_secrets_resp_ok
        .into_inner()
        .secrets_box
        .expect("SecretsBox missing in successful GetSecrets response");
    let secrets_box_ok = SecretsBox::from_proto(secrets_box_ok_proto);

    assert_eq!(
        secrets_box_ok.contained_secret_ids.len(),
        1,
        "SecretsBox should contain one secret ID"
    );
    assert_eq!(
        secrets_box_ok.contained_secret_ids[0], secret_id,
        "SecretsBox contains wrong secret ID"
    );

    // Decrypt the box
    let decrypted_secrets = decrypt_secrets_box(&getter_kx, &secrets_box_ok)
        .expect("Failed to decrypt received SecretsBox");

    assert_eq!(
        decrypted_secrets.len(),
        1,
        "Decrypted secrets count mismatch"
    );
    assert_eq!(
        decrypted_secrets[0].0, secret_id,
        "Decrypted secret ID mismatch"
    );
    assert_eq!(
        decrypted_secrets[0].1, secret_data,
        "Decrypted secret data mismatch"
    );
    assert_eq!(
        decrypted_secrets[0].2, secret_expiry,
        "Decrypted secret expiry mismatch"
    );
    info!("Step 8: GetSecret succeeded and data verified");

    // --- 9. Further GetSecret fails (NO auth consumption) ---
    // Attempt GetSecret again - should SUCCEED again.
    let get_secrets_req_ok_2 = Request::new(ProtoGetSecretsRequest {
        requests: vec![SecretRequest {
            id: Some(secret_id.to_proto()),
        }],
        policy_reports: vec![],                                   // Unused
        requester_env_report: Some(getter_env_report.to_proto()), // Reuse getter's report
    });

    let get_secrets_resp_ok_2 = secrets_grpc
        .get_secrets(get_secrets_req_ok_2)
        .await
        .expect("GetSecrets (ok 2) call failed");

    let secrets_box_ok_proto_2 = get_secrets_resp_ok_2
        .into_inner()
        .secrets_box
        .expect("SecretsBox missing in successful GetSecrets response (2)");
    let secrets_box_ok_2 = SecretsBox::from_proto(secrets_box_ok_proto_2);

    assert_eq!(
        secrets_box_ok_2.contained_secret_ids.len(),
        1,
        "SecretsBox (2) should contain one secret ID as auth not consumed" // Changed assertion
    );
    // Decrypt again to be thorough
    let decrypted_secrets_2 = decrypt_secrets_box(&getter_kx, &secrets_box_ok_2)
        .expect("Failed to decrypt received SecretsBox (2)");
    assert_eq!(decrypted_secrets_2[0].1, secret_data);

    info!("Step 9: Further GetSecret succeeded (auth not consumed)");

    // --- Cleanup (Optional) ---
    // Can call TerminateWorker via gRPC if needed
    // Can call DetachVm via gRPC if needed
}

fn setup_services() -> (
    Arc<Secrets>,
    Arc<RunnerService>,
    MockVmServiceClient,
    SecretsGrpcService,
    EnclaveRunnerGrpcService,
) {
    let secrets_service = Secrets::new();
    let runner_service = Arc::new(RunnerService::new(secrets_service.clone()));
    let mock_vm_client = MockVmServiceClient::new();

    let secrets_grpc = SecretsGrpcService::new(secrets_service.clone());
    let runner_grpc = EnclaveRunnerGrpcService::new(runner_service.clone());

    (
        secrets_service,
        runner_service,
        mock_vm_client,
        secrets_grpc,
        runner_grpc,
    )
}

// --- Helper: Attach mock VM ---
async fn attach_mock_vm(
    runner_service: &RunnerService,
    vm_id: &str,
    mock_client: MockVmServiceClient,
) {
    runner_service
        .attach_mock_client(vm_id.to_string(), mock_client.clone())
        .await;
    info!("Test Setup: Attached mock VM '{}'", vm_id);
}

// --- Helper: Run a policy worker ---
async fn run_policy_worker(runner_grpc: &EnclaveRunnerGrpcService, vm_id: &str) -> String {
    let policy_worker_type_id = "policy-worker";
    let policy_worker_code = b"mock_policy_wasm".to_vec();
    let policy_manifest = b"{}".to_vec();
    let expected_policy_worker_instance_id = format!("instance-{}-1", policy_worker_type_id);

    let run_worker_req = Request::new(RunWorkerRequest {
        vm_id: vm_id.to_string(),
        worker_code: policy_worker_code.clone(),
        manifest: policy_manifest.clone(),
    });

    let run_worker_resp = runner_grpc
        .run_worker(run_worker_req)
        .await
        .expect("RunWorker call failed during setup");
    let run_worker_inner = run_worker_resp.into_inner();
    let policy_worker_id = run_worker_inner.worker_id;

    assert!(
        run_worker_inner.success,
        "RunWorker should succeed during setup"
    );
    assert_eq!(
        policy_worker_id, expected_policy_worker_instance_id,
        "Unexpected worker ID during setup"
    );
    info!(
        "Test Setup: Started policy worker '{}' in VM '{}'",
        policy_worker_id, vm_id
    );
    policy_worker_id
}

// --- Helper: Execute policy and assert success/failure ---
async fn execute_policy(
    runner_grpc: &EnclaveRunnerGrpcService,
    mock_vm_client: &MockVmServiceClient,
    worker_id: &str,
    node_id: &str,
    secret_id: &SecretId,
    should_succeed: bool,
) {
    let policy_req = test_policy_request(node_id, vec![secret_id.clone()]);

    // Configure mock VM
    let vm_response = vec![should_succeed];
    let vm_response_payload = serde_json::to_vec(&vm_response).unwrap();
    mock_vm_client.set_worker_execution_behavior(
        worker_id,
        MockExecutionBehavior::Fixed(vm_response_payload.clone()),
    );

    let execute_req = Request::new(ProtoExecutePolicyRequest {
        worker_id: worker_id.to_string(),
        contexts: vec![policy_req.to_proto()],
    });

    let execute_resp = runner_grpc
        .execute_policy(execute_req)
        .await
        .expect("ExecutePolicy call failed");

    let satisfied_count = execute_resp.into_inner().satisfied_contexts.len();
    if should_succeed {
        assert_eq!(satisfied_count, 1, "Policy should have been satisfied");
        info!(
            "Test Setup: Policy execution succeeded for node '{}', secret '{:?}'",
            node_id, secret_id
        );
    } else {
        assert_eq!(satisfied_count, 0, "Policy should NOT have been satisfied");
        info!(
            "Test Setup: Policy execution failed for node '{}', secret '{:?}'",
            node_id, secret_id
        );
    }
}

// --- Helper: Check secret presence ---
async fn check_secret_exists(secrets_grpc: &SecretsGrpcService, secret_id: &SecretId) -> bool {
    let check_req = Request::new(nxcc_interface::proto::enclave::CheckSecretsRequest {
        ids: vec![secret_id.to_proto()],
    });
    let check_resp = secrets_grpc
        .check_secrets(check_req)
        .await
        .expect("CheckSecrets failed");
    let statuses = check_resp.into_inner().statuses;
    assert_eq!(statuses.len(), 1);
    statuses[0].found
}

fn authorize_self_generation(secrets_service: &Secrets, secret_id: &SecretId) {
    let request = test_policy_request(crate::secrets::SELF_NODE_ID, vec![secret_id.clone()]);
    let report = PolicyExecutionReport {
        request,
        decision: true,
        timestamp: chrono::Utc::now().timestamp() as u64,
    };
    secrets_service.store_authorization(report);
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_put_secrets_mismatched_binding_hash() {
    let (secrets_service, runner_service, mock_vm_client, secrets_grpc, runner_grpc) =
        setup_services();
    let vm_id = "mock-vm-put-badhash";
    attach_mock_vm(&runner_service, vm_id, mock_vm_client.clone()).await;
    let policy_worker_id = run_policy_worker(&runner_grpc, vm_id).await;

    let secret_id = test_secret_id(2001);
    let secret_data = b"data for bad hash test".to_vec();
    let putter_node_id = "node-putter-badhash";

    // 1. Authorize the putter correctly
    execute_policy(
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        putter_node_id,
        &secret_id,
        true,
    )
    .await;

    // 2. Prepare SecretsBox and EnvReport
    let putter_kx = KeyExchangeKeyPair::generate();
    let enclave_pk_bytes = secrets_service
        .get_report(vec![])
        .unwrap()
        .ephemeral_public_key;
    let enclave_pk =
        x25519_dalek::PublicKey::from(<[u8; 32]>::try_from(enclave_pk_bytes.as_slice()).unwrap());

    let secrets_to_send = vec![(secret_id.clone(), secret_data.clone(), 0)];
    let secrets_box = encrypt_secrets_box(&putter_kx, &enclave_pk, &secrets_to_send).unwrap();

    // 3. Create EnvReport with an INCORRECT binding hash
    let correct_binding_hash = secrets_box.calculate_binding_hash();
    let mut incorrect_hash_vec = correct_binding_hash.to_vec();
    incorrect_hash_vec[0] ^= 0xff; // Tamper with the hash

    let putter_env_report = test_sending_env_report(
        putter_node_id,
        putter_kx.public_key().as_bytes(),
        incorrect_hash_vec, // Use the tampered hash
    );

    // 4. Attempt PutSecrets
    let put_secrets_req = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box.to_proto()),
            env_report: Some(putter_env_report.to_proto()),
        }],
    });

    let put_secrets_resp = secrets_grpc
        .put_secrets(put_secrets_req)
        .await
        .expect("PutSecrets call itself should not fail");

    // 5. Assert: PutSecrets should report failure (success=false) and secret not stored
    assert!(
        !put_secrets_resp.into_inner().success,
        "PutSecrets should fail due to hash mismatch"
    );
    assert!(
        !check_secret_exists(&secrets_grpc, &secret_id).await,
        "Secret should not be stored after hash mismatch"
    );
    info!("Test OK: PutSecrets rejected due to mismatched binding hash");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_put_secrets_invalid_secrets_box_structure() {
    let (secrets_service, runner_service, mock_vm_client, secrets_grpc, runner_grpc) =
        setup_services();
    let vm_id = "mock-vm-put-badbox";
    attach_mock_vm(&runner_service, vm_id, mock_vm_client.clone()).await;
    let policy_worker_id = run_policy_worker(&runner_grpc, vm_id).await;

    let secret_id = test_secret_id(2002);
    let putter_node_id = "node-putter-badbox";

    // 1. Authorize the putter correctly
    execute_policy(
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        putter_node_id,
        &secret_id,
        true,
    )
    .await;

    // 2. Create a malformed SecretsBox (e.g., invalid sender public key length)
    let bad_secrets_box = SecretsBox {
        encrypted_payload: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16], // Min length > nonce
        sender_public_key: vec![0; 31], // WRONG LENGTH
        alg: "X25519_AES-GCM-SIV".to_string(),
        contained_secret_ids: vec![secret_id.clone()],
    };

    // 3. Create a corresponding EnvReport (hash calculation doesn't matter as decryption will fail first)
    let putter_kx = KeyExchangeKeyPair::generate(); // Need a valid key *for the report*
    let binding_hash = bad_secrets_box.calculate_binding_hash(); // Hash of the bad box
    let putter_env_report = test_sending_env_report(
        putter_node_id,
        putter_kx.public_key().as_bytes(), // Use a valid key here for the report itself
        binding_hash.to_vec(),
    );

    // 4. Attempt PutSecrets
    let put_secrets_req = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(bad_secrets_box.to_proto()),
            env_report: Some(putter_env_report.to_proto()),
        }],
    });

    let put_secrets_resp = secrets_grpc
        .put_secrets(put_secrets_req)
        .await
        .expect("PutSecrets call itself should not fail");

    // 5. Assert: PutSecrets should report failure (success=false) and secret not stored
    // It fails during decryption due to the invalid key length before even checking the hash.
    assert!(
        !put_secrets_resp.into_inner().success,
        "PutSecrets should fail due to invalid SecretsBox structure"
    );
    assert!(
        !check_secret_exists(&secrets_grpc, &secret_id).await,
        "Secret should not be stored after invalid SecretsBox"
    );
    info!("Test OK: PutSecrets rejected due to invalid SecretsBox structure");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_put_secrets_decryption_failure() {
    let (secrets_service, runner_service, mock_vm_client, secrets_grpc, runner_grpc) =
        setup_services();
    let vm_id = "mock-vm-put-badcrypt";
    attach_mock_vm(&runner_service, vm_id, mock_vm_client.clone()).await;
    let policy_worker_id = run_policy_worker(&runner_grpc, vm_id).await;

    let secret_id = test_secret_id(2003);
    let secret_data = b"data for bad crypto test".to_vec();
    let putter_node_id = "node-putter-badcrypt";

    // 1. Authorize the putter correctly
    execute_policy(
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        putter_node_id,
        &secret_id,
        true,
    )
    .await;

    // 2. Prepare SecretsBox encrypted for the WRONG recipient
    let putter_kx = KeyExchangeKeyPair::generate();
    let wrong_recipient_kx = KeyExchangeKeyPair::generate(); // Generate a dummy recipient keypair
    let enclave_pk_bytes = secrets_service
        .get_report(vec![])
        .unwrap()
        .ephemeral_public_key; // Get real enclave key for report

    let secrets_to_send = vec![(secret_id.clone(), secret_data.clone(), 0)];
    let secrets_box_wrong_key = encrypt_secrets_box(
        &putter_kx,
        wrong_recipient_kx.public_key(), // Encrypt for wrong recipient
        &secrets_to_send,
    )
    .unwrap();

    // 3. Create EnvReport (hash must match the box, even if decryption fails later)
    let binding_hash = secrets_box_wrong_key.calculate_binding_hash();
    let putter_env_report = test_sending_env_report(
        putter_node_id,
        putter_kx.public_key().as_bytes(), // Putter's key in report attestation
        binding_hash.to_vec(),
    );

    // 4. Attempt PutSecrets
    let put_secrets_req = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box_wrong_key.to_proto()),
            env_report: Some(putter_env_report.to_proto()),
        }],
    });

    let put_secrets_resp = secrets_grpc
        .put_secrets(put_secrets_req)
        .await
        .expect("PutSecrets call itself should not fail");

    // 5. Assert: PutSecrets should report failure (success=false) and secret not stored
    // The binding hash check passes, but decryption fails because the enclave uses its key.
    assert!(
        !put_secrets_resp.into_inner().success,
        "PutSecrets should fail due to decryption error"
    );
    assert!(
        !check_secret_exists(&secrets_grpc, &secret_id).await,
        "Secret should not be stored after decryption error"
    );
    info!("Test OK: PutSecrets rejected due to decryption failure (wrong key)");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_get_secrets_unauthorized_node() {
    let (secrets_service, runner_service, mock_vm_client, secrets_grpc, runner_grpc) =
        setup_services();
    let vm_id = "mock-vm-get-unauth";
    attach_mock_vm(&runner_service, vm_id, mock_vm_client.clone()).await;
    let policy_worker_id = run_policy_worker(&runner_grpc, vm_id).await;

    let secret_id = test_secret_id(2004);
    let secret_data = b"secret for auth test".to_vec();
    let authorized_node_id = "node-getter-authorized";
    let unauthorized_node_id = "node-getter-unauthorized";

    // 1. Put a secret legitimately (requires authorizing a putter first)
    let putter_node_id = "node-putter-for-get-test";
    execute_policy(
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        putter_node_id,
        &secret_id,
        true,
    )
    .await;
    let putter_kx = KeyExchangeKeyPair::generate();
    let enclave_pk_bytes = secrets_service
        .get_report(vec![])
        .unwrap()
        .ephemeral_public_key;
    let enclave_pk =
        x25519_dalek::PublicKey::from(<[u8; 32]>::try_from(enclave_pk_bytes.as_slice()).unwrap());
    let secrets_to_send = vec![(secret_id.clone(), secret_data.clone(), 0)];
    let secrets_box_put = encrypt_secrets_box(&putter_kx, &enclave_pk, &secrets_to_send).unwrap();
    let binding_hash_put = secrets_box_put.calculate_binding_hash();
    let putter_env_report = test_sending_env_report(
        putter_node_id,
        putter_kx.public_key().as_bytes(),
        binding_hash_put.to_vec(),
    );
    let put_req = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box_put.to_proto()),
            env_report: Some(putter_env_report.to_proto()),
        }],
    });
    let put_resp = secrets_grpc.put_secrets(put_req).await.unwrap();
    assert!(put_resp.into_inner().success);
    assert!(check_secret_exists(&secrets_grpc, &secret_id).await);
    info!("Test Setup: Secret put successfully");

    // 2. Authorize the *authorized* node to get the secret
    execute_policy(
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        authorized_node_id,
        &secret_id,
        true,
    )
    .await;

    // 3. Attempt GetSecrets from the *unauthorized* node
    let unauthorized_getter_kx = KeyExchangeKeyPair::generate();
    let unauthorized_env_report = test_requesting_env_report(
        unauthorized_node_id,
        unauthorized_getter_kx.public_key().as_bytes(),
    );

    let get_secrets_req = Request::new(ProtoGetSecretsRequest {
        requests: vec![SecretRequest {
            id: Some(secret_id.to_proto()),
        }],
        policy_reports: vec![], // Unused
        requester_env_report: Some(unauthorized_env_report.to_proto()),
    });

    let get_secrets_resp = secrets_grpc
        .get_secrets(get_secrets_req)
        .await
        .expect("GetSecrets call itself should not fail");

    // 4. Assert: GetSecrets should return an empty box
    let secrets_box_resp =
        SecretsBox::from_proto(get_secrets_resp.into_inner().secrets_box.unwrap());
    assert!(
        secrets_box_resp.contained_secret_ids.is_empty(),
        "GetSecrets should return empty box for unauthorized node"
    );
    info!("Test OK: GetSecrets returned empty box for unauthorized node");

    // 5. Verify the authorized node *can* get it (sanity check)
    let authorized_getter_kx = KeyExchangeKeyPair::generate();
    let authorized_env_report = test_requesting_env_report(
        authorized_node_id,
        authorized_getter_kx.public_key().as_bytes(),
    );
    let get_secrets_req_ok = Request::new(ProtoGetSecretsRequest {
        requests: vec![SecretRequest {
            id: Some(secret_id.to_proto()),
        }],
        policy_reports: vec![], // Unused
        requester_env_report: Some(authorized_env_report.to_proto()),
    });
    let get_secrets_resp_ok = secrets_grpc.get_secrets(get_secrets_req_ok).await.unwrap();
    let secrets_box_ok =
        SecretsBox::from_proto(get_secrets_resp_ok.into_inner().secrets_box.unwrap());
    assert_eq!(secrets_box_ok.contained_secret_ids.len(), 1);
    info!("Test OK: Authorized node successfully retrieved secret");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_get_secrets_invalid_requester_report() {
    let (_secrets_service, runner_service, mock_vm_client, secrets_grpc, runner_grpc) =
        setup_services();
    let vm_id = "mock-vm-get-badreport";
    attach_mock_vm(&runner_service, vm_id, mock_vm_client.clone()).await;
    // No need to run worker or put secrets, as the report validation happens first

    let secret_id = test_secret_id(2005);
    let getter_node_id = "node-getter-badreport";

    // 1. Create a malformed EnvReport (invalid public key length)
    let mut bad_env_report = test_requesting_env_report(getter_node_id, &[0; 32]); // Start with valid structure
    bad_env_report.attestation.ephemeral_public_key = vec![0; 31]; // Make key length invalid

    // 2. Attempt GetSecrets
    let get_secrets_req = Request::new(ProtoGetSecretsRequest {
        requests: vec![SecretRequest {
            id: Some(secret_id.to_proto()),
        }],
        policy_reports: vec![], // Unused
        requester_env_report: Some(bad_env_report.to_proto()),
    });

    let result = secrets_grpc.get_secrets(get_secrets_req).await;

    // 3. Assert: GetSecrets should fail with an Internal or InvalidArgument error
    assert!(
        result.is_err(),
        "GetSecrets should fail with invalid report"
    );
    let status = result.err().unwrap();
    // The exact error might depend on where the validation happens (attestation or key extraction)
    // Expecting Internal because the placeholder attestation passes, but key extraction fails.
    assert_eq!(
        status.code(),
        Code::Internal,
        "Expected Internal error status for bad key length post-attestation"
    );
    assert!(
        status
            .message()
            .contains("Invalid ephemeral public key length"),
        "Error message mismatch"
    );

    info!("Test OK: GetSecrets failed due to invalid requester EnvReport");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_runner_ops_non_existent_entities() {
    let (_secrets_service, runner_service, mock_vm_client, _secrets_grpc, runner_grpc) =
        setup_services();
    let vm_id = "mock-vm-ops-exist";
    let worker_id = "worker-ops-exist";
    let non_existent_vm_id = "vm-does-not-exist";
    let non_existent_worker_id = "worker-does-not-exist";

    // 1. Attach a real VM ID
    attach_mock_vm(&runner_service, vm_id, mock_vm_client.clone()).await;
    let real_worker_id = run_policy_worker(&runner_grpc, vm_id).await;

    // 2. Test operations targeting non-existent VM
    let run_worker_req_bad_vm = Request::new(RunWorkerRequest {
        vm_id: non_existent_vm_id.to_string(),
        worker_code: vec![],
        manifest: vec![],
    });
    let result_run_bad_vm = runner_grpc.run_worker(run_worker_req_bad_vm).await;
    assert!(result_run_bad_vm.is_err());
    assert_eq!(
        result_run_bad_vm.err().unwrap().code(),
        Code::FailedPrecondition,
        "RunWorker on non-existent VM"
    );

    let detach_req_bad_vm = Request::new(DetachVmRequest {
        vm_id: non_existent_vm_id.to_string(),
    });
    // Detach is idempotent, should succeed even if VM not found
    let result_detach_bad_vm = runner_grpc.detach_vm(detach_req_bad_vm).await;
    assert!(
        result_detach_bad_vm.is_ok(),
        "DetachVm on non-existent VM should be Ok"
    );

    // 3. Test operations targeting non-existent Worker
    let terminate_req_bad_worker = Request::new(TerminateWorkerRequest {
        worker_id: non_existent_worker_id.to_string(),
    });
    // Terminate is idempotent for not found workers in the runner service logic
    let result_term_bad_worker = runner_grpc.terminate_worker(terminate_req_bad_worker).await;
    assert!(
        result_term_bad_worker.is_ok(),
        "TerminateWorker on non-existent worker should be Ok"
    );

    let invoke_req_bad_worker = Request::new(InvokeWorkerRequest {
        worker_id: non_existent_worker_id.to_string(),
        payload: vec![],
    });
    let result_invoke_bad_worker = runner_grpc.invoke_worker(invoke_req_bad_worker).await;
    assert!(result_invoke_bad_worker.is_err());
    assert_eq!(
        result_invoke_bad_worker.err().unwrap().code(),
        Code::NotFound,
        "InvokeWorker on non-existent worker"
    );

    let exec_req_bad_worker = Request::new(ProtoExecutePolicyRequest {
        worker_id: non_existent_worker_id.to_string(),
        contexts: vec![],
    });
    let result_exec_bad_worker = runner_grpc.execute_policy(exec_req_bad_worker).await;
    assert!(result_exec_bad_worker.is_err());
    assert_eq!(
        result_exec_bad_worker.err().unwrap().code(),
        Code::NotFound,
        "ExecutePolicy on non-existent worker"
    );

    info!("Test OK: Runner operations correctly handled non-existent VMs/workers");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_generate_secrets_workflow() {
    let (secrets_service, runner_service, mock_vm_client, secrets_grpc, runner_grpc) =
        setup_services();
    let vm_id = "mock-vm-generate";
    attach_mock_vm(&runner_service, vm_id, mock_vm_client.clone()).await;
    let policy_worker_id = run_policy_worker(&runner_grpc, vm_id).await;

    let secret_id_gen = test_secret_id(3001);
    let getter_node_id = "node-getter-for-gen";

    // 1. Attempt GenerateSecrets without authorization -> Fails (or does nothing)
    let gen_req_unauth = Request::new(GenerateSecretsRequest {
        ids: vec![secret_id_gen.to_proto()],
    });
    let gen_resp_unauth = secrets_grpc.generate_secrets(gen_req_unauth).await;
    // Depending on implementation (error vs skip), check status or secret existence
    // Current implementation skips, so the call succeeds but secret isn't created.
    assert!(
        gen_resp_unauth.is_ok(),
        "GenerateSecrets call should succeed even if unauthorized (skips)"
    );
    assert!(
        !check_secret_exists(&secrets_grpc, &secret_id_gen).await,
        "Secret should not exist after unauthorized GenerateSecrets"
    );
    info!("Test OK: GenerateSecrets skipped unauthorized request");

    // 2. Authorize self-generation (simulate runner authorizing enclave itself)
    // This requires executing the policy worker for the special SELF_NODE_ID
    execute_policy(
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        crate::secrets::SELF_NODE_ID, // Use the special ID
        &secret_id_gen,
        true, // Assume policy allows self-generation
    )
    .await;
    info!(
        "Test Setup: Authorized self-generation for {:?}",
        secret_id_gen
    );

    // 3. GenerateSecrets successfully
    let gen_req_auth = Request::new(GenerateSecretsRequest {
        ids: vec![secret_id_gen.to_proto()],
    });
    let gen_resp_auth = secrets_grpc.generate_secrets(gen_req_auth).await;
    assert!(
        gen_resp_auth.is_ok(),
        "GenerateSecrets failed: {:?}",
        gen_resp_auth.err()
    );
    assert!(
        check_secret_exists(&secrets_grpc, &secret_id_gen).await,
        "Secret should exist after authorized GenerateSecrets"
    );
    info!("Test OK: GenerateSecrets succeeded");

    // 4. Attempt GenerateSecrets again for the same ID -> Fails (AlreadyExists)
    let gen_req_dup = Request::new(GenerateSecretsRequest {
        ids: vec![secret_id_gen.to_proto()],
    });
    let gen_resp_dup = secrets_grpc.generate_secrets(gen_req_dup).await;
    assert!(gen_resp_dup.is_err());
    assert_eq!(gen_resp_dup.err().unwrap().code(), Code::AlreadyExists);
    info!("Test OK: GenerateSecrets failed for duplicate ID");

    // 5. Attempt PutSecrets for the generated secret -> Fails (Existing is canonical)
    let putter_node_id = "node-putter-for-gen";
    execute_policy(
        // Authorize putter
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        putter_node_id,
        &secret_id_gen,
        true,
    )
    .await;
    let putter_kx = KeyExchangeKeyPair::generate();
    let enclave_pk_bytes = secrets_service
        .get_report(vec![])
        .unwrap()
        .ephemeral_public_key;
    let enclave_pk =
        x25519_dalek::PublicKey::from(<[u8; 32]>::try_from(enclave_pk_bytes.as_slice()).unwrap());
    let secrets_to_send = vec![(secret_id_gen.clone(), b"overwrite attempt".to_vec(), 0)];
    let secrets_box_put = encrypt_secrets_box(&putter_kx, &enclave_pk, &secrets_to_send).unwrap();
    let binding_hash_put = secrets_box_put.calculate_binding_hash();
    let putter_env_report = test_sending_env_report(
        putter_node_id,
        putter_kx.public_key().as_bytes(),
        binding_hash_put.to_vec(),
    );
    let put_req = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box_put.to_proto()),
            env_report: Some(putter_env_report.to_proto()),
        }],
    });
    let put_resp = secrets_grpc.put_secrets(put_req).await.unwrap();
    // PutSecrets should report success=false because the existing secret was not overwritten
    assert!(
        !put_resp.into_inner().success,
        "PutSecrets should not overwrite generated secret"
    );
    info!("Test OK: PutSecrets did not overwrite generated secret");

    // 6. Authorize getter and GetSecrets
    execute_policy(
        // Authorize getter
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        getter_node_id,
        &secret_id_gen,
        true,
    )
    .await;
    let getter_kx = KeyExchangeKeyPair::generate();
    let getter_env_report =
        test_requesting_env_report(getter_node_id, getter_kx.public_key().as_bytes());
    let get_req = Request::new(ProtoGetSecretsRequest {
        requests: vec![SecretRequest {
            id: Some(secret_id_gen.to_proto()),
        }],
        policy_reports: vec![],
        requester_env_report: Some(getter_env_report.to_proto()),
    });
    let get_resp = secrets_grpc.get_secrets(get_req).await.unwrap();
    let secrets_box_get = SecretsBox::from_proto(get_resp.into_inner().secrets_box.unwrap());
    assert_eq!(secrets_box_get.contained_secret_ids.len(), 1);
    assert_eq!(secrets_box_get.contained_secret_ids[0], secret_id_gen);

    // 7. Decrypt and verify data length (we don't know the exact generated data)
    let decrypted_secrets = decrypt_secrets_box(&getter_kx, &secrets_box_get).unwrap();
    assert_eq!(decrypted_secrets.len(), 1);
    assert_eq!(decrypted_secrets[0].0, secret_id_gen);
    assert_eq!(
        decrypted_secrets[0].1.len(),
        32,
        "Generated secret data has wrong length"
    );
    // Ensure it wasn't overwritten by the PutSecrets attempt
    assert_ne!(decrypted_secrets[0].1, b"overwrite attempt".to_vec());
    assert_eq!(
        decrypted_secrets[0].2, 0,
        "Generated secret expiry should be 0"
    ); // Default expiry
    info!("Test OK: GetSecrets retrieved generated secret successfully");
}
