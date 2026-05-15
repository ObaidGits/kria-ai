# KRIA Phase 1 — Semantic Workflow Intelligence: Implementation Plan

## Context

KRIA has a strong mechanical substrate (Rust execution engine, kill switch, HTN executor, daemon protocol, safety boundaries) but its "intelligence" is a keyword substring matcher + unconstrained LLM fallback. Commands like "open gedit and type hello world" work. Commands like "open VS Code and create a fibonacci program and run it" fail not because KRIA can't click/type, but because it lacks the cognition layer that understands *what the task means*, *whether it succeeded semantically*, and *what the environment looks like*. Phase 1 adds that narrow, bounded cognitive interface.

---

## 1. Current KRIA Diagnosis

| Aspect | Diagnosis |
|--------|-----------|
| **What KRIA IS** | A sovereign Rust motor cortex with deterministic guards, HTN executor, uinput daemon, kill switch, and hard audit boundary. Industrial-grade automation substrate. |
| **What KRIA IS NOT** | An intelligent assistant. The "intelligence" is `lower.contains(...)` + `serde_json::from_str()` on LLM output. |
| **Biggest missing layer** | A typed semantic interface between intent and execution: `User Text → IntentCompiler (GuiTaskSpec) → EnvironmentGrounder (OperationalFacts) → GuiPlanner (GoalTree) → ExecutionVerifier (Verifiability)` |
| **Biggest operational weakness** | Verification checks "did I click the button" not "did the task succeed". False success on every type_text. |
| **Biggest architectural strength** | Sovereign Rust planning/safety boundary, immutable plans, kill switch + teardown, audit logging. |
| **Biggest implementation risk** | Integrating existing orphaned modules (`uncertainty/belief_graph`, `failure_analyzer`, `world_model`) before they are audit-ready. |

---

## 2. Core Missing Cognition Layers

| Missing Capability | Why Current KRIA Fails | Consequence | Optimal Bounded Solution |
|---|---|---|---|
| **Typed semantic intent extraction** | `generate_gui_workflow` is two `lower.contains(...)` chains; LLM emits flat JSON | Brittle classification, no clarification path, prompt-injection surface | `IntentCompiler` trait: pure function text→`GuiTaskSpec`. Emits `ClarifyRequest` on ambiguity, never guesses. |
| **Operational environment grounding** | `htn_executor` reads `get_active_window()` once at start; environment never informs planning | Wrong-window grab, missing-app workflows proceed regardless | `EnvironmentGrounder` trait: closed-enum `OperationalFacts` (≤32 facts, ≤10s TTL). CPU-only, <20ms. |
| **Goal-tree planning authority** | Two parallel planners emit flat `Vec<SubGoal>`; RFC 008 Goal Tree is paper-only | Adaptive recovery has no data structure; PRA cannot be type-checked | `GuiPlanner` trait: THE single planner. Rule path first → LLM path → produces typed `GoalTree`. GBNF constraint on LLM output. |
| **Intent-level execution verification** | `VerificationEngine` checks window title/OCR surface, never task success | `type_text` "verified" when bytes leave keyboard; `open editor` verified when window title matches but wrong app focused | `ExecutionVerifier` with **Verifiability Classes**: `WindowState`, `FileSystemEffect`, `ProcessLaunched`, `DeterministicOutput`, `OcrTextPresent`, `UserAttested`, `Unverifiable`. Single bounded check per leaf, ≤500ms, never replans. |
| **Uncertainty-driven control** | `agent/uncertainty/belief_graph.rs` exists but never consulted by GUI path | No mechanism to escalate ambiguous workflows to HITL | `UncertaintyGovernor`: wraps belief_graph, emits 0.0-1.0 score. Below 0.4 → HITL, below 0.2 → KillSwitch. CPU-only, <1ms. |

---

## 3. Detailed Component Designs

### 3.1 IntentCompiler

**Purpose:** Semantic normalization. Converts natural language to typed `GuiTaskSpec`. MUST NOT plan, read environment, or call LLM vision.

**Authority boundaries:**
- MAY classify content as `Literal` vs `Generated` (reuse existing `ContentGenerator`)
- MAY surface `Ambiguity` variants
- MUST NOT emit step lists, read screens, call OmniParser

**Runtime lifecycle:**
```
User text → IntentCompiler::compile() → GuiTaskSpec
                                         ├─ Ok(spec) → proceed
                                         └─ Err(ClarifyRequest) → emit HITL prompt, halt execution
```

