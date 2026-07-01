# Requirements Document

## Introduction

Tier-0 Core System. This document specifies the requirements for a single, authoritative
hardware/resource/model/daemon orchestration foundation for KRIA (the Hardware & Resource
Authority, "HRA"). It is a blueprint, not an implementation. It supersedes the fragmented
resource handling that exists today. Current-reality reports (architecture, problems,
bottlenecks, failure modes, UX, scalability, technical debt) are captured in §0 and drive the
requirements in §3.

---

## 0. Context — Current Reality (baseline being replaced)

These findings are drawn from a forensic read of the current codebase and define the
problems HRA must solve. File references are the authoritative source of truth.

### 0.1 Current Architecture (as built)
- **LLM orchestrator** (`crates/kria-core/src/llm/orchestrator/`): owns llama-server lifecycle
  and dynamic GPU-layer offload. `Orchestrator::start` → `GpuBackend::detect` → `TelemetryActor`
  → `LlamaServerManager::spawn` → `GpuWatchdog::run`.
- **GPU watchdog** (`gpu_watchdog.rs`): VRAM EMA state machine
  (`Idle/Pressured/Cooldown/Recovering/Critical`); thresholds from `threshold.rs`
  (% of total VRAM). Triggers swaps that cancel in-flight streams.
- **Lease manager** (`resource/gpu_lease.rs`): `GpuLeaseManager` state machine
  (`Idle/Held/Recovering/Degraded`), owners `L1Worker/ImageBackend/Vision/Speech/Maintenance`.
- **Telemetry**: three independent stacks — `orchestrator/telemetry.rs` (TelemetryActor→watch),
  `platform/vram.rs` (VramProfiler: NVML/ROCm/Null), `resource/telemetry.rs` (ResourceSnapshot).
- **Hardware detection**: `platform/detect.rs` (nvidia-smi, sysinfo) + `infra/hardware_profiler.rs`
  (NVML, persisted to `~/.kria/hardware_profile.json`).
- **Image**: `image/orchestrator.rs` + `image/swap.rs` (Tier-B drop-and-swap via
  `LlmEvictionController`). **Voice**: `voice/stt.rs`/`tts.rs` (whisper/piper subprocess).
  **Embeddings**: `routing/embed.rs` (fastembed) + `memory/embeddings.rs` (ONNX).

### 0.2 Current Problems (evidence-backed)
1. **Fragmented GPU ownership**: ≥4 independent `GpuLeaseManager` instances (LLM `mod.rs`,
   image `image/orchestrator.rs`, speech `voice_runtime_helpers.rs`, vision `runtime.rs`).
   No global arbiter. Cross-subsystem GPU contention is uncoordinated.
2. **Dead reconcile path**: `set_resource_telemetry` is never called → lease `telemetry` always
   `None` → async `acquire_lease` reconciliation always returns `Degraded`. Only sync
   `acquire_guard`/`acquire_token` paths work.
3. **Stub duplicate**: `tools/vision_automation.rs` defines a second `GpuLeaseManager`
   (scaffolding) that always grants.
4. **Three VRAM truth sources** with three `VramSnapshot` types — divergent readings.
5. **Two embedding engines** (fastembed + ONNX) coexist.
6. **Inconsistent tier logic**: `detect.rs::classify_tier` uses OR, `hardware_profiler.rs`
   uses AND — same hardware → different tier.
7. **Mid-stream interruption by design**: watchdog `cancel_streams()` aborts the live LLM
   stream on any pressure swap → user sees "Optimizing GPU layers..." (`ChatView.tsx`).
8. **False telemetry in reconcile**: speech/vision reconcile snapshots hardcode `vram=0`.
9. **Subprocess-only STT/TTS**: in-process whisper-rs/piper-rs paths `unimplemented`.
10. **Dead code**: `AudioFreezeGuard` v1 (`vad_reset_fn` never fires) vs `AudioFreezeGuardV2`;
    deprecated `create_cuda_telemetry`.

