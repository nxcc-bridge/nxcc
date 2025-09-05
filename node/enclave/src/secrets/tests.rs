use std::thread::sleep;

use alloy_primitives::U256;
use chrono::Utc;
use nxcc_attestation::user_data_binding;
use nxcc_interface::types::{
    policy::{PolicyExecutionReport, PolicyExecutionRequest},
    secrets::{ChainIdentifier, ConsumerInfo},
};

use super::*;
use crate::attestation;

// Helper function to create a test SecretId
fn test_secret_id(id: u64) -> SecretId {
    SecretId {
        chain: ChainIdentifier::ChainId(1),
        identity_address: format!("0x{:040x}", id).parse().unwrap(),
        identity_id: U256::from(id),
    }
}

// Helper to create a specific AttestationBundle
fn test_attestation_bundle(detached_userdata: Vec<u8>) -> AttestationBundle {
    use nxcc_interface::types::attestation::RawAttestation;
    use rand::Rng;

    let userdata_hash = nxcc_attestation::user_data_binding::hash_userdata(&detached_userdata);

    // Parse the userdata to get the actual ephemeral key that should be used
    let user_data =
        nxcc_attestation::user_data_binding::UserData::from_cbor(&detached_userdata).unwrap();
    let ephemeral_key = user_data.ephemeral_public_key;

    // Create valid null evidence with the CORRECT ephemeral key (deterministic for test helper)
    // The randomization happens during real attestation generation
    let null_evidence =
        nxcc_attestation::providers::null::NullAttestationEvidence::new_deterministic(
            userdata_hash.to_vec(),
            ephemeral_key.clone(),
        );
    let evidence_bytes = serde_json::to_vec(&null_evidence).unwrap();

    AttestationBundle {
        raw_attestation: RawAttestation {
            platform_type: "null".to_string(),
            evidence: evidence_bytes,
            certificates: None,
        },
        detached_userdata,
    }
}

// Helper to create an EnvReport with a specific AttestationBundle
fn test_env_report(attestation: AttestationBundle) -> EnvReport {
    EnvReport {
        attestation,
        operator_signature: None, // No operator signature for test
    }
}

// Helper function to create a test PolicyExecutionReport
fn test_policy_report(request: PolicyExecutionRequest, decision: bool) -> PolicyExecutionReport {
    PolicyExecutionReport {
        request,
        decision,
        timestamp: Utc::now().timestamp() as u64,
    }
}

#[test]
fn test_new_secrets_service() {
    let ephemeral_kx_keypair = std::sync::Arc::new(crate::crypto::KeyExchangeKeyPair::generate());
    let mock_gateway = std::sync::Arc::new(crate::attestation::MockGatewayProvider);
    let attestation_manager = Arc::new(
        crate::attestation::PlatformAttestationManager::new(
            ephemeral_kx_keypair.clone(),
            mock_gateway,
        )
        .unwrap(),
    );
    let secrets = Secrets::new_with_keypair(ephemeral_kx_keypair, attestation_manager);
    assert!(secrets.secrets_storage.read().unwrap().is_empty());
    assert!(secrets.authorizations.read().unwrap().is_empty());
    let _pk = secrets.ephemeral_kx_keypair.public_key();
    // Keypair is generated during Secrets::new(), so it should always be available
    assert!(
        !secrets
            .ephemeral_kx_keypair
            .public_key()
            .as_bytes()
            .is_empty()
    );
}

#[tokio::test]
async fn test_get_report() {
    // Initialize the platform attestation manager for this test
    let ephemeral_kx_keypair = std::sync::Arc::new(crate::crypto::KeyExchangeKeyPair::generate());
    let mock_gateway = std::sync::Arc::new(crate::attestation::MockGatewayProvider);
    let attestation_manager = Arc::new(
        crate::attestation::PlatformAttestationManager::new(
            ephemeral_kx_keypair.clone(),
            mock_gateway,
        )
        .unwrap(),
    );
    let secrets = Secrets::new_with_keypair(ephemeral_kx_keypair.clone(), attestation_manager);
    let report = secrets.get_report().await.unwrap();
    let userdata = user_data_binding::UserData::from_cbor(&report.detached_userdata).unwrap();

    assert_eq!(
        userdata.ephemeral_public_key,
        ephemeral_kx_keypair.public_key().as_bytes()
    );
    // assert!(!userdata.block_hashes.is_empty()); // TODO: enable block hash freshness checking
}

