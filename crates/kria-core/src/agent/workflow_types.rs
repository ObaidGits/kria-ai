//! Canonical workflow runtime types — Phase 0 Foundation.
//!
//! These types define the production-grade workflow contracts that replace
//! the ad-hoc string-based frontend communication and dual-truth verification.
//!
//! # Authority
//!
//! This module is the SINGLE source of truth for:
//! - Workflow lifecycle states
//! - Structured telemetry events (frontend contract)
//! - Outcome contracts (plan-bound verification)
//! - Workflow verdicts (completion semantics)
//! - HITL interaction types
//! - Capability declarations
//!
//! # Design Rules
//!
//! - All types are `Serialize + Deserialize` for frontend transport
//! - No LLM calls, no I/O, no async — pure data definitions
//! - Frontend is a pure renderer of these types (never parses strings)
//! - Verdicts are computed deterministically from structural + verification results

use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — Execution Environment (replaces conflated ExecutionTarget)
// ═══════════════════════════════════════════════════════════════════════════════

/// Where shell-equivalent commands physically execute.
/// This is ONLY about the execution environment — NOT about tool categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutionEnvironment {
    /// Local host machine (default)
    Host,
    /// Remote VM via SSH/fleet
    Vm,
    /// Docker container
    Docker,
    /// Google Colab (cloud notebook)
    Colab,
}

impl ExecutionEnvironment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Vm => "vm",
            Self::Docker => "docker",
            Self::Colab => "colab",
        }
    }
}

/// What category of operation a tool performs.
/// Orthogonal to ExecutionEnvironment — informs capability checks, not target validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolCategory {
    Shell,
    Filesystem,
    Browser,
    Desktop,
    Mcp,
    CloudProvider,
    Image,
    Voice,
    Memory,
    Network,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Workflow Lifecycle State Machine
// ═══════════════════════════════════════════════════════════════════════════════

/// Deterministic workflow lifecycle state.
/// Every workflow instance follows this FSM — no exceptions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WorkflowState {
    /// Workflow created, not yet planned
    Created,
    /// Plan generated, ready to execute
    Planned,
    /// Currently executing steps
    Executing { current_step: u32, completed_steps: u32 },
    /// Suspended waiting for human input
    HitlPending { reason: HitlReason, suspended_at_step: u32 },
    /// All steps done, running outcome verification
    Verifying,
    /// Terminal state — verdict computed
    Finalized { verdict: WorkflowVerdict },
    /// Terminal state — user or system cancelled
    Cancelled { reason: String, at_step: u32 },
}

// ═══════════════════════════════════════════════════════════════════════════════
// §3 — Structured Workflow Telemetry (Frontend Contract)
// ═══════════════════════════════════════════════════════════════════════════════

/// Versioned telemetry envelope — the ONLY transport between backend and frontend
/// for workflow-aware interactions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEnvelope {
    /// Protocol version — frontend ignores events with unknown version
    pub version: u8,
    /// Monotonic sequence number for ordering
    pub seq: u64,
    /// The actual event
    pub event: WorkflowTelemetry,
    /// Monotonic timestamp (milliseconds since workflow start)
    pub timestamp_ms: u64,
    /// Which path produced this workflow
    pub source: WorkflowSource,
}

/// Identifies which code path produced the workflow plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowSource {
    /// New substrate router
    SubstrateRouter,
    /// Legacy compatibility shim
    LegacyShim,
    /// ReAct loop (non-workflow path)
    ReactLoop,
}

