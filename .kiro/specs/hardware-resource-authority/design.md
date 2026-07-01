# Design — KRIA Hardware & Resource Authority (HRA)

Status: Blueprint (no implementation). Tier-0 core foundation.
Companion: `requirements.md`, `tasks.md`.

This design has been hardened through a 10-iteration review loop (critical reviewer →
production engineer → SRE → performance engineer → FAANG review board). The final design is
presented first; the **Design Review Log** (§12) records what each iteration broke and how the
design changed, so the reasoning is auditable.

---

## Overview

One **control plane** decides; many **consumers** execute; one **telemetry plane** observes.
Decisions are deterministic (no LLM in the decision path). The authority is crash-recoverable and
fails open to a safe CPU/cloud default. The remainder of this document details architecture,
components, data models, correctness, error handling, and testing.

## Architecture

### 1. Design goals & shape

One **control plane** decides; many **consumers** execute; one **telemetry plane** observes.
Decisions are deterministic. The authority is crash-recoverable and fails open to CPU/cloud.

```
                         ┌──────────────────────────────────────────────┐
                         │            Resource Authority (RA)            │  control plane
                         │  DeviceTable │ Planner │ Scheduler │ Journal   │  (one per host)
                         └───┬───────────────┬───────────────┬───────────┘
            reads (snapshot) │               │ grants lease  │ emits decisions+events
                             │               ▼               │
   ┌─────────────────────────┴──┐   ┌────────────────┐   ┌───┴───────────────────────────┐
   │   Telemetry Collector      │   │  Lease Tokens   │   │   Event/Journal Bus            │
   │ (one thread, watch publish)│   │ (RAII, TTL)     │   │ (correlated, persisted)        │
   └─────────────────────────┬──┘   └────────────────┘   └───────────────────────────────┘
                             ▲  snapshots
   ┌─────────────────────────┴───────────────────────────────────────────────────────────┐
   │ Consumers (data plane, execute under lease):                                          │
   │  LLM runtime │ STT │ TTS │ Wake │ Vision │ OCR │ ImageGen │ Embeddings │ Agents │ Ext  │
   └───────────────────────────────────────────────────────────────────────────────────────┘
```

Supervised by daemons (§9). All planes live inside `kria-core`; `kria-desktop` wires them and
bridges events to the UI.

---

## Components and Interfaces

### 2. Core abstractions

### 2.1 DeviceTable (single source of truth)
```
Device {
  id: DeviceId,                  // Cpu | Gpu(index) | CloudPool(provider)
  kind: DeviceKind,
  total_capacity: Capacity,      // vram_mb | ram_mb | rps/quota (cloud)
  reserved: Capacity,            // safety margin + active leases
  attributes: { latency_class, cost_class, privacy_class, power_class },
  health: DeviceHealth,          // Healthy | Degraded | Offline
  live: DeviceLive,              // from telemetry: free, util, temp, processes
}
```
- The RA owns the only mutable `DeviceTable`. Consumers see read-only projections.
- Multi-GPU is native: one `Device` per GPU index. Cloud pools are `Device`s too (R8.1).

### 2.2 ResourceRequest / Plan / Lease
```
ResourceRequest {
  consumer: ConsumerId,          // Llm | Stt | Tts | Wake | Vision | Ocr | Image | Embed | Agent | Ext
  class: PriorityClass,          // InteractiveFg | RealtimeVoice | InteractiveBg | Batch | Maintenance
  need: ResourceNeed,            // {vram_mb, ram_mb, cpu_threads, exclusivity, model_id, est_ms}
  constraints: { privacy, max_latency_ms, cost_ceiling, allow_cloud, power_budget },
  turn_id: TurnId,               // correlation id
}

Plan {                          // deterministic output of Planner
  device: DeviceId,
  residency: Residency,          // VramHot | RamWarm | DiskCold | Unloaded | Cloud
  budget: Capacity,
  fallback_chain: [Plan...],     // ordered safe fallbacks
  rationale: RationaleCode,      // enum → human string (explainability)
}

Lease {                         // grant; RAII guard releases on drop
  token: LeaseToken, device, budget, class, ttl, turn_id, plan_rationale
}
```

### 2.3 Residency model (per model)
`Unloaded → DiskCold → RamWarm → VramHot` with explicit transitions:
`load, warm, cool(evict_to_ram), unload, swap(device/ngl/ctx), reclaim, recover`.
Each transition is journaled and emits a correlated event.

---

## 3. Resource Authority (control plane)

### 3.1 Components
- **DeviceTable**: live device state (§2.1).
- **Planner** (deterministic): maps `ResourceRequest` + `DeviceTable` + active policy → `Plan`.
  Pure function; testable; no I/O; no LLM. Cost model below.
- **Scheduler**: admission, queueing, priority, preemption, fairness; issues `Lease`s.
- **Pressure Engine**: EMA + dwell + hysteresis per device; produces `PressureLevel` and remedy
  recommendations (R5.3 ordering).
- **Journal**: append-only decision log (lease grant/release, plan, preemption, eviction, failover)
  with monotonic seq + correlation ids. Backs crash recovery + diagnostics.
- **Reconciler**: on boot/restart, diffs journaled leases vs real device processes; reclaims orphans.

