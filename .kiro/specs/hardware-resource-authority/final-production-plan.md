# KRIA Hardware & Resource Authority (HRA) — Final Production Planning Phase

> Planning document only. No code changes, no commits, no wiring are performed or proposed for
> execution here. This is the brutally-honest, code-grounded blueprint for taking the HRA from its
> current **shadow-mostly** state to a **production-owned** orchestrator.
>
> Grounding date: 2026-06-28. Every claim below was verified against live source, not the task
> tracker. Where the tracker and the code disagree, the **code wins** and is noted.

---

## 0. Ground Truth — what is actually live today

This section is the foundation for everything else. It corrects the optimistic "48/48 code-complete"
framing in `implementation-tracker.md`. Code-complete means the *modules exist and are unit-tested*.
It does **not** mean they drive the running product.

Verified facts (file:line grounded):

1. **HRA `request()` (Planner + Scheduler admission) is never called in production.**
   Every call to `LocalAuthority::request` / `HraService::request` lives in `tests/` or `#[cfg(test)]`
   modules (`crates/kria-core/tests/hra_acceptance.rs`, `resource/authority/ra.rs` tests,
   `resource/authority/service.rs` tests). No desktop/runtime code path admits a real workload
   through the authority. The HRA Planner and Scheduler are **shadow-only**.

2. **The only live HRA control hook is the GPU-admission veto in the legacy watchdog.**
   `runtime.rs` calls `set_global_hra(hra.clone())` and `hra.set_shadow_only(!enforce)`
   (`KRIA_HRA_ENFORCE`). The watchdog (`gpu_watchdog.rs::execute_swap_with_target`) consults
   `advise_gpu_admission`. In shadow it only logs; with enforce it can veto a scale-up. That is **one
   decision**, not full admission ownership.

3. **The LLM runs on its own private lease, not the shared arbiter.**
   `llm/orchestrator/mod.rs` constructs `GpuLeaseManager::default()`. Image, vision, and voice share
   `global_gpu_lease()` (`resource/gpu_lease.rs`). The LLM is deliberately *not* on the shared lease
   because the lease is **single-holder**: the LLM holds residency continuously, so sharing would
   deadlock image acquisition. This is the keystone architectural blocker.

4. **Telemetry is NOT unified — 4–5 independent VRAM samplers run concurrently:**
   - `llm/orchestrator/mod.rs:608` → `create_telemetry_actor` (orchestrator `TelemetryActor`).
   - `resource/shared_telemetry.rs` → `build_profiler()` (shared-lease recovery verification).
   - `runtime.rs:1923` → `build_profiler()` (HRA 15s snapshot loop).
   - `image/orchestrator.rs:445` → `build_profiler()` (image VRAM barrier).
   - `agent/loop_engine/mod.rs:4829` → `build_profiler()` (ad-hoc free-VRAM read).
   Task 3's "single TelemetryCollector" (`collector.rs`) exists as a **model**, but nothing in the
   live runtime feeds from one collector. Multiple NVML/sysinfo reads run in parallel.

5. **Idle auto-unload is hardcoded OFF.**
   `runtime.rs:1979` `let idle_release_temporarily_disabled = true;`. The real
   `orch.config.idle_release_enabled` guard is gated behind `&& !idle_release_temporarily_disabled`,
   so configured idle release never runs. A model loaded to VRAM stays resident until pressure/swap.

6. **Journal disk persistence is not wired.**
   `journal.rs` has `to_bytes`/`from_bytes` + corrupt-tail recovery, fully tested. No production code
   writes the journal to disk or replays it on boot. The Reconciler's crash-recovery story is
   therefore **in-memory only** in production.

7. **Real telemetry IS now on the shared lease (the one genuine cutover from Session 10).**
   `runtime.rs` calls `shared_gpu_lease.set_resource_telemetry(SharedResourceTelemetry)`. This fixed
   the `GuardReleasedAwaitingTelemetry` degrade. This is the one place HRA-adjacent plumbing genuinely
   replaced a no-op.

8. **The live daily-stability fixes are real and do not depend on HRA enforce:**
   OOM-aware backoff (`server_manager.rs`), foreground guard against mid-stream swaps
   (`gpu_watchdog.rs` + `StreamActivityGuard`), tool-call 400 retry-without-tools (`local.rs`),
   shared lease with telemetry-backed recovery. These are active in default shadow mode.

