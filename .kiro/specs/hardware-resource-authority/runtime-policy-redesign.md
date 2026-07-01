# KRIA HRA — Runtime Resource Policy Redesign (Planning Only)

> No code changed. Final planning phase. Consolidates the requested deliverables
> (runtime-policy-redesign, residency-policy, gpu-sizing-design, runtime-state-machine,
> migration-policy, watchdog-redesign, startup-policy, ux-policy, architecture-review,
> implementation-order, risk-analysis) into one internally-consistent document.

## 0. The governing law (overrides every optimization)

> **Never interrupt an active user for a performance optimization. A process restart (the only way
> to change `n_gpu_layers` in llama.cpp) is a disruption and is FORBIDDEN for performance reasons.
> Restarts are permitted only for: (a) correctness/safety (OOM, driver reset, telemetry-proven
> unsafe state), or (b) an explicit user workflow (image generation, model/settings change).**

Hard constraint discovered in code + llama.cpp: **`n_gpu_layers` is a launch-time parameter.** Any
ngl resize = a full llama-server restart = the "Optimizing GPU layers" flash. Therefore the entire
policy is built around *minimizing restarts*, not making resizes seamless (impossible for ngl).

Disruption classes (every action is tagged):
- **None** — pure observation / cache reuse. Always allowed.
- **Background** — cloud calls, warm-in-RAM, prewarm into free headroom (no eviction). Allowed when idle.
- **Restart** — kill+respawn llama-server (ngl/ctx change, Tier-B image swap). FORBIDDEN unless law (a)/(b).

---

## G1 — Runtime GPU Sizing (measured-first, calibration-only-as-refinement)

**Root cause of current pain.** Sizing trusted an estimate against fluctuating *total/total-free*
VRAM; on a desktop, Chrome/Discord/OBS/Wayland/games change free VRAM constantly, so any reactive
resize thrashes.

**Decision — a deterministic budget over MEASURED live free VRAM, with a volatility margin:**
```
admissible_ngl_vram =
    measured_free_vram                       (C1 — never Unknown-as-0)
  − driver_runtime_overhead (measured/known)
  − cuda_runtime_overhead   (measured/calibrated refinement, bounded)
  − model_weight_vram(ngl)
  − kv_cache_vram(ctx)
  − mmproj_vram (if GPU-resident)
  − safety_margin
  − volatility_reserve      (NEW: headroom for other apps reclaiming VRAM)
```
- **Measured free is primary.** Calibration is a *bounded correction term* on `cuda_runtime_overhead`
  only (clamped to e.g. ±50% of default), learned from the first successful load, persisted per-GPU.
  It can never become the primary signal and can never push sizing into an unsafe region.
- **Volatility reserve** is the new idea that kills desktop thrash: size for the *sustained* free
  VRAM (a low-percentile of recent readings), not the instantaneous peak. If free VRAM spikes because
  Chrome closed, we do NOT immediately upsize — because Chrome will reopen. Size for the floor, stay.
- **Sizing runs at load time only** (startup, post-image-restore, recovery). It is NOT a steady-state
  loop. Steady state = the residency lock (G3).

**Alternatives weighed:** (A) calibration-primary — rejected (fragile, ignores other apps).
(B) fixed conservative ngl per tier — rejected (wastes big GPUs). (C) measured-first + volatility
reserve + bounded calibration — **chosen** (accurate, desktop-stable, scales low→high end).

**Self-critique:** volatility reserve under-utilizes when the box is dedicated (no other GPU apps).
Mitigation: the reserve adapts to observed volatility — near-zero on a stable/dedicated GPU, larger
on a churny desktop. Measured from telemetry variance, not hardcoded.

---

## G2 — Residency Policy Engine (NEW; watchdog demoted to executor)

**Current:** the GPU watchdog owns BOTH the decision ("should I resize?") and execution. That coupling
is why it optimizes opportunistically.

**Decision — split policy from execution:**
```
TelemetryHub ─┐
Forecast ─────┤
SessionIntent ┤→ Runtime Policy Engine ──Decision{action,rationale,benefit,cost,risk}──> Executor (watchdog)
UserActivity ─┤        (decides)                                                              (does I/O only)
ResidencyLock ┤
BenefitEval ──┘
```
- **Policy Engine** (new, pure/deterministic) owns the decision. Inputs: residency lock state,
  measured telemetry + confidence, forecast (RFE), session intent (SIP), user activity (G6), benefit
  eval (G5), cooldown, mode (G7). Output: one of `Stay | Optimize | Migrate | Defer | Recover |
  Cloud | Reject` + full rationale (G11).
