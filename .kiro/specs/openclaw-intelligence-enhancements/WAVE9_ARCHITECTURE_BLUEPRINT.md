# Wave 9 — Capability Synthesis: Production Architecture Blueprint

> **Status:** Architecture research only. No code changed. This document is the
> implementation blueprint for the next implementation session.
> **Method:** Principal-architect review of the *real* code, spec (R7/R11/R16/R21/R27),
> tasks.md, design.md, and the WAVE9_HANDOFF.md against KRIA's long-term vision.
> **Verdict up front:** current impl scores **58/100**; Wave 9 is **~45% complete**
> against the full R7 vision. The path forward is a **Capability-Graph IR over the
> existing HTN runtime**, not a new engine and not raw code-gen first.

---

## 0. Executive Summary

Wave 9 today is a **deterministic keyword→primitive synthesizer**. It is correct,
safe, honest, and flag-gated — genuinely production-safe for what it does — but it
is a *foundation*, not the R7 "engineer a capability" vision. It can only produce
linear pipelines of 11 pure text primitives, over a single `{text}` input, with no
versioning, no repair, no multi-input, no real NL understanding, and no generated
code (hence no sandbox needed yet).

The single most important architectural decision for Wave 9 is: **what does
synthesis produce?** Options range from raw Rust/Python source (needs a compiler +
sandbox + a reliable code model KRIA does not have) to a typed **Capability-Graph
Intermediate Representation** (IR) that composes audited primitives and *installed
capabilities* into a validated DAG, emitted into KRIA's existing HTN runtime.

**Recommendation:** adopt the **Capability-Graph IR** as the synthesis artifact
(Solution D+F fused). It is pure, deterministic, testable without a model, reuses
the HTN executor / planner / permission / CKB / event bus that already exist, and
degrades gracefully: an LLM *proposes* an IR (Tier-2, generate→validate→repair),
a validator *proves* it type-checks and its effects union is permission-gated, and
raw code-gen (Tier-3) is deferred behind its own flag until a reliable code model
exists. This turns "code generation" (unsafe, model-blocked) into "graph
composition" (safe, verifiable, shippable now).

---

## PHASE 1 — Current-State Audit (verified against code)

### 1.1 The synthesis runtime path (traced end to end)

```
User goal (chat / tool call)
  → agent loop → tools/capability_dispatch → CapabilityPlatform::acquire_for_goal(goal)
     ├─ flag marketplace_v2 OFF → acquire_for_goal_legacy (provider-order, first success)
     └─ flag ON               → acquire_for_goal_reasoned
          1. recommend(goal, 8)  → gather_catalog() ⊕ CatalogRanker (fused relevance+trust)
          2. if ranked.first() == None
                AND self.synthesis_provider == Some(id)
                AND CapabilitySpecification::from_goal(goal).is_some()
             → synthesize_for_goal(goal, id, corr)          ← THE Wave-9 entry
          3. else: Decision Record → dep check → provider.acquire(chosen)
                   → trust gate → refresh → CKB → events

synthesize_for_goal(goal, syn_id, corr):
  emit(Acquire/Started)
  → Decision Record { path=Generate, class=Generation, chosen=None }   ← (H bug: chosen never set)
  → provider.acquire(AcquireRequest{ hint=goal, capability_id=None })
       SynthesisProvider::acquire:
         CapabilitySpecification::from_goal(goal)               ← primitives::infer_pipeline_from_goal
            → Vec<primitive names>  (or honest-decline None)
         persist SynthesizedRecord JSON (atomic temp+rename)
         → descriptor_from(spec, installed=true)               (trust="synthesized", effects=["synthesized"])
  → trust gate (TrustPolicy::evaluate) → quarantine on fail
  → refresh() + invalidate_catalog_cache + CKB record_install/record_outcome
  → emit(Acquire/Ok) + emit(Learn/Ok)

Execution (separate turn):
  CapabilityPlatform::execute(req)
    → quarantine gate → provider.execute
        SynthesisProvider::execute:
          load spec → pipeline = spec.pipeline (or [primitive])
          primitives::apply_pipeline(pipeline, args["text"])   ← in-process, pure, sync
          → Value { "result": ... }
    → circuit breaker + CKB record_outcome + emit(Execute/*)
```

### 1.2 Component / type inventory

