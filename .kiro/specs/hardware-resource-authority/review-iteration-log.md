# HRA Review Iteration Log (Adversarial Hardening)

Panel: Principal Systems Architect, Staff Infra, Distributed Systems, SRE, Performance, Platform,
AI Systems, Security, Reliability, FAANG Review Board. Input = V0 (`requirements.md`/`design.md`/
`tasks.md`). Method: break it, fix it, re-attack from scratch. 5 passes. Each flaw: Problem /
Impact / Severity / Failure Scenario / Recommendation. Fixes folded into the updated specs (new
requirement IDs R14–R23, design §14–§24, tasks 24–41).

Confidence score = panel's production-readiness estimate after that pass.

---

## PASS 1 — attack V0

### F1.1 No workload prediction → every first action pays cold start (Critical)
- Problem: V0 is purely reactive. RA only plans when a request arrives. Model load/warm latency
  (llama-server spawn, ComfyUI cold start, whisper load) is incurred on the critical path.
- Impact: user types image prompt → 8–30 s cold start; voice session start → STT load stall;
  "snappy local AI" promise fails.
- Failure scenario: user opens image panel, types, hits send → ComfyUI not warm → 20 s blank wait.
- Recommendation: add **Workload Prediction Engine (WPE)** — deterministic signals (UI focus,
  panel open, prompt typing, file drop, mic open) → prewarm hints to RA. Never gates admission.

### F1.2 No session intent → residency churns between modes (Critical)
- Problem: RA decides per-request with no notion of "user is in a coding session vs voice session."
- Impact: coding session repeatedly evicts the LLM for a one-off embedding, then reloads → thrash.
- Failure scenario: research session interleaves embeddings + LLM → repeated evict/reload swaps.
- Recommendation: add **Session Intent Profiles (SIP)** — classify active mode (Coding/Voice/Image/
  Automation/Research/Idle) deterministically; bias residency + scheduling per profile.

### F1.3 Telemetry is present-tense only → reacts too late (High)
- Problem: Pressure Engine acts on current EMA. By the time yield fires, VRAM may already be gone.
- Impact: emergency interrupts that could have been avoided with lead time.
- Failure scenario: KV cache grows 400 MB/s during long generation; yield fires at threshold but
  next allocation already OOMs before remedy completes.
- Recommendation: add **Resource Forecasting Engine (RFE)** — linear/EWMA slope projection →
  "VRAM exhaustion in N s"; feed remedies *before* the wall.

### F1.4 Thermal/power are fields, not a policy (High)
- Problem: V0 lists thermal/battery in telemetry but no engine acts on them.
- Impact: laptop throttles mid-session; battery drains; no AC/battery profile switch.
- Failure scenario: 4070 laptop on battery runs full-GPU LLM → 95°C throttle → latency 5×, fan roar.
- Recommendation: add **Thermal & Power Policy Engine (TPPE)** — predictive throttle avoidance,
  power budgets, battery/AC PolicyProfile switching, thermal headroom in Planner cost.

### F1.5 4-tier hardware model too coarse (High)
- Problem: single `HardwareTier` collapses CPU/GPU/RAM/thermal/battery into one axis.
- Impact: CPU-heavy-no-GPU box and GPU-heavy-low-RAM box can map to same tier → wrong plans.
- Failure scenario: 8-core/64 GB/no-GPU workstation tiered "Standard," denied parallelism it has.
- Recommendation: replace single tier with **Capability Vector** (per-resource scores) +
  derived profiles; keep tier as a coarse label only.

### F1.6 Foreground protection is a property, not enforced machinery (Critical)
- Problem: CP4 states the guarantee but nothing structurally prevents a code path from calling a
  disruptive remedy during a foreground turn.
- Impact: regression reintroduces "Optimizing GPU layers..." mid-answer.
- Failure scenario: new contributor adds an eager defrag on pressure → cancels active stream.
- Recommendation: add a **Foreground Guard** chokepoint — all disruptive ops must pass
  `ForegroundGuard::authorize(action)` which denies unless emergency or turn-boundary. Compile-time
  + runtime enforced; tested.

Confidence after Pass 1 fixes: 7.0/10.

---

## PASS 2 — re-attack (assume Pass 1 never happened, then add its fixes' own flaws)

