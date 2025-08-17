use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::RwLock;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, warn};

const MAX_LINES: usize = 10_000;
const MAX_BYTES: usize = 10 * 1024; // 10 KiB
const DEAD_WORKER_RETENTION: Duration = Duration::from_secs(300); // 5 minutes

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub line: String,
    pub timestamp: Instant,
}

#[derive(Debug)]
pub struct LogBuffer {
    history: RwLock<VecDeque<LogEntry>>,
    current_bytes: RwLock<usize>,
    broadcast_sender: broadcast::Sender<LogEntry>,
}

#[derive(Debug)]
pub struct LogStreamer {
    receiver: broadcast::Receiver<LogEntry>,
}

impl LogBuffer {
    pub fn new() -> (Arc<Self>, LogStreamer) {
        let (broadcast_sender, receiver) = broadcast::channel(1000);

        let buffer = Arc::new(Self {
            history: RwLock::new(VecDeque::new()),
            current_bytes: RwLock::new(0),
            broadcast_sender,
        });

        let streamer = LogStreamer { receiver };

        (buffer, streamer)
    }

    pub fn write_log(&self, line: String) {
        let entry = LogEntry {
            line: line.clone(),
            timestamp: Instant::now(),
        };

        // Always update history (writer never blocks indefinitely)
        {
            let mut history = self.history.write();
            let mut current_bytes = self.current_bytes.write();

            // Enforce size limits
            while history.len() >= MAX_LINES || *current_bytes >= MAX_BYTES {
                if let Some(old_entry) = history.pop_front() {
                    *current_bytes = current_bytes.saturating_sub(old_entry.line.len());
                } else {
                    break;
                }
            }

            *current_bytes += line.len();
            history.push_back(entry.clone());
        } // Lock released immediately

        // Stream to all active clients (never blocks - if no receivers, message is dropped)
        if let Err(_) = self.broadcast_sender.send(entry) {
            // No receivers, which is fine
            debug!("No log stream receivers for log line");
        }
    }

    pub fn get_tail(&self, max_lines: Option<usize>) -> Vec<LogEntry> {
        let history = self.history.read();
        let lines_to_take = max_lines.unwrap_or(history.len()).min(history.len());

        if lines_to_take == 0 {
            return Vec::new();
        }

        history
            .iter()
            .skip(history.len() - lines_to_take)
            .cloned()
            .collect()
    }

    pub fn get_all_logs(&self) -> Vec<LogEntry> {
        self.history.read().iter().cloned().collect()
    }

    pub fn create_streamer(&self) -> LogStreamer {
        LogStreamer {
            receiver: self.broadcast_sender.subscribe(),
        }
    }
}