| File | Symbol | Role |
|------|--------|------|
| `capability/intelligence/primitives.rs` | `KNOWN_PRIMITIVES` (11), `apply_primitive`, `apply_pipeline`, `infer_primitive_from_goal`, `infer_pipeline_from_goal` | The audited primitive vocabulary + deterministic goal→pipeline inference. Pure text ops only. |
| `capability/intelligence/synthesis.rs` | `CapabilitySpecification{ capability_id, name, purpose, primitive, pipeline, family, golden_input, golden_output }`, `CapabilityGapAnalyzer`, `GapResolution{UseExisting,Acquire,Synthesize,Decline}` | Deterministic spec derivation + gap classification. `pipeline: Vec<String>` = the composition model. |
| `capability/acl/synthesis.rs` | `SynthesisProvider{ id, store_dir }`, `SynthesizedRecord` | Neutral provider: acquire=generate+persist, execute=apply_pipeline, describe=list installed, remove=delete. Lowest trust; effects=["synthesized"]. |
| `capability/platform.rs` | `acquire_for_goal_reasoned`, `synthesize_for_goal`, `execute`, `with_synthesis` | Brain-owned orchestration; synthesis fall-through; trust gate; CKB; events. |
| `capability/intelligence/planner.rs` | `DefaultCapabilityPlanner::{compose_linear, io_links, union_effects}`, `SolutionPlan`, `PlanStep` | **The real composition engine** — typed IO chaining, effect union, saga. **Currently NOT used by synthesis** (divergence). |
| `capability/events.rs` | `Stage{Negotiate..Learn,Failure,Cancel}`, `Outcome`, `CapabilityEventBus` | Observability. **No synthesis-specific stages.** |
| `capability/intelligence/mod.rs` | traits `CapabilityKnowledge`, `LifecycleManager`, `CapabilityPlanner`, `EvolutionEngine`, `BenchmarkHarness`, `DecisionRecord`, `ExecutionPath::Generate` | Neutral vocabulary + wiring seams. |
| `capability/intelligence/evolution.rs` | `DefaultEvolutionEngine`, `EvolutionStore`, `AutonomyLevel` | Wave-8 self-improvement. Synthesized caps *participate* but are never *repaired/re-synthesized*. |

### 1.3 What genuinely works (do not regress)

- Deterministic, reproducible spec derivation (same goal → same id/pipeline/golden).
- **Honest-decline**: un-expressible goal → `None`, never a fabricated capability.
- Composition of a linear primitive pipeline ("trim then uppercase then reverse").
- Lowest-trust classification; elevated `["synthesized"]` effect forces permission at execute.
- Atomic persistence (temp+rename); idempotent re-acquire by `capability_id`.
- Full wiring into the reasoned acquisition fall-through + Decision Record + CKB + events.
- Flag default OFF ⇒ byte-identical legacy. Neutrality gate green (no cognition in `acl/*`).

---

## PHASE 2 — Every Remaining Gap (A–L from handoff + newly discovered)

### Carried from the handoff (A–L)
- **A** — "Real generation" is a keyword lookup over 11 text ops. No NL understanding, no model, tiny ceiling.
- **B** — No sandbox for generated code (moot today: no code is generated; blocks Tier-3).
- **C** — Composition is **linear-only**; no DAG, no branch/conditional/loop/parallel.
- **D** — **Mono-input** `{text}` hardcoded; no multi-arg, no non-text modality, no typed schema.
- **E** — Events are **coarse**: synthesis reuses `Stage::Acquire/Learn/Failure`; no `SpecCreated / GenerationStarted / GoldenTestStarted / ValidationFinished` granularity.
- **F** — **No Generate UI** (`cpp_synthesize*` commands + Generate tab absent).
- **G** — **No versioning / repair / optimize / migrate** for synthesized caps (version pinned "1.0.0"; re-synth overwrites; not wired to EvolutionEngine as a repair source).
- **H** — **Provenance incomplete**: Decision Record `chosen` stays `None` even on success; no IR hash, no model id, no policy stamp specific to synthesis.
- **I** — **Trust-laundering latent**: effects declared as a flat `["synthesized"]`, not the *union* of per-node effects; a future escalating node would not widen permission.
- **J** — `SynthesisProvider::catalog()` empty by design; synthesis reached only via the empty-ranking fall-through (works, but implicit — a gap analyzer should be the *explicit* driver).
- **K** — **No in-flight lock**: two concurrent identical goals both generate + both write a Decision Record.
- **L** — Cost/latency of a future LLM path is ungoverned (no budget, no IR cache keyed by goal).

### Newly discovered in this audit (not in the handoff)
- **M — R21 pre-activation smoke is NOT enforced on the live path.** `synthesize_for_goal` (and `acquire_for_goal_reasoned`) go acquire → trust-gate → **activate**, never calling `LifecycleManager::smoke_test`. The golden smoke exists only as a descriptor `extensions["smoke"]` and is exercised only by a *unit test that calls `smoke_test` directly*. R21 mandates Download→Sandbox→Verify→**Smoke**→Activate with "failure at any gate ⇒ quarantine". **This is the most serious correctness gap** — a synthesized (or acquired) capability activates without a real liveness gate on the actual runtime path.
- **N — Two parallel composition representations.** `CapabilitySpecification.pipeline: Vec<String>` (synthesis) vs `SolutionPlan`/`PlanStep` (planner). They will diverge; the planner's typed-IO validation, effect-union, saga, and per-step confidence are *not applied* to synthesized pipelines. Root architectural smell.
- **O — Decision Record explainability broken for synthesis (couples with H).** R16.2 ("answer why did you choose X") cannot be satisfied: `chosen=None`, no candidate list, confidence hardcoded 0.5.
- **P — No timeout/cancel on execution** (`apply_pipeline` runs sync in-process). Fine for pure text; violates R12.1 the moment a node does I/O or calls an installed capability.
- **Q — Golden case only covers text primitives.** `golden_case()` hardcodes inputs by primitive name; no generalization to multi-input or capability nodes.
- **R — No reproducibility manifest.** A synthesized capability's on-disk record has spec + created_at, but no policy version, no primitive-set version, no source-goal hash beyond the id prefix; can't prove "re-synthesizing this goal today yields the same artifact."

