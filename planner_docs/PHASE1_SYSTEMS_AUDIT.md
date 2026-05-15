# KRIA Phase 1 Systems Audit — Brutal Honest Assessment

**Auditor:** Kimi K2.6 (Principal Rust Systems Architect / Runtime Cognition Auditor)  
**Date:** 2026-05-13  
**Scope:** P1a IntentCompiler, P1b loop_engine integration, P4 ExecutionVerifier, architecture boundary audit  
**Codebase state:** commit present at `/media/obaid/SSD/KRIA`  

---

# 1. Executive Verdict

**Did the implementation preserve bounded cognition?** **NO.**  
**Is KRIA still architecturally coherent?** **PARTIALLY — coherence is eroding.**  
**Should Phase 2 start now?** **NO. Refactoring is required first.**  

The P1a and P1b "implementations" are **skeletons that compile but do not govern runtime behavior**. The new `IntentCompiler` is called, its result is logged, and then **discarded** — execution proceeds through the old keyword-matcher path. The `BoundedExecutionVerifier` is written but **never wired into the executor**; the old `VerificationEngine` still runs. The `EnvironmentGrounder` is a `NoopEnvironmentGrounder` placeholder. There is no `GuiPlanner` trait, no `GoalTree`, and no `UncertaintyGovernor`.

**In short: Phase 1 shipped theater, not architecture.** The trait definitions are sound, but the integration layer silently preserves every old anti-pattern the architecture was meant to replace. Proceeding to Phase 2 would build on quicksand.

---

# 2. Critical Architectural Violations

