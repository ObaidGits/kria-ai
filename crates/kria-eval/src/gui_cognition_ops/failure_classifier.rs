//! Automatic Failure Classification for GUI Cognition Evals
//!
//! Classifies runtime failures into actionable categories so they can
//! be fixed systematically rather than one-by-one.

use serde::{Deserialize, Serialize};

/// Classified failure from a GUI cognition eval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifiedFailure {
    pub scenario_id: String,
    pub classification: FailureClass,
    pub severity: FailureSeverity,
    pub evidence: String,
    pub probable_cause: String,
    pub suggested_fix: String,
    pub timing_ms: u64,
}

/// Failure classification taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureClass {
    NavigationTimeout,
    FocusDrift,
    CompositorLimitation,
    VerifierFalseNegative,
    RaceCondition,
    LifecycleDesync,
    CapabilityMismatch,
    InteractionUnsafety,
    StaleWindowDetection,
    WorkflowDeadlock,
    TelemetryGap,
    VisibilityFailure,
    RuntimeTimeout,
    CancellationFailure,
    RecoveryFailure,
    AppLaunchFailure,
    EnvironmentInstability,
    MissingDependency,
    PermissionDenied,
    PortConflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FailureSeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// Classify a failure from its error message and context.
pub fn classify_failure(
    error: &str,
    action: &str,
    duration_ms: u64,
    scenario_id: &str,
) -> ClassifiedFailure {
    let lower = error.to_lowercase();

    let (classification, probable_cause, suggested_fix) = if lower.contains("timed out") {
        if lower.contains("browser") || lower.contains("navigate") || action.contains("browser") {
            (
                FailureClass::NavigationTimeout,
                "Browser CDP connection or page load exceeded timeout",
                "Increase timeout, fix CDP connection, or improve xdg-open fallback",
            )
        } else {
            (
                FailureClass::RuntimeTimeout,
                "Step execution exceeded allocated timeout budget",
                "Increase step timeout or optimize execution path",
            )
        }
    } else if lower.contains("target mismatch") || lower.contains("execution_blocked") {
        (
            FailureClass::CapabilityMismatch,
            "Legacy execution authority blocked tool due to target category error",
            "Verify environment override is active in PolicyToolExecutor",
        )
    } else if lower.contains("not found") && lower.contains("application") {
        (
            FailureClass::MissingDependency,
            "Required application is not installed",
            "Add HITL install prompt or suggest alternative app",
        )
    } else if lower.contains("permission denied") {
        (
            FailureClass::PermissionDenied,
            "Insufficient permissions for the requested operation",
            "Add sudo handling or permission escalation HITL",
        )
    } else if lower.contains("focus") || lower.contains("window") && lower.contains("mismatch") {
        (
            FailureClass::FocusDrift,
            "Window focus moved away from expected target during execution",
            "Implement focus re-acquisition or HITL pause on drift",
        )
    } else if lower.contains("port") && (lower.contains("in use") || lower.contains("conflict")) {
        (
            FailureClass::PortConflict,
            "Required port is already in use by another process",
            "Add port conflict detection and alternative port selection",
        )
    } else if lower.contains("lifecycle") || lower.contains("invalid transition") {
        (
            FailureClass::LifecycleDesync,
            "Workflow lifecycle state machine entered invalid state",
            "Audit lifecycle transitions and add invariant monitoring",
        )
    } else if lower.contains("compositor") || lower.contains("wayland") {
        (
            FailureClass::CompositorLimitation,
            "Desktop compositor limitation prevents operation",
            "Add Wayland-specific fallback or degrade gracefully",
        )
    } else {
        (
            FailureClass::EnvironmentInstability,
            "Unclassified runtime failure",
            "Add specific error pattern to failure classifier",
        )
    };

    let severity = match classification {
        FailureClass::NavigationTimeout => FailureSeverity::High,
        FailureClass::RuntimeTimeout => FailureSeverity::High,
        FailureClass::CapabilityMismatch => FailureSeverity::Critical,
        FailureClass::LifecycleDesync => FailureSeverity::Critical,
        FailureClass::WorkflowDeadlock => FailureSeverity::Critical,
        FailureClass::FocusDrift => FailureSeverity::Medium,
        FailureClass::MissingDependency => FailureSeverity::Low,
        FailureClass::PermissionDenied => FailureSeverity::Medium,
        _ => FailureSeverity::Medium,
    };

    ClassifiedFailure {
        scenario_id: scenario_id.to_string(),
        classification,
        severity,
        evidence: error.to_string(),
        probable_cause: probable_cause.to_string(),
        suggested_fix: suggested_fix.to_string(),
        timing_ms: duration_ms,
    }
}