### 0.3 Current Bottlenecks
- `Orchestrator.lifecycle_lock: Mutex<()>` serializes all swap/evict/reload.
- fastembed `static OnceCell<Mutex<TextEmbedding>>` serializes ALL embeddings globally.
- ONNX `Session` behind `Arc<Mutex>` serializes memory embeddings.
- Cross-manager GPU contention NOT serialized (root architectural gap).
- VRAM-only decisions ignore collected `gpu_util_pct`.

### 0.4 Current Failure Modes
- Lease stuck `Recovering` → `Degraded` after timeout → silent CPU downgrade for STT.
- Tier-B swap VRAM timeout twice → sticky session cloud-only (`session_degraded`).
- nvidia-smi wedged → 5s block then `(None,None)` → wrong tier.
- llama-server fails port discovery → spawn error, no automatic structured retry surfaced to UX.

### 0.5 Current UX Problems
- Sudden "Optimizing GPU layers..." mid-answer; input disabled.
- Silent model unload on idle-release; next turn pays cold-start.
- Invisible background swaps; no "why" surfaced.
- Image gen silently degrades to cloud for rest of session.

### 0.6 Current Scalability Problems
- No multi-GPU support (NVML device 0 hardcoded everywhere).
- No cloud-as-first-class resource pool.
- Single-holder lease cannot express co-residency budgets (only S/A tiers parallel, hardcoded).
- No per-subsystem quota or priority class beyond foreground/background/Maintenance.

### 0.7 Current Technical Debt
- Triple telemetry, double embeddings, duplicate lease types.
- Tier classification divergence.
- Dead async lease path.
- Hardcoded device index, hardcoded image tier VRAM cutoffs.

---

## Glossary

- **HRA**: Hardware & Resource Authority — the single resource control plane.
- **RA (Resource Authority)**: the authoritative in-process service that owns all
  CPU/GPU/VRAM/RAM/disk admission decisions.
- **Consumer**: any subsystem requesting resources (LLM, STT, TTS, Vision, OCR, Image,
  Embeddings, Agents, GUI automation, Extensions).
- **Lease**: a time-bounded, accountable grant of a resource budget to one consumer.
- **Residency**: where a model's weights physically live (VRAM hot, RAM warm, disk cold, unloaded).
- **Device**: a compute target (CPU, a specific GPU index, or a cloud endpoint pool).
- **Plan**: a deterministic placement decision (which device, what budget, what fallback).
- **Pressure**: measured scarcity of a resource relative to reserved thresholds.

---

## 2. Single Resource Authority — principle & challenge

**Stated principle**: no subsystem may independently own GPU/VRAM or make resource decisions;
all decisions flow through one authority.

**Verdict: ADOPT, with refinement.** A single *decision* authority is correct and required —
it is the only way to eliminate the fragmented-ownership class of bugs (§0.2.1). However a naive
"single authority does everything synchronously" design becomes a latency bottleneck and a single
point of failure. HRA therefore splits the principle into three planes:

- **Control plane (single authority)**: one `ResourceAuthority` owns all *admission and placement
  decisions* and the single source of truth for device state. Centralized, deterministic, no LLM.
- **Data plane (distributed execution)**: consumers execute work themselves (run llama-server,
  whisper, ComfyUI) under a granted lease. Execution is NOT funneled through the authority.
- **Telemetry plane (single collector, many readers)**: one telemetry collector per host;
  consumers and the authority read snapshots, never sample independently.

This preserves "one brain" for decisions while avoiding a serialized execution chokepoint and a
hard SPOF. The authority must be crash-recoverable and must fail *open to a safe deterministic
default* (CPU/cloud), never fail closed into a hang.

---

## Requirements

Functional requirements in EARS form, grouped by area.

### 3.1 Resource Authority (control plane)
- R1.1 The system SHALL expose exactly one `ResourceAuthority` per host process as the sole
  grantor of CPU/GPU/VRAM/RAM/disk leases.
- R1.2 WHEN any consumer needs a hardware-backed resource, the consumer SHALL request a lease
  from the `ResourceAuthority` and SHALL NOT read raw device telemetry to self-decide.
- R1.3 The `ResourceAuthority` SHALL maintain a single authoritative `DeviceTable` of all devices
  (CPU, each GPU index, cloud pools) with live capacity and reservations.
