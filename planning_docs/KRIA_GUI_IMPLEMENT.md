# KRIA Production GUI Cognition Implementation Architecture

**Version**: 2.0  
**Date**: 2026-05-27  
**Status**: Production-Hardened Implementation Document  
**Scope**: Complete frontend + backend GUI cognition architecture  
**Hardening**: 25 vulnerability mitigations integrated

---

## Executive Summary

KRIA must evolve from a structural workflow executor with bolted-on verification into a **bounded operational cognition system** — one that understands workflow intent, reasons about capabilities, executes hybrid workflows honestly, and collaborates with the user when it cannot proceed alone.

This document defines the complete architecture for that evolution. It is grounded in:

- The audit findings (dual-truth contradiction, category errors, overengineered planner ecosystem, Wayland verification impossibility)
- The correction table (30 identified weaknesses in the audit's own recommendations)
- Real-world workflow expectations (IDE, browser, filesystem, communication, account-aware)

### Core Architectural Thesis

**KRIA is a workflow collaborator, not a workflow executor.**

The difference:
- An executor runs steps and reports success/failure.
- A collaborator understands what the user expects to see, knows what it can and cannot do in this environment, executes the achievable parts, and honestly negotiates the rest with the user.

### What Changes

| From (Current) | To (Target) |
|---|---|
| Two sources of completion truth | One plan-bound outcome contract |
| Re-derived visibility verification | Plan-emitted verification leaves |
| ExecutionTarget conflating environment + category | Separate ExecutionEnvironment + ToolCategory |
| 5+ planners with overlapping scope | One substrate router + capability adapter |
| Binary success/failure | Graded workflow verdict with structured telemetry |
| Implicit workflow state | Explicit lifecycle state machine |
| No capability awareness | Centralized capability negotiation |
| No HITL for visibility uncertainty | Collaborative recovery flows |
| Wayland-unaware verification | Compositor-aware observation abstraction |
| Natural-language frontend contract | Structured workflow telemetry events |

---

## 1. What KRIA Should Actually Become

### The Bounded Operational Cognition Model

KRIA should be an **intelligent collaborative desktop workflow assistant** that:

1. **Understands workflow intent** — not just "open app X" but "the user wants to see a running website in their IDE"
2. **Reasons about execution modes** — backend-safe vs visible vs hybrid, chosen per-step not per-workflow
3. **Knows its capabilities** — what it can do, verify, and recover from in this specific environment
4. **Executes honestly** — never claims success it cannot verify, never hides failure behind partial narratives
5. **Collaborates naturally** — when blocked (login needed, app missing, ambiguous target), asks the user with actionable options
6. **Recovers gracefully** — partial completion is a valid state with clear continuation paths

### What "Bounded Operational Cognition" Means

It is NOT:
- AGI-style reasoning about arbitrary goals
- Recursive self-improvement or meta-planning
- Unbounded exploration of action spaces
- Semantic graph traversal over world models

It IS:
- Deterministic substrate routing with capability-aware adaptation
- Graded confidence in verification outcomes
- Structured collaboration when confidence is low
- Honest reporting of what was achieved vs what was expected
- Environment-aware execution mode selection

### The Five Pillars

```text
┌─────────────────────────────────────────────────────────┐
│                    KRIA Runtime Spine                     │
├─────────────┬──────────────┬──────────────┬─────────────┤
│  Workflow   │  Capability  │   Hybrid     │    HITL     │
│ Intelligence│  Negotiation │  Execution   │ Collaboration│
│             │              │              │             │
│ Intent →    │ Environment →│ Backend +    │ Uncertainty →│
│ Substrate → │ App →        │ GUI +        │ Recovery →  │
│ Contract    │ Verifier →   │ Visible      │ Continuation│
│             │ Capability   │ Surfacing    │             │
│             │ Set          │              │             │
└─────────────┴──────────────┴──────────────┴─────────────┘
         │              │              │              │
         └──────────────┴──────────────┴──────────────┘
                              │
                    Workflow Lifecycle FSM
                              │
                    Structured Telemetry → Frontend
```

---

## 2. Current Failure Analysis

### Why Existing GUI Cognition Fails

The audit identified four root causes. This section refines them with the correction table's feedback:

#### 2.1 Dual-Truth Contradiction

**Problem**: HTN executor declares structural success → OCE re-derives outcomes from user text → verifier checks focus state → contradicts executor.

**Root cause**: No single outcome contract flows from planner through executor to verifier.

**Corrected solution**: Plan-bound `OutcomeContract` attached to `GuiWorkflow` at plan time. The OCE consumes this contract — it does not re-derive. When no workflow exists (ReAct path), OCE may still infer outcomes, but this is explicitly marked as "advisory" not "authoritative".

#### 2.2 Focus Verification After Focus Loss

**Problem**: Verifier asks "is the focused window X?" after KRIA's own UI has reclaimed focus.

**Root cause**: No foreground lease during verification; no causal binding from launch to window.

**Corrected solution**: 
- Foreground lease acquired before launch, held through verification
- Causal launch handle (PID + expected window class) returned from launcher
- Compositor-aware observation abstraction (not just xdotool/AT-SPI)
- Graded confidence rather than binary pass/fail

#### 2.3 ExecutionTarget Category Error

**Problem**: `ExecutionTarget::Browser` is both "where commands run" and "what kind of tool this is", causing `EXECUTION_BLOCKED` for legitimate browser tools.

**Root cause**: Single enum conflating execution environment with tool category.

**Corrected solution**: Split into:
- `ExecutionEnvironment` { Host, Vm, Docker, Colab } — where shell-equivalent commands physically run
- `ToolCategory` { Shell, Filesystem, Browser, Desktop, Mcp, CloudProvider, Image } — what kind of operation
- Execution authority validates `(tool_name, environment)` pairs only
- Tool category informs capability checks, not target validation

#### 2.4 Planner Ecosystem Sprawl

**Problem**: 5+ planners with overlapping scope, no clear authority hierarchy.

**Corrected solution** (per correction table item #2 — classify before deleting):

| Module | Classification | Action |
|--------|---------------|--------|
| `gui_substrate_planner.rs` | Production (working) | Keep, split into modules |
| `intent_compiler_rule.rs` | Production (working) | Keep as primary fast-path |
| `intent_compiler_llm.rs` | Production (fallback) | Keep as LLM fallback |
| `htn_executor.rs` | Production (working) | Keep, extract tests |
| `planner.rs` | Dead experimental | Remove after characterization tests |
| `planner_v2/mod.rs` | Dead experimental | Remove after characterization tests |
| `gui_planner.rs` | Dead experimental | Remove after characterization tests |
| `workflow_compiler.rs` | Partially useful | Extract useful types, remove orchestration |
| `semantic_workflow.rs` | Dead experimental | Remove |
| `opgraph.rs` / `opgraph_compiler.rs` | Dead experimental | Remove |

---

## 3. Frontend ↔ Backend Workflow Truth

### The Synchronization Problem

Today the frontend receives natural-language strings via `StreamEvent::Token` / `StreamEvent::Done` / `StreamEvent::Error`. The shape of the message implicitly carries the contract. This is insufficient.

### Structured Workflow Telemetry Model

The backend emits a **typed workflow telemetry stream** that the frontend renders:

```rust
/// Canonical workflow state event — the ONLY contract between backend and frontend
/// for workflow-aware interactions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowTelemetry {
    /// Workflow has been planned and is about to start
    Started {
        workflow_id: String,
        title: String,
        steps: Vec<StepPreview>,
        execution_mode: ExecutionMode,
        estimated_duration_ms: Option<u64>,
    },
    /// A step has begun execution
    StepStarted {
        workflow_id: String,
        step_index: u32,
        description: String,
        step_type: StepType,
    },
    /// A step completed (structurally)
    StepCompleted {
        workflow_id: String,
        step_index: u32,
        structural_success: bool,
        visibility_confidence: VisibilityConfidence,
        artifacts: Vec<Artifact>,
    },
    /// Workflow needs human input to continue
    HitlRequired {
        workflow_id: String,
        reason: HitlReason,
        options: Vec<HitlOption>,
        context: String,
    },
    /// Workflow completed with a structured verdict
    Completed {
        workflow_id: String,
        verdict: WorkflowVerdict,
        summary: String,
        artifacts: Vec<Artifact>,
        continuation: Option<ContinuationHint>,
    },
    /// Workflow was cancelled
    Cancelled {
        workflow_id: String,
        reason: String,
        completed_steps: u32,
        total_steps: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecutionMode {
    /// All steps are backend/structural — no visible GUI expected
    Structural,
    /// Some steps require visible GUI interaction
    Hybrid { visible_steps: Vec<u32> },
    /// Entire workflow is visible GUI
    Visible,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VisibilityConfidence {
    /// Verified visible (AT-SPI confirmed, CDP confirmed, etc.)
    Confirmed { confidence: f32, evidence: String },
    /// Structurally succeeded but visibility unverifiable in this environment
    StructuralOnly { reason: String },
    /// Verification attempted but inconclusive
    Inconclusive { reason: String, suggestion: Option<String> },
    /// Not applicable (backend-only step)
    NotApplicable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowVerdict {
    /// All steps succeeded, all visibility contracts met
    Complete,
    /// All steps succeeded structurally, some visibility unverifiable
    StructurallyComplete { unverified_outcomes: Vec<String> },
    /// Partial completion — some steps succeeded
    Partial { completed: u32, total: u32, reason: String },
    /// Workflow blocked — needs human intervention
    Blocked { reason: String, recovery: Option<RecoveryPath> },
    /// Workflow failed
    Failed { step: u32, reason: String, recovery: Option<RecoveryPath> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HitlReason {
    LoginRequired { service: String, guidance: String },
    InstallRequired { app: String, install_command: Option<String> },
    AmbiguousTarget { options: Vec<String>, question: String },
    ApprovalNeeded { action: String, risk_level: String },
    VisibilityUncertain { description: String },
    AccountSessionExpired { service: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlOption {
    pub id: String,
    pub label: String,
    pub action_type: HitlActionType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HitlActionType {
    Approve,
    Deny,
    Retry,
    Skip,
    ChooseAlternative { value: String },
    OpenUrl { url: String },
    RunCommand { command: String },
    Cancel,
}
```

### Frontend Rendering Contract

The frontend renders `WorkflowTelemetry` events into:
- **Progress indicators** (step N of M, with per-step status)
- **HITL modals** (login prompts, install prompts, approval dialogs)
- **Verdict badges** (complete ✓, structural ⚙, partial ⚠, failed ✗)
- **Continuation actions** (retry, skip, alternative)

The frontend NEVER parses natural-language strings to determine workflow state.

---

## 4. Runtime Spine Architecture

### The Single Execution Path

Every workflow — GUI, backend, hybrid — flows through one deterministic spine:

```text
User Text
    │
    ▼
┌─────────────────┐
│  Intent Compiler │ ← Rule-based fast path + LLM fallback
│  (deterministic) │   Produces: WorkflowIntent
└────────┬────────┘
         │
         ▼
┌─────────────────────┐
│ Capability Negotiator│ ← Queries environment, app registry, verifier caps
│   (deterministic)    │   Produces: CapabilitySet for this workflow
└────────┬────────────┘
         │
         ▼
┌─────────────────────┐
│  Substrate Router    │ ← Deterministic routing based on intent + capabilities
│  (deterministic)     │   Produces: WorkflowPlan with OutcomeContract
└────────┬────────────┘
         │
         ▼
┌─────────────────────┐
│  Workflow Executor   │ ← Runs steps, holds leases, emits telemetry
│  (async, bounded)    │   Consumes: WorkflowPlan
└────────┬────────────┘
         │
         ▼
┌─────────────────────┐
│  Outcome Verifier    │ ← Verifies plan-bound outcomes using capability-gated probes
│  (bounded, modular)  │   Consumes: OutcomeContract + CapabilitySet
└────────┬────────────┘
         │
         ▼
┌─────────────────────┐
│  Workflow Finalizer  │ ← Single source of verdict truth
│  (deterministic)     │   Produces: WorkflowVerdict + WorkflowTelemetry::Completed
└─────────────────────┘
```

### Key Invariants

1. **One path, always.** No parallel planners. No fallback planners. If the substrate router cannot plan, it returns `WorkflowPlan::Unplannable { reason }` and the finalizer emits `Blocked`.
2. **Plan owns the contract.** `OutcomeContract` is generated at plan time, threaded through execution, consumed by verification. Never re-derived.
3. **Capabilities gate everything.** The substrate router receives `CapabilitySet` and adapts its plan accordingly. It never emits a verification leaf the environment cannot satisfy.
4. **Telemetry is the frontend contract.** Every state transition emits `WorkflowTelemetry`. The frontend is a pure renderer of this stream.
5. **HITL is a first-class state.** When the executor encounters a blocker, it emits `HitlRequired` and suspends. The workflow resumes when the user responds.

---

## 5. Workflow Lifecycle Architecture

### Deterministic State Machine

Every workflow instance follows this state machine. No exceptions.

```text
                    ┌──────────┐
                    │  Created │
                    └────┬─────┘
                         │ plan()
                         ▼
                    ┌──────────┐
              ┌─────│  Planned │
              │     └────┬─────┘
              │          │ execute()
              │          ▼
              │     ┌──────────┐
              │     │ Executing│◄────────────────┐
              │     └────┬─────┘                 │
              │          │                       │
              │     ┌────┴────┐                  │
              │     │         │                  │
              │     ▼         ▼                  │
              │ ┌───────┐ ┌────────┐             │
              │ │StepOk │ │StepFail│             │
              │ └───┬───┘ └───┬────┘             │
              │     │         │                  │
              │     │    ┌────┴────┐             │
              │     │    │         │             │
              │     │    ▼         ▼             │
              │     │ ┌──────┐ ┌───────┐        │
              │     │ │ Hitl │ │Failed │        │
              │     │ └──┬───┘ └───────┘        │
              │     │    │                       │
              │     │    │ user_responds()       │
              │     │    └───────────────────────┘
              │     │
              │     │ all_steps_done()
              │     ▼
              │ ┌──────────┐
              │ │ Verifying│
              │ └────┬─────┘
              │      │ verdict()
              │      ▼
              │ ┌──────────┐
              └►│ Finalized│
                └──────────┘
```

### State Definitions

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowState {
    Created,
    Planned { plan: WorkflowPlan },
    Executing { current_step: u32, completed: Vec<StepResult> },
    HitlPending { reason: HitlReason, suspended_at_step: u32 },
    Verifying { structural_results: Vec<StepResult> },
    Finalized { verdict: WorkflowVerdict },
    Cancelled { reason: String, at_step: u32 },
}
```

### Transition Rules

| From | To | Trigger | Invariant |
|------|-----|---------|-----------|
| Created | Planned | `plan()` succeeds | CapabilitySet must be resolved |
| Created | Finalized(Failed) | `plan()` fails | Unplannable reason recorded |
| Planned | Executing | `execute()` called | Foreground lease acquired if needed |
| Executing | Executing | Step completes successfully | Step index increments |
| Executing | HitlPending | Step needs human input | Telemetry emitted, executor suspends |
| Executing | Finalized(Failed) | Step fails irrecoverably | Abort steps run if defined |
| Executing | Verifying | All steps complete | Structural results collected |
| HitlPending | Executing | User responds | Response applied, execution resumes |
| HitlPending | Cancelled | User cancels | Cleanup runs |
| Verifying | Finalized | Verdict computed | Single source of truth |
| Any | Cancelled | User cancels | Cleanup runs, partial artifacts preserved |

### Cancellation Semantics

```rust
pub struct CancellationContract {
    /// Whether in-flight steps should be interrupted immediately
    pub interrupt_in_flight: bool,
    /// Cleanup actions to run on cancellation
    pub cleanup_steps: Vec<CleanupStep>,
    /// Whether partial artifacts should be preserved
    pub preserve_artifacts: bool,
    /// Maximum time to wait for graceful shutdown
    pub grace_period_ms: u64,
}
```

### Workflow-Scoped Memory

Each workflow instance carries bounded runtime memory:

```rust
pub struct WorkflowMemory {
    /// Artifacts created by completed steps (file paths, PIDs, window handles)
    pub artifacts: Vec<WorkflowArtifact>,
    /// Resolved capabilities at plan time (cached, not re-queried)
    pub capabilities: CapabilitySet,
    /// HITL decisions made during this workflow
    pub decisions: Vec<HitlDecision>,
    /// Monotonic timestamp of last state transition
    pub last_transition_at: Instant,
    /// Total elapsed time budget remaining
    pub budget_remaining_ms: u64,
}
```

---

## 6. Workflow Intelligence Model

### What "Intelligence" Means Here

Workflow intelligence is the system's ability to choose the **right execution strategy** for a given intent in a given environment. It is NOT planning in the AI sense. It is deterministic routing with capability-aware adaptation.

### The Intelligence Stack

```text
Layer 1: Intent Classification
    "What does the user want to achieve?"
    → WorkflowIntent { verb, targets, content, visibility_expectation }

Layer 2: Execution Mode Selection
    "Should this be visible, structural, or hybrid?"
    → ExecutionMode per step

Layer 3: Substrate Selection
    "What physical execution strategy achieves this?"
    → SubstrateChoice per step

Layer 4: Capability Adaptation
    "Can we actually do this here? What do we downgrade?"
    → Adapted plan with honest capability gaps declared
```

### Execution Mode Reasoning

The system classifies each workflow step into one of:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepExecutionMode {
    /// Backend-only. User does not expect to see this happen.
    /// Examples: write file, run command, fetch data
    Backend,
    /// User expects to see this happen on screen.
    /// Examples: open app, navigate browser, show output
    Visible,
    /// Backend execution with visible surfacing at the end.
    /// Examples: generate code (backend) then open in editor (visible)
    HybridSurface,
    /// Requires GUI interaction (clicks, typing into focused app).
    /// Examples: fill form, click button, navigate menu
    Interactive,
}
```

### Visibility Expectation Inference

The system infers visibility expectations from:

1. **Verb semantics**: "open", "show", "play" → Visible; "create", "generate", "calculate" → Backend
2. **Target type**: App targets → Visible; File targets → Backend+Surface; URL targets → Visible
3. **Explicit markers**: "show me", "let me see" → Visible; no marker → infer from verb
4. **Workflow position**: First step often visible (user wants to see it start); middle steps often backend; last step often visible (user wants to see result)

```rust
pub fn infer_visibility(verb: &Verb, target: &TargetRef, user_text: &str) -> StepExecutionMode {
    let has_show_intent = user_text_has_show_intent(user_text);
    let has_run_intent = user_text_has_run_intent(user_text);
    
    match (verb, target) {
        (Verb::Open, TargetRef::App(_)) => StepExecutionMode::Visible,
        (Verb::Open, TargetRef::Url(_)) => StepExecutionMode::Visible,
        (Verb::Open, TargetRef::File(_)) => StepExecutionMode::Visible,
        (Verb::Run, _) if has_show_intent => StepExecutionMode::HybridSurface,
        (Verb::Run, _) => StepExecutionMode::Backend,
        (Verb::Type, _) => StepExecutionMode::Interactive,
        (Verb::Click, _) => StepExecutionMode::Interactive,
        (Verb::Save, _) => StepExecutionMode::Backend,
        _ => StepExecutionMode::Backend,
    }
}
```

### Semantic Workflow Fidelity

The system maintains a **fidelity contract** — a declaration of what the user expects to perceive:

```rust
pub struct FidelityContract {
    /// What the user expects to see happen
    pub visible_expectations: Vec<VisibleExpectation>,
    /// What can be done invisibly without violating trust
    pub backend_safe_operations: Vec<BackendOperation>,
    /// What requires the user's active participation
    pub interactive_requirements: Vec<InteractiveRequirement>,
}

pub enum VisibleExpectation {
    /// User expects an app window to appear
    AppVisible { app: String, with_content: Option<String> },
    /// User expects browser to show a page
    BrowserPageVisible { url_hint: Option<String> },
    /// User expects output to be shown
    OutputSurfaced { description: String },
    /// User expects a file to be opened in an app
    FileOpenedInApp { file_hint: String, app_hint: Option<String> },
}
```

### Safe Fallback Behavior

When the system cannot achieve a visible expectation, it follows this hierarchy:

1. **Attempt visible execution** with capability-appropriate methods
2. **If visible fails but structural succeeds** → report honestly with `StructurallyComplete` verdict
3. **If structural also fails** → attempt recovery or emit `HitlRequired`
4. **If unplannable** → explain clearly, offer alternatives

The system NEVER:
- Claims visible success without verification
- Silently downgrades visible to structural without telling the user
- Retries indefinitely without user awareness
- Hides failures behind partial-success language

---

## 7. Hybrid Workflow Architecture

### Why Hybrid Is The Default

Most real workflows are hybrid. "Open Code and generate a website and run it and show me" has:
- Backend steps: generate files, write to disk, run dev server
- Visible steps: open IDE, show running site
- Surface steps: bring terminal output to foreground

Pure-backend and pure-visible are edge cases. The architecture must treat hybrid as the normal case.

### Hybrid Execution Model

```rust
pub struct HybridWorkflowPlan {
    pub workflow_id: String,
    pub steps: Vec<HybridStep>,
    pub outcome_contract: OutcomeContract,
    pub fidelity_contract: FidelityContract,
    pub cancellation_contract: CancellationContract,
    pub total_budget_ms: u64,
}

pub struct HybridStep {
    pub index: u32,
    pub action: String,
    pub params: serde_json::Value,
    pub execution_mode: StepExecutionMode,
    pub verification: StepVerification,
    pub timeout_ms: u64,
    /// If this step fails, can the workflow continue?
    pub failure_policy: FailurePolicy,
    /// Does this step need the foreground lease?
    pub needs_foreground: bool,
    /// Whether this step is idempotent (safe to retry without cleanup) (Vuln #13)
    pub idempotent: bool,
    /// Cleanup to run before retrying a non-idempotent step (Vuln #13)
    pub cleanup_on_retry: Option<CleanupStep>,
}

pub enum FailurePolicy {
    /// Workflow stops immediately
    Fatal,
    /// Retry with exponential backoff before failing (Vuln #1)
    RetryThenFatal { max_attempts: u8, backoff_ms: u64 },
    /// Retry with backoff, then skip if still failing
    RetryThenSkip { max_attempts: u8, backoff_ms: u64, fallback: Option<FallbackStep> },
    /// Skip this step, continue with next
    Skippable { fallback: Option<FallbackStep> },
    /// Ask the user what to do
    AskUser { question: String, options: Vec<HitlOption> },
}

pub struct StepVerification {
    /// What to verify after this step
    pub leaves: Vec<VerificationLeaf>,
    /// Timeout for verification (single budget, no nesting)
    pub timeout_ms: u64,
    /// Minimum confidence to consider verified
    pub min_confidence: f32,
}
```

### Example: IDE Code-Run Workflow

User: "Open Code and generate a website for Web Development Agency and run it and show me"

```rust
HybridWorkflowPlan {
    steps: vec![
        HybridStep {
            index: 1,
            action: "generate_project",      // LLM generates files
            execution_mode: Backend,
            verification: FileSystemEffect { path: project_dir, kind: Exists },
            needs_foreground: false,
            failure_policy: Fatal,
        },
        HybridStep {
            index: 2,
            action: "open_application_with_file",  // Open IDE
            execution_mode: Visible,
            verification: ProcessLaunched { binary: "code", max_wait_ms: 8000 },
            needs_foreground: true,
            failure_policy: AskUser {
                question: "VS Code couldn't be opened",
                options: vec![install, choose_alternative, skip_to_run],
            },
        },
        HybridStep {
            index: 3,
            action: "execute_bash",          // Run dev server
            execution_mode: Backend,
            verification: DeterministicOutput { expected: "localhost:", output_file },
            needs_foreground: false,
            failure_policy: Fatal,
        },
        HybridStep {
            index: 4,
            action: "open_url",              // Show in browser
            execution_mode: Visible,
            verification: BrowserPageLoaded { url_contains: Some("localhost") },
            needs_foreground: true,
            failure_policy: Skippable {
                fallback: Some(FallbackStep::SurfaceUrl { url: "http://localhost:3000" }),
            },
        },
    ],
    outcome_contract: OutcomeContract {
        required: vec![
            Outcome::FileCreated { path: project_dir },
            Outcome::ProcessRunning { binary: "node" },
        ],
        desired: vec![
            Outcome::AppVisible { app: "code" },
            Outcome::BrowserPageVisible { url_hint: Some("localhost") },
        ],
    },
}
```

### Foreground Lease Protocol

```rust
pub struct ForegroundLeaseProtocol {
    /// Acquire before any visible step
    pub acquire_before: Vec<u32>,  // step indices
    /// Release after verification of visible step
    pub release_after: Vec<u32>,
    /// Maximum hold duration before forced release
    pub max_hold_ms: u64,
    /// Whether KRIA's own window should minimize during lease
    pub minimize_self: bool,
}
```

### Synchronization Checkpoints

Between hybrid steps, the executor inserts synchronization checkpoints:

```rust
pub enum SyncCheckpoint {
    /// Wait for a process to be running before proceeding
    ProcessReady { binary: String, max_wait_ms: u64 },
    /// Wait for a window to appear (compositor-aware)
    WindowAppeared { class_or_pid: WindowIdentifier, max_wait_ms: u64 },
    /// Wait for a file to exist and stabilize (no writes for N ms)
    FileStable { path: PathBuf, settle_ms: u64 },
    /// Wait for a port to be listening
    PortListening { port: u16, max_wait_ms: u64 },
    /// Fixed delay (last resort, explicitly documented why)
    Delay { ms: u64, reason: &'static str },
}
```

---

## 8. Capability-Driven Runtime

### Centralized Capability Negotiation

All capability checks flow through one negotiation point at workflow plan time. No scattered capability checks during execution.

```rust
/// Resolved once per workflow, cached in WorkflowMemory.
/// The substrate router uses this to adapt its plan.
pub struct CapabilitySet {
    pub environment: EnvironmentCapability,
    pub apps: Vec<AppCapability>,
    pub verifier: VerifierCapability,
    pub browser: BrowserCapability,
    pub interaction: InteractionCapability,
}
```

### Environment Capability

```rust
pub struct EnvironmentCapability {
    /// X11, Wayland, or XWayland
    pub session_type: SessionType,
    /// Which compositor (mutter, kwin, sway, etc.)
    pub compositor: Option<String>,
    /// Whether AT-SPI bus is available and operational
    pub atspi_operational: bool,
    /// Whether xdotool works (X11/XWayland only)
    pub xdotool_available: bool,
    /// Whether the uinput daemon is running
    pub uinput_available: bool,
    /// Whether OCR (tesseract) is available
    pub ocr_available: bool,
    /// Display server info
    pub display: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionType {
    X11,
    Wayland,
    XWayland,
    Unknown,
}
```

### App Capability

```rust
pub struct AppCapability {
    /// Canonical app identifier
    pub app_id: String,
    /// Whether the app is installed
    pub installed: bool,
    /// Whether the app is currently running
    pub running: bool,
    /// Whether the app exposes an accessibility tree
    pub accessible: AccessibilityLevel,
    /// Whether the app supports command-line file arguments
    pub supports_file_args: bool,
    /// Whether the app supports deep links / URI schemes
    pub supported_schemes: Vec<String>,
    /// Known window class for verification
    pub window_class: Option<String>,
    /// Known binary name for process verification
    pub binary_name: String,
    /// Whether the app requires login/account
    pub requires_auth: AuthRequirement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessibilityLevel {
    /// Full AT-SPI tree available
    Full,
    /// Partial (some elements, not all)
    Partial,
    /// Not accessible (Electron without flag, etc.)
    None,
    /// Unknown (not yet probed)
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthRequirement {
    None,
    Optional,
    Required { service: String },
    SessionBased { check_method: SessionCheckMethod },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionCheckMethod {
    /// Check for cookie/token file
    FileExists(PathBuf),
    /// Check for running auth process
    ProcessRunning(String),
    /// Check via CDP (browser session)
    BrowserSession { domain: String },
    /// Cannot be checked programmatically
    Unverifiable,
}
```

### Verifier Capability

```rust
pub struct VerifierCapability {
    /// Which verification methods are available in this environment
    pub available_methods: Vec<VerificationMethod>,
    /// Maximum confidence achievable for window state checks
    pub window_state_max_confidence: f32,
    /// Whether CDP is available for browser verification
    pub cdp_available: bool,
    /// Whether filesystem verification is available (always true on Linux)
    pub filesystem_available: bool,
    /// Whether process table verification is available (always true on Linux)
    pub process_table_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationMethod {
    AtSpi,
    Xdotool,
    Cdp,
    ProcessTable,
    FileSystem,
    Ocr,
    PortCheck,
}
```

### Browser Capability

```rust
pub struct BrowserCapability {
    /// Default browser binary
    pub default_browser: Option<String>,
    /// Whether CDP (Chrome DevTools Protocol) is available
    pub cdp_available: bool,
    /// CDP endpoint if available
    pub cdp_endpoint: Option<String>,
    /// Whether the browser supports --remote-debugging-port
    pub supports_remote_debug: bool,
    /// Known browser profiles
    pub profiles: Vec<BrowserProfile>,
}
```

### Interaction Capability

```rust
pub struct InteractionCapability {
    /// Whether keyboard input injection works
    pub keyboard_injection: InputInjectionLevel,
    /// Whether mouse input injection works
    pub mouse_injection: InputInjectionLevel,
    /// Whether clipboard operations work
    pub clipboard_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputInjectionLevel {
    /// Full injection via uinput daemon
    Full,
    /// Partial via xdotool (X11 only)
    XdotoolOnly,
    /// Not available
    None,
}
```

### Capability Resolution

Capabilities are resolved **once** at workflow start:

```rust
pub async fn resolve_capabilities(
    intent: &WorkflowIntent,
    app_registry: &InstalledAppRegistry,
    env_cache: &EnvironmentCache,
) -> CapabilitySet {
    // Environment is cached per session (changes only on login/logout)
    let environment = env_cache.get_or_detect().await;
    
    // App capabilities are resolved for each target in the intent
    let apps = resolve_app_capabilities(intent, app_registry).await;
    
    // Verifier capabilities derived from environment
    let verifier = derive_verifier_capabilities(&environment);
    
    // Browser capabilities probed if intent involves browser
    let browser = if intent.involves_browser() {
        probe_browser_capabilities().await
    } else {
        BrowserCapability::default()
    };
    
    // Interaction capabilities derived from environment + uinput
    let interaction = derive_interaction_capabilities(&environment);
    
    CapabilitySet { environment, apps, verifier, browser, interaction }
}
```

---

## 9. App Capability Model

### Semantic Alias Resolution

The intent compiler must resolve disjunctive and fuzzy app references:

```rust
pub struct AppResolver {
    registry: Arc<InstalledAppRegistry>,
}

impl AppResolver {
    /// Resolve a user-provided app name to one or more candidates.
    /// Handles: "Excel or Calc", "Code", "VS Code", "file manager", "browser"
    pub fn resolve(&self, raw_name: &str) -> AppResolution {
        // Step 1: Check for disjunctive patterns ("A or B", "A/B")
        if let Some(alternatives) = self.split_disjunction(raw_name) {
            for alt in &alternatives {
                if let Some(id) = self.registry.resolve_alias(alt) {
                    return AppResolution::Resolved(id);
                }
            }
            return AppResolution::NoneInstalled {
                tried: alternatives,
                suggestion: self.suggest_alternatives(raw_name),
            };
        }
        
        // Step 2: Direct alias lookup
        if let Some(id) = self.registry.resolve_alias(raw_name) {
            return AppResolution::Resolved(id);
        }
        
        // Step 3: Category-based resolution ("browser", "editor", "file manager")
        if let Some(id) = self.resolve_category(raw_name) {
            return AppResolution::Resolved(id);
        }
        
        // Step 4: Fuzzy match with confirmation threshold
        if let Some((id, confidence)) = self.fuzzy_match(raw_name) {
            if confidence > 0.85 {
                return AppResolution::Resolved(id);
            } else {
                return AppResolution::Ambiguous {
                    candidates: vec![id],
                    question: format!("Did you mean '{}'?", id.display_name()),
                };
            }
        }
        
        AppResolution::NotFound {
            name: raw_name.to_string(),
            suggestion: self.suggest_alternatives(raw_name),
        }
    }
    
    fn split_disjunction(&self, name: &str) -> Option<Vec<String>> {
        let lower = name.to_lowercase();
        // "Excel or Calc" → ["Excel", "Calc"]
        if lower.contains(" or ") {
            return Some(lower.split(" or ").map(|s| s.trim().to_string()).collect());
        }
        // "Excel/Calc" → ["Excel", "Calc"]
        if lower.contains('/') && !lower.contains("://") {
            return Some(lower.split('/').map(|s| s.trim().to_string()).collect());
        }
        None
    }
    
    fn resolve_category(&self, name: &str) -> Option<CanonicalAppId> {
        let lower = name.to_lowercase();
        match lower.as_str() {
            "browser" | "web browser" => self.registry.default_for_category("browser"),
            "editor" | "text editor" => self.registry.default_for_category("editor"),
            "file manager" | "files" => self.registry.default_for_category("file_manager"),
            "terminal" | "console" => self.registry.default_for_category("terminal"),
            "calculator" | "calc" => self.registry.default_for_category("calculator"),
            _ => None,
        }
    }
}

pub enum AppResolution {
    Resolved(CanonicalAppId),
    Ambiguous { candidates: Vec<CanonicalAppId>, question: String },
    NoneInstalled { tried: Vec<String>, suggestion: Option<String> },
    NotFound { name: String, suggestion: Option<String> },
}
```

### App Workflow Semantics

Each app category carries workflow semantics that inform the substrate router:

```rust
pub struct AppWorkflowSemantics {
    /// How this app is best launched
    pub launch_method: LaunchMethod,
    /// How to verify this app is ready for interaction
    pub readiness_check: ReadinessCheck,
    /// Whether this app supports workspace isolation
    pub supports_workspace: bool,
    /// Known startup time (for timeout calibration)
    pub typical_startup_ms: u64,
    /// Whether this app steals focus on launch
    pub steals_focus: bool,
}

pub enum LaunchMethod {
    /// Launch via .desktop file (gio launch)
    DesktopEntry,
    /// Launch via binary with file argument
    BinaryWithArgs { binary: String },
    /// Launch via URI scheme
    UriScheme { scheme: String },
    /// Launch via D-Bus activation
    DbusActivation { bus_name: String },
}

pub enum ReadinessCheck {
    /// Process appears in /proc
    ProcessExists { binary: String },
    /// Window appears with expected class
    WindowAppears { class: String, max_wait_ms: u64 },
    /// Port becomes available (for servers)
    PortListening { port: u16 },
    /// AT-SPI tree becomes available
    AccessibilityTreeReady { app_name: String },
    /// No reliable check available
    BestEffort { delay_ms: u64 },
}
```

---

## 10. Workflow Fidelity Model

### What Fidelity Means

Workflow fidelity is the degree to which the executed workflow matches what the user expected to perceive. It is NOT binary. It is graded.

### Fidelity Levels

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FidelityLevel {
    /// Everything the user expected happened visibly
    Full,
    /// Core goal achieved, some visible expectations met
    High,
    /// Core goal achieved structurally, visible expectations partially met
    Moderate,
    /// Core goal achieved structurally, no visible expectations met
    StructuralOnly,
    /// Partial goal achievement
    Partial,
    /// Nothing achieved
    None,
}
```

### Outcome Contract

The plan-bound outcome contract declares what must be true for each fidelity level:

```rust
pub struct OutcomeContract {
    /// Outcomes that MUST be true for the workflow to be considered successful
    pub required_outcomes: Vec<PlannedOutcome>,
    /// Outcomes that SHOULD be true for full fidelity
    pub desired_outcomes: Vec<PlannedOutcome>,
    /// Outcomes that are nice-to-have but not expected
    pub optional_outcomes: Vec<PlannedOutcome>,
}

pub struct PlannedOutcome {
    /// What we expect to be true
    pub expectation: OutcomeExpectation,
    /// How to verify it (capability-gated at plan time)
    pub verification: Option<VerificationLeaf>,
    /// What confidence level is acceptable
    pub min_confidence: f32,
    /// What to do if verification fails
    pub on_failure: OutcomeFailurePolicy,
}

pub enum OutcomeExpectation {
    FileExists { path: PathBuf },
    ProcessRunning { binary: String },
    AppWindowVisible { app: String, title_hint: Option<String> },
    BrowserAtUrl { url_contains: String },
    OutputContains { substring: String, in_file: PathBuf },
    PortListening { port: u16 },
}

pub enum OutcomeFailurePolicy {
    /// Downgrade fidelity but don't fail
    DowngradeFidelity,
    /// Fail the workflow
    FailWorkflow,
    /// Ask the user
    AskUser { question: String },
    /// Ignore (optional outcome)
    Ignore,
}
```

### Verification Leaf (Capability-Gated)

```rust
pub enum VerificationLeaf {
    /// Check filesystem (always available on Linux)
    FileSystem { path: PathBuf, check: FsCheck },
    /// Check process table (always available on Linux)
    ProcessTable { binary: String, max_wait_ms: u64 },
    /// Check via AT-SPI (only if atspi_operational)
    AtSpi { query: AtSpiQuery },
    /// Check via CDP (only if cdp_available)
    Cdp { check: CdpCheck },
    /// Check via port (always available)
    PortCheck { port: u16, max_wait_ms: u64 },
    /// Check via compositor-aware window observation
    WindowObservation { identifier: WindowIdentifier, max_wait_ms: u64 },
    /// No verification possible — declared honestly
    Unverifiable { reason: String },
}
```

The substrate router ONLY emits verification leaves that the `CapabilitySet` confirms are satisfiable. If AT-SPI is not operational, no `AtSpi` leaves are emitted. If CDP is not available, no `Cdp` leaves are emitted. The plan is honest about what it can verify.

### Graded Visibility Confidence

Instead of binary "verified/unverified", the verifier returns graded confidence:

```rust
pub struct VerificationResult {
    pub leaf: VerificationLeaf,
    pub confidence: f32,        // 0.0 to 1.0
    pub grade: ConfidenceGrade,
    pub evidence: String,
    pub latency_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfidenceGrade {
    /// Strong evidence (filesystem exists, process in /proc, CDP confirms URL)
    Strong,       // confidence >= 0.85
    /// Moderate evidence (AT-SPI found element, window title matches)
    Moderate,     // confidence 0.60-0.84
    /// Weak evidence (OCR found text, heuristic match)
    Weak,         // confidence 0.30-0.59
    /// No evidence (probe failed, timed out, or unavailable)
    NoEvidence,   // confidence < 0.30
}
```

### Verdict Computation

The finalizer computes the verdict from structural results + verification results + outcome contract:

```rust
pub fn compute_verdict(
    structural_results: &[StepResult],
    verification_results: &[VerificationResult],
    contract: &OutcomeContract,
) -> WorkflowVerdict {
    let all_required_structural = contract.required_outcomes.iter().all(|o| {
        structural_results.iter().any(|r| r.satisfies_structurally(o))
    });
    
    if !all_required_structural {
        let failed_step = structural_results.iter()
            .position(|r| !r.success)
            .unwrap_or(0) as u32;
        return WorkflowVerdict::Failed {
            step: failed_step,
            reason: "Required structural outcome not achieved".into(),
            recovery: suggest_recovery(&structural_results[failed_step as usize]),
        };
    }
    
    let desired_verified = contract.desired_outcomes.iter().filter(|o| {
        o.verification.as_ref().map(|v| {
            verification_results.iter().any(|r| r.leaf == *v && r.confidence >= o.min_confidence)
        }).unwrap_or(false)
    }).count();
    
    let desired_total = contract.desired_outcomes.len();
    
    if desired_total == 0 || desired_verified == desired_total {
        WorkflowVerdict::Complete
    } else if desired_verified > 0 {
        WorkflowVerdict::StructurallyComplete {
            unverified_outcomes: contract.desired_outcomes.iter()
                .filter(|o| !is_verified(o, verification_results))
                .map(|o| describe_outcome(o))
                .collect(),
        }
    } else {
        WorkflowVerdict::StructurallyComplete {
            unverified_outcomes: contract.desired_outcomes.iter()
                .map(|o| describe_outcome(o))
                .collect(),
        }
    }
}
```

---

## 11. Verifier Architecture

### One Orchestrator, Modular Evidence Providers

Per correction table item #13: keep one verifier orchestration layer with modular evidence providers.

```rust
pub struct WorkflowVerifier {
    providers: Vec<Box<dyn EvidenceProvider>>,
    capability_set: CapabilitySet,
}

#[async_trait]
pub trait EvidenceProvider: Send + Sync {
    /// What verification methods this provider supports
    fn supported_methods(&self) -> Vec<VerificationMethod>;
    
    /// Whether this provider can handle a given leaf
    fn can_verify(&self, leaf: &VerificationLeaf) -> bool;
    
    /// Execute verification with a single timeout budget
    async fn verify(&self, leaf: &VerificationLeaf, budget_ms: u64) -> VerificationResult;
}

// Concrete providers:
pub struct FileSystemProvider;      // Always available
pub struct ProcessTableProvider;    // Always available
pub struct PortCheckProvider;       // Always available
pub struct AtSpiProvider;           // Only when atspi_operational
pub struct CdpProvider;             // Only when cdp_available
pub struct CompositorObserver;      // Wayland-aware window observation
pub struct XdotoolProvider;         // Only on X11/XWayland
```

### Single Timeout Budget

Each verification leaf carries ONE timeout. No nested timeouts. No stacked caps.

```rust
impl WorkflowVerifier {
    pub async fn verify_outcomes(
        &self,
        contract: &OutcomeContract,
    ) -> Vec<VerificationResult> {
        let mut results = Vec::new();
        
        for outcome in contract.all_outcomes() {
            let Some(leaf) = &outcome.verification else {
                results.push(VerificationResult::unverifiable(&outcome));
                continue;
            };
            
            // Find the appropriate provider
            let provider = self.providers.iter()
                .find(|p| p.can_verify(leaf));
            
            let result = match provider {
                Some(p) => {
                    // Single timeout — the leaf's own budget
                    tokio::time::timeout(
                        Duration::from_millis(leaf.timeout_budget_ms()),
                        p.verify(leaf, leaf.timeout_budget_ms()),
                    ).await.unwrap_or_else(|_| VerificationResult::timeout(leaf))
                }
                None => VerificationResult::no_provider(leaf),
            };
            
            results.push(result);
        }
        
        results
    }
}
```

### Compositor-Aware Window Observation

Per correction table item #28: define a compositor-aware observation abstraction.

```rust
#[async_trait]
pub trait WindowObservationBackend: Send + Sync {
    /// Check if a window matching the identifier exists and is visible
    async fn observe_window(&self, id: &WindowIdentifier) -> WindowObservation;
    
    /// Wait for a window to appear (event-driven where possible)
    async fn wait_for_window(
        &self,
        id: &WindowIdentifier,
        deadline: Instant,
    ) -> Option<WindowObservation>;
}

pub enum WindowIdentifier {
    /// Match by PID (most reliable when available)
    Pid(u32),
    /// Match by window class (WM_CLASS on X11, app_id on Wayland)
    Class(String),
    /// Match by title substring (least reliable, last resort)
    TitleContains(String),
    /// Match by PID + class (strongest)
    PidAndClass { pid: u32, class: String },
}

pub struct WindowObservation {
    /// Whether a matching window was found
    pub found: bool,
    /// Whether the window is mapped (visible on screen)
    pub mapped: bool,
    /// Whether the window has input focus
    pub focused: bool,
    /// Confidence in this observation
    pub confidence: f32,
    /// How the observation was made
    pub method: ObservationMethod,
    /// Freshness of this observation
    pub observed_at: Instant,
}

pub enum ObservationMethod {
    /// AT-SPI accessibility tree query
    AtSpi,
    /// xdotool (X11 only)
    Xdotool,
    /// /proc scan (process exists, window assumed)
    ProcessHeuristic,
    /// Compositor-specific protocol (wlr-foreign-toplevel, etc.)
    CompositorProtocol,
    /// D-Bus signal (window appeared event)
    DbusEvent,
}

// Implementations per session type:
pub struct X11WindowObserver;        // xdotool + AT-SPI
pub struct WaylandWindowObserver;    // AT-SPI + wlr-foreign-toplevel + /proc
pub struct XWaylandWindowObserver;   // xdotool (for XWayland apps) + AT-SPI
```

### Monotonic Freshness Doctrine

Per correction table item #14: all verification results carry a monotonic timestamp.

```rust
pub struct VerificationResult {
    pub leaf: VerificationLeaf,
    pub confidence: f32,
    pub grade: ConfidenceGrade,
    pub evidence: String,
    pub latency_ms: u32,
    /// Monotonic timestamp when this evidence was collected
    pub observed_at: Instant,
    /// Whether this evidence is still considered fresh
    pub fresh: bool,  // false if observed_at + staleness_budget < now
}
```

Evidence older than its staleness budget is marked stale and not used for verdict computation.

---

## 12. Visible vs Structural Execution

### Explicit Execution Modes (User-Visible)

Per correction table item #6: execution modes must be first-class and user-visible.

The frontend shows the user which mode KRIA chose and why:

```text
┌─────────────────────────────────────────────────┐
│ 🔧 Generating project files...          [Backend]│
│ ✓ Created 12 files in ~/projects/agency         │
│                                                  │
│ 🖥️ Opening VS Code...                  [Visible]│
│ ✓ VS Code opened with project                   │
│                                                  │
│ 🔧 Starting dev server...              [Backend]│
│ ✓ Server running on localhost:3000              │
│                                                  │
│ 🖥️ Opening in browser...              [Visible]│
│ ⚙ Opened but visibility unverified             │
│   (Wayland: focus verification unavailable)     │
│                                                  │
│ ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ │
│ Verdict: Structurally Complete                   │
│ All code generated and server running.           │
│ VS Code and browser were opened but KRIA cannot  │
│ confirm they're in the foreground on Wayland.    │
│                                                  │
│ [Bring to Front]  [Open URL]  [Done]            │
└─────────────────────────────────────────────────┘
```

### When To Use Each Mode

| Scenario | Mode | Rationale |
|----------|------|-----------|
| Write files to disk | Backend | No user perception needed |
| Generate code | Backend | LLM work, no GUI |
| Run a command and capture output | Backend | Process execution |
| Open an application | Visible | User expects to see it |
| Navigate browser to URL | Visible | User expects to see it |
| Show output in terminal | HybridSurface | Generate backend, surface visible |
| Fill a form field | Interactive | Requires GUI input |
| Click a button | Interactive | Requires GUI input |
| Install a package | Backend | System operation |
| Open a file in an app | Visible | User expects to see it |

### Mode Downgrade Rules

When a Visible step cannot be verified:

1. **Execute anyway** — the action (launch app, open URL) still runs
2. **Report honestly** — `VisibilityConfidence::StructuralOnly` with reason
3. **Offer action** — "Bring to Front" button in frontend
4. **Never claim visible success** without evidence

When an Interactive step cannot execute (no uinput, Wayland blocks injection):

1. **Do NOT attempt** — interactive steps require confirmed capability
2. **Emit HITL** — "I can't type into this app. Would you like to do this step manually?"
3. **Offer continuation** — after user completes manual step, workflow resumes

---

## 13. HITL + Collaborative Recovery

### Philosophy

HITL is not a failure mode. It is a collaboration mode. The system should feel like a competent assistant asking a reasonable question, not a broken robot requesting help.

### HITL Trigger Taxonomy

```rust
pub enum HitlTrigger {
    // ── Capability Gaps ──
    AppNotInstalled { app: String, install_hint: Option<String> },
    LoginRequired { service: String, login_url: Option<String> },
    SessionExpired { service: String },
    InteractionUnavailable { reason: String },
    
    // ── Ambiguity ──
    AmbiguousApp { candidates: Vec<String>, question: String },
    AmbiguousFile { candidates: Vec<PathBuf>, question: String },
    AmbiguousIntent { interpretations: Vec<String>, question: String },
    
    // ── Safety ──
    DestructiveAction { description: String, risk: RiskLevel },
    CrossTargetAmbiguity { targets: Vec<String>, question: String },
    
    // ── Verification Uncertainty ──
    VisibilityUncertain { step: String, suggestion: String },
    ManualStepNeeded { instruction: String },
    
    // ── Recovery ──
    StepFailed { step: String, error: String, options: Vec<RecoveryOption> },
    WorkflowStalled { reason: String, options: Vec<RecoveryOption> },
}
```

### HITL UI Patterns

#### Pattern 1: App Not Installed

```text
┌─────────────────────────────────────────────────┐
│ ⚠️ VS Code is not installed                     │
│                                                  │
│ I need VS Code to open the project, but it's    │
│ not available on this system.                    │
│                                                  │
│ [Install VS Code]  [Use gedit]  [Cancel]        │
└─────────────────────────────────────────────────┘
```

#### Pattern 2: Login Required

```text
┌─────────────────────────────────────────────────┐
│ 🔐 YouTube login needed                         │
│                                                  │
│ To access your playlist, you need to be logged  │
│ into YouTube. I can open the login page for you.│
│                                                  │
│ [Open Login Page]  [Skip Playlist]  [Cancel]    │
│                                                  │
│ After logging in, click "Continue" to resume.   │
│ [Continue]                                       │
└─────────────────────────────────────────────────┘
```

#### Pattern 3: Ambiguous Target

```text
┌─────────────────────────────────────────────────┐
│ ❓ Which file did you mean?                      │
│                                                  │
│ I found multiple matches in Downloads:           │
│                                                  │
│ ○ test.png (2.4 MB, modified today)             │
│ ○ test_final.png (1.8 MB, modified yesterday)   │
│ ○ test_v2.png (3.1 MB, modified last week)      │
│                                                  │
│ [Open Selected]  [Open All]  [Cancel]           │
└─────────────────────────────────────────────────┘
```

#### Pattern 4: Manual Step Needed

```text
┌─────────────────────────────────────────────────┐
│ 👤 Manual step needed                            │
│                                                  │
│ I can't type into WhatsApp on Wayland.          │
│ Please type the message manually:               │
│                                                  │
│ Message: "hello"                                 │
│ To: Faizan                                       │
│                                                  │
│ I've opened WhatsApp and navigated to the chat. │
│ Click "Done" when you've sent the message.      │
│                                                  │
│ [Done]  [Cancel]                                 │
└─────────────────────────────────────────────────┘
```

### Workflow Continuation After HITL

When the user responds to a HITL prompt, the workflow resumes:

```rust
pub async fn resume_after_hitl(
    workflow: &mut WorkflowInstance,
    response: HitlResponse,
) -> Result<(), WorkflowError> {
    match response {
        HitlResponse::Approve => {
            // Continue from suspended step
            workflow.transition(WorkflowState::Executing {
                current_step: workflow.suspended_at_step(),
                completed: workflow.completed_steps().clone(),
            });
        }
        HitlResponse::ChooseAlternative { value } => {
            // Replan with the chosen alternative
            let new_plan = replan_with_alternative(workflow, &value)?;
            workflow.replace_remaining_steps(new_plan);
            workflow.transition(WorkflowState::Executing { .. });
        }
        HitlResponse::Skip => {
            // Skip the current step, continue with next
            workflow.skip_current_step();
            workflow.transition(WorkflowState::Executing { .. });
        }
        HitlResponse::Cancel => {
            workflow.transition(WorkflowState::Cancelled { .. });
        }
        HitlResponse::ManualComplete => {
            // User did the step manually, mark as done
            workflow.mark_step_manually_completed();
            workflow.transition(WorkflowState::Executing { .. });
        }
    }
    Ok(())
}
```

---

## 14. Account / Login / Install Workflow Handling

### Account-Aware Workflows

Many real workflows require authenticated sessions. KRIA must detect and handle this.

### Session Detection

```rust
pub struct SessionDetector {
    known_services: Vec<ServiceSessionConfig>,
}

pub struct ServiceSessionConfig {
    pub service: String,           // "youtube", "whatsapp", "gmail"
    pub check_method: SessionCheckMethod,
    pub login_url: Option<String>,
    pub native_app: Option<String>,
}

impl SessionDetector {
    pub async fn check_session(&self, service: &str) -> SessionState {
        let config = self.known_services.iter()
            .find(|s| s.service == service);
        
        let Some(config) = config else {
            return SessionState::Unknown;
        };
        
        match &config.check_method {
            SessionCheckMethod::BrowserSession { domain } => {
                // Check via CDP if available
                if let Some(cdp) = get_cdp_connection().await {
                    check_browser_cookies(cdp, domain).await
                } else {
                    SessionState::Unknown
                }
            }
            SessionCheckMethod::FileExists(path) => {
                if path.exists() {
                    SessionState::Active
                } else {
                    SessionState::NotLoggedIn
                }
            }
            SessionCheckMethod::ProcessRunning(process) => {
                if is_process_running(process) {
                    SessionState::Active
                } else {
                    SessionState::NotRunning
                }
            }
            SessionCheckMethod::Unverifiable => SessionState::Unknown,
        }
    }
}

pub enum SessionState {
    Active,
    NotLoggedIn,
    Expired,
    NotRunning,
    Unknown,
}
```

### Login Flow Integration

When a workflow requires login:

1. **Detect** at plan time (capability negotiation phase)
2. **Emit HITL** with login guidance
3. **Suspend** workflow at the login-dependent step
4. **Resume** when user confirms login complete
5. **Re-verify** session state before continuing

```rust
pub fn handle_login_requirement(
    service: &str,
    session_state: SessionState,
    config: &ServiceSessionConfig,
) -> HitlRequired {
    match session_state {
        SessionState::NotLoggedIn | SessionState::Expired => {
            HitlRequired {
                reason: HitlReason::LoginRequired {
                    service: service.to_string(),
                    guidance: format!(
                        "Please log into {} to continue.",
                        service
                    ),
                },
                options: vec![
                    HitlOption {
                        id: "open_login".into(),
                        label: "Open Login Page".into(),
                        action_type: HitlActionType::OpenUrl {
                            url: config.login_url.clone()
                                .unwrap_or_else(|| format!("https://{}.com", service)),
                        },
                    },
                    HitlOption {
                        id: "continue".into(),
                        label: "I'm logged in now".into(),
                        action_type: HitlActionType::Approve,
                    },
                    HitlOption {
                        id: "skip".into(),
                        label: "Skip this step".into(),
                        action_type: HitlActionType::Skip,
                    },
                    HitlOption {
                        id: "cancel".into(),
                        label: "Cancel".into(),
                        action_type: HitlActionType::Cancel,
                    },
                ],
                context: format!(
                    "The workflow needs access to your {} account.",
                    service
                ),
            }
        }
        _ => unreachable!("Only called when login is needed"),
    }
}
```

### Install Flow Integration

When a required app is not installed:

```rust
pub fn handle_install_requirement(
    app: &str,
    resolution: &AppResolution,
) -> HitlRequired {
    let install_command = suggest_install_command(app);
    let alternatives = suggest_alternatives(app);
    
    let mut options = Vec::new();
    
    if let Some(cmd) = &install_command {
        options.push(HitlOption {
            id: "install".into(),
            label: format!("Install {}", app),
            action_type: HitlActionType::RunCommand { command: cmd.clone() },
        });
    }
    
    for alt in &alternatives {
        options.push(HitlOption {
            id: format!("use_{}", alt),
            label: format!("Use {} instead", alt),
            action_type: HitlActionType::ChooseAlternative { value: alt.clone() },
        });
    }
    
    options.push(HitlOption {
        id: "cancel".into(),
        label: "Cancel".into(),
        action_type: HitlActionType::Cancel,
    });
    
    HitlRequired {
        reason: HitlReason::InstallRequired {
            app: app.to_string(),
            install_command,
        },
        options,
        context: format!("{} is not installed on this system.", app),
    }
}
```

---

## 15. Wayland/X11 Reality Architecture

### The Hard Truth

On Linux in 2026:
- **Wayland** is the default on GNOME (Ubuntu 22.04+) and KDE Plasma 6+
- **X11** is legacy but still used by many users and in VMs
- **XWayland** runs X11 apps inside Wayland sessions (most Electron apps)
- **No compositor exposes a reliable "get all windows" API** on Wayland
- **xdotool does not work** on native Wayland windows
- **AT-SPI works** on both X11 and Wayland via D-Bus (when enabled)
- **Focus stealing is compositor-controlled** on Wayland (apps cannot force focus)
- **wlr-foreign-toplevel-management** exists on wlroots compositors (sway) but NOT on GNOME/KDE

### What KRIA Can Reliably Do Per Session Type

| Capability | X11 | Wayland (GNOME) | Wayland (KDE) | XWayland |
|-----------|-----|-----------------|---------------|----------|
| Get active window title | ✓ (xdotool) | ✗ | ✗ | ✓ (for XWayland apps) |
| Get window list | ✓ (wmctrl) | ✗ | ✗ | Partial |
| Focus a window | ✓ (xdotool) | ✗ (compositor decides) | ✗ | ✓ (XWayland apps) |
| Inject keystrokes | ✓ (xdotool/uinput) | ✓ (uinput only) | ✓ (uinput only) | ✓ |
| Inject mouse clicks | ✓ (xdotool/uinput) | ✓ (uinput only) | ✓ (uinput only) | ✓ |
| AT-SPI tree query | ✓ | ✓ (if enabled) | ✓ (if enabled) | ✓ |
| AT-SPI focused app | ✓ | ✓ (if enabled) | ✓ (if enabled) | ✓ |
| Launch app | ✓ | ✓ | ✓ | ✓ |
| Verify process running | ✓ | ✓ | ✓ | ✓ |
| CDP browser control | ✓ | ✓ | ✓ | ✓ |
| Screenshot | ✓ | ✓ (portal) | ✓ (portal) | ✓ |

### Architecture Implications

1. **AT-SPI is the primary observation channel on Wayland.** Not xdotool. Not wmctrl. AT-SPI works via D-Bus and is compositor-independent.

2. **Process table + filesystem are always reliable.** These are the foundation of structural verification.

3. **Focus verification is unreliable on Wayland.** Accept this. Report `StructuralOnly` when focus cannot be confirmed. Offer "Bring to Front" action.

4. **Window appearance detection uses AT-SPI ChildAdded events** (when available) or process-table polling (always available). Not focus polling.

5. **Input injection on Wayland requires uinput daemon.** xdotool won't work for native Wayland windows. The uinput daemon is already part of KRIA.

6. **Focus stealing prevention is a feature, not a bug.** On Wayland, the compositor decides focus. KRIA should not fight this. Instead: launch the app, verify it's running, and let the compositor handle focus. If the user wants it in front, they click it or KRIA offers a "Bring to Front" button that uses `gtk_window_present()` or equivalent.

### Session-Aware Verification Strategy

```rust
pub fn select_verification_strategy(
    session_type: SessionType,
    atspi_available: bool,
) -> VerificationStrategy {
    match (session_type, atspi_available) {
        (SessionType::X11, true) => VerificationStrategy {
            window_state: vec![Method::AtSpi, Method::Xdotool],
            window_focus: vec![Method::Xdotool, Method::AtSpi],
            max_window_confidence: 0.90,
        },
        (SessionType::X11, false) => VerificationStrategy {
            window_state: vec![Method::Xdotool],
            window_focus: vec![Method::Xdotool],
            max_window_confidence: 0.75,
        },
        (SessionType::Wayland, true) => VerificationStrategy {
            window_state: vec![Method::AtSpi, Method::ProcessHeuristic],
            window_focus: vec![Method::AtSpi],  // Best effort only
            max_window_confidence: 0.70,  // Lower ceiling on Wayland
        },
        (SessionType::Wayland, false) => VerificationStrategy {
            window_state: vec![Method::ProcessHeuristic],
            window_focus: vec![],  // Cannot verify focus at all
            max_window_confidence: 0.40,
        },
        (SessionType::XWayland, _) => VerificationStrategy {
            window_state: vec![Method::AtSpi, Method::Xdotool, Method::ProcessHeuristic],
            window_focus: vec![Method::Xdotool],  // Works for XWayland apps
            max_window_confidence: 0.80,
        },
        _ => VerificationStrategy::minimal(),
    }
}
```

### Causal Launch Handle

When KRIA launches an app, it captures a causal handle:

```rust
pub struct LaunchHandle {
    /// PID of the launched process (if available)
    pub pid: Option<u32>,
    /// Expected window class (from app registry)
    pub expected_class: Option<String>,
    /// Expected binary name (for /proc matching)
    pub expected_binary: String,
    /// Timestamp of launch
    pub launched_at: Instant,
    /// How the app was launched
    pub method: LaunchMethod,
}

pub async fn launch_app_with_handle(
    app_id: &CanonicalAppId,
    file_arg: Option<&Path>,
    registry: &InstalledAppRegistry,
) -> Result<LaunchHandle, LaunchError> {
    let manifest = registry.get_manifest(app_id)?;
    
    let child = match &manifest.launch_method {
        LaunchMethod::DesktopEntry => {
            gio_launch(&manifest.desktop_path, file_arg).await?
        }
        LaunchMethod::BinaryWithArgs { binary } => {
            let mut cmd = tokio::process::Command::new(binary);
            if let Some(file) = file_arg {
                cmd.arg(file);
            }
            cmd.spawn()?
        }
        _ => { /* other methods */ todo!() }
    };
    
    Ok(LaunchHandle {
        pid: child.id(),
        expected_class: manifest.window_class.clone(),
        expected_binary: manifest.binary_name.clone(),
        launched_at: Instant::now(),
        method: manifest.launch_method.clone(),
    })
}
```

The verifier uses this handle to match by PID/class rather than title substring:

```rust
pub async fn verify_app_launched(
    handle: &LaunchHandle,
    observer: &dyn WindowObservationBackend,
    budget_ms: u64,
) -> VerificationResult {
    let identifier = if let Some(pid) = handle.pid {
        if let Some(ref class) = handle.expected_class {
            WindowIdentifier::PidAndClass { pid, class: class.clone() }
        } else {
            WindowIdentifier::Pid(pid)
        }
    } else {
        WindowIdentifier::Class(
            handle.expected_class.clone()
                .unwrap_or_else(|| handle.expected_binary.clone())
        )
    };
    
    let deadline = Instant::now() + Duration::from_millis(budget_ms);
    match observer.wait_for_window(&identifier, deadline).await {
        Some(obs) if obs.found && obs.mapped => VerificationResult {
            confidence: obs.confidence,
            grade: if obs.confidence >= 0.85 { Strong } else { Moderate },
            evidence: format!("Window appeared: {:?}", obs.method),
            ..
        },
        Some(obs) if obs.found => VerificationResult {
            confidence: obs.confidence * 0.7,
            grade: Weak,
            evidence: "Process running but window not confirmed mapped".into(),
            ..
        },
        _ => VerificationResult {
            confidence: 0.0,
            grade: NoEvidence,
            evidence: format!("Window not detected within {}ms", budget_ms),
            ..
        },
    }
}
```

---

## 16. Frontend UX + Workflow UI

### Design Principles

1. **Honest** — never hide failure behind ambiguous language
2. **Actionable** — every non-success state has a clear next action
3. **Progressive** — show progress as it happens, not just the final result
4. **Structured** — machine-readable telemetry rendered into clean UI, not parsed strings
5. **Responsive** — HITL prompts appear immediately, not after a timeout

### Workflow Progress Component

```typescript
interface WorkflowProgressProps {
  workflow: WorkflowTelemetry;
  onHitlResponse: (response: HitlResponse) => void;
  onCancel: () => void;
}

// Renders as:
// ┌─────────────────────────────────────────┐
// │ Generating website for Web Dev Agency    │
// │                                          │
// │ ✓ Step 1/4: Generate project files       │
// │ ✓ Step 2/4: Open VS Code         [🖥️]  │
// │ ● Step 3/4: Start dev server...         │
// │ ○ Step 4/4: Open in browser             │
// │                                          │
// │ [Cancel]                                 │
// └─────────────────────────────────────────┘
```

### Verdict Display

```typescript
interface VerdictBadge {
  verdict: WorkflowVerdict;
}

// Complete:           ✓ Complete
// StructurallyComplete: ⚙ Done (visibility unverified)
// Partial:            ⚠ Partial (3/4 steps)
// Blocked:            🔒 Blocked — action needed
// Failed:             ✗ Failed at step 2
```

### HITL Modal Component

```typescript
interface HitlModalProps {
  reason: HitlReason;
  options: HitlOption[];
  context: string;
  onResponse: (option: HitlOption) => void;
}

// Renders as a modal overlay with:
// - Clear explanation of why input is needed
// - Actionable buttons (not just "OK")
// - Context about what the workflow was doing
// - Cancel option always available
```

### Workflow State Synchronization

Per correction table item #19: canonical workflow-state synchronization model.

```typescript
// Frontend maintains a WorkflowStateStore
interface WorkflowStateStore {
  activeWorkflows: Map<string, WorkflowState>;
  
  // Called when backend emits WorkflowTelemetry
  handleTelemetry(event: WorkflowTelemetry): void;
  
  // Called when user responds to HITL
  sendHitlResponse(workflowId: string, response: HitlResponse): void;
  
  // Called when user cancels
  cancelWorkflow(workflowId: string): void;
}

// Backend → Frontend: WorkflowTelemetry events via Tauri event system
// Frontend → Backend: HitlResponse / CancelRequest via Tauri commands
// 
// Invariant: Frontend state is a pure function of received telemetry events.
// Frontend NEVER infers state from message content.
```

### Continuation Actions

After a workflow completes with `StructurallyComplete` or `Partial`:

```typescript
interface ContinuationHint {
  type: 'bring_to_front' | 'open_url' | 'retry_step' | 'open_file' | 'show_output';
  label: string;
  payload: string;
}

// Rendered as action buttons below the verdict:
// [Bring VS Code to Front]  [Open localhost:3000]  [Show Output]
```

---

## 17. Runtime Simplification Recommendations

### Current State (117 files, ~50k lines in agent/)

The agent layer has accumulated modules faster than contracts stabilized. This section classifies every relevant module and recommends action.

### Module Classification

#### Tier 1: Production Core (Keep, Refine)

| Module | Lines | Action |
|--------|-------|--------|
| `gui_substrate_planner.rs` | 3034 | Split into 4 modules (see below) |
| `htn_executor.rs` | 4059 | Extract tests, keep executor |
| `intent_compiler_rule.rs` | ~500 | Keep as primary fast-path |
| `intent_compiler_llm.rs` | 1274 | Keep as LLM fallback |
| `execution_authority.rs` | 1353 | Refactor ExecutionTarget split |
| `execution_verifier_bounded.rs` | 1140 | Refactor into modular providers |
| `atspi_engine.rs` | 1484 | Keep, add capability gating |
| `turn_gate.rs` | 1263 | Keep, simplify |
| `gui_wiring.rs` | 1495 | Simplify routing logic |
| `loop_engine/mod.rs` | 8200 | Split into 4-5 files |

#### Tier 2: Useful Infrastructure (Keep, Stabilize)

| Module | Lines | Action |
|--------|-------|--------|
| `collaborative_decision.rs` | 2585 | Keep for HITL |
| `execution_gate.rs` | ~500 | Keep |
| `gui_lease.rs` | ~200 | Keep, wire into executor |
| `window_observer.rs` | ~300 | Keep, extend for Wayland |
| `turn_memory.rs` | 746 | Keep |
| `resource_lease.rs` | ~400 | Keep |
| `browser_cognition.rs` | ~500 | Keep for CDP |

#### Tier 3: Experimental / Dead (Remove After Characterization Tests)

| Module | Lines | Action |
|--------|-------|--------|
| `planner.rs` | ~400 | Remove |
| `planner_v2/mod.rs` | ~300 | Remove |
| `gui_planner.rs` | 406 | Remove |
| `semantic_workflow.rs` | 1050 | Remove |
| `workflow_compiler.rs` | 1092 | Extract types, remove orchestration |
| `opgraph.rs` | ~500 | Remove |
| `opgraph_compiler.rs` | ~500 | Remove |
| `execution_verifier_impl.rs` | 1401 | Fold into bounded verifier |
| `hybrid_synchronization.rs` | 859 | Remove (replaced by sync checkpoints) |
| `stage_executor.rs` | 1958 | Remove (replaced by htn_executor) |
| `resume_executor.rs` | 908 | Fold into workflow lifecycle |

#### Tier 4: Deferred Cognition (Feature-Flag, Re-Introduce Later)

| Module | Lines | Action |
|--------|-------|--------|
| `ambient_cognition/` | ~1000 | Feature-flag |
| `curiosity/` | ~500 | Feature-flag |
| `self_model/` | ~500 | Feature-flag |
| `psdg/` | ~800 | Feature-flag |
| `world_model/` | ~600 | Feature-flag |
| `prompt_optimizer/` | ~400 | Feature-flag |
| `ml_orchestrator/` | ~500 | Feature-flag |

### Substrate Planner Split

Per correction table item #3: avoid creating a new god-object.

Split `gui_substrate_planner.rs` (3034 lines) into:

```text
substrate/
├── mod.rs              (~100 lines) — public API
├── router.rs           (~400 lines) — intent → substrate selection
├── plan_builder.rs     (~800 lines) — substrate → workflow plan
├── capability_adapter.rs (~300 lines) — adapt plan to capabilities
├── contract_emitter.rs (~200 lines) — emit OutcomeContract
├── code_generator.rs   (~500 lines) — generate code from hints
└── tests.rs            (~700 lines) — all test fixtures
```

### loop_engine Split

Split `loop_engine/mod.rs` (8200 lines) into:

```text
loop_engine/
├── mod.rs              (~200 lines) — public API + AgentLoop struct
├── gui_routing.rs      (~1500 lines) — GUI workflow detection + dispatch
├── react_loop.rs       (~2000 lines) — standard ReAct tool loop
├── llm_dispatch.rs     (~1000 lines) — LLM call + response parsing
├── outcome_finalization.rs (~500 lines) — verdict computation + telemetry
├── helpers.rs          (existing)
├── intent_extractors.rs (existing)
├── tests.rs            (existing)
```

### Characterization Test Requirement

Per correction table item #12: before removing any module, write characterization tests that capture its current behavior. If the tests reveal behavior that no other module provides, extract that behavior before deletion.

```rust
// Example characterization test for planner.rs
#[test]
fn characterize_planner_behavior() {
    // Document what planner.rs does that gui_substrate_planner doesn't
    // If nothing: safe to remove
    // If something: extract into substrate router
}
```

---

## 18. Planner / Runtime Refactor Recommendations

### ExecutionTarget Refactor (Highest Priority)

```rust
// BEFORE (broken):
pub enum ExecutionTarget {
    Host, Vm, Docker, Colab, Browser, Mcp, CloudProvider,
}

// AFTER (correct):
pub enum ExecutionEnvironment {
    Host,
    Vm,
    Docker,
    Colab,
}

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
}

// Execution authority validates:
pub fn validate_execution(
    tool_name: &str,
    environment: ExecutionEnvironment,
    category: ToolCategory,
) -> ValidationResult {
    // Browser tools always run on Host (the browser process is local)
    // MCP tools run on Host (the MCP server is local)
    // Shell tools can run on Host/Vm/Docker
    // etc.
}
```

### Outcome Contract Threading

```rust
// Plan time:
let plan = substrate_router.plan(&intent, &capabilities);
// plan.outcome_contract is set here, once

// Execution time:
let result = executor.execute(&plan).await;
// result carries structural step results

// Verification time:
let verification = verifier.verify(&plan.outcome_contract, &capabilities).await;
// verifier consumes plan.outcome_contract, never re-derives

// Finalization:
let verdict = finalizer.compute(&result, &verification, &plan.outcome_contract);
// Single source of truth
```

### Ownership Matrix

Per correction table item #23:

| Concern | Owner | Authority |
|---------|-------|-----------|
| Intent classification | IntentCompiler | What the user wants |
| Capability resolution | CapabilityNegotiator | What the environment can do |
| Substrate selection | SubstrateRouter | How to achieve the goal |
| Outcome contract | SubstrateRouter (emits) | What success looks like |
| Step execution | WorkflowExecutor | Running each step |
| Foreground lease | WorkflowExecutor | Who owns the screen |
| Evidence collection | EvidenceProviders | What we can observe |
| Verdict computation | WorkflowFinalizer | Whether it worked |
| Telemetry emission | WorkflowFinalizer | What the frontend sees |
| HITL decisions | CollaborativeDecisionStore | User choices |
| Cancellation | WorkflowExecutor | Stopping gracefully |

No module may claim authority over another module's concern.

### Runtime Latency Budgets

Per correction table item #21:

| Phase | Budget | Hard Cap |
|-------|--------|----------|
| Intent compilation (rule) | 5ms | 20ms |
| Intent compilation (LLM) | 2000ms | 5000ms |
| Capability resolution | 50ms | 200ms |
| Substrate routing | 10ms | 50ms |
| Per-step execution | varies | step.timeout_ms |
| Per-step verification | varies | leaf.timeout_budget_ms |
| Verdict computation | 5ms | 20ms |
| Telemetry emission | 1ms | 5ms |
| Total workflow overhead | <100ms | <500ms |

If any phase exceeds its hard cap, it returns a degraded result (not an error) and logs a latency warning.

---

## 19. Production Evaluation Strategy

### Eval Philosophy

Evals measure **workflow correctness**, not tool completion. A workflow that writes a file and opens an editor is not "correct" if the user asked to see a running website.

### Eval Hierarchy

```text
Level 1: Structural Correctness
    "Did the right files get created? Did the right process start?"
    → Deterministic, always runnable, fast

Level 2: Workflow Fidelity
    "Did the workflow match the user's intent?"
    → Requires intent classification + plan inspection

Level 3: Visibility Correctness
    "Did the user see what they expected?"
    → Requires live desktop, session-type-aware

Level 4: Recovery Correctness
    "When things went wrong, did KRIA handle it well?"
    → Requires fault injection

Level 5: Collaboration Correctness
    "When HITL was needed, was the interaction natural?"
    → Requires simulated user responses
```

### Eval Suites

#### Suite 1: IDE Workflows

```rust
pub struct IdeWorkflowEval {
    cases: vec![
        EvalCase {
            prompt: "Open Code and generate a website for Web Dev Agency and run it",
            required_structural: vec![
                "project directory created",
                "index.html exists",
                "dev server process running",
            ],
            required_fidelity: vec![
                "substrate = IdeCodeRunWorkflow",
                "execution_mode includes Visible steps",
                "outcome_contract includes AppVisible(code)",
            ],
            visibility_check: Some("VS Code process launched"),
            hitl_scenarios: vec![
                "code_not_installed → offers install/alternative",
            ],
        },
        // ... more cases
    ],
}
```

#### Suite 2: Browser Workflows

```rust
pub struct BrowserWorkflowEval {
    cases: vec![
        EvalCase {
            prompt: "Open YouTube and play the first song from my playlist",
            required_structural: vec![
                "browser process launched",
                "navigation to youtube.com attempted",
            ],
            required_fidelity: vec![
                "substrate = BrowserNavigate",
                "session_check for youtube performed",
                "login HITL emitted if not logged in",
            ],
            hitl_scenarios: vec![
                "not_logged_in → login HITL with correct options",
                "browser_not_installed → install HITL",
            ],
        },
    ],
}
```

#### Suite 3: Filesystem Workflows

```rust
pub struct FilesystemWorkflowEval {
    cases: vec![
        EvalCase {
            prompt: "Open Downloads and open test.png",
            required_structural: vec![
                "file search in ~/Downloads performed",
                "matching file found or ambiguity HITL emitted",
            ],
            required_fidelity: vec![
                "fuzzy matching attempted for test.png",
                "if multiple matches → ambiguity HITL",
                "if single match → open with default viewer",
            ],
        },
    ],
}
```

#### Suite 4: Communication Workflows

```rust
pub struct CommunicationWorkflowEval {
    cases: vec![
        EvalCase {
            prompt: "Open WhatsApp and text Faizan hello",
            required_structural: vec![
                "whatsapp app/web detection attempted",
                "session state checked",
            ],
            required_fidelity: vec![
                "native app preferred over web",
                "session check performed",
                "if no session → login HITL",
                "message NOT sent without confirmation on dangerous platforms",
            ],
        },
    ],
}
```

#### Suite 5: HITL Correctness

```rust
pub struct HitlCorrectnessEval {
    cases: vec![
        EvalCase {
            scenario: "app_not_installed",
            expected_hitl: HitlReason::InstallRequired { .. },
            expected_options: vec!["install", "alternative", "cancel"],
            on_response_approve: "workflow continues with install",
            on_response_alternative: "workflow replans with alternative app",
            on_response_cancel: "workflow cancelled cleanly",
        },
        EvalCase {
            scenario: "login_required",
            expected_hitl: HitlReason::LoginRequired { .. },
            expected_options: vec!["open_login", "continue", "skip", "cancel"],
            on_response_continue: "session re-verified before proceeding",
        },
    ],
}
```

#### Suite 6: Verdict Correctness

```rust
pub struct VerdictCorrectnessEval {
    cases: vec![
        // MUST NOT claim Complete when visibility unverified
        EvalCase {
            scenario: "wayland_no_atspi",
            structural_success: true,
            visibility_verified: false,
            expected_verdict: WorkflowVerdict::StructurallyComplete { .. },
            forbidden_verdict: WorkflowVerdict::Complete,
        },
        // MUST NOT claim Failed when structural succeeded
        EvalCase {
            scenario: "structural_ok_visibility_timeout",
            structural_success: true,
            visibility_verified: false,
            forbidden_verdict: WorkflowVerdict::Failed { .. },
        },
    ],
}
```

### Eval Execution Modes

| Mode | Environment | Speed | Coverage |
|------|-------------|-------|----------|
| Unit | No desktop needed | Fast (<1s) | Structural + Fidelity |
| Integration | Mock desktop | Medium (<10s) | + Verification logic |
| Live | Real desktop session | Slow (<60s) | + Visibility + HITL |
| Stress | Real desktop, concurrent | Very slow | + Resource arbitration |

---

## 20. Minimal Survivable Production Architecture

### The Target Codebase Shape

After refactoring, the GUI cognition stack should look like:

```text
crates/kria-core/src/agent/
├── mod.rs                          # Agent module root
├── intent_compiler/
│   ├── mod.rs                      # Public API
│   ├── rule_compiler.rs            # Fast deterministic path
│   ├── llm_compiler.rs             # LLM fallback
│   └── app_resolver.rs             # Disjunctive alias resolution
├── capability/
│   ├── mod.rs                      # CapabilitySet + negotiation
│   ├── environment.rs              # Session type, compositor detection
│   ├── app_capability.rs           # Per-app capability probing
│   ├── verifier_capability.rs      # What verification methods work
│   └── browser_capability.rs       # CDP, browser detection
├── substrate/
│   ├── mod.rs                      # SubstrateRouter public API
│   ├── router.rs                   # Intent → substrate selection
│   ├── plan_builder.rs             # Substrate → HybridWorkflowPlan
│   ├── capability_adapter.rs       # Adapt plan to capabilities
│   ├── contract_emitter.rs         # Emit OutcomeContract
│   └── code_generator.rs           # Generate code from hints
├── executor/
│   ├── mod.rs                      # WorkflowExecutor
│   ├── step_runner.rs              # Per-step execution
│   ├── sync_checkpoints.rs         # Between-step synchronization
│   ├── foreground_lease.rs         # Lease management during execution
│   └── cancellation.rs             # Graceful shutdown
├── verifier/
│   ├── mod.rs                      # WorkflowVerifier orchestrator
│   ├── filesystem_provider.rs      # File/dir checks
│   ├── process_provider.rs         # /proc checks
│   ├── atspi_provider.rs           # AT-SPI queries
│   ├── cdp_provider.rs             # Browser verification
│   ├── port_provider.rs            # Port listening checks
│   └── window_observer.rs          # Compositor-aware window detection
├── finalizer/
│   ├── mod.rs                      # WorkflowFinalizer
│   ├── verdict.rs                  # Verdict computation
│   └── telemetry.rs                # WorkflowTelemetry emission
├── hitl/
│   ├── mod.rs                      # HITL trigger + response handling
│   ├── triggers.rs                 # When to ask the user
│   ├── options.rs                  # What options to present
│   └── continuation.rs             # Resume after user response
├── lifecycle/
│   ├── mod.rs                      # WorkflowState FSM
│   ├── transitions.rs              # State transition rules
│   └── memory.rs                   # Workflow-scoped memory
├── authority/
│   ├── mod.rs                      # Execution authority
│   ├── environment_policy.rs       # Environment validation
│   └── safety_policy.rs            # Risk classification
├── loop_engine/
│   ├── mod.rs                      # AgentLoop struct
│   ├── gui_routing.rs              # GUI workflow detection
│   ├── react_loop.rs               # Standard ReAct loop
│   ├── llm_dispatch.rs             # LLM interaction
│   └── outcome_finalization.rs     # Post-loop finalization
└── atspi_engine.rs                 # AT-SPI interaction (keep as-is)
```

### Line Count Target

| Module | Target Lines | Current Equivalent |
|--------|-------------|-------------------|
| intent_compiler/ | ~2000 | ~2500 (3 files) |
| capability/ | ~1500 | ~200 (scattered) |
| substrate/ | ~2500 | 3034 (one file) |
| executor/ | ~2000 | 4059 (one file) |
| verifier/ | ~1500 | 2541 (two files) |
| finalizer/ | ~500 | ~300 (in loop_engine) |
| hitl/ | ~800 | ~200 (in collaborative_decision) |
| lifecycle/ | ~600 | ~0 (doesn't exist) |
| authority/ | ~800 | 1353 (one file) |
| loop_engine/ | ~5000 | 8200 (one file) |
| **Total** | **~17,200** | **~50,000** |

Net reduction: ~33,000 lines removed or feature-flagged. The remaining code is structured, bounded, and each file is <1500 lines.

---

## 21. Recommended Incremental Rollout

### Phase 0: Foundation (Week 1-2)

**Goal**: Establish contracts without breaking existing behavior.

1. Define `ExecutionEnvironment` + `ToolCategory` enums (new file, no changes to existing)
2. Define `WorkflowTelemetry` types (new file)
3. Define `CapabilitySet` types (new file)
4. Define `OutcomeContract` types (new file)
5. Define `WorkflowState` FSM (new file)
6. Write characterization tests for modules marked for removal

**Risk**: Zero. All additive.

### Phase 1: ExecutionTarget Split (Week 2-3)

**Goal**: Eliminate the category error that causes `EXECUTION_BLOCKED` bugs.

1. Introduce `ExecutionEnvironment` alongside existing `ExecutionTarget`
2. Migrate `execution_authority.rs` to use new types
3. Remove `Browser`, `Mcp`, `CloudProvider` from target validation
4. Update `turn_memory.rs` inference
5. Run existing test suite — all must pass

**Risk**: Medium. Touches authority validation. Mitigated by existing tests.

### Phase 2: Plan-Bound Outcome Contract (Week 3-4)

**Goal**: Eliminate the dual-truth contradiction.

1. Add `outcome_contract: OutcomeContract` field to `GuiWorkflow`
2. Make `gui_substrate_planner` emit outcome contracts at plan time
3. Make `loop_engine` pass the contract to verification instead of re-deriving
4. Keep OCE re-derivation as fallback for ReAct path only
5. Update verdict computation to use contract

**Risk**: Medium. Changes the verification path. Mitigated by keeping fallback.

### Phase 3: Capability Negotiation (Week 4-5)

**Goal**: Make the planner environment-aware.

1. Implement `EnvironmentCapability` detection (session type, AT-SPI, etc.)
2. Implement `AppCapability` resolution
3. Wire capability resolution into substrate router
4. Gate verification leaves on capabilities
5. Add `VisibilityConfidence::StructuralOnly` for unverifiable environments

**Risk**: Low-Medium. Additive to planner. May change verification behavior.

### Phase 4: Structured Telemetry (Week 5-6)

**Goal**: Give the frontend structured workflow state.

1. Emit `WorkflowTelemetry` events from executor
2. Add Tauri event channel for telemetry
3. Build frontend `WorkflowProgressComponent`
4. Build frontend `VerdictBadge`
5. Keep existing `StreamEvent::Token` path for non-workflow interactions

**Risk**: Low. Frontend-only changes + new event channel.

### Phase 5: HITL Integration (Week 6-7)

**Goal**: Collaborative recovery for common blockers.

1. Implement `HitlTrigger` detection in executor
2. Wire HITL into workflow lifecycle FSM
3. Build frontend `HitlModal` component
4. Implement continuation after HITL response
5. Add app-not-installed and login-required flows

**Risk**: Medium. New interaction pattern. Requires frontend + backend coordination.

### Phase 6: Simplification (Week 7-9)

**Goal**: Remove dead code, split god files.

1. Remove Tier 3 modules (after characterization tests pass)
2. Feature-flag Tier 4 modules
3. Split `loop_engine/mod.rs`
4. Split `gui_substrate_planner.rs`
5. Extract tests from `htn_executor.rs`

**Risk**: High. Large refactor. Mitigated by characterization tests and incremental approach.

### Phase 7: Wayland Hardening (Week 9-10)

**Goal**: Reliable behavior on Wayland.

1. Implement `WaylandWindowObserver` using AT-SPI + process heuristic
2. Implement causal launch handles
3. Wire foreground lease into visible steps
4. Calibrate confidence grades per session type
5. Run live eval suite on Wayland

**Risk**: Medium. Wayland behavior is environment-dependent.

### Phase 8: Disjunctive Alias + Browser (Week 10-11)

**Goal**: Handle "Excel or Calc" and browser workflows correctly.

1. Implement `AppResolver` with disjunction splitting
2. Implement category-based resolution ("browser", "editor")
3. Implement `BrowserCapability` probing
4. Add session detection for common services
5. Wire into substrate router

**Risk**: Low. Mostly additive to intent compiler.

---

## 22. Deferred Systems

These systems are valuable but should NOT be built until Phases 0-5 are stable:

| System | Why Deferred | Re-Introduction Criteria |
|--------|-------------|------------------------|
| PSDG / World Model | Encodes beliefs on unstable runtime | Runtime contracts stable for 2 weeks |
| Ambient Cognition | Requires stable observation layer | Verifier + capability system proven |
| Curiosity / Self-Model | Meta-cognition on unstable base | Core workflows passing eval suite |
| Prompt Optimizer | Optimizes wrong thing if contracts drift | Intent compiler stable |
| ML Orchestrator | Adds complexity before simplification | Simplified runtime proven |
| OpGraph | Parallel execution model | Sequential model proven first |
| Recursive Planning | Unbounded by design | Bounded planning proven |

### Transitional Compatibility Layer

Per correction table item #25: old workflows must not break during refactor.

```rust
/// Compatibility shim: converts old-style GuiWorkflow (without OutcomeContract)
/// into new-style HybridWorkflowPlan.
pub fn upgrade_legacy_workflow(
    old: &GuiWorkflow,
    user_text: &str,
    capabilities: &CapabilitySet,
) -> HybridWorkflowPlan {
    // Derive outcome contract from old workflow's verification types
    let contract = derive_contract_from_legacy_verifications(&old.sub_goals);
    
    // Convert SubGoals to HybridSteps
    let steps = old.sub_goals.iter().map(|sg| {
        HybridStep {
            index: sg.step,
            action: sg.action.clone(),
            params: sg.params.clone(),
            execution_mode: infer_mode_from_action(&sg.action),
            verification: convert_legacy_verification(&sg.verify, capabilities),
            timeout_ms: sg.timeout_ms.unwrap_or(10_000),
            failure_policy: FailurePolicy::Fatal,
            needs_foreground: is_visible_action(&sg.action),
        }
    }).collect();
    
    HybridWorkflowPlan {
        workflow_id: old.task_id.clone(),
        steps,
        outcome_contract: contract,
        fidelity_contract: FidelityContract::default(),
        cancellation_contract: CancellationContract::default(),
        total_budget_ms: (old.max_duration_sec as u64) * 1000,
    }
}
```

This shim runs during Phase 2-3 to ensure existing substrate planner output still works while the new contract system is being built.

---

## 23. Hard Constraints

### Non-Negotiable Technical Constraints

1. **Single-threaded workflow execution.** One workflow at a time holds the foreground lease. No concurrent GUI workflows fighting for focus.

2. **Deterministic substrate routing.** Same intent + same capabilities = same plan. Always. No randomness, no LLM in the routing path.

3. **Bounded verification.** Every verification leaf has ONE timeout. No retries inside the verifier. If it times out, it returns `NoEvidence`. The finalizer decides what that means.

4. **No focus stealing on Wayland.** KRIA does not attempt to force-focus windows on Wayland. It launches apps and lets the compositor decide. If the user wants focus, they click or use the "Bring to Front" action.

5. **No unverifiable success claims.** If KRIA cannot verify an outcome, it says so. `StructurallyComplete` is honest. `Complete` requires evidence.

6. **No LLM in the verdict path.** The LLM generates code and classifies intent. It does NOT decide whether a workflow succeeded.

7. **Frontend is a pure renderer.** The frontend renders `WorkflowTelemetry` events. It does not parse strings, infer state, or make decisions.

8. **HITL is non-blocking to the system.** When a workflow suspends for HITL, the system is idle. It does not spin, retry, or time out. The user responds when ready.

9. **Capability detection is cached per session.** Environment capabilities don't change within a login session. Detect once, cache, use everywhere.

10. **File size cap: 1500 lines per file.** No file in the GUI cognition stack may exceed 1500 lines. If it does, split it.

### Non-Negotiable UX Constraints

1. **Never lie about success.** If visibility is unverified, say so.
2. **Always offer an action.** Every non-success state has at least one button the user can click.
3. **Never block indefinitely.** Every wait has a timeout. Every timeout has a user-visible consequence.
4. **Explain in one sentence.** Every HITL prompt has a one-sentence explanation of why input is needed.
5. **Preserve artifacts.** Even on failure, files created by completed steps are preserved and shown to the user.

---

## 24. Non-Negotiable Principles

### The Bounded Operational Cognition Doctrine

Per correction table item #20: define what "good cognition" means.

**Bounded operational cognition** is:

1. **Operational** — it serves a concrete workflow goal, not abstract reasoning
2. **Bounded** — it has explicit time, space, and confidence limits
3. **Deterministic where possible** — routing, verification, and verdicts are deterministic; only content generation and intent classification use LLMs
4. **Honest** — it reports what it knows, what it doesn't know, and what it cannot know
5. **Collaborative** — when uncertain, it asks rather than guesses
6. **Recoverable** — every failure state has a defined recovery path
7. **Observable** — every decision is logged with its inputs and rationale

### What This Means In Practice

- The system does NOT "think" about goals. It routes intents to substrates.
- The system does NOT "perceive" the screen. It queries specific evidence providers.
- The system does NOT "plan" in the AI sense. It selects from a finite set of execution strategies.
- The system does NOT "learn" from failures. It reports them honestly and offers recovery.
- The system DOES understand workflow semantics (visible vs structural, hybrid execution).
- The system DOES adapt to capabilities (environment-aware, app-aware).
- The system DOES collaborate with the user (HITL for uncertainty, not just for approval).

### Architectural Governance Doctrine

Per correction table item #30: prevent future sprawl.

**Rules for adding new modules to the GUI cognition stack:**

1. **Justify against the spine.** Every new module must map to exactly one component in the runtime spine diagram. If it doesn't fit, it doesn't belong.

2. **One job per module.** A module that does two things should be two modules.

3. **1500-line cap.** If a module exceeds 1500 lines, it must be split before the PR merges.

4. **Contract-first.** New modules must define their input/output contract before implementation. The contract is reviewed separately from the implementation.

5. **Characterization test on removal.** Before removing any module, write tests that capture its behavior. If behavior is unique and valuable, extract it.

6. **No speculative modules.** Modules are added when a concrete workflow requires them, not when they "might be useful someday."

7. **Feature-flag experimental work.** Anything that isn't proven in production goes behind `#[cfg(feature = "experimental_cognition")]`.

8. **Integration boundary contracts.** Per correction table item #29: any new integration (MCP server, external tool, plugin) must define a typed boundary contract. No stringly-typed integration points.

### Async Orchestration Constraints

Per correction table item #27:

1. **No unbounded spawns.** Every `tokio::spawn` in the workflow path has a corresponding join or cancellation token.
2. **No nested timeouts.** One timeout per operation. Period.
3. **No implicit ordering.** If step B depends on step A, the dependency is explicit in the plan.
4. **Cancellation tokens propagate.** When a workflow is cancelled, all child tasks receive the cancellation signal within 100ms.

---

## 25. Final Production-Ready GUI Cognition Architecture

### The Complete Picture

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                           User Input                                     │
│                    "Open Code and generate a website..."                 │
└────────────────────────────────┬────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                        INTENT COMPILER                                    │
│  Rule-based fast path (5ms) → LLM fallback (2s) → AppResolver           │
│  Output: WorkflowIntent { verb, targets, content, visibility }           │
└────────────────────────────────┬────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                     CAPABILITY NEGOTIATOR                                 │
│  Environment (session type, compositor, AT-SPI)                          │
│  Apps (installed, running, accessible, auth)                             │
│  Verifier (which methods work here)                                      │
│  Browser (CDP, profiles)                                                 │
│  Interaction (uinput, keyboard, mouse)                                   │
│  Output: CapabilitySet                                                   │
│                                                                          │
│  If app not installed → early HITL (install/alternative/cancel)          │
│  If login required → early HITL (login/skip/cancel)                      │
└────────────────────────────────┬────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                       SUBSTRATE ROUTER                                    │
│  Intent + Capabilities → Execution Strategy                              │
│  Selects: FileWriteThenOpen | IdeCodeRun | BrowserNavigate |             │
│           TerminalExecution | AppOpen | Interactive | ...                 │
│  Emits: HybridWorkflowPlan + OutcomeContract + FidelityContract          │
│                                                                          │
│  Capability-gated: only emits verification leaves that can be satisfied  │
│  Honest: declares unverifiable outcomes as StructuralOnly up-front       │
└────────────────────────────────┬────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                      WORKFLOW EXECUTOR                                    │
│  Lifecycle FSM: Created → Planned → Executing → Verifying → Finalized   │
│                                                                          │
│  Per step:                                                               │
│    1. Acquire foreground lease (if visible step)                          │
│    2. Execute action                                                     │
│    3. Wait for sync checkpoint                                           │
│    4. Verify step outcome                                                │
│    5. Emit StepCompleted telemetry                                       │
│    6. Release lease (if held)                                            │
│                                                                          │
│  On blocker: emit HitlRequired, suspend, wait for user response          │
│  On cancel: run cleanup, emit Cancelled telemetry                        │
│  On timeout: emit Failed with recovery suggestion                        │
└────────────────────────────────┬────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                      OUTCOME VERIFIER                                     │
│  Consumes: OutcomeContract (from plan, NOT re-derived)                   │
│  Uses: Modular evidence providers (FS, Process, AT-SPI, CDP, Port)       │
│  Respects: CapabilitySet (only uses available methods)                   │
│  Returns: Vec<VerificationResult> with graded confidence                 │
│                                                                          │
│  Single timeout per leaf. No nested caps. No retries.                    │
│  Monotonic timestamps on all evidence.                                   │
└────────────────────────────────┬────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                     WORKFLOW FINALIZER                                    │
│  Single source of verdict truth.                                         │
│  Inputs: structural results + verification results + outcome contract    │
│  Output: WorkflowVerdict + WorkflowTelemetry::Completed                  │
│                                                                          │
│  Complete: all required + all desired verified                            │
│  StructurallyComplete: all required, some desired unverifiable           │
│  Partial: some required failed                                           │
│  Failed: critical step failed                                            │
│  Blocked: HITL needed (should not reach finalizer)                       │
└────────────────────────────────┬────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         FRONTEND                                          │
│  Pure renderer of WorkflowTelemetry events                               │
│  Progress bars, step indicators, verdict badges                          │
│  HITL modals with actionable buttons                                     │
│  Continuation actions (Bring to Front, Open URL, Retry)                  │
│                                                                          │
│  NEVER parses natural language to determine state                        │
│  NEVER infers workflow progress from message content                     │
└─────────────────────────────────────────────────────────────────────────┘
```

### What This Architecture Achieves

| Requirement | How It's Met |
|-------------|-------------|
| Honest completion reporting | Single finalizer, graded verdicts, no dual-truth |
| Wayland reliability | Capability-gated verification, no focus assumptions |
| Hybrid workflows | Per-step execution mode, foreground lease protocol |
| HITL collaboration | First-class workflow state, structured options |
| App awareness | AppResolver + AppCapability + session detection |
| Account/login handling | SessionDetector + HITL login flows |
| Frontend trust | Structured telemetry, no string parsing |
| Maintainability | 1500-line cap, clear ownership, governance doctrine |
| Debuggability | Monotonic timestamps, execution trace, telemetry log |
| Performance | Latency budgets, cached capabilities, no recursive planning |
| Extensibility | Modular evidence providers, integration boundary contracts |

### What This Architecture Does NOT Do

- It does not pretend to be a general-purpose AI agent
- It does not recursively plan or self-improve
- It does not maintain a persistent world model (deferred)
- It does not do OCR-based verification as a primary method
- It does not fight the compositor for focus
- It does not claim success without evidence
- It does not require an LLM for routing, verification, or verdicts
- It does not grow unboundedly — governance doctrine prevents sprawl

---

## Summary

KRIA's path to production-grade GUI cognition is not about adding more intelligence. It is about:

1. **Collapsing contradictions** (one truth, one contract, one verdict)
2. **Respecting reality** (Wayland limits, capability awareness, honest reporting)
3. **Collaborating with the user** (HITL as a feature, not a failure)
4. **Simplifying the runtime** (fewer modules, clearer ownership, bounded complexity)
5. **Structured communication** (telemetry events, not parsed strings)

The result is a system that does less but does it correctly — and that is worth infinitely more than a system that attempts everything and succeeds at nothing verifiably.

---

*End of base architecture (v1.0). See Section 26-28 for production hardening (v2.0).*


---

## 26. Production Hardening Addendum (25 Vulnerability Mitigations)

This section integrates all 25 identified vulnerabilities as mandatory production contracts. Every item below is a **binding requirement** — not optional, not deferred.

---

### H1. Bounded Retry With Backoff (Vuln #1)

Steps that fail transiently (app slow to start, port not ready) must not immediately fail the workflow.

```rust
pub enum FailurePolicy {
    Fatal,
    /// Retry with exponential backoff before escalating
    RetryThenFatal { max_attempts: u8, initial_backoff_ms: u64 },
    /// Retry, then skip with fallback
    RetryThenSkip { max_attempts: u8, initial_backoff_ms: u64, fallback: Option<FallbackStep> },
    Skippable { fallback: Option<FallbackStep> },
    AskUser { question: String, options: Vec<HitlOption> },
}

/// Retry execution logic (inside WorkflowExecutor)
async fn execute_with_retry(
    step: &HybridStep,
    policy: &FailurePolicy,
) -> StepResult {
    let (max_attempts, initial_backoff) = match policy {
        FailurePolicy::RetryThenFatal { max_attempts, initial_backoff_ms } => (*max_attempts, *initial_backoff_ms),
        FailurePolicy::RetryThenSkip { max_attempts, initial_backoff_ms, .. } => (*max_attempts, *initial_backoff_ms),
        _ => return execute_step_once(step).await,
    };

    let mut backoff = initial_backoff;
    for attempt in 0..max_attempts {
        let result = execute_step_once(step).await;
        if result.success {
            return result;
        }
        if attempt < max_attempts - 1 {
            // Non-idempotent steps need cleanup before retry
            if !step.idempotent {
                if let Some(ref cleanup) = step.cleanup_on_retry {
                    run_cleanup(cleanup).await;
                }
            }
            tokio::time::sleep(Duration::from_millis(backoff)).await;
            backoff = (backoff * 2).min(5000); // Cap at 5s
        }
    }
    StepResult::failed("Exhausted retry budget")
}
```

**Constraints:**
- Maximum 3 retries per step (hard cap, not configurable per-step beyond 3)
- Backoff capped at 5000ms
- Total retry budget counts against `workflow.total_budget_ms`

---

### H2. Workflow Preemption Contract (Vuln #2)

When the user sends a new message while a workflow is executing:

```rust
pub enum PreemptionPolicy {
    /// Pause current workflow, process new message
    PauseAndProcess,
    /// Queue new message, continue current workflow
    QueueBehind,
    /// Cancel current workflow, process new message
    CancelAndProcess,
}

pub struct PreemptionHandler {
    active_workflow: Option<WorkflowInstance>,
}

impl PreemptionHandler {
    pub async fn handle_new_message_during_workflow(
        &mut self,
        new_message: &str,
        active: &mut WorkflowInstance,
    ) -> PreemptionDecision {
        // Classify the new message
        let is_cancel = is_cancel_intent(new_message);
        let is_modification = is_workflow_modification(new_message);
        let is_unrelated = is_unrelated_query(new_message);

        if is_cancel {
            return PreemptionDecision::CancelWorkflow;
        }
        if is_modification {
            // "also do X" or "change the email subject to..."
            return PreemptionDecision::ModifyWorkflow { modification: new_message.to_string() };
        }
        if is_unrelated {
            // Completely different topic — pause workflow, answer, then offer resume
            return PreemptionDecision::PauseAndRespond {
                resume_prompt: "I paused the current workflow. [Resume] [Cancel]".into(),
            };
        }
        // Default: queue behind
        PreemptionDecision::QueueBehind
    }
}

pub enum PreemptionDecision {
    CancelWorkflow,
    ModifyWorkflow { modification: String },
    PauseAndRespond { resume_prompt: String },
    QueueBehind,
}
```

**Rules:**
- "cancel", "stop", "nevermind" → immediate cancellation
- "also", "change", "modify" → workflow modification (replan remaining steps)
- Unrelated question → pause, answer, offer resume
- Ambiguous → queue behind current workflow (don't interrupt)

---

### H3. Capability Re-Validation (Vuln #3)

Capabilities are resolved once at plan time but **spot-checked** before critical steps:

```rust
pub struct CapabilityRevalidator;

impl CapabilityRevalidator {
    /// Lightweight check before a visible step executes.
    /// Does NOT re-run full capability detection — only checks the specific
    /// capability this step depends on.
    pub async fn spot_check(step: &HybridStep, caps: &CapabilitySet) -> SpotCheckResult {
        match step.execution_mode {
            StepExecutionMode::Visible => {
                // Check if the target app is still running (if it was running at plan time)
                if let Some(app_cap) = extract_app_from_step(step, caps) {
                    if app_cap.running && !is_process_running(&app_cap.binary_name) {
                        return SpotCheckResult::Degraded {
                            reason: format!("{} is no longer running", app_cap.app_id),
                            suggestion: "Relaunch the application",
                        };
                    }
                }
                SpotCheckResult::Ok
            }
            StepExecutionMode::Interactive => {
                // Verify uinput daemon is still alive
                if caps.interaction.keyboard_injection == InputInjectionLevel::Full {
                    if !is_process_running("kria-uinput-daemon") {
                        return SpotCheckResult::Degraded {
                            reason: "Input daemon stopped".into(),
                            suggestion: "Restart kria-uinput-daemon",
                        };
                    }
                }
                SpotCheckResult::Ok
            }
            _ => SpotCheckResult::Ok,
        }
    }
}

pub enum SpotCheckResult {
    Ok,
    Degraded { reason: String, suggestion: &'static str },
}
```

**Rules:**
- Spot checks are <10ms (process table scan only)
- Only run before Visible and Interactive steps
- Degraded result → emit HITL, don't silently fail

---

### H4. Session-Level Observation Cache (Vuln #4)

```rust
pub struct SessionObservationCache {
    /// Observed app startup times (calibrates future timeouts)
    app_startup_times: HashMap<String, Vec<u64>>,  // binary → [observed_ms, ...]
    /// Maximum entries per app (bounded)
    max_observations_per_app: usize,  // = 10
}

impl SessionObservationCache {
    pub fn record_startup(&mut self, binary: &str, elapsed_ms: u64) {
        let times = self.app_startup_times.entry(binary.to_string()).or_default();
        if times.len() >= self.max_observations_per_app {
            times.remove(0);
        }
        times.push(elapsed_ms);
    }

    /// Get calibrated timeout for an app based on observed behavior.
    /// Returns None if no observations exist (use default).
    pub fn calibrated_timeout(&self, binary: &str) -> Option<u64> {
        let times = self.app_startup_times.get(binary)?;
        if times.len() < 2 { return None; }
        let avg = times.iter().sum::<u64>() / times.len() as u64;
        // Use 2x average as timeout (generous but informed)
        Some((avg * 2).max(3000).min(30000))
    }
}
```

**Rules:**
- Cache is per-session (cleared on restart)
- Maximum 10 observations per app (bounded memory)
- Used to calibrate timeouts, never to change plans

---

### H5. LLM Output Quality Gate (Vuln #5)

When the LLM fallback in intent compilation returns malformed output:

```rust
pub fn validate_llm_intent_output(raw: &serde_json::Value) -> LlmOutputValidation {
    // Required fields
    let has_verb = raw.get("primary_verb").and_then(|v| v.as_str()).is_some();
    let has_targets = raw.get("targets").and_then(|v| v.as_array()).is_some();

    if !has_verb || !has_targets {
        return LlmOutputValidation::Malformed {
            reason: "Missing required fields (primary_verb, targets)".into(),
        };
    }

    // Verb must be from known set
    let verb = raw["primary_verb"].as_str().unwrap();
    if !["Open", "Type", "Click", "Run", "Save", "Close", "Switch", "Other"].contains(&verb) {
        return LlmOutputValidation::Malformed {
            reason: format!("Unknown verb: {}", verb),
        };
    }

    LlmOutputValidation::Valid
}

pub enum LlmOutputValidation {
    Valid,
    Malformed { reason: String },
}

// In the intent compiler:
// If LLM output is malformed → don't silently fall to Unplannable.
// Instead, emit HITL asking the user to rephrase:
HitlReason::IntentUnclear {
    original_text: user_text.to_string(),
    what_kria_understood: partial_parse_summary,
    suggestion: "Could you rephrase? I'm not sure what action you want me to take.".into(),
}
```

**Rules:**
- LLM output is validated structurally before use
- Malformed output → HITL (not silent failure)
- User sees what KRIA understood and can correct it

---

### H6. Telemetry Persistence (Vuln #6)

```rust
pub struct TelemetryStore {
    db: rusqlite::Connection,
    max_workflows: usize,  // = 100
}

impl TelemetryStore {
    pub fn persist_event(&self, envelope: &TelemetryEnvelope) -> Result<()> {
        self.db.execute(
            "INSERT INTO workflow_telemetry (workflow_id, seq, event_json, timestamp_ms, source)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                extract_workflow_id(&envelope.event),
                envelope.seq,
                serde_json::to_string(&envelope.event)?,
                envelope.timestamp_ms,
                format!("{:?}", envelope.source),
            ],
        )?;
        self.prune_old_workflows()?;
        Ok(())
    }

    fn prune_old_workflows(&self) -> Result<()> {
        // Keep only the last 100 workflows
        self.db.execute(
            "DELETE FROM workflow_telemetry WHERE workflow_id NOT IN (
                SELECT DISTINCT workflow_id FROM workflow_telemetry
                ORDER BY timestamp_ms DESC LIMIT ?1
            )", params![self.max_workflows])?;
        Ok(())
    }
}
```

**Rules:**
- Last 100 workflows persisted to SQLite
- Enables: debug view, retry from checkpoint, "recent workflows" UI
- Pruning is automatic and bounded

---

### H7. Browser Protocol Abstraction (Vuln #7)

```rust
#[async_trait]
pub trait BrowserProtocol: Send + Sync {
    async fn get_current_url(&self) -> Result<String>;
    async fn get_page_title(&self) -> Result<String>;
    async fn navigate(&self, url: &str) -> Result<()>;
    async fn is_connected(&self) -> bool;
    fn protocol_name(&self) -> &'static str;
}

pub struct CdpProvider { /* Chrome DevTools Protocol */ }
pub struct MarionnetteProvider { /* Firefox Remote Protocol */ }

pub struct BrowserVerifier {
    protocol: Box<dyn BrowserProtocol>,
}

impl BrowserVerifier {
    pub async fn detect_and_connect() -> Option<Self> {
        // Try CDP first (Chrome, Chromium, Brave, Edge)
        if let Some(cdp) = CdpProvider::try_connect().await {
            return Some(Self { protocol: Box::new(cdp) });
        }
        // Try Marionette (Firefox)
        if let Some(marionette) = MarionnetteProvider::try_connect().await {
            return Some(Self { protocol: Box::new(marionette) });
        }
        None
    }
}
```

**Rules:**
- Browser verification is protocol-agnostic
- CDP and Marionette are both supported
- Capability detection probes both protocols

---

### H8. Graded AT-SPI Availability (Vuln #8)

```rust
#[derive(Debug, Clone)]
pub enum AtSpiLevel {
    /// Full accessibility stack operational, apps expose trees
    Full,
    /// Bus available but only some apps expose trees
    Partial { accessible_apps: Vec<String> },
    /// Bus exists but no apps detected
    BusOnly,
    /// AT-SPI completely unavailable
    None,
}

pub async fn detect_atspi_level() -> AtSpiLevel {
    let bus_available = check_atspi_bus().await;
    if !bus_available {
        return AtSpiLevel::None;
    }

    let apps = list_accessible_applications().await;
    if apps.is_empty() {
        return AtSpiLevel::BusOnly;
    }

    let toolkit_enabled = check_toolkit_accessibility().await;
    if toolkit_enabled && apps.len() >= 3 {
        AtSpiLevel::Full
    } else {
        AtSpiLevel::Partial { accessible_apps: apps }
    }
}
```

**Impact on verification:**
- `Full` → AT-SPI verification leaves are emitted normally
- `Partial` → AT-SPI leaves only emitted for apps in `accessible_apps` list
- `BusOnly` → No AT-SPI leaves emitted, fall back to process/filesystem
- `None` → No AT-SPI leaves emitted, no AT-SPI provider registered

---

### H9. Foreground Lease Break Detection (Vuln #9)

```rust
pub struct ForegroundLeaseGuard {
    workflow_id: String,
    expected_window: WindowIdentifier,
    /// Polling task that monitors focus during the lease
    monitor_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ForegroundLeaseGuard {
    /// Start monitoring focus. If focus changes unexpectedly, emit a signal.
    pub fn start_monitoring(
        &mut self,
        focus_lost_tx: mpsc::Sender<FocusLostEvent>,
        observer: Arc<dyn WindowObservationBackend>,
    ) {
        let expected = self.expected_window.clone();
        let workflow_id = self.workflow_id.clone();
        self.monitor_handle = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            loop {
                interval.tick().await;
                let obs = observer.observe_window(&expected).await;
                if obs.found && !obs.focused {
                    // Focus moved away from our window
                    let _ = focus_lost_tx.send(FocusLostEvent {
                        workflow_id: workflow_id.clone(),
                        lost_at: Instant::now(),
                    }).await;
                    break;
                }
            }
        }));
    }
}

