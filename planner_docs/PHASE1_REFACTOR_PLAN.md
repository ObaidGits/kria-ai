# KRIA Phase 1 Runtime Authority Refactor Plan

## 1. Refactor Plan

| Priority | Refactor | Why Required | Files Impacted | Risk |
|----------|----------|--------------|----------------|------|
| P0 | Make `IntentCompiler::compile` async | Removes `block_on()` deadlock hazard; required before any async wiring can be correct | `intent_compiler.rs`, `intent_compiler_llm.rs`, `loop_engine/mod.rs` | LOW — signature change propagates to 3 call sites |
| P0 | Add `chat_with_grammar` to `LlmBackend` trait | GBNF constraints require backend support; default impl falls back to `chat` so other backends are unaffected | `llm/mod.rs`, `llm/local.rs` | LOW — trait extension with default body |
| P0 | Enforce GBNF + fail-closed in `LlmIntentCompiler` | Eliminates F1 freeform-JSON vulnerability; unknown enum variants now produce `ClarifyRequest` instead of silent `UserConfirmed` | `intent_compiler_llm.rs` | LOW — replaces `chat` with `chat_with_grammar`, replaces catch-alls with `return Err(...)` |
| P0 | Remove `requires_gui_automation()` substring bypass | This function is a parallel routing authority that overrides `IntentCompiler`; its existence violates "ONE routing authority" | `htn_integration.rs`, `gui_wiring.rs`, `loop_engine/mod.rs` | LOW — pure deletion |
| P0 | Wire `GuiTaskSpec` as SOLE planner input | `generate_workflow` currently ignores `IntentEnvelope` and re-parses raw `user_text`; must consume the compiled `GuiTaskSpec` instead | `gui_wiring.rs`, `htn_integration.rs`, `loop_engine/mod.rs` | MED — rewrites planner matching logic to use typed spec |
| P0 | Create `GuiPlanner` trait, unify rule + LLM paths | Two standalone planner functions still exist; they must collapse behind ONE trait with `plan(&self, task_id, &GuiTaskSpec) -> Option<GuiWorkflow>` | `gui_planner.rs` (new), `htn_integration.rs`, `gui_wiring.rs`, `loop_engine/mod.rs` | MED — new trait + two impls, but no GoalTree changes |
| P0 | Replace `VerificationEngine` with `BoundedExecutionVerifier` in `GuiExecutor` | Old verifier still runs visual-hash + OmniParser checks; new bounded verifier must become the actual runtime | `htn_executor.rs` | MED — field swap + type bridge (`VerificationType` → `Verifiability`) |
| P1 | Fix async hazards in `BoundedExecutionVerifier` | `std::sync::RwLock` and `std::fs` calls block Tokio threads | `execution_verifier_impl.rs` | LOW — mechanical replacements |
| P1 | Remove `gui_cognition_v2` feature flag | Feature flag creates parallel universes; new modules must be unconditional | `mod.rs`, `Cargo.toml` | LOW — removes `#[cfg]` guards |
| P2 | Isolate `uncertainty/mod.rs` admin diagnostics | Module is server-admin code, not GUI cognition; rename to prevent confusion and remove from GUI path | `uncertainty/mod.rs` | LOW — rename + doc comments |
| P2 | Strip `world_model/store.rs` to bounded RAM cache | SQLite S-P-O knowledge graph violates "no symbolic world model" principle | `world_model/store.rs` | MED — schema rewrite, but data loss is acceptable (cache only) |

---

## 2. Authority Graph

### BEFORE (Broken)

```text
User Text
    ├─→ TurnGate ──→ should_route_to_gui_executor(confidence gate)
    │                    ↑
    │                    └─ requires_gui_automation(keyword matcher) ──OVERRIDE
    │
    ├─→ IntentCompiler ──→ GuiTaskSpec ──→ (LOGGED, THEN DISCARDED)
    │
    └─→ generate_gui_workflow(user_text) ──→ keyword matching AGAIN
              ↓
         plan_gui_workflow_via_llm(user_text) ──→ unconstrained LLM JSON
              ↓
         GuiExecutor::verification = VerificationEngine (old visual-hash stack)
              ↓
         ReAct loop (fallback for everything else)
```

