//! Daemon supervisor logic (HRA Task 19 / R11).
//!
//! Pure restart/backoff/circuit-breaker state machine for the supervised daemons
//! (Core/Voice/Wake/GPU Monitor/Health/Extension Host). A daemon crash never crashes Core: the
//! supervisor restarts it with exponential backoff and trips a circuit breaker after repeated
//! failures so restart storms are bounded. The runtime drives this FSM and performs the actual
//! spawn; this module owns the decision logic and is unit-tested.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonState {
    Running,
    Restarting {
        attempt: u32,
        backoff_ms: u64,
    },
    /// Circuit broken after too many failures in the window — stop auto-restarting.
    CircuitOpen,
    Stopped,
}

#[derive(Debug, Clone, Copy)]
pub struct SupervisorPolicy {
    pub base_backoff_ms: u64,
    pub max_backoff_ms: u64,
    /// Failures within `window_ms` before the breaker opens.
    pub failure_threshold: u32,
    pub window_ms: u64,
}

impl Default for SupervisorPolicy {
    fn default() -> Self {
        Self {
            base_backoff_ms: 200,
            max_backoff_ms: 30_000,
            failure_threshold: 5,
            window_ms: 60_000,
        }
    }
}

/// Supervises a single daemon's restart lifecycle. Pure: feed it crash/recovery events + time.
#[derive(Debug, Clone)]
pub struct DaemonSupervisor {
    name: String,
    state: DaemonState,
    policy: SupervisorPolicy,
    failure_times_ms: Vec<u64>,
    attempt: u32,
}

impl DaemonSupervisor {
    pub fn new(name: impl Into<String>, policy: SupervisorPolicy) -> Self {
        Self {
            name: name.into(),
            state: DaemonState::Running,
            policy,
            failure_times_ms: Vec::new(),
            attempt: 0,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn state(&self) -> DaemonState {
        self.state
    }

    fn prune(&mut self, now_ms: u64) {
        let cutoff = now_ms.saturating_sub(self.policy.window_ms);
        self.failure_times_ms.retain(|t| *t >= cutoff);
    }

    /// Record a crash at `now_ms`. Returns the next state: schedules a backoff restart, or opens the
    /// circuit if the failure rate exceeds the threshold within the window.
    pub fn on_crash(&mut self, now_ms: u64) -> DaemonState {
        self.failure_times_ms.push(now_ms);
        self.prune(now_ms);

        if self.failure_times_ms.len() as u32 >= self.policy.failure_threshold {
            self.state = DaemonState::CircuitOpen;
            return self.state;
        }

        self.attempt += 1;
        let backoff = (self.policy.base_backoff_ms * 2u64.saturating_pow(self.attempt - 1))
            .min(self.policy.max_backoff_ms);
        self.state = DaemonState::Restarting {
            attempt: self.attempt,
            backoff_ms: backoff,
        };
        self.state
    }

    /// The daemon came back healthy → reset breaker + backoff.
    pub fn on_recovered(&mut self) {
        self.attempt = 0;
        self.failure_times_ms.clear();
        self.state = DaemonState::Running;
    }

    /// Manual intervention cleared the breaker.
    pub fn reset_circuit(&mut self) {
        if matches!(self.state, DaemonState::CircuitOpen) {
            self.attempt = 0;
            self.failure_times_ms.clear();
            self.state = DaemonState::Running;
        }
    }

    /// Whether the runtime should attempt a (re)spawn now.
    pub fn should_restart(&self) -> bool {
        matches!(self.state, DaemonState::Restarting { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sup() -> DaemonSupervisor {
        DaemonSupervisor::new("voice", SupervisorPolicy::default())
    }

    #[test]
    fn first_crash_schedules_backoff_restart() {
        let mut s = sup();
        let st = s.on_crash(1000);
        assert!(matches!(st, DaemonState::Restarting { attempt: 1, .. }));
        assert!(s.should_restart());
    }

    #[test]
    fn backoff_grows_exponentially_capped() {
        let mut s = sup();
        let mut last = 0;
        for i in 0..4 {
            if let DaemonState::Restarting { backoff_ms, .. } = s.on_crash(1000 + i) {
                assert!(backoff_ms >= last);
                last = backoff_ms;
            }
        }
        assert!(last <= SupervisorPolicy::default().max_backoff_ms);
    }

    #[test]
    fn breaker_opens_after_threshold() {
        let mut s = sup();
        let mut st = DaemonState::Running;
        for i in 0..5 {
            st = s.on_crash(1000 + i);
        }
        assert_eq!(st, DaemonState::CircuitOpen);
        assert!(!s.should_restart());
    }

    #[test]
    fn recovery_resets_breaker_and_backoff() {
        let mut s = sup();
        s.on_crash(1000);
        s.on_crash(1001);
        s.on_recovered();
        assert_eq!(s.state(), DaemonState::Running);
        // next crash starts backoff from attempt 1 again
        assert!(matches!(
            s.on_crash(2000),
            DaemonState::Restarting { attempt: 1, .. }
        ));
    }

    #[test]
    fn old_failures_outside_window_do_not_trip_breaker() {
        let mut s = sup();
        // 4 failures long ago
        for i in 0..4 {
            s.on_crash(1000 + i);
        }
        // one failure far in the future → window prunes the old ones, no trip
        let st = s.on_crash(1000 + SupervisorPolicy::default().window_ms + 5000);
        assert!(matches!(st, DaemonState::Restarting { .. }));
    }

    #[test]
    fn manual_reset_clears_open_circuit() {
        let mut s = sup();
        for i in 0..5 {
            s.on_crash(1000 + i);
        }
        assert_eq!(s.state(), DaemonState::CircuitOpen);
        s.reset_circuit();
        assert_eq!(s.state(), DaemonState::Running);
    }
}