**Rust interface (already defined in intent_compiler.rs):**
```rust
pub trait IntentCompiler: Send + Sync {
    fn compile(&self, user_text: &str, intent: &IntentEnvelope) -> Result<GuiTaskSpec, ClarifyRequest>;
}

// Key types already defined:
pub enum Verb { Open, Type, Click, Run, Save, Close, Switch, Other(String) }
pub enum TargetRef { App(String), File(PathBuf), Url(String), Element(String) }
pub enum ContentClass { Literal(String), Generated { hint: String, language: Option<String> } }
pub enum Ambiguity { AppNotSpecified, FileNotSpecified, MultipleTargetsPossible, ContentScopeUnclear }
pub struct GuiTaskSpec { pub primary_verb: Verb, pub targets: Vec<TargetRef>, pub content: Option<ContentClass>, pub declared_preconditions: Vec<PrereqHint>, pub declared_success_criteria: Vec<SuccessHint>, pub ambiguities: Vec<Ambiguity> }
```

**Implementation approach:**
- LLM-powered: Use `LocalBackend::chat()` with a tight system prompt that extracts `GuiTaskSpec` fields from text.
- Fallback rule-based parser for trivial cases ("open gedit") for <5ms path.
- Integration: Called from `loop_engine/mod.rs` before routing to `htn_integration`.

**Ambiguity resolution:**
```rust
match ambiguity {
    Ambiguity::AppNotSpecified => "Which application should I open? Options: [gedit, VS Code, Firefox, Terminal]",
    // ... all variants must have user-facing clarify options
}
```
NEVER pick a default on `Ambiguity::AppNotSpecified`. Always halt and ask.

**Key files to modify:**
- [intent_compiler.rs](crates/kria-core/src/agent/intent_compiler.rs): Replace `NoopIntentCompiler` with `LlmIntentCompiler` + `RuleIntentCompiler` fallback.

---

### 3.2 EnvironmentGrounder

**Purpose:** Closed-enum operational facts about the live desktop state. Read-only. Bounded.

**Authority boundaries:**
- MAY read window list, process list, file existence, monitor geometry via syscalls
- MUST NOT write state, call LLM, emit plans

**Runtime lifecycle:**
```
GuiTaskSpec → EnvironmentGrounder::ground() → OperationalFacts
                                              ├─ facts fed to GuiPlanner
                                              └─ cached in world_model/store.rs (typed cache only)
```

**Rust interface (already defined in environment_grounder.rs):**
```rust
pub trait EnvironmentGrounder: Send + Sync {
    async fn ground(&self, spec: &GuiTaskSpec) -> OperationalFacts;
}

pub struct OperationalFacts {
    pub focused_window: Option<WindowFact>,           // capped at 1
    pub foreground_processes: Vec<ProcessFact>,        // capped at 8
    pub workspace_root: Option<PathBuf>,
    pub file_facts: Vec<FileFact>,                      // capped at 16, only files from spec
    pub terminal: Option<TerminalFact>,
    pub monitors: Vec<MonitorFact>,                    // multi-monitor map
    pub captured_at: Instant,                          // TTL ≤10s
}
```

**Implementation approach:**
- `XcbGroundImpl`: Use `xcb` library to query window tree, active window, `_NET_CLIENT_LIST`.
- `ProcGroundImpl`: Read `/proc` filesystem for top-8 foreground processes.
- `FileGroundImpl`: `std::fs::metadata()` calls for files in `spec.targets` only.
- `MonitorGroundImpl`: Query RandR/X11 for monitor geometry and scale.
- Cache: Store in `world_model/store.rs` with TTL. No embeddings, no graph.
- TTL enforcement: `OperationalFacts::is_fresh()` check before each use.

**Key files to modify:**
- [environment_grounder.rs](crates/kria-core/src/agent/environment_grounder.rs): Replace `NoopEnvironmentGrounder` with real implementations.
- [world_model/store.rs](crates/kria-core/src/agent/world_model/store.rs): Wire as typed cache for grounder.

---

### 3.3 GuiPlanner (GoalTree)

**Purpose:** THE single planner for GUI tasks. Produces `GoalTree`, not flat `Vec<SubGoal>`.

**Authority boundaries:**
- MAY produce step lists (GoalTree)
- MAY invoke LLM with GBNF constraint
- MUST NOT execute steps, mutate state, call daemon

