use std::{
    collections::{BinaryHeap, HashMap},
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use nxcc_interface::types::worker::events::{CatchUp, RateMode, Schedule};
use tokio::{
    sync::{RwLock, mpsc},
    time::{MissedTickBehavior, interval_at, sleep},
};
use tracing::{debug, error, warn};

use crate::config::SchedulerConfig;

pub type WorkOrderId = String;
pub type HandlerName = String;

/// A scheduled event that needs to be triggered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledEvent {
    pub work_order_id: WorkOrderId,
    pub handler: HandlerName,
    pub next_fire_time: Instant,
    pub schedule: Schedule,
    /// Tracks how many times this event has been triggered (for max_occurrences)
    pub occurrence_count: u64,
}

impl PartialOrd for ScheduledEvent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledEvent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse order for min-heap behavior (earliest times first)
        other.next_fire_time.cmp(&self.next_fire_time)
    }
}

/// Command sent to the scheduler service.
#[derive(Debug)]
pub enum SchedulerCommand {
    /// Add a new scheduled event
    AddScheduledEvent {
        work_order_id: WorkOrderId,
        handler: HandlerName,
        schedule: Schedule,
    },
    /// Remove all scheduled events for a work order
    RemoveWorkOrder { work_order_id: WorkOrderId },
    /// Stop the scheduler
    Stop,
}

/// Event fired by the scheduler when a scheduled event should be triggered.
#[derive(Debug, Clone)]
pub struct ScheduledEventFired {
    pub work_order_id: WorkOrderId,
    pub handler: HandlerName,
}

/// Validates a schedule against the daemon's configuration.
pub fn validate_schedule(
    schedule: &Schedule,
    config: &SchedulerConfig,
) -> Result<(), SchedulerError> {
    match schedule {
        Schedule::Rate(rate_mode) => {
            if rate_mode.period_ms < config.min_schedule_interval_ms {
                return Err(SchedulerError::IntervalTooFast {
                    requested_ms: rate_mode.period_ms,
                    minimum_ms: config.min_schedule_interval_ms,
                });
            }

            // Validate that start_at is not in the past (if specified)
            if let Some(start_at) = &rate_mode.start_at {
                if *start_at < Utc::now() {
                    return Err(SchedulerError::StartTimeInPast {
                        start_time: *start_at,
                    });
                }
            }

            // Validate that end_at is after start_at (if both specified)
            if let (Some(start_at), Some(end_at)) = (&rate_mode.start_at, &rate_mode.end_at) {
                if end_at <= start_at {
                    return Err(SchedulerError::EndTimeBeforeStart {
                        start_time: *start_at,
                        end_time: *end_at,
                    });
                }
            }

            // Validate that max_occurrences is reasonable (if specified)
            if let Some(max_occurrences) = rate_mode.max_occurrences {
                if max_occurrences == 0 {
                    return Err(SchedulerError::ZeroMaxOccurrences);
                }
            }

            Ok(())
        }
    }
}

/// Error types for scheduler operations.
#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error("Schedule interval {requested_ms}ms is too fast, minimum is {minimum_ms}ms")]
    IntervalTooFast { requested_ms: u64, minimum_ms: u64 },
    #[error("Schedule start time {start_time} is in the past")]
    StartTimeInPast { start_time: DateTime<Utc> },
    #[error("Schedule end time {end_time} is before start time {start_time}")]
    EndTimeBeforeStart {
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    },
    #[error("max_occurrences cannot be zero")]
    ZeroMaxOccurrences,
}

/// The scheduler service manages scheduled events and fires them at the appropriate times.
pub struct SchedulerService {
    config: SchedulerConfig,
    /// Command receiver for adding/removing scheduled events
    command_rx: mpsc::UnboundedReceiver<SchedulerCommand>,
    /// Event sender for notifying when scheduled events should be triggered
    event_tx: mpsc::UnboundedSender<ScheduledEventFired>,
    /// Priority queue of scheduled events (min-heap by next_fire_time)
    scheduled_events: BinaryHeap<ScheduledEvent>,
    /// Map from work_order_id to the set of scheduled events for that work order
    work_order_events: HashMap<WorkOrderId, Vec<usize>>,
}

