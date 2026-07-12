# Requirements Document

> **Feature:** OpenClaw Intelligence Enhancements — *KRIA as an autonomous execution intelligence*

## Introduction

KRIA is not an assistant that *picks a tool*; it is an autonomous execution intelligence that
**engineers a solution** to a goal and then executes it. OpenClaw is one execution substrate among
many (native, GUI, browser, Docker, workflow, cloud API, MCP, remote agent, human, and
KRIA-*synthesized* capabilities). The Brain reasons, analyzes, compares, composes, acquires,
generates, verifies, benchmarks, remembers, and continuously improves; providers only execute.

This spec builds the Brain's missing **capability-engineering** layer on top of the existing
provider-neutral Capability Provider Platform (CPP), while reusing — not duplicating — KRIA's existing
execution machinery (the ReAct agent loop, the HTN execution-graph runtime `agent/htn_executor.rs`,
`WorkflowRuntimeRouter`, the semantic `routing/` layer, n8n workflows). The abstraction is
**Capability**, never "Skill"; "Skill" is an OpenClaw implementation detail.

Binding constraints: provider-neutral forever; no hardcoded prompts/keywords/provider branches;
honest degrade (no fabricated success); Capability Knowledge is a swappable abstraction (decoupled
from the pending global Memory redesign); and — because the local model (Qwen3-VL-4B) is weak at
structured reasoning — **all reasoning is tiered and confidence-budgeted** (a fast path for the common
case, deep reasoning only when warranted, honest "ask the user" when confidence is low).

## Glossary
- **Capability:** the neutral unit of execution KRIA reasons about — of a declared **kind**: native,
  installed, gui, browser, docker, workflow, cloud_api, mcp, remote_agent, human, or **synthesized**.
- **Skill:** an OpenClaw-provider implementation of a capability (an ACL detail, never in the Brain).
- **CPP:** Capability Provider Platform (`capability/*`) — the provider-neutral surface.
- **Provider:** a capability source implementing `CapabilityProvider` behind an `acl/*` adapter.
- **Descriptor / Effects:** self-describing capability + declared side-effects (classes,
  reversibility, resource class) that drive permission.
- **Capability Knowledge Base (CKB):** durable, learned knowledge about capabilities — usage,
  outcomes, health, provenance, relationships, preferences, adaptive trust, performance history,
  failure explanations. Supersedes the narrow "Capability Memory" concept.
- **Capability Reasoner:** the neutral component that turns a goal into a **Solution Plan** (compare
  candidates, estimate confidence, choose path: reuse / native / compose / acquire / generate / ask).
- **Solution Plan:** a possibly-multi-step **execution graph** of capabilities with plan-level effects.
- **Capability Planner:** composes capabilities into a Solution Plan and **emits it into the existing
  HTN execution-graph runtime** (does NOT introduce a new executor).
- **Lifecycle Manager / Evolution Engine:** own acquire→verify→smoke→activate→upgrade→replace→rollback
  →delete, and (Evolution) health/benchmark-driven self-improvement.
- **Reasoning budget / tier:** the confidence-gated depth of reasoning and the model tier used
  (planner tier vs executor tier), bounding latency/cost.
- **Grounding:** answering capability questions from CKB/registry, never model free-recall.

---

## Requirements

### R1 — Capability Knowledge Base (foundation)
1. A `CapabilityKnowledge` trait SHALL durably persist, per capability `(provider_id, capability_id)`:
   identity, **kind**, descriptor snapshot + hash, state, trust (+ **adaptive** trust from outcomes),
   declared effects, usage (successes/total), last-N outcomes with **failure explanations**,
   performance history (latency/cost), health, provenance (native/installed/acquired/synthesized),
   first-seen/last-used, and **relationships** (dependencies, composed-with, substitutes/alternatives).
2. It SHALL also persist **preferences/patterns**: preferred providers/capabilities per goal-class,
   execution-style preferences, and per-user/context overrides — as *learned signals*, never
   hardcoded rules.
