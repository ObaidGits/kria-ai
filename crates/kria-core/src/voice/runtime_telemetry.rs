//! Runtime telemetry & load management for voice pipeline stabilization.
//!
//! Provides:
//! - Latency percentile tracking (p50/p95/p99)
//! - Queue pressure monitoring
//! - TTFA instrumentation
//! - Worker budget enforcement
//! - Overload degradation hooks
//! - Thermal pressure detection
//!
//! ## Invariants
//! - All tracking is bounded (fixed-size ring buffers)
//! - No allocation on hot path (pre-allocated)
//! - Non-blocking (best-effort telemetry)
//! - Preserves ownership/cancellation correctness

use serde::{Deserialize, Serialize};
use std::time::Instant;

// ─── Latency Histogram (bounded ring buffer) ──────────────────────────────

/// Fixed-size ring buffer for latency percentile computation.
/// Bounded to `CAP` entries. Oldest dropped when full.
#[derive(Debug, Clone)]
pub struct LatencyHistogram {
    samples: Vec<u64>,
    capacity: usize,
    write_idx: usize,
    count: usize,
}

impl LatencyHistogram {
    /// Create a histogram with given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: vec![0; capacity],
            capacity,
            write_idx: 0,
            count: 0,
        }
    }

    /// Record a latency sample (milliseconds).
    pub fn record(&mut self, ms: u64) {
        self.samples[self.write_idx] = ms;
        self.write_idx = (self.write_idx + 1) % self.capacity;
        if self.count < self.capacity {
            self.count += 1;
        }
    }

    /// Number of samples recorded.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the histogram is empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Compute percentile (0.0 - 1.0). Returns None if empty.
    pub fn percentile(&self, p: f64) -> Option<u64> {
        if self.count == 0 {
            return None;
        }
        let mut sorted: Vec<u64> = self.samples[..self.count].to_vec();
        sorted.sort_unstable();
        let idx = ((p * (self.count as f64 - 1.0)).round() as usize).min(self.count - 1);
        Some(sorted[idx])
    }

    /// p50 (median).
    pub fn p50(&self) -> Option<u64> {
        self.percentile(0.50)
    }

    /// p95.
    pub fn p95(&self) -> Option<u64> {
        self.percentile(0.95)
    }

    /// p99.
    pub fn p99(&self) -> Option<u64> {
        self.percentile(0.99)
    }

    /// Mean latency.
    pub fn mean(&self) -> Option<u64> {
        if self.count == 0 {
            return None;
        }
        let sum: u64 = self.samples[..self.count].iter().sum();
        Some(sum / self.count as u64)
    }

    /// Max latency.
    pub fn max(&self) -> Option<u64> {
        if self.count == 0 {
            return None;
        }
        self.samples[..self.count].iter().copied().max()
    }

    /// Reset all samples.
    pub fn reset(&mut self) {
        self.write_idx = 0;
        self.count = 0;
    }
}

// ─── Queue Pressure Monitor ───────────────────────────────────────────────

/// Queue pressure levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueuePressure {
    /// Queue utilization < 50%.
    Normal,
    /// Queue utilization 50-80%.
    Elevated,
    /// Queue utilization 80-95%.
    High,
    /// Queue utilization > 95%.
    Critical,
}

