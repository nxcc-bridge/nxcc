use nxcc_interface::{
    proto::enclave::{RunWorkerRequest, runner_server::Runner as _, secrets_server::Secrets as _},
    types::{
        ConsumerInfo, DSSE_WORKER_BUNDLE_PAYLOAD_TYPE, DsseEnvelope, DsseSignatureEntry, EnvReport,
        PolicyExecutionRequest, WorkerBundle, WorkerBundlePayload, WorkerBundlePointer,
        WorkerManifest,
    },
};
use tonic::Request;
use tracing::info;

use super::common::*;

#[tokio::test]
#[tracing_test::traced_test]
async fn test_local_worker_secret_authorization_flow() {
    let (secrets_service, runner_service, mock_vm_client, secrets_grpc, runner_grpc) =
        setup_services();
    let vm_id = "mock-vm-local-worker";
    attach_mock_vm(&runner_service, vm_id, mock_vm_client.clone()).await;

    let secret_id_for_worker = test_secret_id(5001);
    let secret_name_in_worker = "MY_WORKER_SECRET".to_string();
    let secret_data_for_worker = b"this is for the local worker".to_vec();

    // 1. Put the secret into the enclave (e.g., via normal P2P or generation)
    // For simplicity, let's use GenerateSecrets after self-authorizing generation.
    authorize_self_generation(&secrets_service, &secret_id_for_worker).await;
    let gen_req = Request::new(nxcc_interface::proto::enclave::GenerateSecretsRequest {
        requests: vec![nxcc_interface::proto::interface::SecretRequest {
            secret_id: Some(secret_id_for_worker.clone().into()),
            consumer: Some(test_consumer_info().into()), // Consumer for generation
        }],
    });
    secrets_grpc
        .generate_secrets(gen_req)
        .await
        .expect("GenerateSecrets failed");
    // Manually update the secret data to the expected value for the test
    secrets_service
        .update_secret_data_for_test(&secret_id_for_worker, secret_data_for_worker.clone());
    assert!(check_secret_exists(&secrets_grpc, &secret_id_for_worker).await);
    info!("Test Setup: Secret for worker generated and stored.");

    // 2. Prepare WorkerManifest and WorkerBundle for the local worker
    let worker_manifest_obj = WorkerManifest {
        bundle: WorkerBundlePointer {
            source: "file:local_worker.js".parse().unwrap(),
            hash: None,
        },
        identities: vec![(secret_id_for_worker.clone(), secret_name_in_worker.clone())],
        userdata: Default::default(),
    };
    let local_worker_payload_struct = WorkerBundlePayload {
        vm: "local-vm".to_string(),
        executable: b"local worker code".to_vec(),
        metadata: Default::default(),
    };
    let mut json_local_worker_payload = serde_json::to_vec(&local_worker_payload_struct).unwrap();
    let local_worker_dsse_envelope = DsseEnvelope {
        payload: base64::encode(&json_local_worker_payload),
        payload_type: DSSE_WORKER_BUNDLE_PAYLOAD_TYPE.to_string(),
        signatures: vec![DsseSignatureEntry {
            key_id: Some("local_worker_key_id".to_string()),
            sig: base64::encode(b"local_worker_signature_bytes_for_dsse_test"),
        }],
    };
    let worker_bundle_obj = WorkerBundle(serde_json::to_vec(&local_worker_dsse_envelope).unwrap());

    // 3. Daemon (simulated by test) orchestrates policy execution for self-authorization
    // 3a. Get Enclave's own EnvReport
    let enclave_attestation_report_proto = secrets_grpc
        .get_report(Request::new(
            nxcc_interface::proto::enclave::GetReportRequest { user_data: vec![] },
        ))
        .await
        .expect("Failed to get enclave report")
        .into_inner();
    let enclave_env_report = EnvReport {
        attestation: EnvReport::from(nxcc_interface::proto::interface::EnvReport {
            attestation: Some(enclave_attestation_report_proto),
            operator_signature: vec![], // Not strictly needed for this part of test
            node_id: "enclave-self".to_string(),
        })
        .attestation,
        operator_signature: vec![],
        node_id: "daemon-self".into(),
    };

    // 3b. Prepare for policy execution
    let worker_consumer_info = ConsumerInfo {
        bundle_hash: worker_bundle_obj.hash_signed_payload(),
        signature: worker_bundle_obj.get_dsse_signature(),
    };

    // 3c. Execute policy for the secret (using a mock policy worker that approves)
    let policy_worker_id_for_self_auth =
        run_policy_worker(&runner_grpc, &mock_vm_client, vm_id).await;

    execute_policy_with_env_report(
        &runner_grpc,
        &mock_vm_client,
        &policy_worker_id_for_self_auth,
        enclave_env_report, // Enclave's own report
        vec![secret_id_for_worker.clone()],
        true, // Expect policy to succeed for self-auth
        worker_consumer_info.clone(),
    )
    .await;
    info!("Test Setup: Enclave authorized itself for local worker secrets.");

    // 4. Run the worker via EnclaveRunnerGrpcService
    // This will internally call `get_secrets_for_local_worker`
    let run_req = Request::new(RunWorkerRequest {
        vm_id: vm_id.to_string(),
        worker_manifest_bytes: serde_json::to_vec(&worker_manifest_obj).unwrap(),
        worker_bundle_bytes: worker_bundle_obj.0.clone(),
    });
    let run_resp_inner = runner_grpc
        .run_worker(run_req)
        .await
        .expect("RunWorker failed for local worker")
        .into_inner();
    assert!(run_resp_inner.success, "Local worker failed to start");

    // 5. Verify the mock VM received the secret in TrustedConfig
    let worker_instance_id = run_resp_inner.worker_id;
    let (_status, _code, _untrusted, trusted_config_received) = mock_vm_client
        .get_worker_config_details(&worker_instance_id)
        .expect("Worker config not found in mock");
    assert_eq!(
        trusted_config_received.secrets.get(&secret_name_in_worker),
        Some(&secret_data_for_worker)
    );
    info!("Test OK: Local worker started and VM received the secret correctly.");
}
