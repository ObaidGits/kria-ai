# Implementation Plan: OpenClaw Production Validation & Hardening

## Overview

This is the task checklist for validating and hardening the OpenClaw subsystem to
production readiness (architecture A0–A9 frozen — repair/integrate/harden/prove only).

**Live progress, findings, fixes, and per-task evidence live in [PROGRESS.md](./PROGRESS.md)**
(moved out of this file so the task list stays a clean checkbox format). Cross-session
handoff notes are in [SESSION_HANDOFF.md](./SESSION_HANDOFF.md).

## Shared conventions (apply to EVERY task)

1. **Harness location.** All validation code lives under `crates/kria-eval/src/openclaw_eval/`, reusing
   `runner.rs`/`suite.rs`/`judge.rs`/`report.rs`. No new crate. No duplicate installer/registry/router/
   runtime/marketplace — bind to real symbols (`ExecutionEngine`, `OpenClawExecutor`,
   `ProductionSkillRegistry`, `SemanticSkillRouter`, `RuntimeManager`, `ContainerPool`, `clawhub`,
   `admission`/`revocation`).
2. **Evidence.** Every check emits an `EvidenceRecord { requirement, layer, name, outcome, metrics,
   correlation_id, evidence, timestamp }`. `Layer ∈ {Ci, Rig, Live, Fault, Soak, Scale, Benchmark}`,
   `Outcome ∈ {Pass, Fail, Skipped(reason)}`.
3. **Isolation.** Rig uses a dedicated container-name prefix + temp `~/.kria` root + a local fixture repo
   server. Validation NEVER touches the user's real skills DB or the live public repo.
4. **Flag + parity for hardening.** Any behavior change lands behind a `KRIA_OPENCLAW_*` flag, default
   decided per task; add a parity test asserting flag-OFF = prior behavior; new serialized fields
   `#[serde(default)]`.
5. **Honesty.** No fake `Pass`, no mock-as-live, no silent bypass. Docker/desktop-absent steps are
   `Skipped(reason)`, never `Pass`.
6. **Regression rule.** No bug fixed without a permanent test in `openclaw_eval/regression/` named
   `regr_<Rxx>_<slug>` that fails with the fix reverted and passes with it; the suite runs every iteration.
7. **Contracts.** Never rename existing Tauri command/event names or config keys; telemetry is additive.
8. **Secrets.** Never log tokens/keys; reference by name (config `.env`).

---

## Tasks

- [x] 1. Harness foundation (`openclaw_eval` scaffolding)
  - [x] 1.1 Create `openclaw_eval/mod.rs` with `EvidenceRecord`, `Layer`, `Outcome`, suite registration, and a results store; wire into `kria-eval` `runner.rs`/`suite.rs`.
    - _Requirements: 10.1, 10.3_
  - [x] 1.2 Implement `rig.rs`: `TestRig::up()/down()` — verify Docker, build/verify pinned `kria/openclaw-substrate:test` image, start local fixture repo server, point a scoped `OpenClawConfig` at it, temp `~/.kria` root, dedicated container prefix; `down()` reaps containers + asserts baseline.
    - _Requirements: 1.1, 2.5, 3.1_
  - [x] 1.3 Implement `leak_detector.rs`: `baseline()` (container count via `docker ps` prefix filter, pool leases, child procs, GPU nvml when present) + `assert_returned_to(baseline)`.
    - _Requirements: 2.4, 7.5, 18.2_
  - [x] 1.4 Implement `fault_injector.rs`: `stop_docker/start_docker`, `kill_container`, `stall_bridge`, `repo_status(500)`, `repo_malformed`; RAII auto-restore on drop.
    - _Requirements: 7.1, 7.2, 7.3, 7.4_
  - [x] 1.5 Implement `fixtures/`: valid signed skill, bad-hash, invalid-manifest, malformed `index.json`, and the drift fixture (index=1 / seeded DB=3) mirroring `clawhub.rs` schema + real `.ocskill` bundle format.
    - _Requirements: 3.2, 3.3, 3.5, 7.4_
  - [x] 1.6 Establish `regression/` suite skeleton + naming convention + per-iteration runner hook.
    - _Requirements: 10.1, 15.5, 20.5_