| Severity | Violation | Why Dangerous | Exact Fix |
|----------|-----------|---------------|-----------|
| 🔴 **CRITICAL** | **IntentCompiler result is logged then discarded** | The entire P1a value proposition is void. `loop_engine/mod.rs:2398-2446` compiles the intent, logs verb/targets, then falls through to `generate_gui_workflow(user_text)` which re-parses with the OLD keyword matcher. Bounded semantic normalization is bypassed by the very code that claims to use it. | Pass `GuiTaskSpec` into `GuiExecutionCoordinator::generate_workflow` and consume it as the SOLE authority for planning. Delete the `requires_gui_automation` substring bypass. |
| 🔴 **CRITICAL** | **`requires_gui_automation` substring bypass still routes** | Even when `IntentCompiler` returns `Ok(non_gui_spec)`, the old `requires_gui_automation(&last_user_text)` call at `loop_engine/mod.rs:2521` can force GUI routing. `gui_wiring.rs:68-115` gates on confidence, but the `\|\|` at 2521 overrides it. | Remove `requires_gui_automation` from the routing expression. GUI routing must be EXCLUSIVELY determined by `IntentCompiler` output + `TurnGate` confidence, never by substring scanning. |
| 🔴 **CRITICAL** | **`BoundedExecutionVerifier` exists but is NEVER used** | `htn_executor.rs:1418-1458` still constructs `VerificationEngine` (the old surface-level checker). `BoundedExecutionVerifier` at `execution_verifier_impl.rs:34` has zero call sites outside its own `#[cfg(test)]` module. Intent-level verification is still "did I click the button." | Replace `VerificationEngine` field in `GuiExecutor` with `Arc<dyn ExecutionVerifier>`. Wire `BoundedExecutionVerifier` as the default. |
| 🔴 **CRITICAL** | **`LlmIntentCompiler::call_llm` blocks a Tokio worker thread** | `intent_compiler_llm.rs:419` calls `self.runtime_handle.block_on(self.backend.chat(...))` inside a synchronous `compile()` trait method. On a loaded runtime this can deadlock or stall the entire executor thread pool. | Change `IntentCompiler::compile` to `async fn compile(...)`. Propagate `async` through the loop engine call site. Never `block_on` inside library code. |
| 🔴 **CRITICAL** | **LLM output has NO GBNF/grammar constraint** | The system prompt at `intent_compiler_llm.rs:15-105` asks for JSON but provides no structural guarantee. `call_llm` uses `backend.chat(...)` not `chat_with_grammar`. The parser then trusts `serde_json::from_str` — exactly the F1 vulnerability the plan flagged. | Use `LocalBackend::chat_with_grammar` (already exists in `llm/local.rs`) with a compiled GBNF schema for `GuiTaskSpec`. Reject anything that does not parse to the closed enum. |
| 🔴 **CRITICAL** | **Silent fallback to `SuccessHint::UserConfirmed` for unknown LLM types** | `intent_compiler_llm.rs:326-327` maps any unknown `success_type` string to `SuccessHint::UserConfirmed`. An LLM hallucination of `"success_type": "FooBar"` becomes a silent user-attestation — which auto-passes. | Replace catch-all with `return Err(ClarifyRequest { question: "Malformed intent classification", ... })`. Fail closed. |
| 🔴 **CRITICAL** | **`std::sync::RwLock` used inside async verifier** | `execution_verifier_impl.rs:38-39` stores `ocr_cache` in `std::sync::RwLock`. `check_ocr_text_present` (`line 323`) acquires this lock across an async boundary, blocking the Tokio thread. | Replace with `tokio::sync::RwLock` or `dashmap::DashMap` for async-safe concurrent access. |
| 🟠 **HIGH** | **`world_model/store.rs` is a full symbolic knowledge graph** | The architecture explicitly banned "symbolic world model" and "vector DB memory systems." `store.rs` implements SQLite-backed S-P-O triples, FTS5 full-text search, archival tables, Bayesian confidence merging, and contradiction resolution. This is EXACTLY the inflation trap V3 warned against. | Strip to a typed RAM-only cache with TTL (`HashMap<K, V>` + `Instant`). Delete SQLite schema, FTS5, archiving, and Bayesian merge logic. |
| 🟠 **HIGH** | **`uncertainty/mod.rs` is system-admin diagnostics, not GUI uncertainty** | The planned `UncertaintyGovernor` (0.0-1.0 score, HITL/KillSwitch thresholds) does not exist. Instead `UncertaintyEngine` plans `systemctl`, `ping`, `curl`, `top` diagnostics for VM administration. It has NO connection to the GUI execution path. | Write `UncertaintyGovernor` as specified in KRIA_UPGRADE.md §3.5: wrap `belief_graph` numeric scores, emit thresholds, NO command generation. |
| 🟠 **HIGH** | **No `GoalTree`, no `GuiPlanner` trait, flat `Vec<SubGoal>` persists** | `htn_integration.rs` still emits `GuiWorkflow { sub_goals: Vec<SubGoal> }`. The two planners (`generate_gui_workflow` and `plan_gui_workflow_via_llm`) were supposed to collapse into ONE `GuiPlanner` trait emitting `GoalTree`. They are both still standalone, still flat, still unvalidated. | Implement `GuiPlanner` trait. Refactor both rule and LLM paths behind it. Emit `GoalTree` with prerequisite/fallback structure. |
| 🟠 **HIGH** | **`EnvironmentGrounder` is a noop placeholder** | `environment_grounder.rs:89-104` — `NoopEnvironmentGrounder` returns empty facts. P2 was supposed to provide real X11/Proc/RandR reads. The planner currently gets zero environmental context. | Implement `XcbGroundImpl`, `ProcGroundImpl`, etc. per KRIA_UPGRADE.md §3.2. Cap at 32 facts, 10s TTL. |
| 🟠 **HIGH** | **`DeterministicOutput` verifier is unimplemented for `TerminalOutput` and `ActiveEditorBuffer`** | `execution_verifier_impl.rs:298-317` — both variants return `verified: false` with "requires shell/visual integration". This is the verifier class that would end the "type_text falsely succeeds" bug class. | Implement `TerminalOutput` by reading from a captured shell pipe or terminal scrollback file. Implement `ActiveEditorBuffer` via OCR on the editor region or via LSP/file-watcher. |
| 🟡 **MED** | **Feature flag `gui_cognition_v2` creates parallel universes** | When enabled, new traits compile but old paths remain dominant. When disabled, the new code vanishes. There is no migration path — the codebase maintains two routing stacks. | Remove the feature flag. Make the new types unconditional. Use `NoopIntentCompiler` as a backward-compatible default, but wire the trait into the runtime unconditionally. |

