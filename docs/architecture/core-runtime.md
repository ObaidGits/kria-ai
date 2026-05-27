# KRIA Core Runtime Architecture

Internal architecture handbook for KRIA's bounded desktop cognition runtime.

Updated from the current KRIA working tree on 2026-05-27. This document describes the live architecture represented in `crates/kria-core`, `crates/kria-desktop`, `crates/kria-eval`, `tests/e2e`, and the existing runtime/eval reports.

## Reader Contract

This handbook is written for mixed readers: someone new to KRIA should understand the
plain-language flow, while an engineer should be able to jump from the explanation to
real source files.

- **Current implementation first:** Sections 1-18 describe the runtime that exists in the
  current codebase unless a limitation is explicitly called out.
- **Future work is separate:** enhancements and long-term direction are kept in
  **Section 19. Future Runtime Roadmap**.
- **Diagrams are explanatory:** they simplify the code path so humans can reason about it;
  they do not replace the referenced source files.
- **Design intent vs current behavior:** "should" language outside the roadmap means a
  safety/design invariant expected by the runtime, not a new future feature.
- **Source truth:** when prose and source disagree, update the prose. The code is the
  operational authority.

---

## 1. Executive Overview

KRIA is a local-first desktop cognition runtime. It is designed to understand a user's operational goal, choose the safest and most reliable execution path, run bounded workflows through policy-governed tools, verify completion with observable evidence, and preserve enough state to recover when the real desktop interrupts.

KRIA is not merely a chatbot with tools. It is also not a macro recorder. Its core problem is harder: translate human intent into real operating-system outcomes while avoiding unbounded autonomy, hallucinated success, unsafe destructive actions, and brittle GUI control.

### What "Desktop Cognition Runtime" Means

```text
User intent
  -> semantic interpretation
  -> bounded workflow plan
  -> grounded desktop awareness
  -> policy/HITL authority
  -> local tool/substrate execution
  -> semantic verification
  -> recovery, memory, transparency
```

A desktop cognition runtime must reason about applications, files, windows, processes, browsers, IDEs, tools, memory, risk, and interruptions as one operational system. KRIA's architecture exists because each of those pieces can fail independently in production.

### High-Level System Overview

```text
+--------------------------------------------------------------------+
| User Interfaces                                                     |
| Tauri desktop, chat UI, voice, tests, API commands                  |
+-------------------------------+------------------------------------+
                                |
+-------------------------------v------------------------------------+
| Turn + Intent Cognition                                             |
| AgentLoop, TurnGate, IntentRouter, IntentCompiler, multi-intent     |
+-------------------------------+------------------------------------+
                                |
+-------------------------------v------------------------------------+
| Planning + Workflow Runtime                                         |
| SubstratePlanner, GuiPlanner, GoalTree, StageExecutor, HTN executor |
+-------------------------------+------------------------------------+
                                |
+-------------------------------v------------------------------------+
| Authority + Safety                                                  |
| Preflight, ExecutionAuthority, PolicyEngine, HITL, audit, halt      |
+-------------------------------+------------------------------------+
                                |
+-------------------------------v------------------------------------+
| Execution Substrates                                                |
| ToolRegistry, shell, files, browser CDP, AT-SPI, GUI daemon, MCP    |
+-------------------------------+------------------------------------+
                                |
+-------------------------------v------------------------------------+
| Verification + Continuity                                           |
| BoundedExecutionVerifier, PSDG, WorkflowContinuation, transparency  |
+--------------------------------------------------------------------+
```

KRIA is local-first because desktop control, private files, browser state, IDE state, and voice interactions are all sensitive. The runtime pushes as much as possible into local deterministic code, local sidecars, bounded local memory, and explicit user confirmation. LLMs can help reason, but they are not the final authority for execution.

---

## 2. Core Architectural Philosophy

### Bounded Cognition

Bounded cognition means every autonomous process has explicit limits:

- finite action budgets
- finite recovery depth
- finite planning outputs
- bounded event queues
- bounded memory injection
- bounded verifier timeouts
- no recursive self-directed planning loops

```text
Unbounded agent loop
  -> "try things until it works"
  -> unsafe, expensive, hard to debug

KRIA bounded loop
  -> plan once
  -> execute finite steps
  -> verify evidence
  -> retry within cap
  -> pause or fail closed
```

This design appears throughout the codebase:

- `goal_tree.rs`: max stages, max actions, max recovery attempts.
- `htn_executor.rs`: action budget and absolute cap.
- `stage_executor.rs`: global action cap and per-stage timeouts.
- `cognition_event_bus/mod.rs`: broadcast capacity and flood guard.
- `psdg/mod.rs`: bounded fact injection.
- `ambient_cognition/mod.rs`: max tick time and no LLM calls.

### Authority Chain

KRIA separates cognition from authority. A model or planner can propose, but other layers decide whether execution may happen and whether it actually worked.

```text
User intent
  -> TurnGate authority: classify operation/risk/route
  -> Planner authority: create bounded workflow
  -> Policy authority: allow, block, or require HITL
  -> Tool authority: perform one concrete action
  -> Verifier authority: decide evidence is sufficient
  -> Recovery authority: retry, pause, escalate, or abort
  -> Human authority: resolve ambiguity/destruction/private state
```

### Why KRIA Avoids Uncontrolled Agent Patterns

| Avoided Pattern | Why It Is Dangerous | KRIA Alternative |
| --- | --- | --- |
| Agent swarms | Hard to audit, competing plans, runaway cost | Single authority chain and typed events |
| Screenshot-only reasoning | Pixels do not prove semantic outcomes | Filesystem, process, CDP, AT-SPI, OCR as fallback |
| Vector-memory chaos | Irrelevant recall can poison context | PSDG and bounded memory reads |
| Recursive autonomy | Infinite loops and unsafe improvisation | GoalTree/HTN with caps |
| Tool success as completion | A command can run without satisfying user intent | Semantic completion contracts |

### Fail-Closed Execution

Fail-closed means KRIA prefers honest failure over fake success. If evidence is absent, low confidence, or unverifiable, the runtime escalates, pauses, or fails. This is especially visible in `execution_verifier.rs`, where `Unverifiable` and `UserAttested` do not silently pass.

---

## 3. Full High-Level Runtime Architecture

### Major Runtime Systems

