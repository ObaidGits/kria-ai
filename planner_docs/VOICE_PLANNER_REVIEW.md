# KRIA Voice Runtime Plan — Critical Evaluation

> **Reviewer:** Principal Real-Time Voice Runtime Engineer
> **Date:** 2026-05-13
> **Reviewed Document:** `planner_docs/VOICE_PLANNER.md`
> **Hardware Target:** RTX 4050 6GB VRAM, Linux desktop, local-first

---

## 1. Executive Verdict

The plan correctly identifies that the v2 concurrency skeleton is sound and that CLI subprocesses are the dominant latency source. The cancellation chain, FSM states, and sentence-level streaming architecture are all well-designed — verified against the actual `pipeline.rs` (1266 lines), `playback.rs` (185 lines), and test suite (5 tests, including barge-in latency assertion).

**However, the plan overreaches in several places:**

- The rolling-window STT strategy underestimates CPU cost on the target hardware
- AEC is treated as a prerequisite for barge-in — it's not; half-duplex barge-in via push-to-talk is valid and ships first
- Several "Phase 3/4" items (conversational filler, adaptive endpointing, phrase caching) are premature optimisation with poor ROI
- The plan lacks bounded resource contracts — no hard limits on concurrent inference, channel depths under backpressure, or thermal throttling

**Overall:** ~70% of the plan is directly actionable. ~20% needs modification. ~10% should be deferred or rejected.

---

## 2. Issue-by-Issue Evaluation

### 🔴 HIGH SEVERITY

---

#### H1: `WhisperRsStt` rolling-window every 500ms may overload CPU

**Valid?** ✅ **Yes — this is a real and underestimated risk.**

On RTX 4050 (6GB), LLM occupies GPU. Whisper-rs on CPU with `ggml-large-v3-turbo-q5_0` takes ~1.5-3s per 2.5s window on 4 threads. A fixed 500ms cadence means inference N is still running when inference N+1 is scheduled. This creates unbounded inference queue growth.

**Proposed fix (adaptive cadence):** Partially correct but underspecified.

**Better fix:** Don't infer on a fixed timer. Infer **once per VAD activity window**:

1. VAD `SpeechStart` → begin accumulating audio into ring buffer
2. Every time VAD reports continued speech AND the previous inference has completed AND ≥500ms of new audio has accumulated → trigger inference
3. VAD `SpeechEnd` → trigger final pass on complete buffer
4. Never run two concurrent whisper inference passes

Implementation: a simple `AtomicBool` guard (`inference_running`) checked before scheduling. No queue — skip the partial if the previous one isn't done.

```
cadence = max(500ms, actual_inference_duration + 100ms)
```

This is self-regulating: on slow hardware, partials come less frequently but never pile up. On fast hardware (GPU whisper), you get one every ~500ms.

**Recommendation:** ✅ **Accept with modification** — use demand-driven inference, not timer-driven.

---

#### H2: Continuous listening + wake-word + STT simultaneously

**Valid?** ⚠️ **Partially valid — but the problem is overstated.**

Looking at the actual code: wake word runs on the same audio stream as VAD. The three-model openWakeWord stack (melspec → embedding → keyword) is lightweight ONNX inference — ~2ms per 80ms frame on CPU. This is negligible.

The real resource conflict is: **wake word + VAD + STT all consuming from the same broadcast channel**. But STT only consumes frames *after* wake/VAD triggers a turn. They're sequential in the current `start_voice_v2_loop` — `run_turn` starts, consumes audio, completes, loops.

**The plan's current architecture already handles this correctly** via the turn loop in `voice_runtime_helpers.rs:407-506`. There's no simultaneous STT + wake because the loop is serial: wake → run_turn → sleep → wake.

**Proposed fix (dynamic pipeline throttling):** Overengineered for the current architecture.

**Better fix:** The existing serial loop is correct. Only concern: when `run_turn` is in Speaking state and the echo gate drops frames (line 351-359), wake word detection is also disabled. This is fine — you can't wake KRIA while it's already talking to you. If AEC is later enabled, wake word naturally resumes during Speaking.

**Recommendation:** ❌ **Reject** — the current serial loop already prevents resource conflict. No throttling states needed. Add a comment documenting the invariant.

---

#### H3: AEC dependency complexity underestimated

**Valid?** ✅ **Yes — but for different reasons than stated.**