pub struct FocusLostEvent {
    pub workflow_id: String,
    pub lost_at: Instant,
}

// In the executor, when FocusLostEvent is received during a visible step:
// → Emit HitlReason::FocusLost instead of failing the step
HitlReason::FocusLost {
    step: current_step_description,
    suggestion: "I lost focus on the target window. Did you switch away?".into(),
    options: vec![
        HitlOption { id: "refocus", label: "I switched back, continue", action_type: Approve },
        HitlOption { id: "skip", label: "Skip this step", action_type: Skip },
        HitlOption { id: "cancel", label: "Cancel workflow", action_type: Cancel },
    ],
}
```

**Rules:**
- Focus monitoring only active during leased visible steps
- 500ms polling interval (not too aggressive)
- Focus loss → HITL, not failure

---

### H10. Accessibility Bootstrapping Flow (Vuln #10)

```rust
pub async fn first_time_accessibility_check(
    env: &EnvironmentCapability,
) -> Option<WorkflowTelemetry> {
    if env.atspi_operational {
        return None; // Already working, no action needed
    }

    // First GUI workflow ever — offer to enable accessibility
    Some(WorkflowTelemetry::HitlRequired {
        workflow_id: "system-setup".into(),
        reason: HitlReason::AccessibilitySetup {
            current_state: describe_atspi_state(env),
            impact: "Without accessibility, KRIA cannot verify if apps opened correctly.".into(),
        },
        options: vec![
            HitlOption {
                id: "enable".into(),
                label: "Enable Accessibility".into(),
                action_type: HitlActionType::RunCommand {
                    command: "gsettings set org.gnome.desktop.interface toolkit-accessibility true".into(),
                },
            },
            HitlOption {
                id: "skip".into(),
                label: "Skip (use structural verification only)".into(),
                action_type: HitlActionType::Skip,
            },
        ],
        context: "This is a one-time setup to improve KRIA's GUI verification.".into(),
    })
}
```

**Rules:**
- Only shown once per installation (persisted flag in config)
- Non-blocking — user can skip and still use KRIA
- Clear explanation of impact

---

### H11. Code Generation Separation (Vuln #11)

```rust
/// Code generation is a separate concern from substrate routing.
#[async_trait]
pub trait CodeGenerator: Send + Sync {
    /// Generate code from a natural-language hint.
    async fn generate(&self, request: CodeGenRequest) -> CodeGenResult;
}