| System | Primary Files | Responsibility |
| --- | --- | --- |
| AgentLoop | `agent/loop_engine/mod.rs` | Main conversational/tool execution loop, result processing, memory writes, routing handoff. |
| TurnGate | `agent/turn_gate.rs` | Operation, hazard, compute, confidence, tool hints. |
| IntentCompiler | `agent/intent_compiler.rs`, `intent_compiler_rule.rs` | Natural language to typed GUI/task spec. |
| SemanticWorkflowFrame | `agent/semantic_workflow.rs` | Deterministic GUI workflow metadata: task family, app anchors, visibility, ambiguity, safety class, fidelity tier. |
| ExecutionModeReasoner | `agent/execution_mode_reasoner.rs` | Selects structural, visible, hybrid, human-collaborative, verification-visible, or silent workflow mode. |
| WorkflowIntentContractRegistry | `agent/workflow_intent_contract.rs` | Declarative workflow contracts for visible coding, browser, media, human review, silent execution, and general visible workflows. |
| VerifierAuthorityEvaluator | `agent/verifier_authority.rs` | Maps required verifiers to authority/freshness requirements and rejects unsupported evidence claims. |
| HybridSynchronizationEvaluator | `agent/hybrid_synchronization.rs` | Defines and evaluates structural-visible sync checkpoints for hybrid workflows. |
| BrowserMediaGovernanceEvaluator | `agent/browser_media_governance.rs` | Adds browser/media session-risk, visible-verifier, and HITL metadata. |
| GoalTree | `agent/goal_tree.rs`, `stage_executor.rs` | Immutable multi-stage workflow and bounded execution. |
| GuiPlanner | `agent/gui_planner.rs` | Unified GUI workflow planning trait and rule planner. |
| SubstratePlanner | `agent/gui_substrate_planner.rs` | Selects file, terminal, browser, keystroke, app, or interaction substrate. |
| EnvironmentGrounder | `agent/environment_grounder.rs` | Bounded operational facts about windows, processes, monitors, display server. |
| ExecutionVerifier | `agent/execution_verifier.rs`, `execution_verifier_bounded.rs` | Evidence-based verification classes and bounded checks. |
| WorkflowContinuationRuntime | `agent/workflow_continuation/mod.rs` | Interruption classification, pause/resume, recovery plans. |
| CollaborativeAutonomyEngine | `agent/collaborative_autonomy/mod.rs` | Decides proceed, notify, clarify, confirm, pause, retry, or escalate. |
| PSDG | `agent/psdg/*`, `agent/world_model/*` | Persistent semantic desktop graph. |
| Event Runtime | `agent/cognition_event_bus/mod.rs`, `infra/event_bus.rs` | Typed operational events and lower-level runtime events. |
| ToolRegistry | `tools/registry.rs`, `tools/mod.rs` | Tool schemas, handlers, environment context, shell state. |
| HITL | `safety/hitl.rs` | Human approval lifecycle. |
| Safety | `safety/policy.rs`, `policy_gate.rs`, `global_halt.rs`, `audit.rs` | Risk classification, blocking, approval, audit, halt. |
| Memory | `memory/*`, `agent/procedural_memory/*` | Conversation, facts, RAG, preferences, procedural workflow patterns. |
| Browser Cognition | `agent/browser_cognition.rs` | Chrome/CDP browser state and actions. |
| IDE Cognition | `agent/ide_cognition.rs` | VS Code/workspace/diagnostic awareness. |
| GUI Cognition | `tools/gui_automation.rs`, `agent/atspi_engine.rs`, `kria-uinput-daemon` | Desktop control and semantic UI interaction. |
| Service Orchestration | `orchestrator/service_orchestrator.rs` | Vision sidecar/uinput daemon lifecycle, health, restart, global halt. |
| Transparency | `agent/execution_transparency/mod.rs` | Human-visible workflow traces and blockers. |

### Runtime Dependency Map

```text
AgentLoop
  |- TurnGate
  |- ToolRegistry
  |- MemoryRuntime
  |- PolicyEngine / HITL / Audit
  |- GuiExecutionCoordinator
       |- IntentCompiler
       |- SemanticWorkflowFrame / FidelityResolution
       |- ExecutionModeReasoner
       |- WorkflowIntentContractRegistry
       |- VerifierAuthorityEvaluator
       |- BrowserMediaGovernance / HybridSynchronization metadata
       |- SubstratePlanner
       |- EnvironmentGrounder
       |- GuiExecutor / StageExecutor
       |- BoundedExecutionVerifier
       |- WorkflowContinuationRuntime
       |- PSDG / Transparency
```

### Authority Boundaries

| Boundary | Allowed | Not Allowed |
| --- | --- | --- |
| IntentCompiler | Normalize intent | Execute tools, inspect runtime state |
| Planner | Emit workflow | Click/type/read screen |
| Executor | Run approved steps | Invent new stages |
| Verifier | Observe and score evidence | Retry or mutate state |
| PSDG | Persist observed facts | Override safety gates |
| Ambient loop | Emit suggestions | Execute actions or call LLM |
| HITL | Approve/deny | Silently assume consent |

---

## 4. End-to-End Runtime Lifecycle

### General Lifecycle

```text
User prompt
  -> AgentLoop receives turn
  -> TurnGate creates TurnGatePlan
  -> routing chooses conversation, direct tool, GUI, workflow, or fallback
  -> intent/workflow compiler creates typed plan
  -> GUI workflows resolve semantic frame, fidelity, mode, contract, and verifier authority metadata
  -> environment grounding supplies bounded facts
  -> policy and execution authority gate each action
  -> ToolRegistry dispatches concrete handlers
  -> verifier checks observable completion
  -> result synthesizer creates human-readable response
  -> memory/PSDG/session/transparency store runtime state
```

### Example 1: "open code and write a program to print pascal triangle and run it and show output"

```text
Prompt
  -> TurnGate: Automate
  -> RuleIntentCompiler: Open App("code"), Generated("pascal triangle", python)
  -> SemanticWorkflowFrame: Coding + required IDE anchor + WorkflowVisible
  -> ExecutionModeDecision: HybridWorkflow / VisibleCodingWorkflow
  -> ContractCheck: IDE file visible + workflow stage/output surfaced requirements
  -> Multi-intent guard: substrate can handle it
  -> SubstratePlanner: IdeCodeRunWorkflow
  -> Tools: write_file source -> write_file runner -> open_application_with_file -> execute_bash terminal launcher
  -> Verifier: FileSystemEffect + ProcessLaunched + DeterministicOutput
  -> Result: source file, runner, captured output, and fallback disclosure when visible terminal launch is unavailable
```

