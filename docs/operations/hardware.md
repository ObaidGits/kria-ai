# KRIA Hardware Operations

Last updated: 2026-05-27

## Purpose

KRIA's hardware layer keeps local model execution, image generation, vision, and speech workloads bounded. It detects host capacity, selects safe runtime defaults, controls GPU leasing, and adapts local llama-server behavior under memory pressure.

Hardware does not decide user intent, safety policy, or workflow completion. It only supplies capacity signals and resource controls.

## Hardware Detection

Detection lives under `crates/kria-core/src/platform/detect.rs` and desktop startup uses it from `init_runtime`.

Detected fields:

- operating system,
- hardware tier,
- CPU core count,
- total RAM,
- NVIDIA VRAM and GPU name when `nvidia-smi` is available,
- package manager,
- hostname,
- best-effort free VRAM,
- image-generation tier.

Detection precedence:

1. environment override,
2. config override,
3. cached `~/.kria/hardware_tier.json`,
4. live detection.

The latest detected hardware snapshot is cached to:

```text
~/.kria/hardware_tier.json
```

## Hardware Tiers

| Tier | Context window | Threads | GPU layers | Vision default |
|---|---:|---:|---:|---|
| `lite` | 1024 | 4 | 0 | no |
| `standard` | 2048 | 6 | 0 | no |
| `performance` | 4096 | 8 | 99 | yes |
| `high` | 8192 | 8 | 99 | yes |

Recommended STT model by tier:

| Tier | STT model |
|---|---|
| `lite` | `ggml-small-q5_1.bin` |
| `standard` | `ggml-medium-q5_0.bin` |
| `performance` | `ggml-large-v3-turbo-q5_0.bin` |
| `high` | `ggml-large-v3-turbo-q5_0.bin` |

## Config Inputs

Hardware config:

```toml
[hardware]
tier = ""                 # empty = auto
max_context_tokens = 0    # 0 = tier default/clamp
gpu_layers = -1           # -1 = tier default
threads = 0               # 0 = tier default
```

Orchestrator config:

```toml
[orchestrator]
enabled = true
poll_interval_secs = 2
yield_threshold_mb = 512
emergency_threshold_mb = 128
recover_threshold_mb = 2048
cooldown_secs = 60
max_transitions_per_hour = 6
idle_release_enabled = true
idle_release_after_secs = 300
```

Startup clamps context, thread count, and GPU layers to the detected tier unless explicitly configured within safe bounds.

## Local LLM Runtime

The local LLM runtime is managed only when the active routing mode is local.

When a cloud/external provider is active, desktop startup skips local llama-server:

```text
cloud/external provider active -> no local GPU allocation
```

Local model selection:

1. Honor an explicit active local model when it matches config and the file exists.
2. Pick the largest configured model that fits the tier memory budget.
3. Fall back to the smallest available configured model.
4. If no model files exist, disable local orchestrator with actionable status.

Model search paths:

- `~/.kria/models/llm`
- workspace `models/llm` discovered by walking up from CWD/executable
- absolute model paths

The runtime can create an ad-hoc model entry when the selected model is an existing GGUF file not already present in config.

## Tier-Aware Orchestrator Tuning

Before starting llama-server, KRIA derives a `ModelProfile` from the selected model and then calls `OrchestratorConfig::tune_for_tier`.

The tuning pass:

- disables `flash_attention` unless the tier/GPU is safe,
- disables `mlock` unless RAM headroom is safe,
- clamps batch size by tier,
- raises safety margin on lower tiers,
- slows polling on lite hardware,
- shortens idle release windows on lite/standard tiers.

This is the freeze-prevention layer. It prevents risky defaults such as locking a large model into RAM on a low-memory machine.

## llama-server Lifecycle

`LlamaServerManager` owns one managed llama-server process.

Key behavior:

- lock-free server state,
- ephemeral port discovery,
- `/health` readiness wait,
- cancellation token for stream abort during swaps,
- SIGTERM/SIGKILL child-guard shutdown ladder,
- optional router-mode model load/unload support,
- slot save path support for KV hand-off,
- vision spawn preflight using model profile and mmproj state.

Server states:

```text
stopped
starting
ready
swapping
error
```

## GPU Watchdog

The watchdog lives in `crates/kria-core/src/llm/orchestrator/gpu_watchdog.rs`.

State machine:

```text
Idle
  -> Pressured
  -> Cooldown
  -> Recovering
  -> Idle

Any non-cooldown state can enter Critical.
```

Anti-thrash controls:

- three-sample EMA smoothing,
- pressure dwell time,
- emergency dwell time,
- recovery dwell time,
- hysteresis band,
- normal transition rate limit,
- emergency transition rate limit,
- hard max dwell cap.

The watchdog publishes VRAM pressure events and triggers layer/context swaps through the local orchestrator when budgets allow.

## GPU Lease Manager

`GpuLeaseManager` lives in `crates/kria-core/src/resource/gpu_lease.rs`.

Lease states:

```text
Idle
Held
Recovering
Degraded
```

Owners:

- `L1Worker`
- `ImageBackend`
- `Vision`
- `Speech`
- `Maintenance`

Behavior:

- Lease requests are queued.
- Foreground work can request recovery from a background holder.
- Leases have TTLs.
- Telemetry reconciliation checks expected owner and VRAM safety.
- Stuck recovery transitions degrade.
- Dropping a lease guard releases or moves through recovery.

No lease means no GPU-bound execution for lease-aware paths.

## Resource Telemetry

Resource snapshots include:

- VRAM total/free/used,
- RAM total/free,
- L1 runtime residency,
- image runtime state,
- high-VRAM processes,
- sample time.

Reconciliation can return:

- healthy,
- VRAM warning,
- process mismatch,
- critical OOM risk.

This protects against stale assumptions such as "KRIA thinks the GPU is idle, but another process is holding VRAM."

## Failure Handling

| Failure | Behavior |
|---|---|
| Hardware detection unavailable | Use defaults/cache and continue conservatively. |
| Requested context too high | Clamp to tier limit and warn. |
| Local model missing | Skip local orchestrator and emit disabled status. |
| Cloud provider active | Skip local llama-server. |
| VRAM pressure sustained | Enter pressured path and swap down when budget allows. |
| Critical VRAM pressure | Enter emergency path with separate rate budget. |
| Recovery unstable | Stay conservative until dwell/rate gates pass. |
| Lease conflict | Queue, recover, or fail with explicit lease error. |
| Telemetry mismatch | Recover or degrade instead of assuming success. |

## Operational Checklist

For local model deployments:

1. Confirm `~/.kria/models/llm` contains the configured GGUF files.
2. Confirm `llama-server` is on PATH or bundled.
3. Confirm `config/default.toml` or `~/.kria/config.toml` has valid `[[llm.models]]` entries.
4. Confirm active provider is `llama_cpp` or routing mode is local.
5. Check `orchestrator:selected` or `orchestrator:disabled` events.
6. Watch VRAM pressure, swap, and idle release logs.

For GPU-heavy workloads:

1. Keep OpenClaw, image generation, vision, speech, and L1 worker from competing without leases.
2. Use cloud/external provider mode when local hardware is insufficient.
3. Avoid manually enabling `mlock` or `flash_attention` on low-memory hosts.
4. Treat repeated watchdog swaps as a capacity issue, not a model-quality issue.

## Invariants

- Hardware state informs routing; it does not own policy.
- Local runtime starts only when selected and viable.
- Unsafe local runtime flags are clamped by tier.
- GPU pressure transitions are hysteresis/rate limited.
- Lease-aware GPU users must respect lease state.
- Missing capacity must degrade honestly, not silently overcommit.
