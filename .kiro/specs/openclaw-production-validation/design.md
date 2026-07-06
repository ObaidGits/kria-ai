# Design Document

Feature: OpenClaw Production Validation & Hardening

## Overview

This design describes the **validation-and-hardening architecture** for OpenClaw. It does **not**
redesign A0–A9 (architecture is LOCKED). It defines how each requirement (R1–R20) is objectively
proven on the real system, how gaps found by validation are hardened behind feature flags with
flag-OFF parity, and how the results aggregate into the freeze gate (R20).

Two truths drive the design:

1. **Validation is code, not opinion.** Every requirement maps to an automated check (CI-safe test,
   live gate, soak probe, or benchmark scenario) that emits an evidence record. The production verdict
   (R10) is computed from those records.
2. **Hardening is surgical.** Where validation fails, the fix is scoped to the failing requirement,
   guarded by a feature flag, and asserted flag-OFF-identical to prior behavior. Architecture is not
   touched.

### Grounding in the real codebase (verified, not assumed)

- Config: `OpenClawConfig` under `[openclaw]` in `~/.kria/config.toml`, `enabled: false` by default
  (`crates/kria-core/src/openclaw/config.rs`).
- Registry default index URL is `https://raw.githubusercontent.com/kria-ai/kria-skills/main/index.json`
  (`clawhub.rs::DEFAULT_REGISTRY_URL`). **This differs from the audit-referenced
  `ObaidGits/kria-skills` repo** — the drift (index vs local DB, and default-URL vs user-URL) is a
  concrete R3.5 finding, validated against the **test rig**, never the live public repo.
- Entry: `register_semantic_openclaw` exposes a single `openclaw` tool; `SemanticOpenClawHandler` is the
  handler; routing is `SemanticSkillRouter::route(RoutingIntent) -> RoutingDecision`
  (`handler.rs`, `semantic_router.rs`).
- Runtime: `RuntimeManager`, `RuntimeRegistry`, `DockerRuntime`, `ContainerPool`, internal
  `RuntimeScheduler`/`HealthMonitor`/`RecoverySystem` (`runtime_manager.rs`, `runtime/docker.rs`,
  `pool.rs`), JSON-RPC via `bridge.rs`.
- **Execution Engine (A7):** real module `crates/kria-core/src/execution/` — `ExecutionEngine`,
  `ExecutionPlanner`/`Goal`/`PlanStep`, `ExecutionGraph`/`GraphNode`/`NodeKind`, `ExecutionScheduler`,
  `ExecutionContext`/`Artifact`, `Executor`/`ExecutorRegistry`/`ExecutorKind`, `ExecutionEventStream`,
  `ExecutionMetrics`, `RecoveryManager`/`RecoveryPolicy`, `DependencyResolver`, `GraphOptimizer`, and
  `OpenClawExecutor` (`execution/executors/openclaw.rs`, wraps `DockerRuntime`/`RuntimeManager`). The
  planner is executor-agnostic; OpenClaw is one executor. This subsystem is validated explicitly (§A7
  Execution Engine validation), never as a black box.
- Trust/admission: `admission.rs`, `approval.rs`, `revocation.rs`, `TrustConfig` (community/verified/
  local tiers, `verified_skips_hitl`) — validated in §Trust & revocation validation.
- Generation: `generation/{designer,codegen,llm_generator,validator,quality,sandbox,pipeline,approval,
  budget,decision}` produces bundles under `bundle/`.
- A legacy path exists: `register_skill` is `#[deprecated]`. R11 validation must confirm chat uses the
  semantic path and the deprecated path is unreachable in production.

## Architecture

### Validation layers

```
Layer 0  CI-safe unit/integration        (no Docker, fixtures + fakes)  → every requirement's logic
Layer 1  Test-rig integration            (real Docker, local image,     → R2,R3,R4,R6,R12
                                          fixture marketplace)
Layer 2  Live gate                        (real desktop, OpenClaw ON,    → R1,R4,R5,R16 acceptance
                                          real prompt/flow)
Layer 3  Failure injection                (fault harness over Layer 1)   → R7
Layer 4  Soak                             (sustained Layer 1 workload)   → R18
Layer S  Scale                            (large fixture repo, 1000+     → R3,R4,R9,R18 headroom
                                          skills / 100+ publishers)
Layer 5  Production benchmark             (mixed workload + faults +     → R20 freeze gate
                                          pressure, clean state)
```

