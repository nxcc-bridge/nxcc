use nxcc_interface::proto::enclave::{
    DeliverBatchEventsRequest, DetachVmRequest, EventDelivery,
    ExecutePolicyRequest as ProtoExecutePolicyRequest, InvokeHttpWorkerRequest, RunWorkerRequest,
    TerminateWorkerRequest, runner_server::Runner as _, secrets_server::Secrets as _,
};
use nxcc_interface::proto::vm::{
    // Added
    Header as ProtoHeader,
    HttpRequest as ProtoHttpRequest,
    HttpResponse as ProtoHttpResponse,
    // runner_server::Runner as _, secrets_server::Secrets as _,
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
        Code::InvalidArgument // This should be FailedPrecondition if VM not attached
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
    use nxcc_interface::proto::interface::{Web3Log as ProtoWeb3Log, event_payload};
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
        handler_name: "default_handler".to_string(),
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

#[tokio::test]
#[tracing_test::traced_test]
async fn test_grpc_invoke_http_worker_success() {
    let (_secrets_service, runner_service, mock_vm_client, _secrets_grpc, runner_grpc) =
        setup_services();
    let vm_id = "mock-vm-grpc-http";
    attach_mock_vm(&runner_service, vm_id, mock_vm_client.clone()).await;
    let worker_id = run_policy_worker(&runner_grpc, &mock_vm_client, vm_id).await; // Use existing helper

    let http_request_proto = ProtoHttpRequest {
        method: "PUT".to_string(),
        uri: "/data/item1".to_string(),
        headers: vec![ProtoHeader {
            key: "Content-Type".to_string(),
            value: b"application/json".to_vec(),
        }],
        body: b"{\"value\": 42}".to_vec(),
    };

    let expected_http_response_proto = ProtoHttpResponse {
        status_code: 201,
        headers: vec![ProtoHeader {
            key: "Location".to_string(),
            value: b"/data/item1".to_vec(),
        }],
        body: b"Created".to_vec(),
    };

    // Configure mock VM to return the expected HTTP response
    mock_vm_client.set_worker_execution_behavior(
        &worker_id,
        nxcc_vm_base::client::mock::MockExecutionBehavior::HttpResponse(
            expected_http_response_proto.clone(),
        ),
    );

    let grpc_request = Request::new(InvokeHttpWorkerRequest {
        worker_id: worker_id.clone(),
        request: Some(http_request_proto.clone()),
    });

    let grpc_response = runner_grpc.invoke_http_worker(grpc_request).await;
    assert!(
        grpc_response.is_ok(),
        "gRPC InvokeHttpWorker failed: {:?}",
        grpc_response.err()
    );
    let response_inner = grpc_response.unwrap().into_inner();
    assert!(
        response_inner.response.is_some(),
        "gRPC response missing HttpResponse payload"
    );
    assert_eq!(
        response_inner.response.unwrap(),
        expected_http_response_proto
    );

    // Verify mock VM was called
    let http_invocations = mock_vm_client.get_http_invocations(&worker_id);
    assert_eq!(http_invocations.len(), 1);
    assert_eq!(http_invocations[0], http_request_proto);
    info!("Test OK: gRPC InvokeHttpWorker succeeded");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_grpc_invoke_http_worker_missing_request_payload() {
    let (_secrets_service, _runner_service, _mock_vm_client, _secrets_grpc, runner_grpc) =
        setup_services();

    let grpc_request = Request::new(InvokeHttpWorkerRequest {
        worker_id: "any-worker-id".to_string(),
        request: None, // Missing HttpRequest payload
    });

    let grpc_response = runner_grpc.invoke_http_worker(grpc_request).await;
    assert!(grpc_response.is_err(), "gRPC call should have failed");
    let status = grpc_response.err().unwrap();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(status.message().contains("Missing HttpRequest"));
    info!("Test OK: gRPC InvokeHttpWorker failed correctly for missing payload");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_grpc_invoke_http_worker_runner_error_mapping() {
    let (_secrets_service, runner_service, _mock_vm_client, _secrets_grpc, runner_grpc) =
        setup_services();
    // We don't need to attach a real VM or worker, as we'll rely on RunnerService's internal checks.

    // Case 1: WorkerNotFound
    let grpc_request_not_found = Request::new(InvokeHttpWorkerRequest {
        worker_id: "worker-does-not-exist-for-grpc".to_string(),
        request: Some(ProtoHttpRequest::default()),
    });
    let grpc_response_not_found = runner_grpc.invoke_http_worker(grpc_request_not_found).await;
    assert_eq!(
        grpc_response_not_found.err().unwrap().code(),
        Code::NotFound
    );

    // Case 2: VmNotAttached (requires a worker_map entry but no VM in vms map)
    let vm_id_detached = "vm-detached-for-grpc";
    let worker_id_on_detached_vm = "worker-on-detached-vm-for-grpc";
    runner_service
        .set_worker_vm_mapping(
            worker_id_on_detached_vm.to_string(),
            vm_id_detached.to_string(),
        )
        .await;
    // Do NOT attach vm_id_detached to runner_service.vms

    let grpc_request_vm_detached = Request::new(InvokeHttpWorkerRequest {
        worker_id: worker_id_on_detached_vm.to_string(),
        request: Some(ProtoHttpRequest::default()),
    });
    let grpc_response_vm_detached = runner_grpc
        .invoke_http_worker(grpc_request_vm_detached)
        .await;
    assert_eq!(
        grpc_response_vm_detached.err().unwrap().code(),
        Code::FailedPrecondition,
        "Expected FailedPrecondition for VmNotAttached"
    );
    info!("Test OK: gRPC InvokeHttpWorker error mapping verified");
}
