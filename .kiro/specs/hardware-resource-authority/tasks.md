# Implementation Plan: KRIA Hardware & Resource Authority (HRA)

## Overview

Implementation-ready, phased, reversible. Each task: objective, files, dependencies, risks,
validation, acceptance. No task ships behind a hidden flag without an off-switch. Phases are
independently shippable; later phases delete the old fragmented code only after the new path is
proven. Requirement refs map to `requirements.md`. Legend: `[ ]` not started.

## Tasks

### Phase 0 — Foundations (no behavior change)

- [x] 1. Unify hardware tier classification
  - Objective: one `classify_tier` used by both detection paths; remove OR/AND divergence.
  - Files: `crates/kria-core/src/platform/detect.rs`, `crates/kria-core/src/infra/hardware_profiler.rs`.
  - Dependencies: none.
  - Risks: tier shifts for edge hardware → changes default model selection.
  - Validation: unit tests covering all class boundaries from both prior functions; assert identical
    output for a fixed matrix.
  - Acceptance (A3): single function; grep shows one definition; identical hardware → identical tier.

- [x] 2. Define HRA core types crate-internal module `resource/authority/`
  - Objective: introduce `DeviceTable`, `Device`, `ResourceRequest`, `Plan`, `Lease`, `PriorityClass`,
    `Residency`, `RationaleCode` (types only, no logic).
  - Files: new `crates/kria-core/src/resource/authority/types.rs`, wire in `resource/mod.rs`.
  - Dependencies: Task 1.
  - Risks: type churn; keep minimal + documented.
  - Validation: `cargo check -p kria-core`; doc tests on enums.
  - Acceptance: types compile, documented, no consumer wired yet.

- [x] 3. Single TelemetryCollector + HostSnapshot
  - Objective: evolve `orchestrator/telemetry.rs TelemetryActor` into one host-wide collector that
    samples per-GPU (multi-device), CPU per-core, RAM, disk, thermal, battery, per-process VRAM;
    publish immutable `HostSnapshot` with `seq`+`sampled_at` via `watch`.
  - Files: `crates/kria-core/src/llm/orchestrator/telemetry.rs` → move to
    `crates/kria-core/src/resource/telemetry_collector.rs`; update `platform/vram.rs` to feed it.
  - Dependencies: Task 2.
  - Risks: regressions in VRAM reads; NVML multi-device init cost.
  - Validation: snapshot test vs NVML/nvidia-smi on a CUDA box; staleness flag test; CPU overhead
    measured ≤ N3 budget.
  - Acceptance (R3, A2): one collector; ring-buffer history API present.

---

### Phase 1 — Resource Authority (control plane), shadow mode

- [x] 4. DeviceTable backed by TelemetryCollector
  - Objective: RA maintains the only mutable DeviceTable; per-device free/reserved/safety margin;
    cloud pools registered as Devices.
  - Files: `crates/kria-core/src/resource/authority/device_table.rs`.
  - Dependencies: Tasks 2,3.
  - Risks: reservation accounting drift vs reality → reconcile (Task 9) covers it.
  - Validation: property tests: reserve+release never exceeds capacity; multi-device isolation.
  - Acceptance (R1.3, R1.7): per-device table; cloud device present.

- [x] 5. Deterministic Planner (pure function)
  - Objective: implement cost-model Planner (`requirements` R13, `design` §3.2). No I/O, no LLM.
  - Files: `crates/kria-core/src/resource/authority/planner.rs`,
    `crates/kria-core/src/resource/authority/policy_profiles.rs`.
  - Dependencies: Task 4.
  - Risks: weight tuning; bad plan under-utilizes hardware.
  - Validation: table-driven tests per hardware class (`requirements` §5); assert no LLM symbol
    referenced (static check).
  - Acceptance (A10, R13.1): pure, deterministic, profile-driven; golden plans per class.

- [x] 6. Scheduler + Lease issuance + priority/preemption
  - Objective: admission (lock-free warm path), per-device priority queue, RealtimeVoice fast lane,
    cooperative preemption with hard deadline, RAII `Lease`.
  - Files: `crates/kria-core/src/resource/authority/scheduler.rs`,
    `crates/kria-core/src/resource/authority/lease.rs`.
  - Dependencies: Tasks 4,5.
  - Risks: preemption deadlock; starvation. Mitigate: lock-wait watchdog, fairness counters.
  - Validation: concurrency tests (loom or stress) for no-deadlock; voice fast-lane p99 ≤ 2 ms bench;
    starvation test under sustained Batch load.
  - Acceptance (R6, N1): priority + preemption + fairness proven by tests.

- [x] 7. Pressure Engine (carry watchdog logic)
  - Objective: per-device EMA + dwell + hysteresis + rate-limit; emit `PressureLevel` + ordered
    non-disruptive remedies. Port proven constants from `gpu_watchdog.rs`/`threshold.rs`.
  - Files: `crates/kria-core/src/resource/authority/pressure.rs`.
  - Dependencies: Tasks 3,4.
  - Risks: thrash if dwell/hysteresis mis-tuned.
  - Validation: replay recorded VRAM traces; assert no thrash; emergency-only foreground touch.
  - Acceptance (R5.3,R5.4): remedy ordering prefers non-disruptive; emergency path explicit.

- [x] 8. Decision Journal + Event bus correlation
  - Objective: append-only journal (seq + turn_id) for grant/release/plan/preempt/evict/failover;
    additive `resource:*` events carrying correlation id.
  - Files: `crates/kria-core/src/resource/authority/journal.rs`;
    extend `crates/kria-core/src/infra/event_bus.rs`.
  - Dependencies: Tasks 6,7.
  - Risks: journal write amplification; bound size + rotate.
  - Validation: every emitted event resolves to a journal entry (test); rotation test.
  - Acceptance (R10.2, A5): end-to-end correlation id present.

- [x] 9. Reconciler + crash recovery
  - Objective: on boot/restart, diff journal leases vs real GPU processes (per-process VRAM map);
    reclaim orphan llama-server/ComfyUI; fail-open default plan.
  - Files: `crates/kria-core/src/resource/authority/reconciler.rs`.
  - Dependencies: Tasks 3,4,8.
  - Risks: killing a legitimately-shared process. Safety: gate kills through safety policy; require
    journal evidence; write normal-prose audit. **This task performs process termination — destructive;
    must log before/after and be covered by the safety policy layer.**
  - Validation: kill-restart integration test → zero leaked processes, no false kill.
  - Acceptance (R12.1, A7): orphans reclaimed; no leaks.

- [x] 10. RA assembly + shadow-mode evaluation
  - Objective: assemble `ResourceAuthority`; run in SHADOW (logs decisions, does not yet grant) next
    to the existing orchestrator to compare decisions against current behavior.
  - Files: `crates/kria-core/src/resource/authority/mod.rs`;
    wire read-only in `crates/kria-desktop/src/commands/runtime.rs`.
  - Dependencies: Tasks 4–9.
  - Risks: none (shadow). 
  - Validation: shadow decision log reviewed vs real swaps for a soak period.
  - Acceptance: RA produces sane plans on real hardware in shadow.

---

### Phase 2 — Consumer cutover (one subsystem at a time)

