//! Predictive engines — Workload Prediction (Task 30 / R14), Resource Forecasting (Task 32 / R16),
//! Autonomous Optimization (Task 34 / R20).
//!
//! ALL advisory. None can call the Scheduler/Planner admission API — these types hold no authority
//! handle (Property 12 enforced by module boundary). They only produce prewarm hints / forecasts /
//! profile suggestions that the runtime may apply within budget + veto rules.

use std::collections::HashMap;

use super::types::ConsumerId;

// ── Workload Prediction Engine (WPE) ─────────────────────────────────

/// Deterministic UI/runtime signals that hint upcoming work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkloadSignal {
    PanelOpened(ConsumerId),
    PromptTyping(ConsumerId),
    FileDropped,
    MicOpened,
    WorkflowStarted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrewarmHint {
    pub consumer: ConsumerId,
    pub confidence: u8, // 0..=100
}

/// Maps a signal to a prewarm hint. The DECISION to prewarm is gated elsewhere by free-headroom +
/// thermal/battery veto (R14.2); this only proposes.
pub fn prewarm_hint(signal: WorkloadSignal) -> Option<PrewarmHint> {
    let (consumer, confidence) = match signal {
        WorkloadSignal::PromptTyping(c) => (c, 80),
        WorkloadSignal::PanelOpened(c) => (c, 45),
        WorkloadSignal::FileDropped => (ConsumerId::Vision, 55),
        WorkloadSignal::MicOpened => (ConsumerId::Stt, 85),
        WorkloadSignal::WorkflowStarted => (ConsumerId::Agent, 70),
    };
    Some(PrewarmHint {
        consumer,
        confidence,
    })
}

/// Gate a prewarm hint: only allow if there is free headroom and no veto. Never evicts (R14.2).
pub fn prewarm_allowed(
    hint: &PrewarmHint,
    free_mb: u64,
    need_mb: u64,
    veto: bool,
    min_conf: u8,
) -> bool {
    !veto && hint.confidence >= min_conf && free_mb >= need_mb
}

// ── Resource Forecasting Engine (RFE) ────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Vram,
    Ram,
    Thermal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Forecast {
    pub resource: ResourceKind,
    /// Seconds until the value crosses `threshold` at the current smoothed slope; None if not
    /// trending toward it.
    pub time_to_threshold_s: Option<f32>,
    pub confidence: f32,
}

/// EWMA-smoothed slope forecaster over a single series. Avoids false alarms on spiky telemetry by
/// requiring a sustained slope (confidence grows with consistent direction).
#[derive(Debug, Clone)]
pub struct Forecaster {
    resource: ResourceKind,
    ewma_value: Option<f64>,
    ewma_slope: f64,
    alpha: f64,
    last_value: Option<f64>,
    consistent: u32,
}

impl Forecaster {
    pub fn new(resource: ResourceKind) -> Self {
        Self {
            resource,
            ewma_value: None,
            ewma_slope: 0.0,
            alpha: 0.4,
            last_value: None,
            consistent: 0,
        }
    }

    /// Push a sample taken `dt_s` seconds after the previous one. Returns a forecast toward
    /// `threshold` (a lower bound the value is decreasing toward, e.g. free VRAM → 0).
    pub fn observe(&mut self, value: f64, dt_s: f32, threshold: f64) -> Forecast {
        let v = match self.ewma_value {
            None => value,
            Some(p) => self.alpha * value + (1.0 - self.alpha) * p,
        };
        if let (Some(last), true) = (self.last_value, dt_s > 0.0) {
            let inst_slope = (v - last) / dt_s as f64; // units/sec
                                                       // consistency: same sign as smoothed slope → grow confidence
            if inst_slope.signum() == self.ewma_slope.signum() || self.ewma_slope == 0.0 {
                self.consistent = (self.consistent + 1).min(10);
            } else {
                self.consistent = 0;
            }
            self.ewma_slope = self.alpha * inst_slope + (1.0 - self.alpha) * self.ewma_slope;
        }
        self.ewma_value = Some(v);
        self.last_value = Some(v);

        let confidence = (self.consistent as f32 / 10.0).min(1.0);
        // Only forecast if decreasing toward the threshold.
        let time = if self.ewma_slope < -1e-6 && v > threshold {
            Some(((threshold - v) / self.ewma_slope) as f32)
        } else {
            None
        };
        Forecast {
            resource: self.resource,
            time_to_threshold_s: time.filter(|t| *t >= 0.0),
            confidence,
        }
    }
}