The actual complexity isn't the build system (clang+cmake are standard on Linux dev machines). The real issues:

1. **Sample rate mismatch:** Capture is 16kHz, playback is 22050Hz. WebRTC APM requires matching rates on capture and render paths. You need a resampler (22050→16000) on the render reference path. This is non-trivial to get right without introducing latency.

2. **Frame alignment:** WebRTC APM expects fixed 10ms frames (160 samples at 16kHz). Capture chunks are ~100ms. You need a frame splitter on the capture path.

3. **Delay estimation:** AEC needs to know the round-trip delay (capture → speakers → mic). This varies by hardware. Incorrect delay = AEC does nothing or makes things worse.

**Proposed fix (fallback half-duplex):** ✅ Correct — this is exactly right.

**Better fix:** Define three runtime modes explicitly:

| Mode | AEC | Echo Gate | Barge-in | When |
|------|-----|-----------|----------|------|
| `half_duplex` | Off | On (mute mic during play) | Push-to-talk only | Default, always works |
| `aec_duplex` | On | Off | VAD-triggered | `voice.aec.enabled = true` + feature compiled |
| `headphone` | Off | Off | VAD-triggered | `voice.aec.mode = "headphone"` (no echo path) |

The "headphone" mode is zero-cost and enables barge-in immediately for users with headsets — no AEC needed. This should ship in Phase 1, not Phase 3.

**Recommendation:** ✅ **Accept with modification** — add headphone mode, defer AEC to Phase 3, but ship headphone-barge-in in Phase 1.

---

#### H4: Sentence-prefetch may race cancellation/barge-in

**Valid?** ⚠️ **Partially valid — but the existing code already handles this correctly.**

Looking at `pipeline.rs:502-543`, the TTS task loop has **cooperative cancel checks between every sentence**:

```rust
for sentence in splitter.push(&tok) {
    if tts_token.is_cancelled() { break 'outer; }
    // synthesize...
    if tts_token.is_cancelled() { break 'outer; }
}
```

And synthesis itself observes `abort_rx` for mid-sentence cancellation. The `PlaybackSink::abort()` clears the rodio queue. So even if sentence N+1 is being synthesized while N plays, cancellation propagates correctly: abort_rx fires → synth stops → pcm_tx closes → drain task exits.