**Bottom line:** the legacy orchestrator is still the decision-maker. HRA observes, emits status, and
can veto exactly one GPU scale-up under an env flag. The "brain" is built and tested in isolation but
is not yet wired to the "hands" for anything except a single veto.

---

## 1. Final Gap Report

Severity = impact on a production-owned orchestrator, not on daily chat usability (chat is already
stable). Each gap cites the code reality from §0.

### CRITICAL (blocks "HRA owns the GPU" claim)

- **C1 — Admission is not gated by HRA (`request()` never called in prod).** §0.1. Without this the
  Planner/Scheduler/Simulator/budget bands are dead weight in production. This is *the* gap; most
  other items are subordinate to it.
- **C2 — Single-holder lease prevents LLM+image co-residency.** §0.3. The lease model cannot express
  "LLM resident AND image resident under a shared budget." Until the co-residency budget+preemption
  model lands, the LLM cannot join the shared arbiter without deadlock. This is **real architectural
  work**, not a wire-up, and it gates C1 for the LLM consumer.
- **C3 — Telemetry not unified (4–5 concurrent samplers).** §0.4. Multiple sources of truth → HRA can
  decide on a different VRAM reading than the watchdog acts on. Correctness hazard for any enforce
  decision; also wasted NVML init/poll cost.

### HIGH (correctness / stability under enforce)

- **H1 — HRA verdict staleness (≤15s snapshot loop).** The 15s loop (`runtime.rs:1923`) is the only
  thing feeding the DeviceTable in prod. A veto decision can be made on data up to 15s old. For tight
  admission/scale-up timing this is too stale; needs a fresh read at decision time.
- **H2 — Emergency swap still cancels the active stream.** Foreground guard defers *non-emergency*
  swaps, but a true OOM emergency proceeds and interrupts the stream (no streaming-checkpoint/resume).
  Task 25's "emergency checkpoint+resume" is modeled but not wired into the live emergency path.
- **H3 — Idle auto-unload disabled (hardcoded flag).** §0.5. On a constrained GPU, a resident idle LLM
  blocks image/voice longer than necessary, increasing swap frequency and "Optimizing GPU layers"
  episodes. Loose end with real UX cost.
- **H4 — Journal not persisted → no true crash recovery in prod.** §0.6. After a hard crash, orphan
  llama-server/ComfyUI processes are reclaimed only by the legacy logic, not HRA journal replay.

### MEDIUM (ownership completeness / observability)

- **M1 — Reconciler live PID reclaim not wired.** Kill-scope + capability gate exist (`reconciler.rs`,
  `security.rs`) but no production path invokes them against real orphan PIDs.
- **M2 — Cloud device live failover not wired.** `cloud_health.rs` breaker model exists; provider
  adapters do not feed real 429/5xx error rates into it.
- **M3 — Metrics/SLO export not wired to observability.** `metrics.rs`/`sla.rs` compute values; no
  scrape/export endpoint surfaces them.
- **M4 — Frontend: 5 of 6 dashboard views are placeholders.** Only the Overview/Resource Dashboard is
  live; Explainability/Session/Forecasting/Recovery/Diagnostics show "awaiting data (shadow mode)".

### LOW (scale / polish)

- **L1 — Multi-GPU validated only headless** (mock DeviceTable). No real two-GPU silicon soak.
- **L2 — No 24h enforce-mode soak** on target hardware.
- **L3 — Diagnostics bundle export is client-side only** (no backend bundle endpoint).
- **L4 — Predictive engines (WPE/SIP/RFE/TPPE/AOL) advisory-only and not consumed in prod** — fine by
  design (advisory), but their output influences nothing live yet.

### NOT AN HRA GAP (explicitly scoped out)

- **Voice 800% CPU spike** = Whisper STT thread fan-out (CPU inference), not the GPU/resource
  orchestrator. Fix belongs in the STT thread cap (`-t`/worker count), tracked separately.
- **Image-gen slowness** = inherent Tier-B drop-swap cost on a ~6GB GPU (two llama-server restarts +
  ComfyUI cold start). Not a bug; only co-residency (C2) or more VRAM removes it.