### 3.2 Planner cost model (deterministic)
Score each candidate `(device, residency)`:
```
feasible iff need.vram_mb <= device.free - device.safety_margin   (or co-residency budget)
cost = w_lat*latency_penalty + w_cost*money + w_power*power + w_priv*privacy_violation
       + w_disrupt*disruption(device)        // disruption = would this evict/interrupt others?
pick min cost among feasible; if none feasible locally and allow_cloud → cloud; else CPU.
```
Weights come from the active **PolicyProfile** (Battery-Saver, Balanced, Performance, Privacy-Strict).
Profiles are config; selection is deterministic from machine state (battery/thermal/AC) + user choice.

### 3.3 Admission & preemption
- Warm-path admission (resource already free) returns in microseconds (lock-free read of a
  per-device atomic free-budget + CAS reserve). Meets N1.
- Contended admission enters a per-device priority queue. `RealtimeVoice` jumps ahead; never
  preempted mid-utterance (R6.2).
- Preemption: higher class needs a held resource → RA sends a **checkpoint request** to the holder
  (cooperative), waits `preempt_deadline`, then forces reclaim via Reconciler if ignored. Always
  journaled with evidence (R12.2).
- Decision deadline (R1.5): if Planner/Scheduler can’t resolve in ≤50 ms, return the precomputed
  safe fallback `Plan` (CPU/cloud). Never block a consumer.

### 3.4 Pressure → remedy (no foreground interruption)
On `PressureLevel::Yield` for a device, RA applies remedies **in order, preferring non-disruptive**:
1. reclaim idle/background residency on that device,
2. shrink context/batch for non-active consumers,
3. downshift GPU layers at next turn boundary,
4. evict a background model to RAM,
5. route new work to another GPU or cloud.
Active foreground stream is touched ONLY at `PressureLevel::Emergency` (true OOM imminent), and
then with explain + checkpoint + auto-resume (R9.4). This directly removes the current
"Optimizing GPU layers..." mid-stream cancel (§0.2.7).

---

## 4. Telemetry plane (single collector)

- One `TelemetryCollector` per host on a dedicated OS thread (evolves the existing
  `TelemetryActor` in `orchestrator/telemetry.rs`). Owns ALL blocking device I/O:
  NVML per device, ROCm, sysinfo CPU/RAM, thermal sysfs, battery, `/proc` GPU process map.
- Publishes one immutable `HostSnapshot` via `watch` channel; RA and consumers borrow it
  (zero-cost). Replaces the three current stacks (R3.2, A2).
- `HostSnapshot { per_device: Vec<DeviceLive>, cpu, ram, disk, thermal, power, sampled_at, seq }`.
- Adaptive cadence: fast (250 ms–1 s) under pressure / active turn, slow (2–5 s) idle. Backpressure
  safe: collector never blocks consumers.
- Ring buffer (e.g., last 10 min) retained for diagnostics (§7).

---

## 5. Model lifecycle (uniform contract)

```
trait ModelLifecycle {
  fn descriptor(&self) -> ModelDescriptor;        // type, vram_est, ram_est, device_affinity
  async fn discover() -> Vec<ModelArtifact>;       // scan dirs / registry / cloud catalog
  async fn load(plan: &Plan) -> Handle;            // honor residency+device from RA
  async fn warm(&self) -> ();                      // prime kernels / KV
  async fn cool(&self) -> ();                      // evict_to_ram / shrink
  async fn unload(&self) -> ();
  async fn swap(&self, new_plan: &Plan) -> ();     // device/ngl/ctx change, KV save/restore
  fn residency(&self) -> Residency;
  fn health(&self) -> ModelHealth;
}
```

Per type (current code → HRA mapping):
- **Local LLM**: wraps `LlamaServerManager` (`server_manager.rs`). Swap uses Router-Mode API first,
  process-restart fallback, slot save/restore (already exists). Now driven by RA `Plan`, not the
  internal watchdog. The watchdog logic moves behind the RA Pressure Engine.
- **Cloud LLM**: `ModelLifecycle` over provider backends (`llm/provider/*`); "load/warm" = connection
  + capability probe; residency=Cloud; no VRAM.
- **STT/TTS**: wrap whisper/piper (`voice/stt.rs`,`tts.rs`); residency RamWarm or device per plan;
  in-process bindings are a future swap-in behind the same contract (resolves §0.2.9 cleanly later).
- **Vision (LLM mmproj)**: co-managed with LLM handle (single server). Vision sidecar = separate
  consumer with its own lease.
- **OCR**: defined as a first-class lifecycle (today only a marker). Loader = sidecar/ONNX; cloud
  fallback allowed.
- **Embeddings**: ONE primary (fastembed, `routing/embed.rs`) with ONNX (`memory/embeddings.rs`) as
  declared fallback; both behind the contract; global mutex replaced by a small worker pool (§8).
- **Image (ComfyUI)**: `image/orchestrator.rs` becomes a consumer; Tier-B drop-swap uses RA
  preemption + lease instead of its own lease + `LlmEvictionController`. The `LlmEvictionController`
  becomes "RA reclaims LLM residency on this device," generalized.

Idle unload (R4.6): RA schedules cool/unload by a predictive idle model (last-activity + time-of-day
+ pending work), and pre-warms on predicted foreground. Always announced (R9.x).

---

## 6. User experience design

- **Status surface**: a single `resource:status` event stream (additive to current
  `orchestrator:*` events) with `{state, cause, eta_ms, remedy, correlation_id}`. UI renders a calm,
  non-blocking banner; foreground input is NOT disabled for non-emergency actions (R9.3).
- **"What is KRIA doing" panel**: live view of active leases, residencies, queue, and current plan
  rationale (R9.5).
