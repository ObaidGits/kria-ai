# KRIA GUI Automation Cognition Runtime

Production architecture handbook for KRIA's desktop cognition runtime.

Updated from the current KRIA working tree on 2026-05-27. Source references point to the files that define the runtime today.

## Reader Contract

This handbook is meant to be readable by both non-specialists and engineers:

- **Human explanation first:** each subsystem is explained in ordinary terms before the
  source-level names are introduced.
- **Current implementation is source-backed:** Sections 1-12 describe the current GUI
  cognition architecture and observed failure modes.
- **Future work is separate:** stabilization and improvement ideas are kept in
  **Section 13. Production Hardening Roadmap**.
- **Diagrams are mental maps:** they compress runtime behavior into readable flows; source
  references remain authoritative.
- **No screenshot-only assumptions:** if a claim sounds like "the screen looked right",
  this document explains what semantic evidence the runtime needs instead.

---

## 1. Executive Overview

KRIA GUI Cognition is the part of KRIA that turns a human desktop instruction into a bounded, verifiable, recoverable workflow. It is not a screen macro system. It is a cognition runtime: it classifies intent, chooses an execution substrate, builds an explicit workflow, performs actions through policy-gated tools, verifies semantic completion, and either finishes with evidence or fails closed.

Simple automation tools usually map text to clicks or keystrokes. KRIA tries to map text to the most reliable operational substrate:

- "write code and run it" usually becomes file creation + process execution, not fragile typing into an editor.
- "open youtube" becomes browser/CDP navigation where possible, not blind coordinate clicking.
- "click the OK button" becomes AT-SPI semantic element lookup when available, not screenshot-only matching.
- "delete all files in Downloads" becomes a destructive filesystem operation requiring policy/HITL, not an autonomous GUI sequence.

The current design is contract-before-substrate: KRIA first records what workflow form the user appears to expect, then selects the safest concrete substrate that can satisfy or honestly degrade that contract.

```text
User prompt
   |
   v
TurnGate / intent routing
   |
   v
IntentCompiler -> GuiTaskSpec
   |
   v
SemanticWorkflowFrame -> ExecutionModeDecision -> WorkflowIntentContract
   |
   v
VerifierAuthorityAssessment / HybridSynchronization metadata
   |
   v
SubstratePlanner / GoalTree compilers
   |
   v
Policy + HITL + execution authority
   |
   v
ToolRegistry -> GUI / browser / filesystem / shell tools
   |
   v
BoundedExecutionVerifier
   |
   v
Result synthesis, PSDG memory, transparency trace, session checkpoint
```

### Cognition Pipeline

```text
Natural language
  -> bounded intent envelope
  -> typed GUI task spec
  -> semantic workflow frame
  -> fidelity and execution-mode decision
  -> declarative workflow contract
  -> substrate decision
  -> immutable workflow
  -> policy-gated execution
  -> observable verification
  -> recovery or completion
```

KRIA uses semantic cognition because GUI automation is unreliable when treated as pixels only. Screenshots do not know whether a file was actually written, whether a browser page finished loading, whether a focused window changed, or whether the output the user asked to "show" was surfaced. KRIA therefore uses layered evidence: filesystem checks, process checks, CDP browser state, AT-SPI accessibility state, OCR when needed, and HITL when an outcome is inherently unverifiable.

---

## 2. High-Level Runtime Architecture

### Layered Architecture Map

```text
+---------------------------------------------------------------------+
| User / UI / Voice / API                                             |
+-------------------------------+-------------------------------------+
                                |
+-------------------------------v-------------------------------------+
| Turn Cognition                                                      |
| TurnGate, routing classifiers, correction context, resource planning|
+-------------------------------+-------------------------------------+
                                |
+-------------------------------v-------------------------------------+
| Intent + Workflow Cognition                                         |
| IntentCompiler, MultiIntentDecomposer, OpGraph, GoalTree, planners  |
+-------------------------------+-------------------------------------+
                                |
+-------------------------------v-------------------------------------+
| Grounding + State Intelligence                                      |
| EnvironmentGrounder, PSDG, browser cognition, IDE cognition         |
+-------------------------------+-------------------------------------+
                                |
+-------------------------------v-------------------------------------+
| Execution Authority                                                 |
| PolicyToolExecutor, PolicyEngine, HITL, audit, preflight, authority |
+-------------------------------+-------------------------------------+
                                |
+-------------------------------v-------------------------------------+
| Substrates + Tools                                                  |
| ToolRegistry, GuiExecutor, StageExecutor, uinput daemon, CDP, AT-SPI|
+-------------------------------+-------------------------------------+
                                |
+-------------------------------v-------------------------------------+
| Verification + Recovery                                             |
| BoundedExecutionVerifier, WorkflowContinuationRuntime, safe abort   |
+---------------------------------------------------------------------+
```

### Major Runtime Systems

