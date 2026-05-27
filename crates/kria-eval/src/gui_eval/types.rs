//! Core data structures for the GUI Automation Evaluation Framework.

use super::governance::{EvalEnvironmentProfile, EvalGovernanceMetadata};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

// ─── Display Server Requirements ─────────────────────────────────────────────

/// Which display server a test requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayServerRequirement {
    /// Test works on any display server (or no display server).
    Any,
    /// Test requires X11 (xdotool/wmctrl available).
    X11Only,
    /// Test requires Wayland.
    WaylandOnly,
    /// Test works on both X11 and Wayland.
    X11OrWayland,
}

impl DisplayServerRequirement {
    /// Check if this requirement is satisfied by the current environment.
    pub fn is_satisfied(&self) -> bool {
        let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
        let _session = &session;
        let has_display = std::env::var("DISPLAY").is_ok();
        let has_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();

        match self {
            Self::Any => true,
            Self::X11Only => has_display && !has_wayland,
            Self::WaylandOnly => has_wayland,
            Self::X11OrWayland => has_display || has_wayland,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::X11Only => "x11_only",
            Self::WaylandOnly => "wayland_only",
            Self::X11OrWayland => "x11_or_wayland",
        }
    }
}

// ─── Failure Categories ───────────────────────────────────────────────────────

/// Root-cause category for a GUI eval failure.
///
/// These categories map directly to architectural weaknesses in the pipeline.
/// The report uses them to produce a prioritized improvement blueprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCategory {
    /// Intent compiler produced wrong verb/target/content.
    SemanticParsing,
    /// App name extracted incorrectly (e.g., "code and" instead of "code").
    AppResolution,
    /// App not found in InstalledAppRegistry.
    AppNotInstalled,
    /// App is installed but not running; lifecycle handling failed.
    AppLifecycle,
    /// App was already running but KRIA launched a duplicate instead of reusing.
    SessionReuse,
    /// Substrate planner chose wrong substrate (e.g., Keystroke instead of FileWriteThenOpen).
    SubstratePlanning,
    /// Workflow executor failed to execute a step.
    WorkflowExecution,
    /// Verifier reported failure for a step that should have succeeded.
    VerificationFailure,
    /// Window state check failed (IPC error, Wayland incompatibility).
    WindowManagement,
    /// KRIA reported success but the expected artifact/effect is absent.
    FalseSuccess,
    /// KRIA triggered web_search/search_news during a GUI-launch workflow.
    RetrievalLeakage,
    /// LLM cloud retries triggered during a GUI workflow.
    CloudLlmLeakage,
    /// Recovery path not taken when it should have been.
    MissingRecovery,
    /// Deterministic eval invariant was violated.
    InvariantViolation,
    /// Workflow timed out.
    Timeout,
    /// Test was skipped due to environment constraints.
    Skipped,
    /// Required eval capability or environment is unavailable.
    EnvironmentBlocked,
    /// Unknown / uncategorized failure.
    Unknown,
}

impl FailureCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SemanticParsing => "semantic_parsing",
            Self::AppResolution => "app_resolution",
            Self::AppNotInstalled => "app_not_installed",
            Self::AppLifecycle => "app_lifecycle",
            Self::SessionReuse => "session_reuse",
            Self::SubstratePlanning => "substrate_planning",
            Self::WorkflowExecution => "workflow_execution",
            Self::VerificationFailure => "verification_failure",
            Self::WindowManagement => "window_management",
            Self::FalseSuccess => "false_success",
            Self::RetrievalLeakage => "retrieval_leakage",
            Self::CloudLlmLeakage => "cloud_llm_leakage",
            Self::MissingRecovery => "missing_recovery",
            Self::InvariantViolation => "invariant_violation",
            Self::Timeout => "timeout",
            Self::Skipped => "skipped",
            Self::EnvironmentBlocked => "environment_blocked",
            Self::Unknown => "unknown",
        }
    }

    /// Priority rank for the improvement blueprint (lower = fix first).
    pub fn priority(&self) -> u8 {
        match self {
            Self::FalseSuccess => 1,
            Self::RetrievalLeakage => 2,
            Self::CloudLlmLeakage => 3,
            Self::AppResolution => 4,
            Self::SemanticParsing => 5,
            Self::SubstratePlanning => 6,
            Self::AppLifecycle => 7,
            Self::SessionReuse => 8,
            Self::WorkflowExecution => 9,
            Self::VerificationFailure => 10,
            Self::WindowManagement => 11,
            Self::MissingRecovery => 12,
            Self::InvariantViolation => 13,
            Self::AppNotInstalled => 14,
            Self::Timeout => 15,
            Self::Skipped => 16,
            Self::EnvironmentBlocked => 16,
            Self::Unknown => 16,
        }
    }
}

