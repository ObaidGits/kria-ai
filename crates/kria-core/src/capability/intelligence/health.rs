//! Wave 8 — Capability Health scoring (neutral, spec R6.1).
//!
//! Turns the CKB's learned per-capability signals (success/total, last failure,
//! latency, consecutive failures) into a neutral [`HealthStatus`] that the
//! Evolution Engine (R6) and family trade-off selection (R17) reason over. No
//! provider-specific logic — a capability from ANY provider is scored the same.

use serde::{Deserialize, Serialize};

/// Neutral, coarse health classification of a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    /// Little/no data yet — treat conservatively, never penalize.
    Unknown,
    /// Performing well.
    Healthy,
    /// Elevated failures/latency — watch, may warrant a proposal.
    Warning,
    /// Chronic failure — a strong evolution trigger.
    Critical,
    /// Explicitly quarantined (trust/integrity gate).
    Quarantined,
}

impl HealthStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Healthy => "healthy",
            Self::Warning => "warning",
            Self::Critical => "critical",
            Self::Quarantined => "quarantined",
        }
    }
}

/// The raw learned signals for one capability (read from the CKB).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityHealth {
    pub provider_id: String,
    pub capability_id: String,
    pub family: String,
    /// Total recorded executions.
    pub total: u64,
    /// Successful executions.
    pub successes: u64,
    /// Consecutive trailing failures (chronic-failure signal).
    pub consecutive_failures: u32,
    pub last_latency_ms: Option<u64>,
    pub last_failure: Option<String>,
    /// Whether the capability is quarantined (trust/integrity gate).
    pub quarantined: bool,
    /// Computed status (filled by [`compute_status`]).
    pub status: HealthStatus,
}

impl CapabilityHealth {
    /// Learned success rate; `None` when unobserved.
    pub fn success_rate(&self) -> Option<f32> {
        if self.total == 0 {
            None
        } else {
            Some(self.successes as f32 / self.total as f32)
        }
    }
}

/// Tunable, versioned health policy (data, not code).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HealthPolicy {
    /// Minimum executions before success-rate is trusted (else Unknown).
    pub min_observations: u64,
    /// Success rate at/below which health is Critical.
    pub critical_success_rate: f32,
    /// Success rate at/below which health is Warning.
    pub warning_success_rate: f32,
    /// Consecutive failures at/above which health is Critical regardless of rate.
    pub critical_consecutive_failures: u32,
}

impl Default for HealthPolicy {
    fn default() -> Self {
        Self {
            min_observations: 3,
            critical_success_rate: 0.40,
            warning_success_rate: 0.75,
            critical_consecutive_failures: 3,
        }
    }
}

impl HealthPolicy {
    /// Compute the neutral [`HealthStatus`] from a capability's signals.
    pub fn compute_status(&self, h: &CapabilityHealth) -> HealthStatus {
        if h.quarantined {
            return HealthStatus::Quarantined;
        }
        if h.consecutive_failures >= self.critical_consecutive_failures && h.total > 0 {
            return HealthStatus::Critical;
        }
        match h.success_rate() {
            None => HealthStatus::Unknown,
            Some(_) if h.total < self.min_observations => HealthStatus::Unknown,
            Some(r) if r <= self.critical_success_rate => HealthStatus::Critical,
            Some(r) if r <= self.warning_success_rate => HealthStatus::Warning,
            Some(_) => HealthStatus::Healthy,
        }
    }
}

/// Apply the policy to fill in `status` for a batch of health snapshots.
pub fn classify(
    policy: &HealthPolicy,
    mut snapshots: Vec<CapabilityHealth>,
) -> Vec<CapabilityHealth> {
    for s in &mut snapshots {
        s.status = policy.compute_status(s);
    }
    snapshots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(total: u64, successes: u64, consec: u32, quarantined: bool) -> CapabilityHealth {
        CapabilityHealth {
            provider_id: "p".into(),
            capability_id: "c".into(),
            family: "Data".into(),
            total,
            successes,
            consecutive_failures: consec,
            last_latency_ms: Some(10),
            last_failure: None,
            quarantined,
            status: HealthStatus::Unknown,
        }
    }

    #[test]
    fn unobserved_is_unknown() {
        let p = HealthPolicy::default();
        assert_eq!(p.compute_status(&h(0, 0, 0, false)), HealthStatus::Unknown);
        assert_eq!(p.compute_status(&h(2, 2, 0, false)), HealthStatus::Unknown);
        // below min_observations
    }

    #[test]
    fn rates_map_to_status() {
        let p = HealthPolicy::default();
        assert_eq!(
            p.compute_status(&h(10, 10, 0, false)),
            HealthStatus::Healthy
        );
        assert_eq!(p.compute_status(&h(10, 7, 0, false)), HealthStatus::Warning);
        assert_eq!(
            p.compute_status(&h(10, 3, 0, false)),
            HealthStatus::Critical
        );
    }

    #[test]
    fn consecutive_failures_force_critical() {
        let p = HealthPolicy::default();
        // Even with decent historical rate, a chronic trailing failure streak is Critical.
        assert_eq!(
            p.compute_status(&h(20, 18, 3, false)),
            HealthStatus::Critical
        );
    }

    #[test]
    fn quarantine_dominates() {
        let p = HealthPolicy::default();
        assert_eq!(
            p.compute_status(&h(10, 10, 0, true)),
            HealthStatus::Quarantined
        );
    }
}