pub struct CodeGenRequest {
    pub hint: String,
    pub language: Option<String>,
    pub raw_user_text: String,
    pub expected_output_hint: Option<String>,
}

pub struct CodeGenResult {
    pub code: String,
    pub language: String,
    pub extension: String,
    pub expected_output: String,
    pub verifiable_substring: String,
}

/// Default implementation: deterministic template-based generation
pub struct TemplateCodeGenerator;

/// Future: LLM-backed code generation
pub struct LlmCodeGenerator { model_router: Arc<ModelRouter> }
```

**Rules:**
- Substrate router calls `CodeGenerator::generate()`, doesn't own the logic
- Generator is injectable (template-based now, LLM-backed later)
- Planner file stays under 1500 lines

---

### H12. Aggregate Workflow Timeout Enforcement (Vuln #12)

```rust
impl WorkflowExecutor {
    async fn execute_step_with_budget_check(
        &mut self,
        step: &HybridStep,
        memory: &mut WorkflowMemory,
    ) -> StepExecutionResult {
        // Check if we have enough budget for this step
        let step_budget = step.timeout_ms;
        if memory.budget_remaining_ms < step_budget {
            // Not enough time — ask user
            return StepExecutionResult::BudgetExhausted {
                remaining_ms: memory.budget_remaining_ms,
                step_needs_ms: step_budget,
                hitl: HitlReason::WorkflowTakingTooLong {
                    elapsed_ms: memory.total_budget_ms() - memory.budget_remaining_ms,
                    remaining_steps: self.remaining_step_count(),
                },
            };
        }

        let start = Instant::now();
        let result = self.execute_step(step).await;
        let elapsed = start.elapsed().as_millis() as u64;

        // Deduct from budget
        memory.budget_remaining_ms = memory.budget_remaining_ms.saturating_sub(elapsed);

        result
    }
}
```

**Rules:**
- Budget checked before each step, not just at the end
- Exhausted budget → HITL with [Continue anyway] [Cancel]
- User can override (they might be willing to wait)

---

### H13. Step Idempotency Contract (Vuln #13)

```rust
pub struct HybridStep {
    // ... existing fields ...
    /// Whether this step is safe to retry without side effects
    pub idempotent: bool,
    /// Cleanup to run before retrying a non-idempotent step
    pub cleanup_on_retry: Option<CleanupStep>,
}