#[test]
fn test_store_and_check_authorization() {
    let ephemeral_kx_keypair = std::sync::Arc::new(crate::crypto::KeyExchangeKeyPair::generate());
    let mock_gateway = std::sync::Arc::new(crate::attestation::MockGatewayProvider);
    let attestation_manager = Arc::new(
        crate::attestation::PlatformAttestationManager::new(
            ephemeral_kx_keypair.clone(),
            mock_gateway,
        )
        .unwrap(),
    );
    let secrets = Secrets::new_with_keypair(ephemeral_kx_keypair, attestation_manager);
    let secret_id = test_secret_id(123);

    let client_kx = KeyExchangeKeyPair::generate();
    let client_userdata =
        user_data_binding::UserData::new(client_kx.public_key().as_bytes().to_vec(), vec![]);
    let client_attestation = test_attestation_bundle(client_userdata.to_cbor().unwrap());
    let client_env_report = test_env_report(client_attestation.clone());

    // Initially, no authorization exists (check with a *different* attestation to be sure)
    let other_kx = KeyExchangeKeyPair::generate();
    let other_userdata =
        user_data_binding::UserData::new(other_kx.public_key().as_bytes().to_vec(), vec![]);
    let other_attestation = test_attestation_bundle(other_userdata.to_cbor().unwrap());
    assert!(!secrets.check_authorization(&other_attestation, &secret_id, &ConsumerInfo::default()));

    // Create and store a policy report with a positive decision, using client_env_report
    let policy_request = PolicyExecutionRequest {
        attestation_claims: None,
        secret_ids: vec![secret_id.clone()],
        consumer: ConsumerInfo::default(),
        env_report: client_env_report.clone(),
    };
    let policy_report_obj = test_policy_report(policy_request.clone(), true);
    secrets.store_authorization(policy_report_obj);

    // Now authorization should exist when checking with the *same* attestation
    assert!(secrets.check_authorization(&client_attestation, &secret_id, &policy_request.consumer));

    // Check with a different attestation (should fail)
    assert!(!secrets.check_authorization(&other_attestation, &secret_id, &policy_request.consumer));
    // Check with same attestation but different secret (should fail)
    assert!(!secrets.check_authorization(
        &client_attestation,
        &test_secret_id(456),
        &policy_request.consumer
    ));
}

#[test]
fn test_store_authorization_with_negative_decision() {
    let ephemeral_kx_keypair = std::sync::Arc::new(crate::crypto::KeyExchangeKeyPair::generate());
    let mock_gateway = std::sync::Arc::new(crate::attestation::MockGatewayProvider);
    let attestation_manager = Arc::new(
        crate::attestation::PlatformAttestationManager::new(
            ephemeral_kx_keypair.clone(),
            mock_gateway,
        )
        .unwrap(),
    );
    let secrets = Secrets::new_with_keypair(ephemeral_kx_keypair, attestation_manager);
    let secret_id = test_secret_id(234);

    let client_kx = KeyExchangeKeyPair::generate();
    let client_userdata =
        user_data_binding::UserData::new(client_kx.public_key().as_bytes().to_vec(), vec![]);
    let client_attestation = test_attestation_bundle(client_userdata.to_cbor().unwrap());
    let client_env_report = test_env_report(client_attestation.clone());

    let policy_request = PolicyExecutionRequest {
        attestation_claims: None,
        secret_ids: vec![secret_id.clone()],
        consumer: ConsumerInfo::default(),
        env_report: client_env_report.clone(),
    };
    let policy_report_obj = test_policy_report(policy_request, false); // Negative decision
    secrets.store_authorization(policy_report_obj);

    assert!(!secrets.check_authorization(
        &client_attestation,
        &secret_id,
        &ConsumerInfo::default()
    ));
}

#[test]
fn test_authorization_determinism_bug() {
    // This test exposes the determinism bug in calculate_authorization_id
    let ephemeral_kx_keypair = std::sync::Arc::new(crate::crypto::KeyExchangeKeyPair::generate());
    let mock_gateway = std::sync::Arc::new(crate::attestation::MockGatewayProvider);
    let attestation_manager = Arc::new(
        crate::attestation::PlatformAttestationManager::new(
            ephemeral_kx_keypair.clone(),
            mock_gateway,
        )
        .unwrap(),
    );
    let secrets = Secrets::new_with_keypair(ephemeral_kx_keypair, attestation_manager);
    let secret_id = test_secret_id(999);

    // Create two different clients to test if the authorization system incorrectly
    // treats different attestations as different authorizations (the bug)
    let client_kx_1 = KeyExchangeKeyPair::generate();
    let client_kx_2 = KeyExchangeKeyPair::generate();

    let client_userdata_1 =
        user_data_binding::UserData::new(client_kx_1.public_key().as_bytes().to_vec(), vec![]);
    let client_userdata_2 =
        user_data_binding::UserData::new(client_kx_2.public_key().as_bytes().to_vec(), vec![]);

    // Generate two different attestation bundles - these will definitely be different
    let client_attestation_1 = test_attestation_bundle(client_userdata_1.to_cbor().unwrap());
    let client_attestation_2 = test_attestation_bundle(client_userdata_2.to_cbor().unwrap());

    // These are different clients, so attestations should definitely be different
    assert_ne!(
        client_attestation_1.raw_attestation.evidence,
        client_attestation_2.raw_attestation.evidence,
        "Attestations should be different with different clients"
    );

    let client_env_report_1 = test_env_report(client_attestation_1.clone());

    // Store authorization using first client's attestation
    let policy_request = PolicyExecutionRequest {
        attestation_claims: None,
        secret_ids: vec![secret_id.clone()],
        consumer: ConsumerInfo::default(),
        env_report: client_env_report_1.clone(),
    };
    let policy_report = test_policy_report(policy_request, true);
    secrets.store_authorization(policy_report);

    // Check authorization with SAME attestation - should work
    assert!(secrets.check_authorization(
        &client_attestation_1,
        &secret_id,
        &ConsumerInfo::default()
    ));

    // Check authorization with DIFFERENT client's attestation - should fail (correctly)
    // This demonstrates that authorization IDs are tied to specific attestation bundles
    let check_result =
        secrets.check_authorization(&client_attestation_2, &secret_id, &ConsumerInfo::default());

    assert!(
        !check_result,
        "Different client should not be authorized for the same secret"
    );
}

