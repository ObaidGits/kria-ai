//! Core data structures for the Real-World Operational Workflow Eval Framework.
//!
//! ## Success Model
//!
//! Five hierarchical levels of success, each independently measured:
//!
//! ```text
//!  ┌─────────────────────────────────────────────────────────┐
//!  │  Observable  — user can SEE the result                  │  highest
//!  │  Semantic    — user goal was actually achieved          │
//!  │  Workflow    — all workflow stages completed            │
//!  │  Tool        — low-level tool executed without error    │  lowest
//!  └─────────────────────────────────────────────────────────┘
//!  Collaborative — orthogonal: recovery/clarity quality
//! ```
//!
//! The framework scores each level independently. A "PASS" requires
//! semantic + observable success; tool success alone is NOT sufficient.

use serde::{Deserialize, Serialize};
use std::time::Duration;

// ─── Workflow Category ────────────────────────────────────────────────────────

/// Semantic category of a workflow, matching kria-core's WorkflowCategory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalWorkflowCategory {
    Coding,
    Browser,
    FileManagement,
    Terminal,
    Debugging,
    Deployment,
    Email,
    Media,
    SystemConfiguration,
    MultiApp,
    Unknown,
}

impl EvalWorkflowCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Coding => "coding",
            Self::Browser => "browser",
            Self::FileManagement => "file_management",
            Self::Terminal => "terminal",
            Self::Debugging => "debugging",
            Self::Deployment => "deployment",
            Self::Email => "email",
            Self::Media => "media",
            Self::SystemConfiguration => "system_configuration",
            Self::MultiApp => "multi_app",
            Self::Unknown => "unknown",
        }
    }
}

// ─── Safety Classification ────────────────────────────────────────────────────

/// Safety class of a workflow eval case.
///
/// Determines whether the eval can run automatically or needs protection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafetyClass {
    /// Always safe to run; creates no persistent side-effects beyond temp files.
    Safe,
    /// Modifies real files outside temp dirs but is reversible.
    Reversible,
    /// Requires KRIA_EVAL_LIVE=1 opt-in; touches real user state.
    LiveOptIn,
    /// Must run in a sandbox or be mocked; never run against real system.
    SandboxOnly,
    /// Permanently blocked; never execute in eval context.
    Blocked,
}

impl SafetyClass {
    /// Returns true if this case may run without user opt-in.
    pub fn is_auto_runnable(self) -> bool {
        matches!(self, Self::Safe | Self::Reversible)
    }

    /// Returns true if this case requires the KRIA_EVAL_LIVE=1 env var.
    pub fn requires_live_opt_in(self) -> bool {
        matches!(self, Self::LiveOptIn)
    }

    /// Returns true if this case must never execute commands directly.
    pub fn must_mock(self) -> bool {
        matches!(self, Self::SandboxOnly | Self::Blocked)
    }
}

// ─── Success Levels ───────────────────────────────────────────────────────────

/// The five independently-measured dimensions of workflow success.
///
/// These are populated by `WorkflowCognitionScorer` from an observation.
/// A workflow that scores `tool_success=true` but `semantic_success=false`
/// represents a common class of KRIA failure: technically executed but
/// did not fulfil the human intent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowSuccessLevels {
    /// Low-level: at least one tool executed without error.
    pub tool_success: bool,
    /// Mid: all required workflow stages completed.
    pub workflow_success: bool,
    /// High: user goal was actually achieved (semantic contract satisfied).
    pub semantic_success: bool,
    /// Highest: the result is visibly surfaced to the user.
    pub observable_success: bool,
    /// Orthogonal: recovery/clarification was handled correctly.
    /// `None` means no interruption occurred; not applicable.
    pub collaborative_success: Option<bool>,
}

impl WorkflowSuccessLevels {
    /// Returns the overall eval verdict: PASS requires semantic + observable.
    pub fn is_passing(&self) -> bool {
        self.semantic_success && self.observable_success
    }

    /// Returns a short summary string for reports.
    pub fn summary(&self) -> String {
        format!(
            "tool={} workflow={} semantic={} observable={} collaborative={}",
            flag(self.tool_success),
            flag(self.workflow_success),
            flag(self.semantic_success),
            flag(self.observable_success),
            self.collaborative_success.map(flag).unwrap_or("n/a")
        )
    }