State transitions:

```text
Idle
  -> Planning
  -> Executing(write_file)
  -> Executing(write_runner)
  -> OpeningEditor
  -> Executing(visible_terminal_launcher_or_structural_fallback)
  -> Verifying(output)
  -> CompletedWithEvidence
```

Decision points:

- If generated code is detected, KRIA avoids keystroke typing.
- If "open code" plus "run/show output" is detected, the mode decision is hybrid visible coding and the substrate planner uses `IdeCodeRunWorkflow`.
- If the visible terminal launcher cannot produce output, the runner falls back structurally and appends an explicit fallback marker to the output artifact.
- If execution output is absent, semantic completion fails.

### Example 2: "open youtube and play latest song from my playlist"

```text
Prompt
  -> TurnGate: browser/automation intent
  -> Intent compiler: YouTube target, play action, private playlist scope
  -> SemanticWorkflowFrame: Media/Browser + account/private context ambiguity
  -> BrowserMediaGovernance: HITL pause required for private/session-dependent playlist state
  -> SubstratePlanner: BrowserNavigate / InteractionHeavy
  -> BrowserCognitionEngine: launch/reuse Chrome with CDP
  -> Verification metadata: BrowserPageVisible, BrowserAccountContext, MediaPlaybackVisible
  -> If login/private playlist needed: HITL/AuthRequired
  -> If "latest" cannot be resolved: clarify or user attestation
```

Recovery branches:

```text
YouTube loads?
  no -> BrowserPageLoaded fail -> retry/abort with evidence
  yes
    |
    +-- playlist visible?
          no -> AuthRequired/HITL
          yes
             |
             +-- latest song identifiable?
                   no -> clarify
                   yes -> play and verify page/media state
```

### Example 3: "fix compile errors in current Rust project"

This is a project cognition workflow, not a single command.

```text
Prompt
  -> TurnGate: ExecuteCode / ComplexTask / coding
  -> EnvironmentGrounder + IDE cognition:
       active project, workspace root, diagnostics if available
  -> Planner:
       inspect errors
       modify files
       run cargo check/test
       verify no compile errors
  -> Policy:
       file edits are Yellow
       command execution can be Red depending action
  -> Verification:
       deterministic cargo output
       file effects
       IDE diagnostics
  -> Recovery:
       if build still fails, classify and continue bounded attempts or ask user
```

Operational flow:

```text
Sense project
  -> run/collect diagnostics
  -> choose bounded edit target
  -> write patch/file
  -> cargo check
  -> verifier evaluates output
  -> success, retry within cap, or HITL
```

Why it is not just `cargo fix`: the user asked to fix the project, not run one tool. KRIA must preserve context, avoid destructive edits, verify compile output, and stop when confidence is too low.

### Example 4: "delete all files in Downloads"

```text
Prompt
  -> TurnGate: Delete, high hazard
  -> target resolution: ~/Downloads
  -> preflight: destructive path validation
  -> policy: Red / requires approval
  -> HITL: explicit confirmation
  -> if approved: bounded file operation
  -> verifier: directory empty / expected files removed
  -> audit log records decision
```

State machine:

```text
Idle -> ClassifiedDestructive -> AwaitingHITL
  -> Denied/Timeout -> AbortedNoAction
  -> Approved -> Executing -> Verifying -> CompletedWithAudit
```

---

## 5. Intent Understanding + Operational Cognition

KRIA uses layered intent understanding:

```text
IntentRouter regex/direct hints
  -> TurnGate operation/hazard/compute
  -> RuleIntentCompiler typed spec
  -> MultiIntentDecomposer for compound prompts
  -> OpGraph / GoalTree compiler when needed
  -> SubstratePlanner chooses execution surface
```

Semantic cognition matters because natural language describes outcomes, not tools. "Show output" means the user must see actual output, not just that a shell command exited. "Open code and write..." means create useful code, not necessarily type into the foreground editor. "My playlist" means private account state, not a public web search.

### Operational Cognition Contract

Operational cognition converts complex user requests into bounded executable workflow structure while preserving runtime authority.

Core invariants:
- PSDG remains the semantic cognition authority.
- GoalTree and StageExecutor remain the execution authority.
- OpGraph compiles into GoalTree.
- No parallel planners or recursive autonomy.
- Verification and HITL gates remain mandatory.

The decomposition stack is intentionally bounded:
1. Rule system for deterministic clause segmentation and intent tagging.
2. Workflow templates for known patterns such as fix, test, commit, recover.
3. Typed intent ontology for coding, debugging, browser, deployment, filesystem, devops, VM/container, communication, research, recovery, and system operations.
4. Optional schema-constrained LLM assistance with capped intent count and no execution authority.

Decomposition outputs an OpGraph with typed intent nodes and explicit dependencies. Execution remains routed through GoalTree and StageExecutor only. Ambiguity does not trigger execution, decomposition never invokes tools or environment queries, and all multi-intent graphs must compile through the deterministic OpGraph to GoalTree compiler.

OpGraph is planning-only, bounded, deterministic, immutable once frozen, and auditable. It never runs tools directly.

### Planning Decision Tree

```text
Does prompt map to direct safe tool?
  yes -> ToolRegistry path
  no
    |
    +-- GUI/desktop goal?
          yes -> GuiExecutionCoordinator
          no
            |
            +-- complex coding/research/workflow?
                  yes -> AgentLoop multi-step tool loop
                  no -> conversational response
```

### Ambiguity Handling

`IntentCompiler` can return `ClarifyRequest`. `CollaborativeAutonomyEngine` can also return `Clarify` when target, scope, or confidence is insufficient. KRIA's philosophy is that ambiguity should stop execution, not be hidden inside guesses.

---

## 6. GoalTree + Planning Architecture

GoalTree is KRIA's immutable multi-stage workflow structure.

```text
GoalTree
  |- preconditions
  |- stages[]
      |- action_group[]
      |- checkpoint
      |- recovery path
      |- context hints
  |- completion contract
  |- global abort
```

### Planning Lifecycle

```text
User text
  -> MultiVerbSpec / OpGraph
  -> RuleBasedWorkflowCompiler or GoalTreeOpGraphCompiler
  -> GoalTree::validate()
  -> StageExecutor::execute_goal_tree(&GoalTree)
```

### Execution Semantics

