# K.R.I.A. Hardware Orchestration (Current Implementation)

Last updated: 2026-04-26

This document describes the hardware orchestration behavior that is currently implemented in this repository.

## Source Of Truth Files

- `crates/kria-core/src/llm/orchestrator/mod.rs`
- `crates/kria-core/src/llm/orchestrator/gpu_watchdog.rs`
- `crates/kria-core/src/llm/orchestrator/server_manager.rs`
- `crates/kria-core/src/llm/orchestrator/child_guard.rs`
- `crates/kria-core/src/llm/orchestrator/strategy.rs`
- `crates/kria-core/src/llm/orchestrator/threshold.rs`
- `crates/kria-core/src/llm/orchestrator/vision_strategy.rs`
- `crates/kria-core/src/llm/orchestrator/vram_budget.rs`
- `crates/kria-core/src/llm/orchestrator/telemetry.rs`
- `crates/kria-core/src/llm/orchestrator/tier_strategy.rs`
- `crates/kria-core/src/llm/model_router.rs`
- `crates/kria-core/src/llm/local.rs`
- `crates/kria-core/src/agent/loop_engine.rs`
- `crates/kria-core/src/image/orchestrator.rs`
- `crates/kria-core/src/image/swap.rs`
- `crates/kria-core/src/platform/vram.rs`
- `crates/kria-core/src/config.rs`
- `crates/kria-desktop/src/commands.rs`
- `kria-modules/src/kria_modules/processors/image.py`

## 1) Runtime Topology And Startup

### 1.1 Desktop startup flow (tier-aware)

The desktop runtime starts orchestration in the background so UI boot is not blocked.

1. Resolve model file paths from:
   - `~/.kria/models/llm/<file>`
   - workspace `models/llm/<file>` while walking parent dirs from cwd
   - raw path as fallback
2. Select model via `tier_strategy::select_model_for_tier(...)`:
   - user override (`llm.active_model`) when file exists
   - tier-fit largest model in effective budget
   - smallest existing fallback
3. Derive `orchestrator.model_profile` using `derive_model_profile(...)`.
4. Apply hardware safety clamps via `OrchestratorConfig::tune_for_tier(...)`.
5. Start `Orchestrator::start(...)` in background.
6. Attach `LlamaServerManager` to `ModelRouter` (`attach_server_manager`).
7. Start optional idle-release monitor (if enabled).

### 1.2 GPU backend detection

`GpuBackend::detect()` behavior:

- macOS: `Metal`
- non-macOS:
  - `Cuda` if NVML init succeeds or `nvidia-smi --query-gpu=name` succeeds
  - `CpuOnly` otherwise

### 1.3 Telemetry pipeline

Orchestrator telemetry is actor-based:

- `TelemetryActor` runs on a dedicated OS thread.
- Publishes snapshots over `tokio::sync::watch`.
- `WatchTelemetry::snapshot()` is non-blocking (borrow/clone).

Sampler selection:

- `Cuda`: NVML sampler -> `nvidia-smi` sampler -> RAM sampler fallback
- `Metal`/`CpuOnly`: RAM sampler

## 2) Target Parameter Strategy

### 2.1 `TargetParams`

`strategy::calculate_target_params(...)` returns:

- `ngl`
- `context`
- `enable_vision` (compat field; derived from `vision_mode`)
- `vision_mode: VisionMode`
- `degradation: DegradationLevel`

### 2.2 CUDA path math

Given `free_vram_mb` and `safety_margin_mb`:

1. `available = free_vram_mb - safety_margin_mb`
2. `mmproj_cost = mmproj_vram_mb` if projector exists, else `0`
3. If `available < base_vram_overhead_mb + mmproj_cost`:
   - `ngl = 0`
   - `context = min_context`
   - `degradation = CpuOnly`
4. Else:
   - `budget_after_base = available - base_vram_overhead_mb - mmproj_cost`
   - `ngl = min(budget_after_base / per_layer_vram_mb, total_layers)`
   - remaining VRAM feeds context using `kv_per_1k_ctx_mb`
   - context is clamped `[min_context, max_context]`