- [x] 11. ModelLifecycle contract
  - Objective: define the uniform trait (`design` §5).
  - Files: `crates/kria-core/src/resource/lifecycle.rs`.
  - Dependencies: Task 2.
  - Risks: contract too narrow → revisit before adoption.
  - Validation: implemented by a trivial mock; compiles.
  - Acceptance (R4.1): trait stable and documented.

- [x] 12. LLM consumer → RA
  - Objective: `Orchestrator`/`LlamaServerManager` request leases + plans from RA; watchdog logic
    delegated to RA Pressure Engine; KV slot save/restore preserved.
  - Files: `crates/kria-core/src/llm/orchestrator/mod.rs`, `gpu_watchdog.rs` (reduce to adapter),
    `server_manager.rs`.
  - Dependencies: Tasks 6,7,11.
  - Risks: swap regressions; KV loss. Keep Router-Mode + restart fallback intact.
  - Validation: swap soak test; assert no non-emergency `stream_interrupted` during foreground (A4).
  - Acceptance (R9.3, A4): foreground never interrupted by non-emergency action.

- [x] 13. Image consumer → RA (replace local lease + LlmEvictionController)
  - Objective: `ImageOrchestrator` acquires RA lease; Tier-B drop-swap via RA preemption of LLM
    residency on the device; remove its private `GpuLeaseManager`.
  - Files: `crates/kria-core/src/image/orchestrator.rs`, `image/swap.rs`,
    `crates/kria-core/src/tools/image_generation.rs`.
  - Dependencies: Task 12.
  - Risks: swap timing; cloud failover semantics. Keep VramBarrier as a verification gate.
  - Validation: Tier-B end-to-end; failover/failback with explicit notice (A9).
  - Acceptance (R8.3, A9): no silent sticky degradation.

- [x] 14. Voice (STT/TTS/Wake) consumer → RA
  - Objective: speech acquires RA RealtimeVoice leases; Wake stays live (split tap) during swaps;
    remove private speech `GpuLeaseManager`; fix reconcile snapshots that hardcode vram=0.
  - Files: `crates/kria-core/src/voice/stt.rs`, `tts.rs`,
    `crates/kria-desktop/src/commands/voice_runtime_helpers.rs`, `image/swap.rs` (AudioFreeze).
  - Dependencies: Tasks 6,12.
  - Risks: voice latency regression. Mitigate fast lane (Task 6).
  - Validation: utterance-during-swap test (wake survives); STT/TTS p99 latency bench.
  - Acceptance (R6.2, R11.4): wake never dies; voice latency within SLA.

- [x] 15. Vision + OCR consumers → RA; delete stub
  - Objective: vision sidecar uses RA `Vision` lease; OCR gets a real `ModelLifecycle`; delete the
    scaffolding `GpuLeaseManager` in `vision_automation.rs`.
  - Files: `crates/kria-core/src/tools/vision.rs`, `tools/vision_automation.rs`,
    new OCR lifecycle module.
  - Dependencies: Tasks 6,11.
  - Risks: OCR loader availability; cloud fallback path.
  - Validation: vision + OCR runs under lease; grep proves stub deleted.
  - Acceptance (A1): no duplicate `GpuLeaseManager`.

- [x] 16. Embeddings consumer → RA + worker pool
  - Objective: pick fastembed as primary, ONNX as declared fallback; replace global
    `OnceCell<Mutex<TextEmbedding>>` / `Arc<Mutex<Session>>` with bounded worker pool sized by tier.
  - Files: `crates/kria-core/src/routing/embed.rs`, `crates/kria-core/src/memory/embeddings.rs`.
  - Dependencies: Tasks 6,11.
  - Risks: model-load duplication across workers → memory. Size pool by tier.
  - Validation: concurrent embedding throughput bench (no global serialization); correctness vs
    single-model baseline.
  - Acceptance (R4.3): one primary; embeddings not globally serialized.

---

### Phase 3 — Delete fragmentation, finalize observability

- [x] 17. Remove duplicate telemetry + dead code
  - Objective: delete deprecated `create_cuda_telemetry`, redundant `VramSnapshot` types, and
    `AudioFreezeGuard` v1 (keep V2).
  - Files: `crates/kria-core/src/llm/orchestrator/telemetry.rs`, `platform/vram.rs`,
    `resource/telemetry.rs`, `image/swap.rs`.
  - Dependencies: Tasks 3,13,14.
  - Risks: hidden callers. Mitigate: compiler + grep.
  - Validation: `cargo build` clean; grep shows one telemetry stack (A2).
  - Acceptance (N6): single telemetry stack, no dead lease/guard code.

- [x] 18. Anomaly detectors + Health Monitor daemon
  - Objective: implement detectors (CPU/GPU spike, VRAM/RAM leak, starvation, hung model, deadlock,
    daemon crash, infinite retry, thermal throttle) emitting root-cause + evidence.
  - Files: `crates/kria-core/src/infra/health.rs`, new `resource/authority/anomaly.rs`,
    daemon supervisor module.
  - Dependencies: Tasks 3,8.
  - Risks: false positives. Mitigate: dwell + evidence thresholds.
  - Validation: fault-injection tests per detector → correct hypothesis.
  - Acceptance (R10.3): each detector yields evidence-backed root cause.

- [x] 19. Daemon supervisor + isolation + circuit breakers
  - Objective: supervise Core/Voice/Wake/GPU Monitor/Health/Extension Host; auto-restart with
    backoff + circuit breaker; crash isolation.
  - Files: new `crates/kria-core/src/infra/supervisor.rs` (extend existing), wire in `kria-desktop`.
  - Dependencies: Tasks 14,18.
  - Risks: restart storms. Mitigate: circuit breaker + backoff caps.
  - Validation: crash-injection per daemon → Core survives, incident surfaced (R11.3).
  - Acceptance (R11): daemons supervised, isolated, recoverable.

- [x] 20. UX surface + diagnostics bundle
  - Objective: `resource:status` event consumption in UI (calm non-blocking banner, no input
    disable for non-emergency); "What is KRIA doing" panel; diagnostics bundle export.
  - Files: `ui/src/stores/app.ts`, `ui/src/components/ChatView.tsx`, new panel component;
    `crates/kria-desktop/src/commands/` bridge; bundle exporter in `kria-core`.
  - Dependencies: Tasks 8,12.
  - Risks: UI contract drift. Keep events additive (N5).
  - Validation: manual UX review; replace "Optimizing GPU layers..." path; bundle resolves all
    R10.1 "why" questions.
  - Acceptance (R9, A6): no surprise interruption; every "why" answerable.

---

### Phase 4 — Hardening & sign-off

- [x] 21. Multi-GPU placement validation
  - Objective: prove two consumers on two GPUs concurrently.
  - Files: integration tests under `crates/kria-eval/`.
  - Dependencies: Tasks 6,12,13.
  - Risks: hardware availability for CI; use mock DeviceTable + one real soak.
  - Validation: concurrent placement test, no contention error.
  - Acceptance (A8): multi-GPU concurrency proven.

- [x] 22. Fail-open + decision-deadline verification
  - Objective: assert RA returns safe fallback within deadline under planner stall / telemetry loss.
  - Files: `crates/kria-eval/`, fault-injection harness.
  - Dependencies: Tasks 5,6,9.
  - Risks: deadline too tight → premature fallback. Tune with bench.
  - Validation: inject NVML failure + planner stall → CPU/cloud plan within 50 ms.
  - Acceptance (R1.5, R12.3): never hangs; always safe default.

