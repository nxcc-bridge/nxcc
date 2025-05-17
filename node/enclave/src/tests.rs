use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use alloy_primitives::{Address, U256};
use nxcc_interface::{
    proto::enclave::{
        DetachVmRequest, ExecutePolicyRequest as ProtoExecutePolicyRequest, GenerateSecretsRequest,
        GetSecretsRequest as ProtoGetSecretsRequest, InvokeWorkerRequest,
        PutSecretsRequest as ProtoPutSecretsRequest, RunWorkerRequest, SecretRequest,
        SecretsBundle, TerminateWorkerRequest, runner_server::Runner as _,
        secrets_server::Secrets as _,
    },
    types::{
        AttestationReport, ConsumerInfo, EnvReport, PolicyExecutionReport,
        PolicyExecutionRequest, SecretId, SecretsBox,
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

// Helper to create an EnvReport with a specific ephemeral public key and user_data.
// This is crucial for ensuring consistency between policy execution context and actual operation context.
fn test_env_report_for_client(
    node_id: &str,
    client_kx_public_key: &[u8],
    user_data_for_attestation: Vec<u8>, // For PutSecrets, this is the binding hash. For GetSecrets, can be anything.
) -> EnvReport {
    EnvReport {
        attestation: AttestationReport {
            measurement: vec![0u8; 32], // Consistent measurement for tests
            ephemeral_public_key: client_kx_public_key.to_vec(),
            block_hashes: vec![vec![1, 2]], // Consistent block_hashes
            user_data: user_data_for_attestation,
        },
        operator_signature: vec![3; 64], // Consistent operator_signature
        node_id: node_id.to_string(),
    }
}

// --- The Integration Test (Refactored) ---

#[tokio::test]
#[tracing_test::traced_test]
async fn test_enclave_workflow() {
    let (secrets_service, runner_service, mock_vm_client, secrets_grpc, runner_grpc) =
        setup_services();

    let vm_id = "mock-vm-01";
    attach_mock_vm(&runner_service, vm_id, mock_vm_client.clone()).await;
    let policy_worker_id = run_policy_worker(&runner_grpc, &mock_vm_client, vm_id).await;

    let secret_id = test_secret_id(12345);
    let secret_data = b"this is the secret data".to_vec();
    let secret_expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;

    let putter_node_id = "node-putter";
    let getter_node_id = "node-getter";

    // --- Putter's identity and secret box preparation ---
    let putter_kx = KeyExchangeKeyPair::generate();
    let enclave_report_for_putter = secrets_service.get_report(vec![]).unwrap(); // Putter gets enclave's pubkey
    let enclave_pk_for_putter = x25519_dalek::PublicKey::from(
        <[u8; 32]>::try_from(enclave_report_for_putter.ephemeral_public_key.as_slice()).unwrap(),
    );
    let secrets_to_send = vec![(secret_id.clone(), secret_data.clone(), secret_expiry, 1)];
    let secrets_box_for_put =
        encrypt_secrets_box(&putter_kx, &enclave_pk_for_putter, &secrets_to_send).unwrap();
    let binding_hash_for_put = secrets_box_for_put.calculate_binding_hash();

    // Putter's EnvReport, which will be used for policy and for PutSecrets
    let putter_env_report = test_env_report_for_client(
        putter_node_id,
        putter_kx.public_key().as_bytes(),
        binding_hash_for_put.to_vec(),
    );

    // --- 3. Policy execution fails (wrong context), PutSecret rejected ---
    info!("Step 3a: Attempting policy execution (expected fail)");
    execute_policy_with_env_report(
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        putter_env_report.clone(), // Putter presents its EnvReport
        vec![secret_id.clone()],
        false, // Expect policy to fail
    )
    .await;

    let put_secrets_req_fail = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box_for_put.clone().into()),
            env_report: Some(putter_env_report.clone().into()), // Putter uses its EnvReport
        }],
    });
    let put_secrets_resp_fail = secrets_grpc
        .put_secrets(put_secrets_req_fail)
        .await
        .unwrap();
    assert!(
        !put_secrets_resp_fail.into_inner().success,
        "PutSecrets should have been rejected"
    );
    assert!(!check_secret_exists(&secrets_grpc, &secret_id).await);
    info!("Step 3b: PutSecret rejected as expected");

    // --- 4. Policy execution succeeds ---
    info!("Step 4: Attempting policy execution (expected success)");
    execute_policy_with_env_report(
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        putter_env_report.clone(), // Putter presents its EnvReport again
        vec![secret_id.clone()],
        true, // Expect policy to succeed
    )
    .await;

    // --- 5. PutSecret succeeds ---
    let put_secrets_req_ok = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box_for_put.clone().into()),
            env_report: Some(putter_env_report.clone().into()), // Putter uses its EnvReport
        }],
    });
    let put_secrets_resp_ok = secrets_grpc.put_secrets(put_secrets_req_ok).await.unwrap();
    assert!(
        put_secrets_resp_ok.into_inner().success,
        "PutSecrets should have succeeded now"
    );
    let status_after_put = get_secret_status(&secrets_grpc, &secret_id).await.unwrap();
    assert!(
        status_after_put.0,
        "Secret should be found after successful Put"
    );
    assert_eq!(
        status_after_put.1, secret_expiry,
        "Stored secret has wrong expiry"
    );
    info!("Step 5: PutSecret succeeded");

    // --- Getter's identity preparation ---
    let getter_kx = KeyExchangeKeyPair::generate();
    // For GetSecrets, user_data in getter's attestation can be arbitrary.
    let getter_env_report = test_env_report_for_client(
        getter_node_id,
        getter_kx.public_key().as_bytes(),
        vec![0u8; 32], // Arbitrary user_data for getter's report
    );

    // --- 6. GetSecret fails (no auth yet for getter) ---
    info!("Step 6b: Attempting GetSecret (expected fail - no auth for getter)");
    let get_secrets_req_fail = Request::new(ProtoGetSecretsRequest {
        requests: vec![SecretRequest {
            id: Some(secret_id.clone().into()),
        }],
        policy_reports: vec![],
        requester_env_report: Some(getter_env_report.clone().into()), // Getter uses its EnvReport
    });
    let get_secrets_resp_fail = secrets_grpc
        .get_secrets(get_secrets_req_fail)
        .await
        .unwrap();
    let secrets_box_fail =
        SecretsBox::from(get_secrets_resp_fail.into_inner().secrets_box.unwrap());
    assert!(
        secrets_box_fail.contained_secret_ids.is_empty(),
        "GetSecrets should return empty box"
    );
    info!("Step 6b: GetSecret failed (no authorization for getter)");

    // --- 7. Policy invoked again for GetSecret request (for getter) ---
    info!("Step 7: Attempting policy execution for GetSecret (expected success)");
    execute_policy_with_env_report(
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        getter_env_report.clone(), // Getter presents its EnvReport
        vec![secret_id.clone()],
        true, // Expect policy to succeed for getter
    )
    .await;

    // --- 8. GetSecret succeeds ---
    info!("Step 8: Attempting GetSecret (expected success)");
    let get_secrets_req_ok = Request::new(ProtoGetSecretsRequest {
        requests: vec![SecretRequest {
            id: Some(secret_id.clone().into()),
        }],
        policy_reports: vec![],
        requester_env_report: Some(getter_env_report.clone().into()), // Getter uses its EnvReport
    });
    let get_secrets_resp_ok = secrets_grpc.get_secrets(get_secrets_req_ok).await.unwrap();
    let secrets_box_ok =
        SecretsBox::from(get_secrets_resp_ok.into_inner().secrets_box.unwrap());
    assert_eq!(secrets_box_ok.contained_secret_ids.len(), 1);
    assert_eq!(secrets_box_ok.contained_secret_ids[0], secret_id);

    let decrypted_secrets = decrypt_secrets_box(&getter_kx, &secrets_box_ok).unwrap();
    assert_eq!(decrypted_secrets.len(), 1);
    assert_eq!(decrypted_secrets[0].0, secret_id);
    assert_eq!(decrypted_secrets[0].1, secret_data);
    assert_eq!(decrypted_secrets[0].2, secret_expiry);
    assert_eq!(decrypted_secrets[0].3, 1);
    info!("Step 8: GetSecret succeeded and data verified");

    // --- 9. Further GetSecret succeeds (auth not consumed by default) ---
    info!("Step 9: Attempting further GetSecret (expected success - auth not consumed)");
    let get_secrets_req_ok_2 = Request::new(ProtoGetSecretsRequest {
        requests: vec![SecretRequest {
            id: Some(secret_id.clone().into()),
        }],
        policy_reports: vec![],
        requester_env_report: Some(getter_env_report.clone().into()), // Getter uses its EnvReport
    });
    let get_secrets_resp_ok_2 = secrets_grpc
        .get_secrets(get_secrets_req_ok_2)
        .await
        .unwrap();
    let secrets_box_ok_2 =
        SecretsBox::from(get_secrets_resp_ok_2.into_inner().secrets_box.unwrap());
    assert_eq!(secrets_box_ok_2.contained_secret_ids.len(), 1);
    info!("Step 9: Further GetSecret succeeded");
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