Cross-cutting probes run within the layers above: **A7 engine probe** (Layer 0 with `MockExecutor` +
Layer 1/2 via `OpenClawExecutor`), **Trust & revocation** and **Concurrency probe** (Layer 1/3), and the
**Regression suite** (every layer, every iteration).

Each layer emits `EvidenceRecord`s to a single results store; R10/R20 aggregate them.

### Harness placement

The validation harness extends the existing **`kria-eval`** crate (which already has `runner.rs`,
`suite.rs`, `judge.rs`, `report.rs`, `sandbox.rs`, `integration_eval/`, `workflow_eval/`). We add a new
module tree `crates/kria-eval/src/openclaw_eval/` rather than a new crate, to reuse the runner/report/
judge plumbing.

```
crates/kria-eval/src/openclaw_eval/
├── mod.rs                 # suite registration + EvidenceRecord types
├── rig.rs                 # test-rig lifecycle (Docker image build/verify, fixture repo server)
├── fixtures/              # fixture skill bundles + index.json variants (incl. drift case)
├── leak_detector.rs       # container/lease/process/GPU baseline snapshot + diff
├── fault_injector.rs      # Docker stop, container kill, bridge stall, repo 500/malformed
├── telemetry_assert.rs    # assert every action emitted a correlated record (R17)
├── ui_sync_probe.rs       # drive desktop commands + assert event/UI reflects backend (R16)
├── pipeline_trace.rs      # assert Root Router → ... → Response path per run (R11)
├── engine_probe.rs        # A7 execution-engine subsystem validation (R4/R11)
├── installer_matrix.rs    # all sources → one installer + trust/revocation (R3/R6/R12/R13)
├── concurrency_probe.rs   # parallel/race/deadlock/contention validation (R2/R6/R18)
├── scale.rs               # large-marketplace + routing/lookup-under-scale validation (R3/R4)
├── soak.rs                # sustained workload driver + periodic baseline checks (R18)
├── upgrade.rs             # old-state → upgrade → assert preserved/migrated (R19)
├── regression/            # permanent regression suite (one test per hardened bug; runs every iteration)
├── report.rs (ext)        # freeze report bundle generator (see §Freeze report bundle)
└── benchmark.rs           # R20 mixed workload + faults + pressure, freeze-gate scorer
```

Live-gate (Layer 2) drives the **same UI backend path** the app uses (desktop command / local API),
never a private shortcut, so a live pass proves the real user path.

### Test rig (isolation from the live public repo)

- **Docker image:** validation builds/pins a local tag (e.g. `kria/openclaw-substrate:test`) so R2.5
  image-verification is exercised deterministically; never depends on registry pulls at test time.
