# Design — OpenClaw Intelligence Enhancements

> Read `requirements.md` first. Grounded in the **real implementation** (verified: `capability/
> {provider,platform,registry,index,descriptor,protocol,state,grants,permission}.rs`,
> `tools/capability_dispatch.rs`, `openclaw/{arg_gen,clawhub}.rs`, `agent/loop_engine/mod.rs`,
> `agent/htn_executor.rs`, `config/store.rs`) and by live-driving `/api/chat`. Code wins over docs.

## Overview
This design turns KRIA from a capability *selector* into a capability *engineer*. A **Capability
Reasoning Pipeline** converts a goal into a **Solution Plan** — which may be a single capability, a
composed execution graph, an acquisition, a synthesis, or a hybrid — grounded in a durable
**Capability Knowledge Base (CKB)**, chosen by a confidence-scored **Reasoner**, composed by a
**Planner** that emits into KRIA's *existing* HTN execution-graph runtime, executed by pure providers,
then **reflected** on and **learned**. All components are neutral (no provider names); providers stay
pure hands. Reasoning is **tiered and budgeted** so the common case stays fast and a weak local model
never loops or hallucinates unchecked. §1–§20 cover vision→exit; format sections are consolidated at
§21–§26.

## 1. Vision
KRIA is an autonomous execution intelligence: it understands goals, analyzes needs/constraints,
compares alternatives across all capability kinds, composes/acquires/generates capabilities, verifies
and benchmarks them, remembers everything, and improves over time. OpenClaw (and future MCP/ClawHub/
enterprise/agent providers) only execute. The abstraction is **Capability**, not "Skill".

## 2. Existing Architecture Analysis (code-verified)

### 2.1 Strong foundations to keep
- Neutral `CapabilityProvider` boundary + `CapabilityPlatform` composition root; OpenClaw confined to
  `acl/openclaw.rs`; grep gate enforces it.
- `FederatedIndex` (semantic⊕lexical⊕success) behind a trait; `ProviderRegistry` refresh + circuit breaker.
- Rich descriptor model already present: `DescriptorVersion`, `TrustInfo`, `Effects`, `QualitySignals`,
  `CostHint`, `UsageStats`, capability `state`s (`Installed→Validated→Enabled`).
- Effects-driven permission + durable `GrantStore`.
- **Crucially: KRIA ALREADY has an execution-graph runtime** — `agent/htn_executor.rs` (GuiWorkflow +
  `WorkflowResult` + compensation-ish verdicts) and `WorkflowRuntimeRouter`, plus the ReAct
  `loop_engine`, the semantic `routing/` layer, n8n workflows, and synthetic Capability/Package flows.

### 2.2 Gaps vs the *engineering* vision
| Area | Reality | Consequence |
|---|---|---|
| Reasoning | ReAct + best-match retrieval; no goal/need/constraint analysis, no strategy | selects, doesn't engineer |
| Composition | none in CPP (platform docstring defers "ExecutionGraph"); HTN exists but not fed by capability planning | composite goals fail |
| Knowledge | ephemeral in-index stats (weight 0.05, lost on restart) | no reuse, no preferences, hallucinated inventory |
| Confidence | raw score + floor | can't decide sufficiency/abstain/compose |
| Lifecycle | ends at install; no verify/smoke/upgrade/replace/rollback/cascade | unsafe autonomy |
| Evolution | none | no self-improvement |
| Synthesis | none | can't create capabilities |
| Neutrality leaks | arg-gen + marketplace-selection inside `openclaw/` | cognition in the hands |
| **Planner proliferation** | ReAct + HTN + WorkflowRuntimeRouter + n8n + synthetic flows coexist | **turn-ownership conflicts**; a new planner would worsen it |
| Plan security | permission is per-capability | composed/generated chains could escalate |
| Config authority | SQLite cutover overrides `config.toml` silently | ops can't trust settings |

## 3. Gap → Requirement map
G-reason→R2; G-compare/confidence→R3; G-compose→R4 (+reuse HTN); G-knowledge→R1; G-lifecycle→R5;
G-evolution→R6; G-synthesis→R7; G-marketplace→R8; G-neutrality→R9; G-planner-proliferation→R10;
G-plan-security→R11; G-hardening→R12; G-validation→R13/R14.

## 4. Iterative Review — dispositions on reviewer concerns (+ self-critique)
| # | Concern | Verdict | Resulting design change |
|---|---|---|---|
| 1 | Selection→Engineering | **Correct** | `CapabilitySelector` → **Capability Reasoner** emitting a Solution Plan; single-cap = fast path |
| 2 | Composition/graphs | **Correct + caveat** | **Capability Planner** composes, but **emits into existing HTN runtime**; no new executor |
| 3 | Skill generation | **Partial** | **Synthesizing provider** (same acquire path), sandboxed+verified, scheduled late |
| 4 | Evolution | **Correct** | **Evolution Engine** (benchmark proxies, auditable, reversible, gated) |
| 5 | Memory→Knowledge | **Correct** | Capability Memory → **CKB** (prefs, provenance, failure explanations, adaptive trust, perf) |
| 6 | Native-first ladder | **Partial** | reject hardcoded ladder; **cost/risk prior** overridable by confidence |
| 7 | Skill→Capability | **Correct** | "Capability" is the abstraction; enumerate **kinds**; "Skill"=OpenClaw detail |
| 8 | Capability graph | **Partial** | relationships first-class in the **data model**; heavy graph reasoning deferred |
| 9 | Acquisition pipeline | **Correct** | merged into the reasoning pipeline (need→eval→acquire→benchmark→verify→learn) |
| 10 | Agent ecosystem | **Correct** | capability kinds incl. agent/remote/cloud; execution not Docker-bound |
| 11 | Decision→reasoning-centric | **Correct + caveat** | adopt the pipeline **tiered/anytime + reasoning budget** |
| A | *(self)* planner proliferation | **Correct — top risk** | R10: one planning authority + arbitration; capability planning feeds HTN |
| B | *(self)* model-capability mismatch | **Correct** | R2.4 planner-model tier seam; confidence-gated depth |
| C | *(self)* latency/cost | **Correct** | R2.2/2.3 fast path + reasoning budget |
| D | *(self)* plan-level effects | **Correct** | R11 union/max-risk permission on the whole plan |
| E | *(self)* saga rollback | **Correct** | R4.3 per-step compensation |
| F | *(self)* benchmark oracles | **Partial** | R6.1 cheap proxies only (smoke/latency/error), honest scope |
| G | *(self)* reasoning trace | **Correct** | R2.5 durable plan/reasoning trace |
| H | *(self)* evolution safety | **Correct** | R6.2 audit + reversible + gated |

