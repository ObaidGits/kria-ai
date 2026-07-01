# KRIA HRA — Runtime Call Graph (verified from repository)

> Standalone. Evidence: file:line. Default = `KRIA_HRA_ENFORCE` unset (shadow).

## Telemetry sources (live)
```
TelemetryHub (resource/telemetry_hub.rs)
  └─ build_profiler()  [platform/vram.rs:412]
       └─ NVML(feature) → ROCm → CliVramProfiler(nvidia-smi) → Null   [C1]
  ← consumers: shared GPU lease (SharedResourceTelemetry), HRA snapshot loop,
    image VRAM barrier, agent loop_engine free-VRAM read.

Orchestrator TelemetryActor (llm/orchestrator/telemetry.rs:368 create_telemetry_actor)
  └─ NVML(feature) → CliBlockingSampler(nvidia-smi) → RAM
  ← consumers: LLM sizing (strategy), GPU watchdog.   [used at mod.rs:615]
```
Two samplers (different layers). `HubTelemetry` (telemetry.rs:78) exists but is **unused** (dead).

## Default (shadow) — GPU consumer flow
```
Consumer (Image/Vision/STT/TTS)
  └─ acquire_guard_gated(owner, …)            [resource/gpu_lease.rs:435]
       └─ global_hra().is_shadow_only() == true  → legacy branch
            └─ acquire_guard → GpuLeaseManager (single-holder) → GPU
LLM:
  claim_l1_lease → GpuLeaseManager::default() (private)        [mod.rs:912/~724]
  + GPU watchdog scale-up: calculate_target_params_prod (CUDA reserve, C4)
       → gpu_in_cooldown() gate (C4) → execute swap → on fail recover to CPU (C3 event)
HRA: observes via TelemetryHub; watchdog logs advisory verdict (no veto in shadow).
```

## Enforce (`KRIA_HRA_ENFORCE=1`) — GPU consumer flow
```
Consumer
  └─ acquire_guard_gated → is_shadow_only()==false
       └─ global_hra().admit_gpu(req, Hot)        [service.rs:175]
            └─ CoResidencyManager::acquire         [co_residency.rs]
                 └─ LocalAuthority::request_on_gpu → Planner → Scheduler → DeviceTable
                      → AdmissionGuard::Granted → consumer runs (executes own load)
LLM: reconcile_l1_lease holds an HRA InteractiveFg admission while GPU-resident [mod.rs:984]
```

## Sizing path (both modes)
```
strategy::calculate_target_params_prod (8 sites)      [C4]
  └─ + cuda_runtime_reserve_mb()  → calculate_target_params (pure)
watchdog scale-up gated by !gpu_in_cooldown()          [gpu_watchdog.rs:342]
swap fail → record_spawn_failure (ceiling+cooldown) → recover CPU + emit swap_failed
```

## UI swap state machine (C3)
```
swap_started → isSwapping=true (+120s safety timeout)   [app.ts:4562]
  ├─ swap_completed → clearSwap()                        [app.ts:~4573]
  ├─ swap_failed    → clearSwap()                        [app.ts:4580]
  ├─ error          → clearSwap()                        [app.ts:4585]
  └─ timeout(120s)  → setIsSwapping(false)               [app.ts:4568]
```

## Key differences shadow vs enforce
- Shadow: legacy `GpuLeaseManager` is the executor; HRA observes only; `admit_gpu` returns
  `AdmissionGuard::Shadow` (inert).
- Enforce: `admit_gpu` routes through CoResidency/Planner/Scheduler/DeviceTable; legacy lease bypassed
  for gated consumers; LLM holds an HRA admission tied to residency.