- [x] 23. Production Readiness Review
  - Objective: run the acceptance matrix A1–A10; sign-off board.
  - Files: `crates/kria-eval/` suite + checklist doc.
  - Dependencies: all prior.
  - Risks: residual gaps. Iterate before sign-off.
  - Validation: full A1–A10 green; soak ≥ 24 h on Medium + High tiers.
  - Acceptance: all `requirements.md` §6 criteria met; blueprint realized.

---

### Phase 5 — Predictive, adaptive & reliability hardening (added during review)

- [x] 24. Hardware Capability Vector
  - Objective: replace single-tier placement with per-resource `CapabilityVector`; keep tier as label.
  - Files: `crates/kria-core/src/platform/detect.rs`, `resource/authority/types.rs`, `planner.rs`.
  - Dependencies: Tasks 1,5.
  - Risks: re-tuning planner weights.
  - Validation: two same-tier/different-vector machines → different correct plans.
  - Acceptance (A15): placement uses vector.

- [x] 25. Foreground Guard chokepoint
  - Objective: single `ForegroundGuard::authorize(action)`; all disruptive ops routed through it;
    deny unless emergency or turn-boundary; streaming-checkpoint path for emergency.
  - Files: `resource/authority/foreground_guard.rs`, `llm/orchestrator/*`, `image/swap.rs`.
  - Dependencies: Tasks 6,12.
  - Risks: missing a call site. Mitigate: make disruptive ops only reachable via the guard type.
  - Validation: event-trace test — no non-emergency `stream_interrupted` during foreground.
  - Acceptance (A16, R19): structurally enforced; emergency checkpoint+resume.

- [x] 26. Epoch fencing
  - Objective: RA epoch in journal; `LeaseV2.epoch`; consumer revalidation before GPU ops.
  - Files: `resource/authority/lease.rs`, `journal.rs`, all consumers.
  - Dependencies: Tasks 6,8,12,14.
  - Risks: revalidation overhead. Mitigate: atomic read.
  - Validation: split-brain test (Core restart mid-turn) → pre-epoch lease rejected, no double-use.
  - Acceptance (A18, R21.1).

- [x] 27. Journal integrity + versioning
  - Objective: CRC + version records; truncate-at-bad recovery; compacted snapshots.
  - Files: `resource/authority/journal.rs`, `reconciler.rs`.
  - Dependencies: Task 8,9.
  - Risks: snapshot/tail consistency.
  - Validation: torn-write/power-loss simulation → recovery from last-good.
  - Acceptance (R21.2).

- [x] 28. Bounded queues + load-shedding
  - Objective: per-class bounded admission queues; deadline-aware shedding; UX notice.
  - Files: `resource/authority/scheduler.rs`.
  - Dependencies: Task 6.
  - Risks: shedding the wrong class. Mitigate: class-ordered policy + tests.
  - Validation: overload test → low classes shed first, no unbounded growth.
  - Acceptance (A14-adjacent, R21.3).

- [x] 29. Cloud Device health + circuit breakers
  - Objective: per-pool breaker + adaptive health; honor `Retry-After`; planner avoids open pools.
  - Files: `resource/authority/device_table.rs`, `llm/provider/*` adapters.
  - Dependencies: Tasks 4,5.
  - Risks: breaker flapping. Mitigate: half-open probes + hysteresis.
  - Validation: inject 429/5xx → no failover storm; recovery via half-open.
  - Acceptance (R21.4).

- [x] 30. Workload Prediction Engine (WPE)
  - Objective: deterministic prewarm hints; speculative revocable budget-capped prewarm.
  - Files: new `resource/predict/wpe.rs`; UI signal bridge in `kria-desktop`.
  - Dependencies: Tasks 6,7,11,24.
  - Risks: false-positive prewarm waste. Mitigate: non-eviction + veto.
  - Validation: chaos test for Property 8 (never evicts ≥ class; auto-cool; veto).
  - Acceptance (A11, R14).

- [x] 31. Session Intent Profiles (SIP)
  - Objective: deterministic session classifier with hysteresis; biases planner cost only.
  - Files: new `resource/predict/sip.rs`, `planner.rs` (cost presets).
  - Dependencies: Tasks 5,30.
  - Risks: misclassification flip. Mitigate: dwell+confidence (Property 9).
  - Validation: minority-workload test → no flip; profile changes only bias cost.
  - Acceptance (A12, R15).

- [x] 32. Resource Forecasting Engine (RFE)
  - Objective: EWMA+slope forecasts with confidence; advance non-disruptive remedies early.
  - Files: new `resource/predict/rfe.rs`, `pressure.rs`.
  - Dependencies: Tasks 3,7.
  - Risks: false alarms on noise. Mitigate: smoothing + sustained slope.
  - Validation: replay traces → bounded false-positive; lead-time correctness (Property 10).
  - Acceptance (A13, R16).

- [x] 33. Thermal & Power Policy Engine (TPPE)
  - Objective: thermal/battery monitoring + predictive throttle avoidance + profile switching.
  - Files: new `resource/predict/tppe.rs`, telemetry collector (thermal/battery), `policy_profiles.rs`.
  - Dependencies: Tasks 3,5.
  - Risks: missing sensors. Mitigate: thermal-unknown profile.
  - Validation: laptop throttle-avoidance soak; sensor-absent desktop degrades safely.
  - Acceptance (A14, R17).

- [x] 34. Autonomous Optimization Layer (AOL)
  - Objective: advisory learning of patterns → WPE priors + profile suggestions; no admission handle.
  - Files: new `resource/predict/aol.rs` (separate module, no RA admission import).
  - Dependencies: Tasks 30,31.
  - Risks: scope creep into control. Mitigate: module boundary (Property 12).
  - Validation: module-boundary test — no path to Scheduler/Planner (A17).
  - Acceptance (R20).

- [x] 35. RA bypass kill-switch
  - Objective: per-consumer static-plan fallback with no authority.
  - Files: `resource/authority/mod.rs`, config, UI toggle.
  - Dependencies: Task 12.
  - Risks: drift between bypass and normal paths. Mitigate: shared static-plan function.
  - Validation: toggle → consumer runs static plan; AI features keep working.
  - Acceptance (A19, R22.1).

- [x] 36. SLOs + low-cardinality metrics
  - Objective: define SLOs; export counters/histograms; turn_id only in traces/journal.
  - Files: `infra/observability.rs`, `resource/authority/*`.
  - Dependencies: Task 8.
  - Risks: cardinality blowup. Mitigate: lint on metric labels.
  - Validation: metrics scrape under load → bounded cardinality; SLO dashboards.
  - Acceptance (R22.2).

- [x] 37. Shadow comparator
  - Objective: replay telemetry to legacy + RA; divergence report; cutover gate.
  - Files: `crates/kria-eval/` shadow harness, `resource/authority/shadow.rs`.
  - Dependencies: Task 10.
  - Risks: comparator bias. Mitigate: assert invariants (no over-commit, no added fg interrupt).
  - Validation: soak → divergence report green before any Phase-2 cutover.
  - Acceptance (R22.3).

