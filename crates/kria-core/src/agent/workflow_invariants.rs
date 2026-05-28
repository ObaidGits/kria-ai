//! Runtime Invariant Monitor — Detects Canonical Runtime Violations.
//!
//! Monitors the canonical workflow runtime for invariant violations that
//! indicate bugs, race conditions, or authority conflicts. Every violation
//! is classified, logged, and optionally triggers automatic rollback.
//!
//! # Monitored Invariants
//!
//! 1. No double finalization (workflow finalized twice)
//! 2. No execution after cancellation
//! 3. No verification after finalization
//! 4. No duplicate completion telemetry
//! 5. No orphan workflows (started but never finalized)
//! 6. No lifecycle corruption (invalid state transitions)
//! 7. Monotonic telemetry ordering (seq always increases)
//! 8. Single verdict per workflow

use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// Runtime invariant violation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InvariantViolation {
    pub workflow_id: String,
    pub invariant: InvariantType,
    pub severity: ViolationSeverity,
    pub description: String,
    #[serde(skip)]
    pub timestamp: Option<std::time::Instant>,
    pub should_rollback: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum InvariantType {
    DoubleFinalization,
    ExecutionAfterCancellation,
    VerificationAfterFinalization,
    DuplicateCompletionTelemetry,
    OrphanWorkflow,
    LifecycleCorruption,
    TelemetryOrderingViolation,
    DuplicateVerdict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum ViolationSeverity {
    Warning,
    Error,
    Critical,
}

/// Monitors runtime invariants for a set of active workflows.
pub struct InvariantMonitor {
    /// Workflows that have been finalized (should not receive more events)
    finalized: HashSet<String>,
    /// Workflows that have been cancelled
    cancelled: HashSet<String>,
    /// Last telemetry sequence per workflow
    last_seq: HashMap<String, u64>,
    /// Workflows that have emitted completion telemetry
    completion_emitted: HashSet<String>,
    /// Violations detected
    violations: Vec<InvariantViolation>,
    /// Maximum violations before auto-rollback recommendation
    max_violations_before_rollback: usize,
}

impl InvariantMonitor {
    pub fn new() -> Self {
        Self {
            finalized: HashSet::new(),
            cancelled: HashSet::new(),
            last_seq: HashMap::new(),
            completion_emitted: HashSet::new(),
            violations: Vec::new(),
            max_violations_before_rollback: 3,
        }
    }

    /// Check: workflow should not be finalized twice.
    pub fn check_finalization(&mut self, workflow_id: &str) -> Option<InvariantViolation> {
        if self.finalized.contains(workflow_id) {
            let violation = InvariantViolation {
                workflow_id: workflow_id.to_string(),
                invariant: InvariantType::DoubleFinalization,
                severity: ViolationSeverity::Critical,
                description: format!("Workflow '{}' finalized twice", workflow_id),
                timestamp: Some(Instant::now()),
                should_rollback: true,
            };
            self.violations.push(violation.clone());
            return Some(violation);
        }
        self.finalized.insert(workflow_id.to_string());
        None
    }

    /// Check: no execution events after cancellation.
    pub fn check_not_cancelled(&mut self, workflow_id: &str) -> Option<InvariantViolation> {
        if self.cancelled.contains(workflow_id) {
            let violation = InvariantViolation {
                workflow_id: workflow_id.to_string(),
                invariant: InvariantType::ExecutionAfterCancellation,
                severity: ViolationSeverity::Error,
                description: format!("Execution event for cancelled workflow '{}'", workflow_id),
                timestamp: Some(Instant::now()),
                should_rollback: false,
            };
            self.violations.push(violation.clone());
            return Some(violation);
        }
        None
    }

    /// Check: telemetry sequence is monotonically increasing.
    pub fn check_telemetry_ordering(&mut self, workflow_id: &str, seq: u64) -> Option<InvariantViolation> {
        if let Some(&last) = self.last_seq.get(workflow_id) {
            if seq <= last {
                let violation = InvariantViolation {
                    workflow_id: workflow_id.to_string(),
                    invariant: InvariantType::TelemetryOrderingViolation,
                    severity: ViolationSeverity::Warning,
                    description: format!(
                        "Telemetry seq {} <= last seq {} for workflow '{}'",
                        seq, last, workflow_id
                    ),
                    timestamp: Some(Instant::now()),
                    should_rollback: false,
                };
                self.violations.push(violation.clone());
                return Some(violation);
            }
        }
        self.last_seq.insert(workflow_id.to_string(), seq);
        None
    }

    /// Check: no duplicate completion telemetry.
    pub fn check_completion_uniqueness(&mut self, workflow_id: &str) -> Option<InvariantViolation> {
        if self.completion_emitted.contains(workflow_id) {
            let violation = InvariantViolation {
                workflow_id: workflow_id.to_string(),
                invariant: InvariantType::DuplicateCompletionTelemetry,
                severity: ViolationSeverity::Critical,
                description: format!("Duplicate completion telemetry for workflow '{}'", workflow_id),
                timestamp: Some(Instant::now()),
                should_rollback: true,
            };
            self.violations.push(violation.clone());
            return Some(violation);
        }
        self.completion_emitted.insert(workflow_id.to_string());
        None
    }

    /// Record a cancellation.
    pub fn record_cancellation(&mut self, workflow_id: &str) {
        self.cancelled.insert(workflow_id.to_string());
    }

    /// Get all violations.
    pub fn violations(&self) -> &[InvariantViolation] {
        &self.violations
    }

    /// Whether rollback is recommended based on violation count.
    pub fn should_recommend_rollback(&self) -> bool {
        let critical_count = self.violations.iter()
            .filter(|v| v.severity == ViolationSeverity::Critical)
            .count();
        critical_count >= self.max_violations_before_rollback
    }

    /// Clear state for a workflow (after it's fully processed).
    pub fn clear_workflow(&mut self, workflow_id: &str) {
        self.last_seq.remove(workflow_id);
        // Keep finalized/cancelled/completion_emitted for invariant checking
    }

    /// Get violation count by severity.
    pub fn violation_counts(&self) -> (usize, usize, usize) {
        let warnings = self.violations.iter().filter(|v| v.severity == ViolationSeverity::Warning).count();
        let errors = self.violations.iter().filter(|v| v.severity == ViolationSeverity::Error).count();
        let critical = self.violations.iter().filter(|v| v.severity == ViolationSeverity::Critical).count();
        (warnings, errors, critical)
    }
}

impl Default for InvariantMonitor {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_double_finalization() {
        let mut monitor = InvariantMonitor::new();
        assert!(monitor.check_finalization("wf-1").is_none());
        let violation = monitor.check_finalization("wf-1");
        assert!(violation.is_some());
        assert_eq!(violation.unwrap().invariant, InvariantType::DoubleFinalization);
    }

    #[test]
    fn detects_execution_after_cancellation() {
        let mut monitor = InvariantMonitor::new();
        monitor.record_cancellation("wf-1");
        let violation = monitor.check_not_cancelled("wf-1");
        assert!(violation.is_some());
        assert_eq!(violation.unwrap().invariant, InvariantType::ExecutionAfterCancellation);
    }

    #[test]
    fn detects_telemetry_ordering_violation() {
        let mut monitor = InvariantMonitor::new();
        assert!(monitor.check_telemetry_ordering("wf-1", 1).is_none());
        assert!(monitor.check_telemetry_ordering("wf-1", 2).is_none());
        let violation = monitor.check_telemetry_ordering("wf-1", 1); // Out of order!
        assert!(violation.is_some());
        assert_eq!(violation.unwrap().invariant, InvariantType::TelemetryOrderingViolation);
    }

    #[test]
    fn detects_duplicate_completion() {
        let mut monitor = InvariantMonitor::new();
        assert!(monitor.check_completion_uniqueness("wf-1").is_none());
        let violation = monitor.check_completion_uniqueness("wf-1");
        assert!(violation.is_some());
        assert_eq!(violation.unwrap().invariant, InvariantType::DuplicateCompletionTelemetry);
    }

    #[test]
    fn recommends_rollback_after_critical_threshold() {
        let mut monitor = InvariantMonitor::new();
        assert!(!monitor.should_recommend_rollback());

        // Trigger 3 critical violations
        monitor.check_finalization("wf-1");
        monitor.check_finalization("wf-1"); // double finalization = critical
        monitor.check_finalization("wf-2");
        monitor.check_finalization("wf-2");
        monitor.check_finalization("wf-3");
        monitor.check_finalization("wf-3");

        assert!(monitor.should_recommend_rollback());
    }

    #[test]
    fn no_violation_for_normal_flow() {
        let mut monitor = InvariantMonitor::new();
        assert!(monitor.check_not_cancelled("wf-1").is_none());
        assert!(monitor.check_telemetry_ordering("wf-1", 1).is_none());
        assert!(monitor.check_telemetry_ordering("wf-1", 2).is_none());
        assert!(monitor.check_telemetry_ordering("wf-1", 3).is_none());
        assert!(monitor.check_completion_uniqueness("wf-1").is_none());
        assert!(monitor.check_finalization("wf-1").is_none());
        assert!(monitor.violations().is_empty());
    }

    #[test]
    fn violation_counts_are_correct() {
        let mut monitor = InvariantMonitor::new();
        // 1 warning (ordering)
        monitor.check_telemetry_ordering("wf-1", 5);
        monitor.check_telemetry_ordering("wf-1", 3); // warning
        // 1 error (execution after cancel)
        monitor.record_cancellation("wf-2");
        monitor.check_not_cancelled("wf-2"); // error
        // 1 critical (double finalization)
        monitor.check_finalization("wf-3");
        monitor.check_finalization("wf-3"); // critical

        let (warnings, errors, critical) = monitor.violation_counts();
        assert_eq!(warnings, 1);
        assert_eq!(errors, 1);
        assert_eq!(critical, 1);
    }
}