// Classification:
// Idempotent: write_file (overwrites), open_application (no-op if running)
// Non-idempotent: execute_bash (may append), send_email, delete_file
// 
// The substrate router sets this at plan time based on the action:
fn classify_idempotency(action: &str) -> bool {
    match action {
        "write_file" | "open_application" | "open_application_with_file"
        | "open_url" | "browser_search" | "focus_window" => true,
        "execute_bash" | "execute_python" | "delete_file" | "move_file"
        | "send_notification" | "gw_gmail_send" => false,
        _ => false, // Default: assume non-idempotent (safe)
    }
}
```

---

### H14. Telemetry Backpressure (Vuln #14)

```rust
pub struct TelemetryChannel {
    /// Bounded channel — 64 events max
    tx: mpsc::Sender<TelemetryEnvelope>,
    rx: mpsc::Receiver<TelemetryEnvelope>,
}

impl TelemetryChannel {
    pub fn new() -> (TelemetrySender, TelemetryReceiver) {
        let (tx, rx) = mpsc::channel(64);
        (TelemetrySender { tx }, TelemetryReceiver { rx })
    }
}

impl TelemetrySender {
    pub async fn emit(&self, event: WorkflowTelemetry, source: WorkflowSource) {
        let envelope = TelemetryEnvelope {
            version: 1,
            seq: next_seq(),
            event: event.clone(),
            timestamp_ms: monotonic_ms(),
            source,
        };

        // Critical events must not be dropped
        let is_critical = matches!(
            event,
            WorkflowTelemetry::Completed { .. }
            | WorkflowTelemetry::HitlRequired { .. }
            | WorkflowTelemetry::Cancelled { .. }
        );

        if is_critical {
            // Block until space available (critical events never dropped)
            let _ = self.tx.send(envelope).await;
        } else {
            // Non-critical: try_send, drop if full
            let _ = self.tx.try_send(envelope);
        }
    }
}
```

**Rules:**
- Channel bounded at 64 events
- Critical events (Completed, HitlRequired, Cancelled) block until delivered
- Non-critical events (StepStarted, StepCompleted) dropped if channel full
- Frontend never sees stale state because critical events always arrive

---

### H15. Dry Run / Plan Preview Mode (Vuln #15)

```rust
pub struct WorkflowPreferences {
    /// Whether to show plan preview before execution
    pub preview_before_execute: PreviewPolicy,
}