#[tokio::test]
async fn test_authorization_determinism_bug_e2e() {
    // This test demonstrates the REAL authorization determinism bug in the full E2E flow:
    // 1. Policy execution creates authorization with AttestationBundle_A
    // 2. generate_secrets() calls get_report() which creates AttestationBundle_B
    // 3. Authorization check fails because hash(A) != hash(B) due to randomization

    let ephemeral_kx_keypair = std::sync::Arc::new(crate::crypto::KeyExchangeKeyPair::generate());
    let mock_gateway = std::sync::Arc::new(crate::attestation::MockGatewayProvider);
    let attestation_manager = Arc::new(
        crate::attestation::PlatformAttestationManager::new(
            ephemeral_kx_keypair.clone(),
            mock_gateway,
        )
        .unwrap(),
    );
    let secrets = Secrets::new_with_keypair(ephemeral_kx_keypair, attestation_manager);
    let secret_id = test_secret_id(888);
    let consumer_info = ConsumerInfo::default();

    // Step 1: Simulate policy execution that grants authorization
    // This would happen when a policy worker evaluates and approves the request
    let policy_attestation = secrets.get_report().await.unwrap(); // AttestationBundle_A
    let policy_env_report = test_env_report(policy_attestation.clone());

    let policy_request = PolicyExecutionRequest {
        attestation_claims: None,
        secret_ids: vec![secret_id.clone()],
        consumer: consumer_info.clone(),
        env_report: policy_env_report,
    };
    let policy_report = test_policy_report(policy_request, true);
    secrets.store_authorization(policy_report);

    // Verify authorization was stored correctly with AttestationBundle_A
    assert!(secrets.check_authorization(&policy_attestation, &secret_id, &consumer_info));

    // Step 2: Generate a fresh attestation to compare
    let fresh_attestation = secrets.get_report().await.unwrap(); // AttestationBundle_B

    // Verify randomization is working (attestations should have different hashes)
    let policy_hash = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        ciborium::into_writer(&policy_attestation, &mut hasher).unwrap();
        hasher.finalize()
    };
    let fresh_hash = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        ciborium::into_writer(&fresh_attestation, &mut hasher).unwrap();
        hasher.finalize()
    };
    assert_ne!(
        policy_hash, fresh_hash,
        "Attestation randomization should produce different hashes"
    );

    // Step 3: Later, generate_secrets() is called
    // This internally calls get_report() again, which creates AttestationBundle_C
    // Due to randomization, AttestationBundle_A != AttestationBundle_C
    let result = secrets
        .generate_secrets(vec![(secret_id.clone(), consumer_info.clone())])
        .await;

    // The bug: generate_secrets should succeed (same client, valid authorization)
    // Verify that the authorization determinism bug is fixed:
    // Even though fresh attestation has different quote hash, authorization should work
    // because it now uses stable ephemeral key instead of entire AttestationBundle
    result.expect("generate_secrets should succeed");

    let secrets_map = secrets.secrets_storage.read().unwrap();
    assert!(
        secrets_map.contains_key(&secret_id),
        "Secret should be generated despite different attestation hashes - authorization uses \
         stable ephemeral key"
    );
}

