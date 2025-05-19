use nxcc_interface::{
    proto::{
        enclave::{
            GenerateSecretsRequest, GetSecretsRequest as ProtoGetSecretsRequest,
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
use crate::crypto::{KeyExchangeKeyPair, decrypt_secrets_box, encrypt_secrets_box};

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
        requests: vec![SecretRequest {
            secret_id: Some(secret_id_gen.clone().into()),
            consumer: Some(test_consumer_info().into()),
        }],
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
        requests: vec![SecretRequest {
            secret_id: Some(secret_id_gen.clone().into()),
            consumer: Some(test_consumer_info().into()),
        }],
    });
    assert!(secrets_grpc.generate_secrets(gen_req_auth).await.is_ok());
    assert!(check_secret_exists(&secrets_grpc, &secret_id_gen).await);
    info!("Test OK: GenerateSecrets succeeded");

    // 4. Attempt GenerateSecrets again for the same ID -> Fails (AlreadyExists)
    let gen_req_dup = Request::new(GenerateSecretsRequest {
        requests: vec![SecretRequest {
            secret_id: Some(secret_id_gen.clone().into()),
            consumer: Some(test_consumer_info().into()),
        }],
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
        test_consumer_info(),
    )
    .await;

    let get_req = Request::new(ProtoGetSecretsRequest {
        requests: vec![SecretRequest {
            secret_id: Some(secret_id_gen.clone().into()),
            consumer: Some(test_consumer_info().into()),
        }],
        requester_env_report: Some(getter_env_report.clone().into()),
    });
    let get_resp = secrets_grpc.get_secrets(get_req).await.unwrap();
    let secrets_box_get = SecretsBox::from(get_resp.into_inner().secrets_box.unwrap());
    assert_eq!(secrets_box_get.contained_secret_ids.len(), 1);
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
