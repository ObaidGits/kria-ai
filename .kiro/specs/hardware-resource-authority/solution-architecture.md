# KRIA HRA & GPU Orchestrator — Production Solution Architecture

> Planning only. No code changed. Source of truth: `hardware-runtime-audit.md` (B1–B12) + current
> code. This consolidates the requested deliverables (architecture-review, production-hardening,
> migration-plan, implementation-order, risk-analysis) and the design addenda for
> requirements/design/tasks. Format per issue: Root cause → current design → alternatives →
> decision → self-critique.

## North-star principles (apply to all decisions)

1. **One source of truth for telemetry.** Exactly one provider; it may use a fallback chain but
   exposes one snapshot type. Never two samplers disagreeing.
2. **"Unknown" ≠ "Zero".** A telemetry source that cannot read VRAM must report `Unknown`, never
   `0 free`. Every consumer treats Unknown as "do nothing disruptive", not "emergency".
3. **HRA is the Resource Kernel, and it fails open.** Every GPU consumer asks HRA; if HRA cannot
   decide (Unknown/timeout) it returns a safe static plan — it never blocks or hangs a consumer.
4. **Foreground is sacred.** The active chat/voice turn is never interrupted by a non-emergency
   action. Disruptive moves happen only at turn boundaries or true emergencies.
5. **No storms.** Every disruptive transition has hysteresis (dwell) + cooldown + a recorded
   failure ceiling. A failed action must not be retried at the same size immediately.
6. **Always recover to a known-good state.** Any failed transition restores the last working
   residency (CPU is the universal floor).

---

## 1. GPU Telemetry (B1/B2/B12) — the keystone

**Root cause.** `build_profiler()` returns `NullProfiler` (0 VRAM) under `--no-default-features`;
the orchestrator's nvidia-smi actor works → two sources, one blind. `reconcile` treats 0 as
`CriticalOomRisk`.

**Current design.** Two independent stacks: `platform::vram::build_profiler` (NVML feature-gated,
no CLI fallback) feeding hub/lease; `orchestrator::telemetry` (NVML→**nvidia-smi CLI**→RAM) feeding
LLM sizing/watchdog.

**Alternatives.**
- **A. Hub consumes the orchestrator's `GpuTelemetry`.** Single source, but inverts layering
  (core `resource::` would depend on `llm::orchestrator::`). Rejected — bad module boundary.
- **B. Give the ONE low-level provider (`build_profiler`) a full fallback chain + a confidence
  field.** Provider order: NVML (feature) → CUDA runtime → **nvidia-smi CLI** → sysinfo (RAM only,
  VRAM = `Unknown`). Snapshot carries `VramReading { free: Option<u64>, total: Option<u64>,
  source, confidence }`. Everyone (hub, lease, orchestrator) uses this one provider. **Chosen.**
- **C. Event-driven NVML push.** Best latency, but NVML lacks a portable event API and needs the
  feature. Keep as a future optimization inside provider A.

**Decision (B).** One provider, fallback chain, **`Unknown` sentinel**. Sampling = hybrid: a single
collector publishes a cached snapshot on a `watch` channel (~1–2s) for dashboards/pressure, plus an
on-demand fresh read for admission/scale decisions. The orchestrator's nvidia-smi actor becomes a
*backend* of this provider, not a separate stack → collapses B12.

**Self-critique.** Threading `Option<u64>`/confidence through every call site is invasive (many
assume `u64`). Mitigation: keep `VramSnapshot` numeric for back-compat but add `confidence: Measured
| Estimated | Unknown`; reconcile/admission branch on confidence. Risk: nvidia-smi subprocess cost
on a 1–2s cadence → mitigate with the cached single collector (one subprocess, shared).

---

## 2. Lease recovery & degradation (B2/B9)

**Root cause.** Recovery samples telemetry; on Unknown(=0 today) → `CriticalOomRisk` →
never reconciles → `Degraded`; degraded is sticky.

**Decision.** Reconcile becomes confidence-aware:
- `Measured` free < hard floor → `CriticalOomRisk` (real).
- `Unknown` after a guard release → **assume reconciled** (the consumer finished; we cannot prove a
  leak, and blocking all consumers is worse). Log "telemetry unknown — assuming released".
- Degraded is **never** entered on Unknown; only on repeated *measured* low-VRAM with an owner
  mismatch. Degraded auto-clears on the next acquire when telemetry is Healthy/Unknown.

