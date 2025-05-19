use nxcc_interface::{
    proto::enclave::{
        PutSecretsRequest as ProtoPutSecretsRequest, SecretsBundle, runner_server::Runner as _,
        secrets_server::Secrets as _,
    },
    types::SecretsBox,
};
use tonic::Request;
use tracing::info;
use x25519_dalek;

use super::common::*;
use crate::crypto::{KeyExchangeKeyPair, encrypt_secrets_box};

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
        test_consumer_info(),
    )
    .await;

    let put_secrets_req = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box.into()),
            env_report: Some(putter_env_report_bad_hash.into()),
            consumer_info: Some(test_consumer_info().into()),
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
        test_consumer_info(),
    )
    .await;

    let put_secrets_req = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(bad_secrets_box.clone().into()),
            env_report: Some(putter_env_report_for_bad_box.into()),
            consumer_info: Some(test_consumer_info().into()),
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
        test_consumer_info(),
    )
    .await;

    let put_secrets_req = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box_wrong_key.clone().into()),
            env_report: Some(putter_env_report_for_wrong_key_box.into()),
            consumer_info: Some(test_consumer_info().into()),
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