---

# 3. IntentCompiler Audit

## Authority Correctness
**FAIL.** The `IntentCompiler` trait contract (`intent_compiler.rs:99-108`) is semantically pure: `text → GuiTaskSpec`. It does NOT read environment, plan, or mutate state. The **implementation** respects this. However, **authority is void because the result is never consumed by downstream layers.**

At `loop_engine/mod.rs:2398-2446`, the `RuleIntentCompiler` produces a `GuiTaskSpec`, logs it, and if there are no ambiguities, **falls through** to the old routing. The `spec` variable goes out of scope. The planner never sees it. The `GuiTaskSpec` is architecturally correct but operationally irrelevant.

## Semantic Quality
**MIXED.** The `LlmIntentCompiler` prompt (`intent_compiler_llm.rs:15-105`) is well-structured, with clear examples and ambiguity detection instructions. However:
- **No grammar constraint** — relies on polite asking, not structural enforcement.
- **No prompt-injection sanitization** — user text is pasted raw into the user message at line 412.
- **Silent catch-all mapping** — unknown `success_type` → `UserConfirmed`, unknown `ambiguity` → `MultipleTargetsPossible`. This is fail-open.

## Boundedness
**PARTIAL.** The `RuleIntentCompiler` is bounded (<5ms, no LLM). The `LlmIntentCompiler` is bounded by `max_tokens: 512` and `temperature: 0.1`, but there is no timeout on the `block_on(backend.chat(...))` call. An LLM backend stall could block the executor thread indefinitely.

## Ambiguity Handling
**CORRECT IN PRINCIPLE, FAIL IN PRACTICE.** The compiler correctly surfaces `ClarifyRequest` when ambiguities are present. The `build_clarify_request` function produces sensible options. **But** because the compiler is only wired under `#[cfg(feature = "gui_cognition_v2")]`, ambiguity handling is not active in the default build.

## Parser Architecture
**BRITTLE.** `RuleIntentCompiler` re-implements the exact same substring anti-pattern the architecture sought to replace (`to_ascii_lowercase()`, `contains("gedit")`, `starts_with("open ")`). This is not a "fallback parser" — it is the **same parser with more structure**. The LLM path is the only truly semantic path, and it is not wired.

## Safety
**CONCERNING.**
- `parse_verb` at `intent_compiler_llm.rs:497` does not validate against an allow-list; it guesses based on prefixes.
- `extract_app` at line 519 hardcodes app names ("gedit", "code", "firefox") instead of querying a system app database.
- The prompt contains the literal string `"Now process this input:"` followed by untrusted user text — a classic prompt-injection surface.

---

# 4. loop_engine Integration Audit

## Routing Coherence
**BROKEN.** The routing logic at `loop_engine/mod.rs:2517-2654` has THREE overlapping authorities:
1. `TurnGate` (`should_route_to_gui_executor`)
2. `requires_gui_automation(&last_user_text)` — old keyword matcher
3. `generate_gui_workflow(task_id, user_text)` — another old keyword matcher inside the coordinator

The architecture specified: `TurnGate → IntentCompiler → GuiPlanner → GuiExecutor`.  
The reality is: `TurnGate → (IntentCompiler → *discarded*) → (old keyword matcher || old keyword matcher) → GuiExecutor`.

## Runtime Correctness
**FRAGILE.** The `IntentCompiler` is called synchronously in the async loop engine. If the LLM path were enabled, `block_on` would stall the event loop. As it stands, only the rule path runs, so it is fast-but-useless.

The compiled `GuiTaskSpec` is not forwarded to `GuiExecutionCoordinator::generate_workflow`, which takes `&IntentEnvelope` and `user_text: &str` but ignores the envelope and passes the raw text to `generate_gui_workflow` (`gui_wiring.rs:124-144`).

## Async Lifecycle
**HAZARDOUS.** `GuiExecutionCoordinator::execute_workflow` (`gui_wiring.rs:147-175`) spawns a heartbeat task that opens a **new backend connection** per workflow (via `YdotoolBackend::new`). The architecture's F2 fix (persistent daemon session) is not implemented.

