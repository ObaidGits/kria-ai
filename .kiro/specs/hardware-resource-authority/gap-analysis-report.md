# HRA Gap Analysis Report

Maps the gap between the **current codebase (today)** and the **target HRA architecture**, with the
task that closes each gap. "Today" references are from the forensic analysis in `requirements.md` §0.

| # | Gap (today → target) | Evidence (today) | Closed by | Severity |
|---|---|---|---|---|
| G1 | 4+ independent lease managers → one authority | `mod.rs`, `image/orchestrator.rs`, `voice_runtime_helpers.rs`, `runtime.rs` | Tasks 4–6,10,12–16 | Critical |
| G2 | 3 telemetry stacks → one collector | `orchestrator/telemetry.rs`, `platform/vram.rs`, `resource/telemetry.rs` | Task 3, 17 | High |
| G3 | Dead reconcile (telemetry never set) → live reconcile + epoch | `set_resource_telemetry` uncalled | Tasks 9, 26 | High |
| G4 | Stub duplicate `GpuLeaseManager` → deleted | `tools/vision_automation.rs` | Task 15 | Medium |
| G5 | Tier OR/AND divergence → one tier fn + capability vector | `detect.rs` vs `hardware_profiler.rs` | Tasks 1, 24 | High |
| G6 | Mid-stream cancel → Foreground Guard + deferral | `gpu_watchdog.rs cancel_streams` → `ChatView.tsx` | Tasks 25, 12 | Critical |
| G7 | Reactive only → predictive (WPE/SIP/RFE) | no prediction today | Tasks 30,31,32 | High |
| G8 | Thermal/battery unused → TPPE | telemetry fields only | Task 33 | High |
| G9 | Global embedding mutex → worker pool | `routing/embed.rs OnceCell<Mutex>` | Task 16 | Medium |
| G10 | Two embedding engines → one primary + fallback | fastembed + ONNX | Task 16 | Medium |
| G11 | Silent sticky cloud degrade → explicit failover/failback | `image session_degraded` | Tasks 13, 29 | High |
| G12 | No crash recovery of orchestrator → journal + reconciler | no journal today | Tasks 8,9,27 | High |
| G13 | Single GPU (device 0 hardcoded) → multi-GPU DeviceTable | NVML index 0 everywhere | Tasks 4, 21 | High |
| G14 | No "why" surface → Explainability + Diagnostics UI | events not correlated | Tasks 8, 40 | High |
| G15 | No anomaly root-cause → detectors | none today | Task 18 | Medium |
| G16 | No bypass/rollback → kill-switch | none today | Task 35 | High |
| G17 | No shadow validation → comparator | none today | Task 37 | High |
| G18 | Process kill ungated → reclaim authz + PID registry | reconcile cleanup kills by sysinfo | Task 38 | High |
| G19 | Privacy not enforced on egress → privacy-bounded failover | cloud fallback unconditional | Task 38 | High |
| G20 | In-proc-only → distributed extension points | Rust in-proc APIs | Task 39 | Medium |
| G21 | No daemon supervision/isolation → supervisor + circuit breakers | partial today | Task 19 | Medium |
| G22 | No SLOs/metrics discipline → SLOs + low-cardinality metrics | tracing only | Task 36 | Medium |
| G23 | Dead code (AudioFreezeGuard v1, deprecated telemetry) → removed | `image/swap.rs`, `telemetry.rs` | Task 17 | Low |

## Coverage
- All Critical gaps (G1, G6) closed in Phases 1–2 + hardening.
- All High gaps mapped to concrete tasks with acceptance criteria.
- No gap left without an owning task. Untracked-gap count: 0.

## Sequencing risk
- G6 (foreground) depends on Task 25 landing with Task 12 (LLM cutover) — must ship together.
- G1 deletions (Phase 3) only after RA proven (shadow + epoch tests) — enforced in `tasks.md` Notes.
- G18/G19 (security) gate before enabling automatic reclaim/failover in production.

## Final-pass gap closures (hardening, not redesign)

| # | Gap (target refinement) | Why existing was insufficient | Closed by | Severity |
|---|---|---|---|---|
| G24 | Residency ownership distributed → single `ResidencyManager` | `ModelLifecycle` had ops, no single owner; 3 callers could race | Task 42 | Medium |
| G25 | Disruptive actions committed blind → pre-commit `simulate()` | Planner ranks placements, didn't pre-flight transition cost | Task 43 | Medium |
| G26 | Concurrency ownership ambiguous → Session Ownership view | SIP gave mode, not a named Foreground Owner across subsystems | Task 44 | Medium |
| G27 | Single safety margin → Soft/Hard/Emergency bands | yield/critical lived only in Pressure, not admission-gating DeviceTable | Task 45 | Medium |
| G28 | No capability metadata → Capability Registry | `ModelDescriptor` lacked capability/quality/latency for selection | Task 46 | Medium |
| G29 | Coarse SLOs → per-operation SLA Target/Warning/Critical | §16.2 SLOs not per user-facing op, no breach surface | Task 47 | Medium |
| G30 | No resource benchmark → Benchmark Framework | `kria-eval` had E2E, no resource-efficiency/regression gate | Task 48 | Medium |

All final-pass gaps are Medium, additive, and extend existing components (no protected component
redesigned). Untracked-gap count remains 0.