- **No silent degradation**: cloud failover/failback emits explicit notice; sticky session
  degradation requires user acknowledgement and shows a "restore local" affordance (R8.3, A9).
- **Emergency UX**: rare, explained, context-preserved, auto-resumed (R9.4).
- Replaces the abrupt "Optimizing GPU layers..." with either (a) nothing (deferred to turn
  boundary), or (b) a labeled, progress-bearing, non-blocking notice when unavoidable.

---

## 7. Observability, diagnostics, self-diagnosis

- **Correlation**: every lease/plan/event carries `turn_id`+`seq`. User-visible event → journal
  entry → telemetry window. Answers all R10.1 "why" questions.
- **Diagnostics bundle**: export `{journal slice, telemetry ring, device table, daemon health,
  active leases}` as a single signed artifact for support.
- **Anomaly detectors** (deterministic, run in Health Monitor daemon):
  - CPU/GPU spike: util > threshold for dwell → attribute to top process/consumer from snapshot.
  - VRAM/RAM leak: monotonic non-reclaimed growth across N idle windows → name the holder.
  - Starvation: queue wait > SLA for a class → report blocking lease.
  - Hung model: lease active but no telemetry progress + health stale → flag + checkpoint.
  - Deadlock: lock-wait watchdog on RA critical sections (timeouts, never indefinite).
  - Daemon crash / infinite retry: supervisor counters + circuit breaker.
  - Thermal throttle: temp > limit + clock drop → correlate to perf dip.
- Each detector emits a **root-cause hypothesis with evidence** (the offending consumer, the
  telemetry samples, the journal entries).

---

## 8. CPU / RAM / Disk / Embeddings concurrency

- **CPU**: RA assigns thread budgets per consumer; under contention it caps llama/whisper threads;
  affinity hints where supported. Spike detector attributes cause.
- **RAM**: `mlock` pre-flight stays (already in `server_manager.rs`) but is RA-gated; RA refuses
  loads that breach RAM safety.
- **Disk**: model cache/temp/logs get quotas + GC; low-space guard blocks new downloads with notice.
- **Embeddings concurrency**: replace the global `OnceCell<Mutex<TextEmbedding>>` and ONNX
  `Arc<Mutex<Session>>` with a small bounded worker pool (N workers sized by tier) so embeddings are
  not globally serialized (fixes §0.3). Still one logical model, many workers.

---

## 9. Daemon architecture

| Daemon | Responsibility | Isolation | IPC | Crash recovery |
|---|---|---|---|---|
| Core | hosts RA, telemetry, lifecycles, agent loop | main process | in-proc | process supervisor restarts; journal replay |
| Voice | capture, VAD, STT/TTS turn pipeline | task group / optional subprocess | event bus + lease | restart; wake stays live |
| Wake | wake-word always-on (split tap) | dedicated thread, never paused | direct trigger | independent of Voice/GPU swaps (R11.4) |
| GPU Monitor | feeds Telemetry per device; NVML/ROCm | dedicated thread | watch snapshot | restart, last-good snapshot |
| Health Monitor | anomaly detectors, daemon supervision | task | journal + events | self-restart; reports incidents |
| Extension Host | sandboxed skills/MCP/OpenClaw | subprocess/Docker | RPC + lease | restart with backoff + circuit breaker |

- Supervisor: backoff + circuit breaker; a daemon crash never crashes Core (R11.3).
- All daemons acquire hardware via RA leases — no daemon owns GPU directly.

---

## 10. Migration / backward compatibility

- Keep Tauri command/event names; add `resource:*` events additively (N5).
- Phase the cutover: introduce RA + single telemetry behind the existing orchestrator, then route
  image/voice/vision/embeddings lease calls to RA, then delete the duplicate `GpuLeaseManager`
  instances and the stub in `vision_automation.rs`, then collapse the watchdog into the RA Pressure
  Engine. Each phase independently shippable and reversible (see `tasks.md`).
- One tier function replaces the OR/AND divergence (§0.2.6).

---

## 11. Risks & mitigations (design-level)

- **RA as SPOF** → crash-recoverable journal + fail-open default + reconciler (R12.1, 12.3).
- **Central decision latency** → lock-free warm-path admission + 50 ms decision deadline.
- **Preemption deadlock** → cooperative checkpoint with hard deadline + forced reclaim; lock-wait
  watchdog on critical sections.
- **Telemetry staleness** → staleness flag + adaptive cadence; decisions on stale data marked.
- **Multi-GPU correctness** → per-device tables, per-device pressure, per-device leases.
- **Cloud cost runaway** → cost_ceiling in constraints + Battery/cost PolicyProfiles.

---

## 12. Design Review Log (iterations 2–10)

**It2 — Critical reviewer (broke v1):**
- v1 funneled execution through the authority → chokepoint. FIX: split control/data/telemetry
  planes; consumers execute under lease (§1).
- v1 had RA synchronously read NVML per request → blocking. FIX: single telemetry collector +
  snapshot reads (§4).

**It3 — improvement:** added Plan/fallback_chain + decision deadline + fail-open default (R1.5, §3.3).

**It4 — Production engineer (scaling):**
- Single GPU assumed. FIX: DeviceTable per-device, multi-GPU placement (R1.7, §2.1).
- Cloud not modeled as resource. FIX: cloud pools are Devices (R8.1, §2.1).
- Global embedding mutex would bottleneck at scale. FIX: bounded worker pool (§8).

**It5 — improvement:** PolicyProfiles (Battery/Balanced/Performance/Privacy) + cost model weights (§3.2).