---

## PHASE 3 — Root Cause, Impact, Risk (per gap cluster)

The gaps cluster into **five root causes**. Fixing the root cause dissolves the cluster.

### RC-1: "The synthesis artifact is a bag of primitive-name strings."
- **Causes:** C, D, I, N, Q, and half of A.
- **Why the current architecture can't solve it:** `Vec<String>` carries no type info, no per-node effects, no topology beyond order, no inputs beyond `{text}`. You cannot type-check, union effects, branch, or multi-input a list of strings.
- **Long-term impact:** every richer feature (DAG, multi-input, effect safety, planner reuse) requires re-representing the artifact anyway. Building more on `Vec<String>` is throwaway work.
- **Risk / debt:** HIGH. It is the load-bearing wall; postponing it multiplies rework.

### RC-2: "Synthesis bypasses the lifecycle gates it claims to reuse."
- **Causes:** M, and the credibility of the whole R7.1 "identical path" claim.
- **Why:** `synthesize_for_goal` and `acquire_for_goal_reasoned` hand-roll acquire→trust→activate and never call `smoke_test`/verify. The `LifecycleManager` exists but is only invoked in tests.
- **Impact:** R21 unmet on the real path; "verified before activation" is aspirational, not enforced.
- **Risk:** HIGH (safety-adjacent) — a broken capability can activate; a future code-gen node would activate un-smoke-tested.

### RC-3: "Provenance/explainability is a stub on the write path."
- **Causes:** H, O, R.
- **Why:** the Decision Record was written before generation and never back-filled; no artifact hash/version stamped.
- **Impact:** R16.2 (why) and R7/R24 reproducibility unmet; audits can't reconstruct *what* was made or *why*.
- **Risk:** MEDIUM — cheap to fix, but blocks trust/audit stories.

### RC-4: "Generation is keyword matching, not reasoning."
- **Causes:** A, L, and the ceiling on usefulness.
- **Why:** no model is invoked; `infer_pipeline_from_goal` is a static table. This was a *deliberate, honest* choice given the model blocker — but it caps synthesis at 11 text ops.
- **Impact:** synthesis is a demo unless it can reach installed capabilities + a model-proposed graph.
- **Risk:** MEDIUM, and **externally blocked** on a reliable code/tool model. Mitigable by making the *model optional*: deterministic path stays, model path is additive + validated.

### RC-5: "No surface / no lifecycle-after-birth."
- **Causes:** E, F, G, J, K, P.
- **Why:** synthesis was landed backend-only; events/UI/versioning/concurrency were deferred.
- **Impact:** not observable/controllable (R27); can't repair/version/migrate a synthesized cap (R7.3).
- **Risk:** MEDIUM — mostly additive, unblocks the "engineer that maintains its creations" story.

---

## PHASE 4 — Solution Space per Root Cause (multiple options, then a pick)

### RC-1: What should synthesis *produce*? (the pivotal decision)

| # | Approach | Pros | Cons | Complexity | Security | Prod-ready now | KRIA-fit |
|---|----------|------|------|-----------|----------|----------------|----------|
| A | **Pure prompt-engineering** (LLM emits an answer, no artifact) | trivial | not a *capability*; no reuse/verify/version | XS | poor | no | ✗ |
| B | **LLM + templates** (fill a Rust/Py template) | familiar | still host code → needs compile+sandbox+model | L | needs sandbox | no (model) | partial |
| C | **LLM + AST generation** | precise | AST for what language? still code→sandbox | XL | needs sandbox | no | ✗ overkill |
| D | **Typed IR (linear/DAG) over primitives** | pure, verifiable, no model needed | bounded to primitive set unless extended | M | trivial (no code) | **yes** | ✓✓ |
| E | **LLM + DSL** (custom textual DSL) | expressive | new parser/semantics = new surface to secure | L | medium | partial | ✗ (proliferation) |
| F | **Capability-Graph IR: nodes = primitive \| installed-capability \| provider** | reuses installed caps → huge reach; typed edges; effect union; maps to HTN | needs a validator + executor bridge | M–L | trivial→sandbox only for code nodes | **yes (Tiers 0–2)** | ✓✓✓ |
| G | **Code synthesis + verifier loop** (generate→compile→test→repair) | maximal power | needs compiler, sandbox, reliable model (blocked) | XXL | hard | no (blocked) | future |
| H | **Planner→Generator→Validator→Repair loop over IR** | structured, testable, model-optional | orchestration cost | M | good | **yes** | ✓✓✓ |