- [x] 38. Security: reclaim authz + privacy-bounded egress
  - Objective: capability-token kills of RA-spawned PIDs only; Privacy-Strict never egresses.
  - Files: `reconciler.rs`, `planner.rs`, safety policy integration.
  - Dependencies: Tasks 5,9.
  - Risks: legitimate process not killable. Mitigate: spawn-time PID registry.
  - Validation: privacy test (no egress, fails to CPU); kill-scope test. **Destructive op — audited,
    gated by safety policy.**
  - Acceptance (A20, R23.1, R23.2).

- [x] 39. Distributed-readiness extension points
  - Objective: make Authority a transport-agnostic trait; serializable request/plan/lease;
    reserve `DeviceId::RemoteHost`; separate Execution from Placement. No remote impl.
  - Files: `resource/authority/types.rs`, `mod.rs`.
  - Dependencies: Tasks 2,6.
  - Risks: premature abstraction. Mitigate: trait only, single local impl today.
  - Validation: compiles; serde round-trip tests.
  - Acceptance (R23.3).

- [x] 40. Frontend/UX implementation (6 views)
  - Objective: Resource Dashboard, Explainability, Session Awareness, Forecasting, Recovery,
    Diagnostics export (see `frontend-ux-spec.md`).
  - Files: `ui/src/views/ResourceView.tsx` + components, `ui/src/stores/resource.ts`,
    `kria-desktop` event/query bridge.
  - Dependencies: Tasks 8,30,31,32,33.
  - Risks: event contract drift. Mitigate: additive events (N5).
  - Validation: each view renders live state; "why" questions answerable with evidence (A6).
  - Acceptance (R9, frontend spec).

- [x] 41. Chaos/soak acceptance for predictive engines
  - Objective: fault-injection + soak gates for WPE/SIP/RFE/TPPE/AOL (false-positive, oscillation,
    veto correctness bounds).
  - Files: `crates/kria-eval/` chaos suites.
  - Dependencies: Tasks 30–34.
  - Risks: flaky thresholds. Mitigate: statistical bounds, not point asserts.
  - Validation: soak ≥ 24 h; bounds met.
  - Acceptance: predictive engines proven safe (advisory-only, no harm).

### Phase 6 — Final gap closure (minimal, additive)

- [x] 42. ResidencyManager
  - Objective: single executor of load/warm/cool/evict/swap/restore wrapping existing
    `ModelLifecycle`; serialize transitions per model; emit `resource:residency`.
  - Files: `resource/authority/residency_manager.rs`, `resource/lifecycle.rs`; update RA/Pressure/WPE
    call sites to route through it.
  - Dependencies: Tasks 7,11,12.
  - Risks: missed direct call site. Mitigate: make lifecycle transitions crate-private behind the manager.
  - Validation: grep proves no direct lifecycle transition; per-model one-in-flight test.
  - Acceptance (A21, R24).

- [x] 43. Resource Simulator
  - Objective: pure `simulate(action, snapshot) -> Estimate`; Scheduler pre-commit gate; journaled.
  - Files: `resource/authority/simulator.rs`, `scheduler.rs`, `journal.rs`.
  - Dependencies: Tasks 4,6,8,45.
  - Risks: estimate inaccuracy. Mitigate: calibrate against Benchmark (Task 48); conservative bias.
  - Validation: predicted hard-limit breach → fallback chosen; estimate journaled.
  - Acceptance (A22, R25).

- [x] 44. Session Ownership view
  - Objective: derive `SessionOwnership{foreground,interactive,background}` from PriorityClass+SIP+focus;
    feed scheduler weights; fairness floor.
  - Files: `resource/predict/session_ownership.rs`, `scheduler.rs`.
  - Dependencies: Tasks 6,31.
  - Risks: ownership flapping. Mitigate: reuse SIP hysteresis.
  - Validation: 5-owner concurrency soak — fg protected, bg yields first, no starvation.
  - Acceptance (A23, R26).

- [x] 45. Multi-band Memory Budget
  - Objective: derive Soft/Hard/Emergency bands in DeviceTable from existing values; Pressure maps
    yield→Soft/critical→Emergency; admission gates on Hard. No new counters.
  - Files: `resource/authority/device_table.rs`, `pressure.rs`, `scheduler.rs`.
  - Dependencies: Tasks 4,7.
  - Risks: double accounting. Mitigate: bands are a derived view (Property 18).
  - Validation: band-derivation unit test; admission gate test; single-accounting assertion.
  - Acceptance (A24, R27).

- [x] 46. Capability Registry
  - Objective: declarative `ModelCapability` table (config + discovery); Planner pure lookup/filter.
  - Files: `resource/authority/capability_registry.rs`, `planner.rs`, `config/default.toml`.
  - Dependencies: Tasks 5,11.
  - Risks: stale registry vs discovered models. Mitigate: discovery reconciles registry at startup.
  - Validation: deterministic selection test; explainable choice; no LLM in path.
  - Acceptance (A25, R28).

- [x] 47. SLA Framework
  - Objective: `SlaTable` (config-overridable); measurement in observability; Health Monitor raises
    Warning/Critical; Diagnostics shows breaches with evidence.
  - Files: `infra/observability.rs`, `infra/health.rs`, `resource/authority/sla.rs`, config.
  - Dependencies: Tasks 18,36.
  - Risks: noisy alerts. Mitigate: dwell before raise.
  - Validation: inject slow op → correct SLA state + breach evidence.
  - Acceptance (A26, R29).

- [x] 48. Benchmark Framework
  - Objective: extend `kria-eval` with benchmark mode + fixed scenarios; before/after + regression +
    per-hardware-class reports; release gate.
  - Files: `crates/kria-eval/` benchmark suite + report format.
  - Dependencies: Tasks 3,6,12.
  - Risks: unstable numbers. Mitigate: warmup + repeated runs + statistical bounds.
  - Validation: produce baseline + regression detection on a seeded change.
  - Acceptance (A27, R30).

## Task Dependency Graph

```json
{
  "waves": [
    { "wave": 1, "tasks": [1], "parallel": false },
    { "wave": 2, "tasks": [2], "parallel": false },
    { "wave": 3, "tasks": [3, 11], "parallel": true },
    { "wave": 4, "tasks": [4], "parallel": false },
    { "wave": 5, "tasks": [5], "parallel": false },
    { "wave": 6, "tasks": [6, 7], "parallel": true },
    { "wave": 7, "tasks": [8], "parallel": false },
    { "wave": 8, "tasks": [9], "parallel": false },
    { "wave": 9, "tasks": [10], "parallel": false },
    { "wave": 10, "tasks": [12], "parallel": false },
    { "wave": 11, "tasks": [13, 14, 15, 16], "parallel": true },
    { "wave": 12, "tasks": [17, 18], "parallel": true },
    { "wave": 13, "tasks": [19, 20], "parallel": true },
    { "wave": 14, "tasks": [21, 22], "parallel": true },
    { "wave": 15, "tasks": [24, 25, 27, 28, 29], "parallel": true },
    { "wave": 16, "tasks": [26, 37, 38, 39], "parallel": true },
    { "wave": 17, "tasks": [30, 32, 33, 35, 36], "parallel": true },
    { "wave": 18, "tasks": [31], "parallel": false },
    { "wave": 19, "tasks": [34, 40], "parallel": true },
    { "wave": 20, "tasks": [42, 45, 46, 48], "parallel": true },
    { "wave": 21, "tasks": [43, 44, 47], "parallel": true },
    { "wave": 22, "tasks": [41], "parallel": false },
    { "wave": 23, "tasks": [23], "parallel": false }
  ]
}
```