**It6 — SRE (reliability):**
- No crash recovery → orphaned llama-server/ComfyUI on RA restart. FIX: Journal + Reconciler
  (R1.6, 12.1, §3.1).
- Preemption could hang. FIX: cooperative checkpoint + hard deadline + forced reclaim (§3.3).
- Daemon crash could cascade. FIX: supervisor + circuit breaker, Wake isolation (§9).

**It7 — improvement:** lock-wait watchdog on RA critical sections; staleness flags on telemetry (§4, §11).

**It8 — Performance engineer (latency):**
- Contended admission added latency to voice. FIX: RealtimeVoice fast lane, preemption-protected
  during utterance, p99 ≤ 2 ms target (R6.2, N1, §3.3).
- Pressure thrash. FIX: EMA + dwell + hysteresis + per-device rate limit (R5.4, §3.4) — carried
  from the proven watchdog logic.

**It9 — improvement:** adaptive telemetry cadence (fast under load, slow idle) to cut steady
overhead to N3 budget (§4).

**It10 — FAANG review board (attempt to reject):**
- "Where is the evidence for every user-visible event?" → correlation id end-to-end + diagnostics
  bundle (R10.2, §7).
- "Prove no LLM in the decision path." → Planner is a pure function; runtime assert + static check
  (R13, A10).
- "Prove single ownership." → grep gates in acceptance (A1/A2/A3); delete duplicates in migration.
- "Prove the UX promise (no surprise interruption)." → event-trace test: no non-emergency
  `stream_interrupted` during foreground (A4); pressure remedies prefer non-disruptive ordering (§3.4).
- Residual accepted risks: in-process whisper/piper bindings deferred (contract ready); distributed
  multi-host out of scope (single-host authority). Board: **accept as production-grade blueprint.**

---

## 13. Contracts summary (for implementation)

- `ResourceAuthority` (control): `request(ResourceRequest) -> Lease | Plan(fallback)`,
  `release(LeaseToken)`, `device_table() -> ReadOnlyView`, `subscribe_events()`.
- `TelemetryCollector`: `latest() -> HostSnapshot`, `history(window) -> &[HostSnapshot]`.
- `ModelLifecycle`: §5.
- `Journal`: `append(Decision)`, `replay() -> Vec<Decision>`, `reconcile(DeviceTable)`.
- `DaemonSupervisor`: `register(Daemon)`, `health() -> Map`, restart/backoff/circuit-break.

All live in `kria-core`; `kria-desktop` wires + bridges events. No public Tauri contract changes.

---

## Data Models

Authoritative types (Rust-shaped pseudocode; canonical definitions land in
`crates/kria-core/src/resource/authority/types.rs`).

```
enum DeviceId { Cpu, Gpu(u32), CloudPool(String) }
enum DeviceKind { Cpu, Gpu, Cloud }
enum Residency { Unloaded, DiskCold, RamWarm, VramHot, Cloud }
enum PriorityClass { InteractiveFg, RealtimeVoice, InteractiveBg, Batch, Maintenance }
enum ConsumerId { Llm, Stt, Tts, Wake, Vision, Ocr, Image, Embed, Agent, Ext }
enum PressureLevel { Normal, Yield, Emergency }
enum RationaleCode { FitsLocal, CoResident, EvictedBg, ShrunkCtx, Downshifted, FailoverCloud, FailOpenCpu }

struct Capacity { vram_mb: u64, ram_mb: u64, cpu_threads: u32, quota_rps: Option<u32> }

struct DeviceLive { free_vram_mb: u64, util_pct: u32, temp_c: Option<u32>,
                    processes: Vec<ProcVram>, sampled_at: Instant, seq: u64 }

struct Device { id: DeviceId, kind: DeviceKind, total: Capacity, reserved: Capacity,
                attributes: DeviceAttrs, health: DeviceHealth, live: DeviceLive }

struct DeviceAttrs { latency_class: u8, cost_class: u8, privacy_class: u8, power_class: u8 }

struct ResourceNeed { vram_mb: u64, ram_mb: u64, cpu_threads: u32, exclusivity: bool,
                      model_id: Option<String>, est_ms: u32 }

struct Constraints { privacy: PrivacyReq, max_latency_ms: u32, cost_ceiling: Option<u32>,
                     allow_cloud: bool, power_budget: PowerReq }

struct ResourceRequest { consumer: ConsumerId, class: PriorityClass, need: ResourceNeed,
                         constraints: Constraints, turn_id: TurnId }

struct Plan { device: DeviceId, residency: Residency, budget: Capacity,
              fallback_chain: Vec<Plan>, rationale: RationaleCode }

struct Lease { token: LeaseToken, device: DeviceId, budget: Capacity, class: PriorityClass,
               ttl: Duration, turn_id: TurnId, rationale: RationaleCode }

struct HostSnapshot { per_device: Vec<DeviceLive>, cpu: CpuLive, ram: RamLive, disk: DiskLive,
                      thermal: ThermalLive, power: PowerLive, sampled_at: Instant, seq: u64 }

struct Decision { seq: u64, turn_id: TurnId, kind: DecisionKind, plan: Option<Plan>,
                  evidence: EvidenceRef, at: Instant }   // journaled, append-only
```

`HostSnapshot` is immutable and published via `watch`. `Device.reserved` = safety margin + sum of
active lease budgets. The journal is the only durable state required for crash recovery.

## Correctness Properties

