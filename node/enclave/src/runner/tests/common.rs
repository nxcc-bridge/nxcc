use std::sync::Arc;

use nxcc_interface::{
    proto::vm::WorkerStatus,
    types::{
        attestation::{AttestationBundle, EnvReport},
        policy::PolicyExecutionRequest,
        secrets::{ChainIdentifier, ConsumerInfo, SecretId},
        worker::{
            DSSE_WORKER_BUNDLE_PAYLOAD_TYPE, DsseEnvelope, DsseSignatureEntry, WorkerBundle,
            WorkerBundlePayload, WorkerBundlePointer, WorkerManifest,
        },
    },
};
use nxcc_vm_base::client::{
    VmClient as _,
    mock::{MockExecutionBehavior, MockVmServiceClient},
};

use super::*;
use crate::{runner::RunnerService, secrets::Secrets};

// Helper function to create a default SecretId for tests
pub fn test_secret_id(id: u64) -> SecretId {
    SecretId {
        chain: ChainIdentifier::ChainId(1),
        identity_address: format!("0x{:040x}", id).parse().unwrap(),
        identity_id: alloy_primitives::Uint::from_limbs_slice(&[id]),
    }
}

// Helper function to create a default PolicyExecutionRequest for tests
pub fn test_policy_request(secret_ids: Vec<SecretId>) -> PolicyExecutionRequest {
    PolicyExecutionRequest {
        secret_ids,
        attestation_claims: None,
        consumer: ConsumerInfo {
            bundle_hash: vec![1; 32],
            signature: vec![2; 64],
        },
        env_report: {
            let ephemeral_key = vec![3; 32];
            let test_userdata = nxcc_attestation::user_data_binding::UserData::new(
                ephemeral_key.clone(),
                vec![
                    nxcc_attestation::BlockInfo {
                        chain_id: 1,
                        chain_name: "test1".to_string(),
                        block_number: 1,
                        block_hash: vec![4, 5],
                        timestamp: 0,
                        fetched_at: 0,
                    },
                    nxcc_attestation::BlockInfo {
                        chain_id: 2,
                        chain_name: "test2".to_string(),
                        block_number: 2,
                        block_hash: vec![6, 7],
                        timestamp: 0,
                        fetched_at: 0,
                    },
                ],
            );
            let detached_userdata = test_userdata.to_cbor().unwrap();
            let userdata_hash =
                nxcc_attestation::user_data_binding::hash_userdata(&detached_userdata);

            // Create valid null evidence
            let null_evidence =
                nxcc_attestation::providers::null::NullAttestationEvidence::new_deterministic(
                    userdata_hash.to_vec(),
                    ephemeral_key.clone(),
                );
            let evidence_bytes = serde_json::to_vec(&null_evidence).unwrap();

            EnvReport {
                attestation: AttestationBundle {
                    raw_attestation: nxcc_interface::types::attestation::RawAttestation {
                        platform_type: "null".to_string(),
                        evidence: evidence_bytes,
                        certificates: None,
                    },
                    detached_userdata,
                },
                operator_signature: None,
            }
        },
    }
}

// Helper setup function
pub fn setup() -> (Arc<Secrets>, RunnerService, MockVmServiceClient) {
    // Generate ephemeral keypair for the attestation manager
    let ephemeral_kx_keypair = std::sync::Arc::new(crate::crypto::KeyExchangeKeyPair::generate());

    let mock_gateway = std::sync::Arc::new(crate::attestation::MockGatewayProvider);
    let attestation_manager = Arc::new(
        crate::attestation::PlatformAttestationManager::new(
            ephemeral_kx_keypair.clone(),
            mock_gateway,
        )
        .expect("Failed to create attestation manager for test"),
    );

    let secrets =
        Secrets::new_with_keypair(ephemeral_kx_keypair.clone(), attestation_manager.clone());
    let runner_service = RunnerService::new(secrets.clone(), attestation_manager);
    let mock_client = MockVmServiceClient::new();
    (secrets, runner_service, mock_client)
}

// Helper to manually "attach" a mock VM
pub async fn attach_mock_vm(
    runner_service: &RunnerService,
    vm_id: &str,
    client: MockVmServiceClient,
) {
    let mut vms_guard = runner_service.vms.write().await;
    vms_guard.insert(vm_id.to_string(), client.into());
}

// Helper to manually add a worker mapping
pub async fn add_worker_mapping(runner_service: &RunnerService, worker_id: &str, vm_id: &str) {
    let mut worker_map_guard = runner_service.worker_map.write().await;
    worker_map_guard.insert(worker_id.to_string(), vm_id.to_string());
}

// Helper to create a default WorkerManifest for tests
pub fn test_worker_manifest() -> WorkerManifest {
    WorkerManifest {
        bundle: WorkerBundlePointer {
            source: "file:dummy.js".parse().unwrap(),
            hash: None,
        },
        identities: vec![],
        userdata: Default::default(),
    }
}

// Helper to create a default WorkerBundle for tests
pub fn test_worker_bundle(executable_code: Vec<u8>) -> WorkerBundle {
    let payload_struct = WorkerBundlePayload {
        vm: "test-vm".to_string(),
        executable: executable_code,
        metadata: Default::default(),
    };
    let json_payload_bytes = serde_json::to_vec(&payload_struct).unwrap();

    let dsse_envelope = DsseEnvelope {
        payload: base64::encode(&json_payload_bytes),
        payload_type: DSSE_WORKER_BUNDLE_PAYLOAD_TYPE.to_string(),
        signatures: vec![DsseSignatureEntry {
            key_id: Some("test_key_id".to_string()),
            // Using a valid base64 string for the mock signature
            sig: base64::encode(b"mock_signature_bytes_longer_than_32_for_base64"),
        }],
    };
    WorkerBundle(serde_json::to_vec(&dsse_envelope).unwrap())
}