### 4b. Second review round — depth refinements (dispositions)
| # | Concern | Verdict | Design change |
|---|---|---|---|
| 1 | Autonomous engineer meta-reasoning | Accept | R2 expanded: goal-class + objective + hidden-req/constraint/unknown inference + strategy gen |
| 2 | Iterative cognitive loop | Accept | R2.5 Hypothesis→Evaluate→Revise→Compare→Reject/Accept within budget |
| 3 | Decision Memory | Accept | CKB **Decision Records** (R16); powers explainability + learning |
| 4 | Explicit Strategy Generator | Accept | `StrategyGenerator` → candidate strategies before plan |
| 5 | Cost Model | Accept | `CostModel` (R15) feeds strategy/candidate selection |
| 6 | Portfolio thinking | Accept (scoped) | family trade-off profile (R17), not full portfolio optimization |
| 7 | Benchmark framework | Accept | `BenchmarkHarness` (R18), proxy scores, honest limits |
| 8 | Retirement | Accept | lifecycle **retirement** stage (R19) |
| 9 | Families | Accept | `family` attribute distinct from `kind` (R17) |
| 10 | Goal taxonomy | Accept (semantic) | `GoalClassifier` first stage (R2.2), learned not keyword |
| 11 | Native-first philosophy | Accept | sufficiency gate skips remote search when native suffices (R3.6) |
| 12 | Per-step confidence | Accept | plan steps carry confidence; clarify only weak steps (R2.10) |
| 13 | CKB→Memory migration | Accept | dedicated migration section + versioned interface (R22, §27) |
| 14 | Deeper lifecycle detail | Accept | §13 table deepened; §28 per-stage detail |
| 15 | ClawHub detail | Accept | R8.5 + §11 ClawHub model (publish/reviews/sig/deps/channels/compat) |
| 16 | Brain/Hands invariant | Accept | R23 enforced invariant; CI guard |
| 17 | Explainability | Accept | user-facing why from Decision Records (R16.2) |
| 18 | Broader synthesis | Accept (phased) | scaffold→repair→refactor→benchmark→optimize→version→migrate (R7.3) |
| 19 | Continuous discovery | Accept (gated) | background proposal loop (R20) |
| 20 | Pre-install sandbox testing | Accept | download→sandbox→verify→smoke→synthetic→benchmark→activate (R21) |
| I | *(self)* CKB concurrency + versioned policy | Accept | R24 transactional CKB, versioned reasoning policy/telemetry |
| II | *(self)* wiring proof | Accept | R25 integration asserts real call paths + startup wiring smoke |
| III | *(self)* remote/cloud egress + cost ceilings | Accept | R26 data-egress effect + cost ceilings |

Self-check after refactor: the architecture now *engineers* solutions (classify→infer→strategize→
iterate→compose), remembers **decisions** as well as outcomes, prices choices with a cost model, reasons
over capability **families**, benchmarks and retires capabilities, composes via the *existing* HTN
runtime (no proliferation), migrates cleanly into the future Memory, and proves its wiring. Remaining
objections (synthesis realism, benchmark oracle depth) are explicitly scoped/deferred with reasons.

## 5. Final Architecture (target)
```
User prompt
  │
  ▼
Prompt Intent Gate + Planning Arbitration (R10)   ── one winner among ReAct/HTN/n8n/synthetic/CPP; trace
  │  (capability-engineering turn)
  ▼
Capability Reasoning Pipeline (R2) — tiered, budgeted, traced, iterative
  1 Goal Classification (taxonomy)  → 2 Objective Understanding
  3 Hidden-Requirement / Constraint / Unknown Inference  (→ clarify if material unknown)
  4 Strategy Generation (multiple strategies; each with confidence + risk + COST + reuse value)
  5 Iterative loop: Hypothesis → Evaluate → Revise → Compare → Reject/Accept  (within budget)
  6 Discovery (CKB-first → Index → Catalog); NATIVE/INSTALLED SUFFICIENCY GATE skips remote if enough
  7 Candidate Comparison + Confidence + CostModel (Reasoner, R3/R15)  ── fast-path exit if high-conf single cap
  8 Acquire / Generate (LifecycleManager / Synthesizing provider; sandbox+smoke+benchmark, R5/R7/R21)
  9 Composition (Capability Planner → Solution Plan; per-step confidence, R4/R2.10)
 10 Execution Planning → emit into EXISTING HTN runtime (agent/htn_executor.rs)
 11 Execution (providers; plan-level permission R11)
 12 Reflection → 13 Learning (write CKB outcomes + Decision Records, R1/R6/R16)
  │
  ▼
CapabilityPlatform surface  ── with_knowledge / with_reasoner / with_planner / with_lifecycle / with_evolution / with_cost_model / with_events
  ├─ CapabilityKnowledge (CKB, trait; SQLite graph-capable; + Decision Records)  [R1/R16]
  ├─ GoalClassifier + StrategyGenerator + CapabilityReasoner + ArgumentGenerator [R2/R3]
  ├─ CostModel                                                    [R15]
  ├─ CapabilityPlanner → HTN runtime adapter (single executor)    [R4]
  ├─ LifecycleManager + EvolutionEngine + BenchmarkHarness + RetirementManager  [R5/R6/R18/R19]
  ├─ ContinuousDiscovery (background, gated)                      [R20]
  ├─ PermissionEngine + GrantStore (plan-level; egress+ceilings)  [R11/R26]
  ├─ FederatedIndex (derived; reads CKB signals)                  [R1.4]
  └─ ProviderRegistry → CapabilityProvider (acl/*)                [R9]
        kinds: native | installed | gui | browser | docker | workflow | cloud_api | mcp | remote_agent | human | synthesized
        families: ocr | vision | pdf | browser | filesystem | translation | automation | coding | reasoning | ...
```

## 6. Phase-by-phase roadmap (reordered around the pipeline)
- **P1 Capability Knowledge Base** — foundation: reuse, grounding, learned signals, relationships,
  preferences, **Decision Records** (R16), **families** (R17), concurrency-safe + versioned + **Memory-migration interface** (R22/R24).
