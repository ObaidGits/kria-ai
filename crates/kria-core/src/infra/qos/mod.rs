use std::collections::{HashMap, VecDeque};
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::KriaSystemConfig;

const STARVATION_PROMOTION_THRESHOLD: u32 = 3;
const CONTROLLER_TICK_OPERATION: &str = "qos_monitor::controller_tick";

/// RFC-005 class tags for all infrastructure tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum QosClass {
    HighRecovery,
    MediumReconnect,
    LowMaintenance,
}

/// Threshold-based scheduler tuning knobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdaptiveQosConfig {
    pub high_recovery_slo_ms: u64,
    pub retry_after_defer_ms: u64,
    pub max_latency_samples: usize,
    pub max_medium_credits: u32,
    pub medium_credit_per_high_completion: u32,
    pub monitor_sample_interval_ms: u64,
    pub max_adaptation_history: usize,
}

impl AdaptiveQosConfig {
    pub fn from_system_config(system_config: &KriaSystemConfig) -> Self {
        Self {
            high_recovery_slo_ms: system_config.qos.high_recovery_slo_ms,
            retry_after_defer_ms: system_config.qos.retry_after_defer_ms,
            max_latency_samples: system_config.qos.max_latency_samples,
            max_medium_credits: system_config.qos.max_medium_credits,
            medium_credit_per_high_completion: system_config.qos.medium_credit_per_high_completion,
            monitor_sample_interval_ms: system_config.qos.monitor_sample_interval_ms,
            max_adaptation_history: system_config.qos.max_adaptation_history,
        }
    }
}

impl Default for AdaptiveQosConfig {
    fn default() -> Self {
        Self::from_system_config(&KriaSystemConfig::default())
    }
}