    /// Numeric score 0.0–1.0 weighting semantic and observable most heavily.
    pub fn weighted_score(&self) -> f32 {
        let tool: f32 = if self.tool_success { 0.10 } else { 0.0 };
        let workflow = if self.workflow_success { 0.15 } else { 0.0 };
        let semantic = if self.semantic_success { 0.40 } else { 0.0 };
        let observable = if self.observable_success { 0.30 } else { 0.0 };
        let collaborative = match self.collaborative_success {
            Some(true) => 0.05,
            Some(false) => -0.05,
            None => 0.0,
        };
        (tool + workflow + semantic + observable + collaborative).clamp(0.0_f32, 1.0_f32)
    }
}

fn flag(b: bool) -> &'static str {
    if b {
        "✓"
    } else {
        "✗"
    }
}

// ─── Observable Output Contract ───────────────────────────────────────────────

/// A single required observable output — what the user must be able to see.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservableOutputContract {
    /// Human-readable description of what should be visible.
    pub description: String,
    /// The response text must contain at least one of these strings.
    pub response_must_contain: Vec<String>,
    /// A file matching this glob must exist (if applicable).
    pub artifact_path_glob: Option<String>,
    /// Minimum size of artifact in bytes (if applicable).
    pub artifact_min_bytes: Option<u64>,
    /// Content substring that must appear in the artifact.
    pub artifact_content_contains: Option<String>,
    /// Whether this output is mandatory for semantic success.
    pub required: bool,
}

// ─── Semantic Completion Contract ─────────────────────────────────────────────

/// Defines exactly what "done" means for a specific workflow.
///
/// These contracts are compiled constants — not LLM outputs.
/// Each workflow category has a canonical contract; individual test cases
/// may extend or tighten the defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticCompletionContract {
    /// Human-readable summary of what success means for this workflow.
    pub success_definition: String,
    /// Workflow category this contract applies to.
    pub category: EvalWorkflowCategory,
    /// Required observable outputs that must be surfaced to the user.
    pub required_observable_outputs: Vec<ObservableOutputContract>,
    /// Response patterns that prove semantic completion.
    pub semantic_success_signals: Vec<String>,
    /// Response patterns that indicate silent / hollow completion — FAIL.
    pub forbidden_silent_completion_patterns: Vec<String>,
    /// Workflow stages that MUST have been executed (by label or action name).
    pub required_stage_labels: Vec<String>,
    /// The final response must NOT claim success if no artifact/output exists.
    pub require_observable_before_success_claim: bool,
}

// ─── Interruption Scenario ────────────────────────────────────────────────────

/// Type of interruption to inject during a workflow eval.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptionKind {
    /// A modal popup appears (e.g., password dialog).
    ModalPopup { description: String },
    /// The uinput daemon crashes mid-workflow.
    DaemonCrash,
    /// The focused window is stolen by another application.
    WindowFocusTheft { stealer_app: String },
    /// The workflow is deliberately paused by the user.
    UserPause,
    /// A network timeout occurs during a browser step.
    NetworkTimeout,
    /// An IDE freeze prevents further action.
    IdeFreeze,
}

/// An interruption to inject during eval execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptionScenario {
    pub kind: InterruptionKind,
    /// After which stage index the interruption fires (0-based).
    pub inject_after_stage: usize,
    /// Expected recovery behavior from KRIA.
    pub expected_recovery: ExpectedRecovery,
}

/// What recovery behavior is expected after an interruption.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedRecovery {
    /// KRIA should retry automatically.
    AutoRetry { max_attempts: u32 },
    /// KRIA should pause and ask the user.
    PauseAndAsk,
    /// KRIA should surface the error clearly and stop.
    FailGracefully,
    /// KRIA should resume from the checkpoint after the interruption resolves.
    ResumeFromCheckpoint,
}

// ─── Workflow Eval Case ───────────────────────────────────────────────────────

