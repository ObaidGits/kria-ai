# KRIA Hardware & GPU Runtime — Production Audit (Root-Cause Only)

> Audit only. No code was changed for this document. Every claim is grounded in source (file:line)
> or in the user's runtime logs (timestamped). This consolidates the requested deliverables
> (timeline, root-cause, state-machine, gpu/cpu pipeline, ui-sync, bug-list, bottlenecks, hardening,
> test plan) into one authoritative report.

Hardware under test (from logs): RTX 4050 Laptop, **6141 MB VRAM**, 15706 MB RAM, 24 cores, tier
`performance`. Build run: `cargo run --no-default-features` (this flag is central to the findings).

---

## 0. THE UNIFYING ROOT CAUSE — telemetry is blind on this build

**`platform::vram::build_profiler()` returns `NullProfiler` (reports 0 free / 0 total VRAM) when the
`nvidia` Cargo feature is not compiled — which is the case under `--no-default-features`.**

Evidence:
- Log: `kria_core::platform::vram: No GPU telemetry available — VramProfiler will report 0 free VRAM (Tier C)`.
- `platform/vram.rs:330` `build_profiler()` gates `NvmlProfiler` behind `#[cfg(feature = "nvidia")]`;
  with the feature off it falls through to `NullProfiler` (`vram.rs:300`, `snapshot()` returns
  `free_mb:0, total_mb:0`). **There is no nvidia-smi CLI fallback in `build_profiler`.**
- BUT the orchestrator's own telemetry **does** have a CLI fallback: log
  `telemetry: using nvidia-smi CLI sampler` (`orchestrator/telemetry.rs` `create_telemetry_actor` →
  `CliTelemetry`). So the orchestrator sees the real 6141/4396 MB, while everything built on
  `build_profiler()` sees 0.

Everything that uses `build_profiler()` is therefore **blind (0 VRAM)** on this build:
- the unified `TelemetryHub` (`telemetry_hub.rs:45`),
- the shared GPU lease's `SharedResourceTelemetry` (`shared_telemetry.rs:37`),
- the HRA snapshot loop + admission verdicts,
- the image VRAM barrier (`image/orchestrator.rs:448`).

This single fact is the upstream cause of **Issue 3** (and the HRA/hub being wrong), and it
interacts with the GPU sizing problems. **Severity: CRITICAL.**

---

## 1. ISSUE 3 — Image "GPU lease degraded: recovery timed out: GuardReleasedAwaitingTelemetry"

**Root cause (exact):** the shared GPU lease has a telemetry source configured
(`SharedResourceTelemetry`, wired at `runtime.rs:~633`), but on this build that source is backed by
the Null profiler → it reports **0 free VRAM**. The lease recovery then can never reconcile:

Pipeline trace:
1. Image releases its lease guard → lease enters `Recovering` (`gpu_lease.rs` `release_token` →
   `transition_to_recovering_locked`).
2. Recovery worker calls `attempt_recovery_pass` (`gpu_lease.rs:~640`) → `telemetry.sample()` →
   `SharedResourceTelemetry::sample` → Null profiler → `VramSnapshot::from_totals(0, 0)`.
3. `ResourceSnapshot::reconcile(&None)` (`telemetry.rs:96`): `available_vram_mb < 200` → **0 < 200**
   → returns `ReconciliationResult::CriticalOomRisk` (`telemetry.rs:100`).
4. `recovery_reconciled` only treats `Healthy | VramWarning` as recovered (`gpu_lease.rs`), so
   `CriticalOomRisk` → **never reconciles**.
5. After `recovery_timeout` the lease → `Degraded { reason: "recovery timed out: ..." }`.
6. Next image/vision/voice acquire hits `GpuLeaseError::Degraded` → the exact UI error.

Note: the Session-10 fix ("no telemetry source → assume reconciled") is **bypassed** here because a
telemetry source *is* set — it just returns garbage 0 because NVML isn't compiled in.