- **P2 Reasoner + Confidence + Cost + Arg-gen** — think-before-execute core: **goal taxonomy**,
  hidden-req/constraint/unknown inference, **StrategyGenerator**, **iterative loop**, **CostModel** (R15),
  **native sufficiency gate** (R3.6), per-step confidence, fast path + budget + planner-model tier.
- **P3 Planning Authority & Arbitration + Capability Planner→HTN** — kill planner proliferation; composition.
- **P4 Complete Lifecycle** — verify/**sandbox+smoke+synthetic** (R21)/transactional/upgrade/replace/
  rollback/**retirement** (R19)/cascade/recovery.
- **P5 Plan-level Effects & Permission + Security** — union/max-risk; sandbox; artifact/signature verify;
  **remote/cloud egress + cost ceilings** (R26).
- **P6 Marketplace Intelligence** — neutral catalogs, integrity, versioning, namespacing, deps, cache,
  **ClawHub model** (publish/reviews/ratings/signatures/channels/compat, R8.5).
- **P7 Provider Neutrality proof + Capability Kinds** — relocate cognition; 2nd provider; async/remote kinds.
- **P8 Evolution + Benchmark** — **BenchmarkHarness** (golden/synthetic, R18) + family trade-off (R17);
  auditable/reversible/gated self-improvement.
- **P9 Capability Synthesis** — guarded synthesizing provider; staged scaffold→repair→refactor→
  benchmark→optimize→version→migrate (R7.3); late, flag-gated.
- **P10 Continuous Discovery & Maintenance** — background, off-by-default proposal loop (R20).
- **P11 Production Hardening** — timeouts/retry/cancel/saga, leak proof, taxonomy, telemetry, traces.
- **P12 Real-UI ≥100-prompt Validation** (composite goals + fault-injection + explainability + **wiring
  proof** R25 + **frontend parity** R27/E16) → **P13 Release gate**.
- **Cross-cutting: Frontend Integration track (R27)** — each phase ships its UI slice (commands +
  events + view) alongside the backend; no backend-only milestone. Long-running **Jobs** (R28) land
  with P4 (durable state) + P11 (hardening); **Autonomy Oversight** (R29) lands with P8/P10 (the phases
  that introduce autonomous actions). **Cost calibration** (R30) rides P2/P8; **seccomp-bound synthesis**
  (R11.4/§38) with P9.
- **Cross-cutting: Wiring Verification (R25/§34)** — the wiring map row for a component MUST be
  implemented (command+event+view+touchpoint) in the same wave that adds the component; asserted at
  every milestone. No dangling command/event/view.

Sequencing rationale: knowledge (P1) → reasoning (P2) → *arbitration+composition before more power*
(P3) so the existing runtimes don't fight; then a trustworthy lifecycle (P4) and its security (P5);
marketplace/neutrality (P6/P7) broaden sources; evolution (P8) and synthesis (P9) are the highest-risk,
most model-dependent — deliberately last; hardening+validation gate release. Adding skills/providers
before P3 is forbidden (multiplies conflicts on an unarbitrated core).

## 7. Dependency Graph
```json
{
  "waves": [
    {"wave":0,"phase":"P0","desc":"Seams: flags, config-authority reconcile, neutral trait skeleton, CI neutrality gate, CapabilityKind+Family enums, versioned reasoning-policy + telemetry schema"},
    {"wave":1,"phase":"P1","desc":"CKB (SQLite graph-capable): reuse, grounding, learned signals, relationships, preferences, Decision Records, families, concurrency-safe, Memory-migration interface"},
    {"wave":2,"phase":"P2","desc":"Reasoner: goal taxonomy + objective/hidden-req/constraint/unknown inference + StrategyGenerator + iterative loop + CostModel + native sufficiency gate + per-step confidence + constrained arg-gen + planner-model seam + reasoning trace + fast-path/budget"},
    {"wave":3,"phase":"P3","desc":"Planning Authority + Arbitration + Capability Planner -> existing HTN runtime + typed IO chaining + saga skeleton"},
    {"wave":4,"phase":"P4","desc":"Lifecycle: verify(hash/sig/schema)+sandbox+smoke+synthetic-prompt test+transactional install+upgrade/replace+rollback+retirement+cascade cleanup+recovery"},
    {"wave":5,"phase":"P5","desc":"Plan-level effects (union/max-risk)+plan permission+sandbox+artifact/signature verify+remote/cloud egress+cost ceilings+secret safety"},
    {"wave":6,"phase":"P6","desc":"Marketplace: neutral ranking signals+integrity+cache+namespacing+versioning+deps + ClawHub model (publish/reviews/ratings/signatures/channels/compat)"},
    {"wave":7,"phase":"P7","desc":"Neutrality proof: relocate arg-gen+marketplace-selection; 2nd lifecycle provider; async/long-running/remote kinds; CI gate"},
    {"wave":8,"phase":"P8","desc":"Evolution + BenchmarkHarness (golden/synthetic proxies) + family trade-off selection; auditable+reversible+gated migrate/replace; CKB learning"},
    {"wave":9,"phase":"P9","desc":"Capability Synthesis provider: staged scaffold->repair->refactor->benchmark->optimize->version->migrate; sandboxed+verified+smoke-gated+lowest-trust, flag-gated, honest-decline"},
    {"wave":10,"phase":"P10","desc":"Continuous Discovery & Maintenance: background off-by-default proposal loop (new/better versions, security fixes, new providers), budget/quiet-hours bounded"},
    {"wave":11,"phase":"P11","desc":"Hardening: timeouts/retry/cancel/saga-compensation + concurrency/backpressure + container-leak proof + failure taxonomy + correlated telemetry/traces"},
    {"wave":12,"phase":"P12","desc":"Real-UI >=100-prompt campaign incl composite goals + fault-injection + explainability + wiring proof + golden goal set; metrics + regression"},
    {"wave":13,"phase":"P13","desc":"Exit-criteria gate (E1-E15) + flag flip + docs + rollback playbooks + CKB->Memory migration validation"}
  ],
  "edges": ["P0->P1","P1->P2","P2->P3","P3->P4","P4->P5","P5->P6","P6->P7","P7->P8","P8->P9","P9->P10","P4->P11","P10->P11","P11->P12","P12->P13"],
  "cross_cutting": ["Wiring Verification (R25) asserted at every milestone"]
}
```

## 8. Wiring & Integration
- Extend the CPP block in `kria-desktop/src/commands/runtime.rs`: open the CKB (SQLite), construct
  Reasoner/Planner/Lifecycle/Evolution, inject via `CapabilityPlatform` builder methods
  (`with_knowledge/with_reasoner/with_planner/with_lifecycle/with_evolution`), mirroring `with_events`.
- The `openclaw` dispatcher + synthetic flows become thin callers of the Reasoner; they stop making
  ranking/threshold decisions.
- **Composition emits into the existing HTN runtime** via a `CapabilityPlanExecutor` adapter around
  `agent/htn_executor.rs` — one executor, not two.
- Grounding tool (`list_installed_skills`) reads the CKB.
- Everything flag-gated; flag-off = byte-identical legacy. Tauri/event/`/api/chat` contracts preserved.

## 9. Prompt Routing & Planning Arbitration (the anti-proliferation design)
- **Single Planning Authority.** One arbiter decides, per turn, which runtime owns it: conversation,
  Settings NLP, GUI/HTN automation, n8n workflow, native tool, or the **Capability Reasoning
  Pipeline**. Inputs: semantic evidence (goal vs each domain corpus), declared effects, confidence,
  and explicit user intent (sticky). Output: one winner + rationale trace + `CapabilityEvent`.
- **Capability planning is a producer, not a competitor.** When the pipeline composes a multi-step
  plan, it does not execute directly — it hands a Solution Plan to the HTN runtime. So HTN remains the
  single execution-graph engine; CPP feeds it. n8n keeps its pre-fallback release-to-agent behavior.
- No keywords: destinations chosen by evidence/effects/confidence; policy weights/thresholds are config.
- Cheap negative gate first → non-capability turns pay ~no latency, byte-identical when flags off.

## 10. Capability Intelligence Architecture
- **CKB (trait):** `record/get/list/list_by_health/relationships/preferences/purge/snapshot`; SQLite,
  graph-capable schema; authoritative learned layer; storage-agnostic (future Memory redesign re-homes it).
- **CapabilityReasoner (trait):** `reason(goal, ctx) -> SolutionPlan { steps, path, confidence,
  rationale }`. Stages 1–5 of the pipeline; **anytime** (returns best-so-far under budget); **tiered**
  (fast path for high-confidence single cap; deep path otherwise); **planner-model seam** (may use a
  stronger model for reasoning when available).
- **CandidateSource (trait):** native / installed(CKB+index) / catalog / synthesizing — uniform.
- **ArgumentGenerator (trait, neutral):** grammar-constrained/structured, schema-validated, bounded repair.
- **CapabilityPlanner:** composes candidates into a typed, saga-structured Solution Plan; emits to HTN.
- **EvolutionEngine:** reads CKB, benchmarks proxies, proposes gated/auditable/reversible migrations.
- **Learning:** every execution + reflection updates the CKB (outcomes, adaptive trust, preferences,
  failure explanations, relationships).

## 11. Marketplace Architecture
Neutral catalogs from any provider; ranking uses `TrustInfo`/`QualitySignals`/`CostHint`/`UsageStats`/
adoption as inputs. Hash + signature verification; quarantine unverifiable; catalog cache with
invalidation (not a 2nd truth). Capability-ID namespacing across registries; semver descriptor
versioning; dependency metadata represented now (resolution deferrable). GitHub Skills today; ClawHub/
MCP/enterprise are just more catalog providers.

## 12. Provider Architecture
Trait extended only additively (`upgrade`/`replace` optional, default `Unsupported`; a synthesizing
provider implements `acquire` to *generate*). `acl/*` owns all native types. Arg-gen + marketplace
match-selection relocate to the neutral layer. Capability **kinds** include async/long-running/
streaming/remote — `CapabilityOutcome::{Value,Stream,Declined}` already supports this; add explicit
long-running/handle semantics for remote agents. Neutrality proven by a 2nd lifecycle provider + CI gate.

## 13. Skill/Capability Lifecycle (per-stage ownership)
| Stage | Owner | Store/Index/Cache | Events | Recovery |
|---|---|---|---|---|
| Understand/Analyze/Strategy | Reasoner | CKB (read) | reasoning trace | ask user on low conf |
| Discover | Reasoner→CKB→Index→Catalog | index+CKB | Discover | lexical-only if embedder down |
| Compare/Confidence | Reasoner | CKB signals | Select{candidates,confidence} | abstain below threshold |
| Compose | Planner→HTN adapter | — | Plan{graph} | reject bad IO chain at plan time |
| Download/Verify | LifecycleManager | staging; hash/sig | Verify:Ok/Failed | quarantine/delete staging |
| Install/Register/Index | provider.acquire (txn) | bundle+skills.db+registry+index | Acquire:Ok | rollback partial |
| Smoke/Activate | LifecycleManager | CKB state=Enabled | Smoke/Activate | rollback+report |
| Execute | providers via HTN | pool | Execute + step events | saga compensation |
| Reflect/Learn | Reasoner/Evolution | CKB (durable) | Learn | restored on boot |
| Upgrade/Migrate/Replace | Lifecycle/Evolution | atomic swap; grants if effects ⊆ | Upgrade/Evolve | rollback to prior |
| Delete/Cleanup | LifecycleManager | cascade bundle+registry+index+CKB+grants+relationships | Remove | idempotent reinstall |
| Recover(restart) | Platform boot | index rebuild+CKB restore+hash reconcile | refresh | — |

## 14. Production Hardening
Timeouts+bounded retries+cancellation on reason/acquire/verify/smoke/execute; async audit; **saga
compensation** on plan cancel/failure. Container-leak proof (pool baseline post-run + shutdown); fix
prewarm noise. Failure taxonomy → recovery. Correlated telemetry + `CapabilityEvent`s + reasoning/plan
traces. Secrets never logged/persisted.

## 15. Testing Strategy
Unit: CKB CRUD+durability+relationships; reasoner confidence + fast-path/budget; arg-gen constrained+
repair; planner typed-IO + saga rollback; lifecycle transactional rollback; cascade cleanup. Integration:
neutral path with `FakeProvider` + 2nd lifecycle provider; grant reuse across restart; plan-level
permission; HTN emission (no duplicate executor). Property: flag-off parity; grant monotonicity;
idempotent reinstall; budget-never-loops. Docker E2E: real substrate acquire/smoke/upgrade/compose/
remove. No fabricated results.

## 16. Real-UI Validation
Real Desktop + local llama.cpp (Qwen3-VL-4B). ≥100 prompts incl. composite multi-step goals; capture
per-prompt routing+arbitration, reasoning/plan trace, candidates+confidence, permission, outcome,
reply, and CKB/grant/index/log/event diffs; score vs R14; root-cause+regress every failure. Harness
evolves from `scripts/kria_campaign.sh`.

## 17. Risk Analysis
| Risk | Impact | Mitigation |
|---|---|---|
| Planner proliferation | turn-ownership conflicts | single authority + arbitration; CPP feeds HTN (R10) |
| Weak local model | bad reasoning/args/synthesis | planner-model seam; constrained arg-gen; confidence-gated depth; ask-user fallback |
| Reasoning latency/cost | slow turns | tiered fast path + reasoning budget |
| Composed/generated escalation | security | plan-level effects+permission; sandbox; verify (R11) |
| Partial plan failure | inconsistent state | saga compensation (R4.3) |
| Synthesis unreliable/unsafe | broken/dangerous caps | late, guarded, sandboxed, smoke-gated, lowest trust (R7) |
| CKB couples to Memory redesign | churn | strict trait boundary |
| Benchmarks lack oracles | wrong evolution | cheap proxies only; gated+reversible |
| Neutrality erosion | re-coupling | relocate cognition + CI gate + 2nd provider |
| Config split-brain | wrong runtime settings | reconcile authority in P0 |

## 18. Migration Strategy
Every phase flag-gated (default OFF→validated→ON). P0 reconciles config authority. CKB ships empty and
back-fills from registry + first executions. Composition routes through HTN behind a flag; legacy
single-capability path stays until the pipeline is validated, then trimmed as debt. Descriptor bumps
additive (minor)/negotiated (major); reversible down-migrations documented per phase. Evolution and
synthesis default OFF until their metrics pass.

## 19. Success Metrics
See R14 (grounding=100%, selection+compose accuracy, composite plan+saga, reuse+preferences,
permission-no-reprompt, lifecycle+evolution green, zero arg-abort, zero leaks/blank/loops). Tracked via
telemetry over time.

## 20. Production Exit Criteria
See requirements E1–E10. P12 is a gate, not a task: release only when all hold, with per-phase rollback.

---

## Architecture
See §5. In brief: **Prompt Intent Gate + Planning Arbitration → Capability Reasoning Pipeline
(tiered/budgeted/traced) → CapabilityPlatform { CKB, Reasoner(+ArgumentGenerator), Planner→HTN adapter,
LifecycleManager, EvolutionEngine, PermissionEngine+GrantStore, FederatedIndex } → ProviderRegistry →
CapabilityProvider (acl/*, all kinds) → execution substrate**. The Planner never executes; it emits a
Solution Plan into KRIA's existing HTN runtime, so there is exactly one execution-graph engine.

## Components and Interfaces
Neutral traits in `capability/intelligence/` (storage/provider-agnostic):
- **`CapabilityKnowledge`** — `record_install · record_outcome · get · list_installed · list_by_health
  · relationships · preferences · purge · snapshot`. Authoritative learned layer (CKB).
- **`CandidateSource`** — `candidates(need, ctx)`; impls: native, installed(CKB+index), catalog,
  synthesizing.
- **`CapabilityReasoner`** — `reason(goal, ctx) -> SolutionPlan`; anytime + tiered + budgeted; emits a
  reasoning trace; uses the planner-model tier when available.
- **`ArgumentGenerator`** — grammar-constrained/structured, schema-validated, bounded repair (neutral).
- **`CapabilityPlanner`** — `compose(need, candidates) -> SolutionPlan`; typed IO chaining; saga steps;
  emits into the HTN runtime via `CapabilityPlanExecutor`.
- **`LifecycleManager`** — `acquire_verified · smoke_test · activate · upgrade · replace · rollback ·
  delete` (transactional, cascading).
- **`EvolutionEngine`** — `evaluate · propose_migration · apply(gated) · record`.
- **Provider trait (additive):** optional `upgrade`/`replace`, default `Unsupported`; synthesizing
  provider implements `acquire` to generate.
Brain depends only on `CapabilityPlatform` builder methods (`with_knowledge/with_reasoner/with_planner/
with_lifecycle/with_evolution`), mirroring `with_events`.

## Data Models
- **`CapabilityKind`** enum: `Native | Installed | Gui | Browser | Docker | Workflow | CloudApi | Mcp |
  RemoteAgent | Human | Synthesized`.
- **`CkbRow`** (SQLite): `provider_id, capability_id, kind, name, descriptor_hash, descriptor_json,
  state, trust, adaptive_trust, effects_json, provenance, successes, total, last_outcomes(ring),
  failure_explanations, perf_history(latency/cost), health, first_seen, last_used`.
- **`CapabilityRelationship`**: `(from_key, rel: DependsOn|ComposedWith|SubstituteFor|SupersededBy,
  to_key, weight)` — first-class graph edges.
- **`Preference`**: `(scope: global|user|goal_class, key, value_json, source: learned|explicit, weight)`.
- **`SolutionPlan`**: `steps: Vec<PlanStep>, path: ExecutionPath (Reuse|Native|Compose|Acquire|Generate|
  Ask), confidence, plan_effects (union/max-risk), rationale, budget_used`.
- **`PlanStep`**: `capability_key, args, inputs_from(step refs), compensation (rollback action),
  timeout, on_error`.
- **Reused:** `CapabilityDescriptor` (+version/trust/quality/cost/usage), `Effects`, `ScopedGrant`,
  `CapabilityEvent`.
- **Persistence:** CKB + grants durable (SQLite, graph-capable); index derived/in-memory; catalog cache
  transient w/ invalidation.

## Correctness Properties

### Property 1: Flag-off parity
With intelligence flags off, behavior is byte-identical to the current CPP + existing runtimes.
**Validates: Requirements 12.1, 10.3**

### Property 2: Grounding soundness
Every "installed / what-can-you-do / why-failed" answer is a subset of the real registry/CKB.
**Validates: Requirements 1.3, 14.1**

### Property 3: Single planning authority
For any turn exactly one runtime is selected (ReAct/HTN/n8n/synthetic/CPP); capability planning emits
into the HTN runtime and never executes a second graph engine.
**Validates: Requirements 10.2, 4.2**

### Property 4: Confidence-gated action
No acquire/generate/execute occurs below the configured confidence threshold; the pipeline asks or
declines instead, and never exceeds the reasoning budget.
**Validates: Requirements 2.3, 3.3**

### Property 5: Plan-level permission (no silent escalation)
Permission is evaluated on the union of a plan's step effects at max risk; one approval covers the
plan; re-prompt only on effect widening.
**Validates: Requirements 11.1, 11.2**

### Property 6: Saga safety
A partial plan failure compensates completed steps or returns an honest partial result — never a silent
half-done state.
**Validates: Requirements 4.3, 12.1**

### Property 7: Transactional lifecycle + cascade delete
Failed verify/smoke leaves no partial artifacts; delete cascades bundle+registry+index+CKB+grants+
relationships; reinstall is idempotent.
**Validates: Requirements 5.2, 5.4**

### Property 8: Neutrality
No provider-name branch in `capability/` outside `acl/`; a second lifecycle provider works through the
identical path (CI-enforced).
**Validates: Requirements 9.1, 9.3**

### Property 9: Evolution safety
Every evolution action is auditable, reversible, and gated (user/confidence); no silent capability swap.
**Validates: Requirements 6.2, 10.2**

### Property 10: Recovery + learning persistence
After restart, CKB + grants persist and influence selection; the index rebuilds from `describe()`;
changed descriptor hashes reconcile.
**Validates: Requirements 1.2, 5.4**

### Property 11: Native sufficiency short-circuit
When a native/installed candidate meets the confidence+cost threshold, remote marketplace search and
generation are not performed (unless explicit user intent requests them).
**Validates: Requirements 3.6, 15.2**

### Property 12: Iterative reasoning terminates
The Hypothesis→Evaluate→Revise loop always terminates within the reasoning budget, returning the
best strategy so far or an honest ask — never an unbounded loop.
**Validates: Requirements 2.5, 2.7**

### Property 13: Explainability fidelity
Every user-facing "why X / why not Y" answer is derivable from the recorded Decision Records + reasoning
trace (no post-hoc fabrication).
**Validates: Requirements 16.1, 16.2**

### Property 14: Pre-activation gating
No acquired/synthesized capability reaches `Enabled` without passing sandbox + verify + smoke +
synthetic-prompt tests; any failure quarantines/rolls back.
**Validates: Requirements 21.1, 7.2**

### Property 15: Wiring integrity
In production paths the dispatcher/synthetic flows call the Reasoner, the Planner emits into the single
HTN executor, and the CKB is consulted before discovery — asserted by integration tests, not assumed.
**Validates: Requirements 25.1, 25.2**

### Property 16: Migration safety
CKB export/import + dual-write/shadow-read allow adopting the future Memory with a reversible cut-over
and zero capability-knowledge loss.
**Validates: Requirements 22.1, 22.2**

## Error Handling
Failures are classified → mapped recovery: transport→retry/backoff; schema→bounded arg repair (never
identical retry); permission→prompt/decline; provider-offline→breaker+decline; context-overflow→
`ContextTooLargeError`; verify→quarantine; smoke→rollback+report; plan-step→saga compensation; budget→
best-so-far or ask-user. Every terminal state emits a `CapabilityEvent` + reasoning/plan trace; no
silent failure, no fabricated success.

## Testing Strategy
See §15 (unit/integration/property/Docker-E2E) and §16 (real-UI ≥100-prompt campaign incl. composite
goals). Gates: flag-off parity (P1), grounding (P2), single-authority + HTN emission (P3),
confidence/budget (P4), plan permission (P5), saga (P6), transactional lifecycle+cascade (P7),
neutrality CI + 2nd provider (P8), evolution safety (P9), recovery+learning (P10). No fabricated results.

---

## 27. CKB → Future Memory Migration (dedicated)
The CKB is intentionally decoupled from the pending global Memory redesign, but the *path* is designed now:
- **Stable versioned interface:** `CapabilityKnowledge` is the only surface Brain callers use; the
  backend (SQLite today) is hidden. A `schema_version` is stored and negotiated.
- **Export/Import:** `snapshot()`/`restore()` produce a portable, versioned document of all capability
  knowledge, decision records, relationships, and preferences.
- **Cut-over plan:** (1) dual-write to old+new during a window; (2) shadow-read + compare; (3) flip
  reads to new; (4) retire old — each step reversible. No Brain caller changes.
- **Invariant:** capability knowledge is never lost or stranded; a failed migration rolls back to the
  prior backend. Validated in P13 (E14).

## 28. Deep Lifecycle Detail (per stage: owner / DB / cache / descriptor / index / events / permission / rollback / recovery)
| Stage | Owner | DB writes | Cache | Descriptor | Index | Events | Permission | Rollback | Recovery |
|---|---|---|---|---|---|---|---|---|---|
| Search/Discover | Reasoner→CKB→Index→Catalog | — | catalog cache | read | read | Discover | — | — | lexical-only if embedder down |
| Compare/Strategy | Reasoner+CostModel | Decision Record (draft) | — | read | read | Select | — | — | abstain/ask on low conf |
| Download | LifecycleManager→provider | staging meta | — | — | — | Acquire:Started | — | delete staging | retry/backoff |
| Verify | LifecycleManager | verify result | — | hash/sig checked | — | Verify:Ok/Failed | — | quarantine | re-download once |
| Sandbox+Smoke+Synthetic | LifecycleManager | test results→CKB | — | — | — | Smoke:Ok/Failed | — | rollback all | — |
| Install | provider.acquire (txn) | skills.db + bundle | — | generated | upsert | Acquire:Ok | — | remove partial artifacts | idempotent reinstall |
| Register | provider registry | skills.db | — | — | — | — | — | unregister | — |
| Index/Embed | Platform.refresh | — | — | — | rebuild/upsert | — | — | rebuild | rebuild from describe() |
| Activate | LifecycleManager | CKB state=Enabled | — | — | — | Activate | — | set Disabled | — |
| Execute | providers via HTN | CKB outcome + perf | — | — | — | Execute + step events | plan-level (R11) | saga compensation | breaker excludes offline |
| Update/Upgrade/Replace | Lifecycle/Evolution | CKB + skills.db (atomic swap) | invalidate | new version | upsert | Upgrade | preserve if effects ⊆ | restore prior | — |
| Retire (deprecate→archive→delete) | RetirementManager | CKB state + archive | invalidate | — | remove | Retire/Remove | revoke grants | recover from archive | reinstall idempotent |
| Delete/Cleanup | LifecycleManager | cascade: bundle+registry+index+CKB+grants+relationships | invalidate | — | remove | Remove | revoke grants | — | idempotent reinstall |
| Recover (restart) | Platform boot | — | rebuild | reconcile hash | rebuild | refresh | grants reloaded | — | CKB restore |

## 29. Cost Model, Decision Memory, Families, Benchmark, Retirement, Continuous Discovery
- **CostModel (R15):** `estimate(candidate|strategy, ctx) -> CostVector { latency_ms, gpu_mb, ram_mb,
  tokens, money, install_cost, maintenance }`. Sources: descriptor `CostHint` + CKB perf history;
  conservative defaults when unknown. Feeds strategy ranking + the native sufficiency gate.
- **Decision Memory (R16):** `DecisionRecord { goal, goal_class, candidates[], chosen, rejected[] with
  reasons, confidence, cost, risk, ts }` in the CKB; drives user-facing explainability and adjusts
  strategy priors over time.
- **Families (R17):** learned/declared `family` groups substitutable capabilities; the Reasoner picks
  within a family by multi-attribute trade-off (accuracy vs speed vs cost vs trust) for the goal-class.
- **BenchmarkHarness (R18):** runs candidates on golden/synthetic inputs in a sandbox → proxy scores →
  CKB; powers evolution + family selection; never blocks the fast path; honest about proxy-not-oracle.
- **RetirementManager (R19):** idle-decay + deprecation → archive (reversible) → delete (cascade);
  keeps the ecosystem clean without losing recoverability.
- **ContinuousDiscovery (R20):** background, off-by-default, budget/quiet-hours-bounded loop that writes
  upgrade/security/new-provider *proposals* to the CKB; elevated changes require R6.2 gating.

## 30. ClawHub Model (forward design, R8.5)
Publishing flow, reviews/ratings, trust tiers, cryptographic signatures, dependency resolution, update
channels (stable/beta), breaking-version signalling, and compat negotiation are all expressed through
the **existing neutral surfaces**: descriptors carry version/trust/signature/deps; the protocol version
negotiates breaking changes; catalogs rank uniformly. Adopting ClawHub is a new `acl/*` catalog
provider + descriptor fields — **zero Brain change** (R9.1). Implemented in P6 as design + schema; live
integration when ClawHub exists.

## 31. Wiring Verification & Concurrency (R24/R25)
- **Wiring proof:** integration tests assert real call paths (dispatcher→Reasoner; Planner→single HTN
  executor, asserting no second graph engine is instantiated; CKB consulted before discovery; grants
  read on authorize; outcomes+Decision Records written). A startup wiring smoke test (debug) asserts
  components are injected when flags ON and absent when OFF (parity).
- **Concurrency:** CKB + GrantStore transactional under parallel turns (no lost updates); execution
  respects container-pool limits with backpressure (no unbounded spawn). Reasoning policy + telemetry
  schemas are versioned and recorded in the trace for reproducibility/A-B.

## 32. Brain/Hands Invariant (R23)
**KRIA = Brain; OpenClaw = Hands. Intelligence belongs to KRIA; execution belongs to the provider.**
No reasoning/selection/strategy/arg-gen/decision logic in any `acl/*` provider — providers only
execute what the Brain decides. Guarded by the CI neutrality gate (no `crate::openclaw`/`mcp::client`
and no provider-name branches in `capability/` outside `acl/`) plus review of `acl/*` for cognition.

---

## 33. Frontend Architecture & Integration (SolidJS Desktop)
The capability-engineering layer is **not backend-only**. Frontend integration is additive and reuses
existing surfaces:
- **Stores:** extend `ui/src/stores/app.ts` (streaming turn state) with capability reasoning/plan/job
  state; a new `ui/src/stores/capability.ts` holds installed/marketplace/families/health/decisions/
  proposals/jobs, hydrated by `cpp_*` commands.
- **Chat panel:** during a turn, render a collapsible **Reasoning/Plan** panel from `capability:reasoning`
  and `capability:plan` events (goal-class, inferred requirements, strategies+rejections, chosen path,
  per-step confidence+cost). Reuses the existing `agent:token/done` streaming + thinking-guard.
- **Plan preview + approval:** composed/elevated plans render a step/effect preview and gate via the
  existing HITL modal (`approve_action`/`deny_action`) extended with a `cpp_plan_approve` path.
- **CapabilitiesView (extend):** tabs — Installed (grounded, `cpp_installed`), Marketplace (existing
  `clawhub_*` + `cpp_recommend`), Families (`cpp_families`), Health (`cpp_health`), Decisions/Why
  (`cpp_decisions`), Evolution Proposals (`cpp_proposals` + approve/undo), Jobs (`cpp_jobs` + controls),
  Timeline (existing `cpp_timeline`), Descriptor (existing `cpp_descriptor`).
- **PermissionManagerView (reuse):** plan-level grants via existing `cpp_list_grants`/`cpp_revoke_grant`.
- **Oversight/Activity feed:** new small view fed by `capability:proposal`/`capability:evolution` +
  `cpp_activity`; autonomy-level control in Settings (SQLite-backed).
- **Web/headless parity:** read paths mirrored under `/api/capability/*` in `local_api.rs`.
- i18n via `ui/src/locales/*`; all panels flag-gated + graceful when a backend component is off.

## 34. End-to-End Wiring Map (no component without its full row)
> Additive only. Existing names preserved. New = to build. Every row: component → command → event →
> frontend → existing-KRIA touchpoint.

| Backend component | Tauri command(s) | Event(s) | Frontend | Existing touchpoint |
|---|---|---|---|---|
| CapabilityKnowledge (CKB) | `cpp_installed`,`cpp_families`,`cpp_health`,`cpp_decisions` (new) | `capability:ckb_updated` (new) | CapabilitiesView tabs, capability.ts store | grounds `list_installed_skills` in `loop_engine`; DB `cpp_knowledge.db` |
| Reasoner + StrategyGen + CostModel | (internal; surfaced via trace) | `capability:reasoning` (new) | Chat Reasoning panel | `agent/loop_engine`; dispatcher `capability_dispatch.rs` |
| ArgumentGenerator (neutral) | — | — | — | relocated from `openclaw/arg_gen.rs` |
| CapabilityPlanner → HTN | `cpp_plan_preview`,`cpp_plan_approve` (new) | `capability:plan`,`capability:plan_step` (new) | Plan preview + HITL modal | `agent/htn_executor.rs` (single executor), `approve_action`/`deny_action` |
| LifecycleManager | `cpp_install`,`cpp_uninstall`,`cpp_upgrade`,`cpp_retire` (new) + existing `clawhub_install_skill`/`clawhub_uninstall_skill`/`clawhub_toggle_skill` | `capability:lifecycle` (new) | Marketplace + Installed tabs, `SkillMarketplace.tsx`, `PermissionModal.tsx` | `acl/openclaw.rs`, `skills.db`, `cpp_knowledge.db`, index refresh |
| PermissionEngine + GrantStore (plan-level) | existing `cpp_authorize`,`cpp_approve`,`cpp_list_grants`,`cpp_revoke_grant`; `openclaw_revoke_grant` | `capability:approval_required` (new; complements `agent:approval_required`) | `PermissionManagerView`, HITL modal | `cpp_grants.db`, `permission.rs` |
| EvolutionEngine | `cpp_proposals`,`cpp_proposal_apply`,`cpp_proposal_undo` (new) | `capability:proposal` (new) | Evolution Proposals tab + Oversight feed | `cpp_knowledge.db`, autonomy-level config |
| BenchmarkHarness | `cpp_benchmark_run`,`cpp_benchmark_results` (new) | `capability:benchmark` (new) | Families/Health tabs | sandbox/Docker substrate |
| RetirementManager | `cpp_retire`,`cpp_recover` (new) | `capability:lifecycle` | Installed tab | cascade cleanup |
| SynthesisProvider | via `cpp_install` (acquire) | `capability:synthesis` (new) | Marketplace (Generate) | `acl/synth.rs` (new), seccomp sandbox |
| ContinuousDiscovery | `cpp_discovery_status` (new) | `capability:proposal` | Oversight feed | background task; config-gated |
| Long-running Jobs | `cpp_jobs`,`cpp_job_control` (new) | `capability:job_progress` (new) | Jobs tab + chat progress | durable `cpp_jobs.db` (new), HTN runtime |
| Timeline/Trace | existing `cpp_timeline` | `capability:*` (all) | Timeline tab | `capability::events` bus |
| Autonomy level + settings | `cpp_set_autonomy`,`cpp_get_autonomy` (new) | — | Settings view | SQLite config store |
| Startup wiring | (registration) | — | — | `runtime.rs` CPP block; wiring smoke test (R25.2) |
| Headless parity | `/api/capability/*` (new) | SSE mirror | web mode | `local_api.rs` (mirrors `/api/chat`) |

Databases: `cpp_grants.db` (existing), `cpp_knowledge.db` (new CKB), `cpp_jobs.db` (new), `skills.db`
(existing audit/registry). Config: autonomy level + flags in SQLite config store. Events: all under the
existing `capability::events` bus + emitted to the desktop event channel like `agent:*`.

## 35. Long-running & Resumable Jobs
A Solution Plan flagged long-running becomes a **Job** with durable state in `cpp_jobs.db`
(queued/running/paused/failed/done + per-step progress + partial results). The HTN runtime executes
steps; each idempotent step records completion so a resumed job skips it. `capability:job_progress`
streams to the Jobs tab + chat; `cpp_job_control` supports pause/resume/cancel (saga-compensating).
Jobs respect concurrency/backpressure (R24) + rate/cost ceilings (R26). Survives restart: on boot,
paused/running jobs are restored and offered for resume.

## 36. Autonomy Oversight & Control Surface
An **autonomy level** (manual / propose-only / auto-with-notice / full-auto, per risk tier) in the
SQLite config governs when evolution/discovery/auto-acquire act without asking (default: propose-only
for elevated effects). Every autonomous action writes a Decision Record + an **Activity feed** entry
(what/why/undo). Elevated/irreversible actions above the level require explicit approval. Undo reverses
via the lifecycle rollback/retirement paths. Surfaced in the Oversight view + Settings.

## 37. Cost Calibration & Correctness-vs-Liveness
CostModel estimates are **calibrated from actuals**: each execution records measured latency/resource/
token/money into CKB perf history, refining future estimates (conservative defaults when uncalibrated).
Explicit limit: verification/smoke/benchmark prove **liveness** (ran, schema-valid, no error), not
semantic **correctness**. For **correctness-critical** goal-classes, KRIA surfaces the output for user
confirmation or runs a validation capability, and never claims correctness it did not verify.

## 38. Synthesis Sandbox Binding
Synthesized-capability execution is bound to the existing hardened sandbox
(`config/seccomp/kria-seccomp.json` + OpenClaw Docker substrate), lowest resource-class + trust tier,
effect-declared, artifact/signature-verified, smoke+synthetic-tested before activation. Generated code
never runs on the host. This reuses the substrate KRIA already ships — no new execution surface.

---

## Additional Correctness Properties

### Property 17: Frontend parity & contract stability
Every capability action is drivable + observable in the Desktop UI via additive `cpp_*` commands and
`capability:*` events; existing command/event names and `/api/chat` shape are unchanged; no dangling
command/event/view.
**Validates: Requirements 27.1, 31.2**

### Property 18: Job durability & resumption
A long-running job survives restart, resumes without repeating completed idempotent steps, and
compensates on cancel/failure.
**Validates: Requirements 28.1, 28.3**

### Property 19: Autonomy bound
No elevated/irreversible autonomous action occurs above the configured autonomy level without explicit
approval; every autonomous action is auditable + reversible + shown in the oversight feed.
**Validates: Requirements 29.2, 29.3**

### Property 20: Honesty on correctness
KRIA never claims a result is correct beyond verified liveness; correctness-critical goal-classes
surface for confirmation; synthesized code runs only under the seccomp-bound sandbox.
**Validates: Requirements 30.2, 11.4**