// ─── Eval Case ────────────────────────────────────────────────────────────────

/// A single GUI automation eval case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiEvalCase {
    /// Unique identifier (e.g., "gui-001-open-chrome-youtube").
    pub id: String,
    /// Human-readable description.
    pub description: String,
    /// The user prompt to send to KRIA.
    pub prompt: String,
    /// What the eval expects to happen.
    pub expected_behavior: ExpectedBehavior,
    /// Display server requirement.
    pub display_server: DisplayServerRequirement,
    /// Category tags for filtering and reporting.
    pub tags: Vec<String>,
    /// Whether this test requires a real running desktop (not CI-safe).
    pub requires_desktop: bool,
    /// Maximum allowed duration for this test.
    pub timeout: Duration,
    /// Phase 1 production eval governance metadata.
    #[serde(default)]
    pub governance: EvalGovernanceMetadata,
}

/// What the eval expects to observe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedBehavior {
    /// Expected substrate (if known).
    pub substrate: Option<String>,
    /// Files that should exist after execution.
    pub expected_artifacts: Vec<ExpectedArtifact>,
    /// Tools that MUST have been called.
    pub required_tools: Vec<String>,
    /// Tools that MUST NOT have been called (retrieval isolation).
    pub forbidden_tools: Vec<String>,
    /// The response must NOT contain these strings (false-success detection).
    pub forbidden_response_patterns: Vec<String>,
    /// The response MUST contain at least one of these strings.
    pub required_response_patterns: Vec<String>,
    /// Whether the workflow should complete successfully.
    pub expect_success: bool,
    /// Whether the app should be detected as already running.
    pub app_already_running: bool,
}

/// An artifact that should exist after execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedArtifact {
    /// Path pattern (may contain wildcards like `~/.kria/generated/fibonacci_*.py`).
    pub path_pattern: String,
    /// Substring that must appear in the file content.
    pub content_contains: Option<String>,
    /// Minimum file size in bytes.
    pub min_size_bytes: Option<u64>,
}

// ─── Workflow Trace ───────────────────────────────────────────────────────────

/// A structured trace of a GUI workflow execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiWorkflowTrace {
    /// Which substrate was selected.
    pub substrate_selected: Option<String>,
    /// Steps executed.
    pub steps_executed: Vec<WorkflowStepTrace>,
    /// Tools called during the workflow (including ReAct loop tools).
    pub tools_called: Vec<String>,
    /// Whether any retrieval tools were called (leakage detection).
    pub retrieval_tools_called: Vec<String>,
    /// Whether cloud LLM was invoked.
    pub cloud_llm_invoked: bool,
    /// Number of LLM retries.
    pub llm_retry_count: u32,
    /// HITL requests observed by the eval auto-approver.
    #[serde(default)]
    pub hitl_requests_observed: u32,
    /// HITL requests approved by the eval auto-approver.
    #[serde(default)]
    pub hitl_auto_approved: u32,
    /// HITL requests still pending after workflow execution.
    #[serde(default)]
    pub hitl_pending_after: u32,
    /// Final response text.
    pub final_response: String,
    /// Duration of the entire workflow.
    pub duration_ms: u64,
    /// Whether the workflow reported success.
    pub reported_success: bool,
    /// Artifacts created.
    pub artifacts_created: Vec<PathBuf>,
}

/// Trace of a single workflow step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStepTrace {
    pub step: usize,
    pub action: String,
    pub success: bool,
    pub error: Option<String>,
    pub verification_result: Option<VerificationTrace>,
    pub duration_ms: u64,
}

/// Trace of a verification attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationTrace {
    pub kind: String,
    pub verified: bool,
    pub confidence: f32,
    pub evidence: String,
    pub retries: u32,
}