| System | Main Files | Runtime Purpose |
| --- | --- | --- |
| TurnGate | `crates/kria-core/src/agent/turn_gate.rs` | Produces `IntentEnvelope`, operation class, hazard hint, compute plan, and routing hints. |
| IntentCompiler | `intent_compiler.rs`, `intent_compiler_rule.rs` | Normalizes user text into `GuiTaskSpec` without executing or reading environment state. |
| SemanticWorkflowFrame | `semantic_workflow.rs` | Classifies task family, app anchors, visibility, observation, collaboration, ambiguity, safety class, and fidelity. |
| ExecutionModeReasoner | `execution_mode_reasoner.rs` | Deterministically selects structural, visible, hybrid, human-collaborative, verification-visible, or silent mode. |
| WorkflowIntentContractRegistry | `workflow_intent_contract.rs` | Holds declarative contracts for visible coding, browser, media, human review, silent execution, and visible verification. |
| VerifierAuthorityEvaluator | `verifier_authority.rs` | Defines what authority and freshness each verifier must provide before a claim can be accepted. |
| HybridSynchronizationEvaluator | `hybrid_synchronization.rs` | Defines structural-visible sync checkpoints for hybrid workflows. |
| BrowserMediaGovernanceEvaluator | `browser_media_governance.rs` | Detects browser/media session risk and required HITL/visible-verifier metadata. |
| GoalTree | `goal_tree.rs`, `stage_executor.rs` | Immutable multi-stage workflow representation and bounded executor. |
| GuiPlanner | `gui_planner.rs` | Unified planner trait plus deterministic `RuleBasedPlanner`. |
| SubstratePlanner | `gui_substrate_planner.rs` | Decides whether the task should use file, terminal, browser, app-open, keystroke, or interaction substrate. |
| EnvironmentGrounder | `environment_grounder.rs` | Bounded snapshot of focused window, visible windows, display server, monitors, terminal cwd, and capabilities. |
| ExecutionVerifier | `execution_verifier.rs`, `execution_verifier_bounded.rs` | Converts planned verification classes into bounded observable checks. |
| WorkflowContinuationRuntime | `workflow_continuation/mod.rs` | Classifies interruptions and creates bounded recovery/pause plans. |
| PSDG | `psdg/mod.rs`, `psdg/env_tracker.rs`, `world_model/*` | Persistent semantic desktop graph for continuity and context injection. |
| HITL | `safety/hitl.rs`, `safety/policy_gate.rs` | Human approval path for red/destructive or unverifiable actions. |
| ToolRegistry | `tools/registry.rs`, `tools/mod.rs` | Registered tool handlers for filesystem, shell, GUI, browser, desktop, AT-SPI. |
| GuiExecutor | `htn_executor.rs`, `gui_wiring.rs` | Executes `GuiWorkflow` sub-goals with kill switch, target lock, retries, and verification. |
| Recovery | `workflow_continuation/mod.rs`, `stage_executor.rs` | Recovery classification, retry/skip/escalate decisions, pause checkpoints. |
| Transparency | `execution_transparency/mod.rs`, `GuiWorkflowViewer.tsx` | Human-visible workflow stage trace and blockers. |

### Prompt Flow Through the System

```text
Prompt
  |
  v
TurnGate::plan_turn()
  - operation = Automate/Search/Delete/etc.
  - risk/hazard hint
  - direct tool hint, if any
  |
  v
GuiExecutionCoordinator::should_route_to_gui_executor()
  - explicit GUI tool hint wins
  - Automate/ConfigureSystem requires confidence >= 0.6
  |
  v
RuleIntentCompiler::compile()
  - Verb, TargetRef, ContentClass, ambiguity
  |
  v
analyze_semantic_workflow()
  - TaskFamily, AppAnchor, VisibilityExpectation, WorkflowFidelityResolution
  |
  v
ExecutionModeReasoner::decide()
  - ExecutionMode, WorkflowContractId, RequiredVerifier list
  |
  v
WorkflowIntentContractRegistry::evaluate()
  - missing requirements, forbidden degradations, fallback/HITL policy
  |
  v
VerifierAuthorityEvaluator::assess()
  - authority/freshness claim boundaries for required verifiers
  |
  v
SubstratePlanner::plan()
  - FileWriteThenOpen
  - TerminalExecution
  - IdeCodeRunWorkflow
  - BrowserNavigate
  - AppOpenOnly
  - Keystroke
  - InteractionHeavy
  - Unknown -> planner fallback
  |
  v
GuiExecutionCoordinator::execute_workflow()
  - heartbeat to daemon
  - policy-wrapped ToolExecutor
  - GuiExecutor or StageExecutor
  |
  v
BoundedExecutionVerifier::verify()
  - filesystem/process/CDP/AT-SPI/OCR/window checks
  |
  v
Success, failure, recovery, HITL pause, or safe abort
```

---

## 3. End-to-End Prompt Lifecycle

### Example 1: "open code and write a program to print pascal triangle and run it and show output"

This is the canonical regression prompt. The current runtime resolves it as a hybrid visible-coding workflow, then uses the IDE code-run substrate when the app anchor is IDE-class.

```text
Prompt
  -> TurnGate: Automate / GUI-capable
  -> RuleIntentCompiler: Open + App("code") + Generated("pascal triangle", python)
  -> SemanticWorkflowFrame: Coding + required IDE anchor + WorkflowVisible
  -> WorkflowFidelityResolution: WorkflowStageFidelity
  -> ExecutionModeDecision: HybridWorkflow / VisibleCodingWorkflow
  -> ContractCheck:
       - source artifact required
       - IDE/app context required
       - workflow/output surfacing required
       - structural execution allowed only as an internal step
  -> Multi-intent guard: detects substrate can handle it, avoids fragile GoalTree path
  -> SubstratePlanner: IdeCodeRunWorkflow
  -> Workflow:
       1. write_file ~/.kria/generated/pascal_*.py
       2. write_file ~/.kria/generated/run_*.sh
       3. open_application_with_file code <file>
       4. execute_bash terminal launcher
  -> Verifier:
       - FileSystemEffect(source exists/contains)
       - FileSystemEffect(runner exists/contains)
       - ProcessLaunched(IDE process best effort)
       - DeterministicOutput(output contains expected lines)
  -> Result synthesis must surface output and disclose structural fallback if visible terminal launch was unavailable
```

Why this no longer relies on typing into VS Code:

- `gui_substrate_planner.rs` recognizes generated code plus run intent.
- `execution_mode_reasoner.rs` marks the request as `HybridWorkflow` with `VisibleCodingWorkflow`.
- For IDE-class apps (`code`, VS Code, IntelliJ, PyCharm, etc.), the substrate is `IdeCodeRunWorkflow`.
- The workflow writes the file structurally, opens the IDE with the file, attempts a visible terminal run, and falls back to structural execution with explicit output-marker disclosure if needed.
- The output is captured as an artifact and must be surfaced to satisfy workflow eval contracts.

Failure points and handling:

| Failure Point | Where Detected | Behavior |
| --- | --- | --- |
| `code` parsed as `code and` | Substrate/app alias tests and eval cases | Regression tests require VS Code alias resolution. |
| Python/code generation missing expected content | File artifact verifier/eval judge | Fail closed; no semantic success. |
| `execute_bash` fails | Tool result in `GuiExecutor`/`StageExecutor` | Abort workflow with error and partial artifacts. |
| Output hidden or not surfaced | `crates/kria-core/tests/workflow_multistep_evals.rs`, `crates/kria-core/tests/real_world_workflow_evals.rs` | Eval fails even if tool execution succeeded. |
| App visible but no semantic output | Contract metadata + bounded verifier + eval contract | Not accepted as complete. |
| Visible terminal unavailable | Runner script fallback marker + captured output | Report degraded/hybrid fallback rather than pretending the visible terminal succeeded. |