- **Watchdog → Executor.** It no longer decides to optimize. It (1) detects emergencies fast (OOM/
  pressure — a local safety reflex that bypasses the policy engine for correctness), and (2) executes
  decisions handed to it, re-validating preconditions under lock + epoch at execution time.
- **Default policy output in Interactive mode = `Stay`** for performance deltas. `Optimize`(restart)
  is only emitted in Maintenance/DeepIdle and only when BenefitEval says worth-it AND the lock permits.

**Interaction with existing modules (reused, not rebuilt):** Policy Engine reads the single
`TelemetryHub`, the `Forecaster`/RFE, `SessionProfile`/SIP, `simulator::simulate` (for feasibility),
`budget` bands; emits to the watchdog executor + `ResidencyManager`. Planner/Scheduler/DeviceTable
unchanged (used by enforce-mode admission).

**Self-critique:** a separate policy layer adds latency to a decision. Mitigation: decisions are
cheap (pure functions over a snapshot) and run off the hot path; the emergency reflex stays in the
watchdog for sub-second OOM response.

---

## G3 — Resident Lock state machine (the UX keystone)

**States:** `Cold → Loading → Resident → ResidentLocked`, plus overlays/branches `PinnedResident`,
`Recovering`, `Emergency`, `Migrating`, `ImageOverride`, `CloudFallback`. (Added vs the ask:
`Loading` distinct from `Migrating`; `Stabilizing` micro-state between Resident and ResidentLocked.)

```
Cold ──load──> Loading ──ok──> Resident ──stabilize(Ns)──> ResidentLocked
                  │ fail                                        │
                  └──> CloudFallback / Cold(CPU)                │
ResidentLocked ──(break condition)──> {Migrating | Recovering | Emergency | ImageOverride}
                                          └──> Loading ──> Resident ──> ResidentLocked
```
- **`ResidentLocked`** = NO resize, NO optimization, NO migration, NO experimentation, NO automatic
  restart. This is the steady state for ~all of a session. It is what permanently kills the
  between-session flapping.
- **Break conditions (only):** explicit user image generation; GPU OOM; driver reset; hardware
  failure; model change; settings change; app restart; explicit maintenance. (Added: **sustained
  correctness-threatening pressure** — measured free below emergency band for a dwell — and **cloud
  health change** when currently on CloudFallback.)
- After any break+reload, it returns to `ResidentLocked`. Optimization is a *transition event*, never
  a steady-state loop.

**Self-critique:** if the model loads on CPU at startup, `ResidentLocked` on CPU means it never gets
GPU. Mitigation: a **single** post-startup "promotion" opportunity (G4) while DeepIdle, simulator-
gated, before locking — then lock. One safe upgrade, then stable forever.

---

## G4 — Runtime Optimization Policy (state-driven, not counter-driven)

Reject "once per session" (counter). **State-driven:** an optimization (restart) is only *eligible*
when ALL hold:
- residency state ∈ {Resident (pre-lock), or CloudFallback/CPU wanting promotion},
- mode == Maintenance (DeepIdle, see G7) — never Interactive,
- user activity == DeepIdle (G6),
- telemetry confidence == Measured (not Unknown),
- forecast: free VRAM sustainably sufficient (RFE, low volatility),
- cooldown elapsed + not within a failure-ceiling,
- `simulator::simulate(new_size)` predicts fit without hard-band breach,
- BenefitEval == Worth-It (G5).
If any fail → `Stay`/`Defer`. Result: optimization happens at most at well-defined idle moments, only
when provably safe and worthwhile — typically **zero or one** restarts per session, never mid-work.

---

## G5 — Benefit Evaluation Engine (Worth-It / Not-Worth-It)

```
expected_speedup   = est_tok_per_sec(target) / est_tok_per_sec(current)      # e.g. 2.5x CPU→GPU
restart_cost_s     = measured/estimated reload time (model+warmup)
interruption_risk  = f(user_activity)            # DeepIdle≈0, Active=∞ (forbidden)
failure_prob       = f(simulator margin, history) # tight fit → higher
WORTH_IT  ⇔  expected_speedup ≥ SPEEDUP_MIN (e.g. 1.3)
          AND interruption_risk == 0 (DeepIdle only)
          AND failure_prob ≤ FAIL_MAX (e.g. 0.10)
          AND restart_cost_s ≤ COST_MAX (or hidden by idle)
```
For a session already GPU-resident at a good size, `expected_speedup ≈ 1.0` → Not-Worth-It → no
restart. CPU→GPU promotion while DeepIdle with a safe margin → Worth-It (one time). Tunable constants.

**Self-critique:** est_tok_per_sec needs a model. Use a coarse per-tier table refined by observed
throughput; conservative defaults. Worst case → Not-Worth-It (bias toward Stay = bias toward no
disruption, which is the governing law).