3. The Brain SHALL consult the CKB **before** discovery (reuse without rediscovery) and SHALL answer
   "what do I have / what can I do / what worked / why did X fail" from the CKB — never model recall.
4. Learned signals (success, recency, adaptive trust, performance) SHALL feed ranking with
   configurable weights (replacing the ephemeral 0.05 in-index nudge) and SHALL be persisted.
5. The trait SHALL be storage-agnostic (SQLite today; graph-capable schema) with NO coupling to the
   current global Memory implementation, re-homable by the future Memory redesign.
6. Removing a capability SHALL cascade-purge its knowledge rows, relationships, index entry, and grants.

### R2 — Capability Reasoning Pipeline (the engineering brain)
1. A goal SHALL flow through an **anytime, tiered** pipeline that reasons like a senior engineer:
   **Goal Classification (taxonomy)** → **Objective Understanding** → **Hidden-Requirement,
   Constraint & Unknown Inference** → **Strategy Generation (multiple candidate strategies)** →
   Discovery → Candidate Comparison → (Acquire/Generate) → Composition → Execution Planning →
   Execution → Reflection → Learning.
2. **Goal taxonomy:** each goal SHALL be classified into a learned/semantic goal-class (e.g.
   information / analysis / transformation / automation / generation / coding / desktop / research /
   vision) to focus strategy — by evidence, NOT keyword rules.
3. **Inference:** the pipeline SHALL infer hidden requirements (e.g. login/cookies/captcha/rate-limit/
   storage for "download from 100 sites"), constraints (privacy, offline, cost), and **unknowns**;
   material unknowns SHALL trigger ONE clarifying question, not a guess.
4. **Strategy Generation:** a `StrategyGenerator` SHALL produce multiple candidate **strategies**
   (e.g. native-only, native+browser, marketplace, generate, hybrid, GUI-fallback, human-fallback),
   each with estimated confidence, **risk**, **cost** (R15), and **future-reuse** value, before any
   plan is built.
5. **Explicit iterative cognitive loop:** within the budget the Reasoner SHALL run
   Hypothesis → Evaluate → Revise → Compare → Reject/Accept, keeping the best strategy — genuine
   multi-round reasoning, not one-pass.
6. The pipeline SHALL **short-circuit**: a high-confidence single-capability match executes on the
   fast path with negligible added latency; deep multi-round reasoning runs ONLY when confidence is
   low or the goal is complex/composite.
7. A **reasoning budget** SHALL bound rounds, candidates, catalog fetches, and wall-time; exceeding it
   SHALL yield the best plan so far or an honest "ask the user", never an unbounded loop.
8. The pipeline SHALL support a **planner model tier** distinct from the executor tier; absence SHALL
   degrade gracefully (fast path + clarify), never fabricate.
9. Every pipeline run SHALL emit a durable, inspectable **reasoning/plan trace** (goal-class,
   inferred requirements, strategies considered + rejected with reasons, chosen strategy/plan),
   correlated end-to-end.
10. **Per-step confidence:** composed plans SHALL carry confidence per step; clarification SHALL be
    requested only for the low-confidence step(s), not the whole plan.

### R3 — Confidence-based candidate comparison & path selection
1. For a need, the Reasoner SHALL gather candidates across **all capability kinds/sources** uniformly
   and compare them on component signals (semantic, lexical, learned-success, adaptive trust, cost,
   recency, quality) with a documented, tunable calibrated confidence.
2. Path selection (reuse-installed / use-native / compose / search-marketplace / acquire / generate /
   ask) SHALL emerge from confidence + effects + **cost/risk priors** — NOT a hardcoded ladder and
   NOT provider names. Native/installed are *preferred as a low-cost/low-risk prior* that sufficient
   evidence can override.
3. Below confidence threshold the Reasoner SHALL ask ONE clarifying question or honestly decline —
   never acquire/generate/execute on a guess.
4. Argument generation SHALL be neutral, schema-grounded, and **constrained** (grammar-constrained
   local / structured cloud), schema-validated with bounded repair (never identical retry). Relocated
   out of `openclaw/arg_gen.rs`.