## Fallback Safety
**DANGEROUS.** When `generate_workflow` returns `None`, the loop falls back to `plan_gui_workflow_via_llm` — an unconstrained LLM JSON planner with NO schema validation (`parse_htn_json` just does `serde_json::from_str`). This is the F1 vulnerability.

## Event Propagation
**ACCEPTABLE.** `log_pipeline_step` calls are present. `StreamEvent` emissions are correct. But there is no `GuiEvent` bus as specified in `GUI_INTELLIGENCE_REVIEW.md` §2.5. Events are ad-hoc strings, not typed `GuiEvent` variants.

---

# 5. ExecutionVerifier Audit

## Boundedness
**INTENDED BUT NOT DEPLOYED.** `BoundedExecutionVerifier` at `execution_verifier_impl.rs:34` has:
- Per-class latency caps (≤500ms)
- No LLM calls
- No replanning
- No retry loops

**But it is never instantiated by the executor.** The old `VerificationEngine` (`htn_executor.rs:1058-1239`) continues to run. It uses `VisualHashVerifier` and `OmniParserCache` — heavy vision-based checks that can exceed the 500ms budget and require GPU resources.

## Semantic Verification Quality
**PARTIALLY IMPLEMENTED, FULLY UNWIRED.**
- `WindowState` — implemented via `GuiBackend::get_active_window()`, correct.
- `FileSystemEffect` — implemented with `std::fs`, correct but blocking.
- `ProcessLaunched` — implemented with `/proc` polling loop. Bounded by `max_wait_ms` but inefficient (reads entire `/proc` every 50ms).
- `DeterministicOutput` — **STUB.** `TerminalOutput` and `ActiveEditorBuffer` always return `verified: false`.
- `OcrTextPresent` — reads from `ocr_cache`, but cache is only populated via explicit `cache_ocr()` call. No automatic population.
- `UserAttested` — correct: always returns `verified: false`.
- `Unverifiable` — correct: always returns `verified: false`.

## Replanning Risk
**PASS (in code), VOID (in deployment).** The verifier code never replans. But since it is not used, this is moot.

## Retry Behavior
**NOT PRESENT IN VERIFIER, PRESENT IN EXECUTOR.** `GuiExecutor` has `BoundedMicroRetries` (`htn_executor.rs:1421, 1941`). The architecture specified that retries live in the executor, not the verifier. This is correct. **However**, the executor retries the OLD `VerificationEngine`, not the new `ExecutionVerifier`.

## Runtime Correctness
- `check_process_launched` uses `std::fs::read_dir("/proc")` in async context without `tokio::fs` or `spawn_blocking`. Blocking syscall in async path.
- `check_file_system_effect` reads files with `std::fs::read`/`read_to_string` — same blocking issue.
- `check_with_timeout` allocates `Box<dyn Future>` on every verify call.

## Verifier Authority Leakage
**NONE.** The verifier code is clean. It does not plan, mutate, or recurse. The leakage is elsewhere: the executor's `VerificationEngine` uses `OmniParserCache` and `VisualHashVerifier`, which touch vision pipelines the verifier was supposed to isolate from.

---

# 6. Runtime & Concurrency Risks