impl LogStreamer {
    pub async fn next_log(&mut self) -> Option<LogEntry> {
        match self.receiver.recv().await {
            Ok(entry) => Some(entry),
            Err(broadcast::error::RecvError::Closed) => None,
            Err(broadcast::error::RecvError::Lagged(_)) => {
                // If we lagged, try to receive the next message
                warn!("Log streamer lagged, may have missed some messages");
                match self.receiver.recv().await {
                    Ok(entry) => Some(entry),
                    Err(_) => None,
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct DeadWorkerLogs {
    pub worker_id: String,
    pub logs: VecDeque<LogEntry>,
    pub death_time: Instant,
}

impl DeadWorkerLogs {
    pub fn new(worker_id: String, logs: VecDeque<LogEntry>) -> Self {
        Self {
            worker_id,
            logs,
            death_time: Instant::now(),
        }
    }

    pub fn is_expired(&self) -> bool {
        self.death_time.elapsed() > DEAD_WORKER_RETENTION
    }

    pub fn get_tail(&self, max_lines: Option<usize>) -> Vec<LogEntry> {
        let lines_to_take = max_lines.unwrap_or(self.logs.len()).min(self.logs.len());

        if lines_to_take == 0 {
            return Vec::new();
        }

        self.logs
            .iter()
            .skip(self.logs.len() - lines_to_take)
            .cloned()
            .collect()
    }

    pub fn get_all_logs(&self) -> Vec<LogEntry> {
        self.logs.iter().cloned().collect()
    }
}

#[derive(Debug)]
pub struct VmmLogManager {
    active_workers: RwLock<HashMap<String, Arc<LogBuffer>>>,
    dead_worker_logs: RwLock<HashMap<String, DeadWorkerLogs>>,
}

impl VmmLogManager {
    pub fn new() -> Arc<Self> {
        let manager = Arc::new(Self {
            active_workers: RwLock::new(HashMap::new()),
            dead_worker_logs: RwLock::new(HashMap::new()),
        });

        // Start cleanup task for expired dead worker logs
        let manager_for_cleanup = Arc::downgrade(&manager);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;

                if let Some(manager) = manager_for_cleanup.upgrade() {
                    let mut dead_logs = manager.dead_worker_logs.write();
                    let initial_count = dead_logs.len();

                    dead_logs.retain(|worker_id, logs| {
                        let should_retain = !logs.is_expired();
                        if !should_retain {
                            debug!("Cleaning up expired dead worker logs for: {}", worker_id);
                        }
                        should_retain
                    });

                    let cleaned_count = initial_count - dead_logs.len();
                    if cleaned_count > 0 {
                        debug!(
                            "Cleaned up {} expired dead worker log entries",
                            cleaned_count
                        );
                    }
                } else {
                    // Manager was dropped, exit cleanup task
                    break;
                }
            }
        });

        manager
    }

    pub fn register_worker(&self, worker_id: String) -> Arc<LogBuffer> {
        let (buffer, _streamer) = LogBuffer::new();
        self.active_workers
            .write()
            .insert(worker_id, buffer.clone());
        buffer
    }

    pub fn terminate_worker(&self, worker_id: &str) {
        let mut active_workers = self.active_workers.write();
        if let Some(buffer) = active_workers.remove(worker_id) {
            let history = buffer.history.read().clone();
            let dead_logs = DeadWorkerLogs::new(worker_id.to_string(), history);

            self.dead_worker_logs
                .write()
                .insert(worker_id.to_string(), dead_logs);
            debug!("Moved worker logs to dead storage: {}", worker_id);
        }
    }

    pub fn get_worker_logs(
        &self,
        worker_id: &str,
        tail_lines: Option<usize>,
    ) -> Option<Vec<LogEntry>> {
        // Check active workers first
        if let Some(buffer) = self.active_workers.read().get(worker_id) {
            return Some(buffer.get_tail(tail_lines));
        }

        // Check dead workers
        if let Some(dead_logs) = self.dead_worker_logs.read().get(worker_id) {
            return Some(dead_logs.get_tail(tail_lines));
        }

        None
    }

    pub fn create_log_streamer(&self, worker_id: &str) -> Option<LogStreamer> {
        if let Some(buffer) = self.active_workers.read().get(worker_id) {
            Some(buffer.create_streamer())
        } else {
            None
        }
    }

    pub fn get_worker_buffer(&self, worker_id: &str) -> Option<Arc<LogBuffer>> {
        self.active_workers.read().get(worker_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::{Duration as TokioDuration, sleep};

    use super::*;

    #[tokio::test]
    async fn test_log_buffer_basic_functionality() {
        let (buffer, mut streamer) = LogBuffer::new();

        // Write a log entry
        buffer.write_log("test log line".to_string());

        // Should be able to get it from tail
        let tail = buffer.get_tail(Some(1));
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].line, "test log line");

        // Should be able to stream it
        let entry = streamer.next_log().await.unwrap();
        assert_eq!(entry.line, "test log line");
    }

    #[tokio::test]
    async fn test_log_buffer_size_limits() {
        let (buffer, _streamer) = LogBuffer::new();

        // Write more than MAX_LINES
        for i in 0..MAX_LINES + 100 {
            buffer.write_log(format!("log line {}", i));
        }

        let all_logs = buffer.get_all_logs();
        assert!(all_logs.len() <= MAX_LINES);

        // Should contain the most recent logs
        assert!(
            all_logs
                .last()
                .unwrap()
                .line
                .contains(&format!("{}", MAX_LINES + 99))
        );
    }

    #[tokio::test]
    async fn test_dead_worker_logs() {
        let (buffer, _streamer) = LogBuffer::new();

        // Add some logs
        buffer.write_log("log 1".to_string());
        buffer.write_log("log 2".to_string());

        // Create dead worker logs
        let history = buffer.history.read().clone();
        let dead_logs = DeadWorkerLogs::new("worker-123".to_string(), history);

        assert_eq!(dead_logs.worker_id, "worker-123");
        assert_eq!(dead_logs.logs.len(), 2);
        assert!(!dead_logs.is_expired());

        let tail = dead_logs.get_tail(Some(1));
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].line, "log 2");
    }

    #[tokio::test]
    async fn test_concurrent_write_and_read() {
        let (buffer, mut streamer) = LogBuffer::new();
        let buffer_clone = buffer.clone();

        // Start writing logs in background
        let write_handle = tokio::spawn(async move {
            for i in 0..100 {
                buffer_clone.write_log(format!("concurrent log {}", i));
                sleep(TokioDuration::from_millis(1)).await;
            }
        });

        // Read logs concurrently
        let mut received_count = 0;
        while received_count < 100 {
            if let Some(_entry) = streamer.next_log().await {
                received_count += 1;
            }
        }

        write_handle.await.unwrap();
        assert_eq!(received_count, 100);
    }