**Pick: F fused with H, expressed as tiers.** Synthesis produces a **Capability-Graph
IR**. Nodes are `Primitive(name)` or `Capability{provider_id, capability_id}` (and,
later, `CodeNode` gated to Tier-3). Edges are typed (reuse `planner::io_links`
semantics on `inputs`/`outputs`). The IR *is* a `SolutionPlan` (or lowers to one),
so the existing planner validates it and the existing HTN runtime executes it. An
LLM, when available and reliable, only *proposes* an IR that is then validated and
repaired — it never emits host code on the safe path. This dissolves RC-1 (typed,
DAG, multi-input, effect-union), RC-4 (reach = every installed capability, model
optional), and N (one representation).

### RC-2: How to enforce pre-activation verification?

| # | Approach | Verdict |
|---|----------|---------|
| 1 | Call `LifecycleManager::smoke_test` inside `synthesize_for_goal` before activate | **Pick** — smallest change, reuses the real gate, satisfies R21 on the live path. |
| 2 | Move all activation through `LifecycleManager::acquire_verified` | Better long-term, but that method currently duplicates less of the platform pipeline; do it in a follow-up refactor. |
| 3 | Provider self-smokes in `acquire` | ✗ — puts cognition/gates in the Hands; violates R23. |

**Pick 1 now, plan 2 as the convergence target.** Add a smoke gate to *both*
`synthesize_for_goal` and `acquire_for_goal_reasoned` (the latter also lacks it),
with `failure ⇒ quarantine + honest error` (R21).

### RC-3: Provenance/explainability?

**Pick:** (a) build the Decision Record **after** generation with `chosen =
Some((pid, cid))`, the real candidate/rejection lists, and confidence; (b) stamp
the on-disk `SynthesizedRecord` with `ir_hash`, `policy_version`,
`primitive_set_version`, `source_goal_hash`, and (when used) `model_id`; (c) surface
these via `cpp_capability_provenance`. Cheap, high audit value.

### RC-4: Real generation without a reliable model?

**Pick: model-optional, validator-mandatory.** Keep the deterministic
`infer_*_from_goal` as **Tier-1** (always on). Add **Tier-2**: an
`IrProposer` trait; the default impl is deterministic (today's inference lowered to
IR), and an `LlmIrProposer` (behind `synthesis_llm` flag) proposes an IR that MUST
pass the same validator + golden smoke or is repaired once, then declined. The
model can never lower the safety bar because the *validator*, not the model, admits
the artifact. This is honest: with no model, Tier-1 still works; with a flaky model,
bad proposals are rejected, never fabricated.

### RC-5: Surface + after-birth lifecycle?

**Pick:** additive `cpp_synthesize`, `cpp_synthesis_preview`, `cpp_synthesis_events`,
`cpp_capability_versions` commands + a **Generate** tab; wire synthesized caps into
the Wave-8 `EvolutionEngine` so a chronically-failing synthesized cap yields a
**Repair** proposal = *re-synthesize from the stored IR + goal* (deterministic,
reversible). Version on every (re)synthesis; keep prior versions for rollback.

---

## PHASE 5 — Deep Research: what artifact survives the future?

Evaluated against five futures: model changes, provider changes, benchmarking,
versioning/migration, and optimization.

- **Raw Rust/Python** survives none cleanly: tied to a toolchain + a code model; every
  provider/runtime change reopens the sandbox contract.
- **DSL** survives model changes but adds a bespoke parser/semantics KRIA must own forever.
- **Capability-Graph IR** survives all five:
  - *Model change* → only the *proposer* changes; the IR + validator + executor are stable.
  - *Provider change* → nodes reference neutral `(provider_id, capability_id)`; a retired provider fails validation loudly, not silently.
  - *Benchmarking* → each node + the whole graph has golden cases; the graph is measurable.
  - *Versioning/migration* → the IR is `serde` + content-hashable; diffs are structural; migration = re-validate against the current registry.
  - *Optimization* → graph rewrites (fuse two primitives, replace a node with a cheaper family member via `select_in_family`) are pure IR→IR transforms.

**Conclusion:** the synthesis artifact SHALL be a **typed, hashable, serde
Capability-Graph IR that lowers to `SolutionPlan`**. Code (Tier-3) is a *node kind*
inside that graph, gated + sandboxed, never the default artifact.

---

## PHASE 6 — Sandbox Architecture (for the eventual Tier-3 code node)

Generated *code* (not primitive/capability nodes) must never touch the host. Reuse
the **existing OpenClaw Docker + `config/seccomp/kria-seccomp.json` substrate** —
add no new sandbox. Pipeline for a `CodeNode`:

```
propose code (Tier-3, flag synthesis_code, model-gated)
  → static checks: forbidden-syscall/deny-list AST scan, dependency allow-list, size/complexity caps
  → policy scan: declared effects must match static capabilities; no undeclared network/fs
  → build INSIDE the seccomp-bound Docker sandbox (no host build)
  → golden + synthetic tests INSIDE the sandbox (bounded cpu/mem/time; network off by default)
  → benchmark (proxy scores)
  → operator approval (HITL) — mandatory for Tier-3
  → promote: lowest trust tier + lowest resource class; effects = static union
  → activate; every run stays in the sandbox; cleanup + container-leak proof (R24.1/R11.2)
  → versioned; rollback = re-activate prior version
```

**This entire phase is deferred behind its own `synthesis_code` flag** and is an
**honest external blocker** (no reliable code model). Tiers 0–2 ship without it.

---

## PHASE 7 — Multi-Input & Typed Schemas

Replace the hardcoded `{text}` with **descriptor-declared JSON-Schema inputs**
(the descriptor already has an `input_schema` field the marketplace path uses).

- Synthesis SHALL emit a real `input_schema` (object with typed, possibly-nested,
  optional properties) derived from the IR's *source* node inputs.
- `io_modality`/`inputs`/`outputs` SHALL be computed from the graph's boundary
  nodes, not hardcoded — enabling `file`, `image`, `audio`, `csv`, `json`, `binary`,
  `stream` modalities as those primitive/capability nodes exist.
- Argument binding reuses the neutral `arg_gen` (`validate_against_schema`,
  bounded repair) — no new validator.
- Streaming/large inputs reuse `CapabilityOutcome::Stream`.

**Design rule:** a synthesized capability's input contract = the *union of unbound
inputs of its source nodes*; internal edges are satisfied by upstream nodes and are
not part of the public schema. This is exactly what `planner::io_links` already
reasons about — reuse it.

---

## PHASE 8 — Capability Composition (beyond linear)

Current composition is a linear fold. The IR generalizes it to a validated DAG that
lowers into the HTN runtime — **no new executor** (kills planner proliferation, R10).

- **DAG**: nodes + typed edges; validate with `io_links`; reject unlinkable graphs at
  synthesis time (not at execute).
- **Parallel branches**: independent sub-DAGs → HTN parallel steps.
- **Conditional / loop**: represented as guarded edges + a bounded-iteration node;
  lower to HTN control steps. (Ship linear+parallel first; conditional/loop next.)
- **Failure handling / checkpointing / rollback**: reuse the saga structure already
  in `PlanStep.compensation` + `SolutionPlan.plan_reversibility`.
- **Reusable graphs**: a validated IR is itself installable as a named capability →
  becomes a `Capability` node in a larger graph (recursion = real capability reuse).

**Design rule:** synthesis composition MUST go through
`DefaultCapabilityPlanner::compose_linear` (extended to `compose_graph`) so effect
union (R11.1), typed IO (R4.4), and saga (R4.3) are enforced once, everywhere.

---

## PHASE 9 — Self-Improvement of Synthesized Capabilities

Wire synthesized caps into the Wave-8 `EvolutionEngine` as first-class subjects:

- **Repair**: chronic failure (health `consecutive_failures`) → `ProposalKind`
  repair = *re-synthesize from the stored IR + source goal*, re-validate, re-smoke;
  on success, version-bump and swap; keep the old version for rollback.
- **Optimize**: benchmark-driven IR rewrite — replace a node with a cheaper
  same-family capability (`select_in_family`), or fuse adjacent pure primitives.
- **Version / migrate**: every (re)synthesis increments `descriptor.version`; the
  on-disk store keeps a version history; migration = re-validate an IR against the
  current registry (a node whose provider vanished fails loudly → repair proposal).
- **Retire / archive / recover**: reuse `LifecycleManager::{retire,recover}` and the
  reversible-retirement machinery already built in Wave 8.
- **Trust evolution**: adaptive trust from CKB outcomes can *raise* a synthesized
  cap above the floor over time — but never above what its effect union permits.
- **Human corrections**: an operator edit of the IR (Generate tab) = a new version
  with provenance `edited_by=operator`.

**Design rule:** synthesis reuses the *same* evolution/benchmark/health machinery;
the only new thing is "repair = regenerate-from-IR", which is deterministic.

---

## PHASE 10 — Generate-Capability UX (SolidJS, additive)

New `Generate` tab in `CapabilitiesView.tsx`, all additive `cpp_*` (R27):

- **Input**: goal text (+ future: attach sample input file for schema inference).
- **Preview (dry-run)**: `cpp_synthesis_preview` → show the proposed IR as a graph
  (nodes + typed edges), declared effects union, trust tier, golden case — *before*
  install. No activation.
- **Progress + streaming logs**: subscribe to granular `capability:synthesis:*`
  events (spec created → generated → validated → smoked → benchmarked → activated).
- **Reasoning / artifacts**: show the Decision Record (why synthesis, why this IR),
  the generated IR JSON, golden/synthetic test results, benchmark proxy scores.
- **Approval**: HITL modal for elevated effects / Tier-3; reuse `HITLModal.tsx`.
- **After-birth**: version tree, trust, run history, **Rollback**, **Edit IR &
  re-synthesize**, **Retire/Recover** — all via `cpp_*`.
