# Hardware orchestrator: behaviour, gaps, and improvement roadmap

This document answers recurring questions about CPU/GPU/RAM spikes when running KRIA, maps them to the current **hardware orchestrator** implementation (`crates/kria-core/src/llm/orchestrator/`), and contrasts that design with what a **production-grade** AI serving stack typically does.

It is written for maintainers and power users; it is not a user-facing tutorial.

---

## 1. Current orchestration architecture (concise)

| Component | Role | Primary files |
|-----------|------|----------------|
| **Orchestrator** | Boots GPU telemetry, computes initial `(ngl, ctx)`, spawns **llama-server**, starts **GPU watchdog**, optional warmup, exposes **GPU lease** to other subsystems (speech, etc.). | `llm/orchestrator/mod.rs` |
| **LlamaServerManager** | Process lifecycle: build CLI (`--n-gpu-layers`, `--ctx-size`, `--batch-size`, optional `--mmproj`, router `--models-dir`, health wait, slot save/restore, API load/unload when supported). | `llm/orchestrator/server_manager.rs` |
| **GpuWatchdog** | Polls VRAM (EMA-smoothed), drives **tiered degradation** (scale `ngl` / emergency paths / cooldowns) to avoid OOM thrash. | `llm/orchestrator/gpu_watchdog.rs` |
| **Strategy calculator** | Maps **model profile** + **free VRAM/RAM** → `TargetParams` (`ngl`, context, vision mode, degradation level). | `llm/orchestrator/strategy.rs` |
| **GpuLeaseManager** | Serialises “who may touch the GPU” across L1, speech, image backends; can enter **Recovering/Degraded** when telemetry or recovery times out. | `resource/gpu_lease.rs` |
| **Telemetry actor** | Dedicated OS thread publishing VRAM/RAM snapshots for the watchdog (avoids blocking the async runtime). | `llm/orchestrator/telemetry.rs` |
| **Voice stack (v2)** | Separate from llama-server: **whisper-rs** / whisper-cli STT, **Piper** TTS, capture, VAD. Heavy CPU when models are large and `ngl` for whisper is 0. | `voice/v2/*`, `voice/stt.rs` |

The orchestrator optimises **VRAM pressure vs. LLM quality** (layer offload, context, vision). It does **not** today unify scheduling of **all** heavy CPU jobs (embedding indexers, STT, TTS, browser, OS) under one global CPU budget.

---

## 2. Answers to the three questions

### 2.1 “On app start my 24-core CPU hits ~90–94% for a few seconds — why?”

**What is happening (typical stack-up):**

1. **llama-server child process** starts and **memory-maps / initialises** a large GGUF. Even with most weights on the GPU (`--n-gpu-layers` > 0), llama.cpp still uses **many host threads** for tensor setup, disk I/O, format parsing, and **CPU-resident** weights (any layers not on GPU, plus embeddings, KV bookkeeping, batching buffers).
2. **Warmup** (`run_warmup_completion`) triggers at least one **real completion** through the server so kernels and memory are touched (`orchestrator/mod.rs` after spawn). That is intentional for predictable first-token latency later, at the cost of a short burst now.
3. **GPU backend detection** may invoke **NVML or `nvidia-smi`** from a blocking task (`GpuBackend::detect`).
4. **Telemetry actor** thread begins periodic polling (light, but overlaps with the above).
5. **Desktop shell** (Tauri + Vite + native deps) may compile, load ONNX/embeddings, or hydrate DB **in parallel** with the orchestrator — OS “% CPU” is **global**, not KRIA-only.

**Is it expected?**  
**Yes**, for a single-node app that **eager-starts** a full LLM runtime and warms it in-process. The spike duration is usually **seconds**, not minutes, unless disk is slow or RAM is tight (paging).

**Can we optimise?**  
Yes, with trade-offs:

| Direction | Effect |
|-----------|--------|
| Pass **`--no-warmup`** to llama-server when acceptable (`server_manager` launch tuning already considers batch/vision profiles). | Shorter startup burst; **first real user prompt** may pay cold-start latency. |
| **Defer** orchestrator start until after first chat, or start with **`ngl = 0`** then promote when idle (policy change; more complex UX). | Lower peak at login; slower first reply if user speaks immediately. |
| **Stagger** other heavy services (embeddings warm, voice model preload) after `STATE_READY` + N seconds. | Spreads CPU load; delays optional features. |
| Ensure **llama-server** is not rebuilt on every dev run (release profile, sccache). | Less *compile* CPU; unrelated to orchestrator but shows up in “app start”. |