/// Canonical workflow telemetry event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowTelemetry {
    /// Workflow planned, about to start
    Started {
        workflow_id: String,
        title: String,
        steps: Vec<StepPreview>,
        execution_mode: ExecutionMode,
        estimated_duration_ms: Option<u64>,
    },
    /// Plan preview for user approval (dry-run mode)
    PlanPreview {
        workflow_id: String,
        title: String,
        steps: Vec<StepPreview>,
        outcome_summary: Vec<String>,
        requires_approval: bool,
    },
    /// Step execution started
    StepStarted {
        workflow_id: String,
        step_index: u32,
        description: String,
        step_type: StepType,
    },
    /// Step completed
    StepCompleted {
        workflow_id: String,
        step_index: u32,
        structural_success: bool,
        visibility_confidence: VisibilityConfidence,
        artifacts: Vec<String>,
    },
    /// Human input required
    HitlRequired {
        workflow_id: String,
        reason: HitlReason,
        options: Vec<HitlOption>,
        context: String,
    },
    /// Workflow completed with structured verdict
    Completed {
        workflow_id: String,
        verdict: WorkflowVerdict,
        summary: String,
        artifacts: Vec<String>,
        continuation: Vec<ContinuationAction>,
    },
    /// Workflow cancelled
    Cancelled {
        workflow_id: String,
        reason: String,
        completed_steps: u32,
        total_steps: u32,
    },
}

/// Preview of a workflow step (shown before/during execution).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepPreview {
    pub index: u32,
    pub description: String,
    pub step_type: StepType,
    pub execution_mode: StepExecutionMode,
}

/// Classification of step type for UI rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepType {
    FileWrite,
    AppLaunch,
    CommandExecution,
    BrowserNavigation,
    Interaction,
    Verification,
}

/// Per-step execution mode — determines visibility expectations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepExecutionMode {
    /// Backend-only. User does not expect to see this happen.
    Backend,
    /// User expects to see this happen on screen.
    Visible,
    /// Backend execution with visible surfacing at the end.
    HybridSurface,
    /// Requires GUI interaction (clicks, typing into focused app).
    Interactive,
}

/// Overall workflow execution mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// All steps are backend/structural
    Structural,
    /// Mix of backend and visible steps
    Hybrid { visible_steps: Vec<u32> },
    /// Entire workflow is visible GUI
    Visible,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §4 — Visibility Confidence (Graded, Not Binary)
// ═══════════════════════════════════════════════════════════════════════════════

/// Graded visibility confidence for a completed step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "level", rename_all = "snake_case")]
pub enum VisibilityConfidence {
    /// Verified visible with strong evidence
    Confirmed { confidence: f32, evidence: String },
    /// Structurally succeeded but visibility unverifiable in this environment
    StructuralOnly { reason: String },
    /// Verification attempted but inconclusive
    Inconclusive { reason: String, suggestion: Option<String> },
    /// Not applicable (backend-only step)
    NotApplicable,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §5 — Workflow Verdict (Single Source of Truth)
// ═══════════════════════════════════════════════════════════════════════════════

/// The canonical workflow completion verdict.
/// Computed ONCE by the WorkflowFinalizer. Never re-derived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowVerdict {
    /// All steps succeeded, all visibility contracts met
    Complete,
    /// Target state was already true before execution
    AlreadySatisfied { evidence: String },
    /// All steps succeeded structurally, some visibility unverifiable
    StructurallyComplete { unverified_outcomes: Vec<String> },
    /// Partial completion — some steps succeeded
    Partial { completed: u32, total: u32, reason: String },
    /// Workflow blocked — needs human intervention (should not reach finalizer)
    Blocked { reason: String },
    /// Workflow failed at a specific step
    Failed { step: u32, reason: String, recovery: Option<RecoveryPath> },
}

/// Suggested recovery path after a failure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoveryPath {
    pub description: String,
    pub actions: Vec<ContinuationAction>,
}

