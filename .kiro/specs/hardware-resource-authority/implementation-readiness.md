# KRIA HRA — Implementation Readiness Review

> No code changed. Verifies every `solution-architecture.md` change is integrable into the current
> code without architectural regressions. Consolidates the requested deliverables
> (implementation-readiness, dependency-graph, checklist, migration-validation, rollback-validation).
> Architecture is frozen; this only finds integration risks + the safe order.

## Codebase facts established (file:line)

- **Three snapshot types** (the telemetry change must respect all):
  - `platform::vram::VramSnapshot { free_mb, total_mb, reserved_mb, vendor }` (`platform/vram.rs:18`),
    `Copy + Serialize`. Produced by `VramProfiler` impls (`vram.rs:122`): `NvmlProfiler`
    (`#[cfg(feature="nvidia")]`), `RocmProfiler` (`vram.rs:228`), `NullProfiler` (`vram.rs:300`,
    returns 0/0). Built by `build_profiler()` (`vram.rs:330`). Consumed by the TelemetryHub, image
    barrier, and `SharedResourceTelemetry`.
  - `resource::telemetry::VramSnapshot { total_mb, free_mb, used_mb }` (`telemetry.rs:6`). Feeds
    `ResourceSnapshot::reconcile` (`telemetry.rs:96`) — the lease degrade path.
  - `orchestrator::telemetry::TelemetrySnapshot { free_vram_mb, total_vram_mb, gpu_util_pct }`
    (`orchestrator/telemetry.rs:21`). Feeds LLM sizing/watchdog.
- **nvidia-smi sampler already exists — but only in the orchestrator layer:** `CliBlockingSampler`
  (`orchestrator/telemetry.rs:205`) + `CliTelemetry` (`:409`). `platform::vram` has **no** CLI
  profiler; `platform::detect::detect_gpu` (`detect.rs:256`) does shell nvidia-smi for *total* VRAM.
- **Reconcile degrade trigger:** `telemetry.rs:100` `available_vram_mb < 200 || is_near_full()`
  → `CriticalOomRisk`. With Null (free=0,total=0) this always fires.
- **swap_failed:** `KriaEvent::LlmSwapFailed` exists; forwarder `runtime.rs:~2083` maps
  started/completed/degradation/stream/vram and drops the rest via `Ok(_) => {}`; UI listens only for
  `swap_started`/`swap_completed` (`app.ts:4553/4560`).
- **Hub init order:** `TelemetryHub::new` (calls `build_profiler`) at `runtime.rs:636`, BEFORE the
  shared-lease telemetry wiring (~633) and BEFORE the orchestrator background block (~1880). Correct.

---

## Per-change integration analysis

### C1 — Telemetry provider: nvidia-smi fallback in `build_profiler` (solves B1/B12)
- **Affected:** `platform/vram.rs` only (new `CliVramProfiler` impl `VramProfiler`; insert into
  `build_profiler` chain NVML → ROCm → **CLI** → Null).
- **Dependencies:** none new. Reuse the nvidia-smi query already proven in `detect_gpu`
  (`detect.rs:256`) and `CliBlockingSampler` (orchestrator) as the parsing reference. **No
  cross-layer dependency** (do NOT import orchestrator into platform — duplicate the ~10-line query
  locally; this is acceptable, layer-correct duplication).
- **Async/sync:** `VramProfiler::snapshot` is `async`; the nvidia-smi subprocess MUST run inside
  `tokio::task::spawn_blocking` (like `CliTelemetry::snapshot` at `telemetry.rs:427`). Failure to do
  so would block a Tokio worker. **This is the one real boundary risk — flagged.**
- **Circular deps:** none.
- **Ownership/duplication:** introduces nvidia-smi sampling in two layers (orchestrator + platform).
  Intentional + isolated; note for future unification, not a blocker.
- **Migration/rollback:** changes the default path on ALL builds (hub/lease/HRA now see real VRAM).
  Rollback = env/feature to force `NullProfiler`. Validate readings match nvidia-smi before trust.
- **Verdict:** Can implement immediately; runtime validation required (compare to `nvidia-smi`).

### C2 — Reconcile treats `total_mb == 0` as Unknown (solves B2/B9; minimal)
- **Affected:** `resource/telemetry.rs` `reconcile` (`:96`) — add an early guard: `if self.vram.total_mb == 0 { return Healthy }` (Unknown ⇒ assume reconciled) BEFORE the `<200` check; and make
  `Degraded` auto-clear on next acquire when Healthy/Unknown (`gpu_lease.rs` acquire path).
- **Key insight:** `total_mb == 0` is a sound Unknown sentinel — a present GPU always reports
  total > 0. **No new enum threaded through the 3 snapshot types** → blast radius minimal.