---

## G6 — User Activity Model

States: `Active` (typing, streaming a response, voice turn, tool running) → `Idle` (no activity <T1)
→ `DeepIdle` (no activity ≥T2, no queued work, no foreground focus on KRIA). Inputs: turn in flight
(`server.has_active_streams`), voice active, recent input timestamp, foreground window, queued prompts.
- **Active** → all Restart-class actions FORBIDDEN (only emergency correctness may override, with
  streaming checkpoint).
- **Idle** → Background-class allowed (cloud, warm-in-RAM); no restarts.
- **DeepIdle** → the only window where a performance promotion (G4) may occur.
Activity gates: optimization, migration, cleanup, preload, image admission timing, recovery
scheduling.

---

## G7 — Runtime Modes

| Mode | Allowed | Forbidden | GPU policy |
|---|---|---|---|
| **Interactive** (default) | None/Background | **all Restart-for-perf** | hold ResidentLocked |
| **Maintenance** (DeepIdle) | one promotion, cleanup, prewarm | mid-work restart | safe upsize if Worth-It |
| **Recovery** | reload last-good, reclaim orphans | new perf opt | restore, don't optimize |
| **Emergency** (OOM/driver) | evict, checkpoint, downsize | upsize | shrink to safe immediately |
| **Background** (no UI focus) | cloud, batch | foreground preempt | low priority |
| **Idle** | Background | restart-for-perf | hold |
| **Cloud** | cloud routing | local restart loops | prefer cloud |
| **Hybrid** | split local+cloud | — | co-reside by budget |

Mode is derived from activity + telemetry + health; transitions are logged.

---

## G8 — Startup Policy

**Critical path = LLM only.** Everything else is background/lazy.
```
process start
  ├─(parallel)─> detect backend + measured VRAM (C1) → size (G1) → spawn llama-server ──> core_llm_ready  ★critical
  ├─(background)─> tool semantic index (C6, lazy; lexical until ready)
  ├─(background)─> voice warmup (deferred until after core_llm_ready)
  ├─(background)─> MCP providers, agents, perception
  └─(lazy)─────> image/ComfyUI (only on first image request)
```
Emit staged readiness: `core_llm_ready` independent of `tools_ready`/`voice_ready`/`mcp_ready`. Target
TTFT < ~10s. Heavy CPU tasks staggered + thread-capped (C7). After core_llm_ready + stabilize → lock.

---

## G9 — Image Generation Policy (the one legitimate restart workflow)

```
image request → measured snapshot + simulator
  ├─ fits co-resident with LLM (simulator pass)        → CoResident (no restart)        UX: "Preparing image…"
  ├─ doesn't fit, user wants local, user Idle/DeepIdle  → Tier-B: LLM→Warm (simulator-gated restart)
  │                                                        UX: "Preparing image (freeing GPU)…" → gen → "Restoring chat…"
  ├─ doesn't fit / Active / privacy-ok cloud            → CloudFallback                  UX: "Using cloud for image…"
  └─ none viable                                        → Reject with reason             UX: explicit message
restore: bring LLM back to its EXACT pre-image ResidentLocked config (no re-sizing decision).
```
This is the only routine restart, it is user-initiated (law (b)), simulator-gated (never fails), and
fully narrated. Restoration is deterministic (reuse the locked config), not a fresh sizing.

---

## G10 — UX Policy (precise, state-mapped)

| State / action | Message |
|---|---|
| Cold/Loading (startup) | "Loading model…" |
| Stabilizing | "Finishing model setup…" |
| ResidentLocked | (no banner — silent, stable) |
| Image co-resident | "Preparing image…" |
| Image Tier-B evict | "Freeing GPU for image…" |
| Image restore | "Restoring chat…" |
| CloudFallback | "Using cloud…" |
| CPU fallback | "Running on CPU (GPU busy)…" |
| Recovering | "Recovering GPU…" |
| Emergency downsize | "Reducing GPU use to stay stable…" |
**Banned:** generic "Optimizing GPU layers" with no specifics. Every banner names the exact action +
clears on terminal event (C3). No banner ever appears for a performance decision in Interactive mode
(there are none).

---

## G11 — Logging (decision-grade)

Every policy decision logs: `correlation_id` (turn/session), `who` (consumer), `why` (rationale code),
`when`, `current_state`, `target_state`, `expected_benefit` (speedup), `expected_cost` (restart_s),
`expected_risk` (failure_prob), `result`, `latency_ms`, `recovery`. Journaled for audit; every
migration is fully explainable post-hoc.

---

## G12 — Cascading architecture review