- **Capability graph**: render the IR DAG (reuse any existing graph viz or a light
  DAG component).

---

## PHASE 11 — Real Desktop Validation Campaign

Config authority = SQLite `~/.kria/kria.db` `config` table. Enable
`capability.intelligence.{marketplace_v2, synthesis}`; build + launch desktop
(`DISPLAY=:1 WEBKIT_DISABLE_COMPOSITING_MODE=1 ./target/debug/kria-desktop`),
API `:3001` with bearer `~/.kria/api_token`.

Campaign (each = real generate → install → execute → learn → verify on disk):

1. **Single primitive**: "reverse a string" → `syn_reverse_*`; execute; assert CKB rows.
2. **Composed pipeline**: "trim then uppercase then reverse" → `syn_pipeline_*`; execute stage-by-stage.
3. **Capability node** (post-IR): goal that composes an *installed* cap (e.g. a marketplace `oc_*` tool) + a primitive.
4. **Honest-decline**: "orchestrate a kubernetes cluster" → declines; assert no artifact, no fabricated success.
5. **Multi-input** (post-schema): a two-argument goal; assert generated `input_schema`.
6. **Smoke-gate failure** (fault injection): force a golden mismatch → assert quarantine, no activation.
7. **Repair**: force chronic failure → EvolutionEngine repair proposal → apply → version bump → rollback.
8. **Provenance/why**: query Decision Record; assert `chosen=Some`, ir_hash, policy version.
9. **UI**: Generate tab preview → approve → progress events → run → rollback (Wave-12 webview harness caveat noted below).

**Honest blocker:** `cpp_*` are Tauri IPC, not HTTP; headless GUI-click automation
is unavailable → full UI-click validation is **Wave 12** scope. Backend command
paths ARE validatable now via the local API / integration tests.

---

## PHASE 12 — Adversarial Self-Review (break the design)

- **"IR is just Vec<String> with extra steps."** → No: nodes are typed, carry
  per-node effects + IO types, form a DAG, and reference installed capabilities.
  It lowers to the *existing* `SolutionPlan`; it is not a parallel model — it
  *replaces* `pipeline: Vec<String>` and *feeds* the planner.
- **"You're building a second planner/executor."** → No: the IR *lowers into*
  `DefaultCapabilityPlanner` + the HTN runtime. Add zero executors. This is the
  explicit anti-proliferation guardrail (R10).
- **"Effect union could still be laundered."** → Mitigated: union computed from
  per-node declared effects via `union_effects`; permission evaluated on the whole
  graph at max risk (R11.1) *before* activation; smoke gate enforced.
- **"Model unreliability leaks fabrication."** → Mitigated: the validator + golden
  smoke, not the model, admit an artifact; no valid IR ⇒ honest-decline.
- **"Coupling to primitives."** → The `NodeOp::Capability` variant removes the
  primitive ceiling: reach = every installed capability. Primitives are just the
  always-available, zero-trust-cost leaf set.
- **"Concurrency."** → In-flight lock keyed by `capability_id` in the provider;
  idempotent re-acquire already returns the existing artifact.
- **Remaining honest limits:** Tier-3 code-gen (no model), full GUI-click validation
  (Wave 12). Both are flagged + documented, not faked.

No superior structure found after iteration: the IR-over-HTN design minimizes new
surface, maximizes reuse, and degrades gracefully. **This is the recommended
architecture.**

---

## FINAL OUTPUT

### 1. Current implementation score: **58 / 100**

| Dimension | Score | Note |
|-----------|------:|------|
| Safety (trust floor, honest-decline, flag-off parity, neutrality) | 9/10 | Strong; only gap = missing live smoke gate. |
| Correctness of what exists | 8/10 | Deterministic, self-consistent, tested. |
| Wiring / integration | 8/10 | Real fall-through + CKB + events + Decision Record. |
| Verification on live path (R21 smoke) | 2/10 | **Not enforced** in `synthesize_for_goal`/`acquire_for_goal_reasoned`. |
| Generation power (R7.1/A) | 4/10 | 11 text primitives only; no model, no capability reach. |
| Composition (R4/C) | 4/10 | Linear only; not through the planner. |
| Multi-input / schemas (D) | 2/10 | Hardcoded `{text}`. |
| Provenance / explainability (R16/H) | 3/10 | Decision Record `chosen=None`; no artifact hash. |
| Versioning / repair / evolution (R7.3/G) | 3/10 | Participates in health only; no repair/version. |
| Observability granularity (E) | 3/10 | Reuses coarse stages. |
| UI / UX (R27/F) | 0/10 | None. |
| Sandbox for code (R11.4/B) | n/a | Correctly moot (no code); Tier-3 deferred. |

### 2. Remaining Wave 9 completion: **~45%** of the full R7 vision
(Foundation + safe scaffold + composition done; IR, capability-node reach,
multi-input, live smoke, provenance, versioning/repair, events, UI, Tier-3 remain.)

