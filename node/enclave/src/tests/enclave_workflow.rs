use std::time::{SystemTime, UNIX_EPOCH};

use nxcc_interface::{
    proto::{
        enclave::{
            ExecutePolicyRequest as ProtoExecutePolicyRequest,
            GetSecretsRequest as ProtoGetSecretsRequest,
            PutSecretsRequest as ProtoPutSecretsRequest, SecretsBundle, runner_server::Runner as _,
            secrets_server::Secrets as _,
        },
        interface::SecretRequest,
    },
    types::SecretsBox,
};
use tonic::Request;
use tracing::info;
use x25519_dalek;

use super::common::*;
use crate::crypto::{KeyExchangeKeyPair, decrypt_secrets_box, encrypt_secrets_box};

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
        test_consumer_info(),
    )
    .await;

    let put_secrets_req_fail = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box_for_put.clone().into()),
            env_report: Some(putter_env_report.clone().into()), // Putter uses its EnvReport
            consumer_info: Some(test_consumer_info().into()),
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
        test_consumer_info(),
    )
    .await;

    // --- 5. PutSecret succeeds ---
    let put_secrets_req_ok = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box_for_put.clone().into()),
            env_report: Some(putter_env_report.clone().into()), // Putter uses its EnvReport
            consumer_info: Some(test_consumer_info().into()),
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
            secret_id: Some(secret_id.clone().into()),
            consumer: Some(test_consumer_info().into()),
        }],
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
        getter_env_report.clone(), // Getter presents its EnvReport again
        vec![secret_id.clone()],
        true, // Expect policy to succeed for getter
        test_consumer_info(),
    )
    .await;

    // --- 8. GetSecret succeeds ---
    info!("Step 8: Attempting GetSecret (expected success)");
    let get_secrets_req_ok = Request::new(ProtoGetSecretsRequest {
        requests: vec![SecretRequest {
            secret_id: Some(secret_id.clone().into()),
            consumer: Some(test_consumer_info().into()),
        }],
        requester_env_report: Some(getter_env_report.clone().into()), // Getter uses its EnvReport
    });
    let get_secrets_resp_ok = secrets_grpc.get_secrets(get_secrets_req_ok).await.unwrap();
    let secrets_box_ok = SecretsBox::from(get_secrets_resp_ok.into_inner().secrets_box.unwrap());
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
            secret_id: Some(secret_id.clone().into()),
            consumer: Some(test_consumer_info().into()),
        }],
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