- Stages run sequentially.
- Actions inside a stage run sequentially.
- Checkpoints are verified after action groups.
- Recovery is capped by `MAX_RECOVERY_ATTEMPTS`.
- The tree is never mutated by execution.
- The executor cannot call planners.

### Retry and Continuation

```text
Stage action succeeds
  -> checkpoint passes -> next stage
  -> checkpoint fails
       -> retry/corrective/skip if recovery allows
       -> classify interruption if still failed
       -> pause or abort
```

This is why GoalTree is safer than a free-form ReAct loop for desktop operations.

---

## 7. Environment Grounding + Runtime Awareness

Environment grounding answers: "What does the operating environment look like right now?"

`environment_grounder.rs` returns `OperationalFacts`:

- display server type
- focused app/window
- visible windows
- terminal cwd
- monitors
- process facts
- grounding capabilities

```text
Grounding request targets
  -> capability probe
  -> xdotool/wmctrl/proc queries where available
  -> bounded facts
  -> optional PSDG env tracker delta write
```

KRIA avoids constant heavy vision loops because they are expensive, fragile, and easy to over-trust. Instead:

- environment grounding is short-lived and bounded
- browser state comes from CDP
- IDE state comes from local state/diagnostics
- desktop events flow through typed event buses
- OCR/vision are fallbacks, not the main awareness layer

### Operational Awareness Flow

```text
OS/application signal
  -> Perception/Cognition event
  -> DesktopAwarenessRuntime snapshot
  -> PSDG fact
  -> Turn context / recovery / suggestions
```

---

## 8. GUI Cognition Integration

GUI cognition is integrated as one substrate family inside the core runtime, not as a separate uncontrolled agent.

```text
GuiExecutionCoordinator
  |- SubstratePlanner
  |- EnvironmentGrounder
  |- GuiExecutor / StageExecutor
  |- BoundedExecutionVerifier
  |- ToolRegistry
       |- GUI tools
       |- browser CDP tools
       |- AT-SPI tools
       |- file/shell tools
```

### GUI Subsystems

| Subsystem | File | Role |
| --- | --- | --- |
| GUI backend | `tools/gui_automation.rs` | IPC client and `GuiBackend` trait. |
| uinput daemon | `crates/kria-uinput-daemon/src/main.rs` | Isolated helper for input/window commands. |
| AT-SPI | `agent/atspi_engine.rs` | Semantic UI element discovery and dialog handling. |
| Browser CDP | `agent/browser_cognition.rs` | Browser state and operations. |
| IDE cognition | `agent/ide_cognition.rs` | Workspace/diagnostic state. |
| GUI wiring | `agent/gui_wiring.rs` | Core integration point. |

### Why GUI Cognition Is Difficult

Desktop automation fails for mundane reasons:

- focus changes between planning and typing
- Wayland blocks global window control
- dialogs steal input
- browser login state is private
- app launch is asynchronous
- visible UI does not prove semantic success

KRIA's answer is substrate-first execution: avoid raw GUI interaction unless the task truly requires it.

---

## 9. Execution Verification System

Execution verification is a separate authority. It observes state and returns evidence.

```text
Action result
  -> VerificationType
  -> Verifiability
  -> BoundedExecutionVerifier
  -> VerifyOutcome { verified, confidence, tier, evidence, latency }
```

### Confidence Model

| Tier | Meaning | Example |
| --- | --- | --- |
| FullSemantic | Strong semantic state evidence | CDP page loaded, AT-SPI element visible |
| PartialObservable | Concrete but partial evidence | file exists, process running |
| StructuralOnly | Weak structural observation | OCR text, process fallback |
| Unobservable | Cannot verify | user attestation needed |

Visible success is not semantic success. A window can be open while the file is missing; a command can exit while output is hidden; a browser can launch but never load the target page. KRIA verifies the operational state the user actually asked for.

---

## Result Synthesis

Result synthesis is the boundary between raw execution output and user-facing response text. Tool output must be normalized before it is shown to the user or injected back into model context.

```text
ToolResult
  -> ResultSynthesizer
  -> SynthesizedResult {
       conversational_summary,
       raw_output,
       execution_metadata,
       outcome_class,
       evidence
     }
  -> UI / AgentLoop / audit
```

Primary implementation:
- `crates/kria-core/src/agent/result_synthesizer.rs`
- `crates/kria-core/src/agent/loop_engine/mod.rs`
- `crates/kria-desktop/src/commands/chat.rs`
- `ui/src/components/MessageBubble.tsx`

Synthesis rules:
- Preserve raw output availability for transparency and debugging.
- Summarize common tool results into useful user-facing language.
- Classify outcomes as success, partial, failure, empty result, or truncated output.
- Attach metadata such as command, exit code, duration, artifact paths, and verification evidence.
- Keep synthesis deterministic by default; LLM-powered synthesis is an optional future path and must not invent facts.

Failure rules:
- If synthesis is too generic, improve tool-specific formatting rather than hiding raw output.
- If raw output is not visible, check UI rendering and metadata plumbing.
- If synthesis adds latency, keep it outside critical execution gating.

---

## 10. Recovery + Workflow Continuation

Workflow continuation lets KRIA pause safely rather than improvising.

```text
Failure/interruption
  -> InterruptionContext
  -> classify_interruption()
  -> plan_recovery()
  -> retry, skip, rollback, escalate, request human, abort
  -> optional pause checkpoint
```

### Interruption Taxonomy

| Class | Typical Cause | Response |
| --- | --- | --- |
| Popup/AuthRequired | Login, sudo, OAuth | HITL or pause |
| FocusTheft | User/app changed focus | pause/retry depending action |
| BrowserStateChanged | navigation/reload | re-ground browser or pause |
| WindowFocusFailed | launch/focus race | bounded retry |
| InfrastructureFailure | daemon/sidecar halt | service recovery or HITL |
| Timeout | stage exceeded budget | recovery or abort |
| ResourceExhausted | disk/OOM | escalate |

Workflow sessions are persisted through `workflow_session.rs`; continuation checkpoints are managed by `WorkflowContinuationRuntime`.

---

## 11. PSDG + Memory Architecture

KRIA has several memory layers:

```text
Conversation Memory
  -> MemoryStore, MemoryManager, facts, preferences, RAG

PSDG
  -> desktop/browser/IDE/workflow semantic graph

Operational Context
  -> current workflow chain, interruption/recovery lineage

Procedural Memory
  -> distilled workflow skill patterns from completed sessions
```