### F2.1 WPE false-positive prewarm wastes VRAM/power (High)
- Problem: prewarm on "panel open" may load ComfyUI the user never uses → steals VRAM from LLM,
  drains battery.
- Impact: prediction harms the very resource it tries to optimize.
- Failure scenario: user browses image panel to look, not generate → ComfyUI warmed, LLM evicted.
- Recommendation: prewarm must be **speculative + revocable + budget-capped**: only into free
  headroom, never evicting a higher class; auto-cool on confidence decay; TPPE/battery veto.

### F2.2 SIP misclassification flips residency destructively (High)
- Problem: a single embedding call inside a coding session could flip SIP → Research → evict LLM.
- Impact: misclassification = thrash, the bug SIP was meant to fix.
- Failure scenario: coding session does a doc search → SIP flips → LLM cooled → next code turn cold.
- Recommendation: SIP uses **hysteresis + dwell + confidence**; profile changes are advisory to the
  Planner cost (bias), never hard residency commands; minority workloads don't flip the profile.

### F2.3 Forecasting on noisy telemetry → false alarms (Medium)
- Problem: naive slope projection on spiky VRAM → frequent false "exhaustion imminent."
- Impact: premature remedies, churn.
- Recommendation: forecast on EMA-smoothed series + require sustained slope + confidence band;
  forecasts are inputs to a deterministic remedy ladder, not direct triggers.

### F2.4 No split-brain protection across daemons (Critical)
- Problem: RA is in Core process, but GPU Monitor/Voice are separate daemons. On Core restart, a
  Voice daemon may still hold a stale lease belief and run whisper on GPU while RA re-grants it.
- Impact: two consumers on one GPU → OOM/corruption (the exact class HRA exists to kill).
- Failure scenario: Core crashes mid-turn; supervisor restarts Core; Voice daemon keeps its lease;
  Reconciler re-grants LLM → both touch GPU.
- Recommendation: leases are **epoch-fenced**. RA holds a monotonically increasing epoch persisted
  in the journal. Every lease carries the epoch; on RA restart epoch++ and all pre-epoch leases are
  invalid. Consumers must revalidate lease epoch before each GPU op (cheap atomic read).

### F2.5 Journal is single-writer durable state → corruption risk (High)
- Problem: append-only journal on disk; torn write on power loss → unparseable tail → recovery fails.
- Impact: RA can't reconstruct state → fails closed or mis-reclaims.
- Recommendation: journal entries are **checksummed records**; recovery truncates at first bad
  record (last-good wins); periodic compacted snapshot + tail; fsync policy bounded.

Confidence after Pass 2 fixes: 8.0/10.

---

## PASS 3 — re-attack

### F3.1 Preemption checkpoint deadline can still hang a foreground turn (High)
- Problem: emergency reclaim asks holder to checkpoint, waits `preempt_deadline`. If the foreground
  LLM is the holder and is mid-token, "graceful" still stalls the user.
- Impact: emergency UX is a freeze, not a graceful pause.
- Recommendation: foreground holders get a **streaming checkpoint**: flush partial response to UI +
  KV slot save, show labeled "freed memory, resuming," auto-continue from saved KV. Bounded to a
  hard wall; if exceeded, abort-with-resume, never silent.

### F3.2 Multi-GPU NUMA / P2P not modeled (Medium)
- Problem: DeviceTable treats GPUs as independent; ignores PCIe topology, NVLink, host-NUMA pinning.
- Impact: cross-GPU model placement may be slow; CPU threads pinned to wrong NUMA node.
- Recommendation: Device carries **topology metadata** (numa_node, p2p_peers, pcie_gen); Planner
  cost adds affinity penalty; out-of-scope for execution split but extension point reserved.

### F3.3 Cloud pool capacity is a guess → failover storms (High)
- Problem: cloud "capacity/quota" is static config; real rate limits/outages unknown until 429/5xx.
- Impact: failover routes everything to a throttled provider → cascading failure.
- Failure scenario: local OOM → all turns failover to one cloud key → 429 → retries → worse.
- Recommendation: cloud Devices get **circuit breakers + adaptive health** (observed latency/error
  rate) feeding DeviceLive; Planner avoids tripped pools; honor `Retry-After`.

### F3.4 No backpressure on request admission queue (Medium)
- Problem: under sustained overload, the per-device queue grows unbounded.
- Impact: memory growth, stale requests admitted late.
- Recommendation: bounded queues per class with **load-shedding** (reject Batch/Maintenance first,
  with explicit UX), deadline-aware dropping of expired requests.