| Risk | Severity | Runtime Impact | Fix |
|------|----------|----------------|-----|
| `block_on(backend.chat(...))` in `LlmIntentCompiler` | 🔴 CRITICAL | Deadlocks or thread starvation under load; blocks Tokio worker for LLM latency (0.5–3s) | Make `IntentCompiler::compile` async; remove `block_on` |
| `std::sync::RwLock` in async verifier | 🔴 CRITICAL | Blocks async executor threads when OCR cache is contended | Use `tokio::sync::RwLock` or `DashMap` |
| `std::fs` blocking calls in async verifier | 🟠 HIGH | `/proc` read_dir, `fs::read`, `fs::metadata` block async threads | Wrap in `tokio::task::spawn_blocking` or use `tokio::fs` |
| Heartbeat task spawns per workflow | 🟠 HIGH | Creates a new TCP/Unix socket connection every workflow instead of persistent session (F2) | Implement `SessionBegin` + connection reuse per KRIA_UPGRADE.md §4.6 |
| `check_process_launched` polls `/proc` in loop | 🟠 HIGH | 10 iterations of full `/proc` scan for a 500ms wait; O(PIDs) per verify | Use `inotify` on `/proc/<pid>/comm` or single read with timeout |
| `Box::pin` allocation per verify timeout | 🟡 MED | Heap allocation on every `verify()` call for the timeout wrapper | Use `tokio::time::timeout` directly on named async fn without boxing |
| Hardcoded `sleep(1500)` after `open_application` | 🟡 MED | Fixed 1.5s blind wait regardless of app readiness; wastes time on fast apps, fails on slow ones | Replace with event-driven readiness probe (poll window state every 250ms up to cap) |
| Hardcoded `sleep(1000)` after `click_element` | 🟡 MED | Unnecessary 1s penalty on every click→type sequence | Replace with focus-state probe or eliminate if daemon guarantees focus sync |
| No VRAM budget enforcement in verifier | 🟡 MED | `VerificationEngine` uses `OmniParserCache` which may trigger GPU load; `BoundedExecutionVerifier` was supposed to be CPU-only | Enforce CPU-only verifier; gate OmniParser behind explicit GPU lease check |

---

# 7. Maintainability & Future Scaling Risks

## What Will Become Painful in 6 Months

1. **Two verification stacks.** The old `VerificationEngine` (visual hash + OmniParser) and new `ExecutionVerifier` (semantic classes) will diverge. Future contributors will patch one and forget the other. The old one must be deleted.

2. **Flat `Vec<SubGoal>` vs `GoalTree` debt.** Every new planner feature (prerequisites, fallback subtrees, safe abort branches) requires ad-hoc fields on `SubGoal` and `GuiWorkflow`. The `GoalTree` refactor will be a breaking change that touches `htn_executor.rs`, `htn_integration.rs`, `gui_wiring.rs`, and all tests. Delaying it makes the refactor cost exponential.

3. **Feature flag maintenance.** `gui_cognition_v2` means CI must test both configurations. The flag is empty (`[]`) — it does not even control dependencies. It adds conditional compilation complexity for zero runtime benefit.

4. **Knowledge graph entanglement.** `world_model/store.rs` has 595 lines of SQLite schema, migration, FTS5, archiving, and Bayesian logic. It looks impressive but violates the architecture. If it is not deleted now, future contributors will start querying it from the GUI path, creating the exact "symbolic world model inflation" the architecture banned.

## Weak Abstractions
- `GuiExecutionCoordinator::generate_workflow` takes `&IntentEnvelope` but ignores it. The parameter is a lie.
- `RuleIntentCompiler` is not a "rule compiler" — it is keyword matching with extra types. The abstraction name creates false confidence.
- `UncertaintyEngine` vs `UncertaintyGovernor` — the wrong abstraction was built. The name similarity will confuse future readers.

## Dangerous Naming
- `NoopIntentCompiler` — sounds harmless, but it is the default when the feature flag is off. "Noop" implies "does nothing," but it returns a spec that bypasses ambiguity handling.
- `VerificationEngine` vs `ExecutionVerifier` — two different things with similar names. One is old, one is new. Renaming the old one to `LegacyVisualVerifier` would prevent accidental misuse.

## Layers That May Drift
- **Planner/executor boundary** — `htn_executor.rs` has `InjectRecovery { subtree: Vec<SubGoal> }` which is PRA injection, but without `FailureSignature`/`BranchIdentity` spiral prevention (F7). When `GoalTree` lands, this recovery logic must be rewritten.
- **Grounder/planner boundary** — `EnvironmentGrounder` is currently a noop. When P2 lands, the planner must consume `OperationalFacts`. The current planner has zero fact consumption plumbing.

---

# 8. Overengineering Detection