Persistent memory services store conversation history, facts, links, preferences, retrieval artifacts, snippets, chunks, and media. Memory provides typed read/write interfaces to orchestration and supports bounded relevance retrieval through indexing and FTS paths.

Memory authority boundaries:
- `MemoryManager` and `MemoryReader` define the runtime contracts.
- `MemoryStore` implements SQLite-backed persistence and retrieval.
- Orchestrator decides what memory is read or written for a turn.
- Memory store executes persistence/retrieval but never plans or executes actions.
- Retrieval is bounded by token/context budgets.
- Persistent state changes must be attributable to turn/runtime events.

Memory failure behavior:
- Store unavailable or corrupt: fail explicit memory operations and continue in degraded mode when possible.
- Query failure: return structured retrieval error and continue with reduced context.
- Write failure: preserve turn execution, mark memory persistence failure for diagnostics.
- Over-budget context: prioritize and trim deterministic memory slices.

### PSDG Architecture

```text
Grounder / Browser / IDE / StageExecutor
  -> PsdgHandle fire-and-forget writes
  -> WorldModelStore
  -> bounded queries / context injection / continuation
```

KRIA uses graph semantics because desktop state is relational: app focus, workflow stage, browser URL, IDE workspace, and interruption lineage are facts about entities. Vector retrieval is still useful for documents and semantic recall, but it should not be the sole operational memory authority.

### Procedural Memory

`procedural_memory/mod.rs` extracts workflow skills from completed sessions:

- bounded skills per category
- bounded tool sequence
- success rate
- average step count
- no LLM
- never auto-triggers workflows

This gives KRIA "what worked before" memory without uncontrolled self-modification.

---

## 12. Event Runtime + Ambient Cognition

KRIA has multiple event layers:

| Layer | File | Purpose |
| --- | --- | --- |
| Infra EventBus | `infra/event_bus.rs` | General runtime events: tools, sidecars, voice, VRAM, LLM swaps. |
| Automation EventBus | `automation/event_bus.rs` | Topic-based automation events. |
| CognitionEventBus | `agent/cognition_event_bus/mod.rs` | Typed workflow/browser/IDE/desktop/policy/suggestion events. |

### Cognition Event Flow

```text
Runtime signal
  -> CognitionEventBus emit()
  -> flood guard deduplicates
  -> broadcast channel cap 256
  -> subscribers:
       AmbientCognitionLoop
       DesktopAwarenessRuntime
       OperationalContextTracker
       UI/transparency consumers
```

### Ambient Cognition

`ambient_cognition/mod.rs` is intentionally humble:

- no LLM calls
- no vision calls
- read-only checks
- minimum tick interval
- max tick budget
- emits suggestions only

This avoids polling-heavy cognition while still allowing KRIA to notice resumable sessions or build failures.

---

## 13. Human-in-the-Loop (HITL)

HITL is not a bolt-on safety prompt. It is part of the cognition model. KRIA treats the human as the final authority for ambiguity, private state, destructive operations, and low-confidence irreversible decisions.

```text
Risk or ambiguity detected
  -> Confirm / Clarify / Escalate
  -> HitlGateway ApprovalRequest
  -> UI/voice/API response
  -> Approved, Denied, or Timeout
  -> audit log
```

HITL balances autonomy with control:

- routine read-only tasks can proceed
- novel tasks can proceed with notice
- destructive tasks require explicit confirmation
- private state requires user participation
- timeout means deny

---

## 14. Safety + Boundedness Engineering

Safety is distributed across several hard boundaries.

```text
Preflight
  -> ExecutionAuthority
  -> PolicyEngine / CapabilityPolicyGate
  -> HITL
  -> GlobalSafetyHalt
  -> Tool sandbox/isolation
  -> Verifier
  -> Audit log
```

### Safety Layers

| Layer | File | Protection |
| --- | --- | --- |
| Preflight | `tools/preflight.rs` | Blocks malformed or obviously dangerous parameters before execution. |
| PolicyEngine | `safety/policy.rs` | Green/Yellow/Red/Black tool risk tiers. |
| CapabilityPolicyGate | `safety/policy_gate.rs` | Capability-based shell command governance. |
| Command classifier | `safety/command_classifier.rs` | Raw shell command tiering. |
| HITL | `safety/hitl.rs` | Human approval with timeout deny. |
| Global halt | `safety/global_halt.rs` | Process-wide automation kill switch. |
| Audit | `safety/audit.rs` | SQLite hash-chained decisions. |
| Rollback | `safety/rollback.rs` | Snapshot/restore support for selected changes. |

Dangerous workflows should run in VM or controlled environments when possible. `tools/mod.rs` includes VM/Docker dispatch support for destructive test paths.

---

## 15. Eval + Testing Architecture

KRIA's evaluation architecture now tests operational cognition, not just declarations.

```text
Unit tests
  -> parser, planner, verifier, policy

Integration evals
  -> GoalTree, fault injection, recovery

Workflow evals
  -> semantic completion contracts

GUI evals
  -> real apps/substrates/artifacts/display-server behavior

VM destructive evals
  -> deletion, process kill, rm paths in safe target

Stress harnesses
  -> live stress, collision, event storm, service failures
```

Key files:

- `crates/kria-core/src/bin/kria-test.rs`
- `crates/kria-core/src/bin/test_gui_e2e.rs`
- `crates/kria-core/src/test_runner/mod.rs`
- `crates/kria-core/tests/workflow_multistep_evals.rs`
- `crates/kria-core/tests/real_world_workflow_evals.rs`
- generated GUI eval reports under `tests-logs/` when evals are run

Old evals missed production failures because they accepted tool success as user success. The newer architecture checks artifacts, output visibility, substrate use, retrieval leakage, false success patterns, Wayland behavior, and recovery semantics.

---

## 16. Production Failures + Root-Cause Analysis

### VS Code Workflow Failures

```text
Prompt: open code and write...
  -> parser consumed conjunction
  -> app target "code and"
  -> app open failed or wrong route
  -> no real artifact/output
  -> old response could still imply completion
```

Architectural fix:

- alias handling in `gui_substrate_planner.rs`
- file/terminal substrate for generated code
- eval cases forbid `code and`
- semantic output contracts

### Daemon-Down Failures

```text
Input action
  -> uinput daemon unavailable
  -> heartbeat/IPC failure
  -> global halt or infrastructure error
  -> workflow abort/pause
```

Assumption that failed: raw input service would always be available. Current mitigation is service orchestration, restart backoff, global halt, and reducing raw input dependency.