### F3.5 Autonomous optimization could drift into control (Critical security/safety)
- Problem: a learning layer that "optimizes" risks influencing admission → non-deterministic core.
- Impact: violates R13 (deterministic decisions); unpredictable production behavior.
- Recommendation: **Autonomous Optimization Layer (AOL)** is strictly advisory: it may only adjust
  *prewarm hints* and *PolicyProfile suggestions*, write to a separate store, and is gated by the
  same budget/veto rules as WPE. It can NEVER call Scheduler/Planner. Enforced by module boundary
  (AOL has no handle to RA admission API).

Confidence after Pass 3 fixes: 8.6/10.

---

## PASS 4 — re-attack (production/SRE/perf focus)

### F4.1 No rollback / kill-switch for the RA itself (High)
- Problem: cutover (tasks 12–16) routes all consumers through RA. If RA misbehaves in prod, no fast
  path back to the old behavior.
- Impact: a bad RA build bricks all AI features.
- Recommendation: **RA bypass mode** — a runtime kill-switch that reverts each consumer to a
  static-plan (full-GPU-or-CPU) direct path, no authority. Per-consumer granularity. Config + UI.

### F4.2 Observability lacks SLO/alerting + cardinality control (Medium)
- Problem: events/journal exist but no defined SLOs, no metric aggregation, unbounded label
  cardinality (turn_id as a metric label would explode).
- Impact: can't alert; metrics backend blows up.
- Recommendation: define **SLOs** (admission p99, voice latency, swap count/hr, OOM events=0);
  metrics are low-cardinality counters/histograms; turn_id only in traces/journal, never metric labels.

### F4.3 Shadow mode can't prove correctness without a comparator (High)
- Problem: Task 10 "shadow mode" logs decisions but there's no defined oracle to say a shadow
  decision was *better/safe*.
- Impact: false confidence; cutover on vibes.
- Recommendation: **Shadow comparator**: replay identical telemetry to old-path and RA; assert RA
  never plans an over-commit, never plans a foreground interrupt the old path avoided; produce a
  divergence report with thresholds gating cutover.

### F4.4 Thermal/power on desktop with no sensors → TPPE blind (Medium)
- Problem: many desktops expose no battery and partial thermal sensors; TPPE may misfire.
- Recommendation: TPPE degrades gracefully to "thermal-unknown" profile (conservative GPU duty
  cycle only if util+time heuristic suggests heat), never blocks on missing sensors.

### F4.5 Embedding worker pool duplicates model memory (Medium)
- Problem: N embedding workers each holding a model copy multiplies RAM/VRAM.
- Recommendation: shared model weights + per-worker session/context, or a single-model
  batched-queue with pipelining; size by capability vector, not fixed N.

Confidence after Pass 4 fixes: 9.1/10.

---

## PASS 5 — re-attack (FAANG board, find anything left)

### F5.1 Frontend not designed for HRA → users still blind (High)
- Problem: V0 mentions a status event but no full UX architecture (dashboard, explainability,
  forecasting, recovery, diagnostics views).
- Impact: the "never surprise the user" requirement has no surface to deliver on.
- Recommendation: full **Frontend/UX architecture** (see `frontend-ux-spec.md`): Resource Dashboard,
  Explainability UI, Session Awareness, Forecasting UI, Recovery UI, Diagnostics export.

### F5.2 No migration data-compat for journal across versions (Medium)
- Problem: journal format will evolve; old journal on upgrade may break recovery.
- Recommendation: journal records are **versioned**; reconciler tolerates unknown future fields and
  refuses only on incompatible major version (then safe cold reconcile from live device state).

### F5.3 Distributed future foreclosed by in-proc-only contracts (Medium)
- Problem: RA API is in-process Rust; cloud bursting / multi-host / remote exec would require a
  rewrite.
- Recommendation: define RA contracts as **transport-agnostic** (request/plan/lease are
  serializable; authority behind a trait that could be a local impl today, a gRPC client tomorrow).
  Add `DeviceId::RemoteHost` + `Execution` extension point. No implementation now; no corner painted.

### F5.4 Security: process-kill reclaim + cloud egress need authz, not just policy (High)
- Problem: Reconciler kills processes and cloud failover egresses data; both reference "safety
  policy" but no explicit privilege boundary or PII-egress rule.
