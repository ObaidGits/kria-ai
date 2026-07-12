//! Wave 11 — Production Hardening: failure taxonomy + retry policy (spec R12.1/
//! R12.3). Pure, neutral, provider-agnostic. Reused by the reliable execution
//! path ([`crate::capability::platform::CapabilityPlatform::execute_reliable`])
//! and the job manager — there is exactly ONE retry/classification implementation.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::capability::error::CapError;

/// The neutral failure taxonomy (spec R12.3): every capability/plan failure maps
/// to exactly one class, which drives the mapped recovery (retry vs decline vs
/// escalate). Provider errors are already translated to [`CapError`] at the ACL
/// boundary, so classification is provider-neutral.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    /// A transient transport/execution hiccup — safe to retry.
    Transient,
    /// The operation exceeded its wall-clock timeout — retry (bounded).
    Timeout,
    /// The provider is offline / circuit-open — retry after backoff.
    Offline,
    /// Permission denied / approval required — NOT retryable (needs a human).
    PermissionDenied,
    /// A capability/descriptor/schema problem — NOT retryable (deterministic).
    Schema,
    /// The requested facet/op is unsupported — NOT retryable.
    Unsupported,
    /// The operation was cancelled — terminal, not a failure to retry.
    Cancelled,
    /// A permanent/honest failure (decline, invalid input) — NOT retryable.
    Permanent,
}

impl FailureClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::Timeout => "timeout",
            Self::Offline => "offline",
            Self::PermissionDenied => "permission_denied",
            Self::Schema => "schema",
            Self::Unsupported => "unsupported",
            Self::Cancelled => "cancelled",
            Self::Permanent => "permanent",
        }
    }

    /// Whether a failure of this class is worth a bounded retry. Only genuinely
    /// transient/availability classes retry; deterministic failures never do
    /// (never infinite/pointless retry, spec R12.1).
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient | Self::Timeout | Self::Offline)
    }
}

/// Classify a [`CapError`] into the neutral taxonomy (spec R12.3). Timeout is
/// synthesized by the caller (the execution wrapper) — a `CapError` alone never
/// carries "timeout", so callers pass [`FailureClass::Timeout`] directly on a
/// timed-out attempt.
pub fn classify(err: &CapError) -> FailureClass {
    match err {
        CapError::Negotiation(_) => FailureClass::Transient,
        CapError::Unsupported(_) => FailureClass::Unsupported,
        CapError::Descriptor(_) => FailureClass::Schema,
        CapError::Discovery(_) => FailureClass::Transient,
        CapError::Permission(_) => FailureClass::PermissionDenied,
        // Acquisition/execution failures are treated as transient (often network/
        // runtime flakiness); the bounded attempt limit prevents pointless loops,
        // and a persistent failure exhausts the budget and surfaces honestly.
        CapError::Acquire(_) => FailureClass::Transient,
        CapError::Execute(_) => FailureClass::Transient,
        CapError::Degraded(_) => FailureClass::Transient,
        CapError::ProviderOffline(_) => FailureClass::Offline,
        CapError::Io(_) => FailureClass::Transient,
    }
}

/// A bounded, jittered retry policy (spec R12.1). Data, not code. Never allows
/// infinite retry: `max_attempts` caps the count and `total_budget` caps the
/// cumulative wall time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetryPolicy {
    /// Total attempts including the first (so `1` = no retry).
    pub max_attempts: u32,
    /// Base backoff before the first retry.
    pub base_delay: Duration,
    /// Cap on any single backoff.
    pub max_delay: Duration,
    /// Jitter fraction (0.0..=1.0) applied to each backoff.
    pub jitter_frac: f32,
    /// Per-attempt wall-clock timeout.
    pub per_attempt_timeout: Duration,
    /// Cumulative wall-time budget across all attempts (0 = unbounded by budget).
    pub total_budget: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(10),
            jitter_frac: 0.2,
            per_attempt_timeout: Duration::from_secs(30),
            total_budget: Duration::from_secs(120),
        }
    }
}