### Wayland Focus Failures

```text
open_application
  -> app process starts
  -> xdotool/window ID unavailable
  -> WindowFocused fails
  -> workflow falsely appears broken
```

Mitigation:

- prefer `ProcessLaunched` for app-open checks
- use AT-SPI/CDP where semantic
- substrate routes avoid focus-sensitive typing

### Hidden Output Failures

```text
execute_bash success
  -> output captured internally
  -> response says "done"
  -> user did not see requested output
```

Mitigation:

- `stage_executor.rs` captures terminal output
- workflow eval contracts require visible output
- GUI evals check output artifacts and response patterns

### Event Storms

Assumption that failed: every event could invalidate or persist state freely. Mitigation includes flood guards, broadcast caps, delta-only PSDG writes, and bounded grounder invalidation tests.

---

## 17. Cross-Platform Runtime Strategy

### Current Linux Strategy

| Capability | Preferred Runtime |
| --- | --- |
| File operations | Native filesystem tools |
| Code execution | Shell/subprocess with policy |
| Browser | Chrome CDP |
| Semantic UI | AT-SPI |
| Raw input | uinput daemon / xdotool fallback |
| Window facts | grounder via xdotool/wmctrl/proc/AT-SPI |

### Wayland vs X11

| Area | X11 | Wayland/XWayland |
| --- | --- | --- |
| Window IDs | Reliable | Partial/unreliable |
| Global input | More available | Restricted |
| Accessibility | Available | Primary semantic path |
| Browser CDP | Works | Works |
| File/process checks | Works | Works |
| Screenshot automation | Possible | Compositor-dependent |

### Non-Linux Portability Boundary

The current production-oriented implementation is Linux-heavy. Windows/macOS support is
not described here as current parity. The portability boundary to preserve is:

- `GuiBackend` becomes OS-specific input backend.
- `EnvironmentGrounder` gets platform implementations.
- Browser CDP stays portable.
- IDE/file/process verification stays mostly portable.
- Policy, HITL, GoalTree, PSDG, ToolRegistry remain core.

Concrete Windows/macOS expansion is roadmap material in Section 19.

---

## 18. Current Architecture Maturity Assessment

| Subsystem | Maturity | Assessment |
| --- | --- | --- |
| ToolRegistry | Production-grade | Clear schema/handler boundary and context support. |
| Policy/HITL/Audit | Strong | Conservative, auditable, timeout-deny. |
| File/terminal substrate | Strong | Best path for coding workflows. |
| GoalTree/StageExecutor | Strong foundation | Bounded and auditable; needs more real recovery actions. |
| Bounded verifier | Strong | Correct authority separation. |
| GUI raw input | Fragile | Platform and daemon dependent. |
| Browser cognition | Good | Needs richer media/playback state. |
| IDE cognition | Experimental-good | Useful but VS Code/state heuristics need hardening. |
| PSDG | Good foundation | Needs more introspection and decay observability. |
| Ambient cognition | Safe foundation | Intentionally limited; good direction. |
| Procedural memory | Experimental | Bounded and safe, but needs more product integration. |
| Service orchestrator | Important but risky | Process supervision is hard; failure UX matters. |

Highest-risk systems:

- raw GUI input on Wayland
- destructive filesystem/shell actions
- browser account/private-state workflows
- stale tool registry or production/eval divergence
- hidden output and false success regressions

---

## 19. Future Runtime Roadmap

```text
Near term
  -> harden service health UX
  -> expand AT-SPI semantic actions
  -> improve browser media verification
  -> richer HITL resume flows
  -> wire verifier-authority and hybrid-sync metadata into hard live completion gates

Mid term
  -> stronger IDE diagnostics loop
  -> procedural workflow recall in planning
  -> remote desktop cognition with same safety model
  -> better operational transparency UI

Long term
  -> distributed runtime
  -> multimodal cognition with strict evidence wrappers
  -> stronger local voice runtime
  -> long-horizon workflows with durable state
```

What should remain stable:

- bounded cognition
- fail-closed verification
- policy/HITL authority
- immutable GoalTree execution
- local-first memory and state
- semantic-contract-before-substrate GUI design
- substrate execution remains bounded and explicit

What should evolve:

- platform-specific grounders
- semantic browser/media state
- live consumption of workflow contracts, verifier authority, and hybrid sync verdicts
- procedural memory use in planning
- recovery action quality
- observability and user-facing diagnostics

---

## 20. Source File Reference Index