    #[tokio::test]
    async fn test_vmm_log_manager_basic_operations() {
        let manager = VmmLogManager::new();

        // Register a worker
        let buffer = manager.register_worker("worker-1".to_string());

        // Write some logs
        buffer.write_log("Hello from worker 1".to_string());
        buffer.write_log("Second log message".to_string());

        // Get logs via manager
        let logs = manager.get_worker_logs("worker-1", None).unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].line, "Hello from worker 1");
        assert_eq!(logs[1].line, "Second log message");

        // Test tail functionality
        let tail_logs = manager.get_worker_logs("worker-1", Some(1)).unwrap();
        assert_eq!(tail_logs.len(), 1);
        assert_eq!(tail_logs[0].line, "Second log message");
    }

    #[tokio::test]
    async fn test_vmm_log_manager_worker_termination() {
        let manager = VmmLogManager::new();

        // Register and populate worker
        let buffer = manager.register_worker("worker-2".to_string());
        buffer.write_log("Before termination".to_string());

        // Verify worker is active
        assert!(manager.get_worker_logs("worker-2", None).is_some());
        assert!(manager.create_log_streamer("worker-2").is_some());

        // Terminate worker
        manager.terminate_worker("worker-2");

        // Logs should still be accessible
        let logs = manager.get_worker_logs("worker-2", None).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].line, "Before termination");

        // But streaming should not be available
        assert!(manager.create_log_streamer("worker-2").is_none());
    }

    #[tokio::test]
    async fn test_vmm_log_manager_multiple_workers() {
        let manager = VmmLogManager::new();

        // Register multiple workers
        let buffer1 = manager.register_worker("worker-1".to_string());
        let buffer2 = manager.register_worker("worker-2".to_string());

        // Write different logs to each
        buffer1.write_log("Worker 1 log".to_string());
        buffer2.write_log("Worker 2 log".to_string());

        // Verify isolation
        let logs1 = manager.get_worker_logs("worker-1", None).unwrap();
        let logs2 = manager.get_worker_logs("worker-2", None).unwrap();

        assert_eq!(logs1.len(), 1);
        assert_eq!(logs2.len(), 1);
        assert_eq!(logs1[0].line, "Worker 1 log");
        assert_eq!(logs2[0].line, "Worker 2 log");

        // Non-existent worker should return None
        assert!(manager.get_worker_logs("worker-3", None).is_none());
    }

    #[tokio::test]
    async fn test_log_streamer_functionality() {
        let manager = VmmLogManager::new();
        let buffer = manager.register_worker("streamer-test".to_string());

        // Create a streamer
        let mut streamer = manager.create_log_streamer("streamer-test").unwrap();

        // Write logs after creating streamer
        buffer.write_log("Stream message 1".to_string());
        buffer.write_log("Stream message 2".to_string());

        // Should receive the logs
        let log1 = streamer.next_log().await.unwrap();
        let log2 = streamer.next_log().await.unwrap();

        assert_eq!(log1.line, "Stream message 1");
        assert_eq!(log2.line, "Stream message 2");
    }

    #[tokio::test]
    async fn test_log_buffer_byte_limits() {
        let (buffer, _streamer) = LogBuffer::new();

        // Write logs that exceed byte limit
        let large_log = "x".repeat(MAX_BYTES / 2);
        buffer.write_log(large_log.clone());
        buffer.write_log(large_log.clone());
        buffer.write_log("small".to_string());

        let logs = buffer.get_all_logs();

        // Should have cleaned up old logs to stay under byte limit
        let total_bytes: usize = logs.iter().map(|entry| entry.line.len()).sum();
        assert!(total_bytes <= MAX_BYTES);

        // Should still have the latest log
        assert!(logs.iter().any(|entry| entry.line == "small"));
    }

    #[tokio::test]
    async fn test_multiple_streamers_same_worker() {
        let manager = VmmLogManager::new();
        let buffer = manager.register_worker("multi-stream-test".to_string());

        // Create multiple streamers
        let mut streamer1 = manager.create_log_streamer("multi-stream-test").unwrap();
        let mut streamer2 = manager.create_log_streamer("multi-stream-test").unwrap();

        // Write a log
        buffer.write_log("Multi-stream message".to_string());

        // Both streamers should receive it
        let log1 = streamer1.next_log().await.unwrap();
        let log2 = streamer2.next_log().await.unwrap();

        assert_eq!(log1.line, "Multi-stream message");
        assert_eq!(log2.line, "Multi-stream message");
    }

    #[tokio::test]
    async fn test_dead_worker_log_expiration() {
        // Create dead logs with custom death time
        let logs = VecDeque::from([LogEntry {
            line: "test log".to_string(),
            timestamp: Instant::now(),
        }]);

        let mut dead_logs = DeadWorkerLogs::new("test-worker".to_string(), logs);

        // Manually set death time to past
        dead_logs.death_time = Instant::now() - Duration::from_secs(400); // 6+ minutes ago

        // Should be expired
        assert!(dead_logs.is_expired());

        // Create fresh dead logs
        let fresh_logs = VecDeque::from([LogEntry {
            line: "fresh log".to_string(),
            timestamp: Instant::now(),
        }]);
        let fresh_dead_logs = DeadWorkerLogs::new("fresh-worker".to_string(), fresh_logs);

        // Should not be expired
        assert!(!fresh_dead_logs.is_expired());
    }
}