### Property 1: Single grantor (no over-commit)
At any instant, the sum of granted budgets per device ≤ device.total − safety_margin. Enforced by
CAS reserve in the scheduler.

**Validates: Requirements 1.4, 5.1**

### Property 2: No uncoordinated co-use
Two leases on the same exclusive device cannot both be `Held` unless the plan explicitly granted a
co-residency budget that fits.

**Validates: Requirements 1.4, 5.2**

### Property 3: Determinism
`Planner::plan(request, snapshot, profile)` is a pure function — identical inputs yield identical
`Plan`. No clock/RNG/LLM/IO inside.

**Validates: Requirements 13.1**

### Property 4: Foreground safety
No non-emergency remedy cancels an active foreground stream; only `PressureLevel::Emergency` may,
and only with checkpoint+resume.

**Validates: Requirements 9.3, 5.3**

### Property 5: Liveness
Every queued request is eventually admitted or returned a fallback within the decision deadline; no
unbounded wait (fairness counters prevent starvation).

**Validates: Requirements 1.5, 6.4**

### Property 6: Recovery soundness
After restart, the reconciler converges DeviceTable to real device state; no journaled lease
references a dead process; no live inference process lacks a lease.

**Validates: Requirements 12.1**

### Property 7: Correlation completeness
Every user-visible `resource:*` event has a resolvable journal `seq` and a telemetry window.

**Validates: Requirements 10.2**

### Property 8: Prewarm non-eviction
A WPE speculative prewarm never reduces the budget available to an equal-or-higher priority class;
it consumes only free headroom and auto-cools on confidence decay or thermal/battery veto.

**Validates: Requirements 14.2**

### Property 9: Session-profile advisory-only
A SIP profile change alters Planner cost weights only; it never issues a hard load/evict command,
and a minority workload within a session never flips the active profile.

**Validates: Requirements 15.3**

### Property 10: Forecast lead-time safety
A forecast may advance a non-disruptive remedy earlier but can never itself cause a foreground
interruption; only the live Emergency level (with checkpoint) may.

**Validates: Requirements 16.2**

### Property 11: Epoch monotonicity (no split-brain)
Leases carry the RA epoch; after restart epoch strictly increases and every pre-epoch lease is
invalid; no two consumers hold an exclusive device in the same epoch.

**Validates: Requirements 21.1**

### Property 12: AOL isolation
The Autonomous Optimization Layer has no reference to the admission API; it can write only to
prewarm-hint / profile-suggestion stores. Enforced at module boundary.

**Validates: Requirements 20.2**

### Property 13: Privacy-bounded failover
Data tagged Privacy-Strict never egresses to a cloud Device; the Planner fails it to CPU instead.

**Validates: Requirements 23.2**

### Property 14: Bounded admission
Per-class admission queues are bounded; under overload the lowest classes are shed first with
explicit UX, and expired-deadline requests are dropped, not admitted late.

**Validates: Requirements 21.3**

### Property 15: Single residency executor
Every load/warm/cool/evict/swap/restore for every model type is executed by the `ResidencyManager`;
no consumer or engine calls a model lifecycle transition directly. Transitions are serialized per
model (one in-flight).

**Validates: Requirements 24.1**

### Property 16: Pre-commit simulation
No unload/swap/evict/migration/image-transition/cloud-failover is committed without a deterministic
`simulate()` estimate journaled alongside the decision; a predicted hard-limit breach forces a
fallback instead of commit.

**Validates: Requirements 25.1**

### Property 17: Single foreground owner
At any instant exactly one Foreground Owner exists; it is preemption-protected, and Background
Owners yield before it under pressure; no owner is starved.

**Validates: Requirements 26.2**

### Property 18: Band-derived budgets (no double accounting)
Soft/Hard/Emergency limits are derived from the existing capacity/reservation/safety values; there
is exactly one accounting of free/reserved per device, and admission gates on Hard while remedies
begin at Soft.

**Validates: Requirements 27.1**

### Property 19: Deterministic capability selection
Model selection is a pure lookup against the CapabilityRegistry; identical request + registry yields
identical model choice; no LLM participates.

**Validates: Requirements 28.2**

## Error Handling

- **Telemetry loss / NVML failure** → mark device `live` stale; Planner treats stale device as
  reduced-confidence; if a turn needs it and it's stale beyond a bound → fail open to CPU/cloud.
- **Planner stall / deadline exceeded** → return precomputed safe fallback Plan (CPU or cloud),
  journal `FailOpenCpu`/`FailoverCloud`, emit notice.
- **Lease TTL expiry** → reconciler reclaims; holder receives `LeaseRevoked`; consumer must
  re-request (idempotent).
- **Preemption ignored past deadline** → forced reclaim via reconciler (process checkpoint then
  terminate), gated by safety policy, with before/after audit evidence.
- **Model load failure** → bounded backoff retry; after N attempts emit structured
  `FailureReport{stage,cause,remedy}` and fall back per plan chain.
- **Journal write failure** → degrade to in-memory journal + warn; never block a decision on
  durable write.
- **Daemon crash** → supervisor restart with backoff + circuit breaker; Core process survives.
- All error paths fail open to a deterministic safe default; none may hang or loop unbounded.

## Testing Strategy

- **Unit**: tier classification matrix; Planner golden plans per hardware class + PolicyProfile;
  pressure EMA/dwell/hysteresis on recorded VRAM traces; lease reserve/release invariants
  (property tests for CP1).
- **Concurrency**: loom/stress on scheduler for no-deadlock + fairness (CP5); voice fast-lane p99
  bench (N1).