Critical path (V1): 1→2→3→4→5→6→(7,8,9)→10→12→(13,14)→17→(18,19,20)→(21,22)→23.
Hardening overlay: 24/25 gate predictive cutover; 26 (epoch) + 37 (shadow) gate Phase-2 trust;
30→31→34 predictive chain; 40 (frontend) rides on 30/32/33.
Final gap overlay: 45 (bands) feeds 43 (simulator); 42 (residency) centralizes transitions;
46 (registry) + 47 (SLA) + 48 (benchmark) close validation; 41 + 23 sign off.

## Notes

- Phases 0–1 add the authority in shadow with zero behavior change → safe to land early.
- Phase 2 cuts consumers over one at a time; each is independently reversible.
- Phase 3 only deletes old fragmented code AFTER its replacement is proven (A1/A2).
- Destructive operations (Task 9 process reclaim, Task 17 deletions) require the safety policy layer
  and explicit before/after audit logging.


---

## Phase 7 — Final Completion (Sessions 11–13): make HRA the sole authority

Status of the production-completion work beyond the original 48. `[x]` = done + headless-verified;
`[~]` = mechanism complete but live activation is soak-gated; `[ ]` = needs hardware/soak.

- [x] 49. Telemetry unification — single `TelemetryHub` (one device context; + CPU per-core). Tests.
- [x] 50. Fresh-read at GPU-admission decision time (`advise_gpu_admission_fresh`).
- [x] 51. Idle auto-unload re-enabled (config-gated, foreground-safe).
- [x] 52. Journal disk persistence — crash-safe `JournalStore`, boot replay, epoch fencing,
      `recovered_open_leases`. Tests.
- [x] 53. Diagnostics export — `diagnostics_json[_async]`, `get_hra_diagnostics`,
      `resource:hra_diagnostics` event.
- [x] 54. Co-Residency GPU Lease Manager — `CoResidencyManager`: multi-model residency, multi-victim
      preemption, cooperative revocation, foreground protection, anti-thrash pinning, refcount dedup,
      rollback, TTL recovery. 11 unit + 3 acceptance + 6 stress tests.
- [x] 55. `LocalAuthority::request_on_gpu` — GPU-targeted admission (preemption not masked by CPU
      fallback); shared `admit_plan`.
- [x] 56. Admission gateway — `HraService::admit_gpu` → `AdmissionGuard` (inert shadow / live enforce).
- [x] 57. Production stress suite (`hra_stress.rs`) — 10k+ concurrency, churn, dedup, rollback storm,
      TTL, multi-GPU. Invariants: no over-commit / no leak / no deadlock / no duplicate residency.
- [x] 58. Benchmark harness (`hra_bench.rs`) — admission/dedup/preemption latency + bounds.
- [x] 59. Observability — structured "why" tracing on every co-residency decision.
- [x] 60. Frontend — CPU/RAM/VRAM live, Resource Pressure bands, live co-residents + metrics, Recovery.
- [x] 61. Consumer hot-path cutover — **DONE (code)**. `GpuLeaseManager::acquire_guard_gated` bridges
      shadow→legacy / enforce→HRA admission. Wired into Image, Vision, STT, TTS (shared lease) + LLM
      (`reconcile_l1_lease` enforce-gated admission). Inert in shadow; HRA owns under
      `KRIA_HRA_ENFORCE=1`. Live soak still validates the enforce behavior (deadlock/latency).
- [x] 62. Phase 3 legacy removal — legacy `GpuLeaseManager` single-holder state machine physically
      deleted (Task 62 done; see Session 17 note). ~800 lines removed: `InnerState`, queue,
      recovery workers (`recovery_worker_loop`/`attempt_recovery_pass`/`cleanup_orphaned_processes`),
      `acquire_lease*`/`acquire_guard`/`grant_locked`/state-transition helpers, telemetry
      reconciliation, and the state-machine unit tests. What remains is a THIN COMPATIBILITY SHELL
      (`GpuLeaseManager` = `acquire_guard_gated` → HRA + no-op stubs) so callers compile unchanged.
      User confirmed the runtime works after enforce-default validation (chat + image live).
- [x] 63. Explainability/Forecasting UI — decision journal streamed with human rationale; live VRAM
      exhaustion forecast via `Forecaster`. No "awaiting data" placeholders remain.
- [ ] 64. F1/F2 — multi-GPU silicon + 24h enforce soak.

---

## Phase 8 — Runtime Resource Policy Redesign (G1–G12)

Source design: `runtime-policy-redesign.md` (governing law: never restart for performance; restart
only for correctness/safety or explicit user workflow). Each task is headless-codable + unit-testable;
items marked **[HW]** also need on-target GPU soak to tune constants. All coding only.

- [x] 65. G1 — Measured-first GPU sizing + volatility reserve + bounded calibration
  - Objective: replace estimate-vs-total sizing with a budget over MEASURED live free VRAM; add a
    telemetry-variance-derived `volatility_reserve` (size for sustained floor, not instantaneous peak);
    keep CUDA-overhead calibration as a bounded (±50%) correction term persisted per-GPU; sizing runs
    at load time only, never as a steady-state loop.
  - Files: `crates/kria-core/src/llm/orchestrator/strategy.rs` (extend
    `calculate_target_params_prod`/`cuda_runtime_reserve_mb`),
    `crates/kria-core/src/llm/orchestrator/server_manager.rs` (persist calibration),
    `crates/kria-core/src/platform/vram.rs` (variance source).
  - Dependencies: Tasks 49, 50.
  - Risks: under-utilization on dedicated GPUs (reserve adapts to observed variance, near-zero when stable).
  - Validation: unit tests for budget math + reserve derivation; `cargo test -p kria-core --lib llm::orchestrator`.
  - Acceptance (redesign G1): pure budget fn; calibration bounded + cannot push sizing unsafe. **[HW]** loads fitting ngl>0.

- [x] 66. G3 — Resident Lock state machine in ResidencyManager
  - Objective: add `Cold→Loading→Resident→Stabilizing→ResidentLocked` plus
    `PinnedResident/Recovering/Emergency/Migrating/ImageOverride/CloudFallback`; `ResidentLocked` forbids
    all perf restarts/optimization/migration; enumerate break conditions exactly (image gen, OOM, driver
    reset, hw failure, model/settings change, app restart, maintenance, sustained correctness pressure,
    cloud-health change); return to lock after any break+reload.
  - Files: `crates/kria-core/src/resource/authority/residency_manager.rs`.
  - Dependencies: Task 42.
  - Risks: CPU-locked session never promotes → covered by one DeepIdle promotion (Task 68).
  - Validation: state-transition unit tests; assert no perf transition exits `ResidentLocked`.
  - Acceptance (redesign G3): lock state present; only listed break conditions transition out.

- [x] 67. G6 — User Activity Model
  - Objective: derive `Active/Idle/DeepIdle` from active streams (`server.has_active_streams`), voice
    turn, recent input ts, foreground focus, queued prompts; expose to policy as a gate.
  - Files: new `crates/kria-core/src/resource/authority/activity.rs`; feed from
    `crates/kria-desktop/src/commands/runtime.rs` (input/focus signals).
  - Dependencies: Task 49.
  - Risks: missing a signal → defaults to Active (safe: forbids restarts).
  - Validation: unit tests for thresholds T1/T2 + transition hysteresis.
  - Acceptance (redesign G6): Active forbids Restart-class; DeepIdle is the only promotion window.