#[test]
fn test_authorization_expiry() {
    let ephemeral_kx_keypair = std::sync::Arc::new(crate::crypto::KeyExchangeKeyPair::generate());
    let mock_gateway = std::sync::Arc::new(crate::attestation::MockGatewayProvider);
    let attestation_manager = Arc::new(
        crate::attestation::PlatformAttestationManager::new(
            ephemeral_kx_keypair.clone(),
            mock_gateway,
        )
        .unwrap(),
    );
    let secrets = Secrets::new_with_keypair(ephemeral_kx_keypair, attestation_manager);
    let secret_id = test_secret_id(345);

    let client_kx = KeyExchangeKeyPair::generate();
    let client_userdata =
        user_data_binding::UserData::new(client_kx.public_key().as_bytes().to_vec(), vec![]);
    let client_attestation = test_attestation_bundle(client_userdata.to_cbor().unwrap());
    let client_env_report = test_env_report(client_attestation.clone());

    let policy_request = PolicyExecutionRequest {
        attestation_claims: None,
        secret_ids: vec![secret_id.clone()],
        consumer: ConsumerInfo::default(),
        env_report: client_env_report.clone(),
    };

    let past_time = Utc::now().timestamp() as u64 - 3601; // Grant was 1h + 1s ago
    let mut policy_report_obj = test_policy_report(policy_request, true);
    policy_report_obj.timestamp = past_time; // Authorization expiry is timestamp + 3600
    secrets.store_authorization(policy_report_obj);

    // Authorization should not be valid because it's expired
    assert!(!secrets.check_authorization(
        &client_attestation,
        &secret_id,
        &ConsumerInfo::default()
    ));

    // Manually check the authorizations map
    let ephemeral_key = client_kx.public_key().as_bytes(); // Extract the same ephemeral key used in userdata
    let auth_id = calculate_authorization_id(ephemeral_key, &secret_id, &ConsumerInfo::default());
    let auth_map = secrets.authorizations.read().unwrap();
    assert!(auth_map.contains_key(&auth_id)); // Should be present
    assert!(*auth_map.get(&auth_id).unwrap() < Utc::now().timestamp() as u64); // But expired
}

#[tokio::test]
async fn test_put_secrets_epk_binding_success() {
    let ephemeral_kx_keypair = std::sync::Arc::new(crate::crypto::KeyExchangeKeyPair::generate());
    let mock_gateway = std::sync::Arc::new(crate::attestation::MockGatewayProvider);
    let attestation_manager = Arc::new(
        crate::attestation::PlatformAttestationManager::new(
            ephemeral_kx_keypair.clone(),
            mock_gateway,
        )
        .unwrap(),
    );
    let secrets = Secrets::new_with_keypair(ephemeral_kx_keypair, attestation_manager); // Receiver
    let secret_id = test_secret_id(456);
    let secret_data = vec![10, 20, 30];
    let expiry = Utc::now().timestamp() as u64 + 3600;

    let sender_kx = KeyExchangeKeyPair::generate(); // Sender's key for DH and attestation

    // Create secrets box
    let secrets_to_send = vec![(secret_id.clone(), secret_data.clone(), expiry, 1)];
    let secrets_box = encrypt_secrets_box(
        &sender_kx,
        secrets.ephemeral_kx_keypair.public_key(),
        &secrets_to_send,
    )
    .unwrap();

    // EnvReport that the sender will present, binding their EPK
    let sender_userdata =
        user_data_binding::UserData::new(sender_kx.public_key().as_bytes().to_vec(), vec![]);
    let presented_attestation = test_attestation_bundle(sender_userdata.to_cbor().unwrap());
    let presented_env_report = test_env_report(presented_attestation.clone());

    // Authorize based on the attestation that will be presented
    let auth_request = PolicyExecutionRequest {
        attestation_claims: None,
        secret_ids: vec![secret_id.clone()],
        consumer: ConsumerInfo::default(),
        env_report: presented_env_report.clone(),
    };
    secrets.store_authorization(test_policy_report(auth_request, true));
    assert!(secrets.check_authorization(
        &presented_attestation,
        &secret_id,
        &ConsumerInfo::default()
    ));

    // --- Receiver Side ---
    let result = secrets
        .put_secrets(vec![(
            secrets_box.clone(),
            presented_env_report.clone(),
            ConsumerInfo::default(),
        )])
        .await;
    assert!(result.is_ok(), "put_secrets failed: {:?}", result.err());
    assert!(result.unwrap(), "put_secrets returned false, expected true");

    let secrets_map = secrets.secrets_storage.read().unwrap();
    assert!(secrets_map.contains_key(&secret_id));
    let stored = secrets_map.get(&secret_id).unwrap();
    assert_eq!(stored.data, secret_data);
    assert_eq!(stored.expiry, expiry);
}

