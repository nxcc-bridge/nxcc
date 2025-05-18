use std::sync::Arc;

use nxcc_interface::{
    proto::vm::WorkerStatus,
    types::{
        AttestationReport, ConsumerInfo, DSSE_WORKER_BUNDLE_PAYLOAD_TYPE, DsseEnvelope,
        DsseSignatureEntry, EnvReport, PolicyExecutionRequest, SecretId, WorkerBundle,
        WorkerBundlePayload, WorkerBundlePointer, WorkerManifest,
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
        chain_id: 1,
        identity_address: format!("0x{:040x}", id).parse().unwrap(),
        identity_id: alloy_primitives::Uint::from_limbs_slice(&[id]),
    }
}

// Helper function to create a default PolicyExecutionRequest for tests
pub fn test_policy_request(node_id: &str, secret_ids: Vec<SecretId>) -> PolicyExecutionRequest {
    PolicyExecutionRequest {
        secret_ids,
        consumer: ConsumerInfo {
            bundle_hash: vec![1; 32],
            signature: vec![2; 64],
        },
        env_report: EnvReport {
            attestation: AttestationReport {
                measurement: vec![0u8; 32],
                ephemeral_public_key: vec![3; 32], // Needs to be 32 bytes for Secrets mock
                block_hashes: vec![vec![4, 5], vec![6, 7]],
                user_data: vec![8, 9],
            },
            operator_signature: vec![10; 64],
            node_id: node_id.to_string(),
        },
    }
}

// Helper setup function
pub fn setup() -> (Arc<Secrets>, RunnerService, MockVmServiceClient) {
    let secrets = Secrets::new();
    let runner_service = RunnerService::new(secrets.clone());
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