/// An action the user can take after workflow completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContinuationAction {
    pub id: String,
    pub label: String,
    pub action_type: ContinuationActionType,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContinuationActionType {
    BringToFront { app: String },
    OpenUrl { url: String },
    RetryStep { step_index: u32 },
    OpenFile { path: String },
    ShowOutput { content: String },
    RetryWorkflow,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §6 — HITL (Human-In-The-Loop) Types
// ═══════════════════════════════════════════════════════════════════════════════

/// Why human input is needed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HitlReason {
    /// App is not installed
    InstallRequired { app: String, install_command: Option<String> },
    /// Login/authentication needed
    LoginRequired { service: String, guidance: String },
    /// Session expired
    SessionExpired { service: String },
    /// Multiple possible targets
    AmbiguousTarget { options: Vec<String>, question: String },
    /// Execution mode choice (GUI vs backend)
    ExecutionModeChoice { task: String, backend_option: String, gui_option: String },
    /// Destructive action needs approval
    ApprovalNeeded { action: String, risk_level: String, description: String },
    /// Visibility cannot be confirmed
    VisibilityUncertain { step_description: String, suggestion: String },
    /// Focus was lost during a visible step
    FocusLost { step_description: String },
    /// Manual step needed (e.g., typing on Wayland)
    ManualStepNeeded { instruction: String, context: String },
    /// Intent unclear after LLM parsing
    IntentUnclear { original_text: String, what_understood: String, suggestion: String },
    /// Workflow taking too long
    BudgetExhausted { elapsed_ms: u64, remaining_steps: u32 },
    /// Accessibility setup needed (first-time)
    AccessibilitySetup { current_state: String, impact: String },
    /// Step failed with recovery options
    StepFailed { step_description: String, error: String },
}

/// An option presented to the user in a HITL prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlOption {
    /// Unique identifier for this option
    pub id: String,
    /// Human-readable button label
    pub label: String,
    /// What happens when clicked
    pub action_type: HitlActionType,
}

/// What action a HITL option triggers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HitlActionType {
    /// Approve and continue
    Approve,
    /// Deny and stop
    Deny,
    /// Retry the failed step
    Retry,
    /// Skip the current step
    Skip,
    /// Choose an alternative (e.g., different app)
    ChooseAlternative { value: String },
    /// Open a URL (e.g., login page)
    OpenUrl { url: String },
    /// Run a pre-registered command (validated against allowlist)
    RunCommand { command: String },
    /// User completed a manual step
    ManualComplete,
    /// Cancel the workflow
    Cancel,
}

/// User's response to a HITL prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlResponse {
    pub workflow_id: String,
    pub option_id: String,
    pub action_type: HitlActionType,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §7 — Outcome Contract (Plan-Bound, Never Re-Derived)
// ═══════════════════════════════════════════════════════════════════════════════

/// Plan-bound outcome contract — declares what must be true for success.
/// Generated at plan time by the substrate router. Consumed by the verifier.
/// NEVER re-derived from user text after planning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutcomeContract {
    /// Outcomes that MUST be true for the workflow to succeed
    pub required: Vec<PlannedOutcome>,
    /// Outcomes that SHOULD be true for full fidelity (visible expectations)
    pub desired: Vec<PlannedOutcome>,
}

impl OutcomeContract {
    pub fn empty() -> Self {
        Self {
            required: Vec::new(),
            desired: Vec::new(),
        }
    }

    pub fn all_outcomes(&self) -> impl Iterator<Item = &PlannedOutcome> {
        self.required.iter().chain(self.desired.iter())
    }
}

/// A single planned outcome with its verification method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedOutcome {
    /// Human-readable description
    pub description: String,
    /// What we expect to be true
    pub expectation: OutcomeExpectation,
    /// Minimum confidence to consider verified
    pub min_confidence: f32,
    /// What to do if verification fails
    pub on_failure: OutcomeFailurePolicy,
}

/// What observable state is expected.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutcomeExpectation {
    FileExists { path: String },
    ProcessRunning { binary: String },
    AppWindowVisible { app: String, title_hint: Option<String> },
    BrowserAtUrl { url_contains: String },
    OutputContains { substring: String, in_file: String },
    PortListening { port: u16 },
}

/// What to do when an outcome verification fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeFailurePolicy {
    /// Downgrade fidelity but don't fail the workflow
    DowngradeFidelity,
    /// Fail the workflow
    FailWorkflow,
    /// Ignore (optional outcome)
    Ignore,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §8 — Capability Set (Resolved Once Per Workflow)
// ═══════════════════════════════════════════════════════════════════════════════