- Evidence: error string matches `gpu_lease.rs` degrade path; `telemetry.rs:100` threshold; log "0 free VRAM".
- Impact: image generation fails; once degraded, voice/vision sharing the lease are blocked too.
- Severity: **CRITICAL**. Probability on this build: **100%** (deterministic — every release degrades).
- Fix strategy (later): give `build_profiler()` an nvidia-smi CLI fallback (so it's never blind),
  OR back `SharedResourceTelemetry` with the orchestrator's working `GpuTelemetry`, OR treat a
  Null/0-total snapshot as "unknown → assume reconciled" instead of CriticalOomRisk.

---

## 2. ISSUE 2 — "Optimizing GPU layers…" never disappears; LLM flaps available/unavailable

**Two independent bugs, either alone causes the stuck overlay.**

**2a. Backend never forwards swap failure.** The event forwarder (`runtime.rs:2083+`) maps
`LlmSwapStarted` → `orchestrator:swap_started`, `LlmSwapCompleted` → `orchestrator:swap_completed`,
plus degradation/stream/vram. But `KriaEvent::LlmSwapFailed` falls into `Ok(_) => {}`
(`runtime.rs:~2131`) → **never emitted to the UI**.

**2b. Frontend has no failure/clear path.** `ui/src/stores/app.ts:4553` listens for
`orchestrator:swap_started` → `setIsSwapping(true)`; `:4560` `swap_completed` → `setIsSwapping(false)`.
**There is no `swap_failed` listener.** `ChatView.tsx:321` shows the overlay `when={isSwapping()}`.

Lifecycle on a failed swap (what the user sees):
1. Watchdog publishes `LlmSwapStarted` → UI `isSwapping=true` → overlay shows.
2. GPU spawn fails (Issue/§4) → backend publishes `LlmSwapFailed`.
3. Forwarder drops it; UI never receives `swap_completed`.
4. `isSwapping` stays `true` **forever** → overlay never clears. State machine has no terminal edge
   for failure. This is a classic missing-event / never-cleared-state defect.

"LLM flaps available/unavailable" = the watchdog kills the working server to swap, the swap fails,
(pre-this-session) no recovery → unavailable; the watchdog later retries and the cycle repeats.

- Evidence: `runtime.rs` `Ok(_) => {}`; `app.ts` only two listeners; `ChatView.tsx:321`.
- Impact: permanent misleading overlay; user thinks LLM is stuck.
- Severity: **HIGH**. Probability: **100%** whenever a swap fails.
- Fix strategy (later): forward `LlmSwapFailed` as `orchestrator:swap_failed`; add a UI listener that
  clears `isSwapping` (and shows a transient "GPU optimization deferred" note). Also clear on
  `orchestrator:error` and on a watchdog timeout.

---

## 3. ISSUE 1 — LLM usable much later than the window opens

**Timeline (from logs, t=0 at 20:53:19.37 launch):**

| t (s) | Event | Note |
|---|---|---|
| 0.0 | process start | window opens fast |
| ~0.15 | hardware detected (cache) | fast |
| 3.0 | embedding model pool init | |
| 3.0–14.4 | **Tool semantic index build (235 tools)** | `Tool semantic index initialized` at ~14.4s — **~11s serial** |
| 14.9 | "KRIA runtime initialized — agent loop active" | |
| 15.6 | **Startup summary 15591ms; "frontend unblocked"; "orchestrator: starting in background"** | UI usable, **LLM not started yet** |
| 15.7 | orchestrator detect backend = Cuda | |
| 15.7 | telemetry nvidia-smi; initial params ngl=0 CpuOnly | |
| 15.7 | spawn llama-server ngl=0 | |
| 21.6 | **llama-server ready ngl=0 (CPU)** — LLM first usable | spawn took ~6s |
| 24.9 | whisper warmup complete (9949 ms) | parallel |
| ~54 | watchdog auto scale-up → GPU ngl=36 | kills CPU server |
| ~114 | GPU swap fails (60s timeout) | → unavailable (pre-fix) |

**Root causes of "LLM late":**
1. **Orchestrator start is deferred to the end of a ~15.6s synchronous init chain.** It is spawned
   only after tool registry, semantic index, MCP scheduling, voice pipeline, perception, etc.
   (`runtime.rs` — orchestrator background block is near the end). The LLM cannot begin loading until
   ~15.6s.
2. **Tool semantic index build (~11s) is on the critical path** before the orchestrator starts
   (`routing/tool_index` builds embeddings for 235 tools). This dominates startup.
3. **LLM spawn is serial after backend detection** (~6s for the CPU llama-server to report its port).
4. Net: first-usable (CPU) ≈ **22s**; full "ready" perception is later because the watchdog then
   immediately tries (and, on this build, fails) a GPU swap.

- Evidence: log timeline above.
- Impact: ~22s to first token capability; UI looks ready at ~15.6s → perceived "LLM starts slowly".
- Severity: **MEDIUM** (usability).
- Fix strategy (later): start the orchestrator/LLM spawn **earlier and in parallel** with the tool
  index + embedding warmup; move the 11s tool-index build off the LLM critical path; consider
  spawning the LLM at ngl-fit immediately instead of CPU-then-swap.

---

## 4. GPU SCALE-UP FAILURE (the engine behind Issues 1 & 2 on this box)

**Root cause:** the watchdog auto-scales `ngl=0 → ngl=36` (all 36 layers) shortly after start, but
only **~4396 MB was actually free** (log: `recovery headroom — free_mb=4396`), not 6141. The layer
estimate (`per_layer_vram_mb=57` × 36 ≈ 2052 + base) does **not** include CUDA runtime context
(~0.6–1 GB) and assumes full total. The GPU server then OOMs or fails to report its port within 60s
→ `SIGKILL` → `failed_ngl=36` → `swap spawn failed`.

Compounding:
- **HRA verdict used stale VRAM.** Log: `HRA GPU-admission verdict ... free 6141 MB ≥ need 3552 MB`
  — it used the **bootstrap total (6141)**, not live free. Because the hub profiler is Null (§0), its
  `HostSnapshot.gpus` is empty, so `HostSnapshot::apply_to` (`collector.rs`) never refreshes the
  device free figure → DeviceTable stays at the bootstrap 6141 forever. HRA is blind. (Shadow mode,
  so it only logged — but under enforce this would mis-admit.)
- **OOM backoff converges slowly.** `clamp_against_failures` (`server_manager.rs:249`) steps down by
  `min_ngl_delta` (≥2) per recorded failure → 36→34→32… each a full 60s-timeout swap cycle → minutes
  of thrash before a fitting ngl is found.
- **(FIXED this session)** failed swap previously left **no server** (no recovery in
  `gpu_watchdog.rs` Err arm). Now recovers to CPU.

- Severity: **HIGH**. Probability on 6 GB + vision model: high.
- Fix strategy (later): size against **live free VRAM** (not total) incl. CUDA overhead margin; pick
  a fitting ngl directly instead of 36-then-backoff; lengthen GPU spawn timeout only if it's
  slow-load (needs the manual `llama-server` test to confirm OOM vs slow).

---

## 5. Runtime state-machine analysis (defects)

- **GPU-swap UI state**: states {idle, swapping}. Missing terminal edge for **failure** → stuck in
  `swapping` (§2). No timeout-driven auto-clear.
- **Lease state machine** (`Idle/Held/Recovering/Degraded`): correct logic, but `Recovering→Idle`
  depends on telemetry that is 0 on this build → always falls to `Degraded` (§1). No "telemetry
  unknown" branch.
- **Degraded is sticky**: `clear_degraded` exists but is not called on the image/voice acquire path —
  once degraded, consumers keep erroring until something explicitly clears it.
- **HRA DeviceTable free**: never refreshed when snapshot has no GPUs → stale-forever (§4).

---

## 6. CPU / daemon / background audit (observations from logs + code)

- **Vision sidecar crash-loop**: `ModuleNotFoundError: No module named 'fastapi'`. The Python venv
  auto-setup failed (`venv creation failed: .../python-env/bin/python3 not found`), so the sidecar
  runs under system python3 without deps → exits → orchestrator auto-restarts (bounded 3×) → repeated
  spawn churn (CPU + log noise). Not HRA, but a real startup cost + the "ocr_dependency degraded".
- **Voice STT whisper warmup**: ~10s (`whisper warmup complete elapsed_ms=9949`) — parallel, but a
  CPU burst at startup. The earlier "800% CPU" is whisper thread fan-out (STT), not the orchestrator.
- **MCP**: `colab-mcp` (bad anaconda python path) + `github` (no `GITHUB_PERSONAL_ACCESS_TOKEN`) fail
  every boot with 2 retries each → ~3s of startup spent on doomed connects. Cosmetic/config.
- **Telemetry duplication**: two samplers — orchestrator nvidia-smi actor (works) + hub build_profiler
  (Null). Inconsistent source of truth (§0).
- **Background loops**: HRA telemetry hub (5s), HRA snapshot loop (on hub change), co-residency reclaim
  (30s), idle-release monitor (10s), reminders (30s), perception loop, health (30s). All bounded; no
  busy-wait observed. The 5s hub poll does an extra `spawn_blocking` sysinfo + CPU sample (100ms
  sleep) — minor.

---

## 7. Bug list (ranked)

| ID | Bug | Severity | Prob | Issue |
|---|---|---|---|---|
| B1 | `build_profiler()` Null (0 VRAM) under `--no-default-features` — no nvidia-smi fallback | CRITICAL | 100% | #3, HRA blind |
| B2 | Lease recovery degrades on 0-VRAM telemetry (CriticalOomRisk) → blocks image/voice/vision | CRITICAL | 100% | #3 |
| B3 | `LlmSwapFailed` not forwarded (`Ok(_)=>{}`) + no UI `swap_failed` listener → overlay stuck | HIGH | 100% on fail | #2 |
| B4 | GPU scale-up to ngl=36 overcommits ~4.4 GB free (no CUDA-overhead margin, uses total) → OOM/timeout | HIGH | high | #2/#4 |
| B5 | (FIXED this session) failed swap left no server → LLM down | CRITICAL | — | #2 |
| B6 | HRA DeviceTable free never refreshed when snapshot has no GPU → stale bootstrap VRAM | HIGH | 100% this build | admission |
| B7 | OOM backoff steps by ≥2 ngl/failure → minutes of swap thrash to converge | MEDIUM | high | #2/#4 |
| B8 | Orchestrator/LLM start deferred behind ~15.6s init incl. ~11s tool-index build | MEDIUM | 100% | #1 |
| B9 | Degraded lease is sticky (no clear on acquire path) | MEDIUM | med | #3 |
| B10 | Vision sidecar venv missing deps (fastapi) → crash-loop + restart churn | MEDIUM | 100% this env | env |
| B11 | MCP colab/github fail every boot (bad python path / missing token) | LOW | 100% this env | config |
| B12 | Two telemetry samplers (nvidia-smi + Null hub) → inconsistent VRAM truth | HIGH | 100% this build | §0 |

---

## 8. Production-hardening plan (priority order — for the NEXT phase, not done here)

1. **B1/B2/B12 (telemetry truth):** make `build_profiler` fall back to nvidia-smi (or have the hub +
   shared lease consume the orchestrator's working telemetry). One source of truth, never 0-when-GPU-present.
2. **B3 (UI):** forward `LlmSwapFailed`; add UI `swap_failed`/`error` listeners that clear `isSwapping`;
   add a watchdog/UI timeout to auto-clear the overlay.
3. **B4/B6/B7 (sizing):** size against live free VRAM + a CUDA-overhead margin; pick a fitting ngl
   directly; refresh DeviceTable free even on empty-GPU snapshots (or don't trust empty as 0).
4. **B9 (degraded sticky):** auto-clear degraded when telemetry is unknown/healthy on next acquire.
5. **B8 (startup):** start LLM spawn in parallel with the tool-index/embedding warmup.
6. **B10/B11 (env):** fix the Python venv (install sidecar requirements) and MCP config (anaconda
   python path, GitHub token) — environmental, user-side.

---

## 9. Hardware test plan (manual, after fixes)

- Confirm `build_profiler` reports real VRAM under `--no-default-features` (log should NOT say "0 free
  VRAM" when an NVIDIA GPU is present).
- Image gen 10× back-to-back → no `GuardReleasedAwaitingTelemetry`, lease returns to Idle.
- Force a GPU swap failure → overlay clears, LLM recovers on CPU, no stuck "Optimizing".
- Measure time-to-first-token from launch (target < ~10s after window).
- Run the manual `llama-server --n-gpu-layers 36` command to classify OOM vs slow-load (decides B4 fix).
- 12h soak: watch VRAM oscillation, swap count, journal growth, thread count.

---

## 10. Verdict

The three symptoms share a small set of real, code-grounded root causes — dominated by **B1/B2**: on
the `--no-default-features` build the VRAM profiler is blind (0), which deterministically degrades the
shared lease (Issue 3) and corrupts HRA's view, while **B3** (missing swap-failure event/listener)
strands the UI overlay (Issue 2) and **B4/B8** explain the slow/failed GPU path (Issue 1). None are
mysteries; all have a clear fix path for the next hardening phase.
