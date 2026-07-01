# GPU Hardware Orchestrator — Hardware E2E Test Report (TRUE)

> Generated on the user's actual machine. Every number below is measured, not assumed.
> Date: 2026-06-30. No fabricated results — what could not run on this host is marked explicitly.

## 1. Target hardware (measured)

| Resource | Value | Source |
|---|---|---|
| GPU | NVIDIA GeForce RTX 4050 Laptop | `nvidia-smi` |
| VRAM total | 6141 MB | `nvidia-smi` |
| VRAM free (at test time) | 5759 MB | `nvidia-smi` |
| NVIDIA driver | 580.159.03 | `nvidia-smi` |
| CPU cores | 24 | `nproc` |
| RAM total | 15706 MB | `free -m` |
| RAM available | ~9607 MB | `free -m` |
| Display | `:1` (X11) + `wayland-0` | env |
| Build flavour | `--no-default-features` (NVML `nvidia` feature NOT compiled — `cargo tauri dev`) | confirmed |
| llama-server | `/home/obaid/.kria/bin/llama-server` (not in PATH; spawned by app) | log |
| LLM model | Qwen3VL-4B-Instruct-Q4_K_M.gguf (36 layers + vision projector) | log |

## 2. The reported bug + root cause (proven from the live log)

**Symptom:** after the footer shows "Assistant ready", the first prompt flips the status to
"Limited availability / Core: LLM server not reachable", then recovers and replies — every run.

**Evidence** — `~/.kria/logs/kria.log.2026-06-30`:

```
14:43:37  orchestrator: initial parameters  ngl:0  ctx:4096  degradation:CpuOnly
14:43:49  server_manager: llama-server is ready  ngl:0  ctx:4096        ← model came up on CPU
14:44:22  watchdog: recovery headroom — entering Recovering  free_mb:5691  delta_ngl:36
14:44:22  server_manager: spawning llama-server  ngl:36  ctx:7718       ← scale-up restart
14:45:22  watchdog: swap spawn failed … timed out waiting for llama-server … 60s
14:45:22  watchdog: CPU recovery spawn succeeded — LLM available on CPU  ← flap back to CPU
14:46:56  … entering Recovering … spawning ngl:36 … clamp ngl:33 … fail … CPU recovery …
(repeats every ~3 min)
```

**Root cause (backend, not frontend):**
1. At startup, `Orchestrator::start()` read `telemetry.snapshot()` **before the telemetry actor's
   first poll completed**, so it saw `free=0, total=0` (cold-start race) and sized the model to
   **CPU (ngl=0)**.
2. The model therefore came up on CPU. The footer correctly showed "ready" (the CPU server *is*
   reachable).
3. The watchdog then saw the real free VRAM (5691 MB) and repeatedly tried to move the model onto
   the GPU by **restarting llama-server** (ngl 36 → 33 → 30 …). Each restart is the
   "LLM server not reachable" window. On the 6 GB card the oversized target failed to load within
   60 s, so it fell back to CPU and tried again — an endless flap.

The frontend was reporting the truth. The defect is the backend cold-start mis-sizing that puts the
model on CPU and then thrashes trying to correct it.

## 3. The fix (backend)

`crates/kria-core/src/llm/orchestrator/mod.rs` — `start()`: added a **cold-start VRAM guard**. On a
GPU backend, if the first telemetry snapshot reports `total_vram_mb == 0` (actor not warm yet), force
a **fresh synchronous VRAM read** (shared `TelemetryHub::sample_now`, else a one-shot
`CliVramProfiler`/nvidia-smi read) *before* sizing. The model is then sized onto the GPU once, at
startup, and never needs a scale-up restart.

This composes with the Phase-8 redesign already in the tree:
- **G1 measured-first sizing + CUDA reserve** → the chosen GPU size is conservative (fits 6 GB).
- **G2 watchdog → executor, `KRIA_GPU_AUTOSCALE` default OFF** → no opportunistic scale-up restarts.
- Result: load once on GPU, lock, stay. No first-prompt flap.

> NOTE: the binary that produced the log above is **stale** (it logs the old
> `watchdog: recovery headroom — entering Recovering` path, not the gated policy path). The user must
> **rebuild** (`cargo tauri dev`) to pick up this fix.

## 4. Tests executed on this hardware

### 4.1 Headless suites (ran on this CPU/RAM)