---

## 2. Final Implementation Roadmap

Ordered by dependency → risk → impact. Phases are independently shippable and reversible. Nothing
here is executed in this planning phase.

### Phase A — Correctness foundation (must precede any enforce trust)

1. **A1 · Unify telemetry (C3).** Make the HRA `TelemetryCollector` the single sampler; have the
   watchdog, shared lease, image barrier, and agent free-VRAM reads consume the published
   `HostSnapshot` instead of each calling `build_profiler()`. *Risk: medium (touches hot paths).
   Reversible: keep old reads behind a feature flag during bake.*
2. **A2 · Fresh-read at decision time (H1).** Add a synchronous fresh-snapshot read in
   `advise_gpu_admission` (and any future `request()` gate) so verdicts never use >N-ms-old data;
   keep the 15s loop only for the dashboard. *Risk: low.*
3. **A3 · Re-enable idle auto-unload (H3).** Remove `idle_release_temporarily_disabled`; restore the
   `orch.config.idle_release_enabled` guard; validate it cooperates with the foreground guard (never
   unloads mid-turn). *Risk: low–medium; needs a short live soak.*

### Phase B — Co-residency model (the keystone, C2)

4. **B1 · Co-residency budget + preemption lease.** Replace single-holder semantics with a per-device
   budget where multiple residents coexist under Soft/Hard/Emergency bands (`budget.rs` already models
   the bands). Define preemption: image admission preempts LLM *residency* (evict-to-RAM) rather than
   blocking. *Risk: HIGH — this is the real architectural build. Land behind shadow comparator first.*
5. **B2 · LLM joins the shared arbiter.** Once B1 exists, route the LLM's residency through
   `global_gpu_lease()`/RA instead of `GpuLeaseManager::default()`. *Depends on B1. Risk: high; gate
   with bypass switch + soak.*

### Phase C — Admission ownership (C1)

6. **C1-impl · Gate consumers through `request()`.** LLM, image, voice, embeddings call
   `HraService::request` before acquiring residency; legacy lease becomes the *executor* under RA
   plans. Flip one consumer at a time behind `KRIA_HRA_ENFORCE` + per-consumer bypass. *Depends on
   B1/B2 for LLM; image/voice/embeddings can flip earlier since they already share the lease.*
7. **C2-impl · Emergency streaming checkpoint (H2).** Wire Task 25's checkpoint+resume into the live
   emergency OOM path so even emergencies don't lose the answer. *Risk: medium.*

### Phase D — Durability & reclaim

8. **D1 · Journal disk persistence + boot replay (H4).** Wire `to_bytes`/`from_bytes` to an fsync'd
   file; replay on boot into the Reconciler. *Risk: low–medium.*
9. **D2 · Live PID reclaim (M1).** On boot, diff journal vs real GPU PIDs; reclaim orphans through the
   capability-gated, safety-policy-audited kill path. **Destructive — must log before/after and route
   through the safety policy layer.** *Risk: medium; requires careful guarding against false kills.*

### Phase E — Scale, cloud, observability, UI

10. **E1 · Cloud failover wiring (M2).** Feed provider 429/5xx into `cloud_health.rs` breakers; Planner
    avoids open pools.
11. **E2 · Metrics/SLO export (M3).** Surface `metrics.rs`/`sla.rs` via `infra/observability.rs`.
12. **E3 · Remaining 5 dashboard views (M4).** Stream real data into Explainability/Session/
    Forecasting/Recovery/Diagnostics.
13. **E4 · Backend diagnostics bundle (L3).**

### Phase F — Validation & sign-off

14. **F1 · Multi-GPU live soak (L1).** 15. **F2 · 24h enforce-mode soak (L2).** 16. **F3 · Production
    Readiness Review** against acceptance matrix A1–A27.

**Critical path:** A1 → A2 → B1 → B2 → C1-impl → F2 → F3. Phases A, D, E can proceed in parallel with
B/C where they don't touch the lease model.

---

## 3. Dead Code Report

Code that exists but is unreferenced by any production path. Removing/wiring is deferred (planning
only). "Dead in prod" ≠ untested — most have unit tests; they are simply not consumed live.