- R1.4 WHEN two consumers request conflicting GPU budgets, the `ResourceAuthority` SHALL resolve
  by deterministic priority + fairness policy (§3.6) and SHALL NOT allow uncoordinated co-use.
- R1.5 IF the `ResourceAuthority` cannot make a decision within a bounded deadline, THEN it SHALL
  return a deterministic safe fallback plan (CPU or cloud) rather than blocking.
- R1.6 The `ResourceAuthority` SHALL persist a minimal decision journal so a crash can be
  reconstructed and a restart can reclaim/release orphaned leases.
- R1.7 The `ResourceAuthority` SHALL support multiple GPUs and SHALL place work per-device.

### 3.2 Hardware discovery
- R2.1 The system SHALL detect CPU (cores, threads, base/affinity), RAM, every GPU (vendor, index,
  total/free VRAM), disk capacity, battery presence/state, and thermal sensors where available.
- R2.2 Hardware discovery SHALL use one code path with one tier-classification function (resolve
  the OR/AND divergence in §0.2.6).
- R2.3 GPU probing SHALL be non-blocking with a bounded timeout and SHALL degrade to a known tier
  on probe failure.
- R2.4 The system SHALL persist a hardware profile and SHALL re-validate it on boot (hot-plug,
  driver change, eGPU) rather than trusting a stale cache.
- R2.5 The system SHALL support NVIDIA (NVML), AMD (ROCm), Apple Silicon (Metal/unified), Intel,
  and CPU-only, reporting unsupported acceleration explicitly.

### 3.3 Telemetry (single collector)
- R3.1 The system SHALL run exactly one telemetry collector per host that owns all blocking
  device I/O on a dedicated thread and publishes immutable snapshots.
- R3.2 All decision logic and all consumers SHALL read telemetry from the collector; no subsystem
  SHALL spawn its own VRAM/CPU sampler.
- R3.3 Telemetry SHALL include per-device VRAM free/total/reserved, GPU utilization, per-process
  VRAM, CPU per-core, RAM, disk, thermal, and power/battery.
- R3.4 Telemetry snapshots SHALL be timestamped and SHALL expose staleness; decisions on stale
  telemetry SHALL be flagged.
- R3.5 Telemetry SHALL retain a bounded ring buffer of history for diagnostics (§3.10).

### 3.4 Model lifecycle (all model types)
For each of: Local LLM, Cloud LLM, STT, TTS, Vision, OCR, Embeddings, Image:
- R4.1 The system SHALL define discovery, load, warm, cool, unload, evict, swap, reuse, and
  recovery for each model type through a uniform `ModelLifecycle` contract.
- R4.2 WHEN a model is requested, the authority SHALL choose residency (VRAM/RAM/disk/unloaded)
  and device deterministically from the active plan.
- R4.3 The system SHALL keep at most one canonical loader per model type (resolve the dual
  embedding engines in §0.2.5 to one primary with an explicit fallback).
- R4.4 WHEN a model load fails, the system SHALL retry with bounded backoff and SHALL surface a
  structured, user-visible failure with cause and remedy.
- R4.5 Model warm/cool transitions SHALL preserve conversational state (KV slot save/restore)
  where the runtime supports it, best-effort otherwise, and SHALL report which occurred.
- R4.6 Idle unload (cost saving) SHALL be policy-controlled, predictable, and SHALL pre-warm
  before the next foreground turn whenever activity is predicted.

### 3.5 GPU/VRAM management
- R5.1 The authority SHALL reserve a configurable VRAM safety margin per device and SHALL never
  admit a plan that would breach it.
- R5.2 The authority SHALL support co-residency budgets (e.g., LLM + image) when device capacity
  proves it fits, instead of a hardcoded tier rule.
- R5.3 WHEN VRAM pressure crosses a yield threshold, the authority SHALL choose the least-disruptive
  remedy in priority order: (a) reclaim idle/background residency, (b) shrink context/batch,
  (c) downshift GPU layers, (d) evict to RAM, (e) route to cloud — and SHALL prefer remedies that
  do NOT interrupt an active foreground stream.