**Where the concern IS valid:** If we add *true* prefetch (synthesize N+1 in a separate task while N plays), then PCM chunks from the stale sentence could arrive at the playback sink after abort. The current architecture doesn't have this problem because synthesis is serial in the `'outer` loop.

**Proposed fix (generation IDs):** Correct for the prefetch case but premature now.

**Better fix for Phase 2:** When adding prefetch, tag each `pcm_tx.send()` with a monotonic `turn_generation: u64`. The playback drain checks `current_generation` before queuing to rodio. Stale chunks are silently dropped. This is a 5-line change to the `Vec<f32>` → `(u64, Vec<f32>)` channel type.

**Do not add generation IDs now** — the serial loop doesn't need them, and premature abstraction adds complexity to every channel consumer.

**Recommendation:** ⏸️ **Defer** — revisit when implementing sentence prefetch in Phase 2. Document the invariant that serial synthesis + cooperative cancel is sufficient for Phase 1.

---

### 🟠 MEDIUM SEVERITY

---

#### M1: Persistent playback sink lifecycle not deeply specified

**Valid?** ✅ **Yes — `begin_session` creates a new `OutputStream` per turn.**

Looking at `playback.rs:93-145`, `begin_session` calls `player.play_samples()` which internally creates a new `OutputStream`. The `PlaybackSink` itself is persistent (lives in `Arc<Mutex<PlaybackSink>>`), but the underlying rodio resources are per-session.

**Proposed fix (sink session supervisor):** Overengineered.

**Better fix:** Hoist the `OutputStream` and `Sink` into the `PlaybackSink` struct. Create once at pipeline start via `set_audio_player`. `begin_session` just resets flags and returns the `pcm_tx`. `abort` calls `sink.clear()` instead of dropping the stream.

This is a localised refactor to `playback.rs` + `AudioPlayer`. No supervisor needed — the `Mutex<PlaybackSink>` already serialises access. If the output device disconnects, detect via rodio error in the drain loop, log, and mark the sink as failed. The next `begin_session` can try to reopen.

**Recommendation:** ✅ **Accept with modification** — simple hoist, no supervisor abstraction.

---

#### M2: Real-time thread priority discussed but not bounded

**Valid?** ✅ **Yes — and the fix is correct as stated.**

Only the CPAL capture callback needs RT priority, and CPAL already requests it on Linux via `SCHED_FIFO` on the audio thread if the process has `CAP_SYS_NICE` (or the user is in the `audio` group). The plan suggesting RT priority for STT/TTS/playback threads is wrong — these are compute-bound, not latency-bound.

**Proposed fix (restrict RT to capture only):** ✅ Exactly right.

**Additional note:** On PipeWire/PulseAudio Linux, CPAL's callback thread already runs at the audio server's priority. Manually setting `SCHED_FIFO` can actually cause priority inversions with PipeWire. Leave it to CPAL/PipeWire.

**Recommendation:** ✅ **Accept as stated** — and add a note to NOT manually elevate priority on PipeWire systems.

---

#### M3: Wake-word always-on + continuous STT duplication

**Valid?** ⚠️ **Marginal concern — see H2 analysis.**

Wake word and STT never run simultaneously in the current architecture. Wake word runs during `Sleeping` state; STT runs during `Listening→Transcribing`. No duplication.

**Proposed fix (separate modes):** This is already the reality:
- `Sleeping` + wake word enabled = wake mode
- `Listening` → `run_turn` = conversation mode

No architectural change needed.

**Recommendation:** ❌ **Reject** — already implemented correctly via the FSM states.

---

#### M4: No thermal/power-awareness

**Valid?** ✅ **Yes — but the fix should be minimal.**

On a laptop with RTX 4050, sustained voice interaction will thermal-throttle the GPU. The LLM will slow down, TTFA will increase, and `OverrunTracker` will fire `voice:degraded`. The existing degradation signal is the right hook.

**Proposed fix (lightweight thermal degradation policy):** Partially correct but too broad.

**Better fix:** The `OverrunTracker` already fires after 3 consecutive overruns. Wire this to:

1. Downgrade `VoiceTier` one level (S→A→C) for the next N turns
2. Log the degradation
3. Auto-recover after M consecutive good turns

This is 20 lines in the turn loop. No thermal sensor polling, no sysfs reads, no power management API. The TTFA metric IS the thermal proxy — if inference is slow, TTFA overruns, tier drops, lighter models are used, inference gets faster. Self-correcting.

**Recommendation:** ✅ **Accept with modification** — use existing `OverrunTracker` as the trigger, not thermal sensors.

---

#### M5: Partial transcript overwrite behavior underspecified

**Valid?** ✅ **Yes — minor but real.**

The `PartialTranscript.text` is "cumulative best-guess text since SpeechStart". But when the UI receives two partials "hel" → "hello wo", it replaces the displayed text. If a partial arrives *after* the final (race condition on the telemetry channel), the UI could flash stale text.

**Proposed fix (stable-partial reconciliation):** Overengineered for the current architecture.

**Better fix:** Add a monotonic `seq: u32` to `PartialTranscript` and `FinalTranscript`. The UI only displays if `seq > last_displayed_seq`. The final always has `seq = u32::MAX`. This is a 3-field addition.

For the channel race: the `partial_pump` task is aborted before `emit_final` is called (line 436). So the race doesn't actually exist in practice. The `seq` field is a defense-in-depth measure, not a critical fix.

**Recommendation:** ✅ **Accept with modification** — add `seq` field, but priority is low. Slot into Phase 1 as a 10-minute change.

---

#### M6: Turn arbiter semantics not deeply defined

**Valid?** ⚠️ **Marginal — the current code IS the turn arbiter.**

Looking at `voice_runtime_helpers.rs:407-506`:

```rust
while voice_active_loop.load(...) {
    v2_loop.force_wake("auto");
    let audio_rx = bt_loop.subscribe();
    // ... build LLM closure ...
    v2_loop.clone().run_turn(audio_rx, llm).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
}
```

This is a serial loop. One turn at a time. `run_turn` owns the turn from start to finish. There is no concurrent turn contention. The `turn_cancel` mutex in `VoicePipelineV2` replaces the previous turn's token atomically (line 346-348).

**Proposed fix (explicit turn-ownership state machine):** The FSM already IS the turn-ownership state machine: `Sleeping → Listening → Transcribing → Thinking → Speaking → Sleeping`. Each state is exclusive. The `CancellationToken` is the ownership token.

Where the concern WOULD be valid: if external commands (`voice_v2_speak`, `force_abort`) can race with `run_turn`. Looking at the code:
- `force_abort` cancels the current turn token → `run_turn` observes and exits
- `voice_v2_speak` calls `run_speak_turn` — but this is called from a different Tauri command, not from the v2 loop

**Real risk:** If a user clicks "speak" via Tauri while the v2 loop is mid-turn, both `run_turn` and `run_speak_turn` could race on `turn_cancel`. The mutex serialises the token swap, but the *meaning* is unclear — which turn wins?

**Better fix:** Add a `Mutex<()>` turn guard: `run_turn` and `run_speak_turn` both `try_lock` at entry. If locked, the late caller gets `Err("turn already active")`. This is cheaper and more correct than a full state machine redesign.

**Recommendation:** ✅ **Accept with modification** — add a try_lock guard, not a new state machine.

---

#### M7: "Conversational filler" feature risky

**Valid?** ✅ **Yes — this is a UX antipattern for local assistants.**

Pre-recorded fillers ("hmm", "let me check") work for cloud assistants because their TTFA is 1-3s. For a local assistant targeting 500ms TTFA, playing a 500ms filler delays the actual response by the filler duration. If the filler plays AND the response arrives fast, the user hears "hmm... Hello!" — jarring.

**Proposed fix (optional + rate-limited):** Insufficient mitigation.

**Better fix:** Reject entirely for Phase 1-3. If TTFA consistently exceeds 2s on C-tier hardware, consider a VISUAL "thinking" indicator (already exists: `voice:state → processing`), not an audio filler. Audio fillers should only be reconsidered if user research specifically requests them.

**Recommendation:** ❌ **Reject** — audio fillers are anti-UX for sub-1s TTFA targets. Use existing visual indicators.

---

#### M8: Voice continuity context could grow endlessly

**Valid?** ✅ **Yes — real issue in `start_voice_v2_loop`.**

Looking at the LLM closure (line 458-468):
```rust
let recent_turns = memory_turn
    .get_recent_turns(&session_id, 5)
    .unwrap_or_default();
