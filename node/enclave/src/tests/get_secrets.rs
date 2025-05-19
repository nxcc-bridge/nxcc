use nxcc_interface::{
    proto::{
        enclave::{
            GetSecretsRequest as ProtoGetSecretsRequest,
            PutSecretsRequest as ProtoPutSecretsRequest, SecretsBundle, runner_server::Runner as _,
            secrets_server::Secrets as _,
        },
        interface::SecretRequest,
    },
    types::SecretsBox,
};
use tonic::{Code, Request};
use tracing::info;
use x25519_dalek;

use super::common::*;
use crate::crypto::{KeyExchangeKeyPair, encrypt_secrets_box};

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
        test_consumer_info(),
    )
    .await;
    let put_req = Request::new(ProtoPutSecretsRequest {
        secrets_bundles: vec![SecretsBundle {
            secrets_box: Some(secrets_box_put.clone().into()),
            env_report: Some(putter_env_report.clone().into()),
            consumer_info: Some(test_consumer_info().into()),
        }],
    });
    let put_resp = secrets_grpc.put_secrets(put_req).await.unwrap();
    assert!(put_resp.into_inner().success, "Initial PutSecrets failed");
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
        test_consumer_info(),
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
            secret_id: Some(secret_id.clone().into()),
            consumer: Some(test_consumer_info().into()),
        }],
        requester_env_report: Some(unauthorized_getter_env_report.clone().into()),
    });
    let get_resp_unauth = secrets_grpc.get_secrets(get_req_unauth).await.unwrap();
    let secrets_box_unauth = SecretsBox::from(get_resp_unauth.into_inner().secrets_box.unwrap());
    assert!(
        secrets_box_unauth.contained_secret_ids.is_empty(),
        "Unauthorized GetSecrets should yield empty box"
    );
    info!("Test OK: GetSecrets returned empty box for unauthorized node");

    // 4. Verify authorized node *can* get it
    let get_req_auth = Request::new(ProtoGetSecretsRequest {
        requests: vec![SecretRequest {
            secret_id: Some(secret_id.clone().into()),
            consumer: Some(test_consumer_info().into()),
        }],
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
            secret_id: Some(secret_id.clone().into()),
            consumer: Some(test_consumer_info().into()),
        }],
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