- **NEW:** Runtime Policy Engine (`resource/authority/policy.rs` — pure decision fn), Residency Lock
  state in `ResidencyManager`, User Activity Model (`resource/authority/activity.rs`), Benefit Eval
  (`resource/authority/benefit.rs`), Runtime Mode derivation.
- **CHANGED:** Watchdog → executor (remove its opportunistic decision; keep emergency reflex + I/O).
  Sizing (G1) gains volatility reserve + bounded calibration. UX/logging extended.
- **REUSED unchanged:** Planner, Scheduler, DeviceTable, CoResidencyManager, Simulator, Forecaster,
  SessionProfile, Journal, Capability Registry, TelemetryHub (C1), confidence reconcile (C2/C5).
- **No new scheduler/planner/telemetry** — the policy engine consumes existing engines. No duplicate
  ownership introduced.

---

## Self-review (independent Principal Architect, brutal pass)

- **Race: policy decides `Optimize`, then user types before executor restarts.** Mitigation: executor
  re-checks user activity + lock + epoch immediately before the disruptive op, under lock; if Active,
  abort the optimization (it was never urgent). Foreground guard already enforces this.
- **Policy conflict: lock says no-restart, emergency says must-shrink.** Resolution order is explicit:
  **Emergency (correctness) > User workflow (image) > Lock > Optimization.** Emergency may break the
  lock (with streaming checkpoint); optimization never may.
- **DeepIdle promotion restart could surprise a user who returns mid-restart.** Mitigation: promotion
  only starts after T2 DeepIdle AND aborts instantly if input arrives during the (short) pre-spawn
  window; worst case a single brief "Loading model…" that the activity check minimizes.
- **Volatility reserve mis-tuned → under-use on dedicated GPU / over-use on churny desktop.**
  Mitigation: derive it from measured telemetry variance, bounded; converges per host.
- **Simulator inaccuracy → a "safe" promotion still OOMs.** Mitigation: conservative simulator bias +
  the failure ceiling/cooldown + recover-to-CPU (already built) as the backstop; one failure pins the
  session. Never thrashes.
- **CPU-locked session never upgrades even when GPU truly frees.** Accepted tradeoff: one DeepIdle
  promotion attempt covers the common case; beyond that, stability > squeezing GPU. Documented.
- **Added latency from policy layer.** Negligible — pure functions; emergency path bypasses it.
No significant remaining architectural weakness: the design is correctness/safety-first,
disruption-minimal, desktop-volatility-aware, and scales low→high-end (lock + measured sizing apply
everywhere; multi-GPU/cloud are extra devices in the same model).

---

## Implementation order (when implementation resumes)

1. **G1 sizing** (measured-first + volatility reserve + bounded calibration) — foundation; makes the
   one-shot load correct. *Headless + hardware-tune.*
2. **G3 Resident Lock** in `ResidencyManager` — the UX keystone; once locked, no perf restarts.
   *Headless.*
3. **G6 User Activity Model** + **G7 modes** — gate all disruptive actions. *Headless.*
4. **G2 Policy Engine** + **G5 Benefit Eval** + **G4 state-driven optimization** — move decisions out
   of the watchdog; default `Stay`. *Headless.*
5. **Watchdog → executor** refactor (keep emergency reflex). *Headless + soak.*
6. **G9 image policy** (simulator-gated Tier-B + deterministic restore) + **G10 UX** + **G11 logging**.
   *Headless + live image test.*
7. **G8 startup** finalization (already C6) + validation.
8. Cascading cleanup; remove dead `HubTelemetry`.

## Risk analysis

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Policy/executor race on activity change | med | med | re-validate under lock+epoch before disruptive op |
| Volatility reserve mis-tuned | med | low | telemetry-variance-derived, bounded |
| DeepIdle promotion surprises returning user | low | low | abort on input; short window; honest banner |
| Simulator over-optimistic → OOM | low | med | conservative bias + ceiling/cooldown + recover-to-CPU |
| CPU-locked session won't upgrade | med | low | one DeepIdle promotion; stability prioritized |
| Refactor regresses emergency OOM reflex | low | high | keep the reflex in the watchdog, test with fault injection |
| Cascading changes introduce inconsistency | med | med | reuse existing engines; no new scheduler/planner; soak |

**Hardware-gated (cannot finish headless):** verify measured sizing loads a fitting ngl; verify no
perf restarts in a real session; tune volatility reserve + benefit constants on the target GPUs.

---

## Bottom line
The architecture moves from "optimize whenever possible" to "stay locked unless correctness, safety,
or the user demands a change." A restart becomes a rare, explicit, simulator-proven, fully-narrated
event — never a background surprise. This is the permanent, optimal, production-grade policy for
low/medium/high-end and multi-GPU/cloud hosts.