- R5.4 The authority SHALL debounce pressure (EMA + dwell + hysteresis) to avoid thrash and SHALL
  rate-limit transitions per device.
- R5.5 VRAM defragmentation/reclaim SHALL be schedulable and SHALL be observable.

### 3.6 Scheduling, priority, contention
- R6.1 The authority SHALL classify every request into a priority class:
  `Interactive-Foreground > Realtime-Voice > Interactive-Background > Batch > Maintenance`.
- R6.2 Realtime-Voice (wake/STT/TTS turn) SHALL be guaranteed low-latency admission and SHALL be
  preemption-protected during an active utterance.
- R6.3 WHEN a higher class needs a resource held by a lower class, the authority SHALL preempt the
  lower class gracefully (checkpoint, then reclaim) within a bounded deadline.
- R6.4 The authority SHALL enforce fairness within a class (no starvation) and SHALL expose queue
  position to consumers.
- R6.5 CPU scheduling SHALL set thread budgets/affinity per consumer and SHALL detect CPU spikes.

### 3.7 CPU / RAM / Disk
- R7.1 The authority SHALL track CPU usage and SHALL cap inference thread counts under contention.
- R7.2 The authority SHALL monitor RAM pressure and SHALL refuse/relocate loads (e.g., drop
  `--mlock`) that would risk OOM/freeze.
- R7.3 The authority SHALL manage disk for model cache, temp, and logs with quotas, GC, and
  low-space protection.

### 3.8 Cloud & hybrid
- R8.1 Cloud endpoints SHALL be modeled as resource pools with capacity, latency, cost, and
  privacy attributes inside the same `DeviceTable`.
- R8.2 The authority SHALL place any model type local, cloud, or hybrid based on the active plan’s
  privacy/latency/cost/power constraints.
- R8.3 WHEN local capacity is insufficient, the authority SHALL fail over to cloud with explicit
  user-visible notice and SHALL fail back when local recovers (no sticky degradation without notice).

### 3.9 User experience
- R9.1 The user SHALL NEVER experience an unexplained model-unavailable, shutdown, sleep, unload,
  or "optimizing GPU layers" interruption.
- R9.2 WHEN any resource action affects the user (swap, evict, fallback, queue wait), the system
  SHALL surface a clear, human-readable status with cause, expected duration, and remedy.
- R9.3 The system SHALL NOT cancel an in-flight foreground response to perform a non-emergency
  resource action; such actions SHALL be deferred to a turn boundary.
- R9.4 WHEN an emergency (true OOM risk) forces interruption, the system SHALL explain it, preserve
  context, and auto-resume.
- R9.5 Background resource work SHALL be visible on demand (a "what is KRIA doing" panel).

### 3.10 Observability & self-diagnosis
- R10.1 The system SHALL answer, with concrete evidence, "why" for each: CPU spike, GPU spike,
  VRAM leak, model unload, swap, STT slowdown, TTS failure, image-gen failure, wake-word failure,
  daemon crash.
- R10.2 The system SHALL correlate every user-visible event to the telemetry window + decision
  journal entry that caused it.
- R10.3 The system SHALL run anomaly detectors for: CPU spike, GPU spike, VRAM leak, RAM leak,
  resource starvation, hung model, deadlock, daemon crash, infinite retry, thermal throttle —
  and SHALL emit a root-cause hypothesis with evidence.
- R10.4 The system SHALL expose a user-facing troubleshooting view and a machine-readable
  diagnostics bundle export.

### 3.11 Daemons
- R11.1 The system SHALL define supervised daemons: Core, Voice, Wake, GPU Monitor, Health Monitor,
  Extension Host.
- R11.2 Each daemon SHALL have explicit responsibilities, isolation boundary, IPC contract, health
  contract, and crash-recovery (auto-restart with backoff + circuit breaker).
- R11.3 Daemon crash SHALL NOT take down the host process; the supervisor SHALL restart and SHALL
  surface the incident.
- R11.4 The Wake daemon SHALL remain live (split tap) during GPU swaps so "Hey Ria" never dies.