async fn attach_mock_vm(
    runner_service: &RunnerService,
    vm_id: &str,
    mock_client: MockVmServiceClient,
) {
    runner_service
        .attach_mock_client(vm_id.to_string(), mock_client)
        .await;
    info!("Test Setup: Attached mock VM '{}'", vm_id);
}

async fn run_policy_worker(
    runner_grpc: &EnclaveRunnerGrpcService,
    mock_vm_client: &MockVmServiceClient, // Added to configure if needed, though not for basic run
    vm_id: &str,
) -> String {
    let policy_worker_type_id = "policy-worker";
    let policy_worker_code = b"mock_policy_wasm".to_vec();
    let policy_manifest = b"{}".to_vec();
    let expected_policy_worker_instance_id = format!("instance-{}-1", policy_worker_type_id);

    // Ensure mock VM is configured to succeed start_worker (default behavior of MockVmServiceClient)
    // If specific behavior is needed for start_worker, configure mock_vm_client here.

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

async fn execute_policy_with_env_report(
    runner_grpc: &EnclaveRunnerGrpcService,
    mock_vm_client: &MockVmServiceClient,
    worker_id: &str,
    client_env_report: EnvReport,
    secret_ids: Vec<SecretId>,
    should_succeed: bool,
) {
    let policy_req_internal = PolicyExecutionRequest {
        secret_ids: secret_ids.clone(),
        consumer: test_consumer_info(),
        env_report: client_env_report.clone(),
    };

    let vm_response: Vec<bool> = if should_succeed {
        vec![true; std::cmp::max(1, secret_ids.len())] // Policy worker might expect at least one result even for empty secret_ids
    } else {
        vec![false; std::cmp::max(1, secret_ids.len())]
    };
    // If secret_ids is empty, the policy might be a general check.
    // The mock VM should return a result array whose length matches what the policy worker outputs.
    // For simplicity, if secret_ids is empty and should_succeed, we assume a single 'true' result.
    // If secret_ids is empty and !should_succeed, a single 'false'.
    // If secret_ids is not empty, length of vm_response matches secret_ids.len().
    let num_contexts_for_vm = if secret_ids.is_empty() {
        1
    } else {
        secret_ids.len()
    };
    let vm_response_for_mock: Vec<bool> = if should_succeed {
        vec![true; num_contexts_for_vm]
    } else {
        vec![false; num_contexts_for_vm]
    };

    let vm_response_payload = serde_json::to_vec(&vm_response_for_mock).unwrap();
    mock_vm_client.set_worker_execution_behavior(
        worker_id,
        MockExecutionBehavior::Fixed(vm_response_payload.clone()),
    );

    let execute_req = Request::new(ProtoExecutePolicyRequest {
        worker_id: worker_id.to_string(),
        contexts: vec![policy_req_internal.into()],
    });

    let execute_resp = runner_grpc
        .execute_policy(execute_req)
        .await
        .expect("ExecutePolicy call failed");
    let satisfied_contexts_proto = execute_resp.into_inner().satisfied_contexts;

    let expected_satisfied_count = if should_succeed && !secret_ids.is_empty() {
        secret_ids.len()
    } else if should_succeed && secret_ids.is_empty() {
        // Policy for no specific secret_ids, but general approval for the client_env_report
        1 // Assuming the policy worker returns one satisfied context in this case
    } else {
        0
    };

    // The number of satisfied contexts returned by ExecutePolicy gRPC should match the number of *input* contexts that were satisfied.
    // Our helper currently sends one PolicyExecutionRequest (which becomes one context for the worker).
    // If that one context is satisfied, satisfied_contexts_proto.len() will be 1.
    let expected_satisfied_contexts_len = if should_succeed { 1 } else { 0 };

    assert_eq!(
        satisfied_contexts_proto.len(),
        expected_satisfied_contexts_len,
        "Policy satisfaction count mismatch"
    );

    if should_succeed {
        info!(
            "Test Setup: Policy execution succeeded for node '{}', secrets '{:?}'",
            client_env_report.node_id, secret_ids
        );
    } else {
        info!(
            "Test Setup: Policy execution failed for node '{}', secrets '{:?}'",
            client_env_report.node_id, secret_ids
        );
    }
}

async fn check_secret_exists(secrets_grpc: &SecretsGrpcService, secret_id: &SecretId) -> bool {
    get_secret_status(secrets_grpc, secret_id)
        .await
        .map_or(false, |s| s.0)
}

async fn get_secret_status(
    secrets_grpc: &SecretsGrpcService,
    secret_id: &SecretId,
) -> Option<(bool, u64)> {
    let check_req = Request::new(nxcc_interface::proto::enclave::CheckSecretsRequest {
        ids: vec![secret_id.clone().into()],
    });
    let check_resp = secrets_grpc
        .check_secrets(check_req)
        .await
        .expect("CheckSecrets failed");
    let statuses = check_resp.into_inner().statuses;
    if statuses.len() == 1 {
        Some((statuses[0].found, statuses[0].expiry))
    } else {
        None
    }
}

async fn authorize_self_generation(secrets_service: &Secrets, secret_id: &SecretId) {
    let self_attestation = secrets_service
        .get_report(vec![])
        .expect("Failed to get self-report for auth");
    let self_env_report = EnvReport {
        attestation: self_attestation,
        operator_signature: vec![], // Not relevant for self-auth policy
        node_id: "self-enclave".to_string(), // Identifier for logging/policy
    };
    let request = PolicyExecutionRequest {
        secret_ids: vec![secret_id.clone()],
        consumer: test_consumer_info(), // Default consumer info
        env_report: self_env_report,
    };
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
    let policy_worker_id = run_policy_worker(&runner_grpc, &mock_vm_client, vm_id).await;

    let secret_id = test_secret_id(2001);
    let putter_node_id = "node-putter-badhash";
    let putter_kx = KeyExchangeKeyPair::generate();
    let enclave_pk_bytes = secrets_service
        .get_report(vec![])
        .unwrap()
        .ephemeral_public_key;
    let enclave_pk =
        x25519_dalek::PublicKey::from(<[u8; 32]>::try_from(enclave_pk_bytes.as_slice()).unwrap());
    let secrets_to_send = vec![(secret_id.clone(), b"data".to_vec(), 0, 1)];
    let secrets_box = encrypt_secrets_box(&putter_kx, &enclave_pk, &secrets_to_send).unwrap();

    let correct_binding_hash = secrets_box.calculate_binding_hash();
    let mut incorrect_hash_vec = correct_binding_hash.to_vec();
    incorrect_hash_vec[0] ^= 0xff; // Tamper with the hash

    // EnvReport with the INCORRECT binding hash (this is what the putter will present)
    let putter_env_report_bad_hash = test_env_report_for_client(
        putter_node_id,
        putter_kx.public_key().as_bytes(),
        incorrect_hash_vec.clone(), // Use the tampered hash
    );

    // Authorize the putter based on the EnvReport it WILL present (even if it's "bad" for binding)
    // This ensures the authorization check passes, so the binding hash check is actually reached.
    execute_policy_with_env_report(
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        putter_env_report_bad_hash.clone(),
        vec![secret_id.clone()],
        true,
    )
    .await;

    let put_secrets_req = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box.into()),
            env_report: Some(putter_env_report_bad_hash.into()),
        }],
    });
    let put_secrets_resp = secrets_grpc.put_secrets(put_secrets_req).await.unwrap();
    assert!(
        !put_secrets_resp.into_inner().success,
        "PutSecrets should fail due to hash mismatch"
    );
    assert!(!check_secret_exists(&secrets_grpc, &secret_id).await);
    info!("Test OK: PutSecrets rejected due to mismatched binding hash");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_put_secrets_invalid_secrets_box_structure() {
    let (secrets_service, runner_service, mock_vm_client, secrets_grpc, runner_grpc) =
        setup_services();
    let vm_id = "mock-vm-put-badbox";
    attach_mock_vm(&runner_service, vm_id, mock_vm_client.clone()).await;
    let policy_worker_id = run_policy_worker(&runner_grpc, &mock_vm_client, vm_id).await;

    let secret_id = test_secret_id(2002);
    let putter_node_id = "node-putter-badbox";
    let putter_kx = KeyExchangeKeyPair::generate(); // For the EnvReport

    let bad_secrets_box = SecretsBox {
        encrypted_payload: vec![1; 16], // Min length > nonce
        sender_public_key: vec![0; 31], // WRONG LENGTH
        alg: "X25519_AES-GCM-SIV".to_string(),
        contained_secret_ids: vec![secret_id.clone()],
    };
    let binding_hash_bad_box = bad_secrets_box.calculate_binding_hash();

    let putter_env_report_for_bad_box = test_env_report_for_client(
        putter_node_id,
        putter_kx.public_key().as_bytes(),
        binding_hash_bad_box.to_vec(),
    );

    execute_policy_with_env_report(
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        putter_env_report_for_bad_box.clone(),
        vec![secret_id.clone()],
        true,
    )
    .await;

    let put_secrets_req = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(bad_secrets_box.clone().into()),
            env_report: Some(putter_env_report_for_bad_box.into()),
        }],
    });
    let put_secrets_resp = secrets_grpc.put_secrets(put_secrets_req).await.unwrap();
    assert!(
        !put_secrets_resp.into_inner().success,
        "PutSecrets should fail due to invalid SecretsBox"
    );
    assert!(!check_secret_exists(&secrets_grpc, &secret_id).await);
    info!("Test OK: PutSecrets rejected due to invalid SecretsBox structure");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_put_secrets_decryption_failure() {
    let (secrets_service, runner_service, mock_vm_client, secrets_grpc, runner_grpc) =
        setup_services();
    let vm_id = "mock-vm-put-badcrypt";
    attach_mock_vm(&runner_service, vm_id, mock_vm_client.clone()).await;
    let policy_worker_id = run_policy_worker(&runner_grpc, &mock_vm_client, vm_id).await;

    let secret_id = test_secret_id(2003);
    let putter_node_id = "node-putter-badcrypt";
    let putter_kx = KeyExchangeKeyPair::generate();
    let wrong_recipient_kx = KeyExchangeKeyPair::generate(); // Encrypt for this wrong key

    let secrets_to_send = vec![(secret_id.clone(), b"data".to_vec(), 0, 1)];
    let secrets_box_wrong_key = encrypt_secrets_box(
        &putter_kx,
        wrong_recipient_kx.public_key(), // Encrypt for wrong recipient
        &secrets_to_send,
    )
    .unwrap();
    let binding_hash_wrong_key_box = secrets_box_wrong_key.calculate_binding_hash();

    let putter_env_report_for_wrong_key_box = test_env_report_for_client(
        putter_node_id,
        putter_kx.public_key().as_bytes(),
        binding_hash_wrong_key_box.to_vec(),
    );

    execute_policy_with_env_report(
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        putter_env_report_for_wrong_key_box.clone(),
        vec![secret_id.clone()],
        true,
    )
    .await;

    let put_secrets_req = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box_wrong_key.clone().into()),
            env_report: Some(putter_env_report_for_wrong_key_box.into()),
        }],
    });
    let put_secrets_resp = secrets_grpc.put_secrets(put_secrets_req).await.unwrap();
    assert!(
        !put_secrets_resp.into_inner().success,
        "PutSecrets should fail due to decryption error"
    );
    assert!(!check_secret_exists(&secrets_grpc, &secret_id).await);
    info!("Test OK: PutSecrets rejected due to decryption failure (wrong key)");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_get_secrets_unauthorized_node() {
    let (secrets_service, runner_service, mock_vm_client, secrets_grpc, runner_grpc) =
        setup_services();
    let vm_id = "mock-vm-get-unauth";
    attach_mock_vm(&runner_service, vm_id, mock_vm_client.clone()).await;
    let policy_worker_id = run_policy_worker(&runner_grpc, &mock_vm_client, vm_id).await;

    let secret_id = test_secret_id(2004);
    let secret_data = b"secret for auth test".to_vec();
    let authorized_node_id = "node-getter-authorized";
    let unauthorized_node_id = "node-getter-unauthorized";

    // 1. Put a secret
    let putter_node_id = "node-putter-for-get-test";
    let putter_kx = KeyExchangeKeyPair::generate();
    let enclave_pk_bytes = secrets_service
        .get_report(vec![])
        .unwrap()
        .ephemeral_public_key;
    let enclave_pk =
        x25519_dalek::PublicKey::from(<[u8; 32]>::try_from(enclave_pk_bytes.as_slice()).unwrap());
    let secrets_to_send = vec![(secret_id.clone(), secret_data.clone(), 0, 1)];
    let secrets_box_put = encrypt_secrets_box(&putter_kx, &enclave_pk, &secrets_to_send).unwrap();
    let binding_hash_put = secrets_box_put.calculate_binding_hash();
    let putter_env_report = test_env_report_for_client(
        putter_node_id,
        putter_kx.public_key().as_bytes(),
        binding_hash_put.to_vec(),
    );
    execute_policy_with_env_report(
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        putter_env_report.clone(),
        vec![secret_id.clone()],
        true,
    )
    .await;
    let put_req = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box_put.clone().into()),
            env_report: Some(putter_env_report.clone().into()),
        }],
    });
    let put_resp = secrets_grpc.put_secrets(put_req).await.unwrap();
    assert!(put_resp.into_inner().success, "Initial PutSecrets failed"); // This was the original panic point
    assert!(check_secret_exists(&secrets_grpc, &secret_id).await);
    info!("Test Setup: Secret put successfully");

    // 2. Authorize the *authorized* node
    let authorized_getter_kx = KeyExchangeKeyPair::generate();
    let authorized_getter_env_report = test_env_report_for_client(
        authorized_node_id,
        authorized_getter_kx.public_key().as_bytes(),
        vec![0u8; 32], // user_data for GetSecrets attestation can be arbitrary
    );
    execute_policy_with_env_report(
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        authorized_getter_env_report.clone(),
        vec![secret_id.clone()],
        true,
    )
    .await;

    // 3. Attempt GetSecrets from the *unauthorized* node
    let unauthorized_getter_kx = KeyExchangeKeyPair::generate();
    let unauthorized_getter_env_report = test_env_report_for_client(
        unauthorized_node_id,
        unauthorized_getter_kx.public_key().as_bytes(),
        vec![1u8; 32], // Different user_data to ensure different attestation if needed
    );
    // DO NOT authorize this unauthorized_getter_env_report

    let get_req_unauth = Request::new(ProtoGetSecretsRequest {
        requests: vec![SecretRequest {
            id: Some(secret_id.clone().into()),
        }],
        policy_reports: vec![],
        requester_env_report: Some(unauthorized_getter_env_report.clone().into()),
    });
    let get_resp_unauth = secrets_grpc.get_secrets(get_req_unauth).await.unwrap();
    let secrets_box_unauth =
        SecretsBox::from(get_resp_unauth.into_inner().secrets_box.unwrap());
    assert!(
        secrets_box_unauth.contained_secret_ids.is_empty(),
        "Unauthorized GetSecrets should yield empty box"
    );
    info!("Test OK: GetSecrets returned empty box for unauthorized node");

    // 4. Verify authorized node *can* get it
    let get_req_auth = Request::new(ProtoGetSecretsRequest {
        requests: vec![SecretRequest {
            id: Some(secret_id.clone().into()),
        }],
        policy_reports: vec![],
        requester_env_report: Some(authorized_getter_env_report.clone().into()),
    });
    let get_resp_auth = secrets_grpc.get_secrets(get_req_auth).await.unwrap();
    let secrets_box_auth = SecretsBox::from(get_resp_auth.into_inner().secrets_box.unwrap());
    assert_eq!(secrets_box_auth.contained_secret_ids.len(), 1);
    info!("Test OK: Authorized node successfully retrieved secret");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_get_secrets_invalid_requester_report() {
    let (_secrets_service, runner_service, mock_vm_client, secrets_grpc, _runner_grpc) =
        setup_services();
    let vm_id = "mock-vm-get-badreport";
    attach_mock_vm(&runner_service, vm_id, mock_vm_client.clone()).await;

    let secret_id = test_secret_id(2005);
    let getter_node_id = "node-getter-badreport";

    let mut bad_env_report_proto: nxcc_interface::proto::interface::EnvReport =
        test_env_report_for_client(getter_node_id, &[0; 32], vec![]).into();
    // Tamper with the attestation part of the proto directly
    bad_env_report_proto
        .attestation
        .as_mut()
        .unwrap()
        .ephemeral_public_key = vec![0; 31]; // Invalid key length

    let get_secrets_req = Request::new(ProtoGetSecretsRequest {
        requests: vec![SecretRequest {
            id: Some(secret_id.clone().into()),
        }],
        policy_reports: vec![],
        requester_env_report: Some(bad_env_report_proto),
    });
    let result = secrets_grpc.get_secrets(get_secrets_req).await;
    assert!(
        result.is_err(),
        "GetSecrets should fail with invalid report"
    );
    let status = result.err().unwrap();
    assert_eq!(status.code(), Code::Internal); // Secrets service maps this to Internal
    assert!(
        status
            .message()
            .contains("Invalid ephemeral public key length")
    );
    info!("Test OK: GetSecrets failed due to invalid requester EnvReport");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_runner_ops_non_existent_entities() {
    let (_secrets_service, runner_service, mock_vm_client, _secrets_grpc, runner_grpc) =
        setup_services();
    let vm_id = "mock-vm-ops-exist"; // This VM will exist
    let non_existent_vm_id = "vm-does-not-exist";
    let non_existent_worker_id = "worker-does-not-exist";

    attach_mock_vm(&runner_service, vm_id, mock_vm_client.clone()).await;
    let _real_worker_id = run_policy_worker(&runner_grpc, &mock_vm_client, vm_id).await; // This worker will exist

    // Test RunWorker on non-existent VM
    let run_req_bad_vm = Request::new(RunWorkerRequest {
        vm_id: non_existent_vm_id.to_string(),
        worker_code: vec![],
        manifest: vec![],
    });
    assert_eq!(
        runner_grpc
            .run_worker(run_req_bad_vm)
            .await
            .err()
            .unwrap()
            .code(),
        Code::FailedPrecondition
    );

    // Test DetachVm on non-existent VM (should be OK)
    let detach_req_bad_vm = Request::new(DetachVmRequest {
        vm_id: non_existent_vm_id.to_string(),
    });
    assert!(runner_grpc.detach_vm(detach_req_bad_vm).await.is_ok());

    // Test TerminateWorker on non-existent worker (should be OK)
    let term_req_bad_worker = Request::new(TerminateWorkerRequest {
        worker_id: non_existent_worker_id.to_string(),
    });
    assert!(
        runner_grpc
            .terminate_worker(term_req_bad_worker)
            .await
            .is_ok()
    );

    // Test InvokeWorker on non-existent worker
    let invoke_req_bad_worker = Request::new(InvokeWorkerRequest {
        worker_id: non_existent_worker_id.to_string(),
        payload: vec![],
    });
    assert_eq!(
        runner_grpc
            .invoke_worker(invoke_req_bad_worker)
            .await
            .err()
            .unwrap()
            .code(),
        Code::NotFound
    );

    // Test ExecutePolicy on non-existent worker
    let exec_req_bad_worker = Request::new(ProtoExecutePolicyRequest {
        worker_id: non_existent_worker_id.to_string(),
        contexts: vec![],
    });
    assert_eq!(
        runner_grpc
            .execute_policy(exec_req_bad_worker)
            .await
            .err()
            .unwrap()
            .code(),
        Code::NotFound
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
    let policy_worker_id = run_policy_worker(&runner_grpc, &mock_vm_client, vm_id).await;

    let secret_id_gen = test_secret_id(3001);

    // 1. Attempt GenerateSecrets without authorization -> Skips, no error
    let gen_req_unauth = Request::new(GenerateSecretsRequest {
        ids: vec![secret_id_gen.clone().into()],
    });
    assert!(secrets_grpc.generate_secrets(gen_req_unauth).await.is_ok());
    assert!(!check_secret_exists(&secrets_grpc, &secret_id_gen).await);
    info!("Test OK: GenerateSecrets skipped unauthorized request");

    // 2. Authorize self-generation
    authorize_self_generation(&secrets_service, &secret_id_gen).await;
    info!(
        "Test Setup: Authorized self-generation for {:?}",
        secret_id_gen
    );

    // 3. GenerateSecrets successfully
    let gen_req_auth = Request::new(GenerateSecretsRequest {
        ids: vec![secret_id_gen.clone().into()],
    });
    assert!(secrets_grpc.generate_secrets(gen_req_auth).await.is_ok());
    assert!(check_secret_exists(&secrets_grpc, &secret_id_gen).await);
    info!("Test OK: GenerateSecrets succeeded");

    // 4. Attempt GenerateSecrets again for the same ID -> Fails (AlreadyExists)
    let gen_req_dup = Request::new(GenerateSecretsRequest {
        ids: vec![secret_id_gen.clone().into()],
    });
    assert_eq!(
        secrets_grpc
            .generate_secrets(gen_req_dup)
            .await
            .err()
            .unwrap()
            .code(),
        Code::AlreadyExists
    );
    info!("Test OK: GenerateSecrets failed for duplicate ID");

    // 5. Attempt PutSecrets for the generated secret -> Fails (Existing is canonical)
    let putter_node_id = "node-putter-for-gen";
    let putter_kx = KeyExchangeKeyPair::generate();
    let enclave_pk_bytes = secrets_service
        .get_report(vec![])
        .unwrap()
        .ephemeral_public_key;
    let enclave_pk =
        x25519_dalek::PublicKey::from(<[u8; 32]>::try_from(enclave_pk_bytes.as_slice()).unwrap());
    let secrets_to_send_put = vec![(secret_id_gen.clone(), b"overwrite attempt".to_vec(), 0, 1)];
    let secrets_box_put =
        encrypt_secrets_box(&putter_kx, &enclave_pk, &secrets_to_send_put).unwrap();
    let binding_hash_put = secrets_box_put.calculate_binding_hash();
    let putter_env_report = test_env_report_for_client(
        putter_node_id,
        putter_kx.public_key().as_bytes(),
        binding_hash_put.to_vec(),
    );
    execute_policy_with_env_report(
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        putter_env_report.clone(),
        vec![secret_id_gen.clone()],
        true,
    )
    .await;
    let put_req = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box_put.clone().into()),
            env_report: Some(putter_env_report.clone().into()),
        }],
    });
    let put_resp = secrets_grpc.put_secrets(put_req).await.unwrap();
    assert!(
        !put_resp.into_inner().success,
        "PutSecrets should not overwrite generated secret"
    );
    info!("Test OK: PutSecrets did not overwrite generated secret");

    // 6. Authorize getter and GetSecrets
    let getter_node_id = "node-getter-for-gen";
    let getter_kx = KeyExchangeKeyPair::generate();
    let getter_env_report = test_env_report_for_client(
        getter_node_id,
        getter_kx.public_key().as_bytes(),
        vec![0u8; 32], // Arbitrary user_data for getter's report
    );
    execute_policy_with_env_report(
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id,
        getter_env_report.clone(),
        vec![secret_id_gen.clone()],
        true,
    )
    .await;

    let get_req = Request::new(ProtoGetSecretsRequest {
        requests: vec![SecretRequest {
            id: Some(secret_id_gen.clone().into()),
        }],
        policy_reports: vec![],
        requester_env_report: Some(getter_env_report.clone().into()),
    });
    let get_resp = secrets_grpc.get_secrets(get_req).await.unwrap();
    let secrets_box_get = SecretsBox::from(get_resp.into_inner().secrets_box.unwrap());
    assert_eq!(secrets_box_get.contained_secret_ids.len(), 1); // This was the failing assertion
    assert_eq!(secrets_box_get.contained_secret_ids[0], secret_id_gen);

    let decrypted_secrets = decrypt_secrets_box(&getter_kx, &secrets_box_get).unwrap();
    assert_eq!(decrypted_secrets.len(), 1);
    assert_eq!(decrypted_secrets[0].0, secret_id_gen);
    assert_eq!(decrypted_secrets[0].1.len(), 32);
    assert_ne!(decrypted_secrets[0].1, b"overwrite attempt".to_vec());
    assert_eq!(decrypted_secrets[0].2, 0);
    assert!(decrypted_secrets[0].3 > 0);
    info!("Test OK: GetSecrets retrieved generated secret successfully");
}