/// Complete capability snapshot for a workflow.
/// Resolved once at plan time, cached in workflow memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitySet {
    pub environment: EnvironmentCapability,
    pub verifier: VerifierCapability,
    pub interaction: InteractionCapability,
}

/// Desktop environment capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentCapability {
    pub session_type: SessionType,
    pub compositor: Option<String>,
    pub atspi_level: AtSpiLevel,
    pub xdotool_available: bool,
    pub uinput_available: bool,
    pub ocr_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionType {
    X11,
    Wayland,
    XWayland,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AtSpiLevel {
    /// Full accessibility stack operational
    Full,
    /// Bus available, only some apps expose trees
    Partial { accessible_apps: Vec<String> },
    /// Bus exists but no apps detected
    BusOnly,
    /// AT-SPI completely unavailable
    None,
}

/// What verification methods are available.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifierCapability {
    pub available_methods: Vec<VerificationMethod>,
    pub window_state_max_confidence: f32,
    pub cdp_available: bool,
    /// Always true on Linux
    pub filesystem_available: bool,
    /// Always true on Linux
    pub process_table_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationMethod {
    AtSpi,
    Xdotool,
    Cdp,
    ProcessTable,
    FileSystem,
    Ocr,
    PortCheck,
}

/// Input injection capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionCapability {
    pub keyboard_injection: InputInjectionLevel,
    pub mouse_injection: InputInjectionLevel,
    pub clipboard_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputInjectionLevel {
    /// Full injection via uinput daemon
    Full,
    /// Partial via xdotool (X11 only)
    XdotoolOnly,
    /// Not available
    None,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §9 — Failure Policy (With Bounded Retry)
// ═══════════════════════════════════════════════════════════════════════════════

/// What to do when a workflow step fails.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FailurePolicy {
    /// Workflow stops immediately
    Fatal,
    /// Retry with exponential backoff, then fail
    RetryThenFatal { max_attempts: u8, initial_backoff_ms: u64 },
    /// Retry with backoff, then skip
    RetryThenSkip { max_attempts: u8, initial_backoff_ms: u64 },
    /// Skip this step, continue with next
    Skippable,
    /// Ask the user what to do
    AskUser { question: String },
}

// ═══════════════════════════════════════════════════════════════════════════════
// §10 — Confidence Grading
// ═══════════════════════════════════════════════════════════════════════════════

/// Graded confidence in a verification result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceGrade {
    /// Strong evidence (filesystem, process table, CDP) — confidence ≥ 0.85
    Strong,
    /// Moderate evidence (AT-SPI, window class match) — confidence 0.60–0.84
    Moderate,
    /// Weak evidence (OCR, heuristic) — confidence 0.30–0.59
    Weak,
    /// No evidence (probe failed, timed out, unavailable) — confidence < 0.30
    NoEvidence,
}

impl ConfidenceGrade {
    pub fn from_confidence(c: f32) -> Self {
        if c >= 0.85 { Self::Strong }
        else if c >= 0.60 { Self::Moderate }
        else if c >= 0.30 { Self::Weak }
        else { Self::NoEvidence }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §11 — Constants
// ═══════════════════════════════════════════════════════════════════════════════

/// Current telemetry protocol version.
pub const TELEMETRY_VERSION: u8 = 1;

/// Maximum retry attempts per step (hard cap).
pub const MAX_RETRY_ATTEMPTS: u8 = 3;

/// Maximum backoff between retries (ms).
pub const MAX_RETRY_BACKOFF_MS: u64 = 5000;

/// Default workflow budget (ms) — 2 minutes.
pub const DEFAULT_WORKFLOW_BUDGET_MS: u64 = 120_000;

/// HITL debounce window (ms).
pub const HITL_DEBOUNCE_WINDOW_MS: u64 = 3000;

/// Maximum HITL prompts before batching.
pub const HITL_BATCH_THRESHOLD: usize = 5;

/// Telemetry channel capacity.
pub const TELEMETRY_CHANNEL_CAPACITY: usize = 64;