impl SchedulerService {
    /// Create a new scheduler service.
    pub fn new(
        config: SchedulerConfig,
        command_rx: mpsc::UnboundedReceiver<SchedulerCommand>,
        event_tx: mpsc::UnboundedSender<ScheduledEventFired>,
    ) -> Self {
        Self {
            config,
            command_rx,
            event_tx,
            scheduled_events: BinaryHeap::new(),
            work_order_events: HashMap::new(),
        }
    }

    /// Run the scheduler service.
    pub async fn run(mut self) {
        debug!("Scheduler service starting");

        loop {
            // Calculate how long to wait until the next event
            let wait_duration = if let Some(next_event) = self.scheduled_events.peek() {
                let now = Instant::now();
                if next_event.next_fire_time <= now {
                    // Event is ready to fire
                    Duration::from_millis(0)
                } else {
                    next_event.next_fire_time.duration_since(now)
                }
            } else {
                // No events scheduled, wait a reasonable amount
                Duration::from_secs(1)
            };

            // Wait for either a command or timeout
            let result = tokio::time::timeout(wait_duration, self.command_rx.recv()).await;

            match result {
                Ok(Some(command)) => {
                    if let Err(e) = self.handle_command(command).await {
                        error!("Error handling scheduler command: {}", e);
                    }
                }
                Ok(None) => {
                    debug!("Scheduler command channel closed, stopping");
                    break;
                }
                Err(_) => {
                    // Timeout - check for events to fire
                    self.fire_ready_events().await;
                }
            }
        }

        debug!("Scheduler service stopped");
    }

    async fn handle_command(&mut self, command: SchedulerCommand) -> Result<(), SchedulerError> {
        match command {
            SchedulerCommand::AddScheduledEvent {
                work_order_id,
                handler,
                schedule,
            } => {
                validate_schedule(&schedule, &self.config)?;
                self.add_scheduled_event(work_order_id, handler, schedule);
            }
            SchedulerCommand::RemoveWorkOrder { work_order_id } => {
                self.remove_work_order(&work_order_id);
            }
            SchedulerCommand::Stop => {
                debug!("Received stop command");
                return Ok(());
            }
        }
        Ok(())
    }

    fn add_scheduled_event(
        &mut self,
        work_order_id: WorkOrderId,
        handler: HandlerName,
        schedule: Schedule,
    ) {
        let now = Instant::now();
        let next_fire_time = self.calculate_next_fire_time(&schedule, now);

        let event = ScheduledEvent {
            work_order_id: work_order_id.clone(),
            handler,
            next_fire_time,
            schedule,
            occurrence_count: 0,
        };

        debug!(
            "Adding scheduled event for work_order {} at {:?}",
            work_order_id, next_fire_time
        );

        // Add to the heap
        self.scheduled_events.push(event);

        // Track in work_order_events map
        self.work_order_events
            .entry(work_order_id)
            .or_insert_with(Vec::new)
            .push(self.scheduled_events.len() - 1);
    }

    fn remove_work_order(&mut self, work_order_id: &WorkOrderId) {
        debug!(
            "Removing all scheduled events for work_order {}",
            work_order_id
        );

        // Remove from work_order_events map
        self.work_order_events.remove(work_order_id);

        // Remove from heap by reconstructing it without the work order's events
        let mut new_heap = BinaryHeap::new();
        while let Some(event) = self.scheduled_events.pop() {
            if event.work_order_id != *work_order_id {
                new_heap.push(event);
            }
        }
        self.scheduled_events = new_heap;
    }

