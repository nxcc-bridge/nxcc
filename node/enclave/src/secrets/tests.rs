use std::thread::sleep;

use alloy_primitives::U256;
use chrono::Utc;
use nxcc_interface::types::{ConsumerInfo, PolicyExecutionReport, PolicyExecutionRequest};

use super::*;

// Helper function to create a test SecretId
fn test_secret_id(id: u64) -> SecretId {
    SecretId {
        chain_id: 1,
        identity_address: format!("0x{:040x}", id).parse().unwrap(),
        identity_id: U256::from(id),
    }
}

// Helper to create a specific AttestationReport
fn test_attestation_report(ephemeral_pk: Vec<u8>, user_data: Vec<u8>) -> AttestationReport {
    AttestationReport {
        measurement: vec![0u8; 32], // Consistent measurement for tests
        ephemeral_public_key: ephemeral_pk,
        block_hashes: vec![vec![1, 2, 3]], // Consistent block_hashes
        user_data,
    }
}

// Helper to create an EnvReport with a specific AttestationReport
fn test_env_report(node_id: &str, attestation: AttestationReport) -> EnvReport {
    EnvReport {
        attestation,
        operator_signature: vec![7; 64], // Consistent signature
        node_id: node_id.to_string(),
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
    let secrets = Secrets::new();
    assert!(secrets.secrets_storage.read().unwrap().is_empty());
    assert!(secrets.authorizations.read().unwrap().is_empty());
    let _pk = secrets.ephemeral_kx_keypair.public_key();
    assert!(Lazy::get(&secrets.ephemeral_kx_keypair).is_some());
}

#[test]
fn test_get_report() {
    let secrets = Secrets::new();
    let user_data = vec![1, 2, 3, 4];
    let report = secrets.get_report(user_data.clone()).unwrap();
    assert_eq!(
        report.ephemeral_public_key,
        secrets.ephemeral_kx_keypair.public_key().as_bytes()
    );
    assert_eq!(report.user_data, user_data);
    assert!(!report.block_hashes.is_empty());
}

#[test]
fn test_store_and_check_authorization() {
    let secrets = Secrets::new();
    let node_id = "test-node-1";
    let secret_id = test_secret_id(123);

    let client_kx = KeyExchangeKeyPair::generate();
    let client_attestation =
        test_attestation_report(client_kx.public_key().as_bytes().to_vec(), vec![0u8; 32]);
    let client_env_report = test_env_report(node_id, client_attestation.clone());

    // Initially, no authorization exists (check with a *different* attestation to be sure)
    let other_kx = KeyExchangeKeyPair::generate();
    let other_attestation =
        test_attestation_report(other_kx.public_key().as_bytes().to_vec(), vec![1u8; 32]);
    assert!(!secrets.check_authorization(&other_attestation, &secret_id, &ConsumerInfo::default()));

    // Create and store a policy report with a positive decision, using client_env_report
    let policy_request = PolicyExecutionRequest {
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
    let secrets = Secrets::new();
    let node_id = "test-node-2";
    let secret_id = test_secret_id(234);

    let client_kx = KeyExchangeKeyPair::generate();
    let client_attestation =
        test_attestation_report(client_kx.public_key().as_bytes().to_vec(), vec![0u8; 32]);
    let client_env_report = test_env_report(node_id, client_attestation.clone());

    let policy_request = PolicyExecutionRequest {
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
fn test_authorization_expiry() {
    let secrets = Secrets::new();
    let node_id = "test-node-3";
    let secret_id = test_secret_id(345);

    let client_kx = KeyExchangeKeyPair::generate();
    let client_attestation =
        test_attestation_report(client_kx.public_key().as_bytes().to_vec(), vec![0u8; 32]);
    let client_env_report = test_env_report(node_id, client_attestation.clone());

    let policy_request = PolicyExecutionRequest {
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
    let auth_id =
        calculate_authorization_id(&client_attestation, &secret_id, &ConsumerInfo::default());
    let auth_map = secrets.authorizations.read().unwrap();
    assert!(auth_map.contains_key(&auth_id)); // Should be present
    assert!(*auth_map.get(&auth_id).unwrap() < Utc::now().timestamp() as u64); // But expired
}

#[test]
fn test_put_secrets_attestation_binding_success() {
    let secrets = Secrets::new(); // Receiver
    let sender_node_id = "test-sender-node";
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
    let binding_hash = secrets_box.calculate_binding_hash();

    // EnvReport that the sender will present (ephemeral_public_key = sender_kx, user_data = binding_hash)
    let presented_attestation = test_attestation_report(
        sender_kx.public_key().as_bytes().to_vec(),
        binding_hash.to_vec(),
    );
    let presented_env_report = test_env_report(sender_node_id, presented_attestation.clone());

    // Authorize based on the attestation that will be presented
    let auth_request = PolicyExecutionRequest {
        secret_ids: vec![secret_id.clone()],
        consumer: ConsumerInfo::default(),
        env_report: presented_env_report.clone(),
    };
    let auth_report_obj = test_policy_report(auth_request, true);
    secrets.store_authorization(auth_report_obj);
    assert!(secrets.check_authorization(
        &presented_attestation,
        &secret_id,
        &ConsumerInfo::default()
    ));

    // --- Receiver Side ---
    let result = secrets.put_secrets(vec![(
        secrets_box.clone(),
        presented_env_report.clone(),
        ConsumerInfo::default(),
    )]);
    assert!(result.is_ok(), "put_secrets failed: {:?}", result.err());
    assert!(result.unwrap(), "put_secrets returned false, expected true");

    let secrets_map = secrets.secrets_storage.read().unwrap();
    assert!(secrets_map.contains_key(&secret_id));
    let stored = secrets_map.get(&secret_id).unwrap();
    assert_eq!(stored.data, secret_data);
    assert_eq!(stored.expiry, expiry);
}

#[test]
fn test_put_secrets_attestation_binding_hash_mismatch() {
    let secrets = Secrets::new();
    let sender_node_id = "test-sender-node-mismatch";
    let secret_id = test_secret_id(457);

    let sender_kx = KeyExchangeKeyPair::generate();
    let secrets_to_send = vec![(
        secret_id.clone(),
        vec![11, 21, 31],
        Utc::now().timestamp() as u64 + 3600,
        1,
    )];
    let secrets_box = encrypt_secrets_box(
        &sender_kx,
        secrets.ephemeral_kx_keypair.public_key(),
        &secrets_to_send,
    )
    .unwrap();

    let correct_binding_hash = secrets_box.calculate_binding_hash();
    let mut incorrect_hash_vec = correct_binding_hash.to_vec();
    incorrect_hash_vec[0] ^= 0xff; // Tamper

    // Attestation that sender *would* present if hash was correct (for auth store)
    let auth_attestation = test_attestation_report(
        sender_kx.public_key().as_bytes().to_vec(),
        correct_binding_hash.to_vec(),
    );
    let auth_env_report = test_env_report(sender_node_id, auth_attestation.clone());

    let auth_request = PolicyExecutionRequest {
        secret_ids: vec![secret_id.clone()],
        consumer: ConsumerInfo::default(),
        env_report: auth_env_report,
    };
    secrets.store_authorization(test_policy_report(auth_request, true));

    // EnvReport with the *incorrect* hash
    let presented_attestation_bad_hash = test_attestation_report(
        sender_kx.public_key().as_bytes().to_vec(),
        incorrect_hash_vec,
    );
    let presented_env_report_bad_hash =
        test_env_report(sender_node_id, presented_attestation_bad_hash);

    let result = secrets.put_secrets(vec![(
        secrets_box.clone(),
        presented_env_report_bad_hash,
        ConsumerInfo::default(),
    )]);
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

#[test]
fn test_put_secrets_existing_is_canonical() {
    let secrets = Secrets::new();
    let node_id = "test-node-canonical";
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
    let binding_hash1 = secrets_box1.calculate_binding_hash();
    let env_report1_attestation = test_attestation_report(
        sender_kx.public_key().as_bytes().to_vec(),
        binding_hash1.to_vec(),
    );
    let env_report1 = test_env_report(node_id, env_report1_attestation.clone());

    let auth_req1 = PolicyExecutionRequest {
        secret_ids: vec![secret_id.clone()],
        consumer: ConsumerInfo::default(),
        env_report: env_report1.clone(),
    };
    secrets.store_authorization(test_policy_report(auth_req1, true));

    let result1 = secrets.put_secrets(vec![(
        secrets_box1,
        env_report1.clone(),
        ConsumerInfo::default(),
    )]);
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
    let binding_hash2 = secrets_box2.calculate_binding_hash();
    // The attestation for auth needs to match this new binding_hash if we were to re-authorize.
    // But for this test, the existing auth (for env_report1's attestation) might be hit if measurements are same.
    // The crucial part is that PutSecrets itself checks for existing.
    // For the second put, the authorization check will use env_report2.attestation.
    // If env_report1.attestation and env_report2.attestation are different (due to user_data/binding_hash),
    // then a *new* authorization for env_report2 would be needed.
    // Let's assume authorization is granted for the second attempt as well, to focus on the canonical check.

    let env_report2_attestation = test_attestation_report(
        sender_kx.public_key().as_bytes().to_vec(),
        binding_hash2.to_vec(),
    );
    let env_report2 = test_env_report(node_id, env_report2_attestation.clone());
    let auth_req2 = PolicyExecutionRequest {
        secret_ids: vec![secret_id.clone()],
        consumer: ConsumerInfo::default(),
        env_report: env_report2.clone(),
    };
    secrets.store_authorization(test_policy_report(auth_req2, true)); // Authorize the second attempt

    let result2 = secrets.put_secrets(vec![(
        secrets_box2,
        env_report2.clone(),
        ConsumerInfo::default(),
    )]);
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

#[test]
fn test_put_secrets_unauthorized_with_attestation() {
    let secrets = Secrets::new();
    let sender_node_id = "test-sender-node-unauth";
    let secret_id = test_secret_id(567);

    let sender_kx = KeyExchangeKeyPair::generate();
    let secrets_to_send = vec![(secret_id.clone(), vec![10, 20, 30], 0, 1)];
    let secrets_box = encrypt_secrets_box(
        &sender_kx,
        secrets.ephemeral_kx_keypair.public_key(),
        &secrets_to_send,
    )
    .unwrap();
    let binding_hash = secrets_box.calculate_binding_hash();

    let presented_attestation = test_attestation_report(
        sender_kx.public_key().as_bytes().to_vec(),
        binding_hash.to_vec(),
    );
    let presented_env_report = test_env_report(sender_node_id, presented_attestation.clone());

    // Do NOT authorize
    assert!(!secrets.check_authorization(
        &presented_attestation,
        &secret_id,
        &ConsumerInfo::default()
    ));

    let result = secrets.put_secrets(vec![(
        secrets_box,
        presented_env_report,
        ConsumerInfo::default(),
    )]);
    assert!(result.is_ok());
    assert!(!result.unwrap()); // Should be false due to no auth
    assert!(
        !secrets
            .secrets_storage
            .read()
            .unwrap()
            .contains_key(&secret_id)
    );
}

#[test]
fn test_put_secrets_expired() {
    let secrets = Secrets::new();
    let node_id = "test-node-6";
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
    let binding_hash = secrets_box.calculate_binding_hash();

    let presented_attestation = test_attestation_report(
        sender_kx.public_key().as_bytes().to_vec(),
        binding_hash.to_vec(),
    );
    let presented_env_report = test_env_report(node_id, presented_attestation.clone());

    let auth_req = PolicyExecutionRequest {
        secret_ids: vec![secret_id.clone()],
        consumer: ConsumerInfo::default(),
        env_report: presented_env_report.clone(),
    };
    secrets.store_authorization(test_policy_report(auth_req, true));

    let result = secrets.put_secrets(vec![(
        secrets_box,
        presented_env_report,
        ConsumerInfo::default(),
    )]);
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

#[test]
fn test_put_secrets_older_ignored() {
    let secrets = Secrets::new();
    let node_id = "test-node-old";
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
    let bh1 = box1.calculate_binding_hash();
    let env1 = test_env_report(
        node_id,
        test_attestation_report(sender_kx.public_key().as_bytes().to_vec(), bh1.to_vec()),
    );
    let auth_req1 = PolicyExecutionRequest {
        secret_ids: vec![secret_id.clone()],
        consumer: ConsumerInfo::default(),
        env_report: env1.clone(),
    };
    secrets.store_authorization(test_policy_report(auth_req1, true));
    assert!(
        secrets
            .put_secrets(vec![(box1, env1.clone(), ConsumerInfo::default())])
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
    let env2 = test_env_report(
        node_id,
        test_attestation_report(
            sender_kx.public_key().as_bytes().to_vec(),
            box2.calculate_binding_hash().to_vec(),
        ),
    );
    let auth_req2 = PolicyExecutionRequest {
        secret_ids: vec![secret_id.clone()],
        consumer: consumer_info_for_auth_req2.clone(), // Use the defined consumer_info
        env_report: env2.clone(),
    };
    secrets.store_authorization(test_policy_report(auth_req2, true));
    let res2 = secrets
        .put_secrets(vec![(box2, env2.clone(), consumer_info_for_auth_req2)]) // Pass it here
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

#[test]
fn test_put_secrets_multiple_bundles() {
    let secrets = Secrets::new();
    let node_id1 = "test-node-7a";
    let node_id2 = "test-node-7b";
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
    let binding_hash1 = secrets_box1.calculate_binding_hash();
    let attestation1 = test_attestation_report(
        sender_kx1.public_key().as_bytes().to_vec(),
        binding_hash1.to_vec(),
    );
    let env_report1 = test_env_report(node_id1, attestation1.clone());

    let consumer_info1 = ConsumerInfo {
        bundle_hash: vec![1],
        signature: vec![1],
    };
    // Authorize node1 for secret1 (using attestation1)
    let auth_req1 = PolicyExecutionRequest {
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
    let binding_hash2 = secrets_box2.calculate_binding_hash();
    let attestation2 = test_attestation_report(
        sender_kx2.public_key().as_bytes().to_vec(),
        binding_hash2.to_vec(),
    );
    let env_report2 = test_env_report(node_id2, attestation2.clone());

    let consumer_info2 = ConsumerInfo {
        bundle_hash: vec![2],
        signature: vec![2],
    };
    // Authorize node2 for secret2 (using attestation2)
    let auth_req2 = PolicyExecutionRequest {
        secret_ids: vec![secret_id2.clone()],
        consumer: consumer_info2.clone(),
        env_report: env_report2.clone(),
    };
    secrets.store_authorization(test_policy_report(auth_req2.clone(), true));

    // --- Put Bundles ---
    let result = secrets.put_secrets(vec![
        (secrets_box1, env_report1, consumer_info1),
        (secrets_box2, env_report2, consumer_info2),
    ]);
    assert!(result.is_ok());

    assert!(result.unwrap()); // True because bundle 2 succeeded

    let secrets_map = secrets.secrets_storage.read().unwrap();
    assert!(!secrets_map.contains_key(&secret_id1)); // Bundle 1 skipped
    assert!(secrets_map.contains_key(&secret_id2)); // Bundle 2 processed
    assert!(!secrets_map.contains_key(&secret_id3_unauth));
    assert_eq!(secrets_map.get(&secret_id2).unwrap().data, vec![4, 5, 6]);
}

#[test]
fn test_get_secrets_authorization_check() {
    let secrets = Secrets::new();
    let requester_node_id = "test-requester-node";
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
    // For GetSecrets, user_data in attestation isn't a binding hash, can be anything or empty.
    let requester_attestation =
        test_attestation_report(requester_kx.public_key().as_bytes().to_vec(), vec![0u8; 32]);
    let requester_env_report = test_env_report(requester_node_id, requester_attestation.clone());

    let consumer_for_auth_req = ConsumerInfo::default();
    // Authorize requester for secret_id1 only, using their specific attestation
    let auth_req = PolicyExecutionRequest {
        secret_ids: vec![secret_id1.clone()],
        consumer: consumer_for_auth_req.clone(),
        env_report: requester_env_report.clone(),
    };
    secrets.store_authorization(test_policy_report(auth_req, true));

    let result = secrets.get_secrets(
        vec![
            (secret_id1.clone(), consumer_for_auth_req.clone()),
            (secret_id2.clone(), consumer_for_auth_req), // Use same consumer for simplicity
        ],
        requester_env_report.clone(), // Requester presents their EnvReport
    );
    assert!(result.is_ok(), "get_secrets failed: {:?}", result.err());
    let secrets_box = result.unwrap();
    assert_eq!(secrets_box.contained_secret_ids.len(), 1);
    assert!(secrets_box.contained_secret_ids.contains(&secret_id1));
}
