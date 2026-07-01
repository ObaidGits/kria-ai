# KRIA HRA — Duplication Report (verified from repository)

> Standalone. No code changed.

## Telemetry / VRAM sampling — DUPLICATED (2 live)
1. `TelemetryHub` (`resource/telemetry_hub.rs`) → `build_profiler()` (`platform/vram.rs:412`,
   now with `CliVramProfiler`). Consumers: shared GPU lease, HRA loop, image barrier, agent loop.
2. Orchestrator `TelemetryActor` (`llm/orchestrator/telemetry.rs:368`) → `CliBlockingSampler`
   (nvidia-smi). Consumer: LLM sizing + watchdog (`mod.rs:615`).

Classification: **Production duplicate, intentional (layer separation).** Both read the same device
via nvidia-smi/NVML. Authoritative for HRA decisions = `TelemetryHub`; authoritative for LLM sizing =
orchestrator actor. Not merged because folding the actor into the hub regressed LLM sizing on
`--no-default-features` (reverted during the LLM fix). Future unification possible but not required;
not a correctness bug (same source). 

Third reference: `agent/loop_engine/mod.rs:4839` calls `build_profiler()` directly only as a
**fallback** when `global_telemetry_hub()` is None — not a steady-state duplicate.

## nvidia-smi query logic — DUPLICATED (acceptable)
- `platform/vram.rs` `CliVramProfiler` (C1) and `llm/orchestrator/telemetry.rs` `CliBlockingSampler`
  both shell `nvidia-smi --query-gpu=memory.*`. **Intentional, layer-correct** (platform must not
  depend on the orchestrator layer). ~10 lines duplicated. Candidate for future shared helper; not a
  defect.
- `platform/detect.rs:256` `detect_gpu` also shells nvidia-smi (one-shot total VRAM at boot) — a third
  copy, but single-use detection, not steady-state sampling.

## Sizing logic — NOT duplicated
- One sizer: `strategy::calculate_target_params` (+ `_prod` wrapper). `tier_strategy.rs` derives the
  `ModelProfile` (per-layer estimates) — a different concern (profile derivation, not runtime sizing).
- `vram_budget.rs` computes visual-token caps — a different concern (image token budget), not GPU
  layer sizing.

## Scheduler / Planner / Residency / Journal / DeviceTable — NOT duplicated
- Single instance of each (see legacy-inventory.md). The legacy `GpuLeaseManager` is a lease arbiter,
  not a second scheduler/planner.

## Recovery — NOT duplicated (two distinct layers, by design)
- Lease recovery (`gpu_lease.rs` Recovering/Degraded) for the shared lease.
- HRA Reconciler (`reconciler.rs`) for journal/PID crash recovery.
These are different scopes (live lease vs crash replay), not duplicates.

## State machines — minor overlap
- Watchdog `WatchdogState` (Idle/Pressured/Recovering/Critical) and lease `GpuLeaseState`
  (Idle/Held/Recovering/Degraded) coexist. Different concerns (sizing pressure vs lease ownership).
  Not a true duplicate; both legitimately exist.

## Summary
- **Real duplicate to resolve (optional):** telemetry samplers (2) + nvidia-smi query (≤3 copies).
  Both acceptable today; unify later for maintainability.
- **No duplicate** scheduler, planner, residency manager, DeviceTable, journal, or GPU sizer.