**Relationship to existing code:**
- `htn_integration.rs` contains `generate_gui_workflow()` (keyword-based rule planner) and `plan_gui_workflow_via_llm()` (LLM planner). Both emit flat `GuiWorkflow { sub_goals: Vec<SubGoal> }`.
- These need to be refactored into a unified `GuiPlanner` trait that emits `GoalTree`.

**New GoalTree types to add:**
```rust
pub struct GoalTree {
    pub task_id: String,
    pub max_duration_sec: u64,
    pub root_goal: Goal,
    pub fallback_subtrees: HashMap<String, Subtree>,  // keyed by prereq failure
    pub safe_abort_steps: Vec<SafeAbortStep>,
}

pub struct Goal {
    pub id: String,
    pub kind: GoalKind,              // Execution | Sense | Compound
    pub prerequisites: Vec<Prerequisite>,
    pub execution_steps: Vec<SubGoal>,
    pub verify: Verifiability,       // P4: linked to ExecutionVerifier
}

pub struct Prerequisite {
    pub id: String,
    pub kind: PrereqKind,           // AppRunning | FileExists | WindowVisible | ...
    pub fallback_subtree_id: Option<String>,  // references fallback_subtrees
}
```

**Implementation approach:**
- `RuleGuiPlanner`: Refactor `generate_gui_workflow()` from `htn_integration.rs` to emit GoalTree shape with prerequisites derived from `OperationalFacts`.
- `LlmGuiPlanner`: Call `LocalBackend::chat_with_grammar()` with GBNF for GoalTree. Validate post-parse: every action in allow-list, every leaf has `Verifiability`.
- `CompositeGuiPlanner`: Try Rule first (fast path, <2ms), fallback to LLM on `PlanError::NoMatch`.
- **Critical**: Collapse TWO existing planners into ONE. Remove `plan_gui_workflow_via_llm` standalone export.

**GBNF constraint (prevents LLM hallucination):**
```ebnf
GoalTree = "{" task_id "," max_duration "," root_goal "," safe_abort "}"
root_goal = "{" id "," kind "," prerequisites "," steps "," verify "}"
subtree = "{" id "," goals "}"
verify = "{" "type" ":" verify_type "," ... "}"
verify_type = "\"WindowState\"" | "\"FileSystemEffect\"" | "\"ProcessLaunched\"" | "\"DeterministicOutput\"" | "\"UserAttested\"" | "\"Unverifiable\""
```

**Key files to modify:**
- [htn_integration.rs](crates/kria-core/src/agent/htn_integration.rs): Refactor `generate_gui_workflow` + `plan_gui_workflow_via_llm` into `RuleGuiPlanner` + `LlmGuiPlanner`.
- [gui_wiring.rs](crates/kria-core/src/agent/gui_wiring.rs): Update `GuiExecutionCoordinator` to use new planner trait.
- New: `crates/kria-core/src/agent/gui_planner.rs` — trait + implementations.

---

### 3.4 ExecutionVerifier

**Purpose:** Bounded single-shot verification per Verifiability leaf. NEVER replans.

**Authority boundaries:**
- MAY read filesystem, spawn subprocess, call OmniParser (for OCR)
- MUST NOT emit step lists, mutate active queue, loop

**Rust interface (already defined in execution_verifier.rs):**
```rust
pub trait ExecutionVerifier: Send + Sync {
    async fn verify(&self, leaf: &Verifiability) -> VerifyOutcome;
}

pub enum Verifiability {
    WindowState { title_contains: Option<String>, class: Option<String> },
    FileSystemEffect { path: PathBuf, kind: FsEffect },
    ProcessLaunched { binary: String, max_wait_ms: u32 },
    DeterministicOutput { expected_substring: String, in_target: VerifyTarget },
    OcrTextPresent { text: String, case_insensitive: bool },
    UserAttested { question: String },
    Unverifiable { reason: String },  // MUST surface HITL, never silent success
}
```