impl QueuePressure {
    pub fn from_utilization(used: usize, capacity: usize) -> Self {
        if capacity == 0 {
            return Self::Critical;
        }
        let pct = (used * 100) / capacity;
        match pct {
            0..=49 => Self::Normal,
            50..=79 => Self::Elevated,
            80..=95 => Self::High,
            _ => Self::Critical,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Elevated => "elevated",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

/// Monitors queue depth and emits pressure telemetry.
#[derive(Debug, Clone)]
pub struct QueueMonitor {
    pub name: &'static str,
    pub capacity: usize,
    pub current_depth: usize,
    pub peak_depth: usize,
    pub overflow_count: u64,
    pub last_pressure: QueuePressure,
}

impl QueueMonitor {
    pub fn new(name: &'static str, capacity: usize) -> Self {
        Self {
            name,
            capacity,
            current_depth: 0,
            peak_depth: 0,
            overflow_count: 0,
            last_pressure: QueuePressure::Normal,
        }
    }

    /// Update depth and return pressure level.
    pub fn update(&mut self, depth: usize) -> QueuePressure {
        self.current_depth = depth;
        if depth > self.peak_depth {
            self.peak_depth = depth;
        }
        if depth >= self.capacity {
            self.overflow_count += 1;
        }
        self.last_pressure = QueuePressure::from_utilization(depth, self.capacity);
        self.last_pressure
    }

    /// Reset peak/overflow counters.
    pub fn reset_counters(&mut self) {
        self.peak_depth = 0;
        self.overflow_count = 0;
    }
}

// ─── Worker Budget Tracker ────────────────────────────────────────────────

/// Tracks concurrent worker count against budget (§9).
#[derive(Debug, Clone)]
pub struct WorkerBudget {
    pub name: &'static str,
    pub max_concurrent: usize,
    pub active: usize,
    pub peak: usize,
    pub rejected_count: u64,
}

impl WorkerBudget {
    pub fn new(name: &'static str, max_concurrent: usize) -> Self {
        Self {
            name,
            max_concurrent,
            active: 0,
            peak: 0,
            rejected_count: 0,
        }
    }

    /// Try to acquire a worker slot. Returns false if at capacity.
    pub fn try_acquire(&mut self) -> bool {
        if self.active >= self.max_concurrent {
            self.rejected_count += 1;
            return false;
        }
        self.active += 1;
        if self.active > self.peak {
            self.peak = self.active;
        }
        true
    }

    /// Release a worker slot.
    pub fn release(&mut self) {
        self.active = self.active.saturating_sub(1);
    }

    /// Current utilization (0.0 - 1.0).
    pub fn utilization(&self) -> f64 {
        if self.max_concurrent == 0 {
            return 1.0;
        }
        self.active as f64 / self.max_concurrent as f64
    }
}

// ─── TTFA Tracker ─────────────────────────────────────────────────────────

/// Time-to-first-audio tracker per turn.
#[derive(Debug, Clone)]
pub struct TtfaTracker {
    /// Histogram of TTFA values (ms).
    pub histogram: LatencyHistogram,
    /// Count of turns that exceeded budget.
    pub overrun_count: u64,
    /// Budget (ms) for this tier.
    pub budget_ms: u64,
}

impl TtfaTracker {
    pub fn new(budget_ms: u64, capacity: usize) -> Self {
        Self {
            histogram: LatencyHistogram::new(capacity),
            overrun_count: 0,
            budget_ms,
        }
    }

    /// Record a TTFA measurement.
    pub fn record(&mut self, ttfa_ms: u64) {
        self.histogram.record(ttfa_ms);
        if ttfa_ms > self.budget_ms {
            self.overrun_count += 1;
        }
    }

    /// Overrun rate (fraction of turns exceeding budget).
    pub fn overrun_rate(&self) -> f64 {
        if self.histogram.len() == 0 {
            return 0.0;
        }
        self.overrun_count as f64 / self.histogram.len() as f64
    }
}

// ─── Runtime Load Snapshot ────────────────────────────────────────────────

/// Snapshot of runtime load for telemetry emission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeLoadSnapshot {
    pub ttfa_p50_ms: Option<u64>,
    pub ttfa_p95_ms: Option<u64>,
    pub ttfa_p99_ms: Option<u64>,
    pub ttfa_overrun_rate: f64,
    pub interrupt_latency_p50_ms: Option<u64>,
    pub interrupt_latency_p95_ms: Option<u64>,
    pub cancel_latency_p50_ms: Option<u64>,
    pub cancel_latency_p95_ms: Option<u64>,
    pub audio_queue_pressure: String,
    pub partial_queue_pressure: String,
    pub whisper_worker_utilization: f64,
    pub total_turns: u64,
    pub total_interruptions: u64,
    pub total_barge_ins: u64,
}

// ─── Overload Degradation ─────────────────────────────────────────────────

/// Overload degradation level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DegradationLevel {
    /// Normal operation.
    None,
    /// Skip optional refinement, increase coalesce interval.
    Light,
    /// Skip refinement, reduce partial rate, increase timeouts.
    Moderate,
    /// Emergency: skip all optional processing.
    Heavy,
}

impl DegradationLevel {
    /// Determine degradation level from runtime signals.
    pub fn from_signals(
        queue_pressure: QueuePressure,
        ttfa_overrun_rate: f64,
        worker_utilization: f64,
    ) -> Self {
        // Heavy: critical queue pressure OR very high overrun
        if queue_pressure == QueuePressure::Critical || ttfa_overrun_rate > 0.5 {
            return Self::Heavy;
        }
        // Moderate: high queue pressure OR sustained overruns
        if queue_pressure == QueuePressure::High
            || ttfa_overrun_rate > 0.3
            || worker_utilization > 0.9
        {
            return Self::Moderate;
        }
        // Light: elevated pressure OR occasional overruns
        if queue_pressure == QueuePressure::Elevated || ttfa_overrun_rate > 0.1 {
            return Self::Light;
        }
        Self::None
    }