pub enum PreviewPolicy {
    /// Always show preview and wait for approval
    Always,
    /// Show preview only for destructive or multi-step workflows
    DestructiveOnly,
    /// Never show preview (execute immediately)
    Never,
}

// In the executor, after planning but before execution:
if preferences.preview_before_execute != PreviewPolicy::Never {
    let should_preview = match preferences.preview_before_execute {
        PreviewPolicy::Always => true,
        PreviewPolicy::DestructiveOnly => plan.has_destructive_steps() || plan.steps.len() > 3,
        PreviewPolicy::Never => false,
    };

    if should_preview {
        emit_telemetry(WorkflowTelemetry::PlanPreview {
            workflow_id: plan.workflow_id.clone(),
            title: plan.title.clone(),
            steps: plan.steps.iter().map(|s| s.preview()).collect(),
            outcome_contract_summary: plan.outcome_contract.summarize(),
            requires_approval: true,
        });
        // Suspend until user approves
        wait_for_hitl_response().await;
    }
}
```

**Rules:**
- Default: `DestructiveOnly` (non-intrusive for simple workflows)
- User can change to `Always` or `Never` in settings
- Preview shows steps + expected outcomes, not raw tool calls

---

### H16. Resilient Session Detection (Vuln #16)

```rust
pub async fn check_session_resilient(service: &str, config: &ServiceSessionConfig) -> SessionState {
    // Best-effort detection with graceful fallback
    let result = tokio::time::timeout(
        Duration::from_millis(500),
        check_session_inner(config),
    ).await;

    match result {
        Ok(state) => state,
        Err(_) => SessionState::Unknown, // Timeout → unknown, not error
    }
}