- **Integration**: shadow-mode decision comparison vs current orchestrator; LLM swap soak (no
  non-emergency interruption, CP4/A4); Tier-B image drop-swap; cloud failover/failback (A9);
  multi-GPU concurrent placement (A8).
- **Fault injection**: NVML failure, planner stall, lease expiry, preemption ignore, daemon crash;
  assert fail-open within deadline (A22/R1.5) and correct anomaly root-cause (R10.3).
- **Recovery**: kill-restart authority → reconcile, zero leaked llama-server/ComfyUI (A7/CP6).
- **End-to-end "why"**: assert each R10.1 question answerable from the diagnostics bundle (A6).
- Suites live under `crates/kria-eval/`.

---

## 14. Predictive & Adaptive Subsystems (added during hardening)

These four engines feed the RA with **advisory** signals. None of them can call the Scheduler or
Planner admission API. They change only residency warmth (prewarm) and Planner cost weights (bias).

### 14.1 Workload Prediction Engine (WPE)
- Inputs (deterministic signals, no LLM): UI focus/panel-open, prompt keystroke activity, file drop,
  mic-open, workflow start, recent tool history.
- Output: `PrewarmHint { consumer, model_id, confidence, ttl }`.
- Rules: prewarm only into free headroom; never evict ≥ class; cap by `prewarm_budget_mb`; revoke on
  confidence decay or TPPE/battery veto. (Property 8.)
- State machine: `Idle → Suspected(conf) → Prewarming(lease=Speculative) → Warm → Cooling → Idle`.

### 14.2 Session Intent Profiles (SIP)
- Classifier: deterministic feature scoring over a sliding window of consumer activity →
  `Coding | Voice | Image | Automation | Research | Idle | Mixed`.
- Hysteresis: profile switches only after dwell + confidence ≥ threshold; minority workloads ignored.
- Effect: sets Planner cost-weight preset and residency preference (e.g., Coding pins LLM warm,
  tolerates embedding bursts without eviction). Advisory only (Property 9).
- State machine: `Idle → Detecting → Active(profile) → (dwell) → Active(profile') | Idle`.

### 14.3 Resource Forecasting Engine (RFE)
- Method: per-resource EWMA + slope projection with confidence band over the telemetry ring.
- Output: `Forecast { resource, time_to_threshold_s, confidence }` for VRAM/RAM/thermal.
- Use: when `time_to_threshold_s < lead_window` and confidence high, RA advances the **non-disruptive**
  remedy ladder early (reclaim idle, shrink bg ctx, route new work elsewhere). Never interrupts
  foreground (Property 10).

### 14.4 Thermal & Power Policy Engine (TPPE)
- Inputs: thermal sensors, GPU clocks/util, battery presence/charge/AC.
- Outputs: active PolicyProfile (Battery-Saver/Balanced/Performance/Privacy-Strict + Thermal-Capped),
  GPU duty-cycle budget, prewarm veto.
- Throttle avoidance: predict junction-temp trend; pre-emptively cap duty cycle before the driver
  throttles. Sensor-absent → conservative "thermal-unknown" profile (Property: R17.3).

### 14.5 Autonomous Optimization Layer (AOL)
- Learns (offline/online, bounded) time-of-day + workload patterns → adjusts WPE hint priors and
  suggests PolicyProfiles. Writes only to advisory stores. No admission handle (Property 12).
- Cold-start neutral: empty model = no influence; cannot cause harm.

---

## 15. Reliability hardening (added during hardening)

### 15.1 Epoch fencing (split-brain kill)
- RA holds `epoch: u64` persisted in the journal. Every `Lease.epoch = current_epoch`.
- On RA (Core) restart: `epoch += 1`, journal records it. All pre-epoch leases invalid.
- Consumers store their lease epoch and revalidate (`ra.current_epoch()` atomic) before each GPU op;
  mismatch → consumer aborts the op and re-requests. (Property 11.)

### 15.2 Journal integrity & versioning
- Record = `{ver, seq, payload, crc32}`; recovery truncates at first bad CRC (last-good wins).
- Periodic compacted snapshot + tail replay. Unknown future fields tolerated; incompatible major
  version → safe cold reconcile from live device state.

### 15.3 Bounded queues + load-shedding
- Per-class bounded queues. Overload sheds Maintenance → Batch → InteractiveBg first, with explicit
  `resource:status` notice. Expired-deadline requests dropped pre-admission.

### 15.4 Cloud Device health
- Each cloud pool has a circuit breaker (closed/half-open/open) driven by observed error rate +
  latency; honors `Retry-After`. Planner skips open pools. Prevents failover storms.

### 15.5 Foreground streaming checkpoint
- Emergency reclaim of a foreground LLM: flush partial tokens to UI, KV slot save, labeled notice,
  auto-resume from saved KV. Hard wall; on exceed → abort-with-resume (never silent).

---

## 16. Operability (added during hardening)

### 16.1 RA bypass kill-switch
- Per-consumer flag (config + UI). When set, that consumer uses a static plan (full-GPU if it fits,
  else CPU/cloud) and does not consult the authority. Lets prod revert instantly if RA misbehaves.

### 16.2 SLOs & metrics
- SLOs: admission p99 ≤ 5 ms; voice admission p99 ≤ 2 ms; OOM events = 0; swaps/hr below budget;
  prewarm-waste ratio bounded. Metrics are low-cardinality counters/histograms; `turn_id` lives only
  in traces + journal.