- [x] 2. R1 — Enable/disable lifecycle validation
  - Drive the real Settings enable/disable path; assert runtime start reports `ready`/`degraded`/`unavailable` honestly; disable reaps pool (leak baseline); Docker-absent = `unavailable` no crash; shutdown tears down containers; flag-OFF = no runtime created.
  - Harden only if a gap is found (behind `KRIA_OPENCLAW_*` flag + parity test + regression test).
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5_

- [x] 3. R2 — Container lifecycle & warm-pool integrity
  - `rig` scenarios: acquire/reuse/release, unhealthy eviction, timeout + cancel termination, image-present verification, JSON-RPC malformed/oversized rejection; N-run leak assertion via `leak_detector`.
  - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5, 2.6_

- [x] 4. A7 Execution Engine probe (`engine_probe.rs`)
  - [x] 4.1 Layer 0 with `MockExecutor` (mirror `execution/tests.rs`): planner→graph correctness, scheduler (parallel/conditional/loop/barrier/subgraph/merge), `ExecutorRegistry` register+lookup+**replace**, `DependencyResolver` (cycle/missing), `RecoveryManager`/`RecoveryPolicy` retry-then-succeed + exhausted, cancellation, `ExecutionEventStream` ordering, `ExecutionMetrics` accuracy, `GraphOptimizer` semantics-preserving.
    - _Requirements: 4.1, 4.2, 11.1_
  - [x] 4.2 Layer 1/2 via `OpenClawExecutor`: OpenClaw node dispatches into `RuntimeManager`/`DockerRuntime`; assert boot wiring lives only in executors boundary.
    - _Requirements: 4.1, 4.5, 11.1_

- [x] 5. R11 — Root Router path integrity (`pipeline_trace.rs`)
  - Subscribe to telemetry along the canonical path `Root Router → openclaw → SemanticSkillRouter::route → ExecutionEngine → OpenClawExecutor → RuntimeManager → DockerRuntime → container → skill → response`; `assert_canonical_path(run_id)` for 100% of runs; assert no runtime entry without a Root Router record; assert deprecated `register_skill` path emits nothing; recorded short-circuits, no silent bypass.
  - _Requirements: 11.1, 11.2, 11.3, 11.4_

- [x] 6. R3 — Marketplace install + drift surfacing
  - `rig` against fixture `index.json`: list == declared + report source URL; download→verify(hash/sig/manifest)→materialize→register; bad-hash/bad-manifest abort registers nothing; installed-view shows version+capabilities; **drift (1-vs-3) surfaced** (DB-only vs index-only); unreachable repo → graceful offline.
  - Harden drift surfacing behind `KRIA_OPENCLAW_DRIFT_SURFACE` if currently silent (+ parity + regression test).
  - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 3.6_

- [x] 7. Trust & revocation validation (installer-matrix trust extension)
  - Over `admission.rs`/`approval.rs`/`revocation.rs`/`TrustConfig`: tier admission (community/verified/local), `verified_skips_hitl`, `community_allows_network`, unknown-default tier; publisher signature verify; unsigned/tampered reject; revoke publisher/skill → propagate to registry + marketplace, artifact non-installable and non-executable on next route; approval-bypass only for configured tiers.
  - _Requirements: 3.2, 3.3, 6.2_

- [x] 8. R12 — Unified installer convergence (`installer_matrix.rs`)
  - Feed one skill through: fixture marketplace, local git-style repo dir, local `.ocskill`, A9-generated bundle; assert identical registry entry + fs layout + DB row (provenance = metadata only); instrumented counter proves one verify→materialize→register path; malformed local bundle aborts.
  - _Requirements: 12.1, 12.2, 12.3, 12.4, 12.5_

- [x] 9. R4 — Execute installed skill end-to-end
  - Live + rig + trace: matched prompt selects OpenClaw → routes to correct skill → runs in container → returns real output; telemetry records router score/registry hit/executor; below-threshold declines cleanly; capability enforcement; container released (leak baseline).
  - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5_

- [x] 10. R13 — Generated ≡ authored skills
  - Assert generated bundle format/manifest structurally identical (provenance only); install+execute via R12 installer + R11 path; assert NO code path branches execution/verification/telemetry on `is_generated`; management (R6) + telemetry event-set identical.
  - _Requirements: 13.1, 13.2, 13.3, 13.4, 13.5_