/// A single real-world workflow eval case.
///
/// Unlike `GuiEvalCase` (which tests substrate-level mechanics), a
/// `WorkflowEvalCase` tests whether KRIA fulfilled a HUMAN INTENT
/// correctly, visibly, and semantically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEvalCase {
    /// Unique identifier (e.g. "wf-coding-001-pascal-triangle").
    pub id: String,
    /// Human-readable description.
    pub description: String,
    /// The user prompt sent to KRIA.
    pub prompt: String,
    /// Semantic category of this workflow.
    pub category: EvalWorkflowCategory,
    /// Semantic completion contract.
    pub contract: SemanticCompletionContract,
    /// Safety classification.
    pub safety_class: SafetyClass,
    /// Optional interruption to inject mid-workflow.
    pub interruption: Option<InterruptionScenario>,
    /// Maximum allowed duration.
    pub timeout: Duration,
    /// Whether this test requires the uinput daemon to be running.
    pub requires_daemon: bool,
    /// Whether this test requires a real display server.
    pub requires_display: bool,
    /// Category tags for filtering.
    pub tags: Vec<String>,
    /// Notes for human reviewers explaining what this eval validates.
    pub eval_notes: String,
}

// ─── Workflow Eval Observation ────────────────────────────────────────────────

/// What was observed during a workflow eval execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEvalObservation {
    pub case_id: String,
    /// The final response text from KRIA.
    pub final_response: String,
    /// Tools called during execution (name strings).
    pub tools_called: Vec<String>,
    /// Stage labels that completed successfully.
    pub completed_stage_labels: Vec<String>,
    /// Whether KRIA claimed success in its response.
    pub reported_success: bool,
    /// Whether the interruption scenario was handled (if applicable).
    pub interruption_handled: Option<bool>,
    /// Artifacts found on disk after execution.
    pub artifacts_found: Vec<ArtifactFound>,
    /// Raw error strings from any failed stages.
    pub stage_errors: Vec<String>,
    /// Total wall-clock duration.
    pub duration_ms: u64,
    /// Whether the uinput daemon was alive at start.
    pub daemon_alive_at_start: bool,
    /// Whether the uinput daemon was alive at end.
    pub daemon_alive_at_end: bool,
}

/// A file artifact found on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactFound {
    pub path: String,
    pub size_bytes: u64,
    pub content_preview: String,
}

// ─── Verdict ─────────────────────────────────────────────────────────────────

/// The verdict for a workflow eval case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEvalVerdict {
    pub case_id: String,
    /// Multi-dimensional success levels.
    pub success_levels: WorkflowSuccessLevels,
    /// Overall verdict kind.
    pub kind: WorkflowVerdictKind,
    /// Primary failure reason (if any).
    pub failure_reason: Option<String>,
    /// Evidence strings.
    pub evidence: Vec<String>,
    /// Weighted quality score 0.0–1.0.
    pub quality_score: f32,
    /// Recommended architectural fix.
    pub recommended_fix: Option<String>,
    /// Human-readable explanation of the verdict.
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowVerdictKind {
    /// All semantic and observable assertions passed.
    Pass,
    /// Test skipped due to environment/daemon/display constraints.
    Skip,
    /// Tool executed but semantic/observable goals not achieved.
    SemanticFail,
    /// KRIA claimed success but the result is not observable.
    ObservableFail,
    /// Workflow interrupted and recovery was incorrect or missing.
    RecoveryFail,
    /// KRIA completed silently without surfacing the result to the user.
    SilentCompletion,
    /// KRIA reported success but no verifiable evidence exists.
    FalseSuccess,
    /// Complete workflow failure.
    Fail,
}

impl WorkflowVerdictKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Skip => "SKIP",
            Self::SemanticFail => "SEMANTIC_FAIL",
            Self::ObservableFail => "OBSERVABLE_FAIL",
            Self::RecoveryFail => "RECOVERY_FAIL",
            Self::SilentCompletion => "SILENT_COMPLETION",
            Self::FalseSuccess => "FALSE_SUCCESS",
            Self::Fail => "FAIL",
        }
    }

    pub fn is_passing(&self) -> bool {
        matches!(self, Self::Pass | Self::Skip)
    }
}