### 3.12 Reliability & safety
- R12.1 The authority SHALL be crash-recoverable: on restart it SHALL reconcile real device state
  vs journaled leases and reclaim orphans.
- R12.2 Every irreversible/destructive reclaim (process kill, model evict) SHALL be logged with
  before/after evidence and SHALL be reversible where possible (checkpoint).
- R12.3 The system SHALL fail open to a deterministic safe default (CPU/cloud) — never into a hang
  or an unbounded retry loop.
- R12.4 No decision path in the control plane SHALL depend on an LLM (§3.13).

### 3.13 Determinism vs LLM
- R13.1 All admission, placement, eviction, and scheduling decisions SHALL be deterministic
  (rule/threshold/cost-model based).
- R13.2 An LLM MAY be used only for non-blocking, explainability/summarization of diagnostics, and
  only when a deterministic explanation is insufficient; it SHALL never gate a resource decision.

### 3.14 Workload Prediction Engine (WPE) — added Pass 1 (F1.1)
- R14.1 The system SHALL derive deterministic prewarm hints from UI/runtime signals (panel focus,
  prompt typing, file drop, mic open, workflow start) and SHALL pass them to the RA as advisory hints.
- R14.2 Prewarm SHALL be speculative, revocable, and budget-capped: it SHALL only consume free
  headroom, SHALL NEVER evict a higher-or-equal priority consumer, and SHALL auto-cool on confidence
  decay or TPPE/battery veto (F2.1).
- R14.3 WPE SHALL NEVER gate or alter an admission decision; it only changes residency warmth.

### 3.15 Session Intent Profiles (SIP) — added Pass 1 (F1.2)
- R15.1 The system SHALL classify the active session mode deterministically:
  `Coding | Voice | Image | Automation | Research | Idle | Mixed`.
- R15.2 SIP SHALL apply hysteresis + dwell + confidence; a minority workload SHALL NOT flip the
  profile (F2.2).
- R15.3 SIP output SHALL bias Planner cost and residency preference only (advisory); it SHALL NEVER
  issue hard evict/load commands.

### 3.16 Resource Forecasting Engine (RFE) — added Pass 1 (F1.3)
- R16.1 The system SHALL forecast near-term resource pressure (e.g., "VRAM exhaustion in N s",
  "RAM in N s", "thermal saturation approaching") from EMA-smoothed telemetry slopes with a
  confidence band (F2.3).
- R16.2 Forecasts SHALL be inputs to the deterministic remedy ladder, applied with lead time before
  the wall, and SHALL NEVER directly trigger a foreground interruption.

### 3.17 Thermal & Power Policy Engine (TPPE) — added Pass 1 (F1.4)
- R17.1 The system SHALL monitor thermal and power/battery state where sensors exist and SHALL
  predict throttle risk.
- R17.2 The system SHALL switch PolicyProfile by power source (AC/battery) and thermal headroom, and
  SHALL apply GPU duty-cycle/clock-aware budgets to avoid throttling.
- R17.3 WHEN sensors are absent, TPPE SHALL degrade to a conservative "thermal-unknown" profile and
  SHALL NEVER block on missing sensors (F4.4).

### 3.18 Hardware Capability Vector — added Pass 1 (F1.5)
- R18.1 The system SHALL describe hardware as a per-resource **Capability Vector**
  (cpu, gpu, vram, ram, thermal, power scores), not a single tier.
- R18.2 The coarse `HardwareTier` MAY remain as a display label but SHALL NOT be the basis for
  placement decisions.

### 3.19 Foreground Session Protection (hardened) — added Pass 1/3 (F1.6, F3.1)
- R19.1 All disruptive operations SHALL pass a single `ForegroundGuard::authorize` chokepoint that
  DENIES unless (a) emergency policy is active, or (b) the action is deferred to a turn boundary.
- R19.2 WHEN an emergency forces interruption of a foreground stream, the system SHALL perform a
  streaming checkpoint (flush partial output + KV save), show a labeled non-silent notice, and
  auto-resume from saved state within a hard bound (F3.1).
- R19.3 The guarantee SHALL be enforced structurally (no bypass path) and proven by event-trace test.