### 16.3 Shadow comparator
- Replays identical `HostSnapshot` stream to the legacy path and RA; asserts RA never over-commits and
  never introduces a foreground interrupt the legacy path avoided; emits a divergence report that
  gates cutover.

---

## 17. Security boundaries (added during hardening)

- **Reclaim authz**: process termination requires a capability token; Reconciler tracks RA-spawned
  PIDs at spawn and may kill ONLY those. Never kills arbitrary PIDs.
- **Privacy-bounded egress**: `Constraints.privacy = Strict` → never placed on a cloud Device; fails
  to CPU. Audited (Property 13).
- **Extension host**: sandboxed subprocess/Docker; acquires resources only via RA lease; cannot read
  the DeviceTable directly.

---

## 18. Distributed-readiness extension points (no implementation now)

- `DeviceId::RemoteHost(host_id, DeviceId)` reserved.
- `ResourceAuthority` is a trait; today a local in-proc impl; a future gRPC client impl could front a
  cluster authority. Request/Plan/Lease are already serializable.
- `Execution` is separated from `Placement`, so remote execution can be added without changing the
  control plane contract.

---

## 19. Updated data models (additions)

```
struct PrewarmHint { consumer: ConsumerId, model_id: String, confidence: f32, ttl: Duration }
enum SessionProfile { Coding, Voice, Image, Automation, Research, Idle, Mixed }
struct Forecast { resource: ResourceKind, time_to_threshold_s: f32, confidence: f32 }
struct CapabilityVector { cpu: u8, gpu: u8, vram: u8, ram: u8, thermal: u8, power: u8 }
enum PolicyProfile { BatterySaver, Balanced, Performance, PrivacyStrict, ThermalCapped }
struct Epoch(u64)
struct LeaseV2 { /* Lease */ epoch: Epoch, speculative: bool }
struct JournalRecord { ver: u16, seq: u64, payload: Decision, crc32: u32 }
enum BreakerState { Closed, HalfOpen, Open { until: Instant } }
struct CloudHealth { breaker: BreakerState, err_rate: f32, p99_ms: u32, retry_after: Option<Instant> }
struct CapabilityToken(Uuid)        // required for Reconciler kill
enum DeviceId2 { Cpu, Gpu(u32), CloudPool(String), RemoteHost(String, Box<DeviceId2>) }
```

---

## 20. Updated daemon set (additions)

| Daemon | Added responsibility |
|---|---|
| Prediction daemon | hosts WPE + SIP + AOL (advisory only; no admission handle) |
| Forecast worker | runs RFE on the telemetry ring |
| Thermal/Power worker | runs TPPE, owns PolicyProfile switching |
| (Core) | now owns epoch, journal integrity, shadow comparator, bypass switch |

All advisory daemons are crash-isolated; their death degrades to "no prediction / no forecast"
(reactive RA still correct), never to a wrong admission decision.

---

## 21. Frontend/UX architecture (summary; full spec in `frontend-ux-spec.md`)

Six views, fed by the additive `resource:*` event stream + a read-only RA snapshot query:
Resource Dashboard, Explainability UI, Session Awareness, Forecasting UI, Recovery UI, Diagnostics
export. Foreground input is never disabled for non-emergency actions; emergency shows a labeled,
progress-bearing, auto-resuming notice.

---

## 22. Final hardening additions (minimal, additive — no protected component redesigned)

Each item below extends an existing component. Rationale: why added / problem solved / why the
existing design was insufficient / why this is the minimal fix.

### 22.1 ResidencyManager (Gap 1)
- **Why**: residency execution was implicitly spread across Planner, per-model `ModelLifecycle`, and
  the Pressure Engine. Three callers of the same transitions → race + test surface explosion.
- **Problem solved**: a single owner serializes per-model transitions and gives one place to test
  and observe load/warm/cool/evict/swap/restore.
- **Why existing was insufficient**: `ModelLifecycle` defines the *operations* but not a single
  *owner*; nothing prevented two callers issuing overlapping transitions.
- **Minimal**: it does NOT decide placement (RA still does). It wraps the existing `ModelLifecycle`
  trait and is called by RA/Pressure/WPE instead of them calling lifecycles directly.
- Residency FSM (per model), owned here:
  `Unloaded → Loading → VramHot ⇄ RamWarm → Cooling → Unloaded`, plus `Swapping`, `Restoring`.
  One in-flight transition per model; others queue. Emits `resource:residency` events.

### 22.2 Resource Simulator (Gap 2)
- **Why**: Scheduler committed disruptive actions blind to their predicted cost.
- **Problem solved**: estimate impact before commit → avoid OOM/regret swaps.
- **Why existing was insufficient**: Planner cost model ranks placements but does not pre-flight the
  *transition* (e.g., will evicting model X actually free enough after fragmentation/lag?).
- **Minimal**: a pure function, not a framework:
  `simulate(action, HostSnapshot) -> Estimate { d_vram_mb, d_ram_mb, est_latency_ms, disruption:
  None|Background|Interactive|Foreground, risk: Low|Med|High }`.
  Scheduler calls it pre-commit; result journaled. A `risk=High` or predicted Hard-limit breach →
  take next fallback. Reuses DeviceTable + bands; adds no new accounting.

### 22.3 Session Ownership (Gap 3)
- **Why**: SIP described *mode* but not *who owns the resource* when many consumers run at once.
- **Problem solved**: removes scheduling ambiguity + thrashing + starvation under concurrency.
- **Why existing was insufficient**: PriorityClass orders requests but didn't name a single
  Foreground Owner across subsystems.