- [ ] 11. R5 — Autonomous skill generation (A9) end-to-end + real-LLM policy
  - [x] 11.1 Layer 0: pipeline design→codegen→validate→test→package with fixture LLM (`llm_fixture.rs`); repair-or-abort; budget/approval boundaries; Dev-Mode gating for non-ready stages. Fixture evidence tagged `fixture` (never counts for freeze).
    - _Requirements: 5.1, 5.2, 5.4, 5.5_
  - [ ] 11.2 Layer 2: real configured LLM backend generates → installs (R12) → executes (R4); evidence tagged `real`.
    - _Requirements: 5.1, 5.3, 13.1_

- [x] 12. R6 — Skill management (update/enable/disable/uninstall/hot-reload)
  - disable stops routing / enable resumes; uninstall removes bundle+registration+DB row, no orphans; update supersedes + cleans old; hot-reload without restart OR clearly state restart required (resolve open question); post-action registry/fs/DB consistency.
  - _Requirements: 6.1, 6.2, 6.3, 6.4, 6.5_

- [x] 13. R7 — Failure injection & recovery
  - Via `fault_injector`: Docker stopped mid-session, container crash mid-run, bridge stall/timeout, repo unreachable/malformed; each → clear reason, no hang, cleanup, app stays usable; post-fault leak baseline.
  - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5_

- [x] 14. Concurrency probe (`concurrency_probe.rs`)
  - Parallel install/uninstall/enable/disable/execute/generate; same-target races (install+uninstall same skill, enable+disable, execute-while-uninstalling) → deterministic consistent state; pool/scheduler/SQLite contention (no lost updates); deadlock/livelock watchdog (bounded time, timeout = failure); recovery under concurrent failure isolates to affected run.
  - _Requirements: 2.1, 2.2, 2.3, 2.4, 6.1, 6.2, 6.5, 18.4_

- [x] 15. R8 + R14 — Settings surface & authority
  - [x] 15.1 R8: assert Settings exposes enable/disable, marketplace source, installed skills (enable/disable/uninstall), generated skills, Developer Mode, health/status + logs; actions reflect real outcome; non-ready features hidden/marked. Confirm real command/event names (open question) before binding.
    - _Requirements: 8.1, 8.2, 8.3_
  - [x] 15.2 R14: all normal settings set from UI, persisted, survive restart, adopted by runtime (or explicit restart notice); no TOML/JSON/env editing required; UI shows persisted values.
    - _Requirements: 14.1, 14.2, 14.3, 14.4, 14.5_

- [x] 16. R16 — UI/backend synchronization (`ui_sync_probe.rs`)
  - Drive real desktop command/event surface; assert install/remove/disable/enable/update, container create/destroy, marketplace sync, health, generation progress reflect within bounded time; dropped-event reconciles on next poll; no contradiction after sync.
  - _Requirements: 16.1, 16.2, 16.3, 16.4, 16.5_

- [x] 17. R9 + R17 — Telemetry, metrics, honest health & completeness
  - [x] 17.1 R9: run counts/outcomes/durations accurate; container/lease counts match `docker ps`; health reflects real state; install/generation written to audit ledger.
    - _Requirements: 9.1, 9.2, 9.3, 9.4_
  - [x] 17.2 R17 (`telemetry_assert.rs`): each action in {install, update, remove, execute, generate, repair, container_create, container_destroy, marketplace_sync, router_select, path_traverse, failure, cancel} → exactly one correlated record with outcome+timing+correlation id; failure/cancel records reason.
    - _Requirements: 17.1, 17.2, 17.3, 17.4_

- [x] 18. R18 — Long-running / soak stability (`soak.rs`)
  - Sustained mixed workload; sample memory (bounded, no monotonic leak), container/lease/GPU return to baseline periodically, DB/registry/fs consistency, warm-pool health, desktop responsiveness.
  - _Requirements: 18.1, 18.2, 18.3, 18.4, 18.5_

- [x] 19. R19 — Upgrade / migration compatibility (`upgrade.rs`)
  - Materialize prior-version state fixture; run migration; assert skills/generated/registry/marketplace/settings/DB preserved; schema/format migrated forward; unmigratable = fail-safe; post-upgrade skills discoverable+executable via R11; idempotent re-run.
  - _Requirements: 19.1, 19.2, 19.3, 19.4, 19.5_

