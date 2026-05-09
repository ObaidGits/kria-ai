//! Preemption Manager — Priority-based task interruption.
//!
//! When a voice command (P0) arrives while a background task (P3/P4) is running:
//! 1. Signal cancellation via CancellationToken
//! 2. Wait up to `grace_period` for graceful shutdown
//! 3. Force-kill (abort) if not stopped in time
//! 4. The GPU lease guard is dropped with the task, freeing VRAM

use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

use super::types::TaskResult;

pub struct PreemptionManager {
    grace_period: Duration,
}

impl PreemptionManager {
    pub fn new(grace_period: Duration) -> Self {
        Self { grace_period }
    }

    /// Wait for a task to complete after cancellation, up to the grace period.
    /// If the task doesn't stop in time, it will be aborted when the JoinHandle is dropped.
    pub async fn wait_for_grace(&self, join: &mut JoinHandle<TaskResult>) {
        let deadline = Instant::now() + self.grace_period;

        while !join.is_finished() && Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        if !join.is_finished() {
            tracing::warn!(
                grace_ms = self.grace_period.as_millis(),
                "Task did not stop within grace period"
            );
        }
    }

    /// Get the configured grace period.
    pub fn grace_period(&self) -> Duration {
        self.grace_period
    }
}