**Problems:**
- Three routing authorities (TurnGate confidence, keyword matcher, IntentCompiler)
- `GuiTaskSpec` never reaches planner
- Raw prompt parsed multiple times by different matchers
- Old verifier still active

### AFTER (Corrected)

```text
User Text
    ↓
TurnGate (confidence + hazard gating only)
    ↓
IntentCompiler ──→ GuiTaskSpec
    ↓
GuiPlanner (ONE trait, rule impl || LLM impl)
    ↓
GuiWorkflow
    ↓
GuiExecutor
    ↓
ExecutionVerifier (BoundedExecutionVerifier ONLY)
    ↓
Result / HITL Escalation
```

**Properties:**
- Single routing authority: `IntentCompiler` output + `TurnGate` confidence
- No substring matching anywhere in the chain
- `GuiTaskSpec` is the only data passed between Compiler → Planner → Executor
- Verifier is bounded, semantic, never replans

---

## 3. Exact Runtime Violations Fixed

| Violation | Root Cause | Fix Applied | Architectural Benefit |
|-----------|------------|-------------|----------------------|
| IntentCompiler output discarded | `loop_engine` compiled intent but fell through to old routing | `GuiTaskSpec` stored in local variable and forwarded to `GuiExecutionCoordinator::generate_workflow` | Semantic normalization becomes actual runtime authority |
| `requires_gui_automation()` keyword bypass | `\|\|` expression at `loop_engine:2521` allowed substring override | Function deleted; `should_route_to_gui_executor` only checks `IntentCompiler` result + confidence | No parallel routing authority |
| `generate_workflow` re-parses raw text | `gui_wiring.rs:130` passes `user_text` to keyword matcher | `generate_workflow` now takes `&GuiTaskSpec`; planner uses typed fields | No raw prompt re-parsing downstream |
| `BoundedExecutionVerifier` unwired | `GuiExecutor` still constructs `VerificationEngine` | Field changed to `Arc<dyn ExecutionVerifier>` with `BoundedExecutionVerifier::new()` as default | Intent-level bounded verification is the only verifier |
| `block_on` inside Tokio runtime | `LlmIntentCompiler.call_llm` called `runtime_handle.block_on` | Trait changed to `async fn compile`; impls are naturally async | No thread starvation or deadlock |
| Freeform JSON without grammar | `backend.chat()` with polite prompt only | `backend.chat_with_grammar()` with JSON schema constraint | Structural guarantee eliminates hallucinated enum values |
| Silent catch-all mapping | Unknown `success_type` → `UserConfirmed` (auto-passes) | Unknown variants return `Err(ClarifyRequest { "Malformed intent classification" })` | Fail-closed instead of fail-open |
| `std::sync::RwLock` in async context | `ocr_cache` used `std::sync::RwLock` | Changed to `tokio::sync::RwLock` | No executor thread blocking |
| Blocking `std::fs` in async verifier | `metadata()`, `read_dir()`, `read()` called directly | Wrapped in `tokio::task::spawn_blocking` | No executor thread blocking |
| Feature flag parallel universe | `gui_cognition_v2` gated all new code | Flag removed; modules unconditional | Single code path; no dual-stack maintenance |

---

## 4. Code-Level Refactor Strategy

### A. `IntentCompiler` Async Conversion

**Trait change:**
```rust
// BEFORE
pub trait IntentCompiler: Send + Sync {
    fn compile(&self, user_text: &str, intent: &IntentEnvelope) -> Result<GuiTaskSpec, ClarifyRequest>;
}

// AFTER
#[async_trait::async_trait]
pub trait IntentCompiler: Send + Sync {
    async fn compile(&self, user_text: &str, intent: &IntentEnvelope) -> Result<GuiTaskSpec, ClarifyRequest>;
}
```