```

Capped at 5 turns. This is already bounded. The plan's concern is unfounded for the current implementation.

**However:** If the user has a long voice session (50+ turns), the session_id stays the same, and `get_recent_turns(5)` returns the 5 most recent. This is correct behavior. The context doesn't grow — it's a sliding window.

**Proposed fix (bounded rolling context):** Already implemented.

**Recommendation:** ❌ **Reject** — already bounded at 5 turns via `get_recent_turns`.

---

### 🟡 LOW SEVERITY

---

#### L1: 16kHz hardcoded capture

**Valid?** ⚠️ **Not a real issue.**

16kHz is the standard for speech recognition (whisper, Kaldi, all major ASR). Higher sample rates add no benefit for speech and double memory/compute. CPAL handles device-native rate → 16kHz resampling internally when available.

**Proposed fix (resampler abstraction):** Unnecessary complexity.

**Recommendation:** ❌ **Reject** — 16kHz is correct for speech. No resampler needed.

---

#### L2: Phrase caching for TTS

**Valid?** ⚠️ **Marginal benefit.**

Common phrases ("Sure!", "Done.", "Here you go.") are short sentences. Piper synthesises a 3-word sentence in ~50ms in-process. Caching saves 50ms but adds cache invalidation complexity (voice change, speed change, etc.).

**Proposed fix (cache ultra-common short phrases):** Low ROI.

**Better fix:** Skip entirely for now. If profiling shows TTS is a bottleneck after piper-rs is wired, revisit. The bottleneck will be cold-load (fixed by persistence) and long sentences (fixed by sub-sentence chunking), not short phrases.

**Recommendation:** ⏸️ **Defer indefinitely** — premature optimisation.

---

#### L3: Wake-word cooldown static

**Valid?** ✅ **Minor but real.**

500ms cooldown is fine. Adaptive cooldown adds complexity for negligible UX improvement.

**Recommendation:** ❌ **Reject** — static 500ms is fine. Revisit only if users report double-trigger issues.

---

#### L4: TTFA targets optimistic on local hardware

**Valid?** ✅ **Yes — the most grounded concern in the low-severity category.**

Current TTFA targets:
- S: 500ms — Achievable with in-process whisper-rs (CPU) + piper-rs + hot LLM on RTX 4050. Tight but possible.
- A: 800ms — Comfortable with the above stack.
- C: 1200ms — CPU-only. Feasible with ggml-small + piper + quantized LLM.

The S-tier 500ms target assumes:
- VAD end → STT final: ~200ms (whisper-rs CPU, 2s utterance, 4 threads)
- STT final → LLM first token: ~100ms (hot model on GPU)
- LLM first token → sentence complete: ~100-200ms (depends on response)
- Sentence → first TTS chunk: ~100ms (piper-rs in-process)
- **Total: ~500-600ms**

This is tight but realistic on RTX 4050 with everything in-process and pre-loaded. The plan's 500ms target is aggressive but not unrealistic — it's a *budget* (overruns are tracked, not fatal).

**Proposed fix (define realistic S/A/B tiers):** The existing S/A/C tiers are fine. Adding a B tier between A and C adds configuration complexity with no clear UX benefit. The `OverrunTracker` handles the gray zone.

**Better fix:** Keep S/A/C. Adjust S-tier budget to 700ms for Phase 2 (CLI engines still partially used), tighten to 500ms in Phase 3 when all engines are in-process. Make the budget per-phase, not per-architecture.

**Recommendation:** ✅ **Accept with modification** — phase-dependent budgets, keep S/A/C tiers.

---

## 3. Disposition Summary

| ID | Issue | Verdict | Action |
|----|-------|---------|--------|
| **H1** | Rolling-window CPU overload | ✅ Accept | Modify: demand-driven inference with AtomicBool guard |
| **H2** | Simultaneous wake+VAD+STT | ❌ Reject | Already serial in the turn loop |
| **H3** | AEC complexity | ✅ Accept | Modify: add headphone mode for Phase 1 barge-in |
| **H4** | Prefetch race with cancellation | ⏸️ Defer | Revisit when implementing sentence prefetch |
| **M1** | Playback sink lifecycle | ✅ Accept | Modify: simple hoist of OutputStream, no supervisor |
| **M2** | RT thread priority scope | ✅ Accept | As stated; add PipeWire caveat |
| **M3** | Wake+STT duplication | ❌ Reject | Already separated by FSM states |
| **M4** | Thermal/power awareness | ✅ Accept | Modify: use OverrunTracker as trigger, not thermal sensors |
| **M5** | Partial overwrite race | ✅ Accept | Modify: add seq field, low priority |
| **M6** | Turn arbiter undefined | ✅ Accept | Modify: try_lock guard, not new state machine |
| **M7** | Conversational filler | ❌ Reject | Anti-UX for sub-1s TTFA; use visual indicators |
| **M8** | Unbounded voice context | ❌ Reject | Already bounded at 5 turns |
| **L1** | 16kHz hardcoded | ❌ Reject | Correct for speech; no change needed |
| **L2** | TTS phrase caching | ⏸️ Defer | Premature optimisation |
| **L3** | Wake-word cooldown | ❌ Reject | Static 500ms is fine |
| **L4** | Optimistic TTFA targets | ✅ Accept | Modify: phase-dependent budgets |

**Score: 7 accepted (5 modified), 6 rejected, 3 deferred.**

---

## 4. Remaining Architecture Risks Not Covered by the Table

### R1: `tokio::Mutex` on `PlaybackSink` in the barge-in hot path

`force_abort` and `spawn_barge_in_watcher` both acquire `self.turn_cancel.lock().await` and `self.playback.lock().await`. If the drain task is blocked in `player.play_samples()` (which calls `spawn_blocking`), the playback mutex is NOT held by the drain task (it runs in the spawned future, not under the mutex). So this is safe.

**But:** `begin_session` holds the playback mutex while spawning the drain task. If `begin_session` and `abort` race, the mutex serialises them correctly. No issue found — just document the invariant.

**Risk level:** Low. No action needed.

### R2: Broadcast channel lag in `start_voice_v2_loop`

The broadcast channel has capacity 128. If the pipeline is in `Thinking` state (LLM generating) and audio frames arrive at 10 per second (100ms chunks), the channel fills in ~12.8 seconds. After that, new subscribers get `RecvError::Lagged`. The capture task handles this with `continue` (line 394).

But: the *next* `run_turn` subscribes to the broadcast *after* the previous turn completes. If frames accumulated during Thinking/Speaking, the new subscriber starts from the current tail — no lag. This is correct.

**Risk level:** Low. No action needed.

### R3: `CliWhisperStt` buffers unbounded audio

`CliWhisperStt::start_stream` buffers ALL frames into a `Vec<f32>` (line 177: `Vec::with_capacity(16_000 * 30)` = 30 seconds). If VAD never fires `SpeechEnd` (stuck open), this grows unbounded. At 16kHz × 4 bytes × 60s = ~3.8 MB/minute — manageable but should be capped.

**Fix:** Add a hard cap at 60 seconds of audio (960,000 samples). After that, close `pcm_rx` and process what we have. This prevents OOM on pathological input.

**Risk level:** Medium. Add to Phase 1.

### R4: No graceful shutdown of the v2 loop

`start_voice_v2_loop` runs `while voice_active_loop.load(...)`. When `stop_voice` sets `voice_active = false`, the loop checks this AFTER `run_turn` completes. If `run_turn` is blocked waiting for STT (whisper-cpp timeout = 45s), the loop won't exit for up to 45 seconds.

**Fix:** `stop_voice` already calls `force_abort` (line 616-617). This cancels the turn token, which causes `run_turn` to bail at the next `select!` branch. The STT handle's `abort` is sent via the `StreamHandle::abort` oneshot. `CliWhisperStt` observes this via `abort_rx` in its loop (line 182-184). So shutdown IS propagated — but the whisper-cpp subprocess may still be running. The subprocess is NOT killed by the oneshot — only the Rust task stops waiting.

**Real fix:** `CliWhisperStt` should kill the child process on abort. Add `child.kill()` in the abort path.

**Risk level:** Medium. Add to Phase 1.

### R5: `run_speak_turn` and `run_turn` share no mutual exclusion

As noted in M6. If the Tauri UI calls `voice_v2_speak` while the continuous loop is running, two turns race. The `turn_cancel` mutex prevents token corruption, but the FSM state becomes incoherent (both turns set states independently).

**Fix:** `try_lock` guard as recommended in M6.

**Risk level:** High if both paths are exposed simultaneously. Medium in practice (v2 loop and voice_v2_speak are typically not active together).

---

## 5. Final Refined Recommendations

### Phase 1 (Immediate — before any engine work)

1. **Headphone mode** — add `voice.aec.mode = "headphone"` config option. When set, disable echo gate, enable VAD-triggered barge-in. Zero-cost, enables barge-in for headset users immediately.

2. **Turn mutual exclusion** — add `Mutex<()>` try_lock in `run_turn` and `run_speak_turn` entry points.

3. **STT buffer cap** — hard limit `CliWhisperStt` buffer at 60 seconds.

4. **Subprocess kill on abort** — `CliWhisperStt` must kill the whisper-cpp child process when the abort oneshot fires.

5. **Persistent OutputStream** — hoist rodio `OutputStream` from per-session to per-pipeline lifetime in `PlaybackSink`.

6. **`seq` field on partials** — monotonic sequence number on `PartialTranscript` and `FinalTranscript`.

### Phase 2 (Engine integration)

7. **Demand-driven STT partials** — implement whisper-rs with `AtomicBool` inference guard, not fixed-timer cadence.

8. **Phase-dependent TTFA budgets** — S-tier starts at 700ms during Phase 2, tightens to 500ms in Phase 3.

9. **OverrunTracker → tier degradation** — wire 3-overrun signal to auto-downgrade tier for N turns with auto-recovery.

### Phase 3 (Full-duplex — only after AEC is proven)

10. **AEC integration** — WebRTC APM with 22050→16000 resampler on render path.

11. **Remove echo gate** — switch from `half_duplex` to `aec_duplex` mode.

### Rejected / Deferred

- ~~Dynamic pipeline throttling states~~ (H2) — already serial
- ~~Generation IDs on chunks~~ (H4) — premature; revisit with prefetch
- ~~Sink session supervisor~~ (M1) — simple hoist suffices
- ~~Conversational filler~~ (M7) — anti-UX
- ~~Bounded rolling context~~ (M8) — already bounded
- ~~16kHz resampler~~ (L1) — correct for speech
- ~~TTS phrase caching~~ (L2) — premature optimisation
- ~~Adaptive wake cooldown~~ (L3) — static 500ms is fine
- ~~Separate wake/conversation mode~~ (M3) — FSM already handles this

---

*End of Critical Evaluation*