### 3.20 Autonomous Optimization Layer (AOL) — added Pass 3 (F3.5)
- R20.1 The system MAY learn user/time-of-day/workload patterns to improve prewarm hints and suggest
  PolicyProfiles.
- R20.2 AOL SHALL be strictly advisory: it SHALL write only to prewarm-hint / profile-suggestion
  stores, SHALL be subject to the same budget/veto rules as WPE, and SHALL have NO handle to the
  Scheduler/Planner admission API (enforced by module boundary).

### 3.21 Reliability: epoch fencing, journal integrity, backpressure — added Pass 2/3
- R21.1 Every lease SHALL carry the RA **epoch**; on RA restart the epoch SHALL increment and all
  pre-epoch leases SHALL be invalid; consumers SHALL revalidate epoch before each GPU op (F2.4).
- R21.2 Journal records SHALL be checksummed and versioned; recovery SHALL truncate at the first bad
  record and tolerate unknown future fields, refusing only on incompatible major version (F2.5, F5.2).
- R21.3 Admission queues SHALL be bounded per class with deadline-aware load-shedding (reject
  Batch/Maintenance first, with explicit UX) (F3.4).
- R21.4 Cloud Devices SHALL have circuit breakers + adaptive health (latency/error rate, honor
  `Retry-After`); the Planner SHALL avoid tripped pools (F3.3).

### 3.22 Operability: bypass, SLOs, shadow comparator — added Pass 4
- R22.1 The system SHALL provide a per-consumer **RA bypass kill-switch** that reverts to a static
  direct plan (full-GPU-or-CPU) with no authority (F4.1).
- R22.2 The system SHALL define SLOs (admission p99, voice latency, swaps/hr, OOM events = 0) and
  SHALL expose low-cardinality metrics (turn_id only in traces/journal, never metric labels) (F4.2).
- R22.3 Shadow mode SHALL include a comparator that replays identical telemetry to old-path and RA,
  asserts RA never over-commits and never adds a foreground interrupt the old path avoided, and
  gates cutover on a divergence report (F4.3).

### 3.23 Security & distributed-readiness — added Pass 5
- R23.1 Reconciler process termination SHALL require a capability token and SHALL target only
  RA-spawned PIDs (tracked at spawn) (F5.4).
- R23.2 Cloud failover SHALL honor `privacy_class`: Privacy-Strict data SHALL NEVER egress; it SHALL
  fail to CPU instead (F5.4).
- R23.3 RA request/plan/lease types SHALL be serializable and the authority SHALL sit behind a
  transport-agnostic trait, with `DeviceId::RemoteHost` reserved, so multi-host/cloud-burst is not
  foreclosed (no implementation now) (F5.3).

### 3.24 Residency Manager — added Final Pass (Gap 1)
- R24.1 The system SHALL have a single `ResidencyManager` that is the only executor of
  load/warm/cool/evict/swap/restore for every model type; Planner/Pressure/WPE SHALL request
  residency transitions through it, not call model lifecycles directly.
- R24.2 The `ResidencyManager` SHALL own the residency state machine per model and SHALL serialize
  transitions per model (one in-flight transition per model), emitting correlated events.
- R24.3 The `ResidencyManager` SHALL NOT make admission/placement decisions; it executes the
  residency target the RA already decided (preserves Planner/Scheduler ownership).

### 3.25 Resource Simulator — added Final Pass (Gap 2)
- R25.1 BEFORE the Scheduler commits an unload/swap/evict/migration/image-transition/cloud-failover,
  the system SHALL run a deterministic `simulate(action, snapshot)` that estimates VRAM impact, RAM
  impact, latency impact, disruption level, and risk level.
- R25.2 The simulation SHALL be a pure function (no I/O, no LLM) and its estimate SHALL be journaled
  with the resulting decision for explainability.
- R25.3 IF the simulation predicts a constraint breach (e.g., post-action free < hard limit), the
  Scheduler SHALL choose the next fallback instead of committing.

### 3.26 Session Ownership — added Final Pass (Gap 3)
- R26.1 At any instant the system SHALL assign exactly one **Foreground Owner** (the consumer the
  user is actively interacting with), zero-or-more **Interactive Owners** (admitted interactive work),
  and **Background Owners** (agents/batch/maintenance).