**Impl changes:**
- `NoopIntentCompiler`: wrap body in `async { ... }`
- `RuleIntentCompiler`: wrap body in `async { ... }`
- `LlmIntentCompiler`: remove `runtime_handle` field and `block_on` call; call `self.backend.chat_with_grammar(...).await` directly

**Call site in `loop_engine`:**
```rust
let spec = compiler.compile(&last_user_text, &turn_gate_plan.intent).await;
```

### B. `LlmBackend` Trait Extension

**Trait addition:**
```rust
async fn chat_with_grammar(
    &self,
    messages: &[ChatMessage],
    json_schema: serde_json::Value,
    temperature: f32,
    max_tokens: u32,
) -> anyhow::Result<LlmResponse> {
    // Default: delegate to chat without grammar
    self.chat(messages, None, temperature, max_tokens).await
}
```

**`LocalBackend` override:** Move existing `chat_with_grammar` body into the trait impl block.

### C. Fail-Closed Mapping in `convert_llm_output`

**Before:**
```rust
other => Verb::Other(other.to_string()),           // Accepts anything
other => TargetRef::Element(format!("{}:{}", other, t.value)), // Accepts anything
other => ContentClass::Generated { hint: other.to_string(), ... }, // Accepts anything
other => PrereqHint::AppOpen(format!("{}:{}", other, p.value)), // Accepts anything
other => SuccessHint::UserConfirmed,               // DANGEROUS: auto-passes
other => Ambiguity::MultipleTargetsPossible,       // Silent default
```

**After:**
```rust
other => return Err(ClarifyRequest {
    question: "Malformed intent classification".to_string(),
    options: vec!["Try rephrasing".to_string()],
}),
```

All five catch-all arms become `return Err(...)`. The compiler now **rejects** unknown values instead of silently normalizing them.

### D. `GuiPlanner` Trait Introduction

**New file `gui_planner.rs`:**
```rust
#[async_trait::async_trait]
pub trait GuiPlanner: Send + Sync {
    async fn plan(&self, task_id: &str, spec: &GuiTaskSpec) -> Option<GuiWorkflow>;
}
```

**`RuleGuiPlanner`** — adapted from existing `generate_gui_workflow`:
- Reads `spec.primary_verb` and `spec.targets` instead of substring matching
- Returns `None` for non-GUI verbs (e.g., `Verb::Other` without GUI targets)
- Preserves existing `build_text_editor_workflow`, `build_click_button_workflow`, etc.

**`LlmGuiPlanner`** — adapted from `plan_gui_workflow_via_llm`:
- Accepts `&GuiTaskSpec` instead of `&str`
- Serializes `GuiTaskSpec` to JSON as LLM context
- Asks LLM to emit `GuiWorkflow` JSON with schema constraint
- Parses with `parse_htn_json` (already exists)

**`GuiExecutionCoordinator`** change:
```rust
// BEFORE
pub fn generate_workflow(&self, task_id: &str, _intent: &IntentEnvelope, user_text: &str) -> Option<GuiWorkflow>

// AFTER
pub async fn generate_workflow(
    &self,
    task_id: &str,
    spec: &GuiTaskSpec,
    planner: &dyn GuiPlanner,
) -> Option<GuiWorkflow> {
    planner.plan(task_id, spec).await
}
```

### E. `ExecutionVerifier` Deployment

**`GuiExecutor` field swap:**
```rust
// BEFORE
verification: VerificationEngine,

// AFTER
verifier: Arc<dyn ExecutionVerifier>,
```

**Constructor change:**
```rust
// BEFORE
verification: VerificationEngine::new(),

// AFTER
verifier: Arc::new(BoundedExecutionVerifier::new()),
```

**Verification call site:**
```rust
// BEFORE (in execute_workflow loop)
self.verification.verify(&sub_goal.verify, &current_window).await

// AFTER
let verifiability = verification_type_to_verifiability(&sub_goal.verify);
self.verifier.verify(&verifiability).await
```

**Bridge function:** A small private `fn verification_type_to_verifiability(v: &VerificationType) -> Verifiability` maps the old step-level types to the new semantic classes. This is a temporary bridge until `GoalTree` replaces `SubGoal` in Phase 2.