5. Candidate comparison SHALL incorporate the **Capability Cost Model** (R15) alongside confidence,
   trust, and quality — selection optimizes a documented multi-attribute objective, not a single score.
6. **Native/installed sufficiency gate:** WHEN a native or already-installed candidate meets the
   confidence + cost threshold for the goal-class, remote marketplace search and generation SHALL be
   **skipped entirely** (no needless latency/egress). Marketplace/generation are reached only on
   insufficiency — a strong, explicit native-first philosophy, still overridable by explicit user intent.

### R4 — Capability Composition & Execution Planning (reuse, don't duplicate)
1. The Planner SHALL compose multiple capabilities into a **Solution Plan** (execution graph:
   sequential, parallel, conditional) for composite goals (e.g. screenshot→OCR→translate→PDF→email).
2. The Planner SHALL **emit the Solution Plan into the existing HTN execution-graph runtime**
   (`agent/htn_executor.rs` / `WorkflowRuntimeRouter`); it SHALL NOT introduce a competing executor.
   A single, documented **planning authority + arbitration** decides which runtime owns a turn.
3. Plans SHALL be **saga-structured**: each step declares a compensation/rollback; a partial failure
   SHALL compensate completed steps or surface an honest partial result — never a silent half-done state.
4. Data flow between steps SHALL be typed via descriptor `inputs`/`outputs`; incompatible chains
   SHALL be rejected at planning time, not discovered at execution time.

### R5 — Complete Capability Lifecycle
1. Explicit, owned, observable stages: Discover → Rank → Select → Download → **Verify(hash/signature/
   schema)** → Install → Register → Descriptor-gen → Embed → Index → **Smoke-test** → Activate →
   Knowledge-record → Reuse → Health-monitor → Upgrade → Migrate → Replace → Rollback → Disable →
   Delete → Cleanup → Recover.
2. Post-install smoke test (descriptor-declared self-check) SHALL gate `Enabled`; failure ⇒ rollback +
   honest report. Install SHALL be transactional (no partial artifacts) and reinstall idempotent.
3. Upgrade/replace SHALL be first-class + atomic, preserving grants under effect monotonicity
   (⊆ prior), re-prompting only on widening.
4. Delete SHALL cascade (bundle+registry+index+CKB+grants+relationships); recovery on restart SHALL
   rebuild the index from `describe()`, restore CKB, and reconcile changed descriptor hashes.

### R6 — Capability Evolution & Self-improvement
1. An **Evolution Engine** SHALL monitor CKB health/performance and, on repeated failure or a
   materially better alternative, PROPOSE upgrade/replace/migrate — benchmarked on **cheap proxies**
   (smoke success, latency, error rate), NOT correctness oracles it cannot have.
2. Evolution actions SHALL be **auditable, reversible, and — for elevated effects — user-gated or
   confidence-gated**; KRIA SHALL never silently swap a capability the user relies on without a trace.
3. Learnings (better provider for a goal-class, chronic failures, adaptive trust changes) SHALL be
   written to the CKB and SHALL influence future selection.

### R7 — Capability Synthesis (generation) — guarded, later
1. Capability generation SHALL be modeled as a **synthesizing `CapabilityProvider`** whose `acquire`
   produces a new capability — reusing the identical acquire→verify→sandbox→smoke→benchmark→activate
   path (no special-case Brain code).
2. Synthesized capabilities SHALL be sandboxed, effect-declared, verified, and smoke-tested before
   activation; they SHALL default to the lowest trust tier and SHALL never bypass permission.
3. Synthesis SHALL be a staged capability spanning: **scaffold → repair → refactor → benchmark →
   optimize → version → migrate**, each stage flag-gated and independently validated. Early stages
   (scaffold/repair of small utilities) ship first; publishing/optimization are latest.
4. This capability is **scheduled late** and flag-gated; it SHALL NOT be on the critical path for the
   common daily-work use cases, and SHALL honestly decline when the available model cannot synthesize
   reliably.