**Implementation per class:**
| Class | Check | Latency cap | Method |
|---|---|---|---|
| `WindowState` | Query active window title/class | ≤100ms | X11/XCB `get_active_window` |
| `FileSystemEffect` | `std::fs::metadata`, read file bytes | ≤100ms | `std::fs` |
| `ProcessLaunched` | Poll `/proc` for PID | ≤500ms | `std::fs` |
| `DeterministicOutput` | Read terminal output or file | ≤200ms | `std::fs` or pipe |
| `OcrTextPresent` | Substring search on cached OCR | ≤300ms | `vision_automation` |
| `UserAttested` | N/A — never auto-verifies | N/A | HITL via `GuiEvent::HumanActivityDetected` |
| `Unverifiable` | N/A | 0ms | Emit `HitlEscalated`, never report success |

**Critical rule:**
> The verifier runs ONCE per leaf. If it returns `verified: false`, the executor handles recovery via pre-registered fallback subtrees. The verifier MUST NOT invoke the planner.

**Key files to modify:**
- [execution_verifier.rs](crates/kria-core/src/agent/execution_verifier.rs): Replace `NoopExecutionVerifier` with `BoundedExecutionVerifier`.
- [htn_executor.rs](crates/kria-core/src/agent/htn_executor.rs): Replace inline `VerificationEngine` with `ExecutionVerifier` trait calls.

---

### 3.5 UncertaintyGovernor

**Purpose:** Single 0.0-1.0 uncertainty score per task. Escalates to HITL or KillSwitch below thresholds.

**Authority boundaries:**
- MAY aggregate verifier outcomes, emit safety events
- MUST NOT emit step lists, call LLM (uses existing belief_graph CPU-only)

**Integration with existing code:**
- Wrap `agent/uncertainty/belief_graph.rs` — exists but is orphaned from GUI path.
- Wire into `GuiExecutionCoordinator` → `GuiEvent bus`.

**Score propagation:**
```
Initial confidence (from IntentEnvelope) → 1.0
Verifier returns verified=true → confidence += 0.1 (capped 1.0)
Verifier returns verified=false → confidence -= 0.25
Ambiguity surfaced → confidence -= 0.4
Step failed → confidence -= 0.3
Below 0.4 → emit HitlEscalated
Below 0.2 → engage KillSwitch
Above 0.6 → autonomous execution
```

**Key files to modify:**
- [uncertainty/mod.rs](crates/kria-core/src/agent/uncertainty/mod.rs): New `UncertaintyGovernor` wrapper.
- [gui_wiring.rs](crates/kria-core/src/agent/gui_wiring.rs): Wire governor into execution flow.

---

## 4. End-to-End Runtime Flow

### Example: "Open VS Code and create a fibonacci program and run it"