#[tokio::test]
async fn test_put_secrets_epk_binding_mismatch() {
    let ephemeral_kx_keypair = std::sync::Arc::new(crate::crypto::KeyExchangeKeyPair::generate());
    let mock_gateway = std::sync::Arc::new(crate::attestation::MockGatewayProvider);
    let attestation_manager = Arc::new(
        crate::attestation::PlatformAttestationManager::new(
            ephemeral_kx_keypair.clone(),
            mock_gateway,
        )
        .unwrap(),
    );
    let secrets = Secrets::new_with_keypair(ephemeral_kx_keypair, attestation_manager);
    let secret_id = test_secret_id(457);

    let sender_kx = KeyExchangeKeyPair::generate();
    let wrong_sender_kx = KeyExchangeKeyPair::generate(); // A different key

    let secrets_to_send = vec![(
        secret_id.clone(),
        vec![11, 21, 31],
        Utc::now().timestamp() as u64 + 3600,
        1,
    )];
    // Box is created with the correct sender_kx
    let secrets_box = encrypt_secrets_box(
        &sender_kx,
        secrets.ephemeral_kx_keypair.public_key(),
        &secrets_to_send,
    )
    .unwrap();

    // But the attestation is generated with the wrong_sender_kx
    let wrong_userdata =
        user_data_binding::UserData::new(wrong_sender_kx.public_key().as_bytes().to_vec(), vec![]);
    let presented_attestation_wrong_key =
        test_attestation_bundle(wrong_userdata.to_cbor().unwrap());
    let presented_env_report_wrong_key = test_env_report(presented_attestation_wrong_key.clone());

    // Authorize the (wrong) attestation
    let auth_request = PolicyExecutionRequest {
        attestation_claims: None,
        secret_ids: vec![secret_id.clone()],
        consumer: ConsumerInfo::default(),
        env_report: presented_env_report_wrong_key.clone(),
    };
    secrets.store_authorization(test_policy_report(auth_request, true));

    let result = secrets
        .put_secrets(vec![(
            secrets_box.clone(),
            presented_env_report_wrong_key,
            ConsumerInfo::default(),
        )])
        .await;
    assert!(result.is_ok());
    assert!(!result.unwrap()); // Should fail due to hash mismatch
    assert!(
        !secrets
            .secrets_storage
            .read()
            .unwrap()
            .contains_key(&secret_id)
    );
}

#[tokio::test]
async fn test_put_secrets_existing_is_canonical() {
    let ephemeral_kx_keypair = std::sync::Arc::new(crate::crypto::KeyExchangeKeyPair::generate());
    let mock_gateway = std::sync::Arc::new(crate::attestation::MockGatewayProvider);
    let attestation_manager = Arc::new(
        crate::attestation::PlatformAttestationManager::new(
            ephemeral_kx_keypair.clone(),
            mock_gateway,
        )
        .unwrap(),
    );
    let secrets = Secrets::new_with_keypair(ephemeral_kx_keypair, attestation_manager);
    let secret_id = test_secret_id(555);
    let initial_secret_data = vec![1, 1, 1];

    let sender_kx = KeyExchangeKeyPair::generate();

    // --- First Put ---
    let secrets_to_send1 = vec![(secret_id.clone(), initial_secret_data.clone(), 0, 1)];
    let secrets_box1 = encrypt_secrets_box(
        &sender_kx,
        secrets.ephemeral_kx_keypair.public_key(),
        &secrets_to_send1,
    )
    .unwrap();
    let userdata1 =
        user_data_binding::UserData::new(sender_kx.public_key().as_bytes().to_vec(), vec![]);
    let env_report1_attestation = test_attestation_bundle(userdata1.to_cbor().unwrap());
    let env_report1 = test_env_report(env_report1_attestation.clone());

    let auth_req1 = PolicyExecutionRequest {
        attestation_claims: None,
        secret_ids: vec![secret_id.clone()],
        consumer: ConsumerInfo::default(),
        env_report: env_report1.clone(),
    };
    secrets.store_authorization(test_policy_report(auth_req1, true));

    let result1 = secrets
        .put_secrets(vec![(
            secrets_box1,
            env_report1.clone(),
            ConsumerInfo::default(),
        )])
        .await;
    assert!(result1.is_ok() && result1.unwrap());
    let initial_timestamp = secrets
        .secrets_storage
        .read()
        .unwrap()
        .get(&secret_id)
        .unwrap()
        .generation_timestamp;

    sleep(std::time::Duration::from_millis(10)); // Ensure timestamp can differ

    // --- Second Put (attempt to overwrite) ---
    let new_secret_data = vec![2, 2, 2];
    let expiry2 = Utc::now().timestamp() as u64 + 3600;
    let secrets_to_send2 = vec![(secret_id.clone(), new_secret_data.clone(), expiry2, 2)];
    // Use same sender_kx, so attestation's PK is same. Box content changes, so binding_hash changes.
    let secrets_box2 = encrypt_secrets_box(
        &sender_kx,
        secrets.ephemeral_kx_keypair.public_key(),
        &secrets_to_send2,
    )
    .unwrap();

    // The attestation is the same since sender_kx is the same.
    let userdata2 =
        user_data_binding::UserData::new(sender_kx.public_key().as_bytes().to_vec(), vec![]);
    let env_report2_attestation = test_attestation_bundle(userdata2.to_cbor().unwrap());

    let env_report2 = test_env_report(env_report2_attestation.clone());
    let auth_req2 = PolicyExecutionRequest {
        attestation_claims: None,
        secret_ids: vec![secret_id.clone()],
        consumer: ConsumerInfo::default(),
        env_report: env_report2.clone(),
    };
    secrets.store_authorization(test_policy_report(auth_req2, true)); // Authorize the second attempt

    let result2 = secrets
        .put_secrets(vec![(
            secrets_box2,
            env_report2.clone(),
            ConsumerInfo::default(),
        )])
        .await;
    assert!(result2.is_ok());
    assert!(result2.unwrap()); // Should update with newer timestamp

    let stored_after = secrets
        .secrets_storage
        .read()
        .unwrap()
        .get(&secret_id)
        .unwrap()
        .clone();
    assert_eq!(stored_after.data, new_secret_data);
    assert!(stored_after.generation_timestamp > initial_timestamp);
}