| Item | Location | Status | Disposition |
|---|---|---|---|
| `LocalAuthority::request` / Planner / Scheduler admission | `resource/authority/ra.rs`, `planner.rs`, `scheduler.rs` | Tested, **never called in prod** | Wire via Phase C, do NOT delete |
| `collector.rs` `TelemetryCollector` / `HostSnapshot` | `resource/authority/collector.rs` | Tested, not fed live | Wire via Phase A1 |
| `simulator.rs` pre-commit gate | `resource/authority/simulator.rs` | Tested, not in admission path | Wire with Phase C (depends on `request()`) |
| `budget.rs` Soft/Hard/Emergency bands | `resource/authority/budget.rs` | Tested, not consumed | Wire via Phase B1 |
| `journal.rs` `to_bytes`/`from_bytes` | `resource/authority/journal.rs` | Tested, no disk IO | Wire via Phase D1 |
| `reconciler.rs` live reclaim + `security.rs` kill-scope | `resource/authority/{reconciler,security}.rs` | Tested, no live PID path | Wire via Phase D2 |
| `cloud_health.rs` breakers | `resource/authority/cloud_health.rs` | Tested, no provider feed | Wire via Phase E1 |
| Predictive engines WPE/SIP/RFE/TPPE/AOL | `resource/authority/{predict,session,thermal}.rs` | Tested, advisory-only, not consumed | Optional Phase E; advisory by design |
| `daemon_supervisor.rs` | `resource/authority/daemon_supervisor.rs` | Tested, not supervising live daemons | Wire via Task 19 (deferred) |
| Genuinely removable (already done) | `create_cuda_telemetry`, `AudioFreezeGuard` v1, `vision_automation.rs` stub lease | Deleted in Sessions 6/8 | None |

**No code should be deleted in this phase.** The only true delete candidates after cutover are the
legacy `GpuLeaseManager::default()` for the LLM (after B2) and redundant `build_profiler()` call sites
(after A1). Everything else is "not yet wired," not "dead."

---

## 4. Integration Report — runtime paths needing HRA ownership

| Runtime path | File | Today | Needs |
|---|---|---|---|
| LLM residency/admission | `llm/orchestrator/mod.rs` (`GpuLeaseManager::default()`), `local.rs` | Private lease + watchdog; HRA veto only | B1 co-residency → B2 shared lease → C1 `request()` gate |
| GPU scale-up decision | `gpu_watchdog.rs::execute_swap_with_target` | HRA veto (shadow/enforce) + foreground guard | A2 fresh-read; C2 emergency checkpoint |
| Image generation | `image/orchestrator.rs`, `tools/image_generation.rs` | Shared lease (no `request()`) | C1 `request()` gate (can flip early) |
| Voice STT/TTS | `commands/voice_runtime_helpers.rs`, `voice/{stt,tts}.rs` | Shared lease (no `request()`) | C1 gate; STT thread cap (separate, non-HRA) |
| Embeddings | `routing/embed.rs` (`EmbedPool`) | Sharded pool, no RA admission | C1 gate (optional; low contention) |
| Telemetry feed | `runtime.rs:1923`, `shared_telemetry.rs`, `image/orchestrator.rs:445`, `agent/loop_engine:4829`, `orchestrator/mod.rs:608` | 4–5 independent samplers | A1 unify to one `HostSnapshot` |
| Idle unload | `runtime.rs:1979` | Hardcoded disabled | A3 re-enable |
| Crash recovery | Reconciler (in-memory) | No journal replay | D1 disk persist + D2 reclaim |
| Status → UI | `runtime.rs` → `resource:hra_status` → `app.ts` → `ResourceDashboard.tsx` | Live (Overview only) | E3 remaining 5 views |

---

## 5. Production Readiness Report

**Daily usability:** READY. Chat, voice, and image work; the crash/OOM-loop/mid-answer-interrupt bugs
are fixed and live in default mode. For a single-GPU end user, the system is stable today.

**HRA as a production-owned orchestrator:** NOT READY. The authority does not own admission (C1); the
co-residency model that would let it own the LLM does not exist (C2); telemetry is fragmented (C3).
Enforce mode (`KRIA_HRA_ENFORCE=1`) currently buys exactly one veto on GPU scale-up — valuable for OOM
avoidance, but not "ownership."