// When session state is Unknown, proceed optimistically:
// - Execute the workflow step
// - If it fails with a login-related error, THEN emit login HITL
// This is more robust than trying to predict session state.

pub fn handle_step_failure_as_login(error: &str, service: &str) -> Option<HitlReason> {
    let lower = error.to_lowercase();
    let login_signals = [
        "login required", "sign in", "not logged in", "session expired",
        "unauthorized", "403", "401", "authentication required",
    ];
    if login_signals.iter().any(|s| lower.contains(s)) {
        Some(HitlReason::LoginRequired {
            service: service.to_string(),
            guidance: format!("Please log into {} to continue.", service),
        })
    } else {
        None
    }
}
```

**Rules:**
- Session detection is best-effort, never blocks workflow planning
- Unknown state → proceed optimistically
- Login failures detected reactively from step errors
- No hardcoded cookie names or session file paths

---

### H17. Telemetry Versioning (Vuln #17)

Already defined in the `TelemetryEnvelope` struct above. Additional rules:

```rust
impl TelemetryReceiver {
    pub fn handle_envelope(&self, envelope: &TelemetryEnvelope) {
        if envelope.version > SUPPORTED_VERSION {
            // Unknown version — log and ignore, don't crash
            tracing::warn!(
                version = envelope.version,
                "Received telemetry with unknown version, ignoring"
            );
            return;
        }
        self.process_event(&envelope.event);
    }
}