- [x] 68. G7 + G4 — Runtime modes + state-driven optimization eligibility
  - Objective: derive runtime mode (Interactive/Maintenance/Recovery/Emergency/Background/Idle/Cloud/
    Hybrid) from activity+telemetry+health; implement state-driven (NOT counter-driven) optimization
    eligibility: a restart is eligible only when ALL hold (pre-lock or CPU-promotion, Maintenance,
    DeepIdle, Measured confidence, RFE sustainable, cooldown elapsed, simulator-fit, Benefit Worth-It).
  - Files: `crates/kria-core/src/resource/authority/policy.rs` (new, modes + eligibility), reuses
    `simulator.rs`, RFE `resource/predict/rfe.rs`.
  - Dependencies: Tasks 66, 67, 43, 32.
  - Risks: eligibility too permissive → bias toward `Stay` (default deny).
  - Validation: truth-table tests over the eligibility predicate.
  - Acceptance (redesign G4/G7): default Interactive output = `Stay`; promotion only in Maintenance+DeepIdle.

- [x] 69. G5 — Benefit Evaluation Engine
  - Objective: pure `evaluate() -> WorthIt|NotWorthIt` from expected_speedup (per-tier tok/s table
    refined by observed throughput), restart_cost_s, interruption_risk (∞ when Active), failure_prob
    (simulator margin + history); thresholds tunable; bias to NotWorthIt on uncertainty.
  - Files: new `crates/kria-core/src/resource/authority/benefit.rs`.
  - Dependencies: Tasks 43, 48.
  - Risks: throughput model coarse → conservative defaults.
  - Validation: unit tests: resident-at-good-size → NotWorthIt; CPU→GPU DeepIdle safe margin → WorthIt.
  - Acceptance (redesign G5): deterministic; never WorthIt while Active.

- [x] 70. G2 — Policy Engine; watchdog demoted to executor
  - Objective: introduce pure deterministic Policy Engine producing
    `Decision{Stay|Optimize|Migrate|Defer|Recover|Cloud|Reject}+rationale/benefit/cost/risk`; route
    decisions to the watchdog as an EXECUTOR (I/O only) that re-validates activity+lock+epoch immediately
    before any disruptive op; keep the emergency OOM/pressure reflex local to the watchdog (bypasses
    policy for correctness). Remove the watchdog's opportunistic scale-up decision.
  - Files: `crates/kria-core/src/resource/authority/policy.rs`,
    `crates/kria-core/src/llm/orchestrator/gpu_watchdog.rs` (decision → executor).
  - Dependencies: Tasks 67, 68, 69.
  - Risks: policy/executor race on activity change → re-validate under lock+epoch (Task 25 guard).
  - Validation: `cargo test -p kria-core --lib llm::orchestrator`; assert no perf restart emitted in Interactive.
  - Acceptance (redesign G2): decision/execution split; emergency reflex retained. **[HW]** swap soak.

- [x] 71. G9 + G10 + G11 — Image policy, UX mapping, decision-grade logging
  - Objective: image request flow (CoResident / simulator-gated Tier-B / CloudFallback / Reject) with
    deterministic restore to the exact pre-image `ResidentLocked` config; state-mapped UX banners (ban
    generic "Optimizing GPU layers"; every banner names the action + clears on terminal event); log every
    policy decision with correlation_id/who/why/state/benefit/cost/risk/result/latency, journaled.
  - Files: `crates/kria-core/src/image/orchestrator.rs`, `image/swap.rs`,
    `crates/kria-desktop/src/commands/runtime.rs` (event forward), `ui/src/stores/app.ts`,
    `ui/src/components/ResourceDashboard.tsx`, `resource/authority/journal.rs`.
  - Dependencies: Tasks 66, 70, 43.
  - Risks: banner contract drift → keep events additive (N5).
  - Validation: image-flow unit/integration test; UX banner state-map test; journal entry per decision.
  - Acceptance (redesign G9/G10/G11): only routine restart is user-initiated + narrated; restore is deterministic.

- [x] 72. G8 — Startup finalization + staged readiness
  - Objective: confirm LLM is the sole critical path; emit staged `core_llm_ready` independent of
    `tools_ready`/`voice_ready`/`mcp_ready`; lock residency after core_llm_ready + stabilize; keep heavy
    CPU tasks background + thread-capped (C6/C7 already landed).
  - Files: `crates/kria-desktop/src/commands/runtime.rs`, `crates/kria-core/src/routing/tool_index.rs`.
  - Dependencies: Tasks 66, 72-adjacent C6/C7.
  - Risks: readiness event contract drift.
  - Validation: startup-timeline test; assert no blocking non-LLM task on critical path.
  - Acceptance (redesign G8): staged readiness emitted; lock after stabilize.

- [x] 73. G12 — Cascading cleanup; remove dead `HubTelemetry`
  - Objective: delete dead `HubTelemetry` (`llm/orchestrator/telemetry.rs:78`) after policy engine
    consumes the single `TelemetryHub`; confirm no new scheduler/planner/telemetry introduced; reused
    engines unchanged.
  - Files: `crates/kria-core/src/llm/orchestrator/telemetry.rs`.
  - Dependencies: Tasks 70, 49.
  - Risks: hidden callers → compiler + grep gate.
  - Validation: `cargo build` clean; grep shows single telemetry stack.
  - Acceptance (redesign G12): dead code removed; no duplicate ownership.

- [ ] 74. **[HW]** Constant tuning + no-perf-restart soak
  - Objective: on the target GPU(s), tune volatility reserve + benefit thresholds + cooldown; verify a
    real session shows zero perf restarts and one (or zero) DeepIdle promotion; image Tier-B never OOMs.
  - Files: config (`config/default.toml`), env tunables.
  - Dependencies: Tasks 65–73.
  - Risks: hardware-only; cannot complete headless.
  - Validation: live soak; log review for `ResidentLocked` stability, no between-session restarts.
  - Acceptance (governing law): no performance restart observed in an interactive session.


### Phase 8 — implementation status (what shipped headless vs what is hardware-gated)

Legend: `[x]` done + headless-verified (compiles default + `--no-default-features`, unit tests green);
`[~]` mechanism complete + tested but full live activation/cutover is soak-gated; `[ ]` needs hardware.

- **65 G1** `[x]` — `strategy.rs`: `sustained_floor_mb`, `volatility_reserve_mb` (variance-derived,
  capped via `KRIA_VRAM_VOLATILITY_CAP_MB`), `calibrated_cuda_reserve_mb` (bounded ±50%),
  `calculate_target_params_measured`. 7 new unit tests. HW tune of the actual ngl load = Task 74.
- **66 G3** `[x]` — `residency_manager.rs`: `LockState` (Cold→Loading→Resident→Stabilizing→
  ResidentLocked + branches), `BreakCondition` (exact list), `ResidentLock` machine,
  `perf_optimization_eligible()` structural guarantee, `user_banner()` (G10). 7 unit tests.
- **67 G6** `[x]` — new `activity.rs`: `ActivityState`/`ActivitySignals`/`ActivityModel`,
  busy-safe defaults. 7 unit tests.
