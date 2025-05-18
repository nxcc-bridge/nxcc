use nxcc_interface::proto::enclave::{
    DetachVmRequest, ExecutePolicyRequest as ProtoExecutePolicyRequest, InvokeWorkerRequest,
    RunWorkerRequest, TerminateWorkerRequest, runner_server::Runner as _,
    secrets_server::Secrets as _,
};
use tonic::{Code, Request};
use tracing::info;

use super::common::*;

#[tokio::test]
#[tracing_test::traced_test]
async fn test_runner_ops_non_existent_entities() {
    let (_secrets_service, runner_service, mock_vm_client, _secrets_grpc, runner_grpc) =
        setup_services();
    let vm_id = "mock-vm-ops-exist"; // This VM will exist
    let non_existent_vm_id = "vm-does-not-exist";
    let non_existent_worker_id = "worker-does-not-exist";

    attach_mock_vm(&runner_service, vm_id, mock_vm_client.clone()).await;
    let _real_worker_id = run_policy_worker(&runner_grpc, &mock_vm_client, vm_id).await; // This worker will exist

    // Test RunWorker on non-existent VM
    let run_req_bad_vm = Request::new(RunWorkerRequest {
        vm_id: non_existent_vm_id.to_string(),
        worker_manifest_bytes: vec![],
        worker_bundle_bytes: vec![],
    });
    assert_eq!(
        runner_grpc
            .run_worker(run_req_bad_vm)
            .await
            .err()
            .unwrap()
            .code(),
        Code::InvalidArgument
    );

    // Test DetachVm on non-existent VM (should be OK)
    let detach_req_bad_vm = Request::new(DetachVmRequest {
        vm_id: non_existent_vm_id.to_string(),
    });
    assert!(runner_grpc.detach_vm(detach_req_bad_vm).await.is_ok());

    // Test TerminateWorker on non-existent worker (should be OK)
    let term_req_bad_worker = Request::new(TerminateWorkerRequest {
        worker_id: non_existent_worker_id.to_string(),
    });
    assert!(
        runner_grpc
            .terminate_worker(term_req_bad_worker)
            .await
            .is_ok()
    );

    // Test InvokeWorker on non-existent worker
    let invoke_req_bad_worker = Request::new(InvokeWorkerRequest {
        worker_id: non_existent_worker_id.to_string(),
        payload: vec![],
    });
    assert_eq!(
        runner_grpc
            .invoke_worker(invoke_req_bad_worker)
            .await
            .err()
            .unwrap()
            .code(),
        Code::NotFound
    );

    // Test ExecutePolicy on non-existent worker
    let exec_req_bad_worker = Request::new(ProtoExecutePolicyRequest {
        worker_id: non_existent_worker_id.to_string(),
        contexts: vec![],
    });
    assert_eq!(
        runner_grpc
            .execute_policy(exec_req_bad_worker)
            .await
            .err()
            .unwrap()
            .code(),
        Code::NotFound
    );

    info!("Test OK: Runner operations correctly handled non-existent VMs/workers");
}