### F. `BoundedExecutionVerifier` Async Safety

1. `ocr_cache: std::sync::RwLock<...>` → `ocr_cache: tokio::sync::RwLock<...>`
2. `std::fs::metadata(path)` → `tokio::task::spawn_blocking(move || std::fs::metadata(path)).await`
3. `std::fs::read_dir("/proc")` → `tokio::task::spawn_blocking(|| std::fs::read_dir("/proc")).await`
4. `std::fs::read(path)` → `tokio::task::spawn_blocking(move || std::fs::read(path)).await`
5. Remove `Box::pin` in `check_with_timeout`; use named async fn with `tokio::time::timeout` directly

### G. Feature Flag Removal

In `mod.rs`, remove all `#[cfg(feature = "gui_cognition_v2")]` guards around:
- `intent_compiler`
- `intent_compiler_llm`
- `environment_grounder`
- `execution_verifier`
- `execution_verifier_impl`

In `Cargo.toml`, keep the feature flag definition for backward compatibility (it can remain as a no-op), but remove all `#[cfg]` usage in the source.

---

## 5. Safety & Boundedness Verification

### Hidden Recursion Check
- `IntentCompiler` → no recursive calls to planner/executor/verifier ✓
- `GuiPlanner` → no recursive calls to compiler/executor/verifier ✓
- `GuiExecutor` → bounded retry loop (fixed `max_attempts`), no recursion into planner ✓
- `ExecutionVerifier` → no calls to compiler/planner/executor ✓
- No `InjectRecovery` paths that loop back to `IntentCompiler` ✓

### Authority Duplication Check
- Only ONE function decides GUI routing: `IntentCompiler::compile` + confidence gating ✓
- No `requires_gui_automation` anywhere ✓
- No raw prompt re-parsing in planner or executor ✓

### Planner Drift Check
- `GuiPlanner` trait does NOT execute (returns `Option<GuiWorkflow>`, never calls tool registry) ✓
- `GuiExecutor` does NOT plan (accepts pre-built `GuiWorkflow`, never modifies it) ✓
- `validate_sub_goals` ensures immutability ✓

### Verifier Replanning Check
- `BoundedExecutionVerifier::verify` returns `VerifyOutcome`; no `WorkflowResult`, no `GuiWorkflow` ✓
- No `InjectRecovery`, no `HITLEscalation`, no retry logic inside verifier ✓
- `check_user_attested` and `check_unverifiable` always return `verified: false` ✓

### Async Hazard Check
- No `block_on` in library code ✓
- No `std::sync::Mutex`/`RwLock` across await points ✓
- No blocking `std::fs` calls in async context ✓

---

## 6. Final Readiness Verdict

### Is Phase 1 now REAL after refactor?
**YES.** The runtime chain is:
```text
IntentCompiler → GuiTaskSpec → GuiPlanner → GuiWorkflow → GuiExecutor → ExecutionVerifier
```
Every link is wired. No authority bypasses. No discarded outputs.

### Is runtime authority finally coherent?
**YES.** `IntentCompiler` is the SOLE semantic authority. `TurnGate` provides confidence gating only. `GuiPlanner` produces the only workflow. `GuiExecutor` executes without planning. `ExecutionVerifier` verifies without replanning.

### Is KRIA ready for Phase 2 after this?
**YES.** Phase 2 (`EnvironmentGrounder`) can now be built safely because:
- P1a (IntentCompiler) is real and wired
- P1b (loop_engine integration) is real and wired  
- P4 (ExecutionVerifier) is real and wired
- The trait boundaries are clean and stable

### What remains intentionally deferred?
- **GoalTree sophistication** — deferred to Phase 2/3 as planned
- **EnvironmentGrounder real impl** — deferred to Phase 2 (currently `Noop`)
- **UncertaintyGovernor** — deferred to later phase (admin diagnostics isolated but not replaced)
- **World Model** — bounded RAM cache is sufficient; symbolic knowledge graph deleted
- **PRA Loop / recovery intelligence** — scaffolding exists but not activated in the bounded path