**Self-critique.** "Assume reconciled on Unknown" could mask a real orphan leak. Mitigation: pair
with the reconciler's process-PID check (when available) and a journal-backed owner list; on Unknown
+ a known orphan PID, reclaim that PID rather than blanket-degrade.

---

## 3. Swap-failure UX & state machine (B3)

**Root cause.** `LlmSwapFailed` not forwarded (`runtime.rs Ok(_)=>{}`); UI has no `swap_failed`
listener → `isSwapping` stuck.

**Decision.** Backend forwards `LlmSwapFailed` → `orchestrator:swap_failed`. UI swap state machine:
`idle → swapping → {completed | failed | timeout} → idle`. Add: (a) `swap_failed` listener clears
`isSwapping` + shows a transient, non-alarming note ("GPU optimization deferred — running on CPU");
(b) a UI-side safety timeout (e.g. 90s) that auto-clears `isSwapping` if no terminal event arrives
(belt-and-suspenders against any future missing event); (c) clear on `orchestrator:error`.

**Self-critique.** A fixed UI timeout could clear during a legitimately long swap. Mitigation: make
the backend emit periodic `swap_progress` heartbeats; the UI timeout resets on each heartbeat.

---

## 4. GPU sizing & scale-up (B4/B6/B7) — kill the swap storm

**Root cause.** Watchdog jumps `0→ngl_max` against **total** VRAM with no CUDA-runtime reserve; only
~4.4 GB was free; OOM/timeout; backoff steps by ~2 ngl → minutes of thrash. HRA used stale 6141.

**Decision — closed-form fit against live, measured free VRAM:**
- `usable = measured_free − cuda_runtime_reserve − safety_margin` (reserve configurable, default
  ~768–1024 MB; covers CUDA context + fragmentation).
- `target_ngl = floor(usable_for_layers / per_layer_vram)`, clamped to `[0, total_layers]`, minus KV
  + mmproj VRAM if GPU-resident. Pick the **fitting** ngl directly — no jump-to-max-then-backoff.
- **Refuse to size on Unknown** → keep current residency (no swap).
- **Hysteresis + cooldown:** require sustained measured headroom for `dwell` before scaling up;
  after a failed GPU spawn, **cooldown** (e.g. 5 min) before retrying GPU at all; never re-attempt
  ≥ the failed ceiling.
- DeviceTable free is refreshed from measured snapshots only; an empty-GPU snapshot (Unknown) does
  **not** overwrite a known value with 0.

**Self-critique.** A static CUDA reserve may be wrong across GPUs/drivers. Mitigation: calibrate the
reserve from the first successful GPU load (measure actual VRAM delta vs estimate) and persist it
per GPU in the journal. Also: closed-form estimate can still be optimistic for vision mmproj →
verify post-spawn measured VRAM and down-clamp on the next decision.

---

## 5. GPU Residency model

**States:** `Cold(disk) → Warm(RAM) → Hot(VRAM) → Cloud`, plus `Pinned` (priority overlay).
**Policy:**
- **LLM = Pinned/Foreground.** Once hot, stays hot; preempts background; never evicted by a lower
  class; only swaps at turn boundary or emergency.
- **Image/Vision = Background.** Co-resident if measured VRAM fits; else preempt LLM to *Warm*
  (Tier-B) only if user explicitly wants local image and accepts the swap, else **cloud/CPU**.
- **Voice STT/TTS = Realtime.** Fast-lane admission; CPU-capable; never blocked; wake stays live.
- **Embeddings = CPU** (no GPU lease) unless a GPU embedding model is configured.
**Transitions** are driven by HRA admission + the Pressure engine, **not** by an eager auto-scale
watchdog. The watchdog becomes a *pressure sensor + emergency actor*, not a growth optimizer.