/// Admission decision used by the wrapper around infra tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QosAdmission {
    Accepted,
    Deferred {
        retry_after: Duration,
        reason: String,
    },
    Rejected {
        reason: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct QosClassCounters {
    pub started: u64,
    pub completed: u64,
    pub deferred: u64,
    pub rejected: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QosTelemetryPacket {
    pub timestamp_unix_ms: u64,
    pub high_recovery_inflight: usize,
    pub medium_reconnect_inflight: usize,
    pub low_maintenance_inflight: usize,
    pub high_recovery_wait_p95_ms: u64,
    pub medium_reconnect_wait_p95_ms: u64,
    pub low_maintenance_wait_p95_ms: u64,
    // Backward-compatible alias for prior field naming.
    pub high_recovery_p95_ms: u64,
    pub high_recovery_slo_ms: u64,
    pub medium_reconnect_credits: u32,
    pub low_drop_rate: f64,
    pub low_defer_rate: f64,
    pub monitor_sample_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QosAdaptationDecision {
    ThrottleLowMaintenance,
    RejectLowMaintenance,
    PromoteMediumReconnect,
    ReleaseLowMaintenanceThrottle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QosAdaptationPacket {
    pub timestamp_unix_ms: u64,
    pub decision: QosAdaptationDecision,
    pub class: QosClass,
    pub operation: String,
    pub reason: String,
    pub high_recovery_wait_p95_ms: u64,
    pub high_recovery_slo_ms: u64,
    pub high_recovery_queue_depth: usize,
    pub medium_reconnect_queue_depth: usize,
    pub low_maintenance_queue_depth: usize,
    pub medium_reconnect_defer_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QosMonitorSample {
    pub timestamp_unix_ms: u64,
    pub high_recovery_queue_depth: usize,
    pub medium_reconnect_queue_depth: usize,
    pub low_maintenance_queue_depth: usize,
    pub high_recovery_wait_p95_ms: u64,
    pub medium_reconnect_wait_p95_ms: u64,
    pub low_maintenance_wait_p95_ms: u64,
}

#[derive(Debug)]
struct QosMonitor {
    sample_interval: Duration,
    max_samples: usize,
    last_sampled_at: Option<Instant>,
    samples: VecDeque<QosMonitorSample>,
}

impl QosMonitor {
    fn new(sample_interval_ms: u64, max_samples: usize) -> Self {
        Self {
            sample_interval: Duration::from_millis(sample_interval_ms.max(1)),
            max_samples: max_samples.max(1),
            last_sampled_at: None,
            samples: VecDeque::new(),
        }
    }

    fn maybe_sample(&mut self, sample: QosMonitorSample) {
        let now = Instant::now();
        let should_sample = self
            .last_sampled_at
            .map(|last| now.duration_since(last) >= self.sample_interval)
            .unwrap_or(true);

        if !should_sample {
            return;
        }

        self.last_sampled_at = Some(now);
        push_bounded(&mut self.samples, sample, self.max_samples);
    }

    fn latest(&self) -> Option<QosMonitorSample> {
        self.samples.back().cloned()
    }

    fn recent(&self, limit: usize) -> Vec<QosMonitorSample> {
        let take = if limit == 0 {
            self.samples.len()
        } else {
            limit
        };

        let mut recent = self
            .samples
            .iter()
            .rev()
            .take(take)
            .cloned()
            .collect::<Vec<_>>();
        recent.reverse();
        recent
    }
}

#[derive(Debug)]
struct QosState {
    high_recovery_inflight: usize,
    medium_reconnect_inflight: usize,
    low_maintenance_inflight: usize,
    high_recovery_wait_ms: VecDeque<u64>,
    medium_reconnect_wait_ms: VecDeque<u64>,
    low_maintenance_wait_ms: VecDeque<u64>,
    medium_reconnect_credits: u32,
    medium_defer_streak_by_operation: HashMap<String, u32>,
    low_maintenance_throttled: bool,
    high: QosClassCounters,
    medium: QosClassCounters,
    low: QosClassCounters,
    monitor: QosMonitor,
    adaptation_history: VecDeque<QosAdaptationPacket>,
}

impl QosState {
    fn new(config: AdaptiveQosConfig) -> Self {
        Self {
            high_recovery_inflight: 0,
            medium_reconnect_inflight: 0,
            low_maintenance_inflight: 0,
            high_recovery_wait_ms: VecDeque::new(),
            medium_reconnect_wait_ms: VecDeque::new(),
            low_maintenance_wait_ms: VecDeque::new(),
            medium_reconnect_credits: 0,
            medium_defer_streak_by_operation: HashMap::new(),
            low_maintenance_throttled: false,
            high: QosClassCounters::default(),
            medium: QosClassCounters::default(),
            low: QosClassCounters::default(),
            monitor: QosMonitor::new(
                config.monitor_sample_interval_ms,
                config.max_latency_samples,
            ),
            adaptation_history: VecDeque::new(),
        }
    }

    fn wait_p95_ms(&self, class: QosClass) -> u64 {
        match class {
            QosClass::HighRecovery => percentile_95(&self.high_recovery_wait_ms),
            QosClass::MediumReconnect => percentile_95(&self.medium_reconnect_wait_ms),
            QosClass::LowMaintenance => percentile_95(&self.low_maintenance_wait_ms),
        }
    }

    fn push_wait_sample(&mut self, class: QosClass, wait_ms: u64, max_samples: usize) {
        match class {
            QosClass::HighRecovery => {
                push_bounded(&mut self.high_recovery_wait_ms, wait_ms, max_samples)
            }
            QosClass::MediumReconnect => {
                push_bounded(&mut self.medium_reconnect_wait_ms, wait_ms, max_samples)
            }
            QosClass::LowMaintenance => {
                push_bounded(&mut self.low_maintenance_wait_ms, wait_ms, max_samples)
            }
        }
    }

    fn build_monitor_sample(&self) -> QosMonitorSample {
        QosMonitorSample {
            timestamp_unix_ms: now_unix_ms(),
            high_recovery_queue_depth: self.high_recovery_inflight,
            medium_reconnect_queue_depth: self.medium_reconnect_inflight,
            low_maintenance_queue_depth: self.low_maintenance_inflight,
            high_recovery_wait_p95_ms: self.wait_p95_ms(QosClass::HighRecovery),
            medium_reconnect_wait_p95_ms: self.wait_p95_ms(QosClass::MediumReconnect),
            low_maintenance_wait_p95_ms: self.wait_p95_ms(QosClass::LowMaintenance),
        }
    }

    fn maybe_sample_monitor(&mut self) {
        self.monitor.maybe_sample(self.build_monitor_sample());
    }

    fn queue_depth_for_class(&self, class: QosClass) -> usize {
        match class {
            QosClass::HighRecovery => self.high_recovery_inflight,
            QosClass::MediumReconnect => self.medium_reconnect_inflight,
            QosClass::LowMaintenance => self.low_maintenance_inflight,
        }
    }

    fn push_adaptation(&mut self, packet: QosAdaptationPacket, max_history: usize) {
        push_bounded(&mut self.adaptation_history, packet, max_history.max(1));
    }
}

/// Shared RFC-005 adaptive scheduler for infrastructure operations.
#[derive(Debug)]
pub struct AdaptiveQosScheduler {
    config: AdaptiveQosConfig,
    state: StdMutex<QosState>,
}

impl AdaptiveQosScheduler {
    pub fn new(system_config: &KriaSystemConfig) -> Self {
        Self::with_config(AdaptiveQosConfig::from_system_config(system_config))
    }

    pub fn with_config(config: AdaptiveQosConfig) -> Self {
        Self {
            config,
            state: StdMutex::new(QosState::new(config)),
        }
    }

    pub fn classify_operation(operation: &str) -> QosClass {
        if operation == "reset_environment::medium_reconnect_slot"
            || operation.contains("reconnect")
        {
            return QosClass::MediumReconnect;
        }

        if operation.starts_with("snapshot::") {
            return QosClass::HighRecovery;
        }

        if operation.starts_with("reset_environment::") {
            return QosClass::HighRecovery;
        }

        QosClass::LowMaintenance
    }

    pub fn try_start_task(&self, class: QosClass, operation: &str) -> QosAdmission {
        let (admission, adaptation_packets) = {
            let mut state = match self.state.lock() {
                Ok(guard) => guard,
                Err(error) => {
                    return QosAdmission::Rejected {
                        reason: format!("qos lock poisoned for operation {operation}: {error}"),
                    };
                }
            };

            let mut adaptation_packets = Vec::new();

            if let Some(packet) =
                Self::refresh_low_maintenance_mode(&mut state, &self.config, operation)
            {
                state.push_adaptation(packet.clone(), self.config.max_adaptation_history);
                adaptation_packets.push(packet);
            }

            let admission = match class {
                // High lane immunity: never throttle or defer high-recovery work.
                QosClass::HighRecovery => {
                    state.high_recovery_inflight = state.high_recovery_inflight.saturating_add(1);
                    state.high.started = state.high.started.saturating_add(1);
                    state.medium_defer_streak_by_operation.remove(operation);
                    QosAdmission::Accepted
                }
                QosClass::MediumReconnect => {
                    let blocked_for_starvation_guard =
                        state.high_recovery_inflight > 0 && state.medium_reconnect_credits == 0;

                    if blocked_for_starvation_guard {
                        let defer_count = {
                            let entry = state
                                .medium_defer_streak_by_operation
                                .entry(operation.to_string())
                                .or_insert(0);
                            *entry = entry.saturating_add(1);
                            *entry
                        };

                        if defer_count > STARVATION_PROMOTION_THRESHOLD {
                            let promoted_after = defer_count;
                            state.medium_defer_streak_by_operation.remove(operation);
                            state.medium_reconnect_inflight =
                                state.medium_reconnect_inflight.saturating_add(1);
                            state.medium.started = state.medium.started.saturating_add(1);

                            let packet = Self::build_adaptation_packet(
                                &state,
                                &self.config,
                                QosAdaptationDecision::PromoteMediumReconnect,
                                QosClass::MediumReconnect,
                                operation,
                                format!(
                                    "medium reconnect deferred {} times; promoted for one cycle",
                                    promoted_after
                                ),
                                promoted_after,
                            );
                            state.push_adaptation(
                                packet.clone(),
                                self.config.max_adaptation_history,
                            );
                            adaptation_packets.push(packet);

                            QosAdmission::Accepted
                        } else {
                            state.medium.deferred = state.medium.deferred.saturating_add(1);
                            QosAdmission::Deferred {
                                retry_after: Duration::from_millis(
                                    self.config.retry_after_defer_ms.max(1),
                                ),
                                reason: format!(
                                    "medium reconnect deferred while waiting for starvation credit (attempt {})",
                                    defer_count
                                ),
                            }
                        }
                    } else {
                        state.medium_defer_streak_by_operation.remove(operation);
                        if state.medium_reconnect_credits > 0 {
                            state.medium_reconnect_credits -= 1;
                        }
                        state.medium_reconnect_inflight =
                            state.medium_reconnect_inflight.saturating_add(1);
                        state.medium.started = state.medium.started.saturating_add(1);
                        QosAdmission::Accepted
                    }
                }
                QosClass::LowMaintenance => {
                    if state.low_maintenance_throttled {
                        state.low.rejected = state.low.rejected.saturating_add(1);
                        let reason = format!(
                            "high recovery wait p95 {}ms exceeded slo {}ms",
                            state.wait_p95_ms(QosClass::HighRecovery),
                            self.config.high_recovery_slo_ms
                        );

                        let packet = Self::build_adaptation_packet(
                            &state,
                            &self.config,
                            QosAdaptationDecision::RejectLowMaintenance,
                            QosClass::LowMaintenance,
                            operation,
                            reason.clone(),
                            0,
                        );
                        state.push_adaptation(packet.clone(), self.config.max_adaptation_history);
                        adaptation_packets.push(packet);

                        QosAdmission::Rejected { reason }
                    } else {
                        state.low_maintenance_inflight =
                            state.low_maintenance_inflight.saturating_add(1);
                        state.low.started = state.low.started.saturating_add(1);
                        QosAdmission::Accepted
                    }
                }
            };

            state.maybe_sample_monitor();

            (admission, adaptation_packets)
        };

        for packet in adaptation_packets {
            emit_adaptation(packet);
        }

        admission
    }

    pub fn finish_task(&self, class: QosClass, total_latency_ms: u64, success: bool) {
        let (packet, adaptation_packets) = {
            let mut state = match self.state.lock() {
                Ok(guard) => guard,
                Err(_) => return,
            };

            let mut adaptation_packets = Vec::new();

            match class {
                QosClass::HighRecovery => {
                    state.high_recovery_inflight = state.high_recovery_inflight.saturating_sub(1);
                    state.high.completed = state.high.completed.saturating_add(1);

                    if success {
                        state.medium_reconnect_credits = state
                            .medium_reconnect_credits
                            .saturating_add(self.config.medium_credit_per_high_completion)
                            .min(self.config.max_medium_credits);
                    }
                }
                QosClass::MediumReconnect => {
                    state.medium_reconnect_inflight =
                        state.medium_reconnect_inflight.saturating_sub(1);
                    state.medium.completed = state.medium.completed.saturating_add(1);
                }
                QosClass::LowMaintenance => {
                    state.low_maintenance_inflight =
                        state.low_maintenance_inflight.saturating_sub(1);
                    state.low.completed = state.low.completed.saturating_add(1);
                }
            }

            // Current runtime contracts only expose total latency; use it as wait/pressure proxy.
            state.push_wait_sample(class, total_latency_ms, self.config.max_latency_samples);

            if let Some(packet) = Self::refresh_low_maintenance_mode(
                &mut state,
                &self.config,
                CONTROLLER_TICK_OPERATION,
            ) {
                state.push_adaptation(packet.clone(), self.config.max_adaptation_history);
                adaptation_packets.push(packet);
            }

            state.maybe_sample_monitor();

            (
                Self::build_packet(&state, self.config.high_recovery_slo_ms),
                adaptation_packets,
            )
        };

        emit_telemetry(packet);
        for adaptation in adaptation_packets {
            emit_adaptation(adaptation);
        }
    }

    pub fn telemetry_snapshot(&self) -> QosTelemetryPacket {
        let state = self
            .state
            .lock()
            .expect("qos scheduler lock poisoned while reading telemetry");
        Self::build_packet(&state, self.config.high_recovery_slo_ms)
    }

    pub fn monitor_snapshot(&self) -> Option<QosMonitorSample> {
        let state = self
            .state
            .lock()
            .expect("qos scheduler lock poisoned while reading monitor snapshot");
        state.monitor.latest()
    }

    pub fn monitor_samples(&self, limit: usize) -> Vec<QosMonitorSample> {
        let state = self
            .state
            .lock()
            .expect("qos scheduler lock poisoned while reading monitor samples");
        state.monitor.recent(limit)
    }

    pub fn adaptation_snapshot(&self, limit: usize) -> Vec<QosAdaptationPacket> {
        let state = self
            .state
            .lock()
            .expect("qos scheduler lock poisoned while reading adaptation snapshots");

        let take = if limit == 0 {
            state.adaptation_history.len()
        } else {
            limit
        };

        let mut recent = state
            .adaptation_history
            .iter()
            .rev()
            .take(take)
            .cloned()
            .collect::<Vec<_>>();
        recent.reverse();
        recent
    }

    fn refresh_low_maintenance_mode(
        state: &mut QosState,
        config: &AdaptiveQosConfig,
        operation: &str,
    ) -> Option<QosAdaptationPacket> {
        let high_wait_p95 = state.wait_p95_ms(QosClass::HighRecovery);
        let should_throttle = high_wait_p95 > config.high_recovery_slo_ms;

        if should_throttle && !state.low_maintenance_throttled {
            state.low_maintenance_throttled = true;
            return Some(Self::build_adaptation_packet(
                state,
                config,
                QosAdaptationDecision::ThrottleLowMaintenance,
                QosClass::LowMaintenance,
                operation,
                format!(
                    "high recovery wait p95 {}ms exceeded slo {}ms",
                    high_wait_p95, config.high_recovery_slo_ms
                ),
                0,
            ));
        }

        if !should_throttle && state.low_maintenance_throttled {
            state.low_maintenance_throttled = false;
            return Some(Self::build_adaptation_packet(
                state,
                config,
                QosAdaptationDecision::ReleaseLowMaintenanceThrottle,
                QosClass::LowMaintenance,
                operation,
                "high recovery wait p95 returned within slo; low maintenance lane released"
                    .to_string(),
                0,
            ));
        }

        None
    }

    fn build_adaptation_packet(
        state: &QosState,
        config: &AdaptiveQosConfig,
        decision: QosAdaptationDecision,
        class: QosClass,
        operation: &str,
        reason: String,
        medium_reconnect_defer_count: u32,
    ) -> QosAdaptationPacket {
        QosAdaptationPacket {
            timestamp_unix_ms: now_unix_ms(),
            decision,
            class,
            operation: operation.to_string(),
            reason,
            high_recovery_wait_p95_ms: state.wait_p95_ms(QosClass::HighRecovery),
            high_recovery_slo_ms: config.high_recovery_slo_ms,
            high_recovery_queue_depth: state.queue_depth_for_class(QosClass::HighRecovery),
            medium_reconnect_queue_depth: state.queue_depth_for_class(QosClass::MediumReconnect),
            low_maintenance_queue_depth: state.queue_depth_for_class(QosClass::LowMaintenance),
            medium_reconnect_defer_count,
        }
    }

    fn build_packet(state: &QosState, high_recovery_slo_ms: u64) -> QosTelemetryPacket {
        let low_attempted = state.low.started + state.low.deferred + state.low.rejected;
        let low_drop_rate = if low_attempted == 0 {
            0.0
        } else {
            state.low.rejected as f64 / low_attempted as f64
        };
        let low_defer_rate = if low_attempted == 0 {
            0.0
        } else {
            state.low.deferred as f64 / low_attempted as f64
        };

        let high_wait_p95_ms = state.wait_p95_ms(QosClass::HighRecovery);
        let medium_wait_p95_ms = state.wait_p95_ms(QosClass::MediumReconnect);
        let low_wait_p95_ms = state.wait_p95_ms(QosClass::LowMaintenance);

        QosTelemetryPacket {
            timestamp_unix_ms: now_unix_ms(),
            high_recovery_inflight: state.high_recovery_inflight,
            medium_reconnect_inflight: state.medium_reconnect_inflight,
            low_maintenance_inflight: state.low_maintenance_inflight,
            high_recovery_wait_p95_ms: high_wait_p95_ms,
            medium_reconnect_wait_p95_ms: medium_wait_p95_ms,
            low_maintenance_wait_p95_ms: low_wait_p95_ms,
            high_recovery_p95_ms: high_wait_p95_ms,
            high_recovery_slo_ms,
            medium_reconnect_credits: state.medium_reconnect_credits,
            low_drop_rate,
            low_defer_rate,
            monitor_sample_count: state.monitor.samples.len(),
        }
    }
}

fn push_bounded<T>(deque: &mut VecDeque<T>, value: T, max_samples: usize) {
    if max_samples == 0 {
        return;
    }

    deque.push_back(value);
    while deque.len() > max_samples {
        let _ = deque.pop_front();
    }
}

fn percentile_95(samples: &VecDeque<u64>) -> u64 {
    if samples.is_empty() {
        return 0;
    }

    let mut sorted = samples.iter().copied().collect::<Vec<_>>();
    sorted.sort_unstable();

    let index = ((sorted.len() as f64) * 0.95).ceil() as usize;
    let idx = index.saturating_sub(1).min(sorted.len().saturating_sub(1));
    sorted[idx]
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

fn emit_telemetry(packet: QosTelemetryPacket) {
    let payload =
        serde_json::to_string(&packet).unwrap_or_else(|error| format!("telemetry_error:{error}"));
    tracing::info!(target: "kria_qos", packet = %payload, "qos_telemetry");
}

fn emit_adaptation(packet: QosAdaptationPacket) {
    let payload =
        serde_json::to_string(&packet).unwrap_or_else(|error| format!("telemetry_error:{error}"));
    tracing::info!(target: "kria_qos", packet = %payload, "qos_adaptation");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_operation_maps_qos_classes() {
        assert_eq!(
            AdaptiveQosScheduler::classify_operation("reset_environment::admission_barrier"),
            QosClass::HighRecovery
        );
        assert_eq!(
            AdaptiveQosScheduler::classify_operation("reset_environment::medium_reconnect_slot"),
            QosClass::MediumReconnect
        );
        assert_eq!(
            AdaptiveQosScheduler::classify_operation("snapshot::qmp_restore"),
            QosClass::HighRecovery
        );
        assert_eq!(
            AdaptiveQosScheduler::classify_operation("write_file::cleanup_sidecar"),
            QosClass::LowMaintenance
        );
    }

    #[test]
    fn low_maintenance_is_rejected_when_high_wait_exceeds_slo() {
        let scheduler = AdaptiveQosScheduler::with_config(AdaptiveQosConfig {
            high_recovery_slo_ms: 10,
            ..AdaptiveQosConfig::default()
        });

        assert_eq!(
            scheduler.try_start_task(QosClass::HighRecovery, "reset_environment::a"),
            QosAdmission::Accepted
        );
        scheduler.finish_task(QosClass::HighRecovery, 200, true);

        let low = scheduler.try_start_task(QosClass::LowMaintenance, "write_file::cleanup");
        assert!(matches!(low, QosAdmission::Rejected { .. }));

        let adaptations = scheduler.adaptation_snapshot(16);
        assert!(adaptations
            .iter()
            .any(|packet| { packet.decision == QosAdaptationDecision::ThrottleLowMaintenance }));
        assert!(adaptations
            .iter()
            .any(|packet| { packet.decision == QosAdaptationDecision::RejectLowMaintenance }));
    }

    #[test]
    fn medium_reconnect_credit_prevents_permanent_starvation() {
        let scheduler = AdaptiveQosScheduler::with_config(AdaptiveQosConfig {
            max_medium_credits: 2,
            medium_credit_per_high_completion: 1,
            ..AdaptiveQosConfig::default()
        });

        assert_eq!(
            scheduler.try_start_task(QosClass::HighRecovery, "reset_environment::a"),
            QosAdmission::Accepted
        );
        scheduler.finish_task(QosClass::HighRecovery, 10, true);

        assert_eq!(
            scheduler.try_start_task(
                QosClass::MediumReconnect,
                "reset_environment::medium_reconnect_slot"
            ),
            QosAdmission::Accepted
        );
    }

    #[test]
    fn medium_reconnect_promotion_triggers_after_three_deferrals() {
        let scheduler = AdaptiveQosScheduler::with_config(AdaptiveQosConfig {
            max_medium_credits: 0,
            medium_credit_per_high_completion: 0,
            ..AdaptiveQosConfig::default()
        });

        assert_eq!(
            scheduler.try_start_task(QosClass::HighRecovery, "reset_environment::a"),
            QosAdmission::Accepted
        );

        for _ in 0..STARVATION_PROMOTION_THRESHOLD {
            assert!(matches!(
                scheduler.try_start_task(
                    QosClass::MediumReconnect,
                    "reset_environment::medium_reconnect_slot",
                ),
                QosAdmission::Deferred { .. }
            ));
        }

        assert_eq!(
            scheduler.try_start_task(
                QosClass::MediumReconnect,
                "reset_environment::medium_reconnect_slot"
            ),
            QosAdmission::Accepted
        );

        scheduler.finish_task(QosClass::MediumReconnect, 20, true);

        assert!(matches!(
            scheduler.try_start_task(
                QosClass::MediumReconnect,
                "reset_environment::medium_reconnect_slot",
            ),
            QosAdmission::Deferred { .. }
        ));

        scheduler.finish_task(QosClass::HighRecovery, 20, true);

        let adaptations = scheduler.adaptation_snapshot(16);
        assert!(adaptations
            .iter()
            .any(|packet| { packet.decision == QosAdaptationDecision::PromoteMediumReconnect }));
    }

    #[test]
    fn qos_monitor_samples_every_100ms_interval() {
        let scheduler = AdaptiveQosScheduler::with_config(AdaptiveQosConfig {
            monitor_sample_interval_ms: 100,
            ..AdaptiveQosConfig::default()
        });

        assert_eq!(
            scheduler.try_start_task(QosClass::HighRecovery, "reset_environment::a"),
            QosAdmission::Accepted
        );
        scheduler.finish_task(QosClass::HighRecovery, 40, true);

        let first_len = scheduler.monitor_samples(0).len();
        assert!(first_len >= 1);

        std::thread::sleep(Duration::from_millis(120));

        assert_eq!(
            scheduler.try_start_task(QosClass::LowMaintenance, "write_file::cleanup"),
            QosAdmission::Accepted
        );
        scheduler.finish_task(QosClass::LowMaintenance, 10, true);

        let samples = scheduler.monitor_samples(0);
        assert!(samples.len() > first_len);
        assert!(scheduler.monitor_snapshot().is_some());
    }

    #[test]
    fn load_test_high_priority_latency_stable_during_low_priority_burst() {
        let scheduler = AdaptiveQosScheduler::with_config(AdaptiveQosConfig {
            high_recovery_slo_ms: 500,
            max_latency_samples: 64,
            max_adaptation_history: 4096,
            max_medium_credits: 0,
            medium_credit_per_high_completion: 0,
            ..AdaptiveQosConfig::default()
        });

        // Prime pressure so low lane enters adaptive reject mode.
        for _ in 0..8 {
            assert_eq!(
                scheduler.try_start_task(QosClass::HighRecovery, "reset_environment::priming"),
                QosAdmission::Accepted
            );
            scheduler.finish_task(QosClass::HighRecovery, 650, true);
        }

        let mut low_rejected = 0u64;
        let mut high_requests = 0u64;

        for index in 0..4_000u64 {
            match scheduler.try_start_task(QosClass::LowMaintenance, "maintenance::burst") {
                QosAdmission::Accepted => {
                    scheduler.finish_task(QosClass::LowMaintenance, 5, true);
                }
                QosAdmission::Deferred { .. } => {}
                QosAdmission::Rejected { .. } => {
                    low_rejected = low_rejected.saturating_add(1);
                }
            }

            if index % 8 == 0 {
                high_requests = high_requests.saturating_add(1);
                assert_eq!(
                    scheduler.try_start_task(
                        QosClass::HighRecovery,
                        "reset_environment::load_test_high_cycle"
                    ),
                    QosAdmission::Accepted
                );
                scheduler.finish_task(QosClass::HighRecovery, 40, true);
            }
        }

        let telemetry = scheduler.telemetry_snapshot();
        assert!(high_requests > 0);
        assert!(
            low_rejected > 0,
            "expected low lane rejections under high wait breach"
        );
        assert!(
            telemetry.high_recovery_wait_p95_ms <= 80,
            "high lane p95 wait should recover/stay stable under burst, got {}ms",
            telemetry.high_recovery_wait_p95_ms
        );

        let adaptations = scheduler.adaptation_snapshot(4096);
        assert!(adaptations
            .iter()
            .any(|packet| { packet.decision == QosAdaptationDecision::ThrottleLowMaintenance }));
        assert!(adaptations
            .iter()
            .any(|packet| { packet.decision == QosAdaptationDecision::RejectLowMaintenance }));
    }
}