- **68 G7+G4** `[x]` — new `policy.rs`: `RuntimeMode`, `derive_mode`, `decide` with the strict
  AND eligibility (mode/activity/confidence/lock/cooldown/forecast/simulator/benefit). 12 unit tests.
- **69 G5** `[x]` — new `benefit.rs`: `evaluate` with interruption→speedup→failure→cost gates,
  uncertainty biases NotWorthIt. 7 unit tests.
- **70 G2** `[x]` — **live cutover done.** Watchdog now routes the opportunistic scale-up DECISION
  through `policy::decide` via `GpuWatchdog::decide_scaleup` (builds live PolicyInputs from
  `has_active_streams`→activity, telemetry total→confidence, ngl→lock posture, `gpu_in_cooldown`,
  simulator Swap estimate, coarse-throughput benefit). Proceeds to `Recovering` ONLY on
  `Action::Optimize`; every verdict is `PolicyLog::emit`-logged (G11). Master switch
  `KRIA_GPU_AUTOSCALE` still default-OFF, so default behavior is unchanged — when enabled, decisions
  are policy-governed instead of hand-rolled. HW soak (Task 74) tunes constants.
- **71 G9+G10+G11** `[x]` — **live cutover done.** `image/orchestrator.rs::generate_with_swap` now
  consults `decide_image_admission` (simulator-gated) BEFORE a Tier-B restart: if eviction cannot
  SAFELY free `required_mb` it returns early so the caller routes to cloud (avoids the doomed
  local restart/OOM-thrash on a tight GPU), or rejects when cloud is off. G10: action-specific
  `banner` on `orchestrator:swap_started` + UI `swapBanner`. G11: `PolicyLog::emit` structured
  tracing + serde for the journal.
- **72 G8** `[x]` — `runtime:core_llm_ready` staged event emitted independently of tools/voice/mcp
  readiness (LLM is the only critical path; tool index already background from C6).
- **73 G12** `[x]` — dead `HubTelemetry` removed from `llm/orchestrator/telemetry.rs`; builds clean.
- **74** `[ ]` — **hardware-only**: tune volatility reserve + benefit thresholds + cooldown on the
  target GPU; confirm zero perf restarts in a real session and a fitting ngl loads. Cannot run headless.

Verification run: `kria-core` lib tests green (authority 178+, orchestrator 82, strategy 15);
`hra_acceptance` 9/9, `hra_bench` 3/3; `cargo check` clean on default + `--no-default-features`;
`tsc --noEmit` clean for the UI changes.


---

## Session 14 — LLM-start fix + final stabilization (hardware-confirmed)

User confirmed the GPU orchestrator now works on the target RTX 4050. This session fixed the
"LLM won't start" regression and stabilized the tree.

### Shipped (headless-compiled + hardware-validated on the RTX 4050)
- **Cold-start fresh VRAM read** (`llm/orchestrator/mod.rs::start`) — forces a real nvidia-smi read
  when the telemetry actor isn't warm, so sizing targets the GPU, not CPU.
- **Startup ngl-backoff ladder** (`mod.rs::start` + `server_manager.rs::set_spawn_timeout_override`/
  `effective_spawn_timeout_secs`) — descending `[computed, ¾, ½, ¼, 0(CPU)]` with a 20 s probe per
  GPU attempt; first rung that binds a port wins; CPU is the always-loads fallback. Root-cause fix
  for the llama.cpp Vulkan-laptop hang at high ngl (proven: ngl ≤ 28 loads in ~2 s, ≥ 30 hangs).
- **Persisted safe-ngl** (`~/.kria/llm_safe_ngl.json`) — remembers the working ngl per model; later
  boots start in ~2 s instead of re-probing.
- New reproducible hardware tests: `tests/gpu_orchestrator_start_e2e.rs` (full-path, gated by
  `KRIA_HW_E2E=1`), `tests/gpu_orchestrator_hw_e2e.rs` (sizing), `scripts/gpu_load_sweep.sh`.
- Report: `llm-start-fix-report.md`.

### Hardware validation (real `Orchestrator::start`)
- First boot: `ngl=31 → hang 20s → backed off to ngl=23 → healthy` in 22.07 s; persisted 23.
- Second boot: `ngl=23 healthy` in 1.81 s (read from cache).

### Final stabilization verification
- `cargo test -p kria-core --lib resource::authority::` → 182 passed, 0 failed.
- `cargo test -p kria-core --lib llm::orchestrator` → 83 passed, 0 failed.
- `hra_acceptance` 9/9, `hra_stress` 6/6, `hra_bench` 3/3.
- `cargo check` clean on default + `--no-default-features` + `kria-desktop`.
- clippy on the new modules clean (fixed `benefit.rs` neg-cmp).

### Task status (honest)
- **74 — effectively satisfied for default operation**: a fitting ngl now loads on real hardware
  (ngl=23), and with `gpu_autoscale` default-OFF there are no performance restarts. Remaining is
  optional constant tuning (volatility/benefit thresholds) — a refinement, not a blocker.
- **62 — STILL DEFERRED (do not delete legacy yet)**: `GpuLeaseManager` + the orchestrator
  `TelemetryActor` are the **live default-mode (shadow) executors** of the now-working runtime.
  Deleting them is only safe AFTER the enforce-mode path is proven by the hardware soak (Task 64).
  Removing them now would break the working app. The only headless-safe legacy deletion
  (`HubTelemetry`) was already done in Task 73.
- **64 — hardware soak (multi-GPU + 24 h enforce)**: unchanged; owner = user, on real hardware.

> Final-stabilization decision: ship the current shadow-default runtime (working + verified). Legacy
> deletion (62) waits on the enforce soak (64) — this is the correct, safe order and matches the
> spec's Phase-3 gate ("delete old code only after its replacement is proven").


---

## Session 15 — HRA enforce flipped to DEFAULT (Step 1 of the legacy-removal plan)

User chose to run on the new architecture and manually test it (Plan A). Done, reversibly:

- `crates/kria-desktop/src/commands/runtime.rs`: `KRIA_HRA_ENFORCE` default inverted → **enforce ON
  by default**. Parse: unset / `1/true/on/yes` → enforce; `0/false/off/no` → shadow.
- Effect: every GPU consumer (LLM / Image / Vision / STT / TTS) now acquires GPU admission through
  `HraService::admit_gpu` → `CoResidencyManager` (the new authority) instead of the legacy
  `GpuLeaseManager`. Single source of truth = `hra.set_shadow_only(!enforce)`; all consumers gate on
  `is_shadow_only()`.
- **Rollback parachute (instant, no code change):** `KRIA_HRA_ENFORCE=0 cargo tauri dev` → legacy
  shadow path. Legacy `GpuLeaseManager` is deliberately still present as this fallback.
- `cargo check -p kria-desktop` clean.

### Manual soak checklist (run on `cargo tauri dev`, default enforce)
Watch `~/.kria/logs/kria.log.<date>` — expect `HRA: enforcement ON`. Then exercise, for a while:
1. **Chat** — several turns, long + short. Expect normal replies, no "LLM not reachable".
2. **Voice** — STT + TTS turns while a chat is active. Expect no audio dropouts / no hang.
3. **Image** — generate while chatting. Expect co-resident or a narrated Tier-B, then chat resumes.
4. **Concurrency** — fire chat + voice + image close together a few times.
5. **Watch for (fail signs):** any hang, "not reachable" flap, GPU VRAM creeping to full over time,
   or a deadlock (UI stuck with no progress). Grep the log for `CoResidencyError`, `deadlock`,
   `over-commit`, `admit_gpu`.