#[tokio::test]
async fn test_put_secrets_unauthorized_with_attestation() {
    let ephemeral_kx_keypair = std::sync::Arc::new(crate::crypto::KeyExchangeKeyPair::generate());
    let mock_gateway = std::sync::Arc::new(crate::attestation::MockGatewayProvider);
    let attestation_manager = Arc::new(
        crate::attestation::PlatformAttestationManager::new(
            ephemeral_kx_keypair.clone(),
            mock_gateway,
        )
        .unwrap(),
    );
    let secrets = Secrets::new_with_keypair(ephemeral_kx_keypair, attestation_manager);
    let secret_id = test_secret_id(567);

    let sender_kx = KeyExchangeKeyPair::generate();
    let secrets_to_send = vec![(secret_id.clone(), vec![10, 20, 30], 0, 1)];
    let secrets_box = encrypt_secrets_box(
        &sender_kx,
        secrets.ephemeral_kx_keypair.public_key(),
        &secrets_to_send,
    )
    .unwrap();
    let userdata =
        user_data_binding::UserData::new(sender_kx.public_key().as_bytes().to_vec(), vec![]);
    let presented_attestation = test_attestation_bundle(userdata.to_cbor().unwrap());
    let presented_env_report = test_env_report(presented_attestation.clone());

    // Do NOT authorize
    assert!(!secrets.check_authorization(
        &presented_attestation,
        &secret_id,
        &ConsumerInfo::default()
    ));

    let result = secrets
        .put_secrets(vec![(
            secrets_box,
            presented_env_report,
            ConsumerInfo::default(),
        )])
        .await;
    assert!(result.is_ok());
    assert!(!result.unwrap()); // Should fail due to no authorization
    assert!(
        !secrets
            .secrets_storage
            .read()
            .unwrap()
            .contains_key(&secret_id)
    );
}

#[tokio::test]
async fn test_put_secrets_expired() {
    let ephemeral_kx_keypair = std::sync::Arc::new(crate::crypto::KeyExchangeKeyPair::generate());
    let mock_gateway = std::sync::Arc::new(crate::attestation::MockGatewayProvider);
    let attestation_manager = Arc::new(
        crate::attestation::PlatformAttestationManager::new(
            ephemeral_kx_keypair.clone(),
            mock_gateway,
        )
        .unwrap(),
    );
    let secrets = Secrets::new_with_keypair(ephemeral_kx_keypair, attestation_manager);
    let secret_id = test_secret_id(678);
    let expiry = Utc::now().timestamp() as u64 - 3600; // Expired

    let sender_kx = KeyExchangeKeyPair::generate();
    let secrets_to_send = vec![(secret_id.clone(), vec![10, 20, 30], expiry, 1)];
    let secrets_box = encrypt_secrets_box(
        &sender_kx,
        secrets.ephemeral_kx_keypair.public_key(),
        &secrets_to_send,
    )
    .unwrap();
    let userdata =
        user_data_binding::UserData::new(sender_kx.public_key().as_bytes().to_vec(), vec![]);
    let presented_attestation = test_attestation_bundle(userdata.to_cbor().unwrap());
    let presented_env_report = test_env_report(presented_attestation.clone());

    let auth_req = PolicyExecutionRequest {
        attestation_claims: None,
        secret_ids: vec![secret_id.clone()],
        consumer: ConsumerInfo::default(),
        env_report: presented_env_report.clone(),
    };
    secrets.store_authorization(test_policy_report(auth_req, true));

    let result = secrets
        .put_secrets(vec![(
            secrets_box,
            presented_env_report,
            ConsumerInfo::default(),
        )])
        .await;
    assert!(result.is_ok());
    assert!(!result.unwrap()); // False because secret was expired
    assert!(
        !secrets
            .secrets_storage
            .read()
            .unwrap()
            .contains_key(&secret_id)
    );
}