- R26.2 WHEN Coding, Voice, Image, Automation, and Background Agents are active simultaneously, the
  Foreground Owner SHALL receive priority + preemption protection; Background Owners SHALL yield
  first under pressure; no owner SHALL be starved (fairness floor per class).
- R26.3 Session ownership SHALL be derived deterministically from PriorityClass + SIP + UI focus and
  SHALL be advisory to scheduling weights only (no hard residency commands).

### 3.27 Multi-band Memory Budget — added Final Pass (Gap 5)
- R27.1 Each resource (VRAM, RAM, Disk) in the DeviceTable SHALL expose three bands derived from
  existing capacity/reservation/safety values (no duplicate accounting): **Soft Limit** (begin
  non-disruptive remedies), **Hard Limit** (refuse new admissions / shed), **Emergency Limit**
  (allow foreground-protecting emergency action).
- R27.2 The Pressure Engine SHALL map its existing yield/critical thresholds onto Soft/Emergency
  bands; the Hard Limit SHALL gate admission in the Scheduler.

### 3.28 Capability Registry — added Final Pass (Gap 6)
- R28.1 The system SHALL maintain a declarative `CapabilityRegistry` describing, per model
  (local/cloud LLM, Vision, STT, TTS, OCR, Embedding, Image): capabilities, quality tier, latency
  class, and resource profile.
- R28.2 The Planner SHALL select models by deterministic lookup against the registry; model
  selection SHALL be explainable and SHALL NEVER be performed by an LLM.

### 3.29 SLA Framework — added Final Pass (Gap 4)
- R29.1 The system SHALL define measurable SLAs with Target/Warning/Critical thresholds for: Voice
  (wake latency, STT latency, TTFA, TTS latency), Chat (first token, completion), Image (queue wait,
  generation start, completion), Automation (task start, completion), Cloud (failover, recovery).
- R29.2 SLA breaches SHALL be measured by the telemetry/observability layer, raised by the Health
  Monitor, and shown in Diagnostics with evidence.

### 3.30 Benchmark Framework — added Final Pass (Gap 7)
- R30.1 The system SHALL provide a benchmark harness supporting before/after comparison, regression
  detection, and hardware-class comparison, measuring VRAM/RAM/CPU/GPU/latency/throughput/queue
  delay/swap frequency/recovery time.
- R30.2 The benchmark harness SHALL be part of production validation (release gate).

---

## 4. Non-Functional Requirements

- N1 **Latency**: lease admission decision p99 ≤ 5 ms on warm path; foreground voice admission
  p99 ≤ 2 ms. Decision deadline (R1.5) ≤ 50 ms before safe fallback.
- N2 **Availability**: authority + telemetry uptime ≥ 99.9% of process lifetime; daemon
  auto-restart ≤ 2 s.
- N3 **Overhead**: telemetry + authority steady-state CPU ≤ 1 core-equiv on Medium tier; memory
  ≤ 150 MB.
- N4 **Portability**: Linux/Windows/macOS; NVIDIA/AMD/Apple/Intel/CPU-only; single→multi-GPU.
- N5 **Backward compatibility**: existing Tauri command/event names preserved; new events additive.
- N6 **Maintainability**: one telemetry stack, one lease type, one tier function, one embedding
  primary. Public contracts documented.
- N7 **Observability**: 100% of user-visible resource events carry a correlation id to a journal entry.
- N8 **Security**: process-kill and cloud-egress are gated through the existing safety policy layer.

---

## 5. Hardware Class Matrix (admission targets)

| Class | GPU | RAM | Default LLM residency | Voice | Image | Embeddings |
|---|---|---|---|---|---|---|
| Ultra-Low | none | 4–8 GB | CPU, tiny model or cloud | CPU subprocess | cloud only | CPU/ONNX small |
| Low | iGPU | 8–16 GB | CPU/partial or cloud | CPU | cloud | CPU |
| Medium | 3050/4050 | 16–32 GB | GPU partial offload | GPU/CPU | Tier-B swap | CPU/GPU |
| High | 4070/4080 | 32–64 GB | GPU full + vision | GPU | co-resident A | GPU |
| Enthusiast | 4090+ | 64–128 GB | GPU full + vision + headroom | GPU | co-resident S | GPU |
| Workstation | multi-GPU | enterprise | per-device placement | dedicated device | dedicated device | dedicated device |
| Cloud server | dedicated | n/a | pinned | n/a/cloud | dedicated | dedicated |

