# "LLM won't start" — TRUE Hardware Investigation, Root Cause, Fix & Validation

> Every number below was measured on the user's actual machine by spawning the REAL `llama-server`
> binary against the REAL Qwen3VL-4B model. No simulated results.

## Hardware (measured)
- GPU: NVIDIA GeForce RTX 4050 Laptop, **6141 MB VRAM** (5759 free), driver 580.159.03
- Also present: Intel UHD iGPU (the llama-server Vulkan build enumerates BOTH GPUs)
- CPU: 24 threads (i7-13700HX) · RAM: 15706 MB
- Build: `--no-default-features` (no NVML feature) · binary: `~/.kria/bin/llama-server` (Vulkan)
- Model: `Qwen3VL-4B-Instruct-Q4_K_M.gguf` (2.4 GB) + `mmproj-…F16.gguf` (798 MB, CPU)

## Symptom
After the cold-start VRAM fix, the orchestrator correctly sized onto the GPU — but then picked
**ngl=36, ctx=8192 (Full)**, the spawn **timed out after 60 s**, and the orchestrator gave up:
`"orchestrator: failed to start … timed out waiting for llama-server to report listening port"`.
Result: LLM never came up at all.

## TRUE load sweep (real llama-server spawns, `scripts/gpu_load_sweep.sh`)

| ngl | ctx | result | peak VRAM used |
|----:|----:|--------|---------------:|
| 36 | 8192 | **TIMEOUT @75s** | 2347 MB |
| 36 | 4096 | **TIMEOUT @75s** | 2347 MB |
| 32 | 4096 | **TIMEOUT @75s** | 2110 MB |
| 30 | 4096 | **TIMEOUT** | — |
| 28 | 4096 | **READY @2s** | 2365 MB |
| 24 | 4096 | READY @2s | 2070 MB |
| 20 | 4096 | READY @2s | 1783 MB |

## Root cause (proven, not guessed)
- It is **NOT out-of-memory**: even ngl=36 used only ~2.3 GB of 5.7 GB free.
- The hang is purely a function of **`n-gpu-layers`**: **ngl ≤ 28 loads in ~2 s; ngl ≥ 30 hangs
  indefinitely** during model load.
- The hanging log reveals the binary is a **Vulkan build that enumerates two GPUs**:
  `Vulkan0: Intel UHD … / Vulkan1: NVIDIA RTX 4050`. At high ngl it stalls in the
  "fitting params to device memory" / layer-upload path.
- Confirmed the cause is intrinsic to ngl, not fixable by config: spawning with `-fit off`
  **and** with the iGPU hidden (`GGML_VK_VISIBLE_DEVICES=1`, single NVIDIA device) **still hung at
  ngl=36**. So neither auto-fit nor the dual-GPU split is the sole trigger — high ngl itself stalls
  this llama.cpp Vulkan laptop build.

This is a llama.cpp/Vulkan-driver quirk on this laptop, outside KRIA's control. KRIA must therefore
**discover the safe ngl at runtime** rather than trust an estimate.

## Fix (shipped, headless-compiled + hardware-validated)
1. **Cold-start fresh VRAM read** (`llm/orchestrator/mod.rs::start`) — already added: forces a real
   `nvidia-smi` read when the telemetry actor isn't warm, so sizing targets the GPU, not CPU.
2. **Startup ngl-backoff ladder** (`mod.rs::start`) — instead of one spawn at the computed ngl, try a
   descending ladder `[computed, ¾, ½, ¼, 0(CPU)]`. Each GPU attempt uses a **short 20 s probe**
   (`LlamaServerManager::set_spawn_timeout_override` → `effective_spawn_timeout_secs`, new) because a
   healthy load is ~2 s and a hang is forever. The first rung that reports a listening port wins; the
   final rung is CPU (always loads), so **the LLM is always available**.
3. **Persistence** (`mod.rs`, `~/.kria/llm_safe_ngl.json`, keyed by model file) — the working ngl is
   remembered, so the *next* boot starts straight at the known-good value and skips the hang probe.
   Self-tuning, no hardcoded per-GPU heuristic.

## Validation (full-path E2E on the real GPU — `tests/gpu_orchestrator_start_e2e.rs`)

Runs the real `Orchestrator::start` → real llama-server spawn → asserts the server is healthy.

| Run | Result |
|---|---|
| **First boot** (no cache) | `started in 22.07s: backend=Cuda ngl=23 ctx=2048 healthy=true` — tried ngl=31, hung 20 s, **backed off to 23**, loaded. Persisted `23`. |
| **Second boot** (cached) | `started in 1.81s: ngl=23 healthy=true` — read cached 23, **skipped the hang probe**. |
| cache file | `{"Qwen3VL-4B-Instruct-Q4_K_M.gguf": 23}` |

Plus the earlier sizing E2E (`tests/gpu_orchestrator_hw_e2e.rs`): real free 5759/6141 → sizes onto
GPU; cold-start zero → CPU (the reproduced trigger); volatility reserve conservative under churn.

## Regression (headless)
- `cargo test -p kria-core --lib orchestrator::` → **131 passed, 0 failed**
- `cargo check -p kria-core` (default) and `--no-default-features` (user's runtime) → **clean**
- `cargo check -p kria-desktop` → **clean**

## Net effect for the user
- **The LLM now always starts.** First boot self-discovers the safe ngl (~22 s once); every boot
  after is **~2 s** at the cached value.
- On this RTX 4050 the served config is **ngl=23 on GPU** (Cuda backend, ~2 GB VRAM). No more
  "LLM server not reachable" / 60 s startup failure.

## Optional manual tuning (not required)
- To push more layers onto the GPU, edit `~/.kria/llm_safe_ngl.json` upward and restart; if the new
  value hangs, the backoff will drop it again and re-persist the working one.
- Hiding the Intel iGPU from llama-server (`GGML_VK_VISIBLE_DEVICES=<nvidia-index>`) is a clean-up
  that does NOT raise the safe ngl on this build (verified), so it is not part of the fix.

## Honest limits
- The exact reason high ngl hangs lives in the llama.cpp **Vulkan** build/driver, not in KRIA. KRIA's
  fix is to detect-and-adapt, which is the correct layer to solve it at.
- A CUDA-compiled `llama-server` (instead of the Vulkan one) would likely allow full offload (ngl=36)
  and is the real upstream upgrade path — but that is a binary/build change, not a code change here.