```
User → "Open VS Code and create a fibonacci program and run it"

[TurnGate] → IntentEnvelope { operation: Automate, compute: L1Text, hazard: Green }
    │
    ▼
[IntentCompiler::compile] → Ok(GuiTaskSpec {
    primary_verb: Verb::Other("develop"),  // not in enum, classify as compound
    targets: [App("VS Code"), App("Terminal")],
    content: Some(Generated { hint: "fibonacci program", language: Some("python") }),
    declared_preconditions: [PrereqHint::AppOpen("VS Code"), PrereqHint::FileExists("./fib.py")],
    declared_success_criteria: [SuccessHint::ProcessExited(0)],
    ambiguities: [Ambiguity::ContentScopeUnclear],  // "run it" — run how? terminal? F5?
})
    │
    ▼
[EnvironmentGrounder::ground] → OperationalFacts {
    focused_window: Some(WindowFact { title: "K.R.I.A.", class: "KriaApp", pid: 12345, monitor_id: 0 }),
    foreground_processes: [ProcessFact { binary: "kria", pid: 12345, cpu_share: 2.1 }, ...],
    workspace_root: Some("/home/obaid/projects"),
    file_facts: [FileFact { path: "/home/obaid/projects/fib.py", exists: false, size: None }],
    terminal: Some(TerminalFact { binary: Some("gnome-terminal"), focused: false }),
    monitors: [MonitorFact { id: 0, geometry: Rect { x: 0, y: 0, w: 1920, h: 1080 }, scale: 1.0, primary: true }],
    captured_at: Instant::now(),
}
    │
    ▼
[GuiPlanner::plan] → GoalTree {
    root_goal: Goal {
        id: "develop-fib",
        kind: Compound,  // multi-stage
        prerequisites: [
            Prereq { id: "p1", kind: AppRunning("code"), fallback_subtree_id: Some("launch-vscode") },
            Prereq { id: "p2", kind: FileExists("./fib.py"), fallback_subtree_id: Some("create-file") },
        ],
        execution_steps: [
            SubGoal { step: 1, action: "open_application", params: {"name": "code"}, verify: WindowState { title_contains: Some("Visual Studio Code") } },
            SubGoal { step: 2, action: "system_sleep", params: {"duration_ms": 3500}, verify: None },  // wait for VS Code window
            SubGoal { step: 3, action: "get_screen_elements", params: {"filter_type": "text"}, verify: ElementsFound { ids: ["editor"], min: 1 } },
            SubGoal { step: 4, action: "click_element", params: {"element_id": "editor"}, verify: ScreenChanged { element_id: Some("editor") } },
            SubGoal { step: 5, action: "type_text", params: {"text": "def fib(n):\n    if n <= 1: return n\n    return fib(n-1) + fib(n-2)\n\nfor i in range(10):\n    print(fib(i))", "interval_ms": 5}, verify: DeterministicOutput { expected_substring: "0\n1\n1\n2\n3\n5\n8\n13\n21\n34", in_target: TerminalOutput } },
        ],
        verify: ProcessExited(0),
    },
    fallback_subtrees: {
        "launch-vscode": Subtree { goals: [SubGoal { step: 1, action: "open_application", params: {"name": "code"}, ... }] },
        "create-file": Subtree { goals: [SubGoal { step: 1, action: "type_text", params: {"text": ""}, verify: FileSystemEffect { path: "./fib.py", kind: Exists } }] },
    },
    safe_abort_steps: [SafeAbortStep { action: "press_shortcut", params: {"keys": ["Escape"] } }],
}
    │
    ▼
[GuiExecutor] → Execute GoalTree linearly
    │
    ├─ Step 1: open_application("code") → daemon action → VERIFIED (WindowState)
    ├─ Step 2: system_sleep(3500) → no-op
    ├─ Step 3: get_screen_elements → elements found
    ├─ Step 4: click_element("editor") → VERIFIED (ScreenChanged)
    ├─ Step 5: type_text(fibonacci code) → ACTION DONE
    │       [ExecutionVerifier] → DeterministicOutput check:
    │           → Spawn terminal → run `python3 fib.py` → capture stdout
    │           → "0\n1\n1\n2\n3\n5\n8\n13\n21\n34" matches expected? → YES
    │           → VerifyOutcome { verified: true, confidence: 0.95, evidence: "output matches" }
    │
    ▼
[UncertaintyGovernor] → score 0.95 → autonomous
    │
    ▼
[TaskCompleted] → Report to user
```

### Failure recovery scenario (wrong window focused):

```
Step 1: open_application("code") → Verifier returns WindowState(title_contains: "VS Code") → FALSE (active window is "K.R.I.A.")
    │
    ▼
[UncertaintyGovernor] → score drops to 0.7
    │
    ▼
[PrereqFailure detected] → Check GoalTree.fallback_subtrees for "launch-vscode" subtree
    │
    ├─ subtree exists AND branch hasn't failed before → inject "launch-vscode" subtree
    │       └─ Step 1: click VS Code window explicitly → verify → PASS
    │       └─ Resume main workflow
    │
    └─ same branch fails AGAIN → HitlEscalated ("Could not focus VS Code. Please click on it manually.")
```

---

## 5. Optimal Implementation Roadmap

| Phase | Goal | Complexity | Runtime Cost | Intelligence Gain | Priority | Dependencies |
|---|---|---|---|---|---|---|
| **P1a** | `IntentCompiler` trait + `LlmIntentCompiler` | Low | <5ms CPU + on-demand L1Text | HIGH | **1** | None |
| **P1b** | Wire `IntentCompiler` into `loop_engine` before routing | Low | Negligible | HIGH | **1** | P1a |
| **P2** | `EnvironmentGrounder` with X11/RandR/Proc reads | Medium | <20ms CPU | HIGH | **2** | None |
| **P4** | `ExecutionVerifier` with all 7 Verifiability classes | Medium | <500ms/leaf CPU | **CRITICAL** | **3** | P2 (needs file facts) |
| **P3** | `GuiPlanner` refactor: GoalTree shape + GBNF LLM | Medium | 0.5-3s L1Text (on miss only) | HIGH | **4** | P1, P2 |
| **P5** | `UncertaintyGovernor` wiring belief_graph | Medium | <1ms CPU | MEDIUM | **5** | P4 |
| **P6** | `SafetyTrustBoundary` OCR sanitization audit | Low | <2ms CPU | HIGH | **6** | None |
| **P7** | GUI Event Bus + tracing spans + NDJSON traces | Low | Negligible | MEDIUM | **7** | P1-P5 |
| **P8** | FailureSignature/BranchIdentity consumption in executor | Medium | CPU | MEDIUM | **8** | P3, P4 |
| **P9** | `OperationalMemory` tier (SQLite EWMA cache) | Medium | <10ms CPU | LOW | **9** | P1-P8 |
| **P10** | Bounded Ctrl+Z rollback for editors | Low | None | LOW | **10** | P3 |