### R8 — Marketplace Intelligence (neutral)
1. Marketplace SHALL be a neutral concept: any provider MAY expose `catalog()`; ranking treats all
   catalogs uniformly (GitHub Skills today, ClawHub/MCP/enterprise later) with no provider branching.
2. Candidate evaluation SHALL use declared quality/trust/cost/maintenance/adoption signals as ranking
   inputs (data, not rules).
3. Artifacts SHALL be integrity-verified (hash) and signature-verified where supported; unverifiable
   artifacts SHALL be quarantined. Catalog cache SHALL have explicit invalidation (not a 2nd truth).
4. Capability-ID **namespacing** across registries + semver descriptor versioning + **dependency
   metadata** SHALL be represented now (resolution may defer) to avoid a future schema break.
5. **ClawHub-specific model** SHALL be designed (even if implemented later): publishing flow,
   reviews/ratings, trust tiers, cryptographic **signatures**, dependency resolution, **update
   channels** (stable/beta), **breaking-version** signalling, and backward/forward **compatibility**
   negotiation via the protocol version. No ClawHub concept SHALL require a Brain change to adopt.

### R9 — Provider Neutrality & Capability Kinds
1. The Brain SHALL contain zero provider-name branches; a CI gate SHALL forbid `crate::openclaw`/
   `mcp::client` and provider-name string branches in `capability/` outside `acl/*`.
2. The capability model SHALL support all **kinds** (native/installed/gui/browser/docker/workflow/
   cloud_api/mcp/remote_agent/human/synthesized), including **async/long-running/streaming and remote**
   execution — the execution model SHALL NOT assume local Docker.
3. Adding a provider/kind SHALL require only a new `acl/*` adapter + registration — proven by a
   **second lifecycle-capable provider** through the identical neutral path (exit criterion).
4. Cognition currently in providers (arg-gen, marketplace match-selection) SHALL move to the neutral layer.

### R10 — Prompt Intent, Routing & Planner Arbitration (conflict elimination)
1. A neutral, evidence-based intent gate SHALL decide when capability execution is appropriate vs
   Settings NLP, GUI Automation, Vision, OCR, Memory, Native tools, Package Manager, Browser, and the
   existing planners (ReAct / HTN / n8n / synthetic flows) — by semantic evidence + effects +
   confidence, NOT keywords, with an inspectable trace.
