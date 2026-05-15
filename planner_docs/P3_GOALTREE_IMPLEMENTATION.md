# P3 — GoalTree Workflow Cognition: Implementation Plan

## Status: COMPLETE (P3a-P3g ALL DONE — 51 tests passing)

---

## ARCHITECTURAL AUDIT — WHAT EXISTS

### Current Runtime Chain (P1 + P2)

```text
User Request
    ↓
TurnGate (confidence routing)
    ↓
IntentCompiler (normalizer → GuiTaskSpec)
    ↓
GuiExecutionCoordinator::generate_workflow()
    ├── EnvironmentGrounder::ground(targets) ← P2
    └── RuleBasedPlanner::plan(spec, &facts)
            ↓
        GuiWorkflow { sub_goals: Vec<SubGoal> }
            ↓
        GuiExecutor::execute_workflow()
            ├── per-step: GlobalSafetyHalt check
            ├── per-step: kill switch preconditions
            ├── per-step: target window lock verification
            ├── per-step: input action hard halt (wrong window)
            ├── per-step: action budget consumption
            ├── per-step: VerificationType check (bounded micro-retry)
            └── on failure: SafeAbortExecutor
```

### What Each Component Owns

| Component | File | Responsibility |
|---|---|---|
| `IntentCompiler` | `intent_compiler.rs` | Normalizes user intent → `GuiTaskSpec` (single verb) |
| `GuiPlanner` trait | `gui_planner.rs` | Produces `GuiWorkflow` from spec + advisory facts |
| `RuleBasedPlanner` | `gui_planner.rs` | Deterministic single-verb → single-SubGoal mapping |
| `LlmHtnPlanner` | `gui_planner.rs` | LLM-generated workflow for unsupported verbs |
| `SimplePlanner` | `gui_planner.rs` | Rule-first → LLM-fallback composite |
| `GuiWorkflow` | `htn_executor.rs` | Immutable workflow: `Vec<SubGoal>` + `safe_abort_steps` |
| `SubGoal` | `htn_executor.rs` | Single action + params + `VerificationType` + timeout |
| `GuiExecutor` | `htn_executor.rs` | Executes workflow with 8-layer safety validation |
| `BoundedExecutionVerifier` | `execution_verifier_impl.rs` | 7 Verifiability classes, ≤500ms each |
| `PrerequisiteChecker` | `htn_executor.rs` | Sense/Focus/State prerequisite verification |
| `SelfCorrection` | `htn_executor.rs` | Bounded recovery injection (spiral detection) |
| `EnvironmentGrounder` | `environment_grounder.rs` | Advisory OS facts (focus, windows, monitors, CWD) |
| `GroundingCache` | `environment_grounder.rs` | ArcSwap + generation counter, 10s TTL |

### Current Limitation: Single-Verb Workflows

The `IntentCompiler` currently produces **exactly one `Verb`** per request.
The `RuleBasedPlanner` maps that verb to **exactly one `SubGoal`**.

This means:
- ✅ "Open VS Code" → works (single verb: `Open`)
- ✅ "Click save button" → works (single verb: `Click`)
- ❌ "Open VS Code and run cargo test" → **impossible** (two verbs, two stages)
- ❌ "Open terminal, create file, run script" → **impossible** (three stages)
- ❌ "Fix failing build and rerun tests" → **impossible** (multi-stage with verification)

**P3 solves this by introducing staged, multi-action workflows.**

---

## THE GOALTREE BOTTLENECK

### What KRIA Cannot Do Today

1. **Multi-stage workflows**: "Open VS Code and run cargo test" requires:
   - Stage 1: Open/focus VS Code → verify window appeared
   - Stage 2: Open terminal in VS Code → verify terminal panel
   - Stage 3: Type `cargo test` → verify command ran
   - Stage 4: Read output → verify success

2. **Cross-app transitions**: "Open browser and search this error" requires:
   - Stage 1: Focus/open browser → verify browser window
   - Stage 2: Navigate to search → verify URL bar focused
   - Stage 3: Type search query → verify results