#[tokio::test]
async fn test_put_secrets_older_ignored() {
    let ephemeral_kx_keypair = std::sync::Arc::new(crate::crypto::KeyExchangeKeyPair::generate());
    let mock_gateway = std::sync::Arc::new(crate::attestation::MockGatewayProvider);
    let attestation_manager = Arc::new(
        crate::attestation::PlatformAttestationManager::new(
            ephemeral_kx_keypair.clone(),
            mock_gateway,
        )
        .unwrap(),
    );
    let secrets = Secrets::new_with_keypair(ephemeral_kx_keypair, attestation_manager);
    let secret_id = test_secret_id(679);

    let sender_kx = KeyExchangeKeyPair::generate();

    // First put with newer timestamp
    let secrets_to_send1 = vec![(secret_id.clone(), vec![1], 0, 2)];
    let box1 = encrypt_secrets_box(
        &sender_kx,
        secrets.ephemeral_kx_keypair.public_key(),
        &secrets_to_send1,
    )
    .unwrap();
    let userdata1 =
        user_data_binding::UserData::new(sender_kx.public_key().as_bytes().to_vec(), vec![]);
    let env1 = test_env_report(test_attestation_bundle(userdata1.to_cbor().unwrap()));
    let auth_req1 = PolicyExecutionRequest {
        attestation_claims: None,
        secret_ids: vec![secret_id.clone()],
        consumer: ConsumerInfo::default(),
        env_report: env1.clone(),
    };
    secrets.store_authorization(test_policy_report(auth_req1, true));
    assert!(
        secrets
            .put_secrets(vec![(box1, env1.clone(), ConsumerInfo::default())])
            .await
            .unwrap()
    );

    let consumer_info_for_auth_req2 = ConsumerInfo::default(); // Define it for auth_req2

    // Second put with older timestamp
    let secrets_to_send2 = vec![(secret_id.clone(), vec![2], 0, 1)];
    let box2 = encrypt_secrets_box(
        &sender_kx,
        secrets.ephemeral_kx_keypair.public_key(),
        &secrets_to_send2,
    )
    .unwrap();
    let userdata2 =
        user_data_binding::UserData::new(sender_kx.public_key().as_bytes().to_vec(), vec![]);
    let env2 = test_env_report(test_attestation_bundle(userdata2.to_cbor().unwrap()));
    let auth_req2 = PolicyExecutionRequest {
        attestation_claims: None,
        secret_ids: vec![secret_id.clone()],
        consumer: consumer_info_for_auth_req2.clone(), // Use the defined consumer_info
        env_report: env2.clone(),
    };
    secrets.store_authorization(test_policy_report(auth_req2, true));
    let res2 = secrets
        .put_secrets(vec![(box2, env2.clone(), consumer_info_for_auth_req2)]) // Pass it here
        .await
        .unwrap();

    assert!(!res2);

    let stored = secrets
        .secrets_storage
        .read()
        .unwrap()
        .get(&secret_id)
        .unwrap()
        .clone();
    assert_eq!(stored.data, vec![1]);
    assert_eq!(stored.generation_timestamp, 2);
}