- [x] 20. Scale validation (`scale.rs`, Layer `Scale`)
  - Generate large fixture repo (≥1000 skills / ≥100 publishers); marketplace sync + delta + search + sort correct & bounded latency; `ProductionSkillRegistry` lookup bounded; `SemanticSkillRouter::route` correct with 1000+ candidates within latency budget; parallel install/update/search; record memory/DB growth/startup/search/install latency vs budgets.
  - _Requirements: 3.1, 4.1, 9.1, 18.1_

- [x] 21. R15 — Honesty sweep (cross-cutting)
  - Assert no operation returns success without occurring; no mock-as-live UI; no silent stage/safety bypass; incomplete features report `degraded`/`unavailable`/`experimental` or Dev-Mode gated; enumerate reachable TODOs/placeholders into Technical Debt.
  - _Requirements: 15.1, 15.2, 15.3, 15.4, 15.5_

- [x] 22. Freeze report bundle + freeze-gate evidence rule (`report.rs` ext)
  - [x] 22.1 Auto-generate report bundle from evidence store: Architecture / Coverage / Execution / Marketplace / ASGS / Performance / Stress / Regression / Risk / Known Issues / Technical Debt / Readiness Score / Go-No-Go / Freeze Verdict.
    - _Requirements: 10.1, 10.2, 20.5, 20.7_
  - [x] 22.2 Freeze-gate scorer: require Layer-1/Layer-2 `Pass` for live runtime/execution/marketplace/desktop checks and `real`-LLM A9 evidence; treat `Skipped` and `fixture` as not-satisfied → No-Go; classify remaining work Critical/Important/Optional/Nice-to-have.
    - _Requirements: 10.1, 10.2, 10.3, 20.5, 20.6, 20.7_

- [x] 23. R20 — Production benchmark & final verdict (`benchmark.rs`)
  - From clean state: ≥100 skill-invocation prompts, 50 installs, 20 updates, 20 removals, 20 generated; interleave desktop restart, Docker restart, induced crashes, cancellation, timeouts + parallel + memory/GPU pressure; assert recovery + 0 leaks after each; post-run baseline + DB/registry/fs consistency + telemetry present; run freeze gate (task 22) and emit reproducible evidence-backed verdict.
  - _Requirements: 20.1, 20.2, 20.3, 20.4, 20.5, 20.6, 20.7, 10.1, 10.2, 10.3_

---

## Real-usage production validation (tasks 24–35)

> Tasks 1–23 prove the system with harness/rig/fixtures. Tasks 24–35 prove it survives **real human
> usage** on the real desktop, real Docker daemon, real `RuntimeManager`/`ExecutionEngine`/
> `SemanticSkillRouter`/`ProductionSkillRegistry`, real marketplace, real containers, real Settings, real
> telemetry. No mocks, no fixtures where a real artifact is required. Evidence is `real`-tagged; `fixture`/
> `Skipped`/simulation never counts toward the freeze verdict.

- [ ] 24. Manual production validation wave (100+ real prompts)
  - [ ] 24.1 Author a real prompt suite (≥100) into `openclaw_eval/manual/prompt_suite.md` spanning: math, files, PDF, CSV, JSON, images, vision, reasoning, coding, browser, web search, multi-skill, generated-skill, marketplace-skill, long-context, filesystem, network, database, GPU, heavy, pipeline, planner, and execution-graph skills.
    - _Requirements: 4.1, 4.2, 11.1_
  - [ ] 24.2 Run each prompt through the real desktop chat with OpenClaw enabled; for every prompt record real evidence of: correct routing (R11 path), correct execution, correct telemetry (R17), correct container cleanup (leak baseline), correct response.
    - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5, 9.1, 11.1, 17.1_
  - [ ] 24.3 Adversarial + control subset: wrong/malformed/ambiguous prompts, prompt injection, repeated prompts, concurrent chats, cancellation/interruption mid-run, container restart mid-run → assert honest decline/recovery, no leak, no wrong-skill force.
    - _Requirements: 4.3, 7.2, 7.3, 15.1, 15.3_
  - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5, 9.1, 11.1, 17.1, 15.1_

