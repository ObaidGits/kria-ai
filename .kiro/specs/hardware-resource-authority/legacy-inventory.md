# KRIA HRA — Legacy Inventory (verified from repository)

> Standalone. Classification of legacy/duplicate resource code. No code changed.

| Symbol | Location | Classification | Evidence / why |
|---|---|---|---|
| `GpuLeaseManager` | `resource/gpu_lease.rs` | **Runtime Active (shadow executor + rollback)** | `global_gpu_lease()` shared by image/vision/voice via `acquire_guard_gated` shadow branch; LLM private `GpuLeaseManager::default()` (`mod.rs:~724`). The executor in default mode. Unsafe to delete. |
| Orchestrator `TelemetryActor` / `create_telemetry_actor` | `llm/orchestrator/telemetry.rs:368` | **Runtime Active** | LLM sizing + watchdog telemetry (`mod.rs:615`). Has nvidia-smi CLI fallback. Unsafe to delete. |
| `CliBlockingSampler` / `CliTelemetry` | `llm/orchestrator/telemetry.rs:204/409` | **Runtime Active (actor backend) / Test-compat** | Used inside the actor; `CliTelemetry` is a legacy async wrapper kept "for test compat" (its doc). `CliTelemetry` itself: NOT VERIFIED to have a production caller — likely test-only. |
| `HubTelemetry` | `llm/orchestrator/telemetry.rs:78` | **DEAD (unused)** | Its only production caller was reverted during the LLM fix; `mod.rs:615` now uses `create_telemetry_actor`. Safe to delete (or re-wire). See dead-code-report.md. |
| `NullProfiler` | `platform/vram.rs:300` | **Runtime Active (fallback)** | Last rung of `build_profiler`; reached only when no GPU telemetry. Keep. |
| `create_cuda_telemetry` | `telemetry.rs:477` | **Already removed** | Comment notes removal (HRA Task 17). |
| `vision_automation.rs` stub `GpuLeaseManager` | `tools/vision_automation.rs:525` | **Already removed (comment only)** | Stub deleted; comment remains. |
| Legacy single-holder semantics in `GpuLeaseManager` | `resource/gpu_lease.rs` | **Runtime Active (rollback)** | Co-residency exists in HRA control plane; legacy lease is the shadow/rollback path. Unsafe to delete pre-soak. |

## Schedulers / planners / residency
- **One** Scheduler (`resource/authority/scheduler.rs`), **one** Planner (`planner.rs`), **one**
  DeviceTable (`device_table.rs`), **one** ResidencyManager (`residency_manager.rs`), **one**
  CoResidencyManager (`co_residency.rs`). No duplicate scheduler/planner found.
- The legacy `GpuLeaseManager` has its own internal state machine (Idle/Held/Recovering/Degraded) but
  **no** separate planner/scheduler — it is a lease arbiter, not a duplicate scheduler.

## Deletion readiness
- **Unsafe to delete now:** `GpuLeaseManager` (+ private LLM instance), orchestrator `TelemetryActor`.
  Reason: they are the default-mode (shadow) executors and the rollback path. Deletion is gated on the
  enforce-mode GPU soak proving the HRA path.
- **Safe to delete now (headless):** `HubTelemetry` (dead). Optionally `CliTelemetry` wrapper if grep
  confirms test-only (NOT VERIFIED here — left in place).