impl RetryPolicy {
    /// A policy that never retries (single attempt) with the given timeout.
    pub fn single(timeout: Duration) -> Self {
        Self {
            max_attempts: 1,
            per_attempt_timeout: timeout,
            ..Default::default()
        }
    }

    /// The backoff before retry `attempt` (1-based: the delay AFTER attempt 1
    /// fails, before attempt 2). Exponential (2^(attempt-1) * base), capped, with
    /// deterministic-enough jitter derived from the wall clock (no rng dep).
    pub fn delay_for(&self, attempt: u32) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }
        let exp = 2u32.saturating_pow(attempt.saturating_sub(1).min(16));
        let base = self.base_delay.saturating_mul(exp).min(self.max_delay);
        let frac = self.jitter_frac.clamp(0.0, 1.0);
        if frac == 0.0 {
            return base;
        }
        let nanos = chrono::Utc::now().timestamp_subsec_nanos() as f64 / 1_000_000_000.0;
        let delta = base.as_secs_f64() * frac as f64 * nanos;
        Duration::from_secs_f64((base.as_secs_f64() + delta).min(self.max_delay.as_secs_f64()))
    }

    /// Whether another attempt is permitted given the attempt count + elapsed
    /// budget + the failure class (only retryable classes, within limits).
    pub fn should_retry(&self, class: FailureClass, attempts_done: u32, elapsed: Duration) -> bool {
        if !class.is_retryable() {
            return false;
        }
        if attempts_done >= self.max_attempts {
            return false;
        }
        if !self.total_budget.is_zero() && elapsed >= self.total_budget {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_maps_and_gates_retry() {
        assert_eq!(
            classify(&CapError::Execute("x".into())),
            FailureClass::Transient
        );
        assert_eq!(
            classify(&CapError::Permission("x".into())),
            FailureClass::PermissionDenied
        );
        assert_eq!(
            classify(&CapError::ProviderOffline("x".into())),
            FailureClass::Offline
        );
        assert_eq!(
            classify(&CapError::Descriptor("x".into())),
            FailureClass::Schema
        );
        assert!(FailureClass::Transient.is_retryable());
        assert!(FailureClass::Timeout.is_retryable());
        assert!(FailureClass::Offline.is_retryable());
        assert!(!FailureClass::PermissionDenied.is_retryable());
        assert!(!FailureClass::Schema.is_retryable());
        assert!(!FailureClass::Unsupported.is_retryable());
        assert!(!FailureClass::Permanent.is_retryable());
        assert!(!FailureClass::Cancelled.is_retryable());
    }

    #[test]
    fn backoff_is_exponential_capped_and_bounded() {
        let p = RetryPolicy {
            max_attempts: 4,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(500),
            jitter_frac: 0.0,
            ..Default::default()
        };
        assert_eq!(p.delay_for(1), Duration::from_millis(100));
        assert_eq!(p.delay_for(2), Duration::from_millis(200));
        assert_eq!(p.delay_for(3), Duration::from_millis(400));
        // Capped at max_delay.
        assert_eq!(p.delay_for(4), Duration::from_millis(500));
        assert_eq!(p.delay_for(10), Duration::from_millis(500));
    }

    #[test]
    fn should_retry_respects_class_attempts_and_budget() {
        let p = RetryPolicy {
            max_attempts: 3,
            total_budget: Duration::from_secs(10),
            ..Default::default()
        };
        // Retryable within limits.
        assert!(p.should_retry(FailureClass::Transient, 1, Duration::from_secs(1)));
        // Non-retryable class.
        assert!(!p.should_retry(FailureClass::PermissionDenied, 1, Duration::from_secs(1)));
        // Attempts exhausted.
        assert!(!p.should_retry(FailureClass::Transient, 3, Duration::from_secs(1)));
        // Budget exhausted.
        assert!(!p.should_retry(FailureClass::Transient, 1, Duration::from_secs(11)));
    }

    #[test]
    fn single_policy_never_retries() {
        let p = RetryPolicy::single(Duration::from_secs(5));
        assert_eq!(p.max_attempts, 1);
        assert!(!p.should_retry(FailureClass::Transient, 1, Duration::ZERO));
    }
}