HRA SHALL select per-class defaults from a deterministic policy table, overridable by config.

---

## 6. Acceptance Criteria (system-level)

- A1 Exactly one `ResourceAuthority` instance grants all GPU/VRAM leases; grep proves no
  subsystem constructs its own `GpuLeaseManager`.
- A2 Exactly one telemetry collector; grep proves no subsystem spawns an independent VRAM/CPU sampler.
- A3 One tier-classification function; identical hardware yields identical tier everywhere.
- A4 No non-emergency action cancels an active foreground stream (verified by event-trace test:
  no `stream_interrupted` without an emergency flag during a foreground turn).
- A5 Every user-visible resource event carries a correlation id resolvable to a journal entry and
  a telemetry window.
- A6 Each "why" question in R10.1 answerable from the diagnostics bundle on demand.
- A7 Kill-restart of the authority reconciles device state and reclaims orphan leases with no
  leaked llama-server/ComfyUI processes.
- A8 Multi-GPU host places two consumers on two devices concurrently without contention error.
- A9 Cloud failover/failback occurs with explicit notice and no silent sticky degradation.
- A10 Control-plane decisions contain zero LLM calls (static analysis + runtime assert).
- A11 WPE prewarm never evicts a higher-or-equal class; speculative loads auto-cool; battery/thermal
  veto honored (chaos test).
- A12 SIP does not flip profile on a minority workload (hysteresis test); profile changes only bias
  cost, never hard-command residency.
- A13 RFE emits lead-time forecasts on smoothed series with bounded false-positive rate (replay test).
- A14 TPPE switches profile on AC/battery + avoids throttle on a thermal-limited laptop; degrades
  safely with no sensors.
- A15 Capability Vector drives placement; two machines with same coarse tier but different vectors
  get different, correct plans.
- A16 `ForegroundGuard` denies all non-emergency disruptive ops during a foreground turn; emergency
  path does streaming checkpoint + auto-resume (event-trace test).
- A17 AOL has no compile-time path to Scheduler/Planner admission API (module-boundary test).
- A18 Epoch fencing: after RA restart, a pre-epoch lease is rejected before any GPU op (split-brain
  test); no double-occupancy.
- A19 RA bypass kill-switch reverts a consumer to static plan with no authority involvement.
- A20 Privacy-Strict data never egresses on cloud failover (fails to CPU); Reconciler kills only
  RA-spawned PIDs with a capability token.
- A21 Every load/warm/cool/evict/swap/restore goes through `ResidencyManager`; grep proves no
  consumer calls a model lifecycle transition directly (Gap 1).
- A22 No unload/swap/evict/failover commits without a journaled `simulate()` estimate; a predicted
  hard-limit breach selects a fallback instead (Gap 2).
- A23 Under 5 simultaneous owners (Coding+Voice+Image+Automation+BackgroundAgent), foreground is
  protected, background yields first, and no owner starves (Gap 3 soak test).
- A24 DeviceTable exposes Soft/Hard/Emergency bands derived from existing values (no duplicate
  accounting); admission gates on Hard, remedies start at Soft (Gap 5).
- A25 Planner selects models by deterministic `CapabilityRegistry` lookup; selection is explainable;
  no LLM in selection (Gap 6).
- A26 SLA thresholds defined; breaches raised by Health Monitor and visible in Diagnostics (Gap 4).
- A27 Benchmark harness produces before/after + regression + per-hardware-class reports and gates
  releases (Gap 7).

---

## 7. Out of Scope

- New model architectures or inference kernels.
- Distributed multi-host scheduling (single-host authority; cloud pools are endpoints, not peers).
- Rewriting voice pipeline internals (HRA consumes them via the lifecycle contract).