- Impact: a compromised consumer could trigger kills; privacy-strict data could egress on failover.
- Recommendation: Reconciler kill requires **capability token** + targets only RA-spawned PIDs
  (tracked at spawn); cloud failover honors `privacy_class` — Privacy-Strict data NEVER egresses,
  fails to CPU instead. Audited.

### F5.5 No chaos/soak acceptance for the new engines (Medium)
- Problem: WPE/SIP/RFE/TPPE/AOL added but no adversarial test gates.
- Recommendation: add chaos/fault-injection + soak acceptance per engine (false-positive bounds,
  oscillation bounds, veto correctness).

Confidence after Pass 5 fixes: 9.6/10. → meets ≥9.5 gate. Stop (further passes negligible benefit).

---

## Residual risks (accepted, tracked)
- In-process whisper/piper bindings still deferred (contract ready; subprocess works).
- Distributed multi-host execution not implemented (extension points reserved, F5.3).
- TPPE accuracy bounded by sensor availability on diverse desktop hardware (F4.4 mitigation).
- AOL learning quality improves over time; cold-start = neutral (no harm, advisory-only).

## Confidence trajectory
Pass1 7.0 → Pass2 8.0 → Pass3 8.6 → Pass4 9.1 → Pass5 9.6.

---

## PASS 6 — Final hardening (gap closure only, preserve architecture)

Constraint: no redesign of RA/Planner/Scheduler/Pressure/DeviceTable/Journal/Reconciler/Policy/
Telemetry/ForegroundGuard/WPE/SIP/TPPE/RFE/AOL. Only fill named gaps with minimal additive extensions.

### F6.1 Residency ownership distributed (Medium) — Gap 1
- Problem: load/warm/cool/evict/swap/restore reachable from Planner, lifecycle, Pressure → race +
  large test surface. Impact: overlapping transitions, hard to observe. Fix: `ResidencyManager`
  single executor wrapping existing `ModelLifecycle`; one in-flight transition per model.
- Residual: requires routing all call sites through it (grep gate, Task 42). Decision: ADOPT minimal.

### F6.2 Blind disruptive commits (Medium) — Gap 2
- Problem: Scheduler committed evict/swap/failover without predicting impact. Fix: pure
  `simulate(action,snapshot)->Estimate`, journaled, pre-commit gate. Why not rejected: cheap, pure,
  preserves determinism, prevents regret swaps. Residual: estimate accuracy → calibrated by Task 48.

### F6.3 Concurrency ownership ambiguity (Medium) — Gap 3
- Problem: 5 simultaneous consumers had no named Foreground Owner → thrash/starvation risk. Fix:
  `SessionOwnership` view (fg/interactive/bg) advisory to scheduler weights + fairness floor. Reuses
  SIP hysteresis. Residual: none material.

### F6.4 Single safety margin too coarse (Medium) — Gap 5
- Problem: one margin couldn't express start-reclaim vs refuse vs emergency. Fix: derive
  Soft/Hard/Emergency bands from existing values (no new counters); Pressure maps yield→Soft,
  critical→Emergency; admission gates Hard. Residual: none (derived view, Property 18).

### F6.5 No model capability metadata (Medium) — Gap 6
- Problem: future many-model selection needs deterministic capability lookup. Fix: declarative
  `CapabilityRegistry`; Planner pure lookup; no LLM selection. Residual: registry/discovery drift →
  reconcile at startup.

### F6.6 SLOs too coarse for user ops (Medium) — Gap 4
- Problem: system SLOs lacked per-operation Target/Warning/Critical + breach surface. Fix: `SlaTable`
  wired to Health Monitor + Diagnostics. Residual: thresholds are initial; Benchmark calibrates.

### F6.7 No resource benchmark/regression gate (Medium) — Gap 7
- Problem: Tier-0 system with no objective before/after or regression detection. Fix: extend
  `kria-eval` with benchmark mode + per-hardware-class reports as a release gate. Residual: number
  stability → warmup + repeats + statistical bounds.

All seven are Medium, additive, and preserve every protected component. No new Critical/High
introduced. Confidence after Pass 6: 9.7/10.

## Confidence trajectory (updated)
Pass1 7.0 → Pass2 8.0 → Pass3 8.6 → Pass4 9.1 → Pass5 9.6 → Pass6 9.7.
