use nxcc_interface::proto::enclave::{
    runner_server::Runner as _, secrets_server::Secrets as _, DeliverBatchEventsRequest,
    DetachVmRequest, EventDelivery, ExecutePolicyRequest as ProtoExecutePolicyRequest,
    RunWorkerRequest, TerminateWorkerRequest,
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
        launch_event_payload: None,
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
    assert!(runner_grpc
        .terminate_worker(term_req_bad_worker)
        .await
        .is_ok());

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

    // Test DeliverBatchEvents with non-existent worker
    use nxcc_interface::proto::interface::{event_payload, Web3Log as ProtoWeb3Log};
    let proto_web3_log = ProtoWeb3Log {
        address: vec![1; 20],
        topics: vec![],
        data: vec![1, 2, 3],
        block_hash: vec![],
        block_number: 123,
        transaction_hash: vec![],
        transaction_index: 0,
        log_index: 0,
        removed: false,
    };
    let dummy_event_payload = nxcc_interface::proto::interface::EventPayload {
        payload: Some(event_payload::Payload::Web3Log(proto_web3_log)),
    };
    let event_delivery = EventDelivery {
        worker_id: non_existent_worker_id.to_string(),
        event_payload: Some(dummy_event_payload),
    };
    let deliver_req_bad_worker = Request::new(DeliverBatchEventsRequest {
        events: vec![event_delivery],
    });
    // deliver_batch_events sends to a channel; it won't immediately know if the worker exists.
    // The internal event loop will handle the non-existent worker.
    // The gRPC call itself should succeed if the request is well-formed.
    let response = runner_grpc
        .deliver_batch_events(deliver_req_bad_worker)
        .await;
    assert!(
        response.is_ok(),
        "DeliverBatchEvents with non-existent worker should still be accepted by gRPC handler: \
         {:?}",
        response.err()
    );
    assert!(response.unwrap().into_inner().success);

    info!("Test OK: Runner operations correctly handled non-existent VMs/workers");
}