2. A **single documented planning authority + arbitration policy** SHALL resolve which subsystem owns
   a turn when several compete (the #1 architectural risk: planner proliferation). Exactly one winner;
   logged rationale; explicit user intent ("use OpenClaw", "install a skill") is sticky and
   non-overridable.
3. Non-capability turns SHALL incur negligible added latency (cheap negative gate first) and be
   byte-identical when flags are off.

### R11 — Plan-level Effects, Permission & Security
1. For a composed/generated Solution Plan, permission SHALL be evaluated on the **union of step
   effects at max risk**, not per-isolated-step, so a benign-looking chain cannot escalate silently.
2. One approval SHALL cover a whole plan at its declared scope; re-prompt only on effect widening
   (monotonicity), so users are never nagged for small steps — yet never surprised by escalation.
3. Synthesized/remote capabilities SHALL be sandboxed and effect-bounded; artifact/signature
   verification SHALL gate activation; secrets SHALL never be logged or persisted to CKB/config.
4. Synthesized-capability execution SHALL be bound to the existing hardened sandbox
   (`config/seccomp/kria-seccomp.json` + the OpenClaw Docker substrate); generated code SHALL never run
   on the host or outside the sandbox, and SHALL run at the lowest resource-class + trust tier.

### R12 — Production Hardening
1. All capability/plan operations SHALL have explicit timeouts, bounded retries, cancellation; async
   paths free of blocking/deadlock; saga compensation on cancel.
2. Container lifecycle leak-free (verified: `docker ps` returns to pool baseline post-run + shutdown);
   resolve/downgrade the "RuntimeManagerSpawn not implemented" prewarm noise.
3. Failure taxonomy (transport/schema/permission/offline/context/verify/smoke/plan-step) → mapped
   recovery; full correlated telemetry + `CapabilityEvent`s + reasoning/plan traces.
4. Secrets never logged/persisted (preserve `redact_secrets`).

### R13 — Full End-to-End, Real-UI Validation
1. Validation SHALL run the real Desktop + local llama.cpp (Qwen3-VL-4B; no downloads, no Ollama, no
   config replacement), driving the true pipeline (UI and/or identical `/api/chat`), never mocked.
2. A campaign of **≥100 realistic prompts** SHALL cover general/layman/dev/automation/marketplace/
   capability/vision/OCR/GUI/desktop/browser prompts; install/remove/reuse/upgrade; **composite
   multi-step goals**; mixed/ambiguous/Hinglish/typo/multi-turn/adversarial/negative/conflicting/long/
   short prompts.
3. Per prompt, capture: routing+arbitration decision, reasoning/plan trace, candidates+confidence,
   permission decision, execution outcome, streamed reply, CKB/grant/index/log/event diffs.
4. Failures SHALL be root-caused (not symptom-patched) and regression-tested. No fabricated results.
5. **Golden goal set:** a labeled set of goals with expected goal-class, strategy, execution-path, and
   (for composites) plan shape SHALL be scored automatically each run (accuracy tracked over time).
6. **Fault-injection (chaos):** validation SHALL induce provider-offline, verify-fail, smoke-fail,
   partial-plan-failure, budget-exhaustion, and permission-deny, proving saga compensation, rollback,
   breaker, and honest degrade — live, not mocked.
7. **Explainability check:** for a sample of turns, KRIA's user-facing "why this / why not that"
   answer SHALL match the recorded decision records (R16).
8. **Wiring proof (R25):** validation SHALL assert the real call paths (dispatcher→Reasoner,
   Planner→HTN single executor, CKB consulted-before-discovery) fire in production, not just in units.

### R14 — Success Metrics
1. Grounding: capability answers match the real registry/CKB 100% (zero hallucination).
2. Selection: correct execution-path + correct single-vs-compose decision ≥ target% on a labeled set;
   wrong-tool and missed-marketplace rates tracked down vs baseline.
3. Composition: composite goals produce a correct multi-step plan that executes with saga rollback on
   induced failure.
4. Reuse & preferences: a used capability is reused later without rediscovery; learned provider/style
   preferences demonstrably influence later selection.
5. Permission: approved plans not re-prompted within scope; `cpp_grants.db` non-empty and honored.
6. Lifecycle+Evolution: install→verify→smoke→reuse→upgrade→replace→rollback→delete→reinstall green;
   an induced chronic failure triggers a benchmarked, auditable evolution proposal.
7. Arg-gen: zero `repeated_identical_failure` schema aborts.
8. Stability: zero container leaks; zero blank replies on non-tool turns; reasoning budget never loops.

### R15 — Capability Cost Model
1. A `CostModel` SHALL estimate, per candidate/strategy: latency, GPU/RAM footprint, token cost, API/
   money cost, install cost, and maintenance burden — from descriptor `CostHint` + learned performance
   history (CKB), degrading to conservative defaults when unknown.
2. Cost SHALL be a first-class input to strategy/candidate selection (R2.4, R3.5) and to the
   native-first sufficiency gate (R3.6).

### R16 — Decision Memory & Explainability
1. The CKB SHALL persist **Decision Records**: for each engineering decision, the goal, goal-class,
   candidates considered, chosen path + capability, and **why alternatives were rejected** (native/
   browser/OCR/marketplace/generate), with the confidence/cost/risk that drove it.
2. KRIA SHALL answer user-facing "why did you choose X / why not Y / why install this" from the
   Decision Records + reasoning trace — grounded, never fabricated.
3. Decision Records SHALL feed learning: repeated good/bad decisions adjust future strategy priors.

### R17 — Capability Families & Portfolio Awareness
1. Capabilities SHALL carry a learned/declared **family** (e.g. ocr, vision, pdf, browser, filesystem,
   translation, automation, coding, reasoning) — distinct from `kind` — to focus discovery/substitution.
2. When multiple capabilities in a family exist, selection SHALL reason over their **trade-off profile**
   (accuracy vs speed vs cost vs trust) for the goal at hand — multi-attribute choice, not a single
   ranking. (Scoped: pragmatic trade-off, not full portfolio optimization.)

### R18 — Capability Benchmark Framework
1. A `BenchmarkHarness` SHALL run candidate capabilities against **golden prompts / synthetic inputs**
   in a sandbox and record proxy scores (success, latency, cost, error class) to the CKB.
2. Benchmarks SHALL power evolution (R6) and family trade-off selection (R17); the framework SHALL be
   honest about its limits (proxies, not correctness oracles) and never block the fast path.

### R19 — Capability Retirement
1. The lifecycle SHALL include **retirement**: Deprecated → Unused (idle-decay) → Archive → Delete →
   Recover, so the ecosystem does not accumulate stale/unused capabilities.
2. Retirement SHALL be gated + reversible (archive before delete; recover on demand) and SHALL cascade
   cleanup (bundle/registry/index/CKB/grants) exactly like R5.4.

### R20 — Continuous Capability Discovery & Maintenance (background, gated)
1. A background, **off-by-default**, budget-bounded loop SHALL periodically check for new/better
   capability versions, security fixes, and newly available providers/catalogs — writing findings to
   the CKB as *proposals*, never auto-applying elevated changes without R6.2 gating.
2. It SHALL respect quiet hours / resource limits and SHALL never degrade foreground latency.

### R21 — Pre-install Sandbox Testing
1. Before activation, an acquired/synthesized capability SHALL pass: Download → **Sandbox** →
   Verify(hash/sig/schema) → **Smoke** → **Synthetic-prompt test** → (optional Benchmark) → Activate.
   Failure at any gate ⇒ quarantine/rollback + honest report; nothing is trusted on download alone.

### R22 — CKB → Future Memory Migration
1. The CKB SHALL expose a stable, versioned interface + export/import so the pending global Memory
   redesign can adopt it **without** changing Brain callers.
2. A documented migration path SHALL exist: dual-write/shadow-read window, schema-version negotiation,
   and a reversible cut-over — so capability knowledge is never lost or stranded during the redesign.

### R23 — Brain/Hands Architectural Invariant
1. The invariant **"KRIA = Brain, OpenClaw = Hands, intelligence belongs to KRIA, execution belongs to
   the provider"** SHALL be an enforced principle: no reasoning/selection/arg-gen/decision logic in any
   `acl/*` provider; providers only execute what the Brain decides. A CI check SHALL guard it (R9.1).

### R24 — Concurrency, Determinism & Versioned Reasoning Policy
1. The CKB and GrantStore SHALL be safe under concurrent turns/sessions (transactional; no lost
   updates); execution SHALL respect container-pool limits with backpressure, not unbounded spawn.
2. Reasoning policy (weights, thresholds, priors) SHALL be **versioned** and recorded in the trace, so
   behavior changes are reproducible, auditable, and A/B-testable. Event/telemetry schemas SHALL be versioned.
3. Cold-start (CKB empty/warming) SHALL behave correctly (fall back to discovery), never block.

### R25 — Wiring Verification (integration must be proven, not assumed)
1. Integration tests SHALL assert the **real production call paths** fire: the `openclaw` dispatcher +
   synthetic flows call the Reasoner (not their own thresholds); the Planner emits into the **existing
   HTN runtime** (no second executor instantiated); the CKB is consulted **before** discovery; grants
   are read on every authorize; outcomes are written to the CKB.
2. A wiring smoke test SHALL run at startup (debug) asserting all intelligence components are injected
   when their flags are ON, and absent (legacy path) when OFF.

### R26 — Remote/Cloud Capability Safety (data egress & cost ceilings)
1. Capabilities of `kind` cloud_api / remote_agent SHALL declare a **data-egress** effect; sending
   user data off-device SHALL require permission under R11 and SHALL be surfaced in the plan.
2. Cloud/remote capabilities SHALL honor per-capability **rate limits + cost ceilings** from config;
   exceeding a ceiling SHALL decline honestly, never silently spend.

### R27 — Frontend & UX Integration (Desktop, SolidJS)
1. Every capability-engineering concept SHALL be observable and controllable from the Desktop UI —
   NOT backend-only. New surfaces SHALL be **additive** (new `cpp_*` commands + `capability:*` events),
   never renaming/removing existing Tauri commands/events (contract stability).
2. **Chat integration:** during a turn the UI SHALL render, via streamed events, the reasoning/plan
   trace (goal-class, inferred requirements, strategies considered/rejected, chosen path + per-step
   confidence + cost) as a collapsible "thinking / plan" panel, reusing the existing streaming model
   (`agent:*` events) and adding `capability:reasoning` / `capability:plan` events.
3. **Plan preview & approval:** for composed/elevated Solution Plans the UI SHALL show a plan preview
   (steps, effects, plan-level permission) and gate execution through the existing HITL/approval UX
   (`approve_action`/`deny_action` + a new `cpp_plan_*` path), never auto-running an elevated plan.
4. **Capability Manager view** SHALL extend the existing `CapabilitiesView` with tabs for: Installed
   (from CKB, grounded), Marketplace (catalog + install/uninstall via existing `clawhub_*`), Families,
   Health, Decision Records/Explainability, Evolution Proposals, and Jobs — backed by new `cpp_*`
   commands (`cpp_installed`, `cpp_families`, `cpp_health`, `cpp_decisions`, `cpp_proposals`, `cpp_jobs`).
5. **Permission/Approval Center** SHALL show plan-level grants and reuse the existing
   `cpp_list_grants`/`cpp_revoke_grant` + `PermissionManagerView`; re-prompt only on effect widening.
6. **Explainability panel:** the UI SHALL let the user ask/inspect "why did you choose X / not Y",
   rendered from Decision Records (R16) via `cpp_decisions`.
7. Web/non-Tauri mode (local API) SHALL expose equivalent read paths (`/api/capability/*`) so the
   same information is available headless (mirrors existing `/api/chat` parity).
8. All new UI SHALL be flag-gated + i18n-ready (existing `ui/src/locales/*`), degrade gracefully when a
   backend component is disabled, and add negligible latency to non-capability turns.

### R28 — Long-running & Resumable Jobs
1. A Solution Plan MAY be a **long-running job** (e.g. "download from 100 sites"); such jobs SHALL have
   durable state (queued/running/paused/failed/done + per-step progress) persisted so they **survive
   restart** and are **resumable**.
2. The UI SHALL show live progress + partial results + cancel/pause/resume (new `capability:job_progress`
   event + `cpp_jobs`/`cpp_job_control` commands); backend SHALL emit correlated progress telemetry.
3. Jobs SHALL be saga-safe (R4.3): cancel/failure compensates completed steps or returns an honest
   partial result; a resumed job SHALL not repeat completed idempotent steps.
4. Jobs SHALL respect concurrency/backpressure limits (R24) and per-capability rate/cost ceilings (R26).

### R29 — Autonomy Oversight & Control Surface
1. Autonomous actions (evolution migrations R6, continuous-discovery proposals R20, auto-acquire) SHALL
   surface in an **Activity/Oversight feed** with what changed, why (Decision Record), and an **undo**.
2. A configurable **autonomy level** (e.g. manual / propose-only / auto-with-notice / full-auto per
   risk tier) SHALL govern when KRIA acts without asking; default SHALL be conservative (propose-only
   for elevated effects). Setting SHALL live in config (SQLite) + a settings UI.
3. No elevated/irreversible autonomous action SHALL occur above the configured autonomy level without
   explicit approval; every autonomous action SHALL be auditable + reversible (R6.2, R19.2).

### R30 — Cost Calibration & Correctness-vs-Liveness (honesty about limits)
1. The Cost Model (R15) SHALL **learn from actuals**: measured latency/resource/cost from executions
   update CKB perf history and refine future estimates (closed loop), degrading to conservative
   defaults when uncalibrated.
2. Verification/smoke/benchmark SHALL be documented as proving **liveness** ("it ran, schema-valid,
   didn't error"), NOT semantic **correctness** of outputs. KRIA SHALL NOT claim a result is correct
   beyond what it verified.
3. For **correctness-critical** goal-classes (declared in the taxonomy), KRIA SHALL surface the result
   for user confirmation or run an available validation capability, rather than silently trusting output.

### R31 — End-to-End Wiring Map & Contract Stability
1. The design SHALL maintain an explicit **wiring inventory** (§34): every new backend component ↔
   Tauri command ↔ event ↔ frontend view ↔ existing-KRIA touchpoint (chat.rs, local_api.rs,
   capability.rs, openclaw.rs, runtime.rs, htn_executor.rs, app.ts, CapabilitiesView,
   PermissionManagerView, SkillMarketplace). No component SHALL be added without its full wiring row.
2. Existing Tauri command/event names and the `/api/chat` shape SHALL be preserved (additive-only);
   a contract-stability test SHALL guard renames/removals.
3. The startup wiring smoke test (R25.2) SHALL assert every new command is registered and every new
   event has a producer + a frontend consumer when its flag is ON (no dangling command/event/view).

---

## Production Exit Criteria (declare production-ready only when ALL hold)
- E1. R1–R12 acceptance criteria implemented behind flags; flag-off = byte-identical legacy.
- E2. Provider neutrality proven by a second lifecycle-capable provider through the identical path (R9.3).
- E3. Full lifecycle + a real composite multi-step plan (with saga rollback) green in real-UI validation.
- E4. ≥100-prompt real-UI campaign meets all R14 metrics; every failure root-caused + regression-tested.
- E5. CKB durable across restart; grounding hallucination rate = 0; learned preferences influence selection.
- E6. Single planning authority + arbitration proven: no planner-proliferation conflicts across
  ReAct/HTN/n8n/synthetic/CPP; no subsystem interference (Settings/GUI/Vision/OCR/Memory/Native/Package/Browser).
- E7. Security: plan-level effects+permission enforced; hash/signature verification; sandboxed
  synthesized/remote capabilities; no secret leakage; no orphan grants.
- E8. Observability: correlated telemetry + events + reasoning/plan traces for every stage.
- E9. No hardcoded prompts/keywords/provider branches in the Brain (CI gate green).
- E10. Evolution actions auditable + reversible + gated; documented rollback for every phase; reversible migrations.
- E11. Reasoning is a genuine iterative loop with goal-class + strategy generation + cost model + per-step
  confidence; the reasoning/plan trace + Decision Records explain every choice; explainability answers
  verified against records (R2, R15, R16).
- E12. Native/installed sufficiency gate proven: marketplace/generation skipped when native suffices (R3.6).
- E13. Families + benchmark + retirement work: family trade-off selection, golden-prompt benchmarks,
  and reversible retirement all demonstrated (R17, R18, R19).
- E14. CKB→Memory migration path validated (dual-write/shadow-read + reversible cut-over); CKB concurrency-safe;
  reasoning policy + telemetry schemas versioned (R22, R24).
- E15. Wiring proven in production paths (dispatcher→Reasoner, Planner→single HTN executor, CKB-before-discovery),
  and remote/cloud data-egress + cost ceilings enforced (R25, R26). Fault-injection campaign green (R13.6).
- E16. **Frontend parity:** every capability action (reason/plan-preview/approve/install/uninstall/reuse/
  explain/jobs/evolution-proposals) is drivable AND observable in the real Desktop UI; additive
  command/event contracts preserved; no dangling command/event/view (R27, R31).
- E17. **Long-running jobs + oversight:** a real long-running composite job survives restart, resumes,
  reports progress/partial results, and its autonomous actions appear in the oversight feed with undo,
  governed by the autonomy level (R28, R29).
- E18. **Honesty on limits:** cost model calibrates from actuals; verification is stated as liveness not
  correctness; correctness-critical classes surface for confirmation (R30). Synthesized code runs only
  under the seccomp-bound sandbox (R11.4).
