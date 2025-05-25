use alloy_primitives::{Address, B256};
use nxcc_interface::types::{EventPayload, Web3Log};
use nxcc_vm_base::client::mock::MockExecutionBehavior;
use tokio::time::{Duration, sleep};

use super::common::*;

#[tokio::test]
async fn test_deliver_single_web3_event_success() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-event-delivery";
    let worker_id = "instance-policy-worker-1"; // MockVmServiceClient default format

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    // Run a worker first
    let manifest = test_worker_manifest();
    let bundle = test_worker_bundle(b"some code".to_vec());
    let launched_worker_id = runner_service
        .run_worker(vm_id.to_string(), manifest, bundle)
        .await
        .expect("Failed to run worker for event delivery test");
    assert_eq!(launched_worker_id, worker_id);

    // Prepare a Web3Log event
    let web3_log = Web3Log {
        address: Address::repeat_byte(0x01),
        topics: vec![B256::repeat_byte(0x02)],
        data: vec![0x03, 0x04].into(),
        block_hash: Some(B256::repeat_byte(0x05)),
        block_number: Some(123),
        transaction_hash: Some(B256::repeat_byte(0x06)),
        transaction_index: Some(1),
        log_index: Some(0),
        removed: false,
    };
    let event_payload = EventPayload::Web3Log(web3_log.clone());

    let handler_name = "handleWeb3Event".to_string();
    let vm_event_invocation_payload = serde_json::to_vec(&crate::runner::VmEventInvocation {
        handler: handler_name.clone(),
        event_payload: event_payload.clone(),
    })
    .unwrap();

    // Configure mock VM to expect this invocation
    mock_client.set_worker_execution_behavior(
        &launched_worker_id,
        MockExecutionBehavior::Fixed(b"event_ack".to_vec()),
    );

    // Deliver the event
    let result = runner_service
        .deliver_batch_events(vec![(
            launched_worker_id.clone(),
            handler_name,
            event_payload,
        )])
        .await;
    assert!(result.is_ok(), "deliver_batch_events failed");

    // Allow some time for the event processing loop to pick up the event
    sleep(Duration::from_millis(200)).await; // Increased delay

    // Verify the mock VM received the invocation
    let invocations = mock_client.get_invocations(&launched_worker_id);
    assert_eq!(
        invocations.len(),
        1,
        "Mock VM should have received one invocation"
    );
    // The actual payload sent to the VM client's invoke_worker is VmEventInvocation serialized
    assert_eq!(
        invocations[0], vm_event_invocation_payload,
        "Invocation payload mismatch"
    );
}

#[tokio::test]
async fn test_deliver_batch_events_multiple() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-batch-event-delivery";
    let worker_id = "instance-policy-worker-1"; // MockVmServiceClient default format

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    let manifest = test_worker_manifest();
    let bundle = test_worker_bundle(b"batch code".to_vec());
    let launched_worker_id = runner_service
        .run_worker(vm_id.to_string(), manifest, bundle)
        .await
        .expect("Failed to run worker for batch event test");
    assert_eq!(launched_worker_id, worker_id);

    let web3_log1 = Web3Log {
        address: Address::repeat_byte(0x11),
        data: vec![1].into(),
        ..Default::default()
    };
    let event_payload1 = EventPayload::Web3Log(web3_log1.clone());
    let handler_name1 = "handler1".to_string();
    let vm_payload1 = serde_json::to_vec(&crate::runner::VmEventInvocation {
        handler: handler_name1.clone(),
        event_payload: event_payload1.clone(),
    })
    .unwrap();

    let web3_log2 = Web3Log {
        address: Address::repeat_byte(0x22),
        data: vec![2].into(),
        ..Default::default()
    };
    let event_payload2 = EventPayload::Web3Log(web3_log2.clone());
    let handler_name2 = "handler2".to_string();
    let vm_payload2 = serde_json::to_vec(&crate::runner::VmEventInvocation {
        handler: handler_name2.clone(),
        event_payload: event_payload2.clone(),
    })
    .unwrap();

    mock_client.set_worker_execution_behavior(
        &launched_worker_id,
        MockExecutionBehavior::Echo, // Echo back payload
    );

    let result = runner_service
        .deliver_batch_events(vec![
            (launched_worker_id.clone(), handler_name1, event_payload1),
            (launched_worker_id.clone(), handler_name2, event_payload2),
        ])
        .await;
    assert!(result.is_ok());

    sleep(Duration::from_millis(300)).await; // Allow time for both events

    let invocations = mock_client.get_invocations(&launched_worker_id);
    assert_eq!(invocations.len(), 2);
    assert!(invocations.contains(&vm_payload1));
    assert!(invocations.contains(&vm_payload2));
}