- [x] 25. Real marketplace validation (live repository, not fixtures)
  - Against a real configured GitHub `index.json` repo: real sync, install, uninstall, update, downgrade, rollback, reinstall, dependency resolution, version upgrade, registry update, router discovery, execution. Confirm the intended production `registry.index_url` (open question) and reconcile the audit's index-vs-DB drift on the real source.
  - _Requirements: 3.1, 3.2, 3.4, 3.5, 3.6, 6.3, 12.1, 19.1, 19.2_

- [x] 26. Generated-skill real validation + persistence
  - Generate multiple real skills via A9 with the real LLM (e.g. EXIF photo rename, PNG→WEBP, download webpage, extract PDF text, CSV→JSON, image resize, weather API, git clone, ZIP extract, markdown convert). Install each, execute each, **restart KRIA**, verify persistence + registry + marketplace representation + re-execution.
  - _Requirements: 5.1, 5.3, 13.1, 13.2, 13.3, 13.4, 6.3, 19.1, 19.4_

- [ ] 27. Long-session stability (4–8h continuous, real)
  - Run OpenClaw continuously 4–8h with mixed + heavy workload: generation, marketplace, execution, install, uninstall, concurrent prompts. Assert no leaks, no deadlocks, no orphan containers, no registry/DB corruption, no memory explosion, no GPU instability. Extends R18 soak to a real desktop session.
  - _Requirements: 18.1, 18.2, 18.3, 18.4, 18.5_

- [ ] 28. UX truthfulness validation
  - Validate every user-visible surface is present and truthful: loading indicators, progress bars (install/generation/execution), notifications, errors, retry, cancel, offline mode, marketplace empty/loading states, container startup/shutdown, health badges. No surface shows success/progress that does not match backend (ties R16).
  - _Requirements: 8.2, 8.3, 15.2, 15.3, 16.1, 16.2, 16.3_

- [x] 29. Performance budgets (measured, not subjective)
  - Replace subjective wording with measured budgets and assert them: semantic routing < 20ms, registry lookup < 5ms, container reuse < 500ms, marketplace search < 100ms, container cold start < 5s, generation < 30s, KRIA restart < 10s. Record measured values as metrics; a budget miss is a Fail (honest), not a reword.
  - _Requirements: 9.1, 9.2, 18.1_

- [ ] 30. Regression capture during real validation (continuous)
  - Every production bug found in tasks 24–29/33/34 immediately produces a permanent `regr_<Rxx>_<slug>` test in `openclaw_eval/regression/` (fails reverted, passes fixed) BEFORE continuing. Suite runs every iteration and at freeze. No discovered bug may ever silently return.
  - _Requirements: 10.1, 15.5, 20.5_

- [ ] 31. Release-candidate checklist generator (`OPENCLAW_RELEASE_CHECKLIST.md`)
  - Auto-generate from the evidence store: architecture status, frozen contracts, container/marketplace/generation/execution/registry/execution-engine/runtime/semantic-router/desktop/settings/telemetry status, known bugs, known limitations, technical debt, performance, memory, GPU, database, stress, and Go/No-Go.
  - _Requirements: 10.1, 10.2, 20.5, 20.7_

- [ ] 32. Feature-completeness matrix generator (`OPENCLAW_FEATURE_MATRIX.md`)
  - Auto-generate a matrix of every feature classified Implemented / Partially Implemented / Experimental / Missing / Blocked / Future, each referencing actual implementation, actual tests, and actual evidence records.
  - _Requirements: 10.1, 10.2, 15.5_

- [x] 33. Capability-class validation (every class individually)
  - For each capability class — filesystem, network, environment, GPU, CPU, memory, secrets, browser, database, subprocess, parallel, execution-graph — plus capability escalation, revocation, approval, and rollback: prove grant → execution → revocation → cleanup, with undeclared access denied per safety policy.
  - _Requirements: 4.4, 3.2, 6.2, 7.5_

- [x] 34. Real failure campaign (intentional breakage)
  - Intentionally break the real system: Docker crash, container crash, bridge timeout, OOM, GPU unavailable, disk full, permission denied, marketplace offline, registry corruption, DB corruption, invalid bundles, broken skills, missing dependencies, power-interruption simulation, and restart during execution / generation / install. Verify recovery, rollback, cleanup (leak baseline), and truthful user feedback for each.
  - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5, 19.3, 18.2_