**Rollout strategy:**
1. First implement P1a+P1b — lowest risk, highest impact per line of code.
2. P4 before P3 is deliberate — honest success/failure signals are more valuable than richer plans.
3. P6 (OCR safety) is cheap and should land early — it's a security hardening.
4. P9 is last — persistent memory is nice-to-have, not the intelligence core.

---

## 6. Biggest Architectural Risks

| # | Risk | Severity | Mitigation |
|---|---|---|---|
| 1 | **Verifier becomes a replanner** — verifier calls LLM or emits steps | CRITICAL | Hard rule in code review: `ExecutionVerifier` trait has no return type that contains step lists |
| 2 | **Cognition layer coupling** — IntentCompiler starts reading screens, Grounder starts planning | HIGH | Invariants enforced in `mod.rs` exports: only trait trait references cross-layer boundaries |
| 3 | **Recursive recovery spiral** — same prereq fails twice → infinite fallback injection | HIGH | P8 (FailureSignature/BranchIdentity) MUST land before any fallback subtree injection is enabled |
| 4 | **Environment state explosion** — grounder gets unbounded facts | MEDIUM | Hard caps: ≤32 facts, ≤10s TTL, only files from `spec.targets` |
| 5 | **LLM planner overreach** — model ignores GBNF constraint | MEDIUM | Post-parse validator rejects unknown actions; falls back to `RuleGuiPlanner` on failure |
| 6 | **Orphaned modules diverging** — `uncertainty/belief_graph` evolves without GUI path consumer | MEDIUM | Integration-readiness audit before P5 (1 day) |
| 7 | **Target lock wrong window** — window spawn tracker not implemented, apps fork | HIGH | `WindowSpawnTracker` must land in P2; critical bug class |
| 8 | **Wayland silent breakage** — xdotool calls fail silently on Wayland | MEDIUM | Already has probe; route modifier release through ydotool in P0 |
| 9 | **Multi-monitor coordinate chaos** — clicks on wrong monitor | MEDIUM | `MonitorFact` in P2; coordinate mapping in executor |
| 10 | **Observability gap** — no way to debug cognition layer failures | LOW | P7 Event Bus + tracing spans are prerequisite for debugging P3-P5 |

---

## 7. Overengineering Warnings

These MUST be rejected in code review:

| Trap | Why It Harms KRIA |
|---|---|
| **Always-on VLM** | 6GB VRAM budget cannot sustain a resident multimodal model + L1 text + OmniParser. Vision is on-demand only. |
| **Vector DB of UI semantics** | `UiPerceptionCache` must be ephemeral, RAM-only, task-scoped. No embeddings persisted. |
| **Symbolic world model** | `EnvironmentGrounder` is a closed-enum of facts. It is NOT a knowledge graph. Any unbounded key/value addition → reject. |
| **Multi-agent swarms** | One executor, one planner. Multi-agent coordination is not in scope. |
| **Autonomous self-rewriting planner** | Plans are immutable post-compile. PRA injects only from pre-registered fallback subtrees. |
| **Unbounded ReAct fallback** | LLM HTN planner runs ONCE, with GBNF, with schema validation. No iterative refinement loops at the GUI layer. |
| **Learned timing model affecting branching** | EWMA per app changes waits, never paths. Timing cannot influence planning decisions. |
| **Cross-task semantic memory** | SessionState carries forward only numeric EWMAs + last-3 outcomes by class. No raw OCR text, no intents, no generated content. |
| **VLM for deceptive-dialog detection** | Heuristic rule check only. VLM would be too slow and too expensive for this. |

---

## 8. Final Verdict

### Is semantic workflow intelligence the TRUE missing layer?

**Yes.** KRIA's mechanical substrate is solid. The failure is cognitive: no typed contract between "what the user wants" and "what the executor runs." Everything else — uncertainty, grounding, verification — is wiring around that spine.