3. **Contextual continuity**: "Continue working on project" requires:
   - Stage 1: Ground current state (what's open, where am I?)
   - Stage 2: Focus/open appropriate app
   - Stage 3: Navigate to correct file/location

### What P3 Adds

A **bounded, immutable, stage-oriented workflow structure** that:
- Compiles multi-verb user requests into typed stages
- Defines explicit verification checkpoints between stages
- Supports bounded recovery paths (not infinite retries)
- Maintains the executor/verifier independence invariant
- Never replans, never recursively extends

---

## P3 CORE TYPES

### GoalTree (Immutable After Compile)

```rust
/// Compiled multi-stage workflow. Immutable after construction.
/// Created by the WorkflowCompiler, consumed by the StageExecutor.
pub struct GoalTree {
    /// Unique workflow identifier
    pub workflow_id: String,
    /// Human-readable description of the full workflow
    pub description: String,
    /// Ordered stages — executed sequentially, never reordered
    pub stages: Vec<WorkflowStage>,
    /// Global completion contract
    pub completion: CompletionContract,
    /// Global safe-abort sequence (runs if any stage fails unrecoverably)
    pub global_abort: Vec<SafeAbortStep>,
    /// Maximum total duration across all stages
    pub max_total_duration_sec: u64,
    /// Preconditions that must be true before any stage begins
    pub preconditions: Vec<Precondition>,
}
```

### WorkflowStage

```rust
/// A single stage in the GoalTree. Contains one or more actions
/// grouped by a shared verification checkpoint.
pub struct WorkflowStage {
    /// Stage index (0-based, for logging)
    pub index: u32,
    /// Human-readable label (e.g., "Open VS Code")
    pub label: String,
    /// The actions to execute in this stage
    pub action_group: ActionGroup,
    /// Verification checkpoint — checked AFTER action_group completes
    pub checkpoint: VerificationCheckpoint,
    /// Bounded recovery if checkpoint fails (optional)
    pub recovery: Option<RecoveryPath>,
    /// Context hints for this stage (advisory, from grounder)
    pub context_hints: StageContextHints,
    /// Per-stage timeout (independent of global timeout)
    pub timeout_sec: u64,
    /// Whether this stage can be skipped during recovery
    pub skippable: bool,
}
```

### ActionGroup

```rust
/// A group of actions within a single stage.
/// Actions execute sequentially within the group.
/// The group is sequential-only, non-transactional (cannot be rolled back).
pub struct ActionGroup {
    /// Ordered actions
    pub actions: Vec<StageAction>,
}

/// A single action within a stage.
/// Maps directly to a tool call + verification.
pub struct StageAction {
    /// Action identifier (e.g., "open_application", "type_text")
    pub action: String,
    /// Parameters for the action
    pub params: serde_json::Value,
    /// Per-action verification (VerificationType from existing system)
    pub verify: VerificationType,
    /// Per-action timeout
    pub timeout_ms: Option<u64>,
}
```

### VerificationCheckpoint

```rust
/// Stage-level verification checkpoint.
/// Checked after all actions in the stage complete.
/// The checkpoint determines whether to proceed to the next stage.
pub enum VerificationCheckpoint {
    /// Verify window state (most common for app-switching stages)
    WindowFocused {
        title_contains: Option<String>,
        class: Option<String>,
        pid: Option<u32>, // Optional, extracted from grounder facts
    },
    /// Verify text appears in a target (terminal output, file, etc.)
    OutputContains {
        expected: String,
        target: VerifyTarget,
        case_insensitive: bool,
    },
    /// Verify process is running
    ProcessRunning {
        binary: String,
    },
    /// Verify filesystem effect
    FileEffect {
        path: PathBuf,
        effect: FsEffect,
    },
    /// No checkpoint — proceed unconditionally.
    /// VALIDATION RULE: Only permitted on the terminal (last) stage.
    None,
}
```

### RecoveryPath

```rust
/// Bounded recovery for a failed checkpoint.
/// Recovery is finite: max 2 attempts, each with a single action sequence.
pub struct RecoveryPath {
    /// Maximum recovery attempts (HARD CAP: 2)
    pub max_attempts: u8,
    /// Recovery action to try
    pub recovery_action: RecoveryAction,
}

/// What to do when a checkpoint fails.
pub enum RecoveryAction {
    /// Retry from a specific action index within the stage.
    /// The compiler guarantees actions[0..restart_from] are idempotent
    /// or already completed. restart_from_index: 0 repeats the whole stage.
    RetryFromAction { 
        restart_from_index: u32 
    },
    /// Execute a specific corrective action, then re-check
    Corrective {
        actions: Vec<StageAction>,
    },
    /// Skip this stage (VALIDATION RULE: only allowed if stage.skippable == true)
    SkipStage,
    /// Abort the entire workflow
    AbortWorkflow,
}
```

### CompletionContract

```rust
/// How the workflow reports success to the caller.
pub enum CompletionContract {
    /// All stages completed and all checkpoints passed
    AllStagesPassed,
    /// Specific final verification
    FinalVerification(VerificationCheckpoint),
    /// User must confirm completion
    UserConfirmation { 
        prompt: String,
        timeout_sec: u64, // Bounded wait, default 60
    },
}
```

### StageContextHints (Advisory, from Grounder)

```rust
/// Advisory context for a stage. Populated from OperationalFacts.
/// The executor MAY use these for logging but MUST NOT skip stages
/// based on them alone.
pub struct StageContextHints {
    /// Expected focused app for this stage (if known, populated via normalize_app_name)
    pub expected_app: Option<String>,
    /// Whether the target app is likely already open (from visible_windows)
    pub target_likely_open: bool,
    /// Expected CWD for terminal stages
    pub expected_cwd: Option<PathBuf>,
}
```

### Precondition

```rust
/// A precondition that must be verified before the workflow begins.
pub enum Precondition {
    /// An app must be available (installed). Probed via which::which().
    AppAvailable(String),
    /// Display server must support queries
    DisplayServerAvailable,
    /// A specific window must be visible
    WindowVisible { class: String },
}
```

---

## P3 IMPLEMENTATION PHASES

### P3a: Core Types ✅ DONE

**Goal**: Define the GoalTree type hierarchy.

**Deliverables**:
- [x] `GoalTree` struct
- [x] `WorkflowStage` struct
- [x] `ActionGroup` + `StageAction` structs
- [x] `VerificationCheckpoint` enum
- [x] `RecoveryPath` + `RecoveryAction` enums
- [x] `CompletionContract` enum
- [x] `StageContextHints` struct
- [x] `Precondition` enum
- [x] Serialize/Deserialize for all types
- [x] `GoalTree::validate()` — boundedness checks (all 11 validation rules)
- [x] Unit tests for type construction (19 tests, all passing)
- [x] `VerificationCheckpoint::to_verifiability()` bridge to existing verifier

**Implementation Notes (P3a)**:
- Added `Serialize, Deserialize` to `FsEffect` and `VerifyTarget` in `execution_verifier.rs` (non-breaking additive change) so `VerificationCheckpoint` can serialize.
- `GoalTree` exposes ZERO `&mut self` methods — immutability enforced structurally.
- `GoalTree::validate()` returns ALL errors, not just the first — aids debugging.
- `StageContextHints` derives `Default` for ergonomic test construction.
- `SafeAbortStep` is re-defined locally (not imported from htn_executor) to keep GoalTree domain-independent.

**File**: `kria-core/src/agent/goal_tree.rs` (new module)

**Boundedness enforcements**:
- `GoalTree.stages.len()` ≤ `MAX_STAGES` (8)
- `ActionGroup.actions.len()` ≤ `MAX_ACTIONS_PER_STAGE` (6)
- `RecoveryPath.max_attempts` ≤ 2
- `max_total_duration_sec` ≤ 300 (5 minutes)
- `GoalTree` implements no `&mut self` methods (immutable after construction)
- `VerificationCheckpoint::None` rejected on non-terminal stages
- `RecoveryAction::SkipStage` rejected unless `stage.skippable == true`

---

### P3b: WorkflowCompiler (Immutable Compilation) ✅ DONE

**Goal**: Compile multi-verb user requests into GoalTree.

**Trait Definition**:
```rust
/// WorkflowCompiler MUST be a pure function. It receives intent specs and facts 
/// and returns a GoalTree. It MUST NOT: call tools, query the OS, mutate state, 
/// perform network I/O, or call other compilers/planners recursively.
pub trait WorkflowCompiler { ... }
```

**Deliverables**:
- [x] `WorkflowCompiler` trait
- [x] `RuleBasedWorkflowCompiler` — deterministic multi-verb patterns
- [x] Pattern: `Open X and Run Y` → 2 stages
- [x] Pattern: `Open X, Type Y, Save` → 3 stages
- [x] All 7 verb types: Open, Switch, Type, Run, Click, Close, Save
- [x] `compile(spec, facts) → Result<GoalTree, CompileError>`
- [x] `CompileError` enum (6 variants: SingleVerb, NoVerbs, TooManyClauses, UnsupportedVerb, MissingParameter, ValidationFailed)
- [x] Validation pass: all stages have valid checkpoints (non-terminal stages get proper checkpoints)
- [x] Validation pass: recovery budget verified via GoalTree::validate()
- [x] `MultiVerbSpec` + `VerbClause` types for multi-verb detection
- [x] Advisory `StageContextHints` populated from `OperationalFacts`
- [x] Non-terminal stages get default recovery (RetryFromAction, max_attempts=1)
- [x] 13 tests passing

**Implementation Notes (P3b)**:
- Save/Close verbs get permissive `WindowFocused` checkpoint when non-terminal (can't verify save effect directly)
- Compiler is sync (not async) — pure function, no I/O
- `build_context_hints()` checks `visible_windows` for target-likely-open advisory
- Compiler auto-validates via `GoalTree::validate()` before returning

**File**: `kria-core/src/agent/workflow_compiler.rs` (new module)

**Key design decisions**: 
1. The WorkflowCompiler is a SEPARATE concern from the GuiPlanner. The existing `GuiPlanner::plan()` continues to work for single-verb requests. The `WorkflowCompiler` is invoked ONLY when the IntentCompiler detects a multi-verb request.
2. Invariant: `WorkflowCompiler` MUST NOT accept single-verb specs. If only one verb is detected, it MUST reject with `CompileError::SingleVerb` and the coordinator MUST fall through to `GuiPlanner::plan()`.

**Multi-verb detection**: Extend `IntentCompiler` to detect conjunctions ("and", "then", ";") and produce a `MultiVerbSpec` alongside the existing `GuiTaskSpec`. The coordinator checks for multi-verb first. For requests that can't be split (e.g. "fix the failing build"), the whole request falls through to `LlmHtnPlanner` which already handles complex single-verb workflows.

---

### P3c: StageExecutor (Bounded Stage Execution) ✅ DONE

**Goal**: Execute GoalTree stages with checkpoint verification.

**Deliverables**:
- [x] `StageExecutor` struct
- [x] `execute_goal_tree(tree: &GoalTree, cancel: CancellationToken) → GoalTreeResult`
- [x] Per-stage execution loop (sequential, bounded)
- [x] Checkpoint verification after each stage
- [x] Recovery path execution (all 4 RecoveryAction variants: RetryFromAction, Corrective, SkipStage, AbortWorkflow)
- [x] Cancellation propagation (cancel any stage → abort + global abort)
- [x] Global timeout enforcement (checked before each stage)
- [x] Per-stage timeout enforcement (effective_deadline = min(stage, global))
- [x] Per-action timeout enforcement (tokio::time::timeout)
- [x] Safe abort on unrecoverable failure
- [x] Integration with existing `ToolExecutor` for action dispatch
- [x] Integration with existing `ExecutionVerifier` for checkpoints (via `to_verifiability()` bridge)
- [x] `GoalTreeResult` + `StageResult` + `StageOutcome` result types
- [x] Completion contract verification
- [x] 9 tests passing

**Implementation Notes (P3c)**:
- StageExecutor takes `&GoalTree` (immutable borrow) — cannot mutate
- Recovery re-checks checkpoint after each recovery action
- None checkpoint short-circuits to true (no verifier call)
- Global abort runs on: cancellation, timeout, unrecoverable stage failure
- Action dispatch uses tokio::time::timeout per-action

**File**: `kria-core/src/agent/stage_executor.rs` (new module)

**Critical invariant**: The StageExecutor NEVER replans. If a stage fails after recovery attempts, it aborts the workflow. It does NOT call the planner to generate new stages.

**Reuse**: The StageExecutor wraps the existing `ToolExecutor` for action dispatch and the existing `BoundedExecutionVerifier` for checkpoint verification. It does NOT duplicate their functionality. Checkpoints map directly to `Verifiability` enums.

---

### P3d: Workflow Continuity Integration ✅ DONE

**Goal**: Wire GoalTree into the existing coordinator + grounder.

**Deliverables**:
- [x] `GuiExecutionCoordinator::generate_multi_stage_workflow()` — compiles MultiVerbSpec via grounder + WorkflowCompiler
- [x] `GuiExecutionCoordinator::execute_goal_tree()` — executes via StageExecutor with heartbeat
- [x] `StageContextHints` populated from `OperationalFacts` (visible_windows check, terminal_cwd)
- [x] All targets from all clauses fed to grounder for relevance filtering
- [x] StageExecutor creates fresh ToolExecutor + Verifier per execution (no stale state)
- [x] Heartbeat task for uinput daemon (same pattern as existing execute_workflow)

**Implementation Notes (P3d)**:
- Single-verb path (`generate_workflow` + `execute_workflow`) is COMPLETELY UNCHANGED
- New methods are additive — no modification to existing code paths
- Grounder is called once at compile time (not per-stage yet — DEFERRED to when cache staleness becomes a real problem)

**Grounder Timing**: The StageExecutor queries the grounder at the start of each stage IF `GroundingCache::get_if_fresh()` returns `None` (stale). If cache is fresh, no query. Maximum one `ground()` call per stage boundary. The executor NEVER polls the grounder mid-stage.

---

### P3e: Executor/Verifier Hardening ✅ DONE

**Goal**: Harden stage-level execution with existing safety layers.

**Deliverables**:
- [x] Global action budget enforcement (MAX_TOTAL_ACTIONS=100, tracked across all stages)
- [x] Safe abort propagation from any stage (global_abort runs on failure/cancel/timeout)
- [x] Recovery limit enforcement (MAX_RECOVERY_ATTEMPTS=2, enforced at both compile and runtime)
- [x] Per-stage timeout enforcement (effective_deadline = min(stage_timeout, global_deadline))
- [x] Global workflow timeout enforcement (checked before each stage)
- [x] Per-action timeout enforcement (tokio::time::timeout per action)
- [x] Cancellation propagation (checked before each stage + before each action)

**Implementation Notes (P3e)**:
- Target window lock per stage DEFERRED — existing kill_switch interceptor handles wrong-window at the tool level already
- Stage-level action budget is subsumed by global MAX_TOTAL_ACTIONS counter — per-stage budgets are implicit via MAX_ACTIONS_PER_STAGE compile-time validation
- All existing executor safety layers preserved — StageExecutor wraps ToolExecutor which goes through the registry, kill_switch still applies

---

### P3f: Observability ✅ DONE

**Goal**: Stage-level visibility for debugging.

**Deliverables**:
- [x] `GoalTreeStatus` struct (serializable with serde)
- [x] `WorkflowStatus` enum: Completed / Failed / Cancelled / InProgress
- [x] `StageStatus` struct with per-stage details
- [x] `StageProgressStatus` enum: Passed / PassedAfterRecovery / Skipped / Failed / Cancelled / TimedOut / Pending / InProgress
- [x] `GoalTreeStatus::from_result()` conversion from GoalTreeResult
- [x] Stage progress: actions_executed, recovery_attempts, duration_ms
- [x] Total elapsed time: elapsed_ms

**Deferred**:
- Tauri command `get_workflow_status` — deferred until frontend integration needed (data types are ready)

**NOT**: cognition dashboards, semantic tracing, AI introspection.

---

### P3g: Tests ✅ DONE

**Goal**: Validate all real-world scenarios.

**Test categories**:
- [x] GoalTree construction + validation (goal_tree.rs — 19 tests)
- [x] Boundedness enforcement: max stages, max actions, max recovery, max duration (goal_tree.rs)
- [x] Single-stage workflow backward compatibility (stage_executor.rs)
- [x] Multi-stage workflow execution (stage_executor.rs)
- [x] Checkpoint verification pass/fail (stage_executor.rs)
- [x] Recovery path execution: retry, corrective, skip, abort (stage_executor.rs)
- [x] Cancellation mid-stage (stage_executor.rs)
- [x] Global timeout enforcement (stage_executor.rs)
- [x] Degraded mode — empty facts compilation (stage_executor.rs)
- [x] GoalTree serialization roundtrip (goal_tree.rs)
- [x] End-to-end compile→execute→observe pipeline (stage_executor.rs)
- [x] All 7 verb compilation patterns (workflow_compiler.rs — 13 tests)
- [x] Observability status conversion + serialization (stage_executor.rs)
- [x] Maximum stages workflow (stage_executor.rs)
- [x] Compiler purity/determinism (workflow_compiler.rs)
- [x] GoalTree immutability during execution (stage_executor.rs)

**Deferred test categories**:
- Wrong-window detection within stage — handled by existing kill_switch; not P3-specific
- Focus change between stages — requires live X11 session; covered by integration tests at app level
- Terminal CWD preservation — requires live terminal; grounder provides advisory hints only

**Test totals**: 51 tests across 3 modules (19 + 13 + 19)

---

## BACKWARD COMPATIBILITY

| Existing Path | P3 Impact |
|---|---|
| `IntentCompiler → GuiTaskSpec (single verb)` | **Unchanged** — single-verb requests continue to use `GuiPlanner::plan()` |
| `RuleBasedPlanner::plan()` | **Unchanged** — still produces single-`SubGoal` workflows |
| `GuiExecutor::execute_workflow()` | **Unchanged** — still executes `GuiWorkflow` directly |
| `BoundedExecutionVerifier` | **Reused** — checkpoint verification delegates to existing verifier |
| `EnvironmentGrounder` | **Reused** — facts populate `StageContextHints` |
| `GroundingCache` | **Reused** — queried between stages if stale |

**New path** (P3 only):
```text
IntentCompiler (detects multi-verb)
    ↓
WorkflowCompiler::compile(multi_spec, facts)
    ↓
GoalTree { stages: [...] }
    ↓
StageExecutor::execute_goal_tree(tree, cancel)
    ↓
  per stage:
    ActionGroup → ToolExecutor (reuse existing)
    Checkpoint → BoundedExecutionVerifier (reuse existing)
    Recovery → bounded retry (max 2)
```

---

## AUTHORITY BOUNDARIES

| Component | CAN | CANNOT |
|---|---|---|
| `IntentCompiler` | Detect multi-verb intent | Execute, query OS, replan |
| `WorkflowCompiler` | Compile GoalTree from spec + facts | Execute, mutate state, extend stages |
| `StageExecutor` | Execute stages, enforce budgets | Replan, add stages, call compiler |
| `BoundedExecutionVerifier` | Verify checkpoints, report outcomes | Replan, retry, mutate workflow |
| `EnvironmentGrounder` | Provide advisory facts | Reason, decide, modify plans |
| `RecoveryPath` | Retry stage actions (max 2) | Generate new stages, recurse |

---

## BOUNDEDNESS INVARIANTS

```text
MAX_STAGES = 8
MAX_ACTIONS_PER_STAGE = 6
MAX_RECOVERY_ATTEMPTS = 2
MAX_TOTAL_ACTIONS = 100 (existing cap)
MAX_WORKFLOW_DURATION_SEC = 300 (5 minutes)
MAX_STAGE_DURATION_SEC = 60 (per stage)
```

These are compile-time constants, not configurable.

---

## REJECTED IDEAS

| Idea | Why Rejected |
|---|---|
| Dynamic subgoal invention | Violates immutability — GoalTree is frozen after compile |
| Recursive replanning on stage failure | Unbounded — use finite RecoveryPath instead |
| LLM-based stage correction | Unbounded latency, uncontrolled output |
| Parallel stage execution | Non-deterministic, hard to verify |
| Stage dependency graph (DAG) | Over-engineering — sequential stages sufficient for desktop workflows |
| GoalTree persistence to disk | Memory/world-model creep — ephemeral only |
| GoalTree modification during execution | Violates immutability invariant |
| Stage-level confidence scoring | Grounder boundary violation — executor verifies, not scores |
| Semantic stage labeling ontology | Ontology creep — free-text labels are sufficient |
| Cross-workflow memory | Memory system creep — each workflow is independent |
| Action fusion optimization | 6 actions per stage max is fine; split stages instead |
| Stage collapsing | If >8 stages, reject; don't add semantic compression logic |
| Regex for output verification | Complexity trap; substring matching handles 99% of cases |

---

## RUNTIME RISKS

| Risk | Mitigation |
|---|---|
| Multi-stage workflows taking too long | Global 300s timeout + per-stage 60s timeout |
| Recovery paths creating infinite loops | Hard cap: 2 attempts, spiral detection reused |
| Stage verification failing spuriously | Checkpoint matches existing VerificationType — proven |
| Focus drift between stages | Re-query grounder between stages, target lock resets per stage |
| GoalTree bloat from LLM compiler | Validation pass enforces MAX_STAGES + MAX_ACTIONS_PER_STAGE |
| Backward-incompatible changes | Single-verb path completely unchanged |

---

## FILE PLAN

| New File | Purpose |
|---|---|
| `kria-core/src/agent/goal_tree.rs` | GoalTree types + validation |
| `kria-core/src/agent/workflow_compiler.rs` | Multi-verb → GoalTree compilation |
| `kria-core/src/agent/stage_executor.rs` | Stage-by-stage execution engine |

| Modified File | Change |
|---|---|
| `kria-core/src/agent/mod.rs` | Add `goal_tree`, `workflow_compiler`, `stage_executor` modules |
| `kria-core/src/agent/gui_wiring.rs` | Add `generate_multi_stage_workflow()` method |
| `kria-core/src/agent/intent_compiler.rs` | Add multi-verb detection output |

---

## IMPLEMENTATION ORDER

```text
P3a: Core Types (GoalTree, WorkflowStage, etc.)
 ↓
P3b: WorkflowCompiler (compile multi-verb → GoalTree)
 ↓
P3c: StageExecutor (execute GoalTree stages)
 ↓
P3d: Continuity Integration (grounder, window, CWD)
 ↓
P3e: Hardening (budgets, timeouts, recovery limits)
 ↓
P3f: Observability (stage status endpoint)
 ↓
P3g: Tests (full scenario coverage)
```

---

## DONE

- [x] Full architectural audit of existing runtime chain
- [x] Identified single-verb bottleneck
- [x] Designed GoalTree type hierarchy
- [x] Defined boundedness invariants
- [x] Planned backward-compatible integration
- [x] Rejected overengineering ideas
- [x] Documented authority boundaries
- [x] Incorporated architectural review refinements
- [x] **P3a: Core types implemented** (goal_tree.rs — 19 tests passing)
  - GoalTree, WorkflowStage, ActionGroup, StageAction
  - VerificationCheckpoint, RecoveryPath, RecoveryAction
  - CompletionContract, StageContextHints, Precondition, SafeAbortStep
  - GoalTree::validate() with 11 boundedness rules
  - VerificationCheckpoint::to_verifiability() bridge
  - Full serde roundtrip support
- [x] **P3b: WorkflowCompiler implemented** (workflow_compiler.rs — 13 tests passing)
  - WorkflowCompiler trait + RuleBasedWorkflowCompiler
  - MultiVerbSpec + VerbClause types
  - All 7 verb types supported (Open, Switch, Type, Run, Click, Close, Save)
  - CompileError enum (6 variants)
  - Advisory StageContextHints from OperationalFacts
  - Compile-time validation via GoalTree::validate()
- [x] **P3c: StageExecutor implemented** (stage_executor.rs — 9 tests passing)
  - Sequential stage execution with checkpoint verification
  - All 4 RecoveryAction variants handled
  - Cancellation + timeout propagation
  - Global abort on failure
  - GoalTreeResult + StageResult result types
- [x] **P3d: Continuity Integration** (gui_wiring.rs — additive methods)
  - `generate_multi_stage_workflow()` compiles MultiVerbSpec via grounder + compiler
  - `execute_goal_tree()` executes via StageExecutor with heartbeat
  - Existing single-verb path completely unchanged
- [x] **P3e: Executor hardening**
  - Global action budget (MAX_TOTAL_ACTIONS=100)
  - Triple timeout enforcement (per-action, per-stage, global)
  - Recovery limit enforcement at compile + runtime
  - Global abort on any failure path
- [x] **P3f: Observability** (stage_executor.rs)
  - GoalTreeStatus, WorkflowStatus, StageStatus, StageProgressStatus
  - GoalTreeStatus::from_result() conversion
  - All types serializable
- [x] **P3g: Comprehensive tests** (51 tests total)
  - 19 goal_tree tests (types, validation, serialization, bridge)
  - 13 workflow_compiler tests (all verb patterns, rejection, purity)
  - 19 stage_executor tests (execution, recovery, observability, integration)

## COMPLETE

All P3 phases (P3a-P3g) are implemented and tested.

## RUNTIME RISK LOG

- No risks observed. WorkflowCompiler remains pure — no I/O, no state, sync function.
- StageExecutor takes `&GoalTree` — cannot mutate. Authority boundary intact.
- Recovery is bounded (max 2 attempts, enforced at both compile and runtime).
- Grounder is called once at compile time, not per-stage boundary. If stale facts become a problem, can add per-stage re-query later (DEFERRED, not blocked).
- Target window lock per-stage DEFERRED — existing kill_switch interceptor handles wrong-window at tool level.

## DEFERRED ITEMS

- Per-stage grounder re-query (cache staleness not yet a real problem)
- Per-stage target window lock (existing kill_switch handles this at tool level)
- Tauri command `get_workflow_status` (data types ready, wiring deferred until frontend needs it)

## AUTHORITY BOUNDARY CHECKS

- ✅ GoalTree has no &mut self methods — immutability enforced
- ✅ GoalTree::validate() is &self only — no mutation
- ✅ VerificationCheckpoint::to_verifiability() is &self only — bridges to existing verifier without side effects
- ✅ No execution logic in goal_tree.rs — data types + validation only
- ✅ WorkflowCompiler::compile() is sync, pure, no I/O — decomposition only
- ✅ WorkflowCompiler rejects SingleVerb — coordinator must use GuiPlanner for single-verb
- ✅ StageExecutor takes &GoalTree (immutable borrow) — cannot mutate workflow
- ✅ StageExecutor never calls any planner or compiler — no replanning
- ✅ StageExecutor delegates to existing ToolExecutor + ExecutionVerifier — no duplication

## REJECTED OVERENGINEERING (implementation phase)

- Rejected: importing SafeAbortStep from htn_executor — defined locally to keep GoalTree self-contained
- Rejected: generic validation trait — concrete validate() method is simpler and sufficient
- Rejected: builder pattern for GoalTree — WorkflowCompiler constructs directly, builder adds indirection
- Rejected: async WorkflowCompiler — it's a pure function, sync is correct and simpler
- Rejected: parallel action execution within stages — sequential is deterministic, sufficient for desktop workflows
- Rejected: staged verifier with retry logic — checkpoint verification delegates to existing BoundedExecutionVerifier, recovery is separate concern