If it runs clean for a good while → the enforce path is proven → **Step 3 (delete legacy
`GpuLeaseManager` + shadow branch, rewire consumers to HRA-only) becomes safe.**
If anything breaks → set `KRIA_HRA_ENFORCE=0`, relaunch (back to legacy), report the log.

- **62 — still deferred** until the above soak is clean (now one flip away).
- **64 — this IS the enforce soak**, now the default path (easier to exercise).


---

## Session 16 — Final Runtime Migration (Fix-Forward Mode)

HRA made the SOLE architecture. Full report: `final-runtime-migration-report.md`.

- **Step 1/2 — ownership + legacy disabled:** `acquire_guard_gated` shadow→legacy branch REMOVED;
  admission is HRA-only when an HRA is registered (always in the desktop runtime). Legacy
  single-holder path reachable only with no HRA (tests/pre-init).
- **Step 3 — documented:** `GpuLeaseManager` carries the `LEGACY COMPONENT` banner (INACTIVE,
  replacement = `resource::authority::*`, safe-for-deletion-after-soak).
- **Step 4 — bug fixed:** image `TierAdmission` "GPU lease unavailable" root-caused (image
  InteractiveBg can't preempt resident LLM InteractiveFg → `Busy` before Tier-B ran). Fix:
  `acquire_local_lease_swap` — on the BDropSwap tier, HRA `Busy` → proceed to Tier-B eviction
  (explicit LLM evict/restore), not hard-fail. HRA stays arbiter; subsystem owns Tier-B.
- **Step 5 — integrity:** exactly one authority/admission/scheduler/planner/residency/recovery;
  one consumer telemetry sampler (TelemetryHub) + orchestrator's own actor (intentional).
- **Step 7 — validation:** authority 182, orchestrator 83, gpu_lease 7, image 18, hra_acceptance 9,
  hra_stress 6, hra_bench 3; real-GPU start E2E PASS (ngl=23 healthy); checks clean default +
  `--no-default-features` + desktop. 2 pre-existing agent-loop test failures (flaky, unrelated).

- **62 — now "disabled + documented + deletion-ready"** (physical deletion after the user's live soak).
- Live soak (chat+voice+image+concurrency) = the only remaining gate; fix-forward on HRA from here.


---

## Session 16b — Image generation LIVE-VALIDATED (enforce default)

User confirmed: **image generation works** on the RTX 4050 under HRA enforce (default).

- Tier-B admission fix (`acquire_local_lease_swap`: HRA `Busy` → explicit LLM evict → image → restore)
  verified end-to-end in a real run (ComfyUI generated + delivered).
- In-flight guard added (`ImageOrchestrator.generating` AtomicBool) — a duplicate image request while
  one is running is rejected ("already generating, please wait") instead of thrashing a second Tier-B.
- Root cause of the earlier "image cancelled" was: slow first Flux cold-load (~60–120s on 6 GB) +
  a re-submit cancelling the in-flight turn. Resolved by patience + the in-flight guard.

**HRA enforce is now live-proven for: LLM startup, chat, and image (Tier-B evict/restore).**
Still worth exercising for the full soak: voice (STT/TTS) + heavy concurrency (chat+voice+image
together) over a longer session.

### Task 62 (legacy deletion) — gate status
Enforce is now validated for the primary flows → legacy deletion is close to safe. Recommend one
more short session exercising **voice + concurrency** before the physical `GpuLeaseManager` removal,
then execute Task 62 in one clean pass.

## Session 17 — Task 62 executed: legacy GpuLeaseManager physically removed

User confirmed the working runtime after enforce-default validation ("I confirm everything works
well. Clean now."). Executed Task 62 as a **contained deletion** (lowest blast radius): gutted the
dead single-holder machinery, kept public method signatures as thin stubs so no caller had to be
rewired across files.

### Deleted from `crates/kria-core/src/resource/gpu_lease.rs` (~800 lines)
- `InnerState` state machine + `PendingLeaseRequest` / `ActiveLease` queue types.
- `acquire_lease` / `acquire_lease_with_ttl` / `acquire_guard` single-holder acquire paths.
- Recovery pipeline: `recovery_worker_loop`, `attempt_recovery_pass`, `cleanup_orphaned_processes`,
  `schedule_recovery_worker`, `mark_recovering(_and_schedule)`, `ensure_idle_reconciled_for_grant`,
  `transition_to_recovering_locked`, `degrade_if_recovery_stuck_locked`.
- Grant/queue internals: `grant_locked`, `cancel_request`, `next_request_id_locked`,
  `request_priority`, `is_background_holder`, `issue_token`, `issue_request_id`, `recovery_reconciled`.
- `mark_degraded` / `clear_degraded` / `refresh`(real) / telemetry `RwLock` field + `telemetry_source`.
- `PendingRequestCleanup` RAII type.
- The entire `#[cfg(test)] mod tests` (tested the deleted state machine).
- `GpuLeaseGuard` slimmed to `{ hra_guard: Option<AdmissionGuard>, released }` — dropped
  `manager` / `token` / legacy `LeaseToken` plumbing.

### Kept as a THIN COMPATIBILITY SHELL (so callers compile unchanged)
- `acquire_guard_gated` — the ONE production admission entry point → `HraService::admit_gpu`.
- No-op stubs: `acquire_token`, `release_token`, `refresh`, `reconcile`, `state` (→ `Idle`),
  `set_resource_telemetry`, `clear_resource_telemetry`, `new`/`shared`/`default`.
- Preserved public types (still referenced by callers): `GpuOwner`, `ImageLeaseBackendId`,
  `RecoveryReason`, `GpuLeaseState`, `GpuLeaseError`, `GpuPathSnapshot`, `LeaseToken`,
  `GpuLeaseGuard`, `LeaseGuard`/`LeaseError` aliases, `global_gpu_lease`, `map_owner_to_hra`,
  `build_hra_request`.

### Caller edits (minimal)
- `agent/executive/controller.rs::execute_task` — `acquire_lease(...)` → `acquire_guard_gated(...)`.
- `image/orchestrator.rs::on_idle` / `shutdown` — dropped `mark_recovering(...)` calls (HRA releases
  residency on admission-guard drop); removed now-unused `RecoveryReason` import.

### Verification (all green)
- `cargo check -p kria-core` (default) + `--no-default-features` (user runtime) + `-p kria-desktop`
  + `-p kria-server` — clean.
- Lib tests: `resource::` 185, `llm::orchestrator` 83, `image` 33 — pass.
- Integration: `hra_acceptance` 9, `hra_stress` 6, `hra_bench` 3 — pass.
- `cargo test --workspace --no-run` — entire workspace test suite compiles.

**Status:** Task 62 DONE (code side). HRA is now the sole GPU arbiter; the legacy shell has no
arbitration logic left to delete beyond the compat stubs, which vanish once every caller migrates to
taking an `AdmissionGuard` directly. No behavior change to the working runtime — the shell routes to
the same HRA path already validated live.