### Can KRIA become a highly intelligent local desktop assistant on this hardware?

**Yes — within a specific operating envelope.** "Intelligent" means: correctly interprets natural-language GUI requests, grounds them against live environment, executes typed plans, verifies intent-level success, and escalates ambiguities. It does NOT mean: reasons in the loop about novel UI. The RTX 4050 + 16GB budget fits the proposed stack: **net new VRAM = 0**, **net new RAM < 30 MB**.

### What should be implemented FIRST?

1. **P1a `IntentCompiler`** — replaces the keyword substring gate with typed normalization + clarification path.
2. **P4 `ExecutionVerifier`** — ends the false-success class of bugs. Highest operational impact per implementation effort.
3. **P3 `GuiPlanner` GoalTree** — makes the LLM planner safe and makes RFC 008 PRA implementable.

### What should NOT be implemented yet?

- Any always-on VLM
- Vector DB or symbolic knowledge graphs
- Multi-agent systems
- Autonomous self-rewriting planners
- Cross-session semantic memory beyond numeric EWMAs

### What is the SINGLE MOST IMPORTANT thing KRIA still lacks?

**Typed Verifiability Classes.** Verification that checks "did the task succeed" not "did the button get clicked." This is the root cause of most "it said Done but didn't work" failures.

### What is the biggest future risk?

**Recursive recovery instability** when `GuiExecutor` actually starts injecting fallback subtrees. The spiral-prevention algorithm (`FailureSignature`/`BranchIdentity` from RFC 008) is paper-only. Until P8 lands and proven by adversarial testing, this is the most likely source of autonomous loop misbehavior.

---

## Critical File Map

| File | Role | Change |
|---|---|---|
| [intent_compiler.rs](crates/kria-core/src/agent/intent_compiler.rs) | P1: Semantic normalization | Replace no-op with `LlmIntentCompiler` |
| [environment_grounder.rs](crates/kria-core/src/agent/environment_grounder.rs) | P2: Operational facts | Replace no-op with real X11/Proc/RandR reads |
| [execution_verifier.rs](crates/kria-core/src/agent/execution_verifier.rs) | P4: Verifiability classes | Replace no-op with `BoundedExecutionVerifier` |
| [htn_integration.rs](crates/kria-core/src/agent/htn_integration.rs) | P3: Planner refactor | Collapse to single `GuiPlanner` trait |
| [gui_wiring.rs](crates/kria-core/src/agent/gui_wiring.rs) | All phases: wiring | Wire new components into coordinator |
| [htn_executor.rs](crates/kria-core/src/agent/htn_executor.rs) | P3-P4: Executor refactor | Consume `GoalTree`, call `ExecutionVerifier` |
| [loop_engine/mod.rs](crates/kria-core/src/agent/loop_engine/mod.rs) | P1b: Integration | Call `IntentCompiler` before routing |
| [agent/mod.rs](crates/kria-core/src/agent/mod.rs) | All phases: exports | Update feature gate for `gui_cognition_v2` |
| [uncertainty/mod.rs](crates/kria-core/src/agent/uncertainty/mod.rs) | P5: UncertaintyGovernor | Wrap belief_graph as governor |
| [world_model/store.rs](crates/kria-core/src/agent/world_model/store.rs) | P2: Typed cache | Wire as grounder cache, not a graph |
| [GUI_INTELLIGENCE_REVIEW.md](docs/GUI_INTELLIGENCE_REVIEW.md) | Reference architecture | Authoritative design document |

---

## Verification

1. **Unit tests**: `cargo test -p kria-core -- intent_compiler execution_verifier environment_grounder`
2. **Property tests**: GoalTree round-trip, planner determinism, FailureSignature uniqueness
3. **Integration tests**: `GuiExecutor` with `MockBackend` (already exists in htn_executor.rs)
4. **E2E sandboxed**: Full stack against `kria-test-app` in Xvfb (`just test-e2e`)
5. **Adversarial**: Deceptive dialog, OCR injection, wrong-window grab, failure spiral (`just test-adversarial`)
6. **Observability**: `~/.kria/traces/<task_id>.ndjson` contains all `GuiEvent` emissions

---

*Plan based on: `docs/GUI_INTELLIGENCE_REVIEW.md` (RFC v2, 2026-05-12) and codebase analysis of `crates/kria-core/src/agent/`*