**Self-critique.** Pinning the LLM hot can starve image on a 6 GB GPU (can't co-reside). That is the
honest hardware limit; the design surfaces it as an explicit choice (co-reside if it fits, else
cloud/CPU image, or user opts into a swap) rather than silent thrash.

---

## 6. Swapping policy

Predictive prewarm = **advisory only** (never evicts ≥ class). Reserve VRAM in DeviceTable *before*
the swap. Hysteresis (dwell before act) + cooldown (after fail). Foreground-protect (done). Emergency
path uses a streaming checkpoint so even an OOM emergency doesn't lose the answer. Image preempts only
Background; LLM Foreground pins. Cloud changes behavior: if cloud is healthy + allowed, prefer cloud
over a disruptive local swap for background work.

---

## 7. Image generation pipeline (Issue 3 end-state)

```
Image request → HRA.admit_gpu(Image, InteractiveBg, vram_est)
  ├─ measured fits alongside LLM → CoResident hot → generate → release (verified/assumed-on-unknown)
  ├─ doesn't fit, user-local → preempt LLM→Warm (Tier-B), generate, restore LLM
  └─ doesn't fit / Unknown / degraded-cloud-ok → cloud fallback (explicit notice)
```
The lease never degrades on Unknown (§2). Admission verdict uses measured-fresh VRAM (§1/§4).

---

## 8. LLM startup (B8) — staged, parallel

**Decision.** Spawn the LLM **immediately and in parallel** with tool-index + embedding warmup.
Emit a staged readiness contract: `core_llm_ready` (llama-server responds) is independent of
`tools_ready` (semantic index) / `voice_ready` / `mcp_ready`. The ~11s tool-index build moves
**off the LLM critical path** (lazy/background; chat works with a smaller hot set first, full index
hot-swaps in). Target: time-to-first-token < ~10s after window.

**Self-critique.** Parallel LLM spawn + tool-index embedding both hit CPU at once → startup spike
(see §9). Mitigation: priority-stagger — LLM spawn gets CPU priority; tool-index runs at lower
priority / capped threads; whisper warmup deferred until after core_llm_ready.

---

## 9. CPU usage (700% spikes)

**Causes (audit §6):** whisper STT thread fan-out (no `-t` cap), plus simultaneous startup work
(tool-index embeddings + app scan + whisper warmup + sidecar venv setup) all racing.
**Decision.** Cap whisper threads (`-t` = min(physical_cores/2, 4)); **stagger** startup (LLM →
then tools → then voice warmup); bound every poll loop (already ≤). Make heavy startup tasks
low-priority and serialized where they contend.

---

## 10. HRA role — Resource Kernel (fail-open Controller)

**Decision.** HRA = **Resource Kernel**: the single authority that owns admission, placement,
residency, preemption, journal, recovery for all hardware. In enforce mode no GPU consumer bypasses
it; in shadow legacy owns (rollback). **Fail-open:** on Unknown/timeout HRA returns a safe static
plan (current residency / CPU) so it never blocks. It owns GPU/VRAM fully; CPU/RAM it *accounts and
advises* (gating CPU is out of scope — OS owns scheduling); Cloud is a Device.

**Self-critique.** A kernel that fails open could mask its own outages. Mitigation: every fail-open
fallback is journaled + surfaced in diagnostics ("HRA degraded → static plan, reason X") so silent
degradation is visible.

---

## 11. Consumer ownership (end-state)

| Consumer | Class | Path | Notes |
|---|---|---|---|
| LLM | Foreground/Pinned | HRA admit (residency-tied) | preempts bg; never mid-stream |
| Image | Background | HRA admit → co-reside/preempt/cloud | |
| Vision | Background | HRA admit | sidecar |
| STT/TTS | Realtime | HRA fast-lane | CPU-capable, never blocked |
| Embeddings | CPU | none (or optional GPU lease) | |
| Cloud LLM | Device | DeviceTable pool | failover target |
| Wake | Realtime | always-on, tiny | never evicted |
| Daemons (llama-server/ComfyUI/whisper/vision) | — | supervised + registered residency | breaker + backoff |

---

## 12. Daemons & lifecycle

llama-server, ComfyUI, whisper, vision sidecar, future MCP = **supervised processes** registered as
HRA residencies with: spawn backoff, circuit breaker, health probe, parent-watchdog, and journaled
PIDs for orphan reclaim. The vision sidecar venv/fastapi failure (B10) is an **environment** fix
(install sidecar requirements) — the supervisor should mark it `unavailable` after N failures and
stop the restart churn instead of looping.

---

## 13. Resource scheduling (Planner/Scheduler/DeviceTable/CoResidency/Registry/Budget/Sim/Predict)

Keep the existing single-instance modules (they are well-factored and tested). Redesign only:
- **DeviceTable free refresh** must be confidence-aware (§4).
- **Admission** must read measured-fresh + refuse-on-Unknown (§1/§4).
- **Predictor/Forecast** stay advisory (no admission handle) — correct as-is.
No duplicate scheduler/planner introduced. CoResidency manager (built) is the residency authority.

---

## 14. Logging (production)

Structured, one line per decision, with: `turn_id` (correlation), `who` (consumer), `why`
(rationale code), `when`, `what` (action), `result`, `latency_ms`, `vram_before/after`, `failure`,
`recovery`. Extend the existing `[HRA][Consumer]` logs with latency + before/after VRAM. Journal is
the durable audit; logs are the live trace.

---

## 15. Failure recovery matrix

| Failure | Detection | Recovery |
|---|---|---|
| GPU OOM / spawn fail | spawn err / 60s timeout | record ceiling, recover to last-good (CPU), cooldown |
| CUDA error / driver reset | provider error | mark device unhealthy, fail to CPU/cloud, probe |
| Telemetry loss | provider Unknown | keep current residency, no disruptive swap |
| Sidecar crash | exit status | supervised restart w/ backoff+breaker; mark unavailable after N |
| Cloud outage | 429/5xx | breaker open, local-only, half-open probe to recover |
| LLM crash | health probe | respawn last-good config |
| Image/voice crash | job err | release lease, restart consumer |

---

## 16. Cloud

Cloud pools are **Devices** in the DeviceTable (already). Failover when local is
Unknown/degraded/insufficient **and** privacy allows (Strict never egresses). Switch is a planner
decision (cost/latency/privacy weights). Recover via half-open circuit breaker. Cloud is preferred
over a disruptive local swap for background work when healthy.

---

## 17. Scalability & future expansion

- **Device abstraction** already multi-device; add `DeviceKind::Npu`/`Accelerator` later without
  redesign. No-GPU → CPU/cloud (fail-open). Multi-GPU → DeviceTable handles N devices + per-device
  budget/preemption.
- **Capability vectors** per device (built) → tier-agnostic placement.
- **Future workloads** (video/audio gen, training, multiple models, background agents) = new
  `ConsumerId`s + co-residency budget entries; the admission/residency model already generalizes.
  No core redesign required.

---

## 18. Implementation order (when implementation resumes)

1. **B1 telemetry provider + `Unknown` confidence** (unblocks everything; fixes Issue 3 root).
2. **B2/B9 reconcile + degraded auto-clear** (Issue 3 fully).
3. **B3 swap-failed event + UI listener + timeout/heartbeat** (Issue 2).
4. **B4/B6/B7 closed-form sizing vs measured free + cooldown + DeviceTable confidence** (Issue 2/4).
5. **B8 staged/parallel startup + B (tool-index off critical path)** (Issue 1).
6. **B-CPU stagger + whisper `-t` cap** (700% spikes).
7. **B10/B11 env: sidecar venv + MCP config; supervisor "mark unavailable after N"**.
8. Re-verify: enforce soak, image/voice/chat, no degrade, no stuck overlay, TTFT < 10s.

---

## 19. Risk analysis

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| `Unknown` sentinel threading misses a call site | med | high | central confidence enum; default conservative; tests |
| CUDA reserve mis-tuned per GPU | med | med | calibrate from first successful load, persist per-GPU |
| Parallel startup CPU contention | med | med | priority-stagger LLM > tools > voice |
| "Assume reconciled on Unknown" hides orphan leak | low | med | pair with PID reconcile + journal owner list |
| UI swap timeout clears during long valid swap | low | low | heartbeat resets timeout |
| Cloud failover egresses Strict data | low | high | privacy gate enforced in planner (existing) |

---

## 20. Principal-reviewer self-critique (brutal pass)

- **Single biggest risk:** the whole design hinges on the telemetry provider being honest (Measured
  vs Unknown). If `nvidia-smi` is also absent, everything is Unknown → HRA fail-open keeps the LLM on
  whatever residency it has and forbids swaps. That is *correct* (safe) but means **no GPU accel on a
  truly telemetry-less box**. Acceptable: GPU accel requires *some* VRAM signal; document it.
- **CPU/RAM not gated by HRA:** deliberate (OS owns scheduling). The "700% CPU" is fixed by capping
  whisper threads + staggering, not by HRA. Don't over-scope HRA into CPU scheduling.
- **Co-residency on 6 GB is marginal:** the honest end-state is LLM-pinned + image-to-cloud/CPU on
  small GPUs, not forced local co-residency. The design makes this explicit, not silent.
- **Migration safety:** every change lands behind shadow/enforce + rollback; telemetry provider
  change is the one that affects the default path — it must be validated that Measured readings match
  nvidia-smi before trusting. No big-bang.

**Conclusion:** after iteration, the remaining weaknesses are inherent hardware limits (small VRAM,
telemetry-less hosts) surfaced explicitly, not architectural defects. The design is ready for phased
implementation, telemetry provider first.