| Suite | Command | Result |
|---|---|---|
| Core lib (all) | `cargo test -p kria-core --lib` | **2687 passed, 1 failed, 2 ignored** |
| — the 1 failure | `agent::loop_engine::tests::deterministic_dispatch_create_project_folder` | **pre-existing, unrelated to HRA** (agent loop test) |
| Orchestrator | `cargo test -p kria-core --lib orchestrator::` | **131 passed, 0 failed** |
| HRA acceptance | `cargo test -p kria-core --test hra_acceptance` | **9 passed, 0 failed** |
| HRA stress (10k+ concurrency) | `cargo test -p kria-core --test hra_stress` | **6 passed, 0 failed** |
| HRA bench (latency bounds) | `cargo test -p kria-core --test hra_bench` | **3 passed, 0 failed** |
| Compile (default) | `cargo check -p kria-core` | **clean** |
| Compile (user runtime) | `cargo check -p kria-core --no-default-features` | **clean** (1 unrelated warning) |

### 4.2 Real-GPU sizing E2E (ran against the actual RTX 4050 via nvidia-smi)

New test: `crates/kria-core/tests/gpu_orchestrator_hw_e2e.rs`
(`cargo test -p kria-core --test gpu_orchestrator_hw_e2e -- --nocapture`). **2 passed, 0 failed.**

Measured output:
```
REAL GPU telemetry: free=5759 MB, total=6141 MB   (CliVramProfiler / nvidia-smi)
cold-start (free=0) sizing      → ngl=0   (cpu_only)          ← the bug trigger, reproduced
real-VRAM (free=5759) sizing    → ngl=29  ctx=4096 (partial_offload)   ← the fix: lands on GPU
measured-first (stable window)  → ngl=29  ctx=4096 (partial_offload)
churn window [5759,4259,5759,4259] → measured ngl=5  vs peak ngl=29    ← volatility reserve holds
```

Interpretation (hardware-true):
- The `CliVramProfiler` reads the real card correctly under `--no-default-features` (free 5759 /
  total 6141). This is exactly the source the cold-start fix uses.
- With real free VRAM the orchestrator now sizes the model **onto the GPU at ngl=29 / 4096 ctx**
  (partial offload) — well within 6 GB. Note it is **not** the ngl=36 that was failing to load in the
  log; the CUDA reserve pulls the target down to a size that fits.
- The cold-start zero reading still maps to CPU (ngl=0) — confirming precisely the trigger the fix
  removes by forcing a fresh read.
- Under simulated desktop VRAM churn, measured-first sizes for the floor (ngl=5), never the transient
  peak — i.e. it will not size into memory another app is about to reclaim.

## 5. What is validated vs what still needs the rebuilt app

**Validated on this hardware (true):**
- Real VRAM telemetry via nvidia-smi CLI profiler (5759/6141).
- Startup sizing now lands on GPU (ngl=29) with the measured free VRAM; cold-start zero → CPU is the
  exact, reproduced flap trigger the fix addresses.
- Volatility reserve keeps sizing conservative under churn.
- All HRA control-plane logic (admission, preemption, co-residency, journal, stress, bench) green.

**Still requires running the REBUILT desktop app (cannot be done from a headless test):**
- Actual `llama-server` load at ngl=29 completing within the health timeout (the log's 60 s timeout
  at ngl=36 needs confirming it disappears at ngl=29 on this box — tune `KRIA_CUDA_RESERVE_MB` /
  Settings → Hardware → "GPU memory reserve" if the first GPU load is still slow/marginal).
- A full interactive session showing the footer stays "Assistant ready" on the first prompt with **no**
  "not reachable" flap and **no** between-session restarts (the redesign's governing law).
- This is Task 74 (hardware soak) — the only remaining open item, and it is hardware-only by nature.

## 6. Action for the user
1. **Rebuild and restart**: `cargo tauri dev` (the running binary is stale and lacks the fix).
2. Watch `~/.kria/logs/kria.log.<date>` on first prompt. Expect:
   - `orchestrator: cold-start fresh VRAM read … sizing on GPU, not CPU`
   - `orchestrator: initial parameters ngl:29` (or similar > 0), `llama-server is ready ngl:<n>` with n>0.
   - **No** `entering Recovering` / `swap spawn failed` / `CPU recovery` loop.
3. If the first GPU load is still slow/marginal, raise Settings → Hardware → **GPU memory reserve**
   to ~1536 MB and retry (gives the loader more headroom).