**Acceptance matrix (A1–A27):** the pure control-plane criteria pass in unit/headless tests. The
*integration* criteria (A4 foreground non-interrupt under emergency, A8 multi-GPU live, A21 residency
single-executor in prod, A22 simulator pre-commit in prod) are **not** met live because the consuming
paths don't call the authority.

**Risk if shipped as-is claiming "HRA owns GPU":** misleading. Honest framing: "Legacy orchestrator
with HRA advisory + one enforce-able GPU veto, plus real OOM/foreground/lease stability fixes."

---

## 6. Frontend Update Plan

Current: `ResourceDashboard.tsx` (Settings → Hardware) renders the Overview from `resource:hra_status`.
Five views are placeholders with an honest "awaiting data (shadow mode)" state.

Plan (Phase E3, no code now):
1. **Explainability view** — render journal rationale codes + last N decisions (needs `resource:*`
   correlation events streamed, currently emitted to journal only).
2. **Session view** — SIP/SessionOwnership state (needs SIP output bridged).
3. **Forecasting view** — RFE EWMA/slope + lead-time (needs RFE output bridged).
4. **Recovery view** — reconciler/journal-replay events + daemon supervisor state (needs D1 + Task 19).
5. **Diagnostics export** — move from client-side to backend bundle (L3/E4).
Keep all events additive (contract rule N5); no rename of existing `resource:hra_status`.

---

## 7. Runtime Validation Plan (manual checklist)

Run by the user on target hardware; agent cannot read live logs.

**Shadow baseline (`cargo tauri dev`):**
- [ ] Use chat / voice / image ~20 min. Filter logs to `hra`, `hardware`, `watchdog`.
- [ ] Confirm no `swap spawn failed` loop; HRA shadow verdicts logged next to legacy action.
- [ ] Confirm shadow verdicts *match* what the legacy strategy actually did (placement sanity).
- [ ] Image gen completes; no `GuardReleasedAwaitingTelemetry` degrade; input stays usable during
  "Optimizing GPU layers" notice.

**Enforce bake (`KRIA_HRA_ENFORCE=1 cargo tauri dev`) — only after shadow looks sane:**
- [ ] Trigger a near-OOM scale-up; confirm `HRA VETO` line appears and model stays on a fitting size
  (no OOM restart).
- [ ] Chat soak ~30 min; confirm no regression vs shadow (no mid-answer restart, no stuck swap).
- [ ] Voice utterance during a swap; confirm wake/voice survives.

**Per-phase gates (future, post-implementation):**
- [ ] A1 unify: only one `build_profiler`/collector active (grep + runtime log).
- [ ] B1/B2: LLM + image resident concurrently under budget without deadlock.
- [ ] D1: kill app mid-load → reboot → journal replay reclaims orphan PIDs, zero leaks.
- [ ] F2: 24h enforce soak, no degrade/OOM-loop; F1: two-GPU placement.

---

## 8. Final Verdict

**Ready with Major Planning Gaps Remaining for full HRA ownership; Ready for daily single-GPU use.**

Two honest layers:

- **As a stability fix set:** DONE and live. The things that actually broke for the user (chat crash,
  OOM ping-pong, mid-answer "Optimizing GPU layers" interruption, image lease degrade) are fixed in
  default mode without enforce. Ship and use it.

- **As "HRA owns the hardware":** NOT done, and the remaining work is **real engineering, not a flip.**
  The blocker is architectural: the single-holder lease (C2) must become a co-residency
  budget+preemption model before the LLM can join the shared arbiter and before `request()` admission
  (C1) can own placement. Telemetry must be unified first (C3) so decisions and actions read the same
  truth. These are Phase A/B/C of the roadmap — weeks of careful, soak-gated work, not a switch.

Recommendation: proceed in roadmap order (A → B → C), each behind shadow comparator + bypass + soak.
Do **not** advertise full HRA ownership until C1 is live and F2 (24h enforce soak) passes. Until then
the accurate description is: *legacy orchestrator with HRA advisory, one enforce-able GPU veto, and a
fully-built control plane waiting to be wired in.*