const SUPPORTED_VERSION: u8 = 1;
```

**Rules:**
- Frontend ignores events with `version > SUPPORTED_VERSION`
- Backend bumps version only on breaking enum changes
- Non-breaking additions (new fields with defaults) don't bump version

---

### H18. Automated Eval Execution (Vuln #18)

```toml
# In Cargo.toml for kria-eval:
[features]
gui_eval_headless = []  # Level 1-2: no display needed
gui_eval_live = []      # Level 3+: requires DISPLAY

# CI configuration:
# Job 1: cargo test -p kria-eval --features gui_eval_headless (every PR)
# Job 2: cargo test -p kria-eval --features gui_eval_live (nightly, with Xvfb)
```

```rust
// Eval runner with environment detection:
pub fn should_run_live_evals() -> bool {
    std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok()
}

#[cfg(test)]
mod eval_level_1 {
    // Structural correctness — always runnable
    #[test] fn ide_workflow_produces_correct_plan() { .. }
    #[test] fn browser_workflow_selects_correct_substrate() { .. }
}

#[cfg(test)]
#[cfg(feature = "gui_eval_live")]
mod eval_level_3 {
    // Visibility correctness — needs live desktop
    #[tokio::test] async fn vs_code_window_appears_after_launch() { .. }
}
```

**Rules:**
- Level 1-2 evals run on every PR (no display needed)
- Level 3+ evals run nightly with virtual display
- Eval failures block merge (Level 1-2) or create issues (Level 3+)

---

### H19. HITL Debouncing (Vuln #19)

```rust
pub struct HitlDebouncer {
    pending: Vec<HitlRequired>,
    last_emit: Instant,
    debounce_window_ms: u64,  // = 3000
    max_batch_size: usize,    // = 5
}