| Trap | Evidence | Verdict |
|------|----------|---------|
| **Symbolic world model** | `world_model/store.rs` — SQLite S-P-O triples, FTS5, Bayesian merge, archival tables, 595 lines | **OVERENGINEERING.** Delete or reduce to RAM-only typed cache. |
| **Vector DB of UI semantics** | `world_model/store.rs` contains `world_facts_fts` virtual table | **OVERENGINEERING.** FTS5 is a search engine, not a cache. GUI facts need no full-text search. |
| **Multi-agent swarms** | Not present in current code | **PASS.** |
| **Autonomous self-rewriting planner** | Not present in current code | **PASS.** |
| **Unbounded ReAct fallback** | `plan_gui_workflow_via_llm` is unconstrained LLM JSON → flat `Vec<SubGoal>` with no schema validation | **PRESENT.** The old LLM planner is exactly the unbounded ReAct fallback the architecture rejected. |
| **Always-on VLM** | `VerificationEngine` uses `OmniParserCache` globally | **PARTIAL.** OmniParser is not "always-on" in the new verifier, but the OLD verifier still forces it. |
| **System admin uncertainty engine** | `uncertainty/mod.rs` plans `systemctl`, `top`, `ping` diagnostics | **OVERENGINEERING.** This is a completely different domain (server ops) shoehorned into the GUI cognition stack. |
| **Diagnostic command generation** | `UncertaintyEngine::plan_diagnostics` generates executable commands | **AUTHORITY VIOLATION.** Uncertainty layer should score and escalate, never generate steps. |

---

# 9. Final Recommendation

## Is implementation quality strong enough to continue?
**NO.** The traits compile, but the runtime path is unchanged. Phase 1 did not deliver the promised architecture.

## Should Phase 2 start now?
**NO.** P2 (`EnvironmentGrounder`) depends on P1b integration working. P1b is theater. Building P2 on top would create more orphaned modules.

## What MUST be refactored before continuing?

In priority order:

1. **Wire `IntentCompiler` as the SOLE routing authority for GUI tasks.** Delete `requires_gui_automation`. Pass `GuiTaskSpec` through to the planner. This is a 1-day refactor but unblocks everything.
2. **Replace `VerificationEngine` with `BoundedExecutionVerifier` in `GuiExecutor`.** Delete or rename the old verifier. Wire the new one. This is the highest operational impact fix.
3. **Make `IntentCompiler::compile` async.** Remove `block_on`. This is a safety fix.
4. **Add GBNF grammar constraint to `LlmIntentCompiler`.** Use existing `chat_with_grammar`. Remove silent catch-all mapping. This is a robustness fix.
5. **Delete `world_model/store.rs` knowledge graph.** Replace with a 50-line typed cache. This prevents future architectural rot.
6. **Delete or isolate `uncertainty/mod.rs` system diagnostics.** Build `UncertaintyGovernor` as specified. Do not let server-admin code leak into the GUI path.

## What is the SINGLE biggest implementation weakness now?

**The IntentCompiler is called but its result is discarded.** This is worse than not having an IntentCompiler. It creates the illusion of bounded semantic routing while preserving every old keyword-matcher bypass. It is architectural theater — the most dangerous kind of technical debt because it misleads reviewers and future maintainers.

## What is the SINGLE strongest implementation decision?

**The `ExecutionVerifier` trait design in `execution_verifier.rs`.** The `Verifiability` enum is exactly right: seven bounded classes, honest `Unverifiable`, no replanning surface in the return type. If this trait were wired into the executor, it would end the false-success bug class. The design is sound; the deployment is missing.

---

## Honest Summary

KRIA's **mechanical substrate** remains excellent. The **trait contracts** for P1 and P4 are well-designed. But **integration is theater**: new modules compile, old paths dominate, and the architecture diagram on paper does not describe the runtime reality.

**Do not start Phase 2 until P1b is real.** That means: `IntentCompiler` output must flow into `GuiPlanner` input, and `GuiPlanner` must be the ONLY step-list producer. Until then, every additional module is another orphan.

The good news: the fixes are small and surgical. The bad news: they require admitting that Phase 1 is not finished.

---

*Audit generated from live codebase analysis. All line numbers and file paths verified against `/media/obaid/SSD/KRIA/crates/kria-core/src/agent/`.*