### 3–7. Issues, root causes, solutions, comparison, recommendation
See PHASE 2 (issues A–R), PHASE 3 (root causes RC-1…RC-5), PHASE 4 (option matrices +
picks). **Recommended architecture: Capability-Graph IR (option F+H) lowering into the
existing `DefaultCapabilityPlanner` + HTN runtime; model-optional proposer; mandatory
validator + golden smoke; Tier-3 code node deferred behind `synthesis_code`.**

### 8. Exact Implementation Roadmap (dependency-ordered)

> Each step: `cargo check -p kria-core && -p kria-desktop`, run capability tests,
> keep neutrality gate green, `cargo fmt` + `cargo clippy` on touched files.
> Every step ships behind the existing `synthesis` flag (Tier-3 behind a new
> `synthesis_code` flag) ⇒ flag-off parity preserved.

- **W9-R1 — Capability-Graph IR (pure, no model).** New
  `capability/intelligence/capability_graph.rs`: `NodeOp{ Primitive(String),
  Capability{provider_id, capability_id} }`, `GraphNode{ id, op, inputs, outputs,
  effects }`, `CapabilityGraph{ nodes, edges, order }` with `validate()` (reuse
  `io_links`), `effects_union()` (reuse `union_effects`), `hash()`, serde,
  `lower_to_plan() -> SolutionPlan`, and a linear/primitive-only executor bridge.
  Register in `intelligence/mod.rs`; add unit tests. *No behavior change yet.*
- **W9-R2 — Generalize `CapabilitySpecification` to carry the IR.** `pipeline:
  Vec<String>` → `graph: CapabilityGraph` (keep a `pipeline()` accessor for
  back-compat; length-1/linear graphs produce identical ids + goldens). Migrate
  `from_goal` to build a linear IR. Provider `execute` runs the IR.
- **W9-R3 — Enforce R21 smoke on the live path (RC-2, gap M).** In
  `synthesize_for_goal` *and* `acquire_for_goal_reasoned`, after acquire + trust
  gate, run `LifecycleManager::smoke_test` (golden case); failure ⇒ quarantine +
  honest error + `Stage::Failure` event. **Highest-value safety fix.**
- **W9-R4 — Fix provenance (RC-3, gaps H/O/R).** Build the Decision Record *after*
  generation with `chosen=Some`, real candidates/rejections, real confidence; stamp
  `SynthesizedRecord` with `ir_hash`, `policy_version`, `primitive_set_version`,
  `source_goal_hash`; add `cpp_capability_provenance`.
- **W9-R5 — Granular synthesis events (gap E).** Add neutral synthesis sub-stages
  (e.g. `Stage::Synthesize` + outcome detail, or a `SynthStage` enum) emitted from
  `synthesize_for_goal`: SpecCreated → GenerationFinished → ValidationFinished →
  SmokeFinished → Activated.
- **W9-R6 — Effect union for composed graphs (gap I).** Compute descriptor effects
  as the *union* of node effects (max risk) via `union_effects`; permission gate on
  the whole graph (R11.1). Add a test that an escalating node widens the union.
- **W9-R7 — In-flight lock (gap K).** `DashMap`/`Mutex<HashSet<capability_id>>` in
  `SynthesisProvider`; concurrent identical goals collapse to one generation.
- **W9-R8 — Capability nodes in the IR (RC-4 reach, gap A).** Allow
  `NodeOp::Capability` referencing installed caps; `lower_to_plan` emits real
  `PlanStep`s executed by the HTN runtime. Reach now = every installed capability.
- **W9-R9 — Multi-input / typed schema (PHASE 7, gap D).** Compute `input_schema` +
  `io_modality`/`inputs`/`outputs` from the graph boundary; reuse `arg_gen` binding.
- **W9-R10 — Versioning + repair via EvolutionEngine (PHASE 9, gap G).** Version on
  every (re)synthesis; store history; `ProposalKind` repair = regenerate-from-IR;
  rollback = re-activate prior version.
- **W9-R11 — `IrProposer` + optional `LlmIrProposer` (RC-4, gaps A/L).** Trait; default
  deterministic; LLM impl behind `synthesis_llm`, validator+smoke mandatory, one
  bounded repair, budget-capped, IR cached by `source_goal_hash`. Honest-decline on
  invalid.
- **W9-R12 — Generate UI + `cpp_*` (PHASE 10, gap F).** `cpp_synthesis_preview`,
  `cpp_synthesize`, `cpp_synthesis_events`, `cpp_capability_versions`; Generate tab.
- **W9-R13 — Tier-3 code node (PHASE 6, gap B).** `NodeOp::Code` behind
  `synthesis_code`; OpenClaw Docker + seccomp build/test/bench; HITL-mandatory.
  **Honest external blocker: reliable code model — build the guarded scaffold, do
  not claim generation works until validated.**
- **W9-R14 — Real desktop campaign (PHASE 11).** Backend paths now; GUI-click = Wave 12.