impl HitlDebouncer {
    pub fn submit(&mut self, hitl: HitlRequired) -> HitlEmitDecision {
        self.pending.push(hitl);

        if self.pending.len() >= self.max_batch_size {
            // Too many — batch immediately
            return self.flush_as_batch();
        }

        if self.last_emit.elapsed() < Duration::from_millis(self.debounce_window_ms) {
            // Within debounce window — hold
            return HitlEmitDecision::Hold;
        }

        // Outside window — emit single
        self.flush_single()
    }

    fn flush_as_batch(&mut self) -> HitlEmitDecision {
        let batch = std::mem::take(&mut self.pending);
        self.last_emit = Instant::now();
        HitlEmitDecision::EmitBatch(HitlBatch {
            title: format!("{} issues need your attention", batch.len()),
            items: batch,
        })
    }

    fn flush_single(&mut self) -> HitlEmitDecision {
        let single = self.pending.remove(0);
        self.last_emit = Instant::now();
        HitlEmitDecision::EmitSingle(single)
    }
}

pub enum HitlEmitDecision {
    EmitSingle(HitlRequired),
    EmitBatch(HitlBatch),
    Hold, // Will be flushed on next tick or when debounce window expires
}
```

**Rules:**
- Maximum 1 HITL modal per 3 seconds
- If >5 HITL triggers fire within the window, batch into one modal
- Batched modal shows all issues with per-issue options
- User can address them individually or "Cancel All"

---

### H20. Legacy Shim Sunset Tracking (Vuln #20)

```rust
pub struct LegacyShimMetrics {
    /// How many workflows used the legacy shim this session
    pub shim_count: AtomicU32,
    /// How many workflows used the new router this session
    pub new_router_count: AtomicU32,
}

impl LegacyShimMetrics {
    pub fn report(&self) -> ShimReport {
        let shim = self.shim_count.load(Ordering::Relaxed);
        let new = self.new_router_count.load(Ordering::Relaxed);
        ShimReport {
            shim_percentage: if shim + new > 0 { (shim * 100) / (shim + new) } else { 0 },
            total_workflows: shim + new,
            recommendation: if shim > 0 && new > shim * 3 {
                "Legacy shim still active — investigate remaining callers"
            } else {
                "Normal operation"
            },
        }
    }
}

// Phase 7: shim emits tracing::warn! on every use
// Phase 8: shim is removed entirely
```

**Rules:**
- Every shim invocation is logged with `tracing::warn!`
- Metrics tracked per session
- Phase 7 deadline: shim must emit warnings
- Phase 8 deadline: shim removed, any remaining callers are compile errors

---

### H21. "Already Satisfied" Detection (Vuln #21)

```rust
pub async fn check_already_satisfied(
    plan: &HybridWorkflowPlan,
    caps: &CapabilitySet,
) -> Option<WorkflowVerdict> {
    // Check if the required outcomes are ALREADY true before execution
    for outcome in &plan.outcome_contract.required_outcomes {
        match &outcome.expectation {
            OutcomeExpectation::ProcessRunning { binary } => {
                if is_process_running(binary) {
                    // App already running — check if it's the right one
                    continue;
                } else {
                    return None; // Not satisfied, proceed with execution
                }
            }
            OutcomeExpectation::AppWindowVisible { app, .. } => {
                // Check if app is already focused
                if let Some(obs) = observe_window_quick(app, caps).await {
                    if obs.found && obs.focused {
                        continue;
                    }
                }
                return None;
            }
            _ => return None, // Can't pre-check this outcome type
        }
    }

    // All required outcomes already satisfied
    Some(WorkflowVerdict::AlreadySatisfied {
        evidence: "All required outcomes were already true before execution".into(),
    })
}

// Add to WorkflowVerdict:
pub enum WorkflowVerdict {
    Complete,
    StructurallyComplete { unverified_outcomes: Vec<String> },
    AlreadySatisfied { evidence: String },  // NEW
    Partial { completed: u32, total: u32, reason: String },
    Blocked { reason: String, recovery: Option<RecoveryPath> },
    Failed { step: u32, reason: String, recovery: Option<RecoveryPath> },
}
```

**Rules:**
- Pre-execution check runs before step 1
- If all required outcomes already true → `AlreadySatisfied` verdict
- Frontend shows: "✓ Already done — VS Code is already open with this project"
- No redundant launches, no duplicate windows

---

### H22. Cancellation Propagation Guarantee (Vuln #22)

```rust
pub struct CancellationEnforcer {
    token: CancellationToken,
    child_tasks: Vec<tokio::task::JoinHandle<()>>,
    grace_period_ms: u64,
}

impl CancellationEnforcer {
    pub async fn cancel_all(&mut self) {
        // Signal all tasks
        self.token.cancel();

        // Wait for graceful shutdown
        let deadline = Instant::now() + Duration::from_millis(self.grace_period_ms);
        for handle in &mut self.child_tasks {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                handle.abort(); // Force kill after grace period
            } else {
                let _ = tokio::time::timeout(remaining, handle).await;
            }
        }

        // Force-kill any remaining child processes
        self.kill_orphaned_processes().await;
    }

    async fn kill_orphaned_processes(&self) {
        // Scan /proc for processes spawned by this workflow
        // (tracked via PID list in WorkflowMemory)
        for pid in &self.spawned_pids {
            let _ = tokio::process::Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .output()
                .await;
        }
    }
}
```

**Rules:**
- Cancellation token propagates to ALL child tasks within 100ms
- Grace period: 2000ms (configurable per workflow)
- After grace period: force-abort remaining tasks
- Orphaned child processes (spawned by execute_bash) are SIGTERM'd
- Partial artifacts are preserved (not deleted on cancel)

---

### H23. User Feedback Loop on Verdicts (Vuln #23)

```rust
// Frontend component:
// After every verdict, show a subtle feedback widget:
// [👍 Correct] [👎 Wrong]

// Backend stores feedback:
pub struct VerdictFeedback {
    pub workflow_id: String,
    pub verdict: WorkflowVerdict,
    pub user_rating: FeedbackRating,
    pub timestamp: Instant,
}

pub enum FeedbackRating {
    Correct,
    Wrong,
    NotRated, // User didn't interact
}

pub struct FeedbackStore {
    db: rusqlite::Connection,
}

impl FeedbackStore {
    pub fn record(&self, feedback: &VerdictFeedback) -> Result<()> {
        self.db.execute(
            "INSERT INTO verdict_feedback (workflow_id, verdict_type, rating, timestamp_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                feedback.workflow_id,
                format!("{:?}", feedback.verdict),
                format!("{:?}", feedback.user_rating),
                feedback.timestamp.elapsed().as_millis() as u64,
            ],
        )?;
        Ok(())
    }

    /// Get false-positive rate for a specific verdict type
    pub fn false_positive_rate(&self, verdict_type: &str) -> f32 {
        // Query: wrong_count / total_rated for this verdict type
        // Used to calibrate confidence thresholds over time
        todo!()
    }
}
```

**Rules:**
- Feedback is optional (user can ignore)
- Stored in SQLite, bounded (last 500 ratings)
- Used to calibrate confidence thresholds (if `StructurallyComplete` is rated "Wrong" >30% of the time, lower the confidence threshold for that verdict)
- Never blocks workflow execution

---

### H24. Multi-Monitor Awareness (Vuln #24)

```rust
pub struct WindowObservation {
    pub found: bool,
    pub mapped: bool,
    pub focused: bool,
    pub confidence: f32,
    pub method: ObservationMethod,
    pub observed_at: Instant,
    /// Which monitor the window is on (if detectable)
    pub monitor_id: Option<u32>,  // NEW
}

// Known limitation documentation:
// On Wayland, monitor assignment is not reliably detectable for all apps.
// KRIA treats any mapped window as "visible" regardless of which monitor.
// Future: use wlr-foreign-toplevel-management on sway/wlroots compositors
// to get per-output window placement.

// For now: if window is found + mapped → visible, regardless of monitor.
// This is correct behavior — a window on monitor 2 IS visible to the user.
```

**Rules:**
- Multi-monitor is a documented known limitation (not a failure)
- Window on any monitor = visible (correct semantics)
- `monitor_id` field reserved for future compositor-specific implementations
- No false failures due to multi-monitor setups

---

### H25. HITL Security Model (Vuln #25)

```rust
pub struct SecureHitlValidator {
    /// Commands that were pre-registered at plan time
    allowed_commands: HashSet<String>,
    /// Option IDs that were emitted in the original HitlRequired event
    allowed_option_ids: HashSet<String>,
}

impl SecureHitlValidator {
    /// Validate that a HITL response matches a pre-registered option.
    /// Rejects any response that wasn't in the original options list.
    pub fn validate_response(
        &self,
        response: &HitlResponse,
        original_options: &[HitlOption],
    ) -> HitlValidation {
        // Check that the response ID matches an original option
        let response_id = response.option_id();
        if !self.allowed_option_ids.contains(response_id) {
            return HitlValidation::Rejected {
                reason: format!("Option '{}' was not in the original options list", response_id),
            };
        }

        // For RunCommand actions, verify the command matches exactly
        if let HitlActionType::RunCommand { command } = &response.action_type {
            if !self.allowed_commands.contains(command.as_str()) {
                return HitlValidation::Rejected {
                    reason: "Command does not match any pre-registered command".into(),
                };
            }
        }

        HitlValidation::Accepted
    }
}

pub enum HitlValidation {
    Accepted,
    Rejected { reason: String },
}

// Security invariant:
// The backend ONLY executes commands that were:
// 1. Generated by the substrate router at plan time
// 2. Included in the HitlRequired event sent to the frontend
// 3. Matched exactly by the response from the frontend
//
// This prevents:
// - XSS in the Tauri webview from executing arbitrary commands
// - Tampered frontend responses from escalating privileges
// - Replay attacks using old HITL option IDs
```

**Rules:**
- All HITL commands are pre-registered at plan time
- Backend rejects any command not in the allowed set
- Option IDs are validated against the original emission
- No arbitrary command execution through HITL responses
- Violation → log security alert + reject silently

---

## 27. Hardening Summary

All 25 vulnerabilities are now integrated as mandatory production contracts:

| # | Vulnerability | Mitigation | Where It Lives |
|---|---|---|---|
| 1 | No retry between steps | `RetryThenFatal` / `RetryThenSkip` in FailurePolicy | WorkflowExecutor |
| 2 | No preemption model | PreemptionHandler with classify + pause/cancel/queue | AgentLoop |
| 3 | Stale capabilities | SpotCheck before visible/interactive steps | CapabilityRevalidator |
| 4 | No cross-workflow memory | SessionObservationCache (bounded, timeout calibration) | Session-level |
| 5 | LLM garbage → silent failure | Structured validation + IntentUnclear HITL | IntentCompiler |
| 6 | No telemetry persistence | SQLite store, last 100 workflows | TelemetryStore |
| 7 | CDP-only browser | BrowserProtocol trait (CDP + Marionette) | BrowserVerifier |
| 8 | Binary AT-SPI state | AtSpiLevel { Full, Partial, BusOnly, None } | CapabilityNegotiator |
| 9 | No lease-break detection | FocusLostEvent → HITL instead of failure | ForegroundLeaseGuard |
| 10 | No accessibility setup | One-time bootstrapping HITL on first GUI workflow | First-run flow |
| 11 | Code gen in planner | CodeGenerator trait, injectable, separate module | substrate/code_generator.rs |
| 12 | No aggregate timeout | Budget check before each step, HITL on exhaustion | WorkflowExecutor |
| 13 | No idempotency contract | `idempotent: bool` + `cleanup_on_retry` per step | HybridStep |
| 14 | No telemetry backpressure | Bounded channel (64), critical events never dropped | TelemetryChannel |
| 15 | No dry-run mode | PlanPreview telemetry + PreviewPolicy setting | WorkflowExecutor |
| 16 | Brittle session detection | Best-effort + reactive login detection from errors | SessionDetector |
| 17 | No telemetry versioning | TelemetryEnvelope with version field | Frontend contract |
| 18 | Evals not automated | Feature-gated test targets, CI integration | kria-eval |
| 19 | HITL bombardment | HitlDebouncer (3s window, batch at 5+) | HITL subsystem |
| 20 | Shim masks bugs | WorkflowSource tracking + sunset deadline | LegacyShimMetrics |
| 21 | No "already done" check | Pre-execution outcome check → AlreadySatisfied | WorkflowExecutor |
| 22 | Cancellation leaks | CancellationEnforcer with grace period + SIGTERM | WorkflowExecutor |
| 23 | No user feedback | Verdict feedback widget → SQLite → threshold calibration | Frontend + FeedbackStore |
| 24 | Multi-monitor blind | monitor_id field + "any monitor = visible" semantics | WindowObservation |
| 25 | HITL command injection | SecureHitlValidator with pre-registered command allowlist | HITL subsystem |

---

## 28. Updated Phase Integration

The 25 hardening items integrate into the existing rollout phases:

| Phase | Additional Hardening Items |
|-------|---------------------------|
| Phase 0 (Foundation) | H17 (versioned envelope), H13 (idempotency types), H8 (AtSpiLevel enum) |
| Phase 1 (ExecutionTarget) | No additional items |
| Phase 2 (Outcome Contract) | H21 (already-satisfied check), H11 (code gen separation) |
| Phase 3 (Capabilities) | H3 (spot-check), H4 (observation cache), H8 (graded AT-SPI), H10 (bootstrapping) |
| Phase 4 (Telemetry) | H6 (persistence), H14 (backpressure), H15 (plan preview), H17 (versioning) |
| Phase 5 (HITL) | H5 (LLM quality gate), H9 (focus-lost), H19 (debouncing), H25 (security) |
| Phase 6 (Simplification) | H20 (shim tracking), H18 (automated evals) |
| Phase 7 (Wayland) | H7 (browser protocol), H24 (multi-monitor) |
| Phase 8 (Alias + Browser) | H16 (resilient session detection) |
| Ongoing | H1 (retry), H2 (preemption), H12 (budget), H22 (cancellation), H23 (feedback) |

---

*End of production-hardened document. Version 2.0.*
