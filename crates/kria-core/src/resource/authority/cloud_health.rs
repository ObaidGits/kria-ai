//! Cloud Device health + circuit breaker (HRA Task 29 / R21.4).
//!
//! Drives a cloud pool's `BreakerState` from observed success/failure so the Planner avoids tripped
//! pools and failover storms are prevented. Honors `Retry-After`. Pure state machine.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breaker {
    Closed,
    Open { until_ms: u64 },
    HalfOpen,
}

#[derive(Debug, Clone)]
pub struct CloudHealth {
    breaker: Breaker,
    consecutive_failures: u32,
    failure_threshold: u32,
    open_cooldown_ms: u64,
    /// Smoothed error rate 0..=1 (EWMA).
    err_rate: f64,
    alpha: f64,
}

impl CloudHealth {
    pub fn new(failure_threshold: u32, open_cooldown_ms: u64) -> Self {
        Self {
            breaker: Breaker::Closed,
            consecutive_failures: 0,
            failure_threshold,
            open_cooldown_ms,
            err_rate: 0.0,
            alpha: 0.3,
        }
    }

    pub fn state(&self) -> Breaker {
        self.breaker
    }

    pub fn error_rate(&self) -> f64 {
        self.err_rate
    }

    /// Whether a request is allowed right now (caller passes `now_ms`).
    pub fn allow(&mut self, now_ms: u64) -> bool {
        match self.breaker {
            Breaker::Closed => true,
            Breaker::HalfOpen => true, // allow a probe
            Breaker::Open { until_ms } => {
                if now_ms >= until_ms {
                    self.breaker = Breaker::HalfOpen; // probe window
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.err_rate = self.alpha * 0.0 + (1.0 - self.alpha) * self.err_rate;
        self.breaker = Breaker::Closed;
    }

    /// Record a failure. `retry_after_ms` (e.g. from an HTTP 429 `Retry-After`) overrides cooldown.
    pub fn record_failure(&mut self, now_ms: u64, retry_after_ms: Option<u64>) {
        self.consecutive_failures += 1;
        self.err_rate = self.alpha * 1.0 + (1.0 - self.alpha) * self.err_rate;
        let trip = self.consecutive_failures >= self.failure_threshold
            || matches!(self.breaker, Breaker::HalfOpen);
        if trip {
            let cooldown = retry_after_ms.unwrap_or(self.open_cooldown_ms);
            self.breaker = Breaker::Open {
                until_ms: now_ms + cooldown,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trips_open_after_threshold_failures() {
        let mut h = CloudHealth::new(3, 1000);
        assert!(h.allow(0));
        h.record_failure(0, None);
        h.record_failure(0, None);
        assert!(matches!(h.state(), Breaker::Closed));
        h.record_failure(0, None); // 3rd → open
        assert!(matches!(h.state(), Breaker::Open { .. }));
        assert!(!h.allow(500)); // still cooling
    }

    #[test]
    fn half_open_probe_then_recover_on_success() {
        let mut h = CloudHealth::new(1, 1000);
        h.record_failure(0, None); // open until 1000
        assert!(!h.allow(500));
        assert!(h.allow(1000)); // half-open probe allowed
        h.record_success();
        assert!(matches!(h.state(), Breaker::Closed));
    }

    #[test]
    fn half_open_failure_reopens() {
        let mut h = CloudHealth::new(1, 1000);
        h.record_failure(0, None);
        h.allow(1000); // → half-open
        h.record_failure(1000, None); // half-open failure reopens
        assert!(matches!(h.state(), Breaker::Open { .. }));
    }

    #[test]
    fn retry_after_overrides_cooldown() {
        let mut h = CloudHealth::new(1, 1000);
        h.record_failure(0, Some(5000));
        assert!(!h.allow(2000));
        assert!(h.allow(5000));
    }
}