#[tokio::test]
async fn test_deliver_event_to_non_existent_worker() {
    let (_secrets, runner_service, mock_client) = setup();
    // No VM or worker setup for this specific test of deliver_batch_events robustness

    let non_existent_worker_id = "worker-does-not-exist";
    let web3_log = Web3Log {
        address: Address::repeat_byte(0x33),
        ..Default::default()
    };
    let event_payload = EventPayload::Web3Log(web3_log);
    let handler_name = "someHandler".to_string();

    // deliver_batch_events itself should succeed as it just sends to a channel.
    // The internal event loop will log an error.
    let result = runner_service
        .deliver_batch_events(vec![(
            non_existent_worker_id.to_string(),
            handler_name,
            event_payload,
        )])
        .await;
    assert!(result.is_ok());

    // No invocations should happen on any mock client
    sleep(Duration::from_millis(100)).await;
    // If we had a way to inspect logs or a global mock client, we could verify no calls.
    // For now, we just ensure the send itself doesn't panic or error out immediately.
    // Also, check that no invocations were recorded for the non-existent worker.
    let invocations = mock_client.get_invocations(non_existent_worker_id);
    assert!(
        invocations.is_empty(),
        "No invocations should be recorded for a non-existent worker"
    );
}

#[tokio::test]
async fn test_deliver_launch_event_success() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-launch-event-delivery";
    let worker_id = "instance-policy-worker-1"; // MockVmServiceClient default format

    attach_mock_vm(&runner_service, vm_id, mock_client.clone()).await;
    // Run a worker first
    let manifest = test_worker_manifest();
    let bundle = test_worker_bundle(b"launch code".to_vec());
    let launched_worker_id = runner_service
        .run_worker(vm_id.to_string(), manifest, bundle)
        .await
        .expect("Failed to run worker for launch event test");
    assert_eq!(launched_worker_id, worker_id);

    // Prepare a Launch event
    let event_payload = EventPayload::Launch;
    let handler_name = "handleLaunch".to_string();
    let vm_event_invocation_payload = serde_json::to_vec(&crate::runner::VmEventInvocation {
        handler: handler_name.clone(),
        event_payload: event_payload.clone(),
    })
    .unwrap();

    mock_client.set_worker_execution_behavior(
        &launched_worker_id,
        MockExecutionBehavior::Fixed(b"launch_ack".to_vec()),
    );
    // Deliver the event
    let result = runner_service
        .deliver_batch_events(vec![(
            launched_worker_id.clone(),
            handler_name,
            event_payload,
        )])
        .await;
    assert!(result.is_ok(), "deliver_batch_events for Launch failed");

    sleep(Duration::from_millis(200)).await;

    let invocations = mock_client.get_invocations(&launched_worker_id);
    assert_eq!(
        invocations.len(),
        1,
        "Mock VM should have received one invocation for Launch"
    );
    // The actual payload sent to the VM client's invoke_worker is VmEventInvocation serialized
    assert_eq!(
        invocations[0], vm_event_invocation_payload,
        "Launch event invocation payload mismatch"
    );
}