    /// Whether refinement should be skipped at this level.
    pub fn skip_refinement(self) -> bool {
        matches!(self, Self::Light | Self::Moderate | Self::Heavy)
    }

    /// Whether partial coalesce interval should increase.
    pub fn increase_coalesce(self) -> bool {
        matches!(self, Self::Moderate | Self::Heavy)
    }
}

// ─── Stress Harness ───────────────────────────────────────────────────────

/// Results from a stress test run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StressResult {
    pub test_name: String,
    pub iterations: u64,
    pub duration_ms: u64,
    pub failures: u64,
    pub p50_ms: Option<u64>,
    pub p95_ms: Option<u64>,
    pub p99_ms: Option<u64>,
    pub max_ms: Option<u64>,
    pub passed: bool,
}

/// Run a timed stress iteration and record latency.
pub fn measure_iteration<F: FnOnce() -> bool>(histogram: &mut LatencyHistogram, f: F) -> bool {
    let start = Instant::now();
    let success = f();
    let elapsed_ms = start.elapsed().as_millis() as u64;
    histogram.record(elapsed_ms);
    success
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_empty() {
        let h = LatencyHistogram::new(100);
        assert!(h.is_empty());
        assert_eq!(h.p50(), None);
        assert_eq!(h.p95(), None);
        assert_eq!(h.mean(), None);
    }

    #[test]
    fn histogram_single_sample() {
        let mut h = LatencyHistogram::new(100);
        h.record(42);
        assert_eq!(h.len(), 1);
        assert_eq!(h.p50(), Some(42));
        assert_eq!(h.p95(), Some(42));
        assert_eq!(h.mean(), Some(42));
    }

    #[test]
    fn histogram_percentiles() {
        let mut h = LatencyHistogram::new(100);
        for i in 1..=100 {
            h.record(i);
        }
        assert_eq!(h.len(), 100);
        // p50 of 1..=100: index = round(0.5 * 99) = 50 → value 51
        assert_eq!(h.p50(), Some(51));
        // p95: index = round(0.95 * 99) = 94 → value 95
        assert_eq!(h.p95(), Some(95));
        // p99: index = round(0.99 * 99) = 98 → value 99
        assert_eq!(h.p99(), Some(99));
        assert_eq!(h.max(), Some(100));
        // mean of 1..=100 = 5050/100 = 50
        assert_eq!(h.mean(), Some(50));
    }

    #[test]
    fn histogram_ring_buffer_wraps() {
        let mut h = LatencyHistogram::new(10);
        for i in 1..=20 {
            h.record(i);
        }
        // Only last 10 samples retained (11..=20)
        assert_eq!(h.len(), 10);
        // median of 11..=20: index = round(0.5 * 9) = 4-5 → value 16
        let p50 = h.p50().unwrap();
        assert!(p50 >= 15 && p50 <= 16, "p50 was {}", p50);
    }

    #[test]
    fn histogram_reset() {
        let mut h = LatencyHistogram::new(100);
        h.record(42);
        h.reset();
        assert!(h.is_empty());
        assert_eq!(h.p50(), None);
    }

    #[test]
    fn queue_pressure_levels() {
        assert_eq!(
            QueuePressure::from_utilization(10, 100),
            QueuePressure::Normal
        );
        assert_eq!(
            QueuePressure::from_utilization(60, 100),
            QueuePressure::Elevated
        );
        assert_eq!(
            QueuePressure::from_utilization(85, 100),
            QueuePressure::High
        );
        assert_eq!(
            QueuePressure::from_utilization(98, 100),
            QueuePressure::Critical
        );
    }

    #[test]
    fn queue_pressure_zero_capacity() {
        assert_eq!(
            QueuePressure::from_utilization(0, 0),
            QueuePressure::Critical
        );
    }

    #[test]
    fn queue_monitor_tracks_peak() {
        let mut m = QueueMonitor::new("test", 64);
        m.update(10);
        m.update(30);
        m.update(20);
        assert_eq!(m.peak_depth, 30);
        assert_eq!(m.current_depth, 20);
    }

    #[test]
    fn queue_monitor_overflow_count() {
        let mut m = QueueMonitor::new("test", 10);
        m.update(5);
        m.update(10);
        m.update(15);
        assert_eq!(m.overflow_count, 2); // 10 and 15 >= capacity
    }

    #[test]
    fn worker_budget_acquire_release() {
        let mut b = WorkerBudget::new("whisper", 1);
        assert!(b.try_acquire());
        assert!(!b.try_acquire()); // at capacity
        assert_eq!(b.rejected_count, 1);
        b.release();
        assert!(b.try_acquire()); // slot freed
    }

    #[test]
    fn worker_budget_utilization() {
        let mut b = WorkerBudget::new("test", 4);
        assert_eq!(b.utilization(), 0.0);
        b.try_acquire();
        b.try_acquire();
        assert_eq!(b.utilization(), 0.5);
    }

    #[test]
    fn ttfa_tracker_overrun() {
        let mut t = TtfaTracker::new(500, 100);
        t.record(300); // under budget
        t.record(600); // over budget
        t.record(400); // under budget
        assert_eq!(t.overrun_count, 1);
        assert!((t.overrun_rate() - 1.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn degradation_level_none() {
        let level = DegradationLevel::from_signals(QueuePressure::Normal, 0.0, 0.5);
        assert_eq!(level, DegradationLevel::None);
        assert!(!level.skip_refinement());
        assert!(!level.increase_coalesce());
    }

    #[test]
    fn degradation_level_light() {
        let level = DegradationLevel::from_signals(QueuePressure::Elevated, 0.05, 0.5);
        assert_eq!(level, DegradationLevel::Light);
        assert!(level.skip_refinement());
        assert!(!level.increase_coalesce());
    }

    #[test]
    fn degradation_level_moderate() {
        let level = DegradationLevel::from_signals(QueuePressure::High, 0.2, 0.8);
        assert_eq!(level, DegradationLevel::Moderate);
        assert!(level.skip_refinement());
        assert!(level.increase_coalesce());
    }

    #[test]
    fn degradation_level_heavy() {
        let level = DegradationLevel::from_signals(QueuePressure::Critical, 0.6, 1.0);
        assert_eq!(level, DegradationLevel::Heavy);
        assert!(level.skip_refinement());
        assert!(level.increase_coalesce());
    }

    #[test]
    fn measure_iteration_records() {
        let mut h = LatencyHistogram::new(100);
        let success = measure_iteration(&mut h, || true);
        assert!(success);
        assert_eq!(h.len(), 1);
        // Should be very fast (< 1ms)
        assert!(h.p50().unwrap() <= 1);
    }

    #[test]
    fn stress_result_serialization() {
        let result = StressResult {
            test_name: "barge_in_storm".to_string(),
            iterations: 100,
            duration_ms: 5000,
            failures: 0,
            p50_ms: Some(2),
            p95_ms: Some(5),
            p99_ms: Some(10),
            max_ms: Some(15),
            passed: true,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("barge_in_storm"));
        let deserialized: StressResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.iterations, 100);
        assert!(deserialized.passed);
    }

    #[test]
    fn runtime_load_snapshot_serialization() {
        let snapshot = RuntimeLoadSnapshot {
            ttfa_p50_ms: Some(200),
            ttfa_p95_ms: Some(450),
            ttfa_p99_ms: Some(800),
            ttfa_overrun_rate: 0.05,
            interrupt_latency_p50_ms: Some(30),
            interrupt_latency_p95_ms: Some(50),
            cancel_latency_p50_ms: Some(10),
            cancel_latency_p95_ms: Some(25),
            audio_queue_pressure: "normal".to_string(),
            partial_queue_pressure: "normal".to_string(),
            whisper_worker_utilization: 0.3,
            total_turns: 100,
            total_interruptions: 5,
            total_barge_ins: 3,
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        let deserialized: RuntimeLoadSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.ttfa_p50_ms, Some(200));
        assert_eq!(deserialized.total_turns, 100);
    }
}