---

### 2.2 “llama-server uses GPU **and** ~5GB+ system RAM — shouldn’t it all be on GPU?”

**No — not with partial GPU offload or with llama.cpp’s normal memory model.**

**Why host RAM stays large:**

1. **`ngl` (GPU layers) is not “all layers”** unless the model fits entirely in VRAM. `strategy::calculate_target_params` deliberately uses **PartialOffload / ReducedContext** when `free_vram - safety_margin` is insufficient. Every **CPU layer** keeps weights and activations in **host RAM**.
2. **KV cache** for the context window scales with **`--ctx-size`** and batching; llama.cpp allocates substantial **CPU RAM** for cache and scratch even when compute is mostly on GPU.
3. **Memory-mapped weights** (`mmap`) often show as **RSS** as pages are touched during load and inference — this is **not** duplicate “full copy on GPU + full copy on RAM” in the naive sense, but **RSS still grows** on the host.
4. **Vision (`--mmproj`)** can place projector weights primarily in **RAM** when running in reduced-GPU / CPU-vision modes (`vision_strategy`, `VisionMode`).
5. **Batch / microbatch / parallel** settings (`launch_tuning` in `server_manager.rs`) increase **temporary activation** footprint in RAM.

**Is it expected?**  
**Yes** for multi-billion-parameter models with realistic VRAM budgets.

**Can we optimise?**  
- Tighten **model profile** VRAM estimates and **safety margin** so the strategy picks a **higher `ngl`** when safe (fewer CPU layers → less RAM, more VRAM).  
- Reduce **`ctx-size`** if conversations do not need 8k–32k tokens resident.  
- Use **API unload** / idle eviction (`Orchestrator` idle paths) so RAM drops when no chat is active (already partially implemented; depends on llama-server build supporting router unload).

---

### 2.3 “When I open the mic, CPU often jumps above 90% for seconds — why?”

**Primary contributors in KRIA’s voice path:**

1. **Whisper STT** (`WhisperRsStt` in `voice/v2/stt.rs`): large models (e.g. **large-v3-turbo**) on **CPU** (`use gpu = 0` in whisper.cpp logs) are **many× slower than realtime**. `decode_once` uses **`spawn_blocking`** with multi-threaded whisper; that legitimately saturates many cores during **partial** (if enabled) and **final** passes.
2. **Sequential mutex** on whisper context prevents corruption but can **lengthen** wall time if partial + final both run.
3. **CLI fallback** (`SpeechToText`) shells out to **whisper-cpp** subprocess — another multi-threaded CPU burst.
4. **Audio capture + VAD** are comparatively cheap; the **acoustic model** dominates.
5. **Piper / ONNX Runtime** TTS (`voice-piper-rs`) adds CPU/GPU bursts when synthesising, separate from llama-server.

**Is it expected?**  
**Yes** for **CPU-bound** STT with large models on a desktop integration.

**Can we optimise?**  
- **Smaller / distilled** STT models for interactive mode.  
- **`KRIA_WHISPER_PARTIAL=0`** to skip rolling partial decodes (lower CPU, no live partial text).  
- **CUDA/Vulkan** whisper feature flags when hardware supports them.  
- **Defer** TTS until after STT if pipeline allows; avoid parallel **llama-server** sampling during the same window (GPU lease already tries to serialise some of this, but CPU STT still fights for cores).

---

## 3. Orchestrator-specific issues (observed / inferred)

| ID | Issue | Symptom | Severity | Notes |
|----|--------|---------|----------|-------|
| O-1 | **Eager bundled startup** | CPU + I/O spike at launch | Medium | By design; needs policy knobs for “light start”. |
| O-2 | **Partial GPU offload** | High RSS alongside VRAM use | Low (educational) | Users misread as “double load”; document in UX/tooltips. |
| O-3 | **GPU lease recovery → Degraded** | Speech fallback cannot lease GPU; STT/TTS paths must tolerate CPU | Medium | Mitigated for whisper-cli lease bypass; root cause is recovery telemetry/timeouts (`GuardReleasedAwaitingTelemetry`). |
| O-4 | **Watchdog-driven swap** | Brief llama-server restart or API load/unload; user-visible latency | Medium | Correct for VRAM safety; production needs **SLO** + user messaging. |
| O-5 | **No global CPU scheduler** | Voice + LLM + OS contention | High | Architectural gap vs. large prod systems. |
| O-6 | **Single local llama-server** | No horizontal scale-out | Low for single-user; high for multi-tenant | Expected for desktop KRIA. |

