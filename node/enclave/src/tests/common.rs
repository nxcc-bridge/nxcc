use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use alloy_primitives::{Address, U256};
use nxcc_interface::{
    proto::enclave::{runner_server::Runner as _, secrets_server::Secrets as _},
    types::{
        AttestationReport, ConsumerInfo, DSSE_WORKER_BUNDLE_PAYLOAD_TYPE, DsseEnvelope,
        DsseSignatureEntry, EnvReport, PolicyExecutionReport, PolicyExecutionRequest, SecretId,
        SecretsBox, WorkerBundle, WorkerBundlePayload, WorkerBundlePointer, WorkerManifest,
    },
};
use nxcc_vm_base::client::mock::{MockExecutionBehavior, MockVmServiceClient};
use tonic::Request;
use tracing::info;

use crate::{
    crypto::{KeyExchangeKeyPair, decrypt_secrets_box, encrypt_secrets_box},
    grpc::{EnclaveRunnerGrpcService, SecretsGrpcService},
    runner::RunnerService,
    secrets::Secrets,
};

pub fn test_secret_id(id_num: u64) -> SecretId {
    SecretId {
        chain_id: 1,
        identity_address: Address::from_slice(&[id_num as u8; 20]),
        identity_id: U256::from(id_num),
    }
}

pub fn test_consumer_info() -> ConsumerInfo {
    ConsumerInfo {
        bundle_hash: vec![1; 32], // Changed from code_hash
        signature: vec![2; 64],   // Assuming signature remains
    }
}

// Helper to create an EnvReport with a specific ephemeral public key and user_data.
// This is crucial for ensuring consistency between policy execution context and actual operation context.
pub fn test_env_report_for_client(
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

pub fn setup_services() -> (
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

pub async fn attach_mock_vm(
    runner_service: &RunnerService,
    vm_id: &str,
    mock_client: MockVmServiceClient,
) {
    runner_service
        .attach_mock_client(vm_id.to_string(), mock_client)
        .await;
    info!("Test Setup: Attached mock VM '{}'", vm_id);
}

pub async fn run_policy_worker(
    runner_grpc: &EnclaveRunnerGrpcService,
    mock_vm_client: &MockVmServiceClient,
    vm_id: &str,
) -> String {
    let policy_worker_type_id = "policy-worker";
    let policy_executable_code = b"mock_policy_wasm".to_vec();

    let policy_manifest_obj = WorkerManifest {
        bundle: WorkerBundlePointer {
            source: "file:mock.js".parse().unwrap(),
            hash: None,
        },
        identities: vec![], // Policies typically don't request secrets themselves
        userdata: Default::default(),
    };
    let policy_payload_struct = WorkerBundlePayload {
        vm: "mock-vm".to_string(),
        executable: policy_executable_code.clone(),
        metadata: Default::default(),
    };
    let json_payload_bytes = serde_json::to_vec(&policy_payload_struct).unwrap();

    let policy_dsse_envelope = DsseEnvelope {
        payload: base64::encode(&json_payload_bytes),
        payload_type: DSSE_WORKER_BUNDLE_PAYLOAD_TYPE.to_string(),
        signatures: vec![DsseSignatureEntry {
            key_id: Some("mock_policy_worker_key_id".to_string()),
            sig: base64::encode(b"mock_policy_worker_signature_bytes_for_dsse"),
        }],
    };
    let policy_bundle_obj = WorkerBundle(serde_json::to_vec(&policy_dsse_envelope).unwrap());

    let policy_manifest_bytes = serde_json::to_vec(&policy_manifest_obj).unwrap();
    let policy_bundle_bytes = policy_bundle_obj.0.clone();

    let expected_policy_worker_instance_id = format!("instance-{}-1", policy_worker_type_id);

    let run_worker_req = Request::new(nxcc_interface::proto::enclave::RunWorkerRequest {
        vm_id: vm_id.to_string(),
        worker_manifest_bytes: policy_manifest_bytes,
        worker_bundle_bytes: policy_bundle_bytes,
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

pub async fn execute_policy_with_env_report(
    runner_grpc: &EnclaveRunnerGrpcService,
    mock_vm_client: &MockVmServiceClient,
    worker_id: &str,
    client_env_report: EnvReport,
    secret_ids: Vec<SecretId>,
    should_succeed: bool,
    consumer_info: ConsumerInfo,
) {
    let policy_req_internal = PolicyExecutionRequest {
        secret_ids: secret_ids.clone(),
        consumer: consumer_info,
        env_report: client_env_report.clone(),
    };

    let vm_response: Vec<bool> = if should_succeed {
        vec![true; std::cmp::max(1, secret_ids.len())]
    } else {
        vec![false; std::cmp::max(1, secret_ids.len())]
    };
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

    let execute_req = Request::new(nxcc_interface::proto::enclave::ExecutePolicyRequest {
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
        1
    } else {
        0
    };

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

pub async fn check_secret_exists(secrets_grpc: &SecretsGrpcService, secret_id: &SecretId) -> bool {
    get_secret_status(secrets_grpc, secret_id)
        .await
        .is_some_and(|s| s.0)
}

pub async fn get_secret_status(
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

pub async fn authorize_self_generation(secrets_service: &Secrets, secret_id: &SecretId) {
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