Execution chain:

```text
RuleIntentCompiler
   |
   v
SemanticWorkflowFrame -> ExecutionModeDecision -> WorkflowIntentContract
   |
   v
SubstratePlanner::plan_ide_code_run_workflow()
   |
   v
ToolRegistry: write_file -> write_file runner -> open_application_with_file -> execute_bash launcher
   |
   v
BoundedExecutionVerifier: FileSystemEffect + DeterministicOutput
   |
   v
Result response includes generated file and visible output
```

### Example 2: "open youtube and play latest song from my playlist"

This is ambiguous and partly high-risk in a different way: "latest song" and "my playlist" depend on user account state and possibly authentication. KRIA should not pretend to know a private playlist or bypass login.

Likely runtime path:

```text
Prompt
  -> TurnGate: Automate/Search/Browser-capable
  -> IntentCompiler: Open/Browser target, content around YouTube/play playlist
  -> SubstratePlanner: BrowserNavigate or InteractionHeavy
  -> BrowserCognitionEngine:
       launch/reuse Chrome with CDP
       navigate/search YouTube
       read page title/url
  -> If login/playlist access needed:
       WorkflowContinuationRuntime: AuthRequired or BrowserStateChanged
       HITL: ask user to sign in/select playlist
  -> Verification:
       BrowserPageLoaded(url/title)
       possibly AccessibilityElement/InteractionOutcome for play button
       UserAttested if private playlist state is not observable
```

Subsystem participation:

- `browser_cognition.rs` launches Chrome with `--remote-debugging-port=9222`, uses a KRIA profile, and exposes `BrowserState`.
- `execution_verifier_bounded.rs` verifies `BrowserPageLoaded` through CDP first, then process/CDP structural fallback.
- `atspi_engine.rs` can inspect semantic UI elements such as buttons and dialogs.
- `workflow_continuation/mod.rs` classifies auth popups and browser state changes.

Decision tree:

```text
Can KRIA open YouTube?
  yes -> BrowserNavigate
  no  -> fail with browser/tool evidence

Is playlist visible without auth?
  yes -> interact via CDP/AT-SPI
  no  -> HITL: user sign-in or playlist selection

Can "latest song" be resolved from visible page state?
  yes -> click/play and verify media/page state
  no  -> ask clarification or user attestation
```

### Example 3: "delete all files in Downloads"

This is destructive. The correct architecture is not to start clicking in the file manager. It should classify the operation as destructive filesystem modification and require explicit approval.

```text
Prompt
  -> TurnGate: Delete, HazardHint Red/Black depending scope
  -> IntentCompiler/routing: filesystem target ~/Downloads/*
  -> PolicyToolExecutor:
       preflight
       execution authority target validation
       PolicyEngine / command classifier
       HITL approval
  -> If approved:
       run safe, bounded file operation
       verify FileSystemEffect / NotExists or directory empty
  -> If denied/timeout:
       abort, no action
```

Safety behavior:

- `safety/command_classifier.rs` defaults shell metacharacters and unknown destructive commands to Red.
- `safety/policy_gate.rs` models capabilities such as `WriteFilesystem` and `SystemDestructive`.
- `safety/hitl.rs` auto-denies on timeout.
- The runtime should prefer a typed file operation or bounded shell command under policy, not GUI clicks.

HITL lifecycle:

```text
Policy decision requires approval
  -> HitlGateway::request_approval_with_id()
  -> frontend receives ApprovalRequest
  -> Approved / Denied / Timeout
  -> audit log records decision
  -> only Approved reaches ToolRegistry
```

---

## 4. Intent Understanding + Cognition Flow

### Intent Understanding

KRIA separates routing from planning:

- `TurnGate` decides broad operation, hazard, compute class, and tool hints.
- `IntentCompiler` normalizes natural language into `GuiTaskSpec`.
- `SemanticWorkflowFrame` and `ExecutionModeDecision` record workflow expectations before physical planning.
- `WorkflowIntentContract` declares what must be true for faithful completion.
- `SubstratePlanner` decides how to physically satisfy the goal.
- `GuiExecutor` and `StageExecutor` execute only the plan they receive.

`intent_compiler.rs` defines the typed vocabulary:

```rust
pub enum Verb { Open, Type, Click, Run, Save, Close, Switch, Other(String) }
pub enum TargetRef { App(String), File(PathBuf), Url(String), Element(String) }
pub enum ContentClass { Literal(String), Generated { hint: String, language: Option<String> } }
```

The important design boundary: the compiler is a normalizer, not a planner. It does not read windows, screenshot the screen, call tools, or mutate state.

### Multi-Verb Prompts

Multi-verb prompts are decomposed by `multi_intent.rs`, `opgraph.rs`, and `opgraph_compiler.rs`. The current architecture also contains a substrate-first guard in `gui_wiring.rs`: when a multi-intent prompt can be handled more reliably by the `SubstratePlanner`, the system avoids producing a brittle `WindowFocused` GoalTree.

```text
"open code and write pascal and run it"
  |
  +-- Multi-intent decomposition sees multiple clauses
  |
  +-- Substrate guard asks: can a concrete substrate handle this?
       |
       +-- yes: IdeCodeRunWorkflow for IDE run/show prompts, TerminalExecution for structural run/show prompts
       +-- no: OpGraph -> GoalTree -> StageExecutor
```

### GoalTree Philosophy

`goal_tree.rs` defines immutable, bounded workflows:

- max 8 stages
- max 6 actions per stage
- max 2 recovery attempts per stage
- sequential execution only
- no runtime replanning
- explicit checkpoints

This is bounded cognition. The runtime can adapt through finite recovery and HITL, but it cannot hallucinate new stages during execution.

### Substrate Routing Matters

The substrate decision is where KRIA becomes a desktop cognition system rather than a macro player.

| User Goal | Bad Substrate | Preferred KRIA Substrate |
| --- | --- | --- |
| Write a program in VS Code | Type thousands of characters into focused window | Write file directly, open editor with file |
| Open Code, run, and show output | Hidden backend-only execution or fragile terminal typing | `IdeCodeRunWorkflow` with source file, runner script, IDE open, captured output, and visible-terminal fallback disclosure |
| Run code and show output without IDE anchor | Open terminal, type command, OCR output | `execute_bash`, capture output artifact, surface response |
| Open YouTube | Search web and summarize | BrowserNavigate/CDP |
| Click dialog button | Coordinate click | AT-SPI element action |
| Type literal text into current field | File write | Keystroke, with focus lock and daemon safety |

