# KRIA GUI Execution

## Purpose

This document describes the current live GUI execution path. GUI cognition in
KRIA is a bounded runtime contract layer around desktop execution, not a
general desktop planner and not a second executor.

Primary implementation areas:
- `crates/kria-core/src/agent/gui_wiring.rs`
- `crates/kria-core/src/agent/gui_substrate_planner.rs`
- `crates/kria-core/src/agent/semantic_workflow.rs`
- `crates/kria-core/src/agent/execution_mode_reasoner.rs`
- `crates/kria-core/src/agent/workflow_intent_contract.rs`
- `crates/kria-core/src/agent/verifier_authority.rs`
- `crates/kria-core/src/agent/hybrid_synchronization.rs`
- `crates/kria-core/src/agent/gui_production_readiness.rs`

## Runtime Flow

Current GUI flow:

```text
user prompt
  -> IntentGate / TurnGate
  -> IntentCompiler / RuleIntentCompiler
  -> GuiTaskSpec
  -> SemanticWorkflowAnalysis
  -> ExecutionModeDecision
  -> WorkflowIntentContract check
  -> VerifierAuthorityAssessment
  -> SubstratePlanner
  -> GuiWorkflow / GoalTree / StageExecutor
  -> tool execution
  -> verifier / result synthesis / final response
```

The semantic workflow objects are resolved before substrate planning. They
record expected workflow shape, fidelity, required visible evidence, fallback
policy, and verifier authority requirements. Concrete desktop steps are still
owned by the substrate planner and executor.

## Semantic Contract Layer

| Object | Owner module | Runtime role | Boundary |
|---|---|---|---|
| `SemanticWorkflowFrame` | `semantic_workflow.rs` | Captures task family, app anchors, visibility, observation, collaboration, ambiguity, and safety class | Does not execute, verify, or plan tools |
| `WorkflowFidelityResolution` | `semantic_workflow.rs` | Selects requested/minimum/planned fidelity tier and degradation policy | Does not probe the desktop |
| `ExecutionModeDecision` | `execution_mode_reasoner.rs` | Selects structural, visible, hybrid, HITL, verification-visible, or silent mode | Does not generate concrete steps |
| `WorkflowIntentContract` | `workflow_intent_contract.rs` | Declares invariants, required app classes, verifier requirements, forbidden degradations, fallback, HITL, and response requirements | Declarative only |
| `VerifierAuthorityAssessment` | `verifier_authority.rs` | States what evidence authority is required for claims | Does not perform live verification by itself |
| `HybridSynchronizationAssessment` | `hybrid_synchronization.rs` | Describes required structural-visible sync checkpoints | Does not refresh apps or repair state |

The contract layer is intentionally deterministic. LLM interpretation may help
create upstream intent, but mode selection and contract evaluation are not LLM
authority.

## Execution Modes

| Mode | Meaning | Typical example |
|---|---|---|
| `StructuralExecution` | Backend/headless completion is sufficient | Create a file, run a safe command, report result |
| `VisibleAppWorkflow` | Real app context is semantically required | Open a spreadsheet and edit visible cells |
| `HybridWorkflow` | Structural work plus visible app/result surfacing | Write code structurally, open IDE, run/surface output |
| `HumanCollaborativeWorkflow` | User review/approval is part of completion | Draft email and wait before sending |
| `VerificationVisibleWorkflow` | Final state must be visible or directly inspectable | Show browser result or terminal output |
| `SilentAutomationWorkflow` | User explicitly allows background execution | Organize files or run maintenance quietly |

The mode decision is trace metadata and a planning constraint. The executor must
not silently reinterpret it into a weaker workflow. If visible fidelity cannot
be satisfied, the user-facing result must be partial or blocked, not full
success.

## Substrate Planner

`gui_substrate_planner.rs` selects bounded substrates from the task spec and
semantic constraints.

Current substrates include:

| Substrate | Use |
|---|---|
| `FileWriteThenOpen` | Create/update a file, then surface it in an app when useful |
| `TerminalExecution` | Run commands and capture output |
| `IdeCodeRunWorkflow` | Coding workflow with IDE/file/run semantics |
| `AppOpenOnly` | Open an application or target document |
| `BrowserNavigate` | Open/navigate/search in a browser-capable surface |
| `InteractionHeavy` | Click/type/interaction tasks that need live GUI capability |
| `Keystroke` | Last-resort text input path |
| `Unknown` | No safe deterministic substrate selected |

The planner prefers safer, verifiable substrates over brittle raw GUI input. Raw
keystroke automation remains last-resort and verifier-gated.

## Readiness Gates

`gui_production_readiness.rs` provides deterministic preflight checks for GUI
modes.

| Readiness mode | Requirements |
|---|---|
| `StructuralOnly` | No display requirement |
| `LiveDesktop` | Display server and AT-SPI should be available |
| `InteractionHeavy` | Display, AT-SPI, uinput, and OCR/vision support are expected |
| `VmIsolated` | Same GUI needs, plus explicit VM eval enablement |

Important environment switches:
- `KRIA_EVAL_GUI=1` allows opt-in live GUI eval behavior.
- `KRIA_EVAL_VM=1` allows VM-isolated GUI eval behavior.
- Wayland interaction-heavy workflows are allowed only with explicit warnings
  because compositor behavior is environment-dependent.

## Verification And Visibility

Verifier authority distinguishes:
- structural evidence: file hashes, process status, command output, exit code,
- surface evidence: window/tab/document/terminal appears to show target state,
- semantic evidence: content/state satisfies task meaning,
- user-confirmed evidence: explicit human approval or confirmation.

Forbidden claims:
- window focus means correct content,
- app launched means workflow complete,
- browser opened means page/account is correct,
- terminal visible means the latest output is visible,
- file exists means IDE buffer is fresh,
- output file means output was shown.

For "show output" prompts, success requires output visible in an app/terminal,
surfaced directly to the user, or explicitly accepted as file-only fallback.

## Hybrid Synchronization

Hybrid workflows require structural-visible synchronization metadata before full
success can be claimed.

Supported checkpoint classes:
- file hash sync,
- workspace identity sync,
- terminal execution freshness,
- browser page freshness,
- account/session sync,
- visible artifact sync.

Visible evidence is invalid when it is stale, points at a different target, lacks
the current run marker, predates the workflow attempt, or conflicts with
structural truth.

## Failure Handling

Common outcomes:
- blocked: required capability or policy gate prevents execution,
- partial: structural work succeeded but visible fidelity was not proven,
- needs human: account/session/manual approval is required,
- failed: execution or verification evidence contradicted completion.

User-facing responses must state:
- what completed,
- what did not complete,
- what evidence exists,
- what visible requirement was missing,
- what safe recovery option remains.

## Current Limits

The current code resolves semantic GUI contracts before substrate planning and
records verifier/synchronization requirements. Live enforcement is still
incremental: the substrate planner and executor own concrete action steps, and
some verifier authority assessments are metadata unless the live path feeds
fresh observed evidence into them.

Production work should focus on turning those contract requirements into hard
executor gates wherever live GUI evidence is available.