- [x] 35. Final freeze validation
  - Freeze is allowed ONLY when EVERY task (1–34), EVERY regression, EVERY benchmark, EVERY manual + live validation, the release checklist (31), and every evidence record is `Pass` with `real` evidence. Enforce `Skipped ≠ Passed`, `fixture ≠ real`, `simulation ≠ production`. Emit the final freeze verdict + reproducibility statement.
  - _Requirements: 10.1, 10.2, 10.3, 20.5, 20.6, 20.7_

## Pipeline root-cause fixes (tasks 36–37)

> Root causes PROVEN in PROGRESS.md ("ROOT-CAUSE ANALYSIS"). General/architectural — no
> prompt matching, no hardcoded keywords, no special cases. Must work for arbitrary future skills.

- [x] 36. RC1 — schema-driven argument generation (fixes "missing required parameter" for every skill)
  - The A6 `openclaw` tool passes the raw freeform `{query}` straight to the selected skill; skills expect their own `inputSchema` (`expression`, `text`, …) → every routed skill fails "missing required parameter".
  - Fix (general): after routing, obtain the selected skill's `inputSchema` from the container's `bridge.list_tools()` (single source of truth), then use the injected `ModelRouter`/`LlmBackend` to generate arguments conforming to that schema from the `query`; pass generated args to `LaunchSpec.params`. Add `#[serde(alias = "inputSchema")]` to `McpToolDef.input_schema`. Thread `model_router` → `register_into_tool_registry` → `register_semantic_openclaw` → `SemanticOpenClawHandler`.
  - Regression + real-Docker: `calculate 3+3` → `{expression:"3+3"}` → 6; `sha256 hash "kria"` → `{text,algorithm}`; no hardcoded per-skill mapping (assert via a novel skill/schema).
  - _Requirements: 4.1, 4.4, 15.1, 15.4_

- [x] 37. RC2 — registry coverage from container `tools/list` (fixes mis-routing, e.g. word-count → oc_web_search)
  - Only 3 skills are seeded into the registry; the substrate image bakes 8, so the router's candidate set is incomplete and non-matching requests mis-route to the nearest enabled skill.
  - Fix (general): sync the registry from `bridge.list_tools()` (name + description + capabilities + `inputSchema`) so EVERY baked/installed skill auto-registers enabled — future skills included, zero per-skill Rust. Registry ⇄ container agreement asserted.
  - Regression + real-Docker: all 8 baked skills discoverable + routable; word-count routes to a text skill, not `oc_web_search`.
  - _Requirements: 3.1, 3.5, 4.1, 4.2_

## Task Dependency Graph

```
1 (foundation)
├── 2  R1 lifecycle
├── 3  R2 container/pool ──────┐
├── 4  A7 engine probe ────────┤
│     └── 5  R11 path integrity │ (needs 3 + 4)
├── 6  R3 marketplace install ──┐
│     ├── 7  Trust & revocation │ (needs 6)
│     └── 8  R12 unified installer (needs 6)
│           └── 9  R4 execute e2e (needs 5 + 8)
│                 ├── 10 R13 generated≡authored (needs 8 + 9)
│                 └── 11 R5 A9 generation (needs 8 + 9)
├── 12 R6 skill management (needs 6)
├── 13 R7 failure injection (needs 3)
├── 14 Concurrency probe (needs 3 + 12)
├── 15 R8 + R14 Settings (needs 2)
├── 16 R16 UI sync (needs 15)
├── 17 R9 + R17 Telemetry (needs 9)
├── 18 R18 Soak (needs 3 + 9 + 12)
├── 19 R19 Upgrade (needs 8)
├── 20 Scale (needs 6 + 9)
├── 21 R15 Honesty sweep (needs all functional tasks 2–20)
├── 22 Freeze report + gate (needs 17 + 21)
│     └── 23 R20 benchmark (needs 22 + everything above)
│
└── REAL-USAGE WAVE (needs 23 green — real desktop/Docker/LLM, no fixtures)
    ├── 24 Manual 100+ prompt wave
    ├── 25 Real marketplace (live GitHub repo)
    ├── 26 Generated-skill real + persistence
    ├── 33 Capability-class validation
    ├── 27 Long-session stability 4–8h        (needs 24–26)
    ├── 28 UX truthfulness                     (needs 24)
    ├── 29 Performance budgets                 (needs 24)
    ├── 34 Real failure campaign               (needs 24–26)
    ├── 30 Regression capture (continuous across 24–29/33/34)
    ├── 31 Release checklist                   (needs 24–34)
    ├── 32 Feature matrix                      (needs 24–34)
    └── 35 Final freeze validation             (needs ALL 1–34)
```