---

## 5. GUI Automation Runtime Internals

### GUI Substrates

`gui_substrate_planner.rs` defines these physical execution modes:

| Substrate | Meaning | Typical Tools |
| --- | --- | --- |
| `FileWriteThenOpen` | Create content as file, then open in app | `write_file`, `open_application_with_file` |
| `TerminalExecution` | Generate file, run command, capture output | `write_file`, `execute_bash`, `open_application_with_file` |
| `IdeCodeRunWorkflow` | Hybrid IDE coding path with visible terminal attempt and structural fallback | `write_file`, `open_application_with_file`, terminal launcher via `execute_bash` |
| `AppOpenOnly` | Launch app only | `open_application` |
| `Keystroke` | Inject text/shortcut | `type_text`, `press_shortcut` |
| `BrowserNavigate` | Browser URL/search | `browser_search`, `open_url`, CDP |
| `InteractionHeavy` | UI elements/dialogs/forms | AT-SPI tools |
| `Unknown` | Planner fallback required | Rule/LLM HTN path |

### Input Injection and IPC

The GUI backend contract is in `tools/gui_automation.rs`:

```text
KRIA core process
  -> YdotoolBackend
  -> Unix domain socket
  -> kria-uinput-daemon
  -> xdotool/uinput-style OS input
```

The daemon (`crates/kria-uinput-daemon/src/main.rs`) accepts JSON commands:

- `Click`
- `Type`
- `Shortcut`
- `ReleaseAll`
- `GetActiveWindow`
- `Heartbeat`
- `TaskComplete`

Safety features:

- dynamic timeouts for long typing
- heartbeat dead-man switch
- emergency modifier release
- active window fallback via AT-SPI and `/proc`
- no LLM memory or user data inside the privileged helper

### Focus Handling

`htn_executor.rs` uses target window locks for input safety:

```text
Before input action:
  get active window
  compare pid/class to target lock
  if mismatch on type/click/shortcut:
      abort immediately
```

This prevents runaway typing into the wrong window. It is intentionally conservative.

### AT-SPI Integration

`atspi_engine.rs` provides semantic GUI access:

- detects AT-SPI bus availability
- lists accessible applications
- prioritizes focused app
- searches accessibility subtrees
- ranks elements by role/name/visibility
- detects/dismisses dialogs

AT-SPI is essential on Wayland because global coordinate and window introspection are restricted.

### Browser Cognition

`browser_cognition.rs` uses Chrome DevTools Protocol:

```text
launch/reuse Chrome with --remote-debugging-port=9222
  -> query tabs via HTTP
  -> communicate over WebSocket CDP
  -> read URL/title/loading/dialog state
  -> persist browser state into PSDG when attached
```

Browser verification uses CDP before weaker fallbacks. This is why `BrowserPageLoaded` is a semantic check when CDP is available.

### IDE Cognition

`ide_cognition.rs` reads VS Code workspace state and can run language-specific checks:

- VS Code recent workspace DB
- log/state heuristics
- Python via ruff or tree-sitter
- Rust/JS checks where available
- PSDG writes for workspace root, active file, and error counts

---

## 6. Execution Verification System

The verifier is intentionally separate from execution. `ExecutionVerifier` only observes and returns `VerifyOutcome`; it does not retry, replan, or fix.

### Verifiability Classes

Defined in `execution_verifier.rs`:

| Class | Evidence Source | Confidence Tier |
| --- | --- | --- |
| `WindowState` | GUI backend, AT-SPI, xdotool fallback | Full/structural depending backend |
| `FileSystemEffect` | filesystem metadata/content | PartialObservable |
| `ProcessLaunched` / `ProcessNotRunning` | `/proc` scan | PartialObservable |
| `DeterministicOutput` | file/terminal/editor output | PartialObservable |
| `OcrTextPresent` | screenshot + Tesseract | StructuralOnly |
| `AccessibilityElement` | AT-SPI tree | FullSemantic |
| `InteractionOutcome` | AT-SPI post-action state | FullSemantic |
| `BrowserPageLoaded` | CDP, process fallback | FullSemantic to StructuralOnly |
| `UserAttested` | human answer | Unobservable until HITL |
| `Unverifiable` | no auto-evidence | Fail closed |

### Verification Lifecycle

```text
Stage/action completes
  |
  v
Convert VerificationType -> Verifiability
  |
  v
BoundedExecutionVerifier::verify()
  |
  +-- success with confidence/evidence
  |
  +-- timeout/failure -> verified=false
          |
          v
      bounded retry in executor
          |
          v
      recovery or abort
```

### Why Visible Apps Can Still Fail

A workflow can fail even when the app visibly opens because "app opened" is not always the semantic goal. Examples:

- The editor opened, but the file was not created.
- The browser opened, but the target page did not load.
- The program ran, but output was not captured or surfaced.
- A window opened under Wayland, but focus could not be verified.
- OCR sees text, but the verifier class requires filesystem or process evidence.

KRIA's verification system is designed to fail closed because false success is more dangerous than honest failure.

---

## 7. Recovery + Interruption Architecture

`workflow_continuation/mod.rs` is the recovery intelligence layer. It classifies failures into interruption classes and creates bounded recovery plans.

### Interruption Taxonomy

```text
InterruptionClass
  |- Popup { is_auth }
  |- FocusTheft
  |- AuthRequired
  |- CompositorEvent
  |- IdeConflict
  |- BrowserStateChanged
  |- NetworkDropped
  |- ProcessCrashed
  |- UserIntervened
  |- Timeout
  |- ResourceExhausted
  |- WindowFocusFailed
  |- InfrastructureFailure
  `- Unknown
```

### Recovery Flow

```text
Verification or stage failure
  |
  v
Build InterruptionContext
  |
  v
WorkflowContinuationRuntime::classify_interruption()
  |
  v
plan_recovery()
  |
  +-- Continue / Retry / SkipStage
  |
  +-- Rollback
  |
  +-- Escalate / RequestHumanIntervention
  |       |
  |       v
  |   pause_workflow() checkpoint
  |
  +-- Abort