`determine_vision_mode(profile, ngl, free_ram_mb)` is called for vision state.

### 2.3 Metal and CPU-only behavior

- `GpuBackend::Metal`:
  - `ngl = total_layers`
  - context scales from free RAM (reserves ~2 GB)
- `GpuBackend::CpuOnly`:
  - `ngl = 0`
  - `context = max_context`

### 2.4 Degradation levels

`degradation_level(ngl, context, profile)` maps to:

- `CpuOnly`
- `HeavyOffload`
- `PartialOffload`
- `ReducedContext`
- `Full`

Rule highlights:

- `ngl == 0` -> `CpuOnly`
- `ngl < total_layers/2` -> `HeavyOffload`
- `ngl == total_layers` but reduced context -> `ReducedContext`

## 3) Vision Degradation Strategy (`VisionMode`)

`vision_strategy.rs` defines:

- `FullGpu`
- `ReducedGpu`
- `CpuVision`
- `Disabled`

Mode selection:

- No projector -> `Disabled`
- `ngl >= vision_min_ngl` -> `FullGpu`
- `0 < ngl < vision_min_ngl` -> `ReducedGpu`
- `ngl == 0`:
  - free RAM >= 2048 MB -> `CpuVision`
  - else -> `Disabled`

Mode contracts:

- `load_mmproj()` is `true` for `FullGpu`, `ReducedGpu`, `CpuVision`.
- `max_image_dimension()`:
  - `FullGpu`: `0` (no cap)
  - `ReducedGpu`: `512`
  - `CpuVision`: `256`
  - `Disabled`: `0` (N/A)

## 4) Watchdog State Machine (`gpu_watchdog.rs`)

### 4.1 Runtime states

- `Idle { since }`
- `Pressured { since, target }`
- `Cooldown { until }`
- `Recovering { since, target }`
- `Critical { since }`

`target` in `Pressured` is a gate target. Actual swap target is recomputed at swap execution time.

### 4.2 Threshold resolution

At watchdog start:

- `Metal`: use `macos_*_ram_mb` thresholds from config.
- non-`Metal`: compute `ThresholdProfile::from_total_vram(total_vram_mb)` then apply config overrides.

Dynamic percentages in `threshold.rs`:

- emergency: 3% (floor 64 MB)
- yield: 10% (floor 256 MB)
- recover: 35% (floor 1024 MB)
- hysteresis: 5% (floor 128 MB)
- safety margin: 8% (floor 256 MB)

Important override semantics:

- Any non-zero config field overrides dynamic value.
- `0` means "use dynamic".

### 4.3 Exact transition logic

- `Idle -> Pressured` when EMA free `< yield_threshold`.
- `Idle -> Recovering` when free `> recover_threshold + hysteresis`, `delta_up >= min_ngl_delta_up`, and normal budget exists.
- `Pressured -> Idle` when free `> yield_threshold + hysteresis`.
- `Pressured -> Cooldown` when pressure dwell elapsed:
  - if `delta < min_ngl_delta`, skip swap and enter cooldown
  - else recompute fresh target and swap
- `Recovering -> Cooldown` when recovery dwell elapsed (swap up).
- `Recovering -> Idle` if free drops below `recover_threshold` before dwell.
- `Cooldown -> Idle` when cooldown timer expires.
- Any non-cooldown state -> `Critical` when free `< emergency_threshold`.
- `Critical` executes emergency swap only after emergency dwell and emergency budget availability.

### 4.4 Anti-thrash controls

- EMA alpha: `0.5`
- hysteresis band
- pressure/recovery/emergency dwell timers
- separate normal/emergency rate buckets
- hard state dwell cap (`state_max_dwell_secs`), reset to `Idle`

### 4.5 Swap execution path from watchdog

`execute_swap_with_target(...)`:

1. publish `LlmSwapStarted`
2. `cancel_streams()` + publish `LlmStreamInterrupted`
3. stop current server (`graceful_stop` or `kill` if emergency)
4. CUDA only: wait for VRAM release
5. spawn new server with target `(ngl, ctx, vision_mode)`
6. on success publish:
   - `LlmSwapCompleted`
   - `LlmDegradationChanged`
7. on failure publish `LlmSwapFailed`

## 5) Server Manager Lifecycle (`server_manager.rs`)

### 5.1 State constants

- `STATE_STOPPED = 0`
- `STATE_STARTING = 1`
- `STATE_READY = 2`
- `STATE_SWAPPING = 3`
- `STATE_ERROR = 4`

### 5.2 Spawn path

`spawn(ngl, context, vision_mode, ...)`:

1. Resolve vision request:
   - `vision_requested = vision_mode.load_mmproj()`
   - `vision_enabled = vision_requested && vision_configured()`
2. Build command with:
   - `--model <path>`
   - `--port 0`
   - `--ctx-size <context>`
   - `--n-gpu-layers <ngl>`
   - `--batch-size ...`
   - `--slot-save-path ...`
   - optional: `--ubatch-size`, `--parallel`, `--no-warmup`, `--flash-attn on`, `--mlock`, `--mmproj`
3. Discover port from stdout/stderr logs.
4. Set API URL (`http://127.0.0.1:<port>/v1`).
5. Poll health until ready.
6. Set current params and `STATE_READY`.

### 5.3 Vision launch tuning

`launch_tuning(config_batch_size, vision_enabled)`:

- Vision-enabled runtime:
  - clamp batch to max 128
  - set `--ubatch-size` equal to batch
  - set `--parallel 1`
  - set `--no-warmup`
- Non-vision runtime:
  - keep configured batch
  - no `ubatch/parallel/no-warmup` overrides

### 5.4 Port and health details

- Port discovery timeout: `port_discovery_timeout_secs` (default 60)
- Health readiness timeout: `health_check_timeout_secs` (default 120)
- Health backoff: `50 -> 100 -> 200 -> 400 -> 800 ms` (cap)

### 5.5 Stream cancellation model

`cancel_token` is renewable:

- `cancel_streams()` cancels current token and mints a fresh one.
- New streams capture the new token.

`swap_done` (`Notify`) is used by clients to await swap completion without busy polling.

### 5.6 Stop paths

- `graceful_stop_with_timeout(...)`:
  - set `STATE_SWAPPING`
  - cancel streams
  - abort log reader
  - `ChildGuard::terminate(timeout)`
  - set `STATE_STOPPED`
- `kill()`:
  - set `STATE_SWAPPING`
  - cancel streams
  - abort log reader
  - `ChildGuard::force_kill()`
  - set `STATE_STOPPED`

### 5.7 Child process guard (`child_guard.rs`)

- Linux pre-exec:
  - `setsid()`
  - `prctl(PR_SET_PDEATHSIG, SIGKILL)`
- Graceful ladder: `SIGTERM -> wait(timeout) -> SIGKILL -> wait`
- Force kill wait timeout: up to 3s
- Drop safety: synchronous kill + `start_kill()` fallback

## 6) API-Level Unload/Load (Router Mode)

### 6.1 Endpoints

Resolved by `v1_models_endpoint(base_url, action)`:

- `.../v1/models/unload`
- `.../v1/models/load`

### 6.2 `api_unload_model()`

Flow:

1. require non-empty API URL
2. set `STATE_SWAPPING`
3. cancel streams
4. POST unload JSON body `{ "model": <model_path> }`
5. status handling:
   - `404/501`: set `STATE_READY`, return Router Mode unsupported error
   - non-success other: set `STATE_READY`, return error
   - transport error/timeout: set `STATE_READY`, return error
6. success path:
   - set `current_ngl=0`
   - set `current_vision=false`
   - keep process alive
   - remain in swapping lifecycle until later completion