    async fn fire_ready_events(&mut self) {
        let now = Instant::now();

        // Collect events that are ready to fire
        let mut events_to_fire = Vec::new();
        let mut events_to_reschedule = Vec::new();

        while let Some(event) = self.scheduled_events.peek() {
            if event.next_fire_time > now {
                break; // No more ready events
            }

            let mut event = self.scheduled_events.pop().unwrap();

            // Check if event should still fire (work order might have been removed)
            if !self.work_order_events.contains_key(&event.work_order_id) {
                continue;
            }

            events_to_fire.push(ScheduledEventFired {
                work_order_id: event.work_order_id.clone(),
                handler: event.handler.clone(),
            });

            event.occurrence_count += 1;

            // Check if we should reschedule this event
            if self.should_reschedule(&event) {
                let next_fire_time = self.calculate_next_fire_time(&event.schedule, now);
                event.next_fire_time = next_fire_time;
                events_to_reschedule.push(event);
            }
        }

        // Reschedule recurring events
        for event in events_to_reschedule {
            self.scheduled_events.push(event);
        }

        // Fire the events
        for event_fired in events_to_fire {
            debug!(
                "Firing scheduled event for work_order {} handler {}",
                event_fired.work_order_id, event_fired.handler
            );

            if let Err(e) = self.event_tx.send(event_fired) {
                error!("Failed to send scheduled event: {}", e);
            }
        }
    }

    fn should_reschedule(&self, event: &ScheduledEvent) -> bool {
        match &event.schedule {
            Schedule::Rate(rate_mode) => {
                // Check max_occurrences limit
                if let Some(max_occurrences) = rate_mode.max_occurrences {
                    if event.occurrence_count >= max_occurrences {
                        return false;
                    }
                }

                // Check end_at time
                if let Some(end_at) = &rate_mode.end_at {
                    if Utc::now() >= *end_at {
                        return false;
                    }
                }

                true
            }
        }
    }

    fn calculate_next_fire_time(&self, schedule: &Schedule, reference_time: Instant) -> Instant {
        match schedule {
            Schedule::Rate(rate_mode) => {
                self.calculate_rate_mode_next_fire_time(rate_mode, reference_time)
            }
        }
    }

    fn calculate_rate_mode_next_fire_time(
        &self,
        rate_mode: &RateMode,
        reference_time: Instant,
    ) -> Instant {
        let now_utc = Utc::now();

        // If start_at is specified and we're before it, use start_at
        if let Some(start_at) = &rate_mode.start_at {
            if now_utc < *start_at {
                // Convert UTC time to Instant
                let chrono_duration = *start_at - now_utc;
                let duration_until_start =
                    chrono_duration.to_std().unwrap_or(Duration::from_secs(0));
                return reference_time
                    + duration_until_start
                    + Duration::from_millis(rate_mode.phase_ms);
            }
        }

        // For immediate start or if we're past start_at, schedule for the next period
        reference_time + Duration::from_millis(rate_mode.period_ms)
    }
}

/// Handle for communicating with the scheduler service.
pub struct SchedulerHandle {
    command_tx: mpsc::UnboundedSender<SchedulerCommand>,
    event_rx: Arc<RwLock<Option<mpsc::UnboundedReceiver<ScheduledEventFired>>>>,
}

impl SchedulerHandle {
    /// Create a new scheduler handle and spawn the scheduler service.
    pub fn new(config: SchedulerConfig) -> Self {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let scheduler = SchedulerService::new(config, command_rx, event_tx);

        // Spawn the scheduler service
        tokio::spawn(async move {
            scheduler.run().await;
        });

        Self {
            command_tx,
            event_rx: Arc::new(RwLock::new(Some(event_rx))),
        }
    }

    /// Add a scheduled event.
    pub async fn add_scheduled_event(
        &self,
        work_order_id: WorkOrderId,
        handler: HandlerName,
        schedule: Schedule,
    ) -> Result<(), String> {
        self.command_tx
            .send(SchedulerCommand::AddScheduledEvent {
                work_order_id,
                handler,
                schedule,
            })
            .map_err(|e| format!("Failed to send add command: {}", e))
    }

    /// Remove all scheduled events for a work order.
    pub async fn remove_work_order(&self, work_order_id: WorkOrderId) -> Result<(), String> {
        self.command_tx
            .send(SchedulerCommand::RemoveWorkOrder { work_order_id })
            .map_err(|e| format!("Failed to send remove command: {}", e))
    }