```

Recovery is capped at depth 2. This prevents recursive "try something else forever" behavior.

### Daemon and Focus Failures

Infrastructure failures are treated differently from desktop state failures. A dead uinput daemon or service halt maps to `InfrastructureFailure`, while a window not focusing maps to `WindowFocusFailed`. This distinction matters because daemon failures need service recovery or user action; focus failures may be transient.

---

## 8. PSDG + Runtime State Intelligence

PSDG means Persistent Semantic Desktop Graph. It is KRIA's bounded memory of desktop facts, implemented over `WorldModelStore`.

```text
Desktop/browser/IDE/workflow events
  |
  v
PsdgHandle fire-and-forget writes
  |
  v
WorldModelStore (SQLite/WAL)
  |
  v
bounded context injection / continuation / introspection
```

### What PSDG Stores

- focused application
- browser URL and title
- IDE workspace root, active file, error count
- terminal cwd
- visible window count
- workflow stage outcomes
- active workflow status

### Bounded Memory Philosophy

PSDG is observational only:

- writes are fire-and-forget
- reads are bounded to `MAX_CONTEXT_FACTS`
- confidence threshold is enforced
- it never bypasses policy/HITL
- event storms are reduced by delta tracking in `EnvironmentStateTracker`

`environment_grounder.rs` also contains event storm and cache invalidation tests, including `event_storm_invalidation_bounded`.

---

## 9. Human-in-the-Loop (HITL)

HITL exists because not all desktop tasks are safely automatable. Some require human judgment, credentials, or explicit destructive confirmation.

### HITL Triggers

- red/destructive policy decision
- authentication prompt
- user-attested verification
- ambiguous target
- resource exhaustion
- repeated recovery failure
- private account state such as "my playlist"

### HITL Flow

```text
PolicyToolExecutor
  -> PolicyEngine decision requires approval
  -> HitlGateway creates ApprovalRequest
  -> UI/voice/API presents request
  -> Approved / Denied / Timeout
  -> AuditLogger records outcome
  -> Approved continues, Denied/Timeout aborts
```

`HitlGateway` auto-denies on timeout. This is a trust boundary: silence is not consent.

---

## 10. Eval + Testing Architecture

KRIA's eval system has moved from declarative "tool returned success" checks toward semantic, observable, real-world cognition evals.

### Testing Layers

```text
Unit tests
  -> parser/planner/verifier contracts

Integration evals
  -> GoalTree, fault injection, policy, recovery

Workflow evals
  -> semantic completion contracts

GUI evals
  -> real substrate pipeline, apps, artifacts, display server behavior

VM destructive evals
  -> delete/kill/rm operations in safe virtual environment
```

### Key Eval Files

- `crates/kria-core/src/bin/test_gui_e2e.rs`
- `crates/kria-core/src/bin/kria-test.rs`
- `crates/kria-core/src/test_runner/mod.rs`
- `crates/kria-core/tests/workflow_multistep_evals.rs`
- `crates/kria-core/tests/real_world_workflow_evals.rs`

### GUI Eval Evidence

Generated GUI eval reports under `tests-logs/` should report:

| Metric | Value |
| --- | ---: |
| Total cases | Number of evaluated prompts |
| Passed | Cases satisfying structural and workflow assertions |
| Failed | Cases requiring investigation |
| Skipped | Cases omitted by environment gates |
| False success count | Completion claims without sufficient evidence |
| Retrieval leakage count | Prompt/context leakage into generated artifacts |
| Display server | Runtime GUI environment |

Important cases include:

- `regression-003-code-pascals-triangle`
- `wayland-003-workflow-survives-window-id-failed`
- `bugfix-005-daemon-wayland-fallback`
- output size limits and artifact isolation cases

### Why Old Evals Missed Production Bugs

Old evals often validated only that a tool call returned success. They missed:

- success claims without visible output
- `code and` parsing bugs
- output artifacts not reported after partial success
- Wayland `WINDOW_ID_FAILED` behavior
- stale tool registry handlers returning "tool does not implement execute"
- focus verification failures after app launch

The newer evals require semantic and observable success, not just tool success.

---

## 11. Wayland vs X11 Deep Analysis

### Architectural Comparison

| Capability | X11 | Wayland / XWayland |
| --- | --- | --- |
| Global coordinate input | Possible with xdotool | Restricted by compositor; xdotool only sees XWayland clients |
| Active window query | `xdotool`, `wmctrl` | Often unavailable or partial |
| Focus semantics | Client/window-manager visible | Compositor-controlled |
| Accessibility | AT-SPI available on Linux desktops | AT-SPI often the best semantic path |
| Browser control | CDP works on both | CDP works on both |
| File/process verification | Works | Works |
| Keystroke injection | More reliable | Fragile/restricted unless helper path works |

### Why Wayland Is Harder

Wayland intentionally prevents arbitrary clients from spying on or controlling other clients. That is good security, but it breaks classic automation assumptions:

- no universal window IDs
- no reliable global focus query
- no universal coordinate clicking
- compositor-specific behavior
- Electron/Chrome may run through XWayland or native Wayland depending launch flags

The current architecture prefers non-GUI substrates, CDP, AT-SPI, filesystem, and
process verification; raw input is treated as the least reliable path and used only
where the available substrate requires it.

### Compatibility Matrix

| Runtime Path | X11 | XWayland | Native Wayland |
| --- | --- | --- | --- |
| FileWriteThenOpen | Good | Good | Good |
| TerminalExecution | Good | Good | Good |
| BrowserNavigate/CDP | Good | Good | Good |
| AppOpenOnly | Good | Good | Usually good, focus evidence weaker |
| AT-SPI InteractionHeavy | Good | Good | Good if accessibility bus available |
| xdotool WindowFocused | Good | Partial | Poor |
| Keystroke injection | Good | Partial | Fragile |

---

## 12. Real Root-Cause Failure Analysis

### Failure: "open code and write program..." Did Not Complete Correctly

Root causes represented in the current code/eval history:

```text
Prompt parser consumed conjunction
  -> app target became "code and"
  -> open_application failed or opened wrong target
  -> workflow claimed progress without artifact/output evidence