// ── Autonomous Optimization Layer (AOL) ──────────────────────────────

/// Advisory store of learned priors. Holds NO authority handle — it can only emit hints/suggestions
/// that the runtime may use, never admission decisions (Property 12).
#[derive(Debug, Clone, Default)]
pub struct AutonomousOptimizer {
    /// hour-of-day (0..24) → most-used consumer count
    hourly: HashMap<u8, HashMap<ConsumerId, u32>>,
}

impl AutonomousOptimizer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an observed activity at `hour`.
    pub fn record(&mut self, hour: u8, consumer: ConsumerId) {
        *self
            .hourly
            .entry(hour % 24)
            .or_default()
            .entry(consumer)
            .or_insert(0) += 1;
    }

    /// Suggest the consumer most likely to be used at `hour` (advisory prewarm prior). Cold-start
    /// returns None → no influence, cannot harm.
    pub fn suggest_prewarm(&self, hour: u8) -> Option<ConsumerId> {
        self.hourly
            .get(&(hour % 24))?
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(c, _)| *c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_signal_high_confidence() {
        let h = prewarm_hint(WorkloadSignal::PromptTyping(ConsumerId::Llm)).unwrap();
        assert_eq!(h.consumer, ConsumerId::Llm);
        assert!(h.confidence >= 80);
    }

    #[test]
    fn prewarm_blocked_without_headroom_or_on_veto() {
        let h = PrewarmHint {
            consumer: ConsumerId::Image,
            confidence: 90,
        };
        assert!(!prewarm_allowed(&h, 1000, 4000, false, 50)); // no headroom
        assert!(!prewarm_allowed(&h, 8000, 4000, true, 50)); // veto
        assert!(prewarm_allowed(&h, 8000, 4000, false, 50)); // ok
    }

    #[test]
    fn forecaster_predicts_exhaustion_when_decreasing() {
        let mut f = Forecaster::new(ResourceKind::Vram);
        // free VRAM dropping 500/s from 5000 toward 0.
        let mut last = Forecast {
            resource: ResourceKind::Vram,
            time_to_threshold_s: None,
            confidence: 0.0,
        };
        let mut v = 5000.0;
        for _ in 0..6 {
            v -= 500.0;
            last = f.observe(v, 1.0, 0.0);
        }
        assert!(last.time_to_threshold_s.is_some());
        assert!(last.confidence > 0.0);
    }

    #[test]
    fn forecaster_no_alarm_when_stable() {
        let mut f = Forecaster::new(ResourceKind::Vram);
        let mut last = None;
        for _ in 0..5 {
            last = Some(f.observe(5000.0, 1.0, 0.0));
        }
        assert!(last.unwrap().time_to_threshold_s.is_none());
    }

    #[test]
    fn aol_cold_start_is_neutral() {
        let aol = AutonomousOptimizer::new();
        assert!(aol.suggest_prewarm(9).is_none());
    }

    #[test]
    fn aol_learns_dominant_consumer_per_hour() {
        let mut aol = AutonomousOptimizer::new();
        for _ in 0..3 {
            aol.record(9, ConsumerId::Llm);
        }
        aol.record(9, ConsumerId::Image);
        assert_eq!(aol.suggest_prewarm(9), Some(ConsumerId::Llm));
    }
}
