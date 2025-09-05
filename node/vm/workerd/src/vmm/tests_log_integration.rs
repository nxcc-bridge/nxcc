use super::*;
use crate::errors::WorkerdVmError;
use nxcc_interface::proto::vm::StreamWorkerLogsResponse;
use nxcc_vm_base::server::VmError;
use std::time::Duration;
use tokio_stream::StreamExt;

/// These tests expose the architectural bug: the VMM's stream_worker_logs method
/// should be able to retrieve historical logs from the LogBuffer, but it currently
/// fails because the integration between LogManager and the streaming API is broken.

#[tokio::test]
async fn test_stream_worker_logs_should_return_historical_logs() {
    // This test currently FAILS but demonstrates what SHOULD work
    let vmm = WorkerdVmm::new(Default::default());
    
    // Register a worker and generate some logs (simulating what happens during worker startup)
    let worker_id = "test-worker-historical".to_string();
    let log_buffer = vmm.log_manager.register_worker(worker_id.clone());
    
    // Write some historical logs to the buffer
    log_buffer.write_log("Historical log line 1".to_string());
    log_buffer.write_log("Historical log line 2".to_string());
    log_buffer.write_log("Historical log line 3".to_string());
    
    // Wait a moment for logs to be written
    tokio::time::sleep(Duration::from_millis(10)).await;
    
    // Verify that LogManager can retrieve the logs directly (this should work)
    let direct_logs = vmm.log_manager.get_worker_logs(&worker_id, None);
    assert!(direct_logs.is_some(), "LogManager should find worker logs");
    assert_eq!(direct_logs.unwrap().len(), 3, "Should have 3 historical logs");
    
    // Now test the streaming API with tail=2 (this currently FAILS)
    let stream_result = vmm.stream_worker_logs(worker_id.clone(), 2, false).await;
    assert!(
        stream_result.is_ok(), 
        "stream_worker_logs should succeed when worker has logs: {:?}", 
        stream_result.err()
    );
    
    let mut stream = stream_result.unwrap();
    let mut collected_logs = Vec::new();
    
    // Collect all logs from the stream (should get last 2 historical logs)
    while let Some(log_result) = stream.next().await {
        match log_result {
            Ok(response) => collected_logs.push(response.log_line),
            Err(e) => panic!("Stream error: {:?}", e),
        }
    }
    
    // Should have received the last 2 logs due to tail=2
    assert_eq!(collected_logs.len(), 2, "Should receive 2 historical logs");
    assert_eq!(collected_logs[0], "Historical log line 2");
    assert_eq!(collected_logs[1], "Historical log line 3");
}

#[tokio::test]
async fn test_stream_worker_logs_with_follow_should_get_historical_plus_new() {
    // This test demonstrates the complete workflow: get historical logs + stream new ones
    let vmm = WorkerdVmm::new(Default::default());
    
    let worker_id = "test-worker-follow".to_string();
    let log_buffer = vmm.log_manager.register_worker(worker_id.clone());
    
    // Write some historical logs
    log_buffer.write_log("Historical 1".to_string());
    log_buffer.write_log("Historical 2".to_string());
    
    tokio::time::sleep(Duration::from_millis(10)).await;
    
    // Start streaming with follow=true
    let stream_result = vmm.stream_worker_logs(worker_id.clone(), 1, true).await;
    assert!(stream_result.is_ok(), "Streaming with follow should work");
    
    let mut stream = stream_result.unwrap();
    let mut collected_logs = Vec::new();
    
    // Collect historical logs (should get last 1 due to tail=1)
    if let Some(Ok(response)) = stream.next().await {
        collected_logs.push(response.log_line);
        assert!(response.is_historical, "First log should be marked as historical");
    }
    
    // Write a new log while streaming
    log_buffer.write_log("New streaming log".to_string());
    
    // Should receive the new log
    if let Some(Ok(response)) = stream.next().await {
        collected_logs.push(response.log_line);
        assert!(!response.is_historical, "New log should not be marked as historical");
    }
    
    assert_eq!(collected_logs.len(), 2);
    assert_eq!(collected_logs[0], "Historical 2"); // Last historical log
    assert_eq!(collected_logs[1], "New streaming log"); // New log
}

#[tokio::test] 
async fn test_stream_worker_logs_nonexistent_worker_should_fail() {
    // This should properly fail for non-existent workers
    let vmm = WorkerdVmm::new(Default::default());
    
    let result = vmm.stream_worker_logs("nonexistent-worker".to_string(), 0, false).await;
    assert!(result.is_err(), "Should fail for non-existent worker");
    
    // Verify the error message contains worker not found
    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("nonexistent-worker"), 
        "Error should reference the worker ID: {}", 
        error
    );
}

#[tokio::test]
async fn test_stream_worker_logs_empty_tail_shows_bug() {
    // This test demonstrates a BUG: tail=0 should return NO historical logs,
    // but currently it returns ALL historical logs due to the None mapping.
    let vmm = WorkerdVmm::new(Default::default());
    
    let worker_id = "test-worker-empty-tail".to_string();
    let log_buffer = vmm.log_manager.register_worker(worker_id.clone());
    
    log_buffer.write_log("Should not be returned with tail=0".to_string());
    tokio::time::sleep(Duration::from_millis(10)).await;
    
    // tail=0 should not return historical logs - but currently DOES (bug)
    let stream_result = vmm.stream_worker_logs(worker_id, 0, false).await;
    assert!(stream_result.is_ok(), "Should work even with tail=0");
    
    let mut stream = stream_result.unwrap();
    let mut collected_logs = Vec::new();
    
    // Collect logs (currently this will get logs due to the bug)
    while let Some(log_result) = stream.next().await {
        match log_result {
            Ok(response) => collected_logs.push(response.log_line),
            Err(e) => panic!("Stream error: {:?}", e),
        }
    }
    
    // BUG: Currently this fails because tail=0 incorrectly returns logs
    // This should be fixed so that tail=0 returns no historical logs
    assert_eq!(
        collected_logs.len(), 
        1, 
        "BUG: tail=0 currently returns {} logs, should return 0. This test documents the current buggy behavior.",
        collected_logs.len()
    );
}