- **Dependencies:** none. Independent of C1 (and a safety net even if C1 lands).
- **Ownership conflicts:** none. **Rollback:** trivial (revert the guard).
- **Verdict:** Can implement immediately. Pairs with C1 (C1 makes 0 rare; C2 makes 0 safe).

### C3 — Forward `LlmSwapFailed` + UI clear (solves B3)
- **Affected:** `runtime.rs` forwarder (add `LlmSwapFailed => emit "orchestrator:swap_failed"`);
  `app.ts` (add `swap_failed` + `error` listeners → `setIsSwapping(false)`; optional heartbeat
  timeout); `ChatView.tsx` unaffected (already `when={isSwapping()}`).
- **Dependencies/async:** none; pure event plumbing. **Circular:** none. **Rollback:** trivial.
- **Verdict:** Can implement immediately. No prerequisites.

### C4 — Closed-form GPU sizing vs measured free + CUDA reserve + cooldown (solves B4/B6/B7)
- **Affected:** `orchestrator/strategy.rs` (headroom→ngl calc; watchdog log shows it already uses
  `free_mb`, but with no CUDA reserve), `orchestrator/threshold.rs`, `server_manager.rs`
  (`clamp_against_failures` already exists; add cooldown timer + direct fit), `gpu_watchdog.rs`
  (scale-up gating). HRA admission (`service.rs advise_gpu_admission`) must read measured-fresh.
- **Dependencies:** **requires C1** (needs real measured free; on Null it must refuse to size).
- **Init/ownership:** contained in orchestrator + HRA verdict; no new module.
- **Rollback:** config flag for reserve size + cooldown duration; revert to current strategy.
- **Verdict:** Requires prerequisite (C1) + runtime validation; hardware validation to tune reserve.

### C5 — DeviceTable free refresh confidence (B6)
- **Affected:** `collector.rs HostSnapshot::apply_to` (don't overwrite known free with 0 when GPU
  list empty/Unknown). **Depends on C1** (once C1 lands, snapshots carry real GPUs).
- **Verdict:** Requires prerequisite (C1). Small, contained.

### C6 — Staged/parallel startup (B8)
- **Affected:** `runtime.rs` startup ordering (orchestrator start currently near the end, after the
  ~11s tool-index build). Move orchestrator spawn earlier / parallel; emit `core_llm_ready`
  independent of `tools_ready`.
- **Dependencies (init-order risk — HIGHEST):** `Orchestrator::start` needs `model_router`,
  `event_bus`, `handle`, config, paths — all constructed earlier in `run()`. Moving the spawn
  earlier requires those to be ready at the new point. **Must map the exact prerequisites before
  moving.** Tool-index → background requires the agent loop to tolerate a not-yet-ready index
  (degrade to smaller hot set).
- **Rollback:** keep current ordering behind a flag.
- **Verdict:** Requires prerequisite (dependency map) + runtime validation. Medium-high risk;
  schedule AFTER C1–C3.

### C7 — CPU stagger + whisper `-t` cap (700% spikes)
- **Affected:** `voice_runtime_helpers.rs` / STT config (thread cap); startup task scheduling.
- **Dependencies:** none hard. **Rollback:** config. **Verdict:** Can implement immediately
  (independent); runtime validation for CPU profile.

### C8 — Daemon supervisor "mark unavailable after N" (B10 churn) + env fixes
- **Affected:** orchestrator vision-sidecar restart loop (cap restarts → mark unavailable). The
  fastapi/venv + MCP token issues are **environment** (user-side), not code.
- **Verdict:** Code part = optional improvement; env part = user action (not implementation).

---

## Implementation dependency graph

```
C1 (telemetry CLI fallback) ──┬──> C4 (sizing vs measured free)
                              ├──> C5 (DeviceTable free confidence)
                              └──> (de-blinds HRA admission)
C2 (reconcile total==0 Unknown) ── independent (pairs with C1)   [Issue 3 closed by C1+C2]
C3 (swap_failed event+UI)        ── independent                  [Issue 2 closed]
C6 (staged startup)              ── needs dependency map; after C1–C3   [Issue 1]
C7 (CPU stagger + whisper -t)    ── independent
C8 (supervisor cap + env)        ── independent / user-side
```

## Safe implementation order + markers