| Subsystem | File | Key Functions / Types | Purpose |
| --- | --- | --- | --- |
| Core exports | `crates/kria-core/src/lib.rs` | module declarations | Top-level crate architecture. |
| Agent exports | `crates/kria-core/src/agent/mod.rs` | module declarations, public exports | Defines cognition runtime modules. |
| Main loop | `agent/loop_engine/mod.rs` | `AgentLoop`, tool failure classification, turn processing | Conversational and tool orchestration hot path. |
| Intent router | `agent/router.rs` | `IntentRouter`, `IntentResult`, `Intent` | Direct/complex/conversation routing. |
| Turn gate | `agent/turn_gate.rs` | `TurnGate`, `IntentEnvelope`, `ResourcePlan` | Operation, hazard, confidence, compute routing. |
| Intent compiler | `agent/intent_compiler.rs` | `GuiTaskSpec`, `IntentCompiler` | Typed intent contract. |
| Rule compiler | `agent/intent_compiler_rule.rs` | `RuleIntentCompiler::compile` | Deterministic prompt normalization. |
| Semantic workflow | `agent/semantic_workflow.rs` | `analyze_semantic_workflow`, `SemanticWorkflowFrame`, `WorkflowFidelityResolution` | GUI workflow expectation and fidelity metadata. |
| Execution mode | `agent/execution_mode_reasoner.rs` | `ExecutionModeReasoner::decide`, `ExecutionModeDecision` | Deterministic visible/structural/hybrid mode selection. |
| Workflow contracts | `agent/workflow_intent_contract.rs` | `WorkflowIntentContractRegistry::evaluate`, `WorkflowIntentContract` | Declarative workflow invariants and verifier requirements. |
| Verifier authority | `agent/verifier_authority.rs` | `VerifierAuthorityEvaluator`, `VerifierAuthorityRequirement` | Authority/freshness boundaries for visible and structural evidence. |
| Hybrid sync | `agent/hybrid_synchronization.rs` | `HybridSynchronizationEvaluator`, checkpoints | Structural-visible divergence metadata for hybrid workflows. |
| Browser/media governance | `agent/browser_media_governance.rs` | `BrowserMediaGovernanceEvaluator` | Session/private-state governance for browser and media workflows. |
| Multi-intent | `agent/multi_intent.rs` | `RuleBasedMultiIntentDecomposer` | Compound prompt decomposition. |
| OpGraph | `agent/opgraph.rs`, `opgraph_compiler.rs` | `GoalTreeOpGraphCompiler` | Intent graph to GoalTree. |
| Workflow compiler | `agent/workflow_compiler.rs` | `RuleBasedWorkflowCompiler`, `MultiVerbSpec` | Multi-stage workflow compilation. |
| GoalTree | `agent/goal_tree.rs` | `GoalTree`, `WorkflowStage`, `VerificationCheckpoint` | Immutable workflow model. |
| Stage executor | `agent/stage_executor.rs` | `StageExecutor::execute_goal_tree` | Bounded GoalTree execution. |
| GUI wiring | `agent/gui_wiring.rs` | `GuiExecutionCoordinator`, `PolicyToolExecutor` | GUI/core integration and policy-wrapped execution. |
| Substrate planner | `agent/gui_substrate_planner.rs` | `SubstratePlanner::plan`, `ExecutionSubstrate` | Physical execution substrate selection. |
| HTN executor | `agent/htn_executor.rs` | `GuiExecutor::execute_workflow`, `TaskRuntimeState` | Bounded sub-goal GUI workflow executor. |
| Grounder | `agent/environment_grounder.rs` | `LiveEnvironmentGrounder::ground`, `OperationalFacts` | Runtime desktop facts. |
| Verifier model | `agent/execution_verifier.rs` | `Verifiability`, `VerifyOutcome` | Verification contract. |
| Bounded verifier | `agent/execution_verifier_bounded.rs` | `BoundedExecutionVerifier::verify` | Production verification authority. |
| Continuation | `agent/workflow_continuation/mod.rs` | `WorkflowContinuationRuntime`, `InterruptionClass` | Pause/resume/recovery classification. |
| Collaborative autonomy | `agent/collaborative_autonomy/mod.rs` | `CollaborativeAutonomyEngine`, `AutonomyDecision` | Proceed/clarify/confirm/retry/escalate policy. |
| Event bus | `agent/cognition_event_bus/mod.rs` | `CognitionEventBus`, `CognitionEvent` | Typed operational cognition events. |
| Ambient cognition | `agent/ambient_cognition/mod.rs` | `AmbientCognitionLoop::run_tick` | Bounded background suggestions. |
| Desktop awareness | `agent/desktop_awareness/mod.rs` | `DesktopAwarenessRuntime::apply_event` | Live operational snapshot. |
| Operational context | `agent/operational_context/mod.rs` | `OperationalContextTracker` | Workflow/interruption lineage. |
| Suggestions | `agent/operational_suggestions/mod.rs` | `OperationalSuggestionsEngine` | Rate-limited proactive suggestions. |
| PSDG | `agent/psdg/mod.rs` | `PsdgHandle`, `record_*` | Persistent semantic desktop graph. |
| PSDG env tracker | `agent/psdg/env_tracker.rs` | `EnvironmentStateTracker::track` | Grounding-to-graph deltas. |
| Browser cognition | `agent/browser_cognition.rs` | `BrowserCognitionEngine`, `BrowserState` | CDP browser state/action substrate. |
| IDE cognition | `agent/ide_cognition.rs` | `IdeCognitionEngine`, `IdeState` | Workspace/diagnostic awareness. |
| AT-SPI | `agent/atspi_engine.rs` | `AtSpiEngine::find_elements` | Semantic GUI element access. |
| GUI tools | `tools/gui_automation.rs` | `GuiBackend`, `YdotoolBackend`, `KillSwitchInterceptor` | GUI IPC/backend contract. |
| uinput daemon | `crates/kria-uinput-daemon/src/main.rs` | `DaemonRequest`, `handle_request`, `handle_client` | Isolated input helper. |
| Tool registry | `tools/registry.rs` | `ToolRegistry`, `ToolHandler`, `build_default_registry` | Tool schemas and handlers. |
| Tool context | `tools/mod.rs` | `ToolContext`, `vm_dispatch_command` | Environment/cancellation/shell state for tools. |
| Preflight | `tools/preflight.rs` | `run_preflight`, `preflight_shell`, `preflight_file_op` | Deterministic pre-execution validation. |
| Policy | `safety/policy.rs` | `PolicyEngine`, `RiskLevel`, `PolicyDecision` | Tool risk tiers. |
| Policy gate | `safety/policy_gate.rs` | `CapabilityPolicyGate`, `CommandCapability` | Capability-based command safety. |
| Command classifier | `safety/command_classifier.rs` | `classify` | Raw shell risk classification. |
| HITL | `safety/hitl.rs` | `HitlGateway`, `ApprovalRequest` | Human approval runtime. |
| Global halt | `safety/global_halt.rs` | `engage_halt`, `release_halt`, `is_halted` | Master automation kill switch. |
| Audit | `safety/audit.rs` | `AuditLogger::log` | Hash-chained decision log. |
| Memory manager | `memory/manager.rs` | `MemoryManager`, `MemoryReader`, `MemoryRuntime` | Runtime memory boundary. |
| Memory store | `memory/store.rs` | `MemoryStore` | SQLite conversation/fact/document storage. |
| Procedural memory | `agent/procedural_memory/mod.rs` | `ProceduralWorkflowMemory::ingest_session` | Bounded workflow skill extraction. |
| Result synthesis | `agent/result_synthesizer.rs` | `ResultSynthesizer`, `SynthesizedResult` | Human-readable tool result rendering. |
| Service orchestration | `orchestrator/service_orchestrator.rs` | `ServiceOrchestrator::start`, `run_health_check` | Sidecar/daemon lifecycle and health. |
| Infra events | `infra/event_bus.rs` | `EventBus`, `KriaEvent` | General runtime event stream. |
| Execution trace | `infra/execution_trace.rs` | `ExecutionTrace`, `TraceNode` | Per-turn causal tool trace. |
| GUI eval runner | `crates/kria-core/src/bin/test_gui_e2e.rs` | GUI E2E binary | Runs GUI-oriented runtime checks. |
| KRIA test runner | `crates/kria-core/src/bin/kria-test.rs` | test CLI entry | Drives the local KRIA test harness. |
| Test runner core | `crates/kria-core/src/test_runner/mod.rs` | test runner types | Organizes runtime/eval execution. |
| Workflow evals | `crates/kria-core/tests/workflow_multistep_evals.rs`, `crates/kria-core/tests/real_world_workflow_evals.rs` | workflow tests | Observable workflow success definitions. |
| Integration evals | `crates/kria-core/tests/eval_integration_tests.rs`, `crates/kria-core/tests/agent_end_to_end_recovery.rs` | integration tests | Recovery/failure behavior tests. |

