# KRIA HRA — Final Repository Audit (independent, evidence-based)

> No code changed. Independent verification of the C1–C7 implementation claims against the repository.
> "Verified from repository." = proven by file:line. "Rejected." = claim is false. NOT VERIFIED =
> no evidence found. Previous implementation reports treated as untrusted.

## PART 1 — Implementation claim verification

### C1 — Telemetry — ✅ VERIFIED
- `CliVramProfiler` exists: `platform/vram.rs:337` (`pub struct CliVramProfiler`), impl `VramProfiler`
  at `:358` using async `tokio::process::Command("nvidia-smi")` at `:361`. Verified from repository.
- `build_profiler` instantiates it: `vram.rs:412` chain NVML(`#[cfg(feature="nvidia")]`) → ROCm →
  **`CliVramProfiler::try_new()` (`:425`)** → `NullProfiler::new()` (`:430`). Verified.
- Unknown uses `total_mb == 0`: `CliVramProfiler::snapshot` returns `vendor: Unknown, total_mb: 0`
  on failure (`vram.rs:~370`); `NullProfiler` returns 0/Unknown. Test `null_profiler_is_unknown_not_zero_gpu` (`:535`). Verified.
- Can a runtime path still return fake 0 VRAM? On an NVIDIA host with a working driver, no — the CLI
  rung returns real values. **Residual:** if neither NVML feature nor `nvidia-smi` is present,
  `NullProfiler` returns 0 (correctly flagged Unknown via `total_mb==0`, handled by C2). NOT a blind
  spot — it is explicit Unknown.

### C2 — Reconcile — ✅ VERIFIED
- Unknown skips degradation: `resource/telemetry.rs:102` `if self.vram.total_mb == 0 { return Healthy }`
  before the `<200`/CriticalOomRisk check. Verified.
- Degraded auto-clears: `resource/gpu_lease.rs:217` (async acquire) + `:389` (`acquire_token`) reset
  `Degraded → Idle` on a fresh acquire. Verified.
- Can `GuardReleasedAwaitingTelemetry` still degrade the lease permanently? With C1 (real VRAM) +
  C2 (Unknown→Healthy + auto-clear), no permanent degrade path remains for Unknown telemetry. Tests
  `recover_with_unknown_telemetry_goes_idle_not_degraded`, `degraded_lease_auto_clears_on_acquire`
  (`gpu_lease.rs:1235+`). Verified.

### C3 — UI swap — ✅ VERIFIED
- Backend emits: `runtime.rs:2140` `KriaEvent::LlmSwapFailed → emit "orchestrator:swap_failed"`;
  `runtime.rs:2181` emits `orchestrator:error`. Verified.
- Frontend listens + resets: `app.ts:4580` `swap_failed` → `clearSwap()`; `:4585` `error` →
  `clearSwap()`; `:4568` 120s safety timeout auto-clears `isSwapping`. Verified.
- Any path leaves `isSwapping=true`? swap_completed/failed/error/timeout all clear it. No remaining
  stuck path found. Verified.

### C4 — GPU sizing — ✅ VERIFIED
- Production calls `_prod`: all 8 live call sites use `strategy::calculate_target_params_prod`
  (`gpu_watchdog.rs:314,333,400,487`; `mod.rs:628,1133,1347,1849`). Bare `calculate_target_params`
  appears only in `strategy.rs` tests. No old prod callsite remains. Verified.
- Reserve: `cuda_runtime_reserve_mb()` (`strategy.rs:68`, default 1024, `KRIA_CUDA_RESERVE_MB`),
  folded by `_prod` (`:83`). Verified.
- Cooldown: `server_manager.rs:151` `last_gpu_failure_ms`, stamped in `record_spawn_failure` (`:288`),
  `gpu_in_cooldown()` (`:303`); watchdog gate `!self.server.gpu_in_cooldown()` (`gpu_watchdog.rs:342`).
  Verified.
- Sizing/watchdog bypass? Emergency `execute_swap` also uses `_prod` (`:487`). No bypass found.

### C5 — DeviceTable — ✅ VERIFIED
- `collector.rs HostSnapshot::apply_to` iterates `self.gpus`; an empty (Unknown) snapshot performs no
  GPU `refresh_free`, preserving prior measured free. Test `empty_gpu_snapshot_preserves_prior_free`.
  Invalid telemetry cannot overwrite valid telemetry. Verified.

### C6 — Startup — ✅ VERIFIED
- `SharedToolIndex::empty()` (`routing/tool_index.rs`) + background `rebuild` in `runtime.rs:1107–1125`
  ("building in background (non-blocking startup, C6)"). The blocking `SharedToolIndex::new().await`
  is gone from `runtime.rs` (only present in tests). LLM no longer waits behind the ~11s index build.
  Verified. **Runtime-validation pending** (tool-routing degrades to lexical during the build window —
  not headlessly provable to be regression-free).

### C7 — CPU — ✅ VERIFIED
- `voice/stt.rs:384` `default_whisper_threads` = `(n.get()/2).clamp(1,4)` (was `clamp(1,8)`). Verified.
- Remaining heavy CPU at startup (ranked, from logs + code): (1) whisper warmup (~10s, now ≤4 threads),
  (2) tool-index embedding build (~11s, now **background** per C6), (3) embedding pool init, (4) app
  registry scan, (5) vision sidecar restart churn (env: missing fastapi). No new busy-wait introduced.

## PART 1 verdict
All seven claims **Verified from repository.** One correctness note (not a C-claim defect): see the
dead-code report — `HubTelemetry` is unused after the LLM-fix revert.

## Cross-cutting findings
- **Telemetry: 2 live samplers remain** (intentional, different layers): `TelemetryHub`
  (`build_profiler`→`CliVramProfiler`) for HRA/lease/image/agent; orchestrator `create_telemetry_actor`
  (`CliBlockingSampler` nvidia-smi) for LLM sizing/watchdog. See duplication-report.md.
- **Runtime ownership unchanged:** default = shadow (`KRIA_HRA_ENFORCE` default false); legacy lease
  is the executor; HRA owns admission only under enforce. See runtime-callgraph.md.

## PART 8 — failure analysis (highlights, evidence-based)
- GPU disappears / driver reset → profiler returns Unknown(total 0) → C2 keeps lease Idle; sizing
  refuses GPU (CpuOnly). Safe.
- CUDA OOM on spawn → `record_spawn_failure` ceiling + cooldown + watchdog recovers to CPU
  (`gpu_watchdog.rs` Err arm). Safe (validated headless; real-GPU timing pending).
- Unknown telemetry → C1/C2 handle (no false degrade). Safe.
- Swap failure → C3 clears overlay; backend recovers CPU. Safe.
- Sidecar crash (vision) → bounded restart (3×) then churn; **env issue** (missing fastapi), not HRA.
- Double ownership under enforce → consumers route through one shared lease / co-residency; no second
  arbiter. Under shadow, legacy only. No double-grant path found.

## PART 9 — stress reasoning (no code run)
- 10 concurrent chats: single llama-server, serialized; foreground-guard prevents mid-stream swap.
- Image while chat: shared lease; on small GPU, image → cloud/CPU or Tier-B swap; C2 prevents degrade.
- Low VRAM/No GPU: sizing → CpuOnly; profiler Unknown → CPU. Safe.
- 2 GPUs: DeviceTable multi-device; co-residency tested headless; real silicon pending.
- Weakness: under sustained failed GPU swaps the cooldown (120s) gates retries — good — but if the
  GPU is permanently too small for any ngl>0, it will settle on CPU after one failure + cooldown
  (acceptable). Real timing needs the GPU.