1. **C1** telemetry CLI fallback — *Can implement immediately; runtime validation required.*
2. **C2** reconcile Unknown + degraded auto-clear — *Can implement immediately.* → Issue 3 resolved.
3. **C3** swap_failed forward + UI clear — *Can implement immediately.* → Issue 2 resolved.
4. **C7** whisper `-t` cap + startup stagger — *Can implement immediately; runtime validation.*
5. **C5** DeviceTable free confidence — *Requires C1.*
6. **C4** closed-form sizing + cooldown — *Requires C1; runtime + hardware validation.*
7. **C6** staged/parallel startup — *Requires dependency map; runtime validation.* → Issue 1.
8. **C8** supervisor cap — *Optional improvement.* (env fixes = user action.)

---

## Second pass — independent Principal Engineer trying to reject the plan

- **"C1 duplicates nvidia-smi sampling — tech debt."** Accepted but correct: platform must not depend
  on the orchestrator layer; ~10 lines duplicated is cheaper than a layering inversion. Future
  unification noted, not a blocker. **Not rejected.**
- **"C1 changes the default path globally — risky."** True. Mitigation: C1 only *adds* a fallback
  rung; NVML path (when feature on) is unchanged; the new rung activates exactly where today it is
  Null (the broken case). Net change is strictly an improvement on `--no-default-features`. Rollback
  via env. **Not rejected; runtime-validate readings.**
- **"C2 'assume reconciled on Unknown' can hide an orphan VRAM leak."** Real. Mitigation: C2 is a
  *safety net*; C1 makes Unknown rare; the reconciler's PID/journal path (existing) still reclaims
  known orphans. On a telemetry-less host, blocking all consumers (today's behavior) is strictly
  worse than assuming-released. **Accepted as the lesser evil; documented.**
- **"C3 UI timeout could clear during a long valid swap."** Mitigation: heartbeat resets the timeout;
  default timeout generous (≥90s). **Not rejected.**
- **"C4 reserve is hardware-specific — you'll mis-size."** True for a static reserve. Mitigation:
  start conservative (≈1 GB), calibrate from the first successful load, persist per-GPU. Until
  calibrated, conservative under-utilizes but never OOMs. **Accepted; hardware-validate.**
- **"C6 reordering startup will hit a missing dependency mid-move."** Highest risk. Mitigation:
  mandatory pre-step — produce the exact `Orchestrator::start` prerequisite list from `runtime.rs`
  and only move the spawn to a point where all are constructed; gate behind a flag. **Sequenced last;
  not attempted until the dependency map exists.**
- **"Three snapshot types will rot."** The plan deliberately does NOT unify them now (would be a
  large risky refactor); `total_mb==0` Unknown convention avoids touching them. Unification is a
  future optional item. **Not rejected.**

No remaining *blocking* implementation risk. The only items that can surprise mid-implementation are
C4 (reserve tuning — hardware) and C6 (startup reorder — needs the dependency map first); both are
sequenced accordingly.

---

## Migration validation (per change)

- **C1:** after landing, log MUST NOT say "0 free VRAM" when an NVIDIA GPU is present; hub/lease/HRA
  VRAM == nvidia-smi within tolerance. Diff against `orchestrator` telemetry for one session.
- **C2:** image gen 10× → no `GuardReleasedAwaitingTelemetry`; lease returns Idle; on a forced
  telemetry-less run, consumers still work (assume-reconciled path).
- **C3:** force a swap failure → overlay clears; `swap_failed` event observed; LLM recovers (CPU).
- **C4:** GPU swap picks a fitting ngl on first try (no 36→34→… ladder); no OOM; cooldown observed
  after a failure.
- **C6:** time-to-first-token < ~10s after window; chat works before tool-index ready; no missing-dep
  panic at startup.
- **C7:** whisper CPU bounded; no 700% startup spike.

## Rollback validation (per change)

- **C1:** env forces NullProfiler → reverts to pre-change telemetry; verify no panic, HRA shadow only.
- **C2:** revert guard → reconcile returns to prior behavior (verified by unit test on a 0/0 snapshot).
- **C3:** remove listener/forward → overlay behavior reverts (no new state introduced).
- **C4:** config sets reserve=0 + cooldown=0 → prior sizing behavior.
- **C6:** flag restores original startup ordering.
- **All:** each change is independently revertible; none shares mutable state with another except
  C4/C5 which both *read* C1's telemetry (read-only dependency, safe to revert in any order after C1).

---

## Final readiness verdict

Every frozen-architecture change maps to specific files with no circular dependencies and no
unresolved ownership conflicts. Two boundary risks are explicitly flagged and contained: the
nvidia-smi subprocess must run via `spawn_blocking` (C1), and the startup reorder (C6) must be
preceded by an `Orchestrator::start` dependency map. Issues 2 and 3 are closeable immediately
(C1+C2+C3) with trivial rollback; Issue 1 (C6) is sequenced last behind its prerequisite. **Cleared
for continuous implementation in the order above** — no architectural surprises expected mid-way.