Request timeout is `health_check_timeout_secs` clamped to `[1, 30]`.

### 6.3 `api_load_model()`

Flow:

1. require API URL
2. POST load body `{ "model": <model_path> }` (120s client timeout)
3. wait for health
4. set `STATE_READY`

### 6.4 Current wiring status

- `Orchestrator::evict_to_cpu()` calls `api_unload_model()` first.
- `Orchestrator::restore_from_cpu()` currently performs process-stop + respawn path and does **not** call `api_load_model()`.
- Since orchestrator spawn uses `--model` (not `--models-dir`), many llama.cpp builds will return `404/501` for unload and trigger fallback restart.

## 7) Image Generation Tiering And Tier-B Swap

### 7.1 Image tiers (`platform/vram.rs`)

Classification by live `VramSnapshot.free_mb`:

- `SHighRes`: >= 14000 MB
- `AStandard`: >= 10000 MB
- `BDropSwap`: >= 4000 MB
- `CRejectOrCloud`: < 4000 MB or no real GPU telemetry

`required_free_mb()`:

- S: 6500
- A: 6000
- B: 4500
- C: 0

### 7.2 Tier-B barrier target (`image/orchestrator.rs`)

For swap admission:

- if total VRAM `<= 8192`: `required_mb = max(round(total * 0.38), 2000)`
- else: `required_mb = 4500`

### 7.3 `generate_with_swap(...)` behavior

1. If no LLM evictor provided -> generate locally (no swap path).
2. Emit blackout and voice events.
3. Pause audio callbacks if registered.
4. Acquire `EvictionToken`:
   - performs LLM eviction via controller
   - enforces VRAM barrier when eviction actually occurred
5. On barrier timeout:
   - sleep 600 ms
   - retry once
   - on second failure increment `hang_count`
   - after 2+ hangs set `session_degraded = true`
6. Run local generation.
7. `token.restore().await` restores LLM path.
8. Resume audio callbacks.
9. Emit restored event and run swap defrag tick.

### 7.4 `EvictionToken::acquire(...)`

- If LLM already CPU-resident:
  - skip eviction call
  - skip barrier wait (logs snapshot only)
- Else:
  - call `evict_to_cpu()` once
  - wait `VramBarrier.await_free()`

`VramBarrier` defaults:

- poll: 50 ms
- timeout: 3 s
- stable samples: 3 consecutive above threshold

### 7.5 Orchestrator eviction/restore implementation

`evict_to_cpu()`:

1. stop watchdog
2. best-effort `save_slot_kv(slot=0, file="kria_tier_b.bin")`
3. try API unload first
4. if unload fails:
   - graceful stop
   - wait VRAM release (CUDA)
   - spawn `ngl=0`, preserving context
   - determine CPU vision mode dynamically via free RAM
   - best-effort restore slot KV

`restore_from_cpu()`:

1. stop watchdog
2. save slot KV
3. graceful stop CPU runtime
4. wait VRAM release (CUDA)
5. recompute target params from current telemetry
6. respawn GPU runtime
7. best-effort restore slot KV
8. restart watchdog

## 8) Agent-Side Vision Preflight For `analyze_image`

### 8.1 Cap decision pipeline (`loop_engine.rs`)

`compute_visual_token_cap()`:

1. Pull profile/safety margin/current params from orchestrator server manager when available.
2. Build an effective `VisionMode` for current runtime.
3. If vision disabled -> hard cap = 0.
4. Query free VRAM from `platform::vram::build_profiler()`.
5. Compute `safe_cap` via `calculate_safe_visual_tokens(...)`.
6. Compute `mode_cap` from `VisionMode::max_image_dimension()`:
   - `0` means unbounded (`u32::MAX`)
   - otherwise converted to token cap using 14-patch estimator
7. Final cap logic:
   - if `safe_cap == 0`: fallback to mode cap (or 4096 for unbounded mode)
   - else min(`safe_cap`, `mode_cap`) where applicable
   - enforce minimum 64 when vision is enabled