```

Current mitigation:

- substrate planner resolves `code` aliases via `app_alias_to_binary_pub`.
- regression evals forbid "application 'code and'" and "not found".
- code-generation prompts route to file/terminal substrates.
- run-and-show contracts require output evidence.

### Failure: Daemon Down

```text
Input action requested
  -> GuiBackend cannot connect or heartbeat fails
  -> GLOBAL_SAFETY_HALT / IPC error
  -> InfrastructureFailure classification
  -> safe abort or HITL pause
```

The daemon has heartbeat and emergency release logic, but raw input workflows remain fragile when the service is unavailable. File/terminal/browser substrates reduce dependency on the daemon.

### Failure: Focus Verification

```text
open_application returns
  -> window manager has not focused window yet
  -> WindowFocused checkpoint fails
  -> old behavior retried blindly or aborted
  -> new behavior uses launch grace period, ProcessLaunched alternatives, recovery classification
```

`gui_planner.rs` explicitly moved simple app open/switch verification from `WindowState` toward `ProcessLaunched` because `WindowState` depends on GUI queries that fail on Wayland.

### Failure: Hidden Output

```text
program executed successfully
  -> output captured in tool result or file
  -> final response says "done" without showing output
  -> human goal not satisfied
```

Mitigation lives in:

- `stage_executor.rs`: captures terminal output from `execute_bash`
- `crates/kria-core/tests/workflow_multistep_evals.rs`: requires response/output signals
- GUI eval report cases: artifact and response assertions

### Failure: Event Storms

Event storms can repeatedly invalidate grounding/cache state. KRIA mitigates this with:

- bounded `OperationalFacts`
- cache generation in `GroundingCache`
- delta-only PSDG writes in `EnvironmentStateTracker`
- tests in `environment_grounder.rs`

---

## 13. Production Hardening Roadmap

### Maturity Analysis

| Area | Status | Notes |
| --- | --- | --- |
| Substrate routing | Strong | Key reliability win; keep expanding deterministic coverage. |
| Semantic workflow metadata | Implemented | Frame, fidelity, mode, contract, verifier authority, and hybrid-sync metadata are generated before substrate planning. |
| File/terminal workflows | Strong | Best current production path for non-IDE coding and terminal tasks. |
| Hybrid IDE coding workflow | Implemented | Uses structural file write plus IDE surfacing plus visible terminal attempt and structural fallback. |
| Browser/media governance | Implemented as metadata | Detects private/account/session risk and required HITL/visible verifier metadata; live media verification still needs hardening. |
| Browser CDP cognition | Good | Managed browser navigation/search paths exist; richer media/playback semantic verification is still needed. |
| Verification architecture | Strong foundation | Fail-closed and evidence-oriented; verifier-authority metadata still needs deeper live enforcement. |
| Wayland raw input | Fragile | Avoid unless necessary; continue AT-SPI/CDP migration. |
| HITL | Good foundation | Needs richer frontend lifecycle and resumable approvals. |
| PSDG | Good foundation | Keep bounded; improve contradiction/decay visibility. |
| Recovery | Medium | Classification exists; more real-world recovery actions needed. |
| GUI evals | Strong and improving | Continue live and VM destructive coverage. |

### Priority Matrix

| Priority | Work | Reason |
| --- | --- | --- |
| P0 | Keep semantic contract metadata ahead of substrate planning | Prevents visible/app anchors from being silently discarded. |
| P0 | Enforce verifier-authority/hybrid-sync metadata in live completion gates | Turns metadata into hard success/partial-success decisions. |
| P0 | Keep file/terminal substrate as default for structural coding | Highest reliability, least GUI fragility. |
| P0 | Keep `IdeCodeRunWorkflow` for IDE-anchored run/show prompts | Preserves IDE visibility while retaining structural reliability. |
| P0 | Harden destructive workflow HITL and rollback | Prevent data loss. |
| P1 | Expand AT-SPI action tools | Needed for real app interaction on Wayland. |
| P1 | Improve browser media verification | YouTube/media tasks need playback-state evidence. |
| P1 | Service health UX for daemon/sidecars | Make infrastructure failures actionable. |
| P2 | More session resume flows | Long workflows need better continuation UX. |
| P2 | PSDG introspection UI | Helps debug semantic state. |

### What Should Remain Unchanged

- Keep planner/executor/verifier separated.
- Keep GoalTree immutable.
- Keep verification fail-closed.
- Keep HITL for destructive and unverifiable operations.
- Keep PSDG observational, not authoritative.

### What Should Be Rewritten or Reworked

- Raw xdotool-dependent focus logic should continue being replaced by AT-SPI/CDP/process evidence.
- Interaction-heavy workflows need fewer coordinate assumptions.
- Browser/media tasks need richer live semantic verification beyond page load and metadata.
- Recovery actions should become more concrete, not only classified.

---

## 14. Key Source File Reference Index

| Subsystem | File | Important Functions / Types | Purpose |
| --- | --- | --- | --- |
| Turn routing | `crates/kria-core/src/agent/turn_gate.rs` | `TurnGate`, `IntentEnvelope`, `Operation`, `ResourcePlan` | Top-level intent, hazard, and routing boundary. |
| Intent model | `crates/kria-core/src/agent/intent_compiler.rs` | `GuiTaskSpec`, `Verb`, `TargetRef`, `ContentClass`, `IntentCompiler` | Typed semantic GUI task contract. |
| Rule compiler | `crates/kria-core/src/agent/intent_compiler_rule.rs` | `RuleIntentCompiler::compile` | Deterministic NL-to-`GuiTaskSpec` normalization. |
| GUI planning | `crates/kria-core/src/agent/gui_planner.rs` | `GuiPlanner`, `RuleBasedPlanner::plan` | Planner trait and simple deterministic workflow generation. |
| Semantic workflow | `crates/kria-core/src/agent/semantic_workflow.rs` | `analyze_semantic_workflow`, `SemanticWorkflowFrame`, `WorkflowFidelityResolution` | Extracts deterministic workflow expectation and fidelity metadata. |
| Execution mode | `crates/kria-core/src/agent/execution_mode_reasoner.rs` | `ExecutionModeReasoner::decide`, `ExecutionModeDecision` | Selects structural/visible/hybrid/HITL workflow mode. |
| Workflow contracts | `crates/kria-core/src/agent/workflow_intent_contract.rs` | `WorkflowIntentContractRegistry`, `ContractCheck` | Declarative GUI workflow invariant registry. |
| Verifier authority | `crates/kria-core/src/agent/verifier_authority.rs` | `VerifierAuthorityEvaluator`, `ObservedVerifierEvidence` | Authority and freshness boundaries for verifier evidence. |
| Hybrid sync | `crates/kria-core/src/agent/hybrid_synchronization.rs` | `HybridSynchronizationEvaluator`, checkpoint types | Structural-visible reconciliation metadata. |
| Browser/media governance | `crates/kria-core/src/agent/browser_media_governance.rs` | `BrowserMediaGovernanceEvaluator` | Browser/media HITL and visible-verifier metadata. |
| App registry | `crates/kria-core/src/platform/app_registry.rs` | `InstalledAppRegistry`, alias resolution | Installed-app and class-alias resolution for app names such as code, text editor, Excel/Calc. |
| Substrate planning | `crates/kria-core/src/agent/gui_substrate_planner.rs` | `SubstratePlanner::plan`, `ExecutionSubstrate`, `plan_ide_code_run_workflow`, `plan_terminal_execution`, `plan_browser_search`, `app_alias_to_binary_pub` | Chooses physical execution substrate. |
| GUI coordinator | `crates/kria-core/src/agent/gui_wiring.rs` | `GuiExecutionCoordinator`, `should_route_to_gui_executor`, `generate_workflow`, `execute_workflow`, `execute_goal_tree`, `PolicyToolExecutor` | Wires routing, planning, policy, execution, verification, heartbeat, session persistence. |
| HTN executor | `crates/kria-core/src/agent/htn_executor.rs` | `GuiExecutor::execute_workflow`, `TaskRuntimeState`, `GuiWorkflow`, `SubGoal`, `WorkflowResult` | Executes linear GUI sub-goal workflows with kill switch, target lock, retries, and aborts. |
| GoalTree data | `crates/kria-core/src/agent/goal_tree.rs` | `GoalTree`, `WorkflowStage`, `VerificationCheckpoint`, `RecoveryPath` | Immutable bounded multi-stage workflow representation. |
| Stage executor | `crates/kria-core/src/agent/stage_executor.rs` | `StageExecutor::execute_goal_tree`, `execute_stage`, `StageOutcome`, `GoalTreeResult` | Executes immutable GoalTree workflows with stage checkpoints and recovery budgets. |
| Grounding | `crates/kria-core/src/agent/environment_grounder.rs` | `LiveEnvironmentGrounder::ground`, `OperationalFacts`, `GroundingCache`, `DisplayServerType` | Bounded desktop environment snapshot. |
| Verification model | `crates/kria-core/src/agent/execution_verifier.rs` | `Verifiability`, `VerifyOutcome`, `VerificationConfidenceTier` | Verification contract and evidence classes. |
| Bounded verifier | `crates/kria-core/src/agent/execution_verifier_bounded.rs` | `BoundedExecutionVerifier::verify`, `verify_inner` | Canonical production verifier. |
| Legacy verifier impl | `crates/kria-core/src/agent/execution_verifier_impl.rs` | `check_browser_page_loaded`, `check_file_system_effect`, `check_process_launched` | Older/deprecated implementation still useful for behavior history. |
| GUI tools | `crates/kria-core/src/tools/gui_automation.rs` | `GuiBackend`, `YdotoolBackend`, `IpcRequest`, `KillSwitchInterceptor` | Atomic GUI backend and IPC client. |
| uinput daemon | `crates/kria-uinput-daemon/src/main.rs` | `DaemonRequest`, `handle_request`, `get_active_window_via_atspi`, `execute_emergency_release`, `handle_client` | Isolated helper for OS input and active window info. |
| AT-SPI | `crates/kria-core/src/agent/atspi_engine.rs` | `AtSpiEngine::find_elements`, `detect_dialog`, `dismiss_dialog`, `AccessibleElement` | Semantic accessibility substrate. |
| Browser cognition | `crates/kria-core/src/agent/browser_cognition.rs` | `BrowserCognitionEngine`, `launch_with_debugging`, `get_state`, `BrowserState` | CDP browser automation and state reading. |
| IDE cognition | `crates/kria-core/src/agent/ide_cognition.rs` | `IdeCognitionEngine::get_state`, `check_file`, `IdeState` | Workspace/diagnostic awareness. |
| Workflow continuation | `crates/kria-core/src/agent/workflow_continuation/mod.rs` | `InterruptionClass`, `RecoveryAction`, `WorkflowContinuationRuntime`, `pause_workflow`, `plan_recovery` | Pause/resume/recovery classification. |
| PSDG | `crates/kria-core/src/agent/psdg/mod.rs` | `PsdgHandle`, `record_app_focus`, `record_browser_navigation`, `record_workflow_stage` | Persistent semantic desktop graph. |
| PSDG env tracking | `crates/kria-core/src/agent/psdg/env_tracker.rs` | `EnvironmentStateTracker::track` | Converts grounding snapshots into semantic graph deltas. |
| HITL | `crates/kria-core/src/safety/hitl.rs` | `HitlGateway`, `ApprovalRequest`, `ApprovalResponse` | Approval channel for red/untrusted actions. |
| Policy gate | `crates/kria-core/src/safety/policy_gate.rs` | `CapabilityPolicyGate`, `PolicyDecision`, `CommandCapability` | Capability-based command risk evaluation. |
| Command classifier | `crates/kria-core/src/safety/command_classifier.rs` | `classify`, `CommandClassification` | Raw shell command risk classifier. |
| GUI eval runner | `crates/kria-core/src/bin/test_gui_e2e.rs` | GUI E2E binary | Runs GUI-oriented runtime checks. |
| KRIA test runner | `crates/kria-core/src/bin/kria-test.rs` | test CLI entry | Drives the local KRIA test harness. |
| Test runner core | `crates/kria-core/src/test_runner/mod.rs` | test runner types | Organizes runtime/eval execution. |
| Workflow eval tests | `crates/kria-core/tests/workflow_multistep_evals.rs`, `crates/kria-core/tests/real_world_workflow_evals.rs` | workflow tests | Semantic completion and workflow success checks. |
| Recovery/failure tests | `crates/kria-core/tests/eval_integration_tests.rs`, `crates/kria-core/tests/agent_end_to_end_recovery.rs` | integration tests | Tests bounded failure and recovery behavior. |

---

## Closing Model

KRIA's GUI runtime is best understood as a layered authority system:

```text
Intent authority decides what the user meant.
Planner authority decides a bounded workflow.
Execution authority decides whether each action may run.
Tool authority performs one concrete operation.
Verifier authority decides whether evidence is enough.
Recovery authority decides whether to retry, pause, or abort.
Human authority resolves unsafe, ambiguous, private, or destructive moments.
```

The architecture is strongest where it treats the desktop as a semantic operating environment rather than a bitmap. Its most production-grade path today is semantic-contract metadata plus substrate-aware automation with explicit artifacts and verifiable outcomes. Its most fragile path remains raw GUI input under Wayland-like constraints. The hardening direction is therefore clear: make the metadata hard runtime gates, expand semantic substrates, strengthen live verification, improve recovery, and keep HITL where autonomy should not guess.

---

## Vision Gap Analysis: GUI Cognition vs True Desktop Cognition

The expected GUI vision is "true desktop cognition, not keyboard puppeteering." The
current architecture understands that principle and already contains AT-SPI, CDP,
IDE cognition, semantic workflow contracts, substrate routing, verifiers, HITL, and global halt. The main issue is
coverage and consistency: semantic substrates exist, but raw input/focus paths still
remain fragile and some workflows still depend on whether the right app/window/substrate
was inferred correctly.

### Main GUI Issues

| Issue | Point Of Failure | Why It Blocks The Vision | Implementation Change | Impact |
| ----- | ---------------- | ------------------------ | --------------------- | ------ |
| Raw input is still a fragile fallback | Wayland focus, XWayland window IDs, daemon state, target locks | Makes workflows feel like puppeteering when semantic path is unavailable | Treat raw input as last-resort only after AT-SPI/CDP/files/process checks fail; require explicit verifier checkpoints | Fewer false successes and fewer "typed into wrong place" risks |
| Browser cognition is CDP-based but not universal | CDP requires managed Chrome/debug port | Browser workflows may fall back to GUI unnecessarily | Add browser session manager that can attach/launch/recover CDP reliably and expose tab/page state to PSDG | Browser tasks become semantic and recoverable |
| IDE cognition is partial | VS Code state reading, diagnostics by file, tree-sitter/ruff/rust/js checks | "Fix compile errors" needs LSP-grade project understanding | Add stronger LSP session manager and workspace diagnostic cache; integrate with compiler runs | Coding workflows become operationally intelligent |
| Wayland-native automation is not solved | Synthetic input/focus restrictions | Cross-platform desktop intelligence needs compositor-aware design | Create Wayland-native strategy: accessibility first, portal/ydotool/uinput guarded paths, compositor-specific capability matrix | More predictable Linux desktop support |
| Contract metadata is not yet a hard live gate everywhere | Metadata exists for mode, verifier authority, and hybrid sync, but legacy completion paths can still rely on structural evidence | App opening can be mistaken for full visible workflow success | Make `WorkflowIntentContract`, `VerifierAuthorityAssessment`, and `HybridSynchronizationVerdict` mandatory for final GUI completion status | Fewer hallucinated completions |
| Recovery actions are still broad | Classifies daemon/focus/popup failures, but recovery may be generic | A coworker should propose concrete next actions | Map interruption classes to executable recovery plans with verifier checkpoints | Recovery becomes useful, not just explanatory |

### GUI Data Flow Upgrade

Current semantic path:

```text
Prompt
  -> intent/substrate decision
  -> GUI workflow/tool
  -> focus/input/action
  -> verifier
