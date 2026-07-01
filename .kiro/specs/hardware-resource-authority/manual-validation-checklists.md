# HRA Manual Validation Checklists (hardware-in-the-loop)

These are the checks that require the user's physical machine (GPU/mic/speakers) and cannot be run
headless. Run them in order. Two run modes:

- **Shadow** (default): `cargo tauri dev` — HRA observes/advises only; legacy lease owns. Must be
  byte-for-behavior identical to before this work.
- **Enforce**: `KRIA_HRA_ENFORCE=1 cargo tauri dev` — HRA owns GPU admission/co-residency. This is
  the cutover validation. Rollback = unset the env var.

Filter logs to the authority with `target:"hra"` (and `watchdog`, `hardware`).

---

## 0. Pre-flight (regression — headless, already green)
- [ ] `cargo check -p kria-core` and `-p kria-desktop` pass.
- [ ] `cargo test -p kria-core --lib resource` → 158 pass.
- [ ] `cargo test -p kria-core --test hra_acceptance` → 9 pass.
- [ ] `cargo test -p kria-core --test hra_stress` → 6 pass.
- [ ] `cargo test -p kria-core --test hra_bench -- --nocapture` → 3 pass; note the latency numbers.

## 1. Shadow baseline (no behavior change expected)
- [ ] Launch `cargo tauri dev`. App boots; chat works as before.
- [ ] Settings → Hardware: Resource Dashboard renders. Overview shows GPU VRAM, CPU%, RAM.
- [ ] Telemetry line shows `source: unified_hub` (confirms single sampler).
- [ ] Explainability lists decisions with plain-language "why".
- [ ] Forecasting shows a VRAM forecast (stable or time-to-exhaustion).
- [ ] Logs show `HRA: enforcement SHADOW`. No `HRA VETO` lines (shadow never vetoes).
- [ ] 20 min of normal use: no new errors, no swap-spawn loop, no degraded lease.

## 2. LLM checklist
- [ ] First token latency feels unchanged vs before (shadow).
- [ ] Long chat turn: no mid-answer "Optimizing GPU layers" interruption (foreground guard).
- [ ] Idle for `idle_release_after_secs`: model unloads (idle re-enabled); next prompt reloads cleanly.
- [ ] Enforce mode: trigger a near-OOM scale-up → expect `HRA VETO` line, model stays on a fitting
  size, NO OOM restart loop.

## 3. Image checklist
- [ ] Generate an image while idle: completes; no `GuardReleasedAwaitingTelemetry` degrade.
- [ ] Generate an image **while a chat answer streams**: input stays usable; calm "optimizing in
  background" notice (not a hard block).
- [ ] Enforce mode: during image gen, Session-Awareness view shows BOTH llm + image as co-residents
  if VRAM allows, OR a preemption decision in Explainability if it doesn't.
- [ ] Repeat image gen 10× back-to-back: no VRAM leak (Overview free-VRAM returns to baseline), no
  stuck swap.

## 4. Voice checklist
- [ ] STT: speak a phrase → transcribed; latency acceptable.
- [ ] TTS: assistant speaks; no audio glitch.
- [ ] Voice **while chatting**: wake word still responsive; no deadlock.
- [ ] Voice interruption ("stop"): aborts current TTS promptly.
- [ ] Confirm voice CPU spike (whisper threads) is within expectation — this is STT thread count, not
  the GPU orchestrator (tune `-t` separately if needed).

## 5. Co-residency checklist (enforce mode)
- [ ] LLM + STT concurrently → both function; Session view shows co-residents.
- [ ] LLM + Image → co-reside if VRAM fits; else fg preempts bg (Explainability shows "preempted").
- [ ] LLM + Voice → realtime voice not starved.
- [ ] Image + Vision → both run or queue without deadlock.
- [ ] Low-VRAM device: confirm graceful CPU fallback (no hang) when GPU can't fit.
- [ ] No duplicate residency: a model never appears twice in the residents list.

## 6. Recovery checklist
- [ ] Kill the app mid-chat (hard kill). Relaunch.
- [ ] On boot, logs show journal recovery; if a prior lease was open, a reclaim/`recovered` line.
- [ ] No orphan llama-server/ComfyUI process left holding VRAM after relaunch (check `nvidia-smi`).
- [ ] Recovery view shows the epoch incremented (fencing) after restart.
- [ ] Leave a consumer idle past lease TTL (enforce) → co-residency sweep logs a reclaim; VRAM frees.

## 7. CPU / pressure soak
- [ ] Under sustained multi-consumer load, Resource Pressure view transitions ok → soft → hard
  correctly as free VRAM drops; recovers when load releases.
- [ ] No runaway CPU from the telemetry hub (5s cadence) or the 30s reclaim sweep.

## 8. Enforce cutover sign-off (gates legacy deletion)
- [ ] 30–60 min enforce soak across chat/voice/image: zero deadlocks, zero OOM loops, zero stuck
  swaps, zero mid-answer interrupts (non-emergency).
- [ ] Shadow-gate / divergence: HRA decisions matched what actually happened (sane placement).
- [ ] If all above pass → the consumer cutover is validated; legacy `GpuLeaseManager` +
  orchestrator `TelemetryActor` can be deleted (Phase 3) and enforce made default.

## 9. Multi-GPU (if available)
- [ ] Two GPUs: two large consumers land on two distinct devices (no over-commit on one).
- [ ] Preemption stays device-local.

---

### Rollback at any point
Unset `KRIA_HRA_ENFORCE` and relaunch → instant return to the legacy-owned (shadow) path. No code
change or data migration required.