- **Minimal**: a thin `SessionOwnership` view derived from PriorityClass + SIP + UI focus:
  `{ foreground: ConsumerId, interactive: [ConsumerId], background: [ConsumerId] }`. Advisory to
  scheduler weights only. Fairness floor per class prevents starvation. No new daemon (lives in the
  Prediction daemon next to SIP).

### 22.4 Multi-band Memory Budget (Gap 5)
- **Why**: single safety margin under-expressed the difference between "start reclaiming" and
  "refuse" and "emergency."
- **Problem solved**: clear, testable bands mapped to existing thresholds.
- **Why existing was insufficient**: yield/critical existed in the Pressure Engine but not as a
  first-class DeviceTable property the Scheduler could gate admission on.
- **Minimal**: derive bands from existing values — `soft = total - safety - soft_pct`,
  `hard = total - safety`, `emergency = total - emergency_pct`. ONE accounting. Pressure yield→Soft,
  critical→Emergency; admission gate→Hard. No duplicate counters (Property 18).

### 22.5 Capability Registry (Gap 6)
- **Why**: future many-model support needs a deterministic way to answer "what can this model do."
- **Problem solved**: explainable, LLM-free model selection across all model types.
- **Why existing was insufficient**: `ModelDescriptor` carried resource estimates but not
  capability/quality/latency-class metadata for selection.
- **Minimal**: a declarative table (config + discovered):
  `ModelCapability { id, kind, capabilities:[..], quality_tier, latency_class, resource_profile }`.
  Planner does a pure lookup/filter. No AI selection (Property 19).

### 22.6 SLA Framework (Gap 4)
- **Why**: SLOs existed at system level (§16.2) but not per user-facing operation with
  Target/Warning/Critical bands.
- **Problem solved**: measurable, alertable per-operation guarantees wired to Health + Diagnostics.
- **Why existing was insufficient**: §16.2 SLOs were coarse; no per-op thresholds, no breach surface.
- **Minimal**: an `SlaTable` (data, below) consumed by Health Monitor + Diagnostics; no new engine.

### 22.7 Benchmark Framework (Gap 7)
- **Why**: no objective before/after or regression gate for a Tier-0 system.
- **Problem solved**: production validation + regression detection per hardware class.
- **Why existing was insufficient**: `kria-eval` has E2E suites but no resource-efficiency benchmark.
- **Minimal**: extend `kria-eval` with a benchmark mode + fixed scenarios; emits comparable reports.

---

## 23. Final-pass data models (additions)

```
// Gap 1
enum ResidencyState { Unloaded, Loading, VramHot, RamWarm, Cooling, Swapping, Restoring }
trait ResidencyManager { async fn transition(model: ModelId, target: Residency) -> Result<()>; }

// Gap 2
enum Disruption { None, Background, Interactive, Foreground }
enum RiskLevel { Low, Med, High }
struct Estimate { d_vram_mb: i64, d_ram_mb: i64, est_latency_ms: u32,
                  disruption: Disruption, risk: RiskLevel }

// Gap 3
struct SessionOwnership { foreground: Option<ConsumerId>,
                          interactive: Vec<ConsumerId>, background: Vec<ConsumerId> }

// Gap 5
struct Budget { soft_mb: u64, hard_mb: u64, emergency_mb: u64 }   // derived view, not new counters

// Gap 6
enum QualityTier { Draft, Standard, High, Max }
enum LatencyClass { Realtime, Interactive, Batch }
struct ModelCapability { id: String, kind: ConsumerId, capabilities: Vec<String>,
                         quality_tier: QualityTier, latency_class: LatencyClass,
                         resource_profile: ResourceNeed }

// Gap 4
enum SlaState { Ok, Warning, Critical }
struct Sla { op: String, target_ms: u32, warning_ms: u32, critical_ms: u32 }
struct SlaTable { entries: Vec<Sla> }

// Gap 7
struct BenchResult { scenario: String, hw_class: String, vram_peak_mb: u64, ram_peak_mb: u64,
                     cpu_pct: f32, gpu_pct: f32, p50_ms: u32, p99_ms: u32,
                     throughput: f32, queue_delay_ms: u32, swaps: u32, recovery_ms: u32 }
```

---

## 24. SLA reference table (initial targets — Target / Warning / Critical, ms)

| Operation | Target | Warning | Critical |
|---|---|---|---|
| Voice: wake latency | 150 | 300 | 600 |
| Voice: STT latency (per utterance) | 800 | 1500 | 3000 |
| Voice: TTFA (time to first audio) | 500 | 900 | 1800 |
| Voice: TTS latency (per sentence) | 400 | 800 | 1500 |
| Chat: first token | 700 | 1500 | 4000 |
| Chat: response completion (short) | 4000 | 8000 | 20000 |
| Image: queue wait | 1000 | 3000 | 8000 |
| Image: generation start | 3000 | 8000 | 20000 |
| Image: generation completion | 20000 | 45000 | 90000 |
| Automation: task start | 1000 | 3000 | 8000 |
| Automation: task completion | (scenario) | (scenario) | (scenario) |
| Cloud: failover | 500 | 1500 | 4000 |
| Cloud: recovery (failback) | 2000 | 5000 | 15000 |

Values are initial; the Benchmark Framework (Gap 7) calibrates per hardware class and the SlaTable is
config-overridable. Health Monitor raises Warning/Critical; Diagnostics shows breaches with evidence.