```

Target cognition path:

```text
Prompt
  |
  v
DesktopSemanticState
  |-- active app/window
  |-- browser URL/title/DOM state
  |-- IDE workspace/file/diagnostics
  |-- filesystem/process artifacts
  |-- accessibility tree/dialogs
  |
  v
SubstrateDecision
  |-- file/API/CDP/LSP first
  |-- AT-SPI second
  |-- OCR/vision third
  |-- raw input last
  |
  v
WorkflowWithCompletionContract
  |
  v
Verified outcome or recoverable blocker
```

### Implementation Priorities

| Priority | Change | Files To Start | Expected Impact |
| -------- | ------ | -------------- | --------------- |
| P0 | Make GUI workflow terminal states explicit | `agent/htn_executor.rs`, `agent/gui_wiring.rs`, `agent/workflow_continuation/mod.rs` | No silent/ambiguous GUI failures |
| P0 | Add stronger focus and target-lock tests | `tools/gui_automation.rs`, `agent/htn_executor.rs`, GUI eval tests | Reduces wrong-window input |
| P1 | Browser session manager for CDP attach/launch/recover | `agent/browser_cognition.rs`, `tools/browser_agent.rs` | Browser workflows become reliable semantic workflows |
| P1 | LSP workspace runtime for IDE cognition | `agent/ide_cognition.rs`, `tools/developer.rs` | Better code/debug/run workflows |
| P1 | Completion contracts per GUI category | `agent/observable_completion/mod.rs`, `agent/workflow_expectation/mod.rs` | Visible success becomes semantic success |
| P2 | Wayland-native capability layer | `platform/*`, `tools/gui_automation.rs`, `agent/atspi_engine.rs` | Clear runtime behavior on Wayland |
| P2 | Dialog/popup recovery catalog | `agent/atspi_engine.rs`, `agent/workflow_continuation/mod.rs` | Real collaborative recovery from interruptions |

### Practical Example: "Open Code, Write Program, Run, Show Output"

Expected robust path:

```text
1. Classify as coding workflow.
2. Prefer file write + shell execution over typing into VS Code.
3. Use IDE only for visibility/context, not as primary execution if file tools work.
4. Run program with captured stdout.
5. Verify output contains Pascal triangle.
6. If VS Code is opened, verify file/tab/workspace separately from command output.
```

Impact:

- Less dependence on fragile focus.
- Faster execution.
- Verifiable output.
- Better user trust.

### Practical Example: "Open YouTube And Play Latest Song From My Playlist"

Expected robust path:

```text
1. Classify as browser + personal account workflow.
2. Use CDP/browser state first.
3. Detect login/private playlist ambiguity.
4. Ask for help only if account/playlist cannot be resolved.
5. Verify page/media state rather than only URL opened.
```

Impact:

- Natural collaboration without unsafe guessing.
- Less blind clicking.
- Better recovery when auth or playlist state is missing.