---

## Closing Model

KRIA's core runtime is a deliberately bounded operational system. It is strongest when it treats user prompts as requests for verified outcomes, not as permission for open-ended tool use. The architecture's central bet is that useful desktop autonomy requires less improvisation, not more: typed intent, explicit workflows, grounded state, policy authority, verifiable evidence, recoverable sessions, and a human collaborator at the boundaries where autonomy should not guess.

---

## Vision Gap Analysis: Core Runtime vs Operational Coworker Goal

KRIA's core architecture already points in the right direction: bounded cognition,
policy authority, verifiers, workflow continuation, PSDG, and local-first execution.
The gap is that these systems are not yet consistently unified into one operational
runtime contract. Some workflows still look like independent subsystems passing text and
tool results around rather than one coherent coworker reasoning over a shared state.

### Core Runtime Issues

| Issue | Failure Mode | Why It Matters | Implementation Change | Impact |
| ----- | ------------ | -------------- | --------------------- | ------ |
| Too many orchestration paths | Similar tasks can travel through router, fallback, GUI coordinator, direct tool match, or ReAct loop differently | Makes behavior hard to predict and debug | Define a single `TurnFrame -> Plan -> Execute -> Verify -> Synthesize` contract and make special paths plug into it | More maintainable and easier to reason about |
| GoalTree/OpGraph not always the main execution contract | Multi-step workflows can degrade into model-driven tool loops | Long tasks become brittle | Use GoalTree/OpGraph as the default for multi-step operational prompts | Better workflow continuity and recovery |
| State is distributed | PSDG, TurnMemory, WorkflowSession, execution transparency, and tool messages are separate | Hard to know "what KRIA knows" | Introduce a runtime state ledger linking turn ID, workflow ID, goal ID, tool calls, evidence, verifier results | Better debugging, resuming, and user explanations |
| Ambient cognition is intentionally limited | The assistant does not yet feel like a proactive coworker | Long-term JARVIS-like behavior needs context awareness | Keep ambient cognition advisory-only, but feed it from stable PSDG/event summaries | Safe proactive suggestions without uncontrolled autonomy |
| Cross-platform goal exceeds current Linux-heavy implementation | Wayland/X11/Linux paths dominate | Vision says cross-platform desktop intelligence | Formalize platform adapter traits for windowing, accessibility, input, process, browser | Clear path to Windows/macOS without rewriting core |
| Eval failures show integration fragility | Latest report has App Logic and Smoke failures | Production-grade means full test report must pass consistently | Treat failed suites as architecture blockers, not just test cleanup | Higher confidence before adding advanced autonomy |

### Test Signals To Treat As Architecture Issues

Historical eval reports surfaced these architecture-level signals:

| Suite | Status | Concrete Signal | Architecture Meaning |
| ----- | ------ | --------------- | -------------------- |
| App Logic | Failed | missing `ToolEnd` for MCP prompt-output tools | MCP/tool event lifecycle is not yet reliable enough for external-system coworker workflows |
| Smoke | Failed | Docker service status routed to `manage_service` instead of expected `execute_bash` | routing semantics for inspect/manage operations need clearer contracts |
| Cognitive E2E | Passed | chat regression and cognitive tests pass | cognitive foundations are useful but not sufficient |
| Safety/chaos | Passed | destructive VM and red-tier chaos passed | safety layer is a relative strength |

### Data Flow Target

Current data flow is functional but scattered:

```text
TurnGate
  -> router/tool hints
  -> prompt context
  -> LLM/tool loop
  -> tool messages
  -> verifier/synthesizer
```

Preferred coworker-grade flow:

```text
UserGoal
  |
  v
OperationalContext
  |-- desktop state
  |-- files/workspace
  |-- browser/IDE state
  |-- active goals/workflows
  |
  v
ExecutionPlan
  |-- stages
  |-- tools/substrates
  |-- policy expectations
  |-- completion criteria
  |
  v
RuntimeLedger
  |-- every action
  |-- every result
  |-- every verifier decision
  |-- every recovery branch
  |
  v
Grounded response
```

### Implementation Roadmap For Reaching The Vision

| Priority | Implementation | Files | Expected Result |
| -------- | -------------- | ----- | --------------- |
| P0 | Fix all red test report failures before adding features | failing tests under `crates/kria-core/tests` | Stabilizes current architecture |
| P0 | Enforce event lifecycle invariant: every planned/attempted tool gets terminal event | `agent/loop_engine/mod.rs`, `mcp/*`, `tools/registry.rs` | UI and tests never hang waiting for missing tool completion |
| P1 | Build unified runtime ledger | `agent/execution_trace.rs`, `agent/execution_transparency/mod.rs`, `agent/workflow_session.rs` | One place to inspect operational truth |
| P1 | Make multi-step prompts graph-first | `agent/goal_tree.rs`, `agent/opgraph.rs`, `agent/workflow_compiler.rs` | Less ReAct fragility, better recovery |
| P1 | Unify PSDG, desktop awareness, and environment grounding | `agent/psdg/*`, `agent/desktop_awareness/*`, `agent/environment_grounder.rs` | Better "what is happening on my desktop?" understanding |
| P2 | Add platform adapter contracts | `platform/*`, `agent/atspi_engine.rs`, `tools/gui_automation.rs` | Clear Linux/Windows/macOS boundary |
| P2 | Add operational eval gates per workflow class | `crates/kria-core/tests/*`, `test_runner/mod.rs` | Prevents regressions from passing shallow tests |

### Success Definition

KRIA reaches the core runtime expectation when a multi-step request can be explained as:

```text
I understood the goal.
I selected the safest substrate.
I executed bounded stages.
I verified the requested outcome.
I preserved enough state to continue.
I can explain what failed and what to do next.
```

Today, those pieces exist in code, but the next maturity step is to make them one
consistent runtime contract.