**Sequencing:** R1→R2 unlock everything. R3 is the top *safety* fix (do it early,
even before R2 if needed). R4–R7 are cheap, high-value, do next. R8–R10 deliver the
"engineer" leap. R11 is model-gated. R12 is the UX. R13 is the deferred, flagged,
sandboxed frontier. R14 validates.

### 9. Risks
- **R21 smoke gap (M)** is live now — prioritize R9-R3.
- IR migration (R2) touches the artifact type → guard with back-compat accessor +
  golden-id-stability tests (same goal ⇒ same id/golden as today).
- LLM proposer (R11) can regress latency/cost → budget + cache + flag OFF by default.
- Tier-3 (R13) is the only genuinely blocked item → keep isolated behind its flag.

### 10. Migration strategy
- IR is additive: `CapabilitySpecification` gains `graph`; `pipeline()` derives from
  it. On-disk records: read old `{pipeline: [...]}` → lift to a linear graph on load
  (version the record schema; write new format going forward).
- No Tauri command/event renames (R27 contract stability); all new surfaces additive.

### 11. Backward compatibility
- `synthesis` flag OFF ⇒ byte-identical legacy (mandatory + tested via `all_disabled`).
- Existing 6 Wave-9 tests + primitive/synthesis units MUST stay green; add golden-id
  stability assertions so the IR refactor cannot change existing capability ids.

### 12. Testing strategy
- **Unit:** IR validate/lower/hash/effects-union; escalating-node widens union;
  linear graph == today's pipeline (id + golden parity).
- **Integration (real SQLite CKB):** fall-through → synthesize → smoke → activate →
  execute → CKB rows; smoke-failure ⇒ quarantine, no activation; repair proposal →
  apply → version bump → rollback; provenance `chosen=Some` + ir_hash present.
- **Neutrality:** gate stays green (no provider branching / cognition in `acl/*`).
- **Concurrency:** two identical goals ⇒ one generation, one Decision Record.
- **Flag-off parity:** all-disabled ⇒ legacy path unchanged.

### 13. Desktop validation strategy
PHASE 11 campaign; backend command paths via local API `:3001` now; GUI-click via a
webview harness = Wave 12. Verify real artifacts on disk + real CKB rows, never mocks.

### 14. Security strategy
- Effect union at max risk + whole-graph permission (R11.1) *before* activation.
- Mandatory pre-activation golden smoke (R21); quarantine on any gate failure.
- Lowest trust tier + lowest resource class for synthesized caps; trust earned via
  approval + benchmark, never granted on creation.
- Tier-3 code strictly inside the seccomp-bound OpenClaw Docker sandbox; never on
  host; container-leak proof (R24.1). No secrets to CKB/config/logs (R11.3/R12.4).

### 15. Performance strategy
- Tier-1/2 (deterministic + capability nodes) are cheap + synchronous-safe (pure /
  bounded). Add per-node timeout + cancel when capability/code nodes appear (R12.1).
- LLM proposer budgeted + cached by `source_goal_hash`; off critical path (R7.4).

### 16. Future-proofing analysis
The IR is `serde` + content-hashable + validator-gated + lowers to `SolutionPlan`.
It survives model swaps (change only the proposer), provider changes (nodes are
neutral coordinates; missing provider fails loudly → repair), benchmarking (nodes +
graph are measurable), versioning/migration (structural diff + re-validate), and
optimization (pure IR→IR rewrites). Code is a *node kind*, not the artifact — so the
safe majority of synthesis never depends on a compiler or a code model.

### 17. Why this is the best possible architecture
It **adds no new engine** (reuses HTN, planner, permission, CKB, events, evolution,
OpenClaw sandbox — satisfying the anti-proliferation and reuse guardrails), it is
**verifiable without a model** (validator + golden smoke admit artifacts, not the
LLM — satisfying honest-decline over fabrication), it **removes the primitive
ceiling** via capability nodes (reach = the whole installed ecosystem), it is
**safe by construction** (typed edges, effect union, mandatory smoke, lowest trust,
sandboxed code tier), and it **degrades gracefully** (deterministic Tier-1 always
works; model + code tiers are additive, flagged, and honestly blocked where the
environment blocks them). Every alternative either introduces a rival
engine/parser, depends on a reliable code model KRIA does not have, or cannot be
verified without executing untrusted code — all of which this design avoids.

---

## Appendix — Next-session start order (fast path)
1. **W9-R3** (live smoke gate) — safety, smallest, do first.
2. **W9-R1 + W9-R2** (IR + generalize spec) — unlocks everything; keep id/golden parity.
3. **W9-R4 + W9-R6 + W9-R7** (provenance, effect union, in-flight lock) — cheap, high value.
4. **W9-R5** (granular events) → **W9-R8/R9** (capability nodes + multi-input) — the leap.
5. **W9-R10** (versioning/repair) → **W9-R11** (LLM proposer) → **W9-R12** (Generate UI).
6. **W9-R13** (Tier-3, flagged, sandboxed — do not claim done) → **W9-R14** (campaign).