Critical path: `1 → 3/4 → 5 → 6 → 8 → 9 → 17 → 22 → 23 → 24 → 34 → 31 → 35`. Tasks 2, 12, 13, 15, 19, 20
can run in parallel once their prerequisites are green; the real-usage wave (24–34) runs on the real
desktop after 23. The **iteration gate** still serializes acceptance: one requirement done at a time, its
live gate green before the next is accepted.

```json
{
  "waves": [
    { "wave": 1, "tasks": ["1"], "parallel": false },
    { "wave": 2, "tasks": ["2", "3", "4", "6"], "parallel": true },
    { "wave": 3, "tasks": ["5", "7", "8", "12", "13", "15"], "parallel": true },
    { "wave": 4, "tasks": ["9", "16", "19", "20"], "parallel": true },
    { "wave": 5, "tasks": ["10", "11", "14", "17", "18"], "parallel": true },
    { "wave": 6, "tasks": ["21"], "parallel": false },
    { "wave": 7, "tasks": ["22"], "parallel": false },
    { "wave": 8, "tasks": ["23"], "parallel": false },
    { "wave": 9, "tasks": ["24", "25", "26", "33"], "parallel": true },
    { "wave": 10, "tasks": ["27", "28", "29", "34"], "parallel": true },
    { "wave": 11, "tasks": ["30"], "parallel": false },
    { "wave": 12, "tasks": ["31", "32"], "parallel": true },
    { "wave": 13, "tasks": ["35"], "parallel": false }
  ]
}
```

## Notes

- **No A0–A9 redesign.** Every task validates or adds harness/regression code, or lands a surgical
  hardening fix behind a `KRIA_OPENCLAW_*` flag with flag-OFF parity. Bind to real symbols only; introduce
  no duplicate installer/registry/router/runtime/marketplace/generation/execution systems.
- **Iteration gate is mandatory.** Per task: flag-OFF parity (if changed) → CI green → Layer-1/2 gate green
  → 0 leaks (leak detector at baseline) → no regression (prior-passed set + `regression/` suite) → bug
  fixes carry a permanent `regr_<Rxx>_<slug>` test. Do not advance until green.
- **Skipped ≠ Passed, fixture ≠ real, simulation ≠ production.** Docker/desktop-absent steps record
  `Skipped(reason)`; the freeze verdict (22/23/35) rejects Skipped, `fixture`-LLM, and simulated evidence.
- **Real-usage wave (24–35) is mandatory for freeze.** Tasks 1–23 prove the system with harness/fixtures;
  tasks 24–35 prove it under real human usage (real desktop/UI/Docker/RuntimeManager/ExecutionEngine/
  SemanticSkillRouter/marketplace/registry/containers/settings/telemetry). Task 35 freezes ONLY when every
  task 1–34, every regression, benchmark, manual + live validation, and the release checklist (31) is
  `Pass` with `real` evidence. Generated artifacts: `OPENCLAW_RELEASE_CHECKLIST.md` (31),
  `OPENCLAW_FEATURE_MATRIX.md` (32) — both auto-generated from the evidence store, not hand-written.
- **Open questions to resolve during tasks (branch, not redesign):** exact Settings command/event names
  (task 15), hot-reload vs restart (task 12, R6.4), intended production `registry.index_url` — kria-ai
  default vs a user repo (task 6).
- **Build/verify:** CI-safe tests `cargo test -p kria-eval`; Docker-gated scenarios behind a
  `requires_docker` marker; scale behind a `scale` marker; live gate on a real desktop with OpenClaw
  enabled from the UI.
- **Honesty over green.** Never weaken a check to pass; prefer an honest `degraded`/`inconclusive` verdict
  and record it as evidence.