// ─── Observation ─────────────────────────────────────────────────────────────

/// What was observed during a GUI eval case execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiEvalObservation {
    pub case_id: String,
    /// Structured preflight result for capability/environment gating.
    #[serde(default)]
    pub preflight: GuiEvalPreflight,
    /// Structured workflow trace.
    pub trace: GuiWorkflowTrace,
    /// Raw stream events (for debugging).
    pub raw_events: Vec<serde_json::Value>,
    /// Artifacts found on disk after execution.
    pub artifacts_found: Vec<ArtifactObservation>,
    /// App lifecycle state at time of execution.
    pub app_lifecycle_state: AppLifecycleState,
    /// Display server detected.
    pub display_server_detected: String,
    /// Timing breakdown.
    pub timings: TimingBreakdown,
}

/// An artifact found on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactObservation {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub content_preview: String,
    pub content_matches_expected: bool,
}

/// App lifecycle state at time of execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppLifecycleState {
    /// Whether the target app was running before the test.
    pub was_running_before: bool,
    /// Whether the target app is running after the test.
    pub is_running_after: bool,
    /// PID of the app (if running).
    pub pid: Option<u32>,
    /// Whether KRIA detected the existing session.
    pub session_reused: bool,
}

/// Timing breakdown for a GUI eval case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingBreakdown {
    pub total_ms: u64,
    pub intent_compilation_ms: u64,
    pub substrate_planning_ms: u64,
    pub workflow_execution_ms: u64,
    pub verification_ms: u64,
}

// ─── Preflight Capability Gating ────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuiEvalPreflightStatus {
    Runnable,
    EnvironmentBlocked,
}

impl Default for GuiEvalPreflightStatus {
    fn default() -> Self {
        Self::Runnable
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GuiEvalEnvironmentClassification {
    pub detected_display_server: String,
    pub session_type: Option<String>,
    pub has_display: bool,
    pub has_wayland_display: bool,
    pub kria_eval_gui_enabled: bool,
    pub kria_eval_vm_enabled: bool,
    pub xdotool_available: bool,
    pub wmctrl_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiEvalPreflightCheck {
    pub capability: String,
    pub required: bool,
    pub available: bool,
    pub blocker_kind: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GuiEvalPreflight {
    pub status: GuiEvalPreflightStatus,
    pub required_environment_profile: Option<EvalEnvironmentProfile>,
    pub environment: GuiEvalEnvironmentClassification,
    pub required_capabilities: Vec<String>,
    pub missing_capabilities: Vec<String>,
    pub blocking_reasons: Vec<String>,
    pub checks: Vec<GuiEvalPreflightCheck>,
}

// ─── Verdict ─────────────────────────────────────────────────────────────────

/// The verdict for a GUI eval case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiEvalVerdict {
    pub case_id: String,
    pub kind: GuiEvalVerdictKind,
    /// Root-cause category (for the improvement blueprint).
    pub failure_category: Option<FailureCategory>,
    /// Human-readable explanation.
    pub explanation: String,
    /// Evidence supporting the verdict.
    pub evidence: Vec<String>,
    /// Recommended fix (architectural).
    pub recommended_fix: Option<String>,
    /// Execution quality score (0.0–1.0).
    pub quality_score: f32,
}

/// The outcome of a GUI eval case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuiEvalVerdictKind {
    /// All assertions passed, artifacts verified.
    Pass,
    /// Test was skipped (environment constraint).
    Skip,
    /// Required capability or environment is unavailable.
    EnvironmentBlocked,
    /// Test failed with a specific root cause.
    Fail,
    /// KRIA reported success but verification found no evidence.
    FalseSuccess,
    /// KRIA triggered retrieval tools during a GUI workflow.
    RetrievalLeakage,
}

impl GuiEvalVerdictKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Skip => "SKIP",
            Self::EnvironmentBlocked => "ENVIRONMENT_BLOCKED",
            Self::Fail => "FAIL",
            Self::FalseSuccess => "FALSE_SUCCESS",
            Self::RetrievalLeakage => "RETRIEVAL_LEAKAGE",
        }
    }

    pub fn is_passing(&self) -> bool {
        matches!(self, Self::Pass)
    }
}