#[tokio::test]
async fn test_put_secrets_multiple_bundles() {
    let ephemeral_kx_keypair = std::sync::Arc::new(crate::crypto::KeyExchangeKeyPair::generate());
    let mock_gateway = std::sync::Arc::new(crate::attestation::MockGatewayProvider);
    let attestation_manager = Arc::new(
        crate::attestation::PlatformAttestationManager::new(
            ephemeral_kx_keypair.clone(),
            mock_gateway,
        )
        .unwrap(),
    );
    let secrets = Secrets::new_with_keypair(ephemeral_kx_keypair, attestation_manager);
    let secret_id1 = test_secret_id(789); // Authorized for node1
    let secret_id2 = test_secret_id(790); // Authorized for node2
    let secret_id3_unauth = test_secret_id(791); // Unauthorized in bundle 1

    let sender_kx1 = KeyExchangeKeyPair::generate();
    let sender_kx2 = KeyExchangeKeyPair::generate();

    // --- Bundle 1 Prep (node1, secret1 - auth, secret3 - unauth) ---
    let secrets_to_send1 = vec![
        (secret_id1.clone(), vec![1, 2, 3], 0, 1),
        (secret_id3_unauth.clone(), vec![9, 9, 9], 0, 1),
    ];
    let secrets_box1 = encrypt_secrets_box(
        &sender_kx1,
        secrets.ephemeral_kx_keypair.public_key(),
        &secrets_to_send1,
    )
    .unwrap();
    let userdata1 =
        user_data_binding::UserData::new(sender_kx1.public_key().as_bytes().to_vec(), vec![]);
    let attestation1 = test_attestation_bundle(userdata1.to_cbor().unwrap());
    let env_report1 = test_env_report(attestation1.clone());

    let consumer_info1 = ConsumerInfo {
        bundle_hash: vec![1],
        signature: vec![1],
    };
    // Authorize node1 for secret1 (using attestation1)
    let auth_req1 = PolicyExecutionRequest {
        attestation_claims: None,
        secret_ids: vec![secret_id1.clone()],
        consumer: consumer_info1.clone(),
        env_report: env_report1.clone(),
    };
    secrets.store_authorization(test_policy_report(auth_req1.clone(), true));
    // Node1 is NOT authorized for secret_id3_unauth with attestation1

    // --- Bundle 2 Prep (node2, secret2 - auth) ---
    let secrets_to_send2 = vec![(secret_id2.clone(), vec![4, 5, 6], 0, 1)];
    let secrets_box2 = encrypt_secrets_box(
        &sender_kx2,
        secrets.ephemeral_kx_keypair.public_key(),
        &secrets_to_send2,
    )
    .unwrap();
    let userdata2 =
        user_data_binding::UserData::new(sender_kx2.public_key().as_bytes().to_vec(), vec![]);
    let attestation2 = test_attestation_bundle(userdata2.to_cbor().unwrap());
    let env_report2 = test_env_report(attestation2.clone());

    let consumer_info2 = ConsumerInfo {
        bundle_hash: vec![2],
        signature: vec![2],
    };
    // Authorize node2 for secret2 (using attestation2)
    let auth_req2 = PolicyExecutionRequest {
        attestation_claims: None,
        secret_ids: vec![secret_id2.clone()],
        consumer: consumer_info2.clone(),
        env_report: env_report2.clone(),
    };
    secrets.store_authorization(test_policy_report(auth_req2.clone(), true));

    // --- Put Bundles ---
    let result = secrets
        .put_secrets(vec![
            (secrets_box1, env_report1, consumer_info1),
            (secrets_box2, env_report2, consumer_info2),
        ])
        .await;
    assert!(result.is_ok());

    assert!(result.unwrap()); // True because bundle 2 succeeded

    let secrets_map = secrets.secrets_storage.read().unwrap();
    assert!(!secrets_map.contains_key(&secret_id1)); // Bundle 1 skipped
    assert!(secrets_map.contains_key(&secret_id2)); // Bundle 2 processed
    assert!(!secrets_map.contains_key(&secret_id3_unauth));
    assert_eq!(secrets_map.get(&secret_id2).unwrap().data, vec![4, 5, 6]);
}

#[tokio::test]
async fn test_get_secrets_authorization_check() {
    let ephemeral_kx_keypair = std::sync::Arc::new(crate::crypto::KeyExchangeKeyPair::generate());
    let mock_gateway = std::sync::Arc::new(crate::attestation::MockGatewayProvider);
    let attestation_manager = Arc::new(
        crate::attestation::PlatformAttestationManager::new(
            ephemeral_kx_keypair.clone(),
            mock_gateway,
        )
        .unwrap(),
    );
    let secrets = Secrets::new_with_keypair(ephemeral_kx_keypair, attestation_manager);
    let secret_id1 = test_secret_id(890); // Authorized
    let secret_id2 = test_secret_id(891); // Not authorized for this requester

    // Store secrets
    let mut smap = secrets.secrets_storage.write().unwrap();
    smap.insert(
        secret_id1.clone(),
        StoredSecret {
            data: vec![1],
            expiry: 0,
            generation_timestamp: 0,
        },
    );
    smap.insert(
        secret_id2.clone(),
        StoredSecret {
            data: vec![2],
            expiry: 0,
            generation_timestamp: 0,
        },
    );
    drop(smap);

    let requester_kx = KeyExchangeKeyPair::generate();
    let requester_userdata =
        user_data_binding::UserData::new(requester_kx.public_key().as_bytes().to_vec(), vec![]);
    let requester_attestation = test_attestation_bundle(requester_userdata.to_cbor().unwrap());
    let requester_env_report = test_env_report(requester_attestation.clone());

    let consumer_for_auth_req = ConsumerInfo::default();
    // Authorize requester for secret_id1 only, using their specific attestation
    let auth_req = PolicyExecutionRequest {
        attestation_claims: None,
        secret_ids: vec![secret_id1.clone()],
        consumer: consumer_for_auth_req.clone(),
        env_report: requester_env_report.clone(),
    };
    secrets.store_authorization(test_policy_report(auth_req, true));

    let result = secrets
        .get_secrets(
            vec![
                (secret_id1.clone(), consumer_for_auth_req.clone()),
                (secret_id2.clone(), consumer_for_auth_req), // Use same consumer for simplicity
            ],
            requester_env_report.clone(), // Requester presents their EnvReport
        )
        .await;
    assert!(result.is_ok(), "get_secrets failed: {:?}", result);
    let secrets_box = result.unwrap();
    assert_eq!(secrets_box.contained_secret_ids.len(), 1);
    assert!(secrets_box.contained_secret_ids.contains(&secret_id1));
}