- **Fixture marketplace:** a local static file server serves controlled `index.json` + `.ocskill`
  bundles from `openclaw_eval/fixtures/`. Config `registry.index_url` is pointed at the fixture server
  for validation. Fixtures include:
  - a valid signed skill,
  - a skill with a bad hash (R3.3),
  - an invalid manifest (R3.3),
  - a **drift fixture**: `index.json` listing 1 skill while the seeded DB holds 3 (reproduces the
    audit's 1-vs-3 finding for R3.5),
  - a malformed `index.json` (R7.4).

### Evidence + results model

```rust
struct EvidenceRecord {
    requirement: String,      // "R3.2"
    layer: Layer,             // Ci | Rig | Live | Fault | Soak | Scale | Benchmark
    name: String,             // human label
    outcome: Outcome,         // Pass | Fail | Skipped(reason)
    metrics: Map<String, Value>, // durations, counts, container baseline delta, ...
    correlation_id: Uuid,     // links to telemetry (R17)
    evidence: Vec<String>,    // log excerpts, docker ps snapshots, telemetry ids
    timestamp: DateTime,
}
```

`report.rs` renders per-requirement pass/fail with evidence; the freeze-gate scorer (R20) requires every
R1–R19 requirement to have ≥1 Pass and 0 Fail across its mapped layers before emitting a "frozen".

## Components and Interfaces

### 1. Rig manager (`rig.rs`)
- `TestRig::up()` → verifies Docker, builds/verifies pinned test image, starts fixture repo server,
  points a scoped `OpenClawConfig` at the fixtures, seeds the skills DB for drift cases.
- `TestRig::down()` → stops fixture server, reaps all test containers, restores config, asserts baseline.
- Guarantees isolation: uses a dedicated container name prefix + a temp `~/.kria` root so validation
  never touches the user's real skills/DB.

### 2. Leak detector (`leak_detector.rs`)
- `baseline()` snapshots: OpenClaw container count (`docker ps` filtered by prefix), active leases (pool
  API), child processes, and GPU memory (nvml) when present.
- `assert_returned_to(baseline)` diffs after each run/scenario. Used by R2.4, R7.5, R18.2/18.5, R20.4.

### 3. Fault injector (`fault_injector.rs`)
- `stop_docker()/start_docker()`, `kill_container(id)`, `stall_bridge(id)`, `repo_status(500)`,
  `repo_malformed()`. Each returns a handle that auto-restores on drop. Drives R7 and R20.2.

### 4. Pipeline tracer (`pipeline_trace.rs`)
- Subscribes to telemetry emitted along the **canonical runtime path**:
  `Root Router → openclaw tool → SemanticSkillRouter::route → ExecutionEngine → OpenClawExecutor →
  RuntimeManager → DockerRuntime → container → skill → response`. `OpenClawExecutor` is an explicit,
  asserted stage (never hidden).
- `assert_canonical_path(run_id)` verifies each stage produced an ordered record; fails on any missing/
  reordered stage or a run that reached the runtime without a Root Router record (R11.1/11.2).
- Also asserts the `#[deprecated] register_skill` path emitted nothing (unreachable in production).

### 5. Installer matrix (`installer_matrix.rs`)
- Feeds one skill through each source: fixture marketplace, a local git-style repo dir, a local
  `.ocskill` file, and an A9-generated bundle. Asserts all four produce structurally identical registry
  entry + filesystem layout + DB row (provenance differs only as metadata) → R12.4, R13.1/13.2.
- Asserts a single verify→materialize→register code path is invoked for all (instrumented counter).
- **Trust & revocation** (extends `admission.rs`/`approval.rs`/`revocation.rs`, `TrustConfig`): asserts
  tier-based admission (community/verified/local), `verified_skips_hitl` policy, unsigned/tampered
  bundle rejection, and that revoking a publisher or a skill propagates to the registry + marketplace
  view so the revoked artifact is no longer installable/executable → R3.2, R6.2. (See §Trust & revocation
  validation.)

### 6. Telemetry asserter (`telemetry_assert.rs`)
- For each executed action in {install, update, remove, execute, generate, repair, container_create,
  container_destroy, marketplace_sync, router_select, path_traverse, failure, cancel}, asserts exactly
  one correlated `EvidenceRecord`/audit entry exists with outcome + timing + correlation id (R17).

### 7. UI-sync probe (`ui_sync_probe.rs`)
- Invokes the real desktop/Tauri command surface and the emitted events; asserts the UI-facing state
  (skill list, container status, health, sync, generation progress) reflects the backend within a
  bounded time, and that a dropped event reconciles on next poll (R16.4). Does not require a rendered
  DOM — asserts at the command/event contract the UI binds to (per structure.md: never change command/
  event names).

### 8. Soak driver (`soak.rs`) and Upgrade harness (`upgrade.rs`) and Benchmark (`benchmark.rs`)
- Soak: runs a long mixed workload, sampling leak-detector baselines + DB consistency periodically (R18).
- Upgrade: materializes a prior-version state fixture, runs the migration, asserts preservation +
  idempotency + fail-safe (R19).
- Benchmark: orchestrates the R20 workload (100 prompts / 50 installs / 20 updates / 20 removals / 20
  generated) interleaved with restart/crash/cancel/timeout + parallel + memory/GPU pressure, then runs
  the freeze-gate scorer.

### 9. Engine probe (`engine_probe.rs`)
- See §A7 Execution Engine validation.

### 10. Concurrency probe (`concurrency_probe.rs`)
- See §Concurrency validation.

### 11. Scale harness (`scale.rs`)
- See §Scale validation.

### 12. Regression suite (`regression/`)
- See §Permanent regression framework.

## A7 Execution Engine validation

The A7 subsystem (`crates/kria-core/src/execution/`) is validated explicitly — never as a black box.
`engine_probe.rs` exercises each single-authority owner named in `execution/mod.rs` and asserts OpenClaw
runs as one executor among a pluggable set, so future executors (GUI/Native/MCP/Browser/Memory/Cloud/
Agent) plug into the same validated interface without redesign.

| A7 concern | Owner (real symbol) | Validation |
|---|---|---|
| Planning | `ExecutionPlanner` / `Goal` / `PlanStep` | a Goal produces a correct `ExecutionGraph`; planner contains zero executor-specific logic (asserted) |
| Graph | `ExecutionGraph` / `GraphNode` / `NodeKind` / `ExecutorKindTag` | graph correctness: nodes, edges, node kinds; parallel / conditional / loop / barrier / subgraph / merge nodes resolve as declared |
| Scheduling | `ExecutionScheduler` / `ScheduleResult` / `ScheduleStatus` | ready-node scheduling, parallel dispatch, backpressure, no starvation |
| Context | `ExecutionContext` / `Artifact` | artifacts flow between nodes; context isolation per run |
| Executor interface | `Executor` / `ExecutorRegistry` / `ExecutorKind` / `ExecutorHealth` / `ExecutionRequest` | executor registration, lookup by kind, **replacement** of a registered executor, health reporting |
| OpenClaw executor | `OpenClawExecutor` (wraps `DockerRuntime`/`RuntimeManager`) | OpenClaw node dispatches into the runtime; boot wiring lives only in the executors boundary (asserted) |
| Dependencies | `DependencyResolver` / `DependencyIssue` | dependency resolution + cycle/missing-dep detection |
| Recovery | `RecoveryManager` / `RecoveryPolicy` / `RecoveryAction` / `RecoveryOutcome` | retry (then-succeed and exhausted paths — mirrors `execution/tests.rs`), rollback, checkpoint, recovery under failure |
| Events | `ExecutionEventStream` / `ExecutionEvent` | event ordering matches execution order; every node start/finish/fail emits an event (feeds R17) |
| Metrics | `ExecutionMetrics` / `ExecutionMetricsSnapshot` | per-node/per-run durations + outcomes recorded and accurate |
| Optimization | `GraphOptimizer` / `OptimizationReport` | optimizer preserves graph semantics (optimized graph ≡ result of unoptimized) |
| Cancellation | engine + scheduler | cancelling a run stops scheduling, cancels in-flight nodes, cleans the OpenClaw container (ties Property 2) |

Acceptance: engine-probe scenarios pass at Layer 0 (with a `MockExecutor`, as in `execution/tests.rs`)
**and** at Layer 1/2 through `OpenClawExecutor` against real Docker, and the pipeline tracer confirms
the `ExecutionEngine → OpenClawExecutor → RuntimeManager` segment for every real run.
**Validates: R4, R11.**

## Trust & revocation validation

`installer_matrix.rs` (trust extension) exercises `admission.rs`, `approval.rs`, `revocation.rs`, and
`TrustConfig`:

- **Trust tiers:** community / verified / local admission behaves per `TrustConfig`; `verified_skips_hitl`
  honored; `community_allows_network` enforced; unknown-source default tier applied.
- **Publisher verification:** a verified publisher's signature validates; an unknown/invalid publisher is
  treated per policy (HITL / reject).
- **Unsigned / tampered bundles:** rejected at admission (parity with R3.3), nothing registered.
- **Revocation:** revoking a **publisher** or a **skill** propagates to the registry and the marketplace
  view; the revoked artifact becomes non-installable and, if already installed, non-executable on next
  route (routing declines it).
- **Approval-bypass rules:** only the configured tiers bypass HITL; no path silently bypasses approval
  (ties Property 5).

**Validates: R3.2, R3.3, R6.2.**

## Concurrency validation

`concurrency_probe.rs` drives real concurrent load against the runtime, registry, pool, and scheduler,
asserting correctness and absence of races/deadlocks. It reuses A0–A9 components (no new locking system).

- **Parallel operations:** concurrent installs, uninstalls, enable, disable, executions, and generations.
- **Same-target races:** concurrent install+uninstall of the **same** skill, concurrent enable+disable,
  concurrent execute of a skill being uninstalled → deterministic, consistent end state (no partial rows).
- **Contention:** `ContainerPool` lease contention, `RuntimeScheduler` contention, SQLite write
  contention → bounded waits, no lost updates.
- **Deadlock / livelock:** a watchdog asserts every concurrent scenario completes within a bounded time;
  a timeout is reported as a deadlock/livelock failure (never silently retried to a fake pass).
- **Lock ordering / thread safety:** concurrent scenarios run under stress repetition; any inconsistent
  state or panic is a failure.
- **Recovery under concurrent failure:** inject a container crash while other runs are in flight; assert
  only the affected run fails and the rest complete, baseline restored.

**Validates: R2.1–R2.4, R6.1–R6.5, R18.2–R18.4.**

## Permanent regression framework

Every bug found and fixed during hardening MUST produce a permanent regression test before the fix is
considered done. Rules:

1. **No fix without a regression test.** A hardening change is not "done" (iteration gate) until a named
   test reproducing the original failure is added to `openclaw_eval/regression/` and passes with the fix,
   and is asserted to **fail** with the fix reverted (proves it guards the bug).
2. **Mandatory forever.** Regression tests are never deleted or weakened; the freeze gate requires the
   full regression suite green.
3. **Runs every iteration.** The regression suite is part of the per-iteration "no regression" step, so a
   previously fixed bug can never silently return.
4. **Traceable.** Each regression test references the requirement + the fix it guards (name convention
   `regr_<Rxx>_<slug>`), and emits an `EvidenceRecord` under the requirement it protects.

**Validates: iteration gate (all requirements); feeds R10/R20 evidence.**

## Scale validation

`scale.rs` validates behavior at marketplace/registry scale so a freeze cannot hide an O(n) cliff. It uses
a generated **large fixture repo** (never the live public repo) and the real registry/router/runtime.

- **Scale fixture:** ≥ 1000 skills across ≥ 100 publishers in a generated `index.json` + bundle set.
- **Marketplace under scale:** full sync, incremental/delta sync, search, and sort over the 1000+ index;
  assert correctness + bounded latency.
- **Registry lookup under scale:** `ProductionSkillRegistry` lookup / `get_enabled_skills` latency stays
  bounded with 1000+ installed skills.
- **Semantic routing under scale:** `SemanticSkillRouter::route` selects the correct skill with 1000+
  candidates within a bounded latency budget (no accuracy collapse).
- **Parallel at scale:** parallel installs, parallel updates, and parallel searches against the large
  repo (composes with the concurrency probe).
- **Execution / generation under scale:** execution and generation remain correct when many skills are
  installed.
- **Degradation analysis:** record memory usage, SQLite DB growth, startup time, search latency, and
  install latency as scale metrics; assert they stay within stated budgets (regression on these is a
  scale failure, not a silent slowdown).

Scope note: R3/R20 acceptance counts (50 installs / 100 prompts) remain the functional gate; scale
validation is an **additional** production-hardening layer (Layer `Scale`) proving headroom, not a
replacement for the functional gates.

**Validates: R3 (scale headroom), R4 (routing/exec at scale), R9 (metrics), R18 (resource growth).**

## Requirement → validation mapping

| Req | Primary layer(s) | Key check |
|-----|------------------|-----------|
| R1  | Live + Rig | enable/disable from Settings drives runtime up/down; teardown baseline; Docker-absent = honest `unavailable` |
| R2  | Rig + Fault | acquire/reuse/release, unhealthy eviction, timeout/cancel cleanup, image verify, bridge robustness; leak assert |
| R3  | Rig | fixture index sync count, verify+install, bad-hash/bad-manifest abort, drift (1-vs-3) surfaced, offline graceful |
| R4  | Live + Rig + Trace | matched prompt runs skill in container, returns real output, path recorded, capability enforced, released |
| R5  | Live + Rig | design→codegen→validate→test→package truthful; repair-or-abort; installs+runs via normal path; budget/approval; Dev-Mode gating |
| R6  | Rig | disable/enable routing, uninstall cleanup, update supersede, hot reload or explicit restart, registry/fs/DB consistency |
| R7  | Fault | Docker down, crash mid-run, bridge stall, repo unreachable/malformed → clear error + baseline restored |
| R8  | Live | Settings exposes required controls; actions reflect real outcome; non-ready features hidden/marked |
| R9  | Rig + Live | run counts/outcomes/durations accurate; container counts match `docker ps`; health honest; audit written |
| R10 | Aggregate | verdict computed from evidence; remaining work classified; reproducible from clean state |
| R11 | Trace | canonical path for 100% of runs; no bypass; short-circuits recorded; deprecated path silent |
| R12 | Installer matrix | all sources converge to one verify→materialize→register pipeline; identical results |
| R13 | Installer matrix + Trace | generated ≡ authored bundle/install/exec/telemetry/lifecycle; no `is_generated` branch |
| R14 | Live | all normal settings set from UI, persisted, adopted; no file editing required; UI shows persisted values |
| R15 | All | no fake success / mock UI / silent bypass; incomplete = honest state or Dev-Mode; TODOs enumerated |
| R16 | UI-sync probe | backend changes reflect in UI within bound; dropped-event reconciliation; no contradiction |
| R17 | Telemetry asserter | every listed action → one correlated record with outcome+timing |
| R18 | Soak | bounded memory; container/lease/GPU return to baseline; DB consistent; pool healthy; desktop responsive |
| R19 | Upgrade | skills/registry/marketplace/generated/settings/DB preserved; migrate or fail-safe; idempotent |
| R20 | Benchmark | full mixed workload + faults + pressure pass; freeze-gate enumeration all green; reproducible |

Cross-cutting validation components (each feeds the requirements noted):

| Component | Layer(s) | Feeds |
|-----------|----------|-------|
| A7 engine probe (`engine_probe.rs`) | CI + Rig + Trace | R4, R11 |
| Trust & revocation (installer-matrix ext) | Rig | R3.2, R3.3, R6.2 |
| Concurrency probe (`concurrency_probe.rs`) | Rig + Fault | R2, R6, R18 |
| Scale harness (`scale.rs`) | Scale | R3, R4, R9, R18 |
| Regression suite (`regression/`) | every layer, every iteration | all requirements (guard) |

## Freeze report bundle

`report.rs` (extended) generates the freeze report bundle automatically from the `EvidenceRecord` store —
nothing manual. On a freeze-gate run it emits:

- **Architecture Report** — components validated + single-authority invariants confirmed (A0–A9 unchanged).
- **Coverage Report** — per-requirement (R1–R20) Pass/Fail/Skipped with evidence links.
- **Execution Report** — A7 engine-probe results (planner/graph/scheduler/executor/recovery/events).
- **Marketplace Report** — sync/install/verify/trust/revocation + scale results.
- **ASGS Report** — A9 generation → validate → repair → package → install → execute results (real LLM).
- **Performance Report** — durations, latencies, startup time from metrics.
- **Stress Report** — soak + concurrency + benchmark pressure results.
- **Regression Report** — full regression-suite status (must be green).
- **Risk Report** — open risks with severity.
- **Known Issues** — honest list of failing/degraded/experimental items.
- **Technical Debt** — enumerated TODOs/placeholders reachable in production (ties R15.5).
- **Production Readiness Score** — computed per component + overall.
- **Go / No-Go** and **Freeze Verdict** — derived from the freeze gate (below).

**Validates: R10.1, R10.2, R20.5, R20.7.**

## Real-LLM policy (ASGS / A9)

- **Layer 0 (CI):** A9 generation MAY use the fixture LLM (`kria-eval/llm_fixture.rs`) to validate
  pipeline wiring deterministically. A fixture-LLM pass NEVER counts toward production readiness.
- **Layer 2 (production validation):** A9 validation MUST use the **real configured LLM backend**
  (local llama-server or the configured cloud model). Only a real-LLM generation → install → execute
  run satisfies R5/R13 for the freeze verdict.
- The evidence record carries the LLM mode (`fixture` | `real`); the freeze scorer rejects a "frozen"
  verdict if any A9 requirement's satisfying evidence is `fixture` (ties Property 5 honesty).

**Validates: R5, R13.**

## Freeze gate — evidence rule (Skipped ≠ Passed)

The architecture freeze / production-ready verdict (R10/R20) SHALL NOT succeed unless **real execution
evidence** is present:

1. For R1, R4, R5, R14, R16 (and the A7/Trust/Concurrency/Scale/Marketplace components), the freeze gate
   requires a Layer-2 (Desktop) and/or Layer-1 (real Docker/Runtime/Execution/Marketplace) evidence
   record with `Outcome::Pass`.
2. `Outcome::Skipped(reason)` is treated as **not satisfied** for freeze purposes — a Skipped live/Docker/
   runtime/execution/marketplace check can never stand in for a Pass.
3. If the environment cannot provide real execution (no Docker/desktop), the freeze scorer emits
   **No-Go** with the missing evidence listed — it never emits "frozen" on CI-only evidence.
4. A9 evidence must additionally be `real`-LLM (per Real-LLM policy) to count.

**Validates: R10.1, R10.3, R20.5, R20.7; enforces Property 9.**

## Feature-flag & parity model (hardening)

Every hardening change is guarded by a named flag (e.g. `openclaw_drift_surface`, `openclaw_ui_sync`),
defaulting OFF. Design rules:

1. Flag-OFF path is byte-for-byte prior behavior, asserted by a dedicated parity test per fix.
2. A fix's Layer-2 live gate must pass with the flag ON before the requirement is marked done.
3. Flags are exposed per R14 (Developer Mode / experimental where not yet production).
4. The freeze gate (R20.5) is evaluated with all production flags in their intended production state.

## Data Models

- **Fixture `index.json`** mirrors the real ClawHub schema (`clawhub.rs`), with variant files for
  valid / bad-hash / bad-manifest / drift / malformed.
- **`.ocskill` bundle** uses the real bundle format under `openclaw/bundle/` so installer-matrix
  convergence (R12) is meaningful.
- **Prior-version state fixture** (R19): a serialized skills DB + registry + settings from an older
  layout, checked into `openclaw_eval/fixtures/upgrade/`.

## Error Handling

- All validation failures are non-fatal to the harness: a scenario failure produces `Outcome::Fail`
  with evidence and continues, so one run yields a complete requirement scorecard.
- Rig teardown is best-effort but always asserts baseline; a teardown leak is itself a recorded R2/R18
  failure, never swallowed.
- Fault handles auto-restore on drop (RAII) so an aborted scenario cannot leave Docker stopped or the
  repo returning 500.
- Live-gate steps that need Docker are `Skipped(reason)` (not Fail) when Docker is intentionally absent,
  keeping CI honest (ties to R15 — no fake pass). Per the freeze-gate evidence rule, `Skipped` never
  counts as `Pass` for a freeze verdict (Skipped ≠ Passed).

## Testing Strategy

- **CI-safe (Layer 0):** runs everywhere, no Docker; validates logic with fakes. Required green before
  any live work per the iteration gate.
- **Rig/Fault/Soak/Scale (Layers 1,3,4,S):** run where Docker is available; gated by a `requires_docker`
  marker; Scale additionally gated by a `scale` marker (large fixtures).
- **A7 engine probe:** logic at Layer 0 (`MockExecutor`), real dispatch at Layer 1/2 (`OpenClawExecutor`).
- **Concurrency + Trust/revocation:** Layer 1/3; **Regression suite:** every iteration + at freeze.
- **Live (Layer 2):** run on a real desktop with OpenClaw enabled from the UI; drives the same backend
  path as the app.
- **Iteration mechanics:** one requirement at a time — CI green → live gate green → 0 leaks → no
  regression (re-run all previously passed requirements) → advance. `Iteration 1 → Fix → Iteration 2 →
  … → Final Acceptance`.
- **Benchmark (Layer 5):** the R20 freeze gate; run from clean state; its scorer emits the go/no-go the
  R10 verdict consumes.

## Correctness Properties

These are invariants the validation must hold true across every layer; each is asserted by the mapped
harness component, not assumed.

### Property 1: Path integrity (R11)
For every skill run, the recorded telemetry path equals the canonical sequence Root Router → openclaw →
SemanticSkillRouter → ExecutionEngine → OpenClawExecutor → RuntimeManager → DockerRuntime → container →
skill → response. No run reaches the runtime without a Root Router record; `OpenClawExecutor` is an
explicit asserted stage; the deprecated `register_skill` path emits nothing.
**Validates: Requirements 11.1, 11.2, 11.3, 11.4, 4.1, 4.2**

### Property 2: Leak-freedom (R2/R7/R18/R20)
After any completed, failed, or cancelled run — and after full suites — OpenClaw container count, lease
count, child processes, and GPU memory return to the pre-run baseline.
**Validates: Requirements 2.3, 2.4, 7.5, 18.2, 18.5, 20.4**

### Property 3: Installer convergence (R12/R13)
For any source (marketplace, GitHub, local `.ocskill`, generated), the produced registry entry,
filesystem layout, and DB row are structurally identical; exactly one verify→materialize→register path
is invoked; no execution branch keys off provenance.
**Validates: Requirements 12.1, 12.2, 12.4, 13.1, 13.2, 13.3**

### Property 4: State consistency (R3.5/R6.5/R18.3/R19)
Registry, filesystem, and skills DB are mutually consistent after every management action, soak, and
upgrade; any divergence (e.g. index-vs-DB drift) is surfaced, never silently reconciled to a single
count.
**Validates: Requirements 3.5, 6.5, 18.3, 19.1, 19.5**

### Property 5: Honesty (R15)
No operation returns success unless it occurred; no UI shows mock/placeholder as live; no stage/safety
check is silently bypassed; incomplete features report `degraded`/`unavailable`/`experimental` or are
Developer-Mode gated.
**Validates: Requirements 15.1, 15.2, 15.3, 15.4, 15.5**

### Property 6: Observability (R9/R17)
Every action in the R17 set produces exactly one correlated evidence/audit record with outcome and
timing; reported health/counts match real Docker state.
**Validates: Requirements 9.1, 9.2, 9.3, 9.4, 17.1, 17.2, 17.3, 17.4**

### Property 7: UI truth (R14/R16)
The UI reflects persisted config and backend state within a bounded time; after synchronization no UI
value contradicts the backend.
**Validates: Requirements 14.2, 14.5, 16.1, 16.2, 16.3, 16.4, 16.5**

### Property 8: Flag-OFF parity (hardening)
With a fix's flag OFF, behavior is byte-for-byte the prior behavior.
**Validates: Requirements 1.5, 15.1**

### Property 9: Reproducibility (R10/R20)
A production-ready verdict is recomputable by re-running the full suite from a clean state and is backed
by evidence records, not opinion.
**Validates: Requirements 10.1, 10.3, 20.7**

### Property 10: A7 engine correctness (R4/R11)
The planner contains zero executor-specific logic; a `Goal` yields a correct `ExecutionGraph`; the
scheduler drives nodes (parallel/conditional/loop/barrier/subgraph/merge) per the graph; retry/recovery,
dependency resolution, cancellation, events, and metrics behave as specified; OpenClaw is one executor
among a pluggable set; the optimizer preserves graph semantics.
**Validates: Requirements 4.1, 4.2, 4.5, 11.1**

### Property 11: Trust & revocation integrity (R3/R6)
Trust-tier admission and approval-bypass follow `TrustConfig`; unsigned/tampered bundles are rejected;
revoking a publisher or skill propagates to registry + marketplace so the artifact is no longer
installable/executable.
**Validates: Requirements 3.2, 3.3, 6.2**

### Property 12: Concurrency safety (R2/R6/R18)
Under concurrent installs/uninstalls/enable/disable/execute/generate — including same-target races —
end state is deterministic and consistent; no deadlock/livelock (bounded-time watchdog); contention on
pool/scheduler/SQLite yields no lost updates; concurrent failure isolates to the affected run.
**Validates: Requirements 2.1, 2.2, 2.3, 2.4, 6.1, 6.2, 6.5, 18.4**

### Property 13: Scale headroom (R3/R4/R9/R18)
With ≥1000 skills across ≥100 publishers, marketplace sync/search/sort, registry lookup, and semantic
routing stay correct within bounded latency; memory, DB growth, startup, and latency stay within stated
budgets.
**Validates: Requirements 3.1, 4.1, 9.1, 18.1**

### Property 14: Regression permanence (iteration gate)
Every hardened bug has a permanent named regression test that fails without the fix and passes with it;
the regression suite is never weakened and runs every iteration and at freeze.
**Validates: Requirements 10.1, 15.5, 20.5**

### Property 15: Real-execution freeze evidence (Skipped ≠ Passed)
A "frozen"/production-ready verdict requires Layer-1/Layer-2 evidence with `Outcome::Pass` for the live
runtime/execution/marketplace/desktop checks and `real`-LLM evidence for A9; any `Skipped` or `fixture`
evidence yields No-Go.
**Validates: Requirements 10.1, 10.3, 20.5, 20.7**

## Open questions to resolve in tasks

1. Confirm the exact Settings command/event names OpenClaw already exposes (structure.md forbids
   renaming them) so R14/R16 probes bind to the real contract.
2. Confirm whether hot reload (R6.4) is supported or a restart is required — the answer sets the
   acceptance branch, not a redesign.
3. Confirm the intended production `registry.index_url` (kria-ai default vs a user repo) so R3 validates
   against the correct configured source via the rig.