### 8.2 Tool-call injection

Before running `analyze_image`, agent loop injects:

- `hard_visual_token_cap: <computed_cap>`

### 8.3 Sidecar enforcement (`kria-modules/processors/image.py`)

Python image processor uses `hard_visual_token_cap` in preprocessing and fallback planning (resize/ROI/token accounting) before visual inference.

## 9) Orchestrator Operations And Observability

### 9.1 Runtime controls

Implemented controls in `Orchestrator`:

- `ensure_ready(reason)`
- `restart(reason)` with cooldown/backoff
- `release_if_idle(reason)`
- `shutdown()`

Desktop command layer calls `ensure_ready(...)` before active turns and tracks active turn counters for idle-release safety.

### 9.2 Idle release

If enabled:

- monitor checks `active_turns == 0`, voice inactive, no swap in progress
- if idle duration exceeds threshold, releases llama-server
- emits `orchestrator:idle_released`

### 9.3 Status endpoint

`get_orchestrator_status` returns:

- backend
- current ngl/context/degradation
- server state code + swapping flag
- process_alive + health flags
- idle-release config and idle duration
- API URL

### 9.4 Event stream

Watchdog/server events published on `EventBus` and forwarded to UI:

- `orchestrator:swap_started`
- `orchestrator:swap_completed`
- `orchestrator:degradation_changed`
- `orchestrator:stream_interrupted`
- `orchestrator:vram_pressure`

## 10) Defaults Snapshot (Orchestrator)

From `OrchestratorConfig::default()`:

- poll interval: `2s`
- thresholds: yield `512`, emergency `128`, recover `2048`
- hysteresis: `256`
- safety margin: `512`
- pressure dwell: `5s`
- emergency dwell: `750ms`
- recovery dwell: `30s`
- cooldown: `60s`
- state max dwell: `300s`
- rate limits: normal `6/hr`, emergency `3/hr`
- min delta down/up: `3 / 6`
- stop timeout: `5s`
- health timeout: `120s`
- port discovery timeout: `60s`
- VRAM release timeout: `5s`
- restart cooldown: `10s`
- restart backoff: `350ms`
- idle release: enabled, `300s` after idle, check every `10s`
- macOS thresholds: yield `2048`, emergency `1024`, recover `4096`

Model profile defaults:

- layers `28`
- per-layer VRAM `165 MB`
- base overhead `200 MB`
- KV per 1k ctx `100 MB`
- context `[2048, 8192]`
- vision projector enabled
- `vision_min_ngl = 15`
- `mmproj_vram_mb = 1300`

## 11) Current Caveats / Implementation Notes

- Dynamic threshold scaling is opt-in per field: non-zero config values override dynamic percentages.
- `ThresholdProfile.safety_margin_mb` is computed but runtime strategy currently uses `config.safety_margin_mb` directly.
- VRAM release waits compare against `config.yield_threshold_mb` (not the dynamic threshold profile value).
- `api_load_model()` exists but is not used in current orchestrator restore flow.
- Audio pause/resume in Tier-B swap is callback-driven; behavior is active only when hooks are registered.
- Agent loop still uses `disable_inline_images_for_turn` for runtime fallback stripping. `VisionMode` is primarily used for preflight cap derivation there.

## 12) Changelog

### v4 (2026-04-26)

- Rebased document to current code paths in desktop startup, orchestrator, server manager, and image Tier-B swap.
- Corrected CPU fallback and vision behavior to `VisionMode`-based logic.
- Corrected Tier-B restore description to current respawn flow.
- Added tier-aware model selection/profile derivation and config tuning details.
- Added operational controls (`ensure_ready`, idle release, status/events) and current caveats.

### v3 (2026-04-25)

- Added API-level unload/load paths, preflight VRAM budgeting, and multi-tier vision modes.

### v2 (2026-04-25)

- Added pressured target recomputation, dynamic threshold module, and renewable cancellation token notes.