    /// Get the event receiver (can only be called once).
    pub async fn take_event_receiver(
        &self,
    ) -> Option<mpsc::UnboundedReceiver<ScheduledEventFired>> {
        let mut event_rx_guard = self.event_rx.write().await;
        event_rx_guard.take()
    }

    /// Stop the scheduler service.
    pub async fn stop(&self) -> Result<(), String> {
        self.command_tx
            .send(SchedulerCommand::Stop)
            .map_err(|e| format!("Failed to send stop command: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration as ChronoDuration;

    use super::*;

    #[test]
    fn test_validate_schedule_period_too_fast() {
        let config = SchedulerConfig {
            min_schedule_interval_ms: 100,
        };

        let schedule = Schedule::Rate(RateMode::new(50)); // 50ms < 100ms minimum
        let result = validate_schedule(&schedule, &config);

        assert!(matches!(
            result,
            Err(SchedulerError::IntervalTooFast {
                requested_ms: 50,
                minimum_ms: 100
            })
        ));
    }

    #[test]
    fn test_validate_schedule_valid() {
        let config = SchedulerConfig {
            min_schedule_interval_ms: 100,
        };

        let schedule = Schedule::Rate(RateMode::new(200)); // 200ms > 100ms minimum
        let result = validate_schedule(&schedule, &config);

        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_schedule_start_time_in_past() {
        let config = SchedulerConfig {
            min_schedule_interval_ms: 100,
        };

        let past_time = Utc::now() - ChronoDuration::hours(1);
        let mut rate_mode = RateMode::new(200);
        rate_mode.start_at = Some(past_time);
        let schedule = Schedule::Rate(rate_mode);

        let result = validate_schedule(&schedule, &config);
        assert!(matches!(
            result,
            Err(SchedulerError::StartTimeInPast { .. })
        ));
    }

    #[test]
    fn test_validate_schedule_end_before_start() {
        let config = SchedulerConfig {
            min_schedule_interval_ms: 100,
        };

        let start_time = Utc::now() + ChronoDuration::hours(2);
        let end_time = Utc::now() + ChronoDuration::hours(1); // end before start

        let mut rate_mode = RateMode::new(200);
        rate_mode.start_at = Some(start_time);
        rate_mode.end_at = Some(end_time);
        let schedule = Schedule::Rate(rate_mode);

        let result = validate_schedule(&schedule, &config);
        assert!(matches!(
            result,
            Err(SchedulerError::EndTimeBeforeStart { .. })
        ));
    }

    #[test]
    fn test_validate_schedule_zero_max_occurrences() {
        let config = SchedulerConfig {
            min_schedule_interval_ms: 100,
        };

        let mut rate_mode = RateMode::new(200);
        rate_mode.max_occurrences = Some(0);
        let schedule = Schedule::Rate(rate_mode);

        let result = validate_schedule(&schedule, &config);
        assert!(matches!(result, Err(SchedulerError::ZeroMaxOccurrences)));
    }

    #[tokio::test]
    async fn test_scheduler_handle_basic_operations() {
        let config = SchedulerConfig {
            min_schedule_interval_ms: 10,
        };

        let handle = SchedulerHandle::new(config);
        let mut event_rx = handle.take_event_receiver().await.unwrap();

        // Add a scheduled event
        let result = handle
            .add_scheduled_event(
                "test-work-order".to_string(),
                "test-handler".to_string(),
                Schedule::Rate(RateMode::new(50)),
            )
            .await;
        assert!(result.is_ok());

        // Wait a bit and check if event fires
        tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
            .await
            .expect("Should receive scheduled event")
            .expect("Event should be Some");

        // Remove the work order
        let result = handle
            .remove_work_order("test-work-order".to_string())
            .await;
        assert!(result.is_ok());

        // Stop the scheduler
        let result = handle.stop().await;
        assert!(result.is_ok());
    }
}