---

## 4. How a production-grade system usually differs

| Dimension | Typical production AI stack | KRIA desktop orchestrator today |
|-----------|------------------------------|----------------------------------|
| **Process model** | Separate **stateless** inference pods + queue (K8s, autoscale) | **One** (or few) **llama-server** processes tied to the desktop app |
| **GPU sharing** | **Time-slicing / MIG / separate pools** per workload class | **Lease + watchdog** on one consumer GPU |
| **CPU isolation** | **cgroups**, **CPU sets**, dedicated nodes for STT | Best-effort OS scheduling |
| **Cold vs warm** | **Scale-to-zero** + paid cold start **SLO** | **Eager warm** for best UX on one machine |
| **Observability** | **Per-request** tracing, saturation metrics, budgets | Health registry + logs; room for richer metrics |
| **Failure domains** | STT/LLM/TTS **independent** failure + backoff | Shared machine; lease degradation couples subsystems |
| **Capacity planning** | **Discrete** models per SKU (small/medium/large) | **Strategy** adapts `ngl`/ctx; user still picks **which** GGUF |

KRIA’s orchestrator is **appropriate for a single-user, local-first assistant**: it prioritises **not crashing the GPU** and **recovering from VRAM pressure** over **minimising every CPU spike**. Production stacks trade **complexity and ops cost** for **multi-tenant isolation and tail-latency SLOs**.

---

## 5. Recommended improvement roadmap

### Short term (low risk)

- **User-facing copy**: explain GPU+RAM behaviour in settings (“CPU layers use system RAM”).  
- **Startup profile**: optional **`--no-warmup`** or delayed warmup for laptops on battery.  
- **Voice**: document **`KRIA_WHISPER_PARTIAL`**, model size guidance, and GPU whisper flags.  
- **Lease / telemetry**: reduce false **Degraded** transitions (tune `recovery_timeout`, ensure GPU owner releases lease promptly after STT).

### Medium term

- **Global load governor**: a small **CPU budget** token shared by STT spawn, Piper init, and optional background indexers — queue work when LLM is under load.  
- **Tiered voice**: “**Fast path**” (small STT model) vs “**Accurate path**” (large model) selectable per session.  
- **Metrics**: export **p50/p95** STT milliseconds, llama-server **tokens/s**, VRAM headroom — surface in debug panel.

### Long term

- **Optional remote backends**: same UI, but STT/LLM/TTS hit **managed APIs** when local resources are insufficient.  
- **True idle hibernation**: unload **both** whisper and llama weights when idle > N minutes (today partial for LLM; extend to voice).  
- **Multi-instance llama** only where router + VRAM allows — generally **not** for single-GPU consumer hardware.

---

## 6. Code map (for implementers)

- `crates/kria-core/src/llm/orchestrator/mod.rs` — `Orchestrator::start`, warmup spawn, lease wiring.  
- `crates/kria-core/src/llm/orchestrator/server_manager.rs` — `spawn`, `launch_tuning`, `run_warmup_completion`.  
- `crates/kria-core/src/llm/orchestrator/gpu_watchdog.rs` — pressure state machine, swap triggers.  
- `crates/kria-core/src/llm/orchestrator/strategy.rs` — `calculate_target_params`, degradation levels.  
- `crates/kria-core/src/resource/gpu_lease.rs` — acquire/release, degraded/recovering behaviour.  
- `crates/kria-core/src/voice/v2/stt.rs` — whisper-rs streaming, mutex, partial gating.

---

## 7. Revision history

| Date | Author | Change |
|------|--------|--------|
| 2026-05-14 | Engineering (assistant) | Initial document from codebase review and user-reported telemetry. |

This document should be updated when material behaviour changes (e.g. new idle policies, remote inference, or a unified CPU governor).
