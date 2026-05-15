# KRIA Voice Runtime — Final Consolidated Implementation Plan

> **Status:** Single Source of Truth
> **Date:** 2026-05-13
> **Target:** Assistant-grade realtime voice interaction (local-first)
> **Hardware:** RTX 4050 6GB / 16GB RAM / Linux desktop
> **Goal:** Siri/Gemini/Assistant-class UX via bounded, deterministic, interruptible runtime

---

## 1. Executive Summary

KRIA's v2 voice pipeline has a **sound concurrency skeleton** — the `CancellationToken` chain, 6-state FSM, sentence-level streaming, and barge-in architecture are correctly designed and tested. The critical gap is that every engine node (STT, TTS) is either a CLI subprocess or a compile-time stub. Until in-process engines replace them, assistant-grade latency (<500ms TTFA) is impossible.

**Current state:** v1 default (sequential, blocking, ~2-5s TTFA). v2 scaffolded (1266-line orchestrator, 5 integration tests, CLI fallback engines).

**Path forward:** Four phases — runtime hardening → in-process engines → advanced orchestration → UX polish. Serial-first, bounded, no concurrency explosion.

**Non-goals:** AGI, autonomous cognition, emotional AI, recursive reasoning, cloud-first, speculative orchestration.

---

## Phase 1 Live Implementation Tracker

### DONE

- Baseline audit started for:
  - v2 turn lifecycle (`run_turn`, `run_speak_turn`)
  - playback ownership (`PlaybackSink`, `AudioPlayer`)
  - abort/cancellation flow (`force_abort`, turn token path)
  - Tauri/UI telemetry mapping (`voice_runtime_helpers.rs`, `ui/src/stores/app.ts`)
- F2 strict turn mutual exclusion implemented:
  - `VoicePipelineV2.turn_guard` added and enforced via `try_lock()` in both `run_turn` and `run_speak_turn`
  - overlapping turn calls now fail fast with explicit error (`voice runtime busy: active turn in progress`)
  - explicit runtime telemetry emitted on rejection (`VoiceTelemetry::BusyRejected`)
  - Tauri event mapping added (`voice:busy`)
  - UI busy indicator path added (`voiceState = "busy"` with bounded revert)
- Phase 1 test added and passing:
  - `voice::v2::pipeline::tests::concurrent_turn_rejected_with_busy_telemetry`
- F5 centralized abort propagation implemented:
  - `abort_root(reason, emit_barge)` now owns root cancellation propagation (token cancel + playback abort + interruption telemetry)
  - `force_abort` and active turn cancel branches route through the same path
  - turn finalization centralized via `finalise_turn_to(...)`
- F1 subprocess cleanup correctness implemented:
  - CLI STT now uses abortable command execution with explicit `child.kill().await` + `child.wait().await` on cancel/timeout
  - `CliWhisperStt` abort bridge wired to cancellation token
  - STT stream handle is now explicitly aborted and bounded-wait joined during turn cancellation
  - STT audio buffer capped at 60s to prevent pathological growth
- F4/F8 persistent playback lifecycle + recovery implemented:
  - `AudioPlayer` moved to dedicated playback worker thread with persistent `OutputStream` + single active `Sink`
  - runtime invalidation + lazy reopen on failure
  - `PlaybackSink` retries once after invalidation and emits failure/recovery runtime events
  - abort now stops active sink deterministically via `player.stop_now()`
- F7 headphone-mode runtime implemented:
  - `start_voice_v2_loop` now honors `voice.mode = "headphone"`
  - speaking-state mic gate is bypassed only in headphone mode (half-duplex remains default)
  - UI receives `voice:io_mode` for microphone/headphone indicator state
- F6 partial transcript stabilization implemented:
  - `PartialTranscript` now has deterministic `seq`
  - pipeline partial pump enforces monotonic seq ownership and drops stale out-of-order updates
  - UI partial listener applies bounded frequency updates and seq ordering guard
- Phase 1 TTFA instrumentation expanded:
  - `VoiceMetrics` now includes mic/VAD/STT/LLM/TTS/playback milestone fields
  - builder marks now wired in v2 turn path
  - UI receives TTFA (`t_first_audio_out_ms`) via telemetry and shows it in overlay metadata
- Runtime telemetry/events extended:
  - added interruption, playback failure, playback recovered, busy rejected telemetry surfaces
  - mapped to Tauri events: `voice:interruption`, `voice:playback_failure`, `voice:playback_recovered`, `voice:busy`
  - UI indicators wired for busy/interruption/playback health/io mode
- Stable state/cancellation hardening:
  - `set_state` now suppresses duplicate emissions to reduce flicker
  - barge-in and force-abort cancellation waits remain bounded
- Additional tests added and passing:
  - `voice::metrics::tests::builder_records_phase1_milestones`
  - `voice::v2::stt::tests::partial_transcript_seq_is_monotonic_field`
  - existing regression tests re-run and passing: partial ordering + barge-in + overlap rejection
- Live-runtime deadlock root cause fixed (2026-05-14):
  - v2 capture forwarder previously exited permanently when `broadcast::Sender::send` saw zero subscribers at startup.
  - this caused turns to block indefinitely at `stt_handle.join()` with no frames delivered to STT.
  - forwarder now ignores/no-ops that condition and keeps streaming until a turn subscriber attaches.
- Added transient debug path for STT isolation:
  - env flag `KRIA_VOICE_TRANSCRIPT_ONLY=1` now bypasses LLM/TTS and returns transcript-only response text.
  - new Tauri command `voice_transcribe_audio_file(path)` transcribes a WAV file directly via configured whisper path.
- Added upload-driven STT debug path:
  - new Tauri command `voice_transcribe_uploaded_audio(name, bytes)` for UI file uploads (no direct filesystem path needed).
  - chat attach input now accepts audio files and routes them to transcript-only debug response.

### IN PROGRESS

- Live validation for microphone capture + STT-final emission after forwarder fix (collecting user runtime logs).
- Mic empty-transcript hardening pass:
  - adjusted whisper-rs final decode to avoid over-suppressing blank output on short utterances.
  - added minimum speech-audio gate before end-of-speech finalization so very short false-start captures don't finalize too early.
  - added targeted STT/capture diagnostics for first chunk + final decode buffer characteristics.
- Mic stuck-thinking root-cause pass from live logs:
  - identified long awaited partial decode path in whisper-rs stream loop causing STT finalization delays/timeouts.
  - moved partial decode off hot ingest path and bounded finalization wait for in-flight partial decode.
  - restored attach picker to unrestricted multi-format selection while preserving direct audio-upload transcription path.
- Live log root-cause update (kria_stt_logs.txt):
  - STT finalization now completes, but whisper-rs repeatedly returns `text_len=0` on mic input while upload path transcribes correctly.
  - added deterministic CLI fallback from whisper-rs on empty final decode (`transcribe_samples_abortable` on same PCM buffer).
  - when transcript remains empty, run-turn now exits with explicit STT error instead of entering thinking with empty user text.
- Follow-up fix for "first turn works, next turns empty":
  - removed cross-turn `inference_running` coupling in whisper-rs backend.
  - partial-decode in-flight tracking is now stream-local/per-turn, preventing stale previous-turn partial state from affecting subsequent turns.

### TODO

- None for Phase 1 hardening scope.

### BLOCKED

- None currently.

### DEFERRED

- Full AEC duplex path (explicitly Phase 3)
- speculative sentence prefetch / autonomous retry behavior (rejected for Phase 1)

### implementation notes

- Current v2 now enforces strict single active turn ownership at runtime entrypoints while preserving serial-first orchestration.
- `AudioPlayer` now owns a dedicated playback worker with persistent runtime and single sink ownership.
- UI now has explicit busy-reject visibility via `voice:busy` event and bounded `busy` state.
- Startup ordering hazard discovered: capture forwarding can begin before first turn subscription; broadcaster no-subscriber path must be non-fatal.
- Added no-frame watchdog (`NO_FRAME_TIMEOUT_MS`) in capture task to fail fast when no audio frames arrive, instead of hanging on STT join forever.
- Capture stream lifecycle bug fixed: `AudioCaptureHandle` now remains alive for the forwarder task lifetime (was being dropped early, causing `frame_count=0` turns).
- VAD endpointing now requires bounded minimum speech-audio before silence-finalization to reduce early-finalize empty transcript turns.

### runtime risks

- `voice:busy` uses timed UI revert; if canonical state is delayed, UI may briefly show stale local state before next backend state event.
- Playback recovery currently retries once per chunk; persistent hardware/device instability still requires operator-visible diagnostics.
- Transcript-only debug mode is env-gated and temporary; must remain disabled in normal assistant runs to preserve full voice UX.

### architectural decisions

- Keep serial-first runtime; implement strict entry guard instead of introducing parallel orchestration layers.
- Emit explicit busy rejection telemetry instead of speculative queueing/retries.

### rejected ideas

- No turn queueing in Phase 1 (adds hidden orchestration and non-deterministic turn timing).
- No background auto-retry of rejected turn requests (violates bounded deterministic runtime rule).

### test progress

- Existing v2 tests currently cover happy path, barge-in, idempotent abort, and partial ordering.
- Phase 1 overlap test passing (`concurrent_turn_rejected_with_busy_telemetry`).
- Partial ordering regression passing (`streaming_partials_pumped_to_telemetry`).
- Barge-in cancellation regression passing (`barge_in_cancels_tts_immediately`).
- New Phase 1 milestone metrics and seq field tests passing.

### latency observations

- Fresh stream open penalty removed from per-chunk playback path by persistent worker runtime.
- Telemetry now captures expanded milestone timings for runtime profiling.
- Added minimum speech-audio endpoint gate may slightly increase end-of-speech latency for very short utterances, but improves transcript reliability under microphone noise/false starts.

### playback/runtime issues discovered

- Busy/rejected turn silence issue resolved via explicit telemetry + UI event wiring.
- Playback ownership now centralized to one active sink per worker runtime with lazy reopen on failure.
- Capture forwarder startup race caused STT starvation and infinite "thinking" symptom; fixed by keeping forwarder alive when no turn subscriber is present.
- Additional starvation root-cause confirmed in live logs: capture handle drop caused stream shutdown despite "audio capture started" log.
- Async-runtime playback panic fixed:
  - removed runtime-thread blocking waits in playback stop path.
  - `PlaybackSink::abort` now aborts playback via async task (`player.stop_now().await`) instead of blocking call.
- STT debug telemetry now includes final transcript preview text in `voice:debug` (`stt_final`) for log/UI verification.

## Phase 2 Live Implementation Tracker

### DONE

- Audited current STT subprocess path and v2 wiring:
  - `voice/v2/stt.rs` currently uses `CliWhisperStt` as working path; `WhisperRsStt` is scaffold-only.
  - `voice/v2/mod.rs::build_v2_with_cli_engines` currently hard-pins CLI STT/TTS wrappers.
  - `voice/v2/pipeline.rs` partial pump already enforces monotonic `seq` handling and first-partial metric marking.
- Re-baselined repository checks before Phase 2 edits:
  - `cargo check -p kria-core`
  - `cargo check -p kria-desktop`
  - `ui npm run -s check`
- P2.1 implemented (initial production path):
  - `WhisperRsStt` now uses in-process `whisper-rs` inference (no subprocess in primary path when feature is compiled and model exists).
  - persistent model/context lifecycle added via `OnceCell<Arc<WhisperContext>>` (warm reuse across turns).
  - runtime selection in `build_v2_with_cli_engines` now prefers whisper-rs primary STT when available; deterministic CLI fallback retained when model path is missing or feature is not compiled.
- P2.2 implemented (initial bounded realtime partial loop):
  - demand-driven rolling-window partial decode cadence (~350ms trigger budget, 2.5s rolling window).
  - explicit `AtomicBool` inference guard added to enforce one active STT inference at a time.
  - bounded STT capture buffer (60s) and monotonic seq-only partial emission preserved.
  - cancellation-aware decode path wired using whisper abort callback + single abort root token bridge.
- P2.3 completed end-to-end (feature-on path validated):
  - `PiperRsTts` now integrates in-process `piper-rs` runtime with persistent synthesizer reuse.
  - bounded streamed chunk emission path added (`synthesize_streamed`, bounded chunk size/padding, bounded playback queue compatibility).
  - deterministic TTS backend selection added: piper-rs primary when available, deterministic CLI fallback retained.
  - interruption-safe chunk cancellation added via watch-to-atomic abort bridge checked per chunk emission.
  - local vendor patch landed for `piper-rs v0.1.9` ORT 2.0 compatibility (`vendor/piper-rs` + workspace `[patch.crates-io]` override).
  - feature-on compile validation now passes:
    - `cargo check -p kria-core --features voice-piper-rs`
    - `cargo check -p kria-desktop --features kria-core/voice-piper-rs`
    - `cargo test -p kria-core --lib --features voice-whisper-rs,voice-piper-rs voice::v2::pipeline::tests::happy_path_runs_through_states`
    - `cargo test -p kria-core --lib --features voice-whisper-rs,voice-piper-rs voice::v2::pipeline::tests::barge_in_cancels_tts_immediately`
    - `cargo test -p kria-core --lib --features voice-whisper-rs,voice-piper-rs voice::v2::pipeline::tests::barge_in_latency_under_budget`

### IN PROGRESS

- P2.3.4 live-device percentile capture (`KRIA_VOICE_LIVE=1`) for first-chunk/playback-start TTFA traces.

### TODO

- None.

### BLOCKED

- `native-audio` aggregate feature remains blocked by missing system package:
  - `webrtc-audio-processing.pc` not installed on host, so `voice-aec` cannot compile here.
  - This does **not** block Phase 2 P2.1/P2.2/P2.3 completion (AEC is deferred/non-goal for this phase).

### DEFERRED

- Full duplex/AEC, speculative synthesis, and autonomous orchestration (explicitly out of Phase 2).
- Optional post-Phase-2 refinements:
  - OverrunTracker hysteresis tuning (cooldown + deterministic restore)
  - benchmark expansion automation (TTFA/first-partial/first-chunk/interruption percentiles)

### latency observations

- Phase 1 baseline checks remain stable; no regressions before Phase 2 start.
- Main remaining TTFA penalties are still CLI STT/TTS process boundaries.
- Post-P2.1/P2.2 core pipeline test timings remain stable (no orchestration-latency regressions in current harness).
- P2.3 code-path integration keeps default (feature-off) latency stable; no regression observed in interruption/happy-path proxy timings.

### TTFA measurements

- Baseline reference remains Phase 1/validation telemetry and command-loop timings; live end-to-end TTFA re-measurement will be repeated after P2.1/P2.2 and again after P2.3.
- Current environment still lacks full live speech-session percentile capture in this pass; benchmark proxies updated under `benchmark results`.
- Feature-off proxy post-P2.3 benchmark update (3 runs):
  - `barge_in_cancels_tts_immediately`: avg **585.6ms**, p95 **588.6ms**, worst **603.7ms**
  - `barge_in_latency_under_budget`: avg **573.8ms**, p95 **579.9ms**, worst **580.4ms**
  - `happy_path_runs_through_states`: avg **606.9ms**, p95 **619.0ms**, worst **621.1ms**
- Feature-on proxy smoke (voice-whisper-rs + voice-piper-rs, targeted unit tests):
  - `happy_path_runs_through_states`: pass, warm elapsed **0.67s** (cold build run **17.56s**)
  - `barge_in_cancels_tts_immediately`: pass, elapsed **0.67s**
  - `barge_in_latency_under_budget`: pass, elapsed **0.62s**

### realtime UX findings

- Partial ordering stability is already improved; responsiveness ceiling is currently bounded by subprocess STT/TTS startup and full-sentence synthesis behavior.
- With whisper-rs enabled, partial loop behavior is now demand-driven and bounded inside STT backend rather than subprocess-bound.
- P2.3 implementation now adds in-process TTS primary-path selection logic and bounded chunk streaming orchestration hooks; assistant-feel uplift depends on clearing feature-on blockers and capturing live TTFA traces.

### playback observations

- `PiperRsTts` now has persistent runtime ownership (`Mutex<Option<Arc<PiperSpeechSynthesizer>>>`) for model warm reuse.
- `voice/v2/mod.rs` now retains deterministic fallback: piper-rs primary when available, CLI fallback when feature/model path unavailable.
- Cancellation path is deterministic: watch-based abort flag is bridged into a shared atomic checked between chunk emissions, preventing post-abort chunk enqueue.

### streaming observations

- P2.3 chunk streaming path implemented with bounded `chunk_size`/`chunk_padding` and bounded playback queueing (`PlaybackSink` channel remains size 4).
- Chunk emission is playback-aware via bounded channel backpressure and immediate stop on closed/aborted session.
- No speculative synthesis branches were introduced; one sentence stream at a time remains enforced.

### assistant UX findings

- Expected UX gain is earlier first-audio and reduced robotic sentence boundary waits once feature-on runtime path is active.
- Feature-on compile blocker is removed; direct assistant-grade live validation is now limited only by live-device test gating (`KRIA_VOICE_LIVE` and host audio setup), not dependency incompatibility.

### architectural decisions

- Keep orchestration serial-first and bounded; integrate in-process engines behind existing `Stt`/`Tts` traits rather than redesigning pipeline control flow.

### rejected ideas

- No speculative parallel synthesis queues.
- No hidden adaptive orchestration loops beyond bounded cadence/hysteresis controls.

### runtime risks

- Whisper CPU inference cadence can starve capture if decode windows are oversized or cadence is too aggressive.
- Feature-gated native path must degrade deterministically to CLI fallback with explicit telemetry/logging.
- Whisper confidence is currently not token-probability-derived in this pass (`confidence = 0.0` placeholder on final transcript), so confidence-sensitive post-processing should not rely on it yet.

### benchmark results

- Pre-change baseline checks pass (core/desktop/ui).
- Additional latency benchmarks will be appended after each P2 increment.
- Post-change focused checks pass:
  - `cargo check -p kria-core`
  - `cargo check -p kria-core --features voice-whisper-rs`
  - `cargo check -p kria-desktop`
  - `ui npm run -s check`
- Post-change focused runtime tests pass:
  - `streaming_partials_pumped_to_telemetry`
  - `concurrent_turn_rejected_with_busy_telemetry`
  - `builder_records_phase1_milestones`
- Post-change benchmark sample (3-run each):
  - `happy_path_runs_through_states`: avg **618.0ms**, worst **646.2ms**
  - `streaming_partials_pumped_to_telemetry`: avg **635.1ms**, worst **640.8ms**
  - `barge_in_latency_under_budget`: avg **578.1ms**, worst **591.7ms**
  - `concurrent_turn_rejected_with_busy_telemetry`: avg **835.9ms**, worst **861.5ms**
- P2.3 implementation checks:
  - default path: `cargo check -p kria-core`, `cargo check -p kria-desktop`, `ui npm run -s check` ✅
  - focused regressions: `barge_in_cancels_tts_immediately`, `streaming_partials_pumped_to_telemetry`, `concurrent_turn_rejected_with_busy_telemetry` ✅
  - feature-on build (`voice-piper-rs`) ✅ passing with local vendor patch

### Phase 2 readiness notes

- Phase 2 started.
- P2.1 and P2.2 are in place for STT.
- P2.3 implementation is complete and feature-on validated (compile + targeted runtime tests).

## Real-World Voice Runtime Validation (Post-Phase 1)

### VALIDATION RESULTS

- Scope executed: orchestration/runtime tests, interruption/cancel tests, playback/state tests, live-suite gating checks, toolchain responsiveness probes.
- Result summary:
  - `voice::v2::pipeline` critical paths: pass (happy path, barge-in, abort idempotency, partial telemetry, busy-reject invariant).
  - Live voice suite command execution: pass, with env-gated skips where `KRIA_VOICE_LIVE` is unset.
  - Desktop/UI validation commands: pass (`cargo check -p kria-desktop`, `ui npm run -s check`).
- Confidence level: high for deterministic orchestration correctness under test harness; medium for hardware/device behavior because this host lacks full real live-mode enablement and scripted device hotplug tests.

### RUNTIME OBSERVATIONS

- Serial-first orchestration remains stable under repeated invocation; no overlapping active-turn regressions observed.
- Centralized abort path is behaving deterministically (idempotent force-abort and barge-in cancellation tests remain green).
- Persistent playback runtime removes per-turn stream setup churn and keeps sink ownership singular.
- Busy-reject telemetry + UI state improved operator clarity; runtime no longer "silently ignores" overlapping turn attempts.

### UX ISSUES

- Assistant-feel is improved but not yet assistant-grade:
  - Turn rejection is clear but still abrupt in rapid command chaining (busy flashes can feel "hard stop" vs graceful conversational handoff).
  - TTFA visibility exists, but users still perceive think/speak transitions as mechanical when response starts near upper bound.
  - Partial transcript stability is better; remaining friction is cadence smoothness (updates are correct but still visibly stepped under bursty text).

### LATENCY FINDINGS

- Measured command-loop responsiveness (3-run sample):
  - `cargo test` quick pipeline path: avg **654.9ms**, worst **662.5ms**
  - `cargo check -p kria-desktop -q`: avg **781.9ms**, worst **824.3ms**
  - `ui npm run -s check`: avg **2259.6ms**, worst **2351.7ms**
- Repeated orchestration/interruption test invocations (5-run sample each) remained stable with no degradation trend.
- Runtime latency instrumentation is now present (mic/VAD/STT/LLM/TTS/playback milestones), but this environment did not produce full live conversational latency traces for true end-to-end percentile reporting.

### INTERRUPTION FINDINGS

- Barge-in/cancel behavior is stable under repeated regression tests.
- Forced abort remains bounded and idempotent; no stale state lock-in observed after consecutive abort paths.
- Repeated wake/turn-entry pressure is correctly rejected when busy; no evidence of concurrent turn overlap.
- Remaining UX gap: interruption feels functionally correct, but turn handoff messaging can feel binary (reject-or-run) under rapid user chaining.

### PLAYBACK FINDINGS

- Persistent playback runtime with lazy re-open materially improved playback continuity and reduced lifecycle churn risk.
- Failure/recovery telemetry surfaces are now sufficient to diagnose runtime sink issues from UI.
- No sink-duplication regressions observed in current test scope.
- Remaining risk: real hardware route churn (device unplug/replug during active turn) is only partially validated in this environment.

### DEVICE ISSUES

- Host capability probe:
  - `pw-cli` present
  - `arecord`/`aplay` present
  - PipeWire user service active
  - PulseAudio user service inactive
  - `pactl` missing
- Consequence: PipeWire-centric diagnostics are available, but PulseAudio control-path validation and `pactl`-driven scripted checks are not fully runnable here.

### STABILITY ISSUES

- No test-detected regressions in turn ownership, abort finalization, or partial-ordering invariants.
- No observed crash loops, deadlocks, or concurrent synthesis path activation in validation scope.
- Known non-critical stability caveat: UI `busy` timeout fallback can briefly diverge from backend state if state events are delayed.

### REAL-WORLD FAILURE CASES

- Failure case: rapid consecutive user commands during an active turn.
  - Current behavior: deterministic rejection with explicit busy signal.
  - Impact: correctness preserved, but can feel less conversational in high-interrupt sessions.
- Failure case: missing live-mode env (`KRIA_VOICE_LIVE` unset) in operator/test environment.
  - Current behavior: tests soft-skip live validations.
  - Impact: reduces confidence in true microphone/headphone hotplug behavior until live mode runs are executed.
- Failure case: missing `pactl` in environment.
  - Impact: limits scriptable device-route validation depth.

### PHASE 2 READINESS

- **Readiness verdict (updated):** Phase 2 implementation complete (P2.1 + P2.2 + P2.3), with live percentile validation follow-ups ongoing.
- **Closed blockers:**
  - Feature-on `voice-piper-rs` compile/runtime path now works via local vendor patch.
  - Bindgen header resolution for espeak path is solved via explicit clang include env.
- **Remaining follow-up (validation):**
  - Capture live end-to-end TTFA + interruption percentiles (`KRIA_VOICE_LIVE=1`).
  - Run explicit headphone unplug/replug + default-device-switch recovery traces.
- **Ideas still rejected for this phase:**
  - Full duplex/AEC rollout
  - speculative parallel synthesis
  - autonomous retry/queue orchestration
  - multi-agent runtime behavior

## Phase 2 Deep Realtime Validation (Post-P2.3)

### VALIDATION RESULTS

- Validation scope executed:
  - deep pipeline timing sweeps (warm baseline vs CPU-stressed)
  - interruption/cancel regression tests
  - live-suite status checks (`voice_live_tests`)
  - host audio runtime/device probes (PipeWire/Pulse path visibility)
- Result:
  - serial-first orchestration tests remain green in both normal and stressed conditions.
  - interruption paths remain deterministic in current harness (no overlap regressions observed).
  - feature-on piper-rs build path is now unblocked and validated through compile + targeted pipeline tests.

### STREAMING FINDINGS

- P2.3 streaming runtime is wired and now executable under feature-on builds (persistent piper synthesizer + chunk-stream emission path + deterministic fallback).
- Playback queue remains bounded (`mpsc` capacity 4) and no speculative chunk scheduling path was introduced.
- Chunk ownership model remains serial-first (single active sink/session path), with abort bridge propagating to synthesis loop.

### TTFA FINDINGS

- Proxy timing (warm baseline, 5-run) after P2.3 code integration:
  - `barge_in_cancels_tts_immediately`: avg **588.2ms**, p95 **593.8ms**, worst **617.0ms**
  - `barge_in_latency_under_budget`: avg **563.1ms**, p95 **585.4ms**, worst **596.8ms**
  - `happy_path_runs_through_states`: avg **604.1ms**, p95 **612.2ms**, worst **619.9ms**
- Under CPU stress (warm, 5-run):
  - `barge_in_cancels_tts_immediately`: avg **1403.6ms**, p95 **1421.3ms**, worst **1491.9ms**
  - `barge_in_latency_under_budget`: avg **1453.0ms**, p95 **1465.1ms**, worst **1686.3ms**
  - `happy_path_runs_through_states`: avg **1634.8ms**, p95 **1694.8ms**, worst **2209.4ms**
- Before/after TTFA (Phase 2 objective) is now unblocked for feature-on runtime collection; full live percentile capture is still pending `KRIA_VOICE_LIVE=1` sessions.

### INTERRUPTION FINDINGS

- `barge_in_cancels_tts_immediately` and `barge_in_latency_under_budget` remain consistently passing.
- No stale state-lock or overlapping turn evidence observed in repeated cancellation pressure tests.
- Interruption quality is functionally correct in harness, but UX still risks feeling binary under rapid chaining (busy-reject conversational harshness remains).

### PLAYBACK FINDINGS

- Playback ownership invariant holds in current validated path (single sink/session model preserved).
- No new sink-duplication regressions detected in regression suite.
- Full piper chunk-playback smoothness and first-chunk/start-latency validation is blocked until feature-on runtime path is executable.

### UX OBSERVATIONS

- Runtime determinism and interruption correctness are improved versus pre-Phase-2 baseline.
- KRIA still does not consistently feel “assistant-grade” in this environment because direct piper streaming latency/smoothness cannot yet be observed end-to-end.
- Busy-state rejection behavior remains one of the highest-friction conversational feel issues.

### LATENCY REGRESSIONS

- No default-path regression was observed in focused pipeline timing compared to recent Phase-2 baseline windows.
- Major degradation under synthetic CPU saturation is expected and measurable; p95/worst tails widen significantly.
- One-time outlier spikes can occur in first cold run of a benchmark case; warm-run statistics should be treated as the stable indicator.

### STABILITY ISSUES

- **Feature-on TTS blocker status (UPDATED 2026-05-13):**
  - ✅ `espeak-rs-sys` bindgen header issue resolved via explicit clang include env:
    - `LIBCLANG_PATH=/lib/x86_64-linux-gnu`
    - `BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include"`
  - ✅ `piper-rs v0.1.9` ORT/ndarray mismatch resolved by vendoring and patching:
    - local crate override: `vendor/piper-rs` + workspace `[patch.crates-io]`
    - updated crate deps to ORT rc12 + ndarray 0.17
    - adapted tensor extraction/construction for ORT 2.0 (`try_extract_tensor` tuple API, owned tensor inputs, mutable session run path)
  - ✅ feature-on compile/runtime path now healthy for Phase 2 scope
- Live hardware tests are still mostly gated (`KRIA_VOICE_LIVE` not set; 7 ignored, 2 pass).

### PHASE 2 READINESS

- **Phase 2 status:** achieved for implementation scope (P2.1 + P2.2 + P2.3 complete).
- Achieved:
  - whisper-rs primary STT path
  - bounded realtime partial loop
  - code-level piper-rs streaming integration and deterministic fallback wiring
- Remaining validation follow-up (non-blocking for implementation completion):
  - capture live first-chunk/playback-start percentile metrics with `KRIA_VOICE_LIVE=1`
  - run explicit headphone unplug/replug + default-device-switch stress captures on live path
- **Readiness verdict:** Phase 2 implementation is complete; keep Phase 3 deferred until live percentile validation report is captured.

### REAL-WORLD FAILURE CASES

- Failure case (historical, now resolved): enabling `voice-piper-rs` on host build chain failed.
  - Resolution: local `vendor/piper-rs` patch + ORT 2.0 compatibility update removed compile blocker.
- Failure case: high CPU contention with multitasking/background load.
  - Impact: interruption/happy-path latencies degrade materially (p95/worst tails widen).
  - User effect: conversational snappiness degrades; responsiveness feels less assistant-like.
- Failure case: live device-path validation remains environment-gated.
  - Impact: cannot claim robust headphone/device-switch streaming recovery quality yet.

### REJECTED IDEAS

- Do not add speculative pre-synthesis or parallel future-branch playback to mask latency.
- Do not redesign orchestration into multi-owner playback graphs.
- Do not add duplex/AEC work in this phase as a workaround for unresolved piper feature-on validation.
- Do not add autonomous retries/hidden background synthesis loops; keep bounded deterministic cancellation ownership.

---

## 2. Current Runtime Audit

### 2.1 v1 Pipeline (`VoicePipeline`)

| Aspect | Status | Issue |
|--------|--------|-------|
| Capture → VAD → STT → Agent → TTS → Play | Working | Sequential, blocking |
| STT | `whisper-cpp` CLI subprocess | Cold-loads 1.6GB model per call |
| TTS | `piper` CLI subprocess | Synthesizes entire response then plays |
| VAD | Silero ONNX | Properly integrated |
| Partials | Disabled by default | Each partial spawns fresh subprocess |
| Barge-in | None | Mic muted during TTS via atomic bool |
| Echo prevention | `mic_muted` atomic + 300ms delay | No real AEC |

### 2.2 v2 Pipeline (`VoicePipelineV2`)

| Aspect | Status | Issue |
|--------|--------|-------|
| Concurrency skeleton | Complete | CancellationToken chain verified |
| FSM | 6 states (Sleeping/Listening/Transcribing/Thinking/Speaking/BargeIn) | Correct |
| Barge-in | Architecturally complete | ≤50ms target, tested with stubs |
| Sentence splitter | Working | Abbreviation-aware, Hinglish support |
| STT engine | `CliWhisperStt` | Buffers entire utterance, shells out |
| TTS engine | `CliPiperTts` | Synthesizes whole sentence, one chunk |
| `WhisperRsStt` | Compiles, bails at runtime | Stub |
| `PiperRsTts` | Compiles, bails at runtime | Stub |
| AEC | Passthrough stub | WebRTC APM not wired |
| Wake word | Feature-gated, models not shipped | Compiles |
| Post-edit | Decision logic works | Not wired in pipeline |
| Playback sink | Works with stubs | New OutputStream per session |

### 2.3 Tauri Integration

| Aspect | Status |
|--------|--------|
| `start_voice` | Builds v1, optionally hot-swaps v2 |
| `start_voice_v2_loop` | Continuous capture → serial `run_turn` loop |
| `voice_v2_speak` | Speak-only turn from text prompt |
| Telemetry pump | Maps v2 events → Tauri events |
| `stop_voice` | Sets `voice_active=false` + `force_abort` |

### 2.4 Source File Map

| File | Lines | Purpose | Status |
|------|-------|---------|--------|
| `voice/v2/pipeline.rs` | 1266 | FSM orchestrator, `run_turn`, barge-in | Scaffold |
| `voice/v2/stt.rs` | 285 | `Stt` trait + CliWhisper/Sidecar/WhisperRs | CLI works |
| `voice/v2/tts.rs` | 157 | `Tts` trait + CliPiper/PiperRs | CLI works |
| `voice/v2/playback.rs` | 185 | `PlaybackSink` + abort | Works |
| `voice/v2/sentence.rs` | 204 | `SentenceSplitter` | Working |
| `voice/v2/aec.rs` | 144 | `AecProcessor` | Passthrough |
| `voice/v2/post_edit.rs` | 253 | `HinglishPostEditor` | Logic works |
| `voice/v2/wake.rs` | 551 | `WakeWordDetector` (openWakeWord) | Feature-gated |
| `voice/v2/mod.rs` | 118 | Module root, `build_v2_with_cli_engines` | Working |
| `voice/stt.rs` | 307 | v1 whisper-cpp CLI wrapper | Working |
| `voice/tts.rs` | 324 | v1 piper CLI wrapper | Working |
| `voice/capture.rs` | 367 | CPAL mic capture | Working |
| `voice/playback.rs` | 134 | rodio `AudioPlayer` | Working |
| `voice/vad.rs` | 333 | Silero VAD ONNX | Working |
| `voice/tier.rs` | 248 | VoiceTier S/A/C profiles | Working |
| `voice/metrics.rs` | 207 | TTFA telemetry + OverrunTracker | Working |
| `commands/voice.rs` | 750 | Tauri commands | Working |
| `commands/voice_runtime_helpers.rs` | 547 | Pipeline builders + v2 loop | Working |

---

## 3. Assistant UX Gap Analysis

| Gap | Current | Target | Priority |
|-----|---------|--------|----------|
| Response start delay | 2-5s | <500ms (S), <800ms (A), <1200ms (C) | **Critical** |
| No streaming STT | Buffers entire utterance | Rolling-window partials every 500ms | **Critical** |
| No streaming TTS | Entire sentence → one chunk | Incremental PCM chunks ~120ms | **High** |
| Sentence-serial synthesis | Synth N, play, synth N+1 | Overlap: synth N+1 while playing N | **High** |
| Push-to-talk only | Button/hotkey required | Wake word + continuous mode | **High** |
| No barge-in (v1) | Mic muted during playback | VAD-triggered mid-sentence interrupt | **High** |
| Echo cancellation | Mic-mute flag | AEC or headphone-mode bypass | **Medium** |
| No thinking feedback | Silence during LLM gen | Visual indicator (not audio filler) | **Medium** |
| Static turn-taking | Fixed 500ms VAD silence | Adaptive (future) | **Low** |

---

## 4. Runtime Bottleneck Analysis

### Current (v1, estimated)

| Stage | Latency | Bottleneck |
|-------|---------|------------|
| VAD speech-end detection | 500-1000ms | `vad_silence_ms` = 500 |
| Audio → temp WAV write | 5-20ms | Disk I/O |
| whisper-cpp cold-load | 800-2000ms | Model load per subprocess |
| whisper-cpp inference | 500-3000ms | Utterance length dependent |
| LLM TTFT | 500-2000ms | Model inference |
| piper CLI cold-load | 200-500ms | Model load per subprocess |
| piper synthesis | 200-1000ms | Entire response |
| Playback stream open | 50-200ms | New `OutputStream` per call |
| **Total TTFA** | **~2500-8000ms** | **Unacceptable** |

### Target (v2, in-process)

| Stage | Target | Method |
|-------|--------|--------|
| VAD speech-end | 300-500ms | Tuned silence timeout |
| STT final | 200-400ms | In-process whisper-rs, pre-loaded |
| Post-edit (if triggered) | 0-250ms | Concurrent with first LLM tokens |
| LLM first token | 100-300ms | Streaming, hot on GPU |
| Sentence split + TTS first chunk | 100-200ms | In-process piper-rs, pre-loaded |
| Playback start | 10-30ms | Persistent rodio sink |
| **Total TTFA** | **~400-900ms** | **Assistant-grade** |

---

## 5. Realtime Streaming Architecture

### Data Flow (Target)

```text
Mic (CPAL 16kHz) ──→ broadcast(128)
                        │
                        ├──→ [VAD task] → SpeechStart/SpeechEnd events
                        ├──→ [Wake task] → WakeWordEvent (Sleeping only)
                        └──→ [STT task] → PartialTranscript / FinalTranscript
                                              │
                                              ▼
                                    [LLM token stream] (mpsc(64))
                                              │
                                              ▼
                                    [SentenceSplitter] → sentences
                                              │
                                              ▼
                                    [TTS synth] → PCM chunks (mpsc(4))
                                              │
                                              ▼
                                    [PlaybackSink drain] → rodio speakers
                                              │
                                              ▼
                                    [AEC ref tap] → echo cancellation
```

### Channel Budget

| Channel | Type | Capacity | Rationale |
|---------|------|----------|-----------|
| Capture fan-out | `broadcast` | 128 | ~12.8s at 10 chunks/s; prevents lag |
| STT PCM feed | `mpsc` | 64 | ~6.4s buffered; STT drains fast |
| LLM tokens | `mpsc` | 64 | Token backpressure unlikely |
| TTS PCM → playback | `mpsc` | 4 | ~480ms buffered; tight = low latency |
| Partials/telemetry | `mpsc::unbounded` | ∞ | Low volume, never blocks producers |
| Abort signal | `watch` | 1 | Single-value, latest-wins |

### Threading Model

| Thread/Task | Kind | Priority | Purpose |
|-------------|------|----------|---------|
| CPAL callback | OS audio thread | RT (managed by CPAL/PipeWire) | Capture samples |
| Capture pump | `std::thread` | Normal | CPAL → broadcast channel |
| STT inference | `spawn_blocking` | Normal | CPU-bound whisper-rs |
| LLM inference | tokio task | Normal | GPU-bound, async streaming |
| Sentence split | inline in TTS task | — | Zero-alloc hot path |
| TTS synth | `spawn_blocking` | Normal | CPU-bound piper-rs |
| Playback drain | tokio task | Normal | Feeds rodio synchronously |
| VAD | inline per-chunk | — | <1ms per 100ms chunk |
| Wake word | inline per-chunk | — | <2ms per 80ms frame |

**Rule:** RT priority is ONLY for the CPAL audio thread (managed by PipeWire/ALSA, not by us). All other threads are normal priority. No manual `SCHED_FIFO`.

---

## 6. Recommended Runtime Architecture

### Core Struct

```rust
pub struct VoicePipelineV2 {
    profile: VoiceTierProfile,
    stt: Arc<dyn Stt>,
    tts: Arc<dyn Tts>,
    playback: Arc<Mutex<PlaybackSink>>,
    wake: Option<Arc<WakeWordDetector>>,
    aec: Arc<Mutex<AecProcessor>>,
    post_editor: Arc<HinglishPostEditor>,
    audio_player: Arc<Mutex<Option<Arc<AudioPlayer>>>>,
    state_tx: watch::Sender<VoiceSessionState>,
    telemetry_tx: mpsc::UnboundedSender<VoiceTelemetry>,
    current_turn: Arc<Mutex<Option<MetricsBuilder>>>,
    turn_cancel: Arc<Mutex<CancellationToken>>,
    turn_guard: Arc<tokio::sync::Mutex<()>>,  // NEW: mutual exclusion
}
```

### Echo Mode Configuration

```rust
pub enum EchoMode {
    HalfDuplex,   // mic muted during playback (default, always works)
    Headphone,    // no echo path, VAD barge-in enabled (zero-cost)
    AecDuplex,    // WebRTC APM, full barge-in (Phase 3)
}
```

---

## 7. FSM + Turn Ownership Model

### States

```text
┌──────────┐  wake/PTT   ┌───────────┐  VAD end   ┌──────────────┐
│ Sleeping │ ──────────→ │ Listening │ ─────────→ │ Transcribing │
└──────────┘             └───────────┘            └──────┬───────┘
      ▲                                                   │ final
      │                                                   ▼
      │  done           ┌──────────┐  1st token  ┌──────────┐
      ├─────────────── │ Speaking │ ←─────────── │ Thinking │
      │                 └────┬─────┘             └──────────┘
      │                      │ VAD SpeechStart
      │                      ▼
      │                 ┌──────────┐
      └──────────────── │ BargeIn │
                        └──────────┘
```

### Turn Ownership Invariants

1. **Only one active turn exists** — enforced by `turn_guard: Mutex<()>`. Both `run_turn` and `run_speak_turn` acquire this at entry via `try_lock`. Second caller gets `Err("turn already active")`.
2. **Per-turn cancellation token** — fresh `CancellationToken` per turn, stored in `turn_cancel`. Stale tokens from previous turns are inert.
3. **`force_abort` is idempotent** — cancels token + clears playback + finalizes metrics. Safe to call at any time from any state.
4. **State transitions are strictly ordered** — only the turn owner calls `set_state`. External callers go through `force_abort` (→ Sleeping) or `force_wake` (Sleeping → Listening).

---

## 8. STT Runtime Design

### Architecture

```rust
#[async_trait]
pub trait Stt: Send + Sync {
    fn engine_id(&self) -> &'static str;
    async fn start_stream(
        self: Arc<Self>,
        pcm_rx: mpsc::Receiver<AudioChunk>,
        partial_tx: mpsc::UnboundedSender<PartialTranscript>,
    ) -> anyhow::Result<StreamHandle>;
}
```

### Backend Priority

| Backend | When | Latency | Status |
|---------|------|---------|--------|
| `WhisperRsStt` (CUDA) | S/A tier + feature compiled | ~200ms/2s utterance | Phase 2 |
| `WhisperRsStt` (CPU) | C tier or no GPU | ~800ms/2s utterance | Phase 2 |
| `SidecarStt` | Python sidecar available | ~500ms | Stub |
| `CliWhisperStt` | Always available fallback | ~2000ms (cold-load) | Working |

### Demand-Driven Partial Strategy (Phase 2)

```
VAD SpeechStart → accumulate audio in ring buffer
                → set inference_ready = true

Inference loop (spawn_blocking):
  if inference_ready AND !inference_running AND new_audio >= 500ms:
    inference_running.store(true)
    result = whisper_rs.decode(ring_buffer_last_2500ms)
    emit PartialTranscript { text, seq, confidence }
    inference_running.store(false)

VAD SpeechEnd → trigger final pass on complete buffer
             → emit FinalTranscript
```

**Key constraint:** `inference_running: AtomicBool` ensures at most one whisper inference runs at a time. If the previous partial is still computing, the next trigger is skipped — no queue, no pile-up.

**Minimum finalization cadence:** If `inference_running` has been true for >3s (stuck inference), force-timeout and emit a partial from the last known good result. This prevents silent hangs.

### CliWhisperStt Hardening (Phase 1)

Current issues:
1. Buffer grows unbounded if VAD never fires SpeechEnd
2. Child subprocess not killed on abort

Fixes:
```rust
// Hard cap: 60 seconds of audio (960,000 samples at 16kHz)
const MAX_BUFFER_SAMPLES: usize = 16_000 * 60;

// In the drain loop:
if buffer.len() >= MAX_BUFFER_SAMPLES {
    break; // Process what we have
}

// On abort: kill the child process
_ = &mut abort_rx => {
    if let Some(ref mut child) = child_handle {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    break;
}
```

---

## 9. TTS Runtime Design

### Architecture

```rust
#[async_trait]
pub trait Tts: Send + Sync {
    fn engine_id(&self) -> &'static str;
    fn sample_rate(&self) -> TtsSampleRate;
    async fn synthesize_sentence(
        self: Arc<Self>,
        sentence: String,
        pcm_tx: mpsc::Sender<Vec<f32>>,
        abort_rx: watch::Receiver<bool>,
    ) -> anyhow::Result<()>;
}
```

### Backend Priority

| Backend | Chunk Size | Latency per sentence | Status |
|---------|-----------|---------------------|--------|
| `PiperRsTts` | ~120ms PCM chunks | 100-200ms to first chunk | Phase 2 |
| `CliPiperTts` | 1 giant chunk (whole sentence) | 200-1000ms | Working |

### Streaming Strategy (Phase 2)

- Piper-rs decodes phonemes → mel spectrogram → vocoder in ~120ms chunks
- Each chunk (~2600 samples at 22050Hz) pushed to `pcm_tx` immediately
- Playback starts on FIRST chunk — doesn't wait for entire sentence
- `abort_rx` checked between chunk iterations for cancellation

### Sentence-Serial vs Prefetch

**Phase 1-2:** Serial synthesis — one sentence at a time in the `'outer` loop. Cooperative cancel checks between sentences.

**Phase 3 (optional):** Prefetch — begin synth of sentence N+1 while N plays. Requires generation IDs on PCM chunks to prevent stale data reaching playback after barge-in. Deferred until profiling proves it's necessary.

---

## 10. Playback Runtime Design

### Current Issue

`begin_session` creates a new rodio `OutputStream` per turn. This adds 50-200ms latency on Linux (PipeWire/ALSA stream negotiation).

### Target Architecture

```rust
pub struct PlaybackSink {
    sample_rate: u32,
    // Persistent rodio resources (created once at pipeline start)
    output_stream: Option<(OutputStream, OutputStreamHandle)>,
    sink: Option<rodio::Sink>,
    // Per-session state
    pcm_tx: Option<mpsc::Sender<Vec<f32>>>,
    abort_tx: watch::Sender<bool>,
    drain: Option<JoinHandle<()>>,
    // AEC reference tap
    aec_ref_tx: Option<mpsc::UnboundedSender<Vec<f32>>>,
    // Telemetry
    first_audio_emitted: Arc<AtomicBool>,
    first_audio_callback: Option<Arc<dyn Fn(Instant) + Send + Sync>>,
    // Health
    healthy: Arc<AtomicBool>,
}
```

### Lifecycle

1. **Pipeline start:** `PlaybackSink::new()` opens `OutputStream` + `Sink` once
2. **Turn start:** `begin_session()` resets abort flag, creates `pcm_tx`, spawns drain task
3. **During turn:** drain task feeds PCM chunks to the persistent `Sink`
4. **Turn end / barge-in:** `abort()` calls `sink.clear()`, signals abort, drops pcm_tx
5. **Device failure:** drain task detects rodio error → sets `healthy = false`. Next `begin_session()` attempts to reopen the stream (lazy recovery).

### Health Verification

```rust
impl PlaybackSink {
    fn ensure_healthy(&mut self) -> bool {
        if !self.healthy.load(Ordering::Relaxed) {
            // Try to reopen
            match self.try_reopen() {
                Ok(()) => { self.healthy.store(true, Ordering::Relaxed); true }
                Err(e) => { tracing::warn!("playback reopen failed: {e}"); false }
            }
        } else {
            true
        }
    }
}
```

---

## 11. Cancellation Architecture

### Single-Root Propagation

```text
force_abort() or VAD barge-in
         │
         ▼
  turn_cancel.lock().cancel()  ← SINGLE cancellation root
         │
         ├──→ capture_task (stops forwarding audio)
         ├──→ partial_pump (stops forwarding partials)
         ├──→ tts_task inner loop (cooperative check between sentences)
         │       └──→ bridge → watch<bool> → Tts::synthesize_sentence abort_rx
         └──→ playback.abort() (clears rodio sink immediately)
```

### Centralized Abort (Improvement)

Currently `force_abort` performs 3 separate lock acquisitions (turn_cancel, playback, current_turn). Consolidate into a single atomic operation:

```rust
pub async fn force_abort(&self) {
    // 1. Acquire turn_cancel and playback under one critical section
    let cancel = self.turn_cancel.lock().await;
    cancel.cancel();
    
    let mut pb = self.playback.lock().await;
    pb.abort();
    drop(pb);
    drop(cancel);
    
    // 2. Finalize metrics
    let mut cur = self.current_turn.lock().await;
    if let Some(builder) = cur.take() {
        let metrics = builder.finalise();
        let _ = self.telemetry_tx.send(VoiceTelemetry::Metrics(metrics));
    }
    self.set_state(VoiceSessionState::Sleeping);
}
```

### Barge-In Timing Budget

| Step | Budget | Mechanism |
|------|--------|-----------|
| VAD fires SpeechStart | 0ms | Event received by watcher task |
| Watcher acquires turn_cancel | <5ms | tokio Mutex, uncontended hot path |
| token.cancel() propagates | <1ms | All clones observe in same tick |
| bridge flips abort_rx | <1ms | watch::send is O(1) |
| Synth backend observes abort | <10ms | Next poll point in synthesis |
| playback.abort() clears sink | <5ms | rodio sink.clear() is immediate |
| **Total barge-in latency** | **<25ms** | Well under 50ms budget |

---

## 12. Audio Orchestration Model

### Device Management

```rust
pub struct AudioDeviceManager {
    capture_device: Option<String>,  // from config
    output_device: Option<String>,   // from config
    // Runtime state
    active_capture: Option<AudioCaptureHandle>,
    active_output: Option<(OutputStream, OutputStreamHandle)>,
}
```

### Device Selection

- **Capture:** User-configured device name, or CPAL default. Enumerated via `AudioCapture::list_devices()`.
- **Output:** User-configured device name, or CPAL default. Enumerated via `AudioPlayer::list_output_devices()`.
- **Hot-swap:** Device change requires `force_abort` + rebuild capture/playback. Not hot-swappable mid-turn.

### PipeWire/PulseAudio Considerations

- CPAL on Linux uses ALSA by default. PipeWire provides ALSA compatibility layer.
- PipeWire manages RT priority for audio threads automatically. Do NOT manually set `SCHED_FIFO`.
- Log the detected audio subsystem at startup via `pactl info` or `/proc/asound/version` for diagnostics.
- Buffer sizes: CPAL requests ~100ms chunks. PipeWire may adjust quantum. Accept whatever the server provides.

### Resource Constraints (RTX 4050 6GB)

| Resource | Budget | Allocation |
|----------|--------|------------|
| GPU VRAM | 6GB total | LLM: ~4-5GB, Whisper (if GPU): ~1GB |
| CPU threads | 8 logical cores | Whisper-rs: 4 threads, Piper-rs: 2 threads, system: 2 |
| RAM | 16GB | Models: ~6GB, app: ~2GB, system: ~8GB |

**Rule:** Whisper and LLM CANNOT share GPU simultaneously on 6GB VRAM. Whisper runs on CPU (4 threads). LLM owns the GPU exclusively. This is not a limitation — CPU whisper-rs on a modern 8-core is ~200ms for 2s utterance with ggml-large-v3-turbo-q5.

---

## 13. Device Management Design

### Configuration

```toml
[voice]
capture_device = ""       # empty = system default
output_device = ""        # empty = system default
aec_mode = "half_duplex"  # "half_duplex" | "headphone" | "aec_duplex"
```

### Runtime Behavior

| Event | Action |
|-------|--------|
| App start | Open capture + output devices from config |
| Device disconnected | Mark unhealthy, emit `voice:device_error`, retry on next turn |
| User changes device in UI | `force_abort` → reconfigure → resume |
| Output device fails mid-playback | Drain task exits with error → `healthy = false` |
| Capture device fails | Capture thread exits → broadcast closes → turn fails → retry |

### Headphone Mode Detection (Future)

Auto-detection via PulseAudio/PipeWire port metadata (`headphone` vs `speaker` profile). Manual toggle in Phase 1; optional auto-detect in Phase 4.

---

## 14. VAD / Wake Word Runtime

### VAD (Silero ONNX)

- **Input:** 16kHz mono f32 chunks (~100ms = 1600 samples)
- **Output:** `VadResult::SpeechStart` | `SpeechEnd` | `Silence`
- **Latency:** <1ms per chunk (ONNX inference on CPU)
- **State:** Internal LSTM states maintained across chunks
- **Silence timeout:** 500ms of silence after speech → SpeechEnd
- **Fallback:** Energy threshold when Silero model unavailable

### Wake Word (openWakeWord, Phase 4)

- **Stack:** melspectrogram.onnx → embedding_model.onnx → hey_ria.onnx
- **Input:** 80ms frames (1280 samples at 16kHz)
- **Latency:** <2ms per frame for 3-model inference
- **Sensitivity:** configurable threshold (default 0.5)
- **Cooldown:** 500ms after detection (prevents double-trigger)
- **Active only during `Sleeping` state** — disabled during active turns

### Interaction

```text
Sleeping + wake_word_enabled:
  Audio chunks → Wake task
  WakeWordEvent → force_wake("oww") → Listening

Sleeping + push_to_talk:
  Button/hotkey → force_wake("ptt") → Listening

Listening/Transcribing/Thinking/Speaking:
  Wake word task SUSPENDED (no inference, saves CPU)
```

---

## 15. Barge-In Architecture

### Modes by Echo Configuration

| Mode | Barge-In Mechanism | Echo Handling | When Available |
|------|-------------------|---------------|----------------|
| `half_duplex` | Push-to-talk only (PTT) | Mic muted during playback | Default, always |
| `headphone` | VAD-triggered (SpeechStart while Speaking) | None needed (no echo path) | Phase 1 |
| `aec_duplex` | VAD-triggered | WebRTC APM cancels echo | Phase 3 |

### VAD Barge-In Flow

```text
1. Pipeline in Speaking state
2. Audio chunks still flowing to VAD (headphone/AEC mode)
3. VAD detects SpeechStart
4. spawn_barge_in_watcher sends to turn_cancel
5. CancellationToken.cancel() propagates:
   - TTS task: breaks 'outer loop
   - Bridge: flips abort_rx watch
   - Synth backend: observes abort, stops mid-decode
   - PlaybackSink: abort() clears rodio queue
6. FSM → BargeIn state
7. Pipeline emits VoiceTelemetry::BargeIn
8. 250ms grace period for TTS task wind-down
9. If stuck: hard abort (task.abort())
10. FSM → Sleeping
11. Next turn begins immediately
```

### Headphone Mode (Phase 1 — Zero-Cost Barge-In)

Users with headphones have NO echo path (speakers don't feed back into mic). This means:
- Disable echo gate (don't mute mic during playback)
- Enable VAD on audio stream continuously
- Barge-in fires immediately on SpeechStart while Speaking
- No AEC dependency, no signal processing, no latency
- **Implementation:** Single config flag → skip `mic_muted` toggle in the v2 loop

---

## 16. TTFA Optimization Strategy

### Measurement

TTFA = `t_first_audio_out - t_speech_end`

Already implemented in `MetricsBuilder` (`voice/metrics.rs`). Emitted as `VoiceTelemetry::Metrics` per turn.

### Budgets (Phase-Dependent)

| Tier | Phase 1 (CLI) | Phase 2 (In-Process) | Phase 3 (Optimized) |
|------|---------------|---------------------|---------------------|
| S | 2000ms | 700ms | 500ms |
| A | 3000ms | 1000ms | 800ms |
| C | 5000ms | 1500ms | 1200ms |

### Degradation Strategy

Uses existing `OverrunTracker` (3 consecutive overruns → trigger):

```
OverrunTracker fires →
  1. Log degradation event
  2. Emit voice:degraded telemetry to UI
  3. Downgrade tier one level (S→A, A→C)
  4. Cooldown: hold degraded tier for 10 turns minimum
  5. Recovery: after 5 consecutive good turns at degraded tier, restore original
```

**Hysteresis:** `cooldown_turns: u8 = 10` before attempting restoration. Prevents oscillation between tiers on a thermal boundary.

### Optimization Levers (by impact)

| Lever | Savings | Phase |
|-------|---------|-------|
| In-process STT (whisper-rs, pre-loaded) | 800-2000ms | Phase 2 |
| Persistent playback stream | 50-200ms | Phase 1 |
| In-process TTS (piper-rs, pre-loaded) | 200-500ms | Phase 2 |
| Streaming TTS (first chunk, not full sentence) | 100-400ms | Phase 2 |
| Post-edit skip on high confidence | 0-250ms | Phase 1 |
| Reduced VAD silence timeout (300ms vs 500ms) | 200ms | Phase 2 |

---

## 17. Resource Management Strategy

### Model Loading

| Model | Size (disk) | Load Time | Lifecycle |
|-------|-------------|-----------|-----------|
| Silero VAD | ~2MB | <100ms | Once at app start |
| openWakeWord (3 models) | ~10MB | <200ms | Once if wake enabled |
| whisper-rs (ggml-large-v3-turbo-q5) | ~1.6GB | ~2s | Once at pipeline start |
| piper-rs (voice model) | ~65MB | ~500ms | Once at pipeline start |
| LLM (quantized) | ~4GB | ~5s | Managed by kria-core LLM router |

**Rule:** All models loaded at pipeline start. No cold-loads during turns. Model load failures are fatal at startup — not silently degraded mid-conversation.

### VRAM Exclusion Protocol

- `GpuLease` system already exists in `kria-core`
- Whisper and LLM must NOT hold VRAM simultaneously
- Solution: whisper runs on CPU. LLM owns GPU exclusively.
- If future hardware has >8GB VRAM: allow concurrent GPU via lease system

---

## 18. Concurrency / Threading Rules

### Hard Rules

1. **Serial turns** — only one `run_turn` or `run_speak_turn` active at any time
2. **No nested spawns** — each stage spawns at most one task; no recursive spawning
3. **Bounded channels** — all inter-task channels have explicit capacity (except telemetry)
4. **No busy-polling** — all waits use `tokio::select!` or channel recv, never `try_recv` + sleep
5. **Single cancel root** — one `CancellationToken` per turn; all tasks clone it
6. **spawn_blocking for CPU work** — whisper inference, piper synthesis, rodio playback
7. **No manual thread priorities** — CPAL/PipeWire manage audio thread RT priority
8. **Mutex discipline** — `tokio::Mutex` for async-held guards; `std::Mutex` for sync-only (VAD state)
9. **No std::thread::spawn in hot path** — only CPAL capture uses std::thread

### Task Budget per Turn

| Task | Spawned by | Lifetime | Cancel via |
|------|-----------|----------|------------|
| `capture_task` | run_turn | Listening → turn end | CancellationToken |
| `partial_pump` | run_turn | Listening → Transcribing | abort() then abort() |
| `tts_task` | run_turn | Speaking → turn end | CancellationToken + abort_rx |
| `bridge` | tts_task | Speaking → turn end | tts_task exits → abort() |
| `drain` | begin_session | Speaking → turn end | abort_rx watch |
| `barge_in_watcher` | caller | Entire session | Drop handle |

**Maximum concurrent tasks per turn: 5** (capture, partial_pump, tts, bridge, drain). This is bounded and constant.

---

## 19. GUI / UX Update Plan

### Voice Panel Components

#### 1. Assistant State Indicator
- **Sleeping:** Subtle pulse dot (grey)
- **Listening:** Active mic icon, pulsing blue ring
- **Transcribing:** Text appears character-by-character, amber ring
- **Thinking:** Animated dots or spinner, purple ring
- **Speaking:** Waveform animation, green ring
- **BargeIn:** Flash red briefly, transition back to Listening

#### 2. Live Partial Transcript Display
- Shows streaming partial text as it arrives
- Replaces incrementally (uses `seq` field for ordering)
- Final transcript highlighted/locked when complete
- Reset display on new turn

#### 3. Realtime Conversation Panel
- Scrollable conversation history
- User messages (from STT) in chat bubble style
- Assistant responses (from LLM) with streaming token visualization
- Interruption markers shown inline ("interrupted" badge)
- Turn boundaries visible

#### 4. TTFA Metrics Display (Debug Overlay)
- Toggle-able overlay showing per-turn:
  - `t_first_partial` (ms)
  - `t_final` (ms)
  - `t_first_audio_out` (ms)
  - TTFA budget vs actual (bar chart)
  - Overrun warning if budget exceeded
  - Current tier (S/A/C) and degradation status

#### 5. Waveform / VAD Visualization
- Real-time audio waveform from capture stream
- VAD state overlay (speech region highlighted)
- Useful for debugging mic issues and VAD sensitivity

#### 6. Voice Controls
- **Mic selector:** Dropdown of available capture devices
- **Speaker selector:** Dropdown of available output devices
- **Echo mode:** Half-duplex / Headphone / AEC (when available)
- **Wake word toggle:** Enable/disable "Hey Ria"
- **Voice mode switch:** v1 (legacy) / v2 (streaming)
- **Push-to-talk button:** Hold or toggle

#### 7. Status Indicators
- **Degraded mode:** Yellow warning banner when tier downgraded
- **Device error:** Red banner with retry button
- **Feature availability:** Icons showing which native backends are compiled
- **Connection status:** Pipeline active/inactive

#### 8. Interruption UX
- When user interrupts (barge-in): assistant text fades with "interrupted" marker
- No jarring audio cut — playback sink clears queue smoothly
- Quick transition: visual state changes within 1 frame (~16ms)
- No audio filler or "hmm" sounds (rejected as anti-UX)

#### 9. Recovery/Failure UX
- STT timeout: "I didn't catch that" message + auto-retry
- TTS failure: Text response shown without audio
- Playback device lost: "Audio output unavailable" + retry
- Pipeline crash: "Voice restarting..." + auto-restart

### Tauri Event Bus (Existing)

```
voice:state        → { state: "listening" | "thinking" | ... }
voice:partial      → { text, engine, seq }
voice:final        → { text, engine, confidence }
voice:metrics      → { tier, ttfa_budget_ms, t_first_audio_out_ms, ... }
voice:barge_in     → {}
voice:wake         → { phrase, score, source }
voice:error        → { message }
voice:degraded     → { from_tier, to_tier }    // NEW
voice:device_error → { device, error }         // NEW
voice:first_audio  → {}
```

---

## 20. Runtime Invariants

These invariants MUST hold at all times. Violations are bugs.

1. **INV-TURN:** At most one active turn exists. Enforced by `turn_guard: Mutex<()>`.

2. **INV-CANCEL:** All per-turn tasks observe the same `CancellationToken`. A single `cancel()` call stops all of them.

3. **INV-STT:** STT inference never overlaps with itself. `AtomicBool` guard in whisper-rs; serial subprocess in CLI fallback.

4. **INV-WAKE:** Wake word inference is disabled during active turns (Listening through Speaking). Only runs during Sleeping.

5. **INV-PLAYBACK:** At most one playback drain task exists. `begin_session` creates one; `abort` stops it.

6. **INV-STATE:** FSM state transitions are strictly ordered: Sleeping→Listening→Transcribing→Thinking→Speaking→Sleeping (or →BargeIn→Sleeping). No skipping.

7. **INV-ABORT:** `force_abort()` always terminates at Sleeping state. Idempotent — safe to call multiple times.

8. **INV-SERIAL:** The v2 loop is serial: `run_turn` completes (or is cancelled) before the next one starts.

9. **INV-BOUNDED:** All inter-task channels have finite capacity (except low-volume telemetry). Backpressure is handled by blocking the sender.

10. **INV-RESOURCE:** Whisper inference runs on CPU only. LLM owns GPU exclusively. No concurrent GPU contention.

11. **INV-GOAL:** The runtime serves assistant UX — not AGI, not autonomous agents, not recursive reasoning.

---

## 21. Vulnerabilities / Flaws Table

### All Identified Issues (Consolidated)

| # | Severity | Issue | Root Cause | Impact | Fix Phase |
|---|----------|-------|-----------|--------|-----------|
| 1 | 🔴 High | `CliWhisperStt` subprocess not killed on abort | `abort_rx` stops Rust task but child process orphaned | Zombie processes, stale resource hold | Phase 1 |
| 2 | 🔴 High | `run_turn` vs `run_speak_turn` externally raceable | No mutual exclusion between the two entry points | FSM corruption, dual state writes | Phase 1 |
| 3 | 🔴 High | All STT/TTS engines are CLI subprocesses | v2 skeleton uses stubs; no in-process impl | 2-5s TTFA, cold-load per call | Phase 2 |
| 4 | 🔴 High | Whisper-rs rolling-window 500ms may pile up | Fixed-timer cadence ignores actual inference time | CPU overload, partial queue growth | Phase 2 |
| 5 | 🟠 Medium | Persistent `OutputStream` recovery path lightly specified | Current: new stream per session. Failure = stuck | Silent output failure | Phase 1 |
| 6 | 🟠 Medium | `AtomicBool` inference guard may skip too many partials | Guard is binary: running/not. No fallback cadence | Long silence with no UI feedback | Phase 2 |
| 7 | 🟠 Medium | Overrun-triggered tier downgrade hysteresis unspecified | `OverrunTracker` fires but no recovery policy | Oscillation between tiers | Phase 2 |
| 8 | 🟠 Medium | Headphone mode detection manual-only | Config only; no auto-detection | Users forget to set mode | Phase 4 |
| 9 | 🟠 Medium | Partial transcript `seq` monotonicity edge cases | No seq field exists today | Race: stale partial after final | Phase 1 |
| 10 | 🟠 Medium | `force_abort` semantics distributed | 3 separate mutex acquisitions, not atomic | Potential partial-abort state | Phase 1 |
| 11 | 🟠 Medium | STT buffer grows unbounded in CliWhisperStt | `Vec::with_capacity(16_000*30)` but no hard cap | OOM on pathological input | Phase 1 |
| 12 | 🟠 Medium | New OutputStream per turn adds 50-200ms latency | `begin_session` opens fresh rodio stream | Wasted TTFA budget | Phase 1 |
| 13 | 🟡 Low | Long-lived playback sink resource leakage | Persistent stream never verified healthy | Stuck output after sleep/resume | Phase 1 |
| 14 | 🟡 Low | Fixed 60s VAD cap may truncate dictation | Hard cap prevents >60s utterances | Edge case for dictation users | Phase 4 |
| 15 | 🟡 Low | TTFA budgets hardware-sensitive | Fixed ms targets; variance across machines | Budget violations on slower hardware | Phase 2 |
| 16 | 🟡 Low | PipeWire/PulseAudio variance | CPAL ALSA backend; behavior varies | Silent device failures | Phase 1 |
| 17 | 🟡 Low | Broadcast channel lag during Thinking | 128 capacity; ~12.8s at 10 chunks/s | Unlikely but possible frame loss | — (acceptable) |

---

## 22. Accepted Fixes

### Phase 1 Fixes (Immediate, pre-engine work)

#### F1: Subprocess Kill on Abort
- **File:** `voice/v2/stt.rs` (`CliWhisperStt::start_stream`)
- **Change:** Store `Child` handle. On `abort_rx` fire: `child.kill().await` + `child.wait().await`
- **Lines affected:** ~15 lines in the spawn closure
- **Test:** Verify no zombie `whisper-cpp` processes after abort

#### F2: Turn Mutual Exclusion
- **File:** `voice/v2/pipeline.rs`
- **Change:** Add `turn_guard: Arc<tokio::sync::Mutex<()>>` to `VoicePipelineV2`. Both `run_turn` and `run_speak_turn` call `self.turn_guard.try_lock()` at entry; bail with `"turn already active"` if locked.
- **Lines affected:** +3 (struct field) + 5 per entry point
- **Test:** Concurrent `run_turn` + `run_speak_turn` → second returns error

#### F3: STT Buffer Cap
- **File:** `voice/v2/stt.rs` (`CliWhisperStt::start_stream`)
- **Change:** `const MAX_BUFFER_SAMPLES: usize = 16_000 * 60;` — break out of drain loop when exceeded
- **Lines affected:** +3
- **Test:** Feed >60s of audio → verify truncation without OOM

#### F4: Persistent OutputStream
- **File:** `voice/v2/playback.rs`
- **Change:** Hoist `OutputStream` + `OutputStreamHandle` into `PlaybackSink` struct. `begin_session` reuses existing stream; only creates fresh on first call or after failure.
- **Lines affected:** ~40 refactor
- **Test:** Two consecutive `begin_session` calls use same stream

#### F5: Centralized Abort
- **File:** `voice/v2/pipeline.rs` (`force_abort`)
- **Change:** Acquire `turn_cancel` and `playback` in defined order within one logical operation. Document lock ordering.
- **Lines affected:** ~10 refactor
- **Test:** `force_abort` under contention never deadlocks (stress test)

#### F6: Partial Transcript Seq Field
- **File:** `voice/v2/stt.rs`
- **Change:** Add `pub seq: u32` to `PartialTranscript` and `FinalTranscript`. STT backends increment per emission. Reset to 0 at turn start.
- **Lines affected:** +2 (struct) + 3 (increment logic)
- **Test:** Verify seq monotonically increases within a turn

#### F7: Headphone Mode Config
- **File:** `config.rs` (VoiceConfig), `voice/v2/pipeline.rs`, `commands/voice_runtime_helpers.rs`
- **Change:** Add `pub aec_mode: String` to VoiceConfig (default "half_duplex"). When "headphone": skip mic_muted toggle, enable barge-in watcher during Speaking.
- **Lines affected:** ~20 across files
- **Test:** Headphone mode + barge-in fires without AEC

#### F8: Playback Health Check
- **File:** `voice/v2/playback.rs`
- **Change:** Add `healthy: Arc<AtomicBool>`. Drain task sets false on rodio error. `begin_session` calls `ensure_healthy()` which attempts lazy reopen.
- **Lines affected:** ~25
- **Test:** Simulate device failure → next session reopens cleanly

#### F9: Audio Subsystem Logging
- **File:** `commands/voice_runtime_helpers.rs`
- **Change:** At pipeline start, log CPAL host ID and default device names. On Linux, attempt to read PipeWire vs PulseAudio vs ALSA status.
- **Lines affected:** ~15
- **Test:** Verify log output contains audio subsystem identification

### Phase 2 Fixes (Engine integration)

#### F10: Demand-Driven STT Partials
- **File:** `voice/v2/stt.rs` (new `WhisperRsStt` impl)
- **Change:** Implement inference loop with `AtomicBool` guard. Minimum finalization cadence: if inference_running >3s, force-emit last known partial and reset.
- **Lines affected:** ~150 new implementation
- **Test:** Verify never >1 concurrent whisper inference; verify forced partial after 3s stuck

#### F11: Overrun Hysteresis
- **File:** `voice/metrics.rs`
- **Change:** Extend `OverrunTracker` with `degraded_tier: Option<VoiceTier>`, `cooldown_remaining: u8`, `good_streak: u8`. On fire: downgrade tier, set cooldown=10. Recovery after 5 good turns.
- **Lines affected:** ~30
- **Test:** Simulate overrun sequence → verify tier drops → verify recovery after good turns

#### F12: Phase-Dependent TTFA Budgets
- **File:** `voice/tier.rs`
- **Change:** Add `ttfa_budget_ms_phase(phase: u8)` method. Phase 1=relaxed, Phase 2=moderate, Phase 3=target.
- **Lines affected:** ~15
- **Test:** Budget values match spec table

---

## 23. Rejected Ideas

| Idea | Reason for Rejection |
|------|---------------------|
| **Dynamic pipeline throttling states** | v2 loop is already serial. No concurrent resource conflict exists. |
| **Generation IDs on PCM chunks** | Premature. Current serial TTS loop + cooperative cancel already prevents stale data. Only needed if sentence prefetch is added (Phase 3+). |
| **Sink session supervisor** | Overengineered. Simple hoist of OutputStream + lazy reopen on failure suffices. |
| **Conversational audio filler ("hmm", "let me check")** | Anti-UX for sub-1s TTFA targets. Playing a 500ms filler delays the actual response. Use visual indicators instead. |
| **Bounded rolling voice context** | Already bounded at 5 turns via `get_recent_turns(5)`. No change needed. |
| **16kHz resampler abstraction** | 16kHz is correct for speech recognition. Higher rates add no benefit and double compute. |
| **TTS phrase caching** | Premature optimization. In-process piper-rs synth of short phrases takes ~50ms. Cache adds invalidation complexity (voice change, speed change) for negligible savings. |
| **Adaptive wake-word cooldown** | Static 500ms is fine. No user reports of double-trigger. Revisit only on evidence. |
| **Separate wake-mode vs conversation-mode runtime** | FSM already separates these. Wake word only runs during Sleeping. No architectural change needed. |
| **Manual SCHED_FIFO priority** | PipeWire manages audio thread RT priority automatically. Manual elevation causes priority inversions on PipeWire systems. |
| **Concurrent GPU whisper + LLM** | 6GB VRAM insufficient. Whisper on CPU is ~200ms for short utterances — acceptable. |
| **Cloud/hybrid voice fallback** | Violates local-first principle. All processing stays on-device. |
| **Emotional tone detection** | Out of scope. Not an assistant UX requirement. |
| **Speculative LLM pre-generation** | Unpredictable, wastes GPU cycles, violates determinism. |

---

## 24. Deferred Ideas

| Idea | Rationale for Deferral | Revisit When |
|------|----------------------|--------------|
| **Sentence prefetch** (synth N+1 while playing N) | Serial is sufficient for Phase 1-2. Only add if TTFA profiling shows inter-sentence gap is a bottleneck. Requires generation IDs. | Phase 3 profiling |
| **AEC full duplex** | Requires WebRTC APM + resampler (22050→16000) + frame aligner + delay estimation. Complex and hardware-dependent. Headphone mode gives barge-in for free. | Phase 3 |
| **Auto headphone detection** | Requires PipeWire/PulseAudio port introspection. Low priority vs manual toggle. | Phase 4 |
| **Configurable VAD cap (>60s)** | Edge case for dictation. Current 60s hard cap is sufficient for conversational turns. | Phase 4 |
| **Adaptive VAD silence timeout** | Fixed 300-500ms is fine for conversational interaction. Adaptive adds complexity. | Phase 4 |
| **Sub-sentence TTS chunking** | Splitting mid-sentence for even lower latency. Requires phoneme-boundary alignment. High complexity, marginal gain over sentence-level streaming. | Phase 4+ |
| **Multi-voice TTS** | Single voice is sufficient for assistant UX. | Post-v1.0 |
| **Voice activity continuity scoring** | Advanced turn-taking with overlap detection. Research-grade. | Post-v1.0 |

---

## 25. Detailed Implementation Phases

### Phase 1: Runtime Hardening (1-2 weeks)

**Goal:** Make v2 pipeline production-safe with CLI engines. Fix all correctness bugs. Ship headphone barge-in.

| Task | Priority | Effort | Fixes |
|------|----------|--------|-------|
| Add `turn_guard` mutual exclusion | Critical | 1h | F2 |
| Kill child process on CliWhisperStt abort | Critical | 2h | F1 |
| Cap STT buffer at 60s | High | 30min | F3 |
| Hoist OutputStream to persistent sink | High | 4h | F4, F8 |
| Centralize force_abort lock ordering | High | 1h | F5 |
| Add `seq` field to PartialTranscript | Medium | 1h | F6 |
| Add headphone mode config + wiring | High | 3h | F7 |
| Audio subsystem detection logging | Low | 1h | F9 |
| TTFA budget relaxation for Phase 1 | Medium | 30min | F12 |
| Update existing tests for new fields | Medium | 2h | — |

**Deliverables:**
- v2 pipeline safe to run as default (replace v1)
- Headphone users get zero-cost barge-in
- No zombie processes on abort
- No OOM on long utterances
- Playback survives device disconnects
- All 5 existing tests pass + new tests for F1-F9

**Validation:**
```bash
cargo test -p kria-core --features voice-whisper-rs
# Manual: start v2, speak, press abort → no zombie whisper-cpp
# Manual: headphone mode → speak during assistant → barge-in fires
# Manual: unplug speaker → next turn recovers automatically
```

---

### Phase 2: In-Process Engines (2-4 weeks)

**Goal:** Replace CLI subprocesses with in-process whisper-rs and piper-rs. Achieve <800ms TTFA on A-tier.

| Task | Priority | Effort | Fixes |
|------|----------|--------|-------|
| Wire `whisper-rs` crate into `WhisperRsStt` | Critical | 8h | F10 |
| Implement demand-driven partial loop | Critical | 6h | F10, #4, #6 |
| Wire `sonata-synth` (piper-rs) into `PiperRsTts` | Critical | 8h | — |
| Implement chunked TTS output (~120ms chunks) | High | 4h | — |
| Overrun hysteresis + tier degradation | Medium | 3h | F11 |
| Phase-dependent TTFA budgets | Medium | 1h | F12 |
| Reduce VAD silence timeout to 300ms (configurable) | Medium | 1h | — |
| Integration test: full turn with in-process engines | High | 4h | — |
| Benchmark: TTFA measurement on RTX 4050 | High | 2h | — |

**Deliverables:**
- whisper-rs running in-process on CPU (4 threads)
- piper-rs running in-process, streaming ~120ms chunks
- Partials appearing in UI every ~500ms during speech
- TTFA ≤800ms on A-tier hardware (measured)
- Tier auto-degradation on overruns with hysteresis

**Validation:**
```bash
cargo test -p kria-core --features voice-whisper-rs,voice-piper-rs
# Benchmark: run 10 voice turns → average TTFA < 800ms
# Verify: partials arrive during speech (check telemetry)
# Verify: overrun → tier drop → recovery (simulate slow inference)
```

---

### Phase 3: Advanced Realtime Orchestration (2-3 weeks)

**Goal:** Full-duplex via AEC or headphone mode proven in production. Sentence streaming fully optimized.

| Task | Priority | Effort | Fixes |
|------|----------|--------|-------|
| AEC integration (WebRTC APM) | High | 12h | — |
| Render-path resampler (22050→16000) | High | 4h | — |
| Frame splitter for AEC (10ms frames) | Medium | 3h | — |
| AEC delay estimation heuristic | Medium | 4h | — |
| Sentence prefetch (optional, profile-gated) | Low | 6h | — |
| Generation IDs on PCM if prefetch enabled | Low | 2h | — |
| VAD sensitivity tuning for AEC mode | Medium | 2h | — |
| End-to-end barge-in test with real audio | High | 4h | — |

**Deliverables:**
- AEC-duplex mode working for speaker+mic users
- Barge-in latency <50ms measured end-to-end
- TTFA ≤500ms on S-tier (in-process + streaming + persistent sink)
- Optional sentence prefetch behind feature flag

**Validation:**
```bash
cargo test -p kria-core --features voice-whisper-rs,voice-piper-rs,voice-aec
# Manual: speak while assistant plays → barge-in within 50ms
# Manual: verify no echo feedback in AEC mode
# Benchmark: 10 turns with AEC → TTFA < 500ms
```

---

### Phase 4: UX Refinement + Production Hardening (2-3 weeks)

**Goal:** Polish interaction quality. Ship to users.

| Task | Priority | Effort |
|------|----------|--------|
| GUI: live partial transcript display | High | 6h |
| GUI: assistant state indicator (animated) | High | 4h |
| GUI: TTFA debug overlay (toggle) | Medium | 4h |
| GUI: waveform/VAD visualization | Medium | 6h |
| GUI: device selection dropdowns | Medium | 3h |
| GUI: interruption UX (fade + badge) | High | 3h |
| GUI: degraded mode banner | Medium | 2h |
| GUI: recovery/failure messages | Medium | 3h |
| Wake word model shipping + activation | Medium | 4h |
| Auto headphone detection (PipeWire port) | Low | 4h |
| Configurable VAD cap for dictation | Low | 1h |
| Stress test: 100 consecutive turns | High | 4h |
| Memory leak audit (long session) | High | 4h |
| Latency regression test suite | High | 6h |

**Deliverables:**
- Full voice UI with real-time feedback
- Wake word functional with "Hey Ria"
- No memory leaks over 100+ turns
- Regression test suite for TTFA and barge-in latency
- Production-ready voice runtime

---

## 26. Testing Strategy

### Unit Tests (per module)

| Module | Tests | Focus |
|--------|-------|-------|
| `SentenceSplitter` | Push/flush, abbreviations, Hinglish, empty | Correctness |
| `PlaybackSink` | State transitions, abort, health check | Lifecycle |
| `CliWhisperStt` | Buffer cap, abort kills child, empty utterance | Safety |
| `OverrunTracker` | Fire on 3rd, reset, hysteresis | Degradation logic |
| `VoiceTier` | Tier mapping, budget values, overrides | Configuration |
| `MetricsBuilder` | Timestamp ordering, finalize | Telemetry |
| `PartialTranscript` | Seq monotonicity, per-turn reset | Ordering |

### Integration Tests (pipeline level)

| Test | What It Verifies |
|------|-----------------|
| `happy_path_runs_through_states` | Full turn: Sleeping→Listening→...→Sleeping ✅ exists |
| `barge_in_cancels_tts_immediately` | VAD SpeechStart during Speaking → cancel ✅ exists |
| `force_abort_returns_to_sleeping_idempotently` | Multiple abort calls safe ✅ exists |
| `force_wake_transitions_to_listening` | PTT path ✅ exists |
| `streaming_partials_pumped_to_telemetry` | Partial ordering in telemetry ✅ exists |
| `barge_in_latency_under_budget` | <200ms cancel latency ✅ exists |
| `concurrent_turn_rejected` | Two run_turn calls → second errors 🆕 Phase 1 |
| `stt_buffer_cap_prevents_oom` | >60s audio truncated 🆕 Phase 1 |
| `abort_kills_subprocess` | No zombie processes after abort 🆕 Phase 1 |
| `headphone_barge_in_without_aec` | Barge-in in headphone mode 🆕 Phase 1 |
| `playback_recovers_after_device_error` | Lazy reopen 🆕 Phase 1 |
| `overrun_triggers_tier_degradation` | 3 overruns → tier drops 🆕 Phase 2 |
| `tier_recovers_after_good_turns` | 5 good → restore 🆕 Phase 2 |
| `in_process_stt_never_overlaps` | AtomicBool guard 🆕 Phase 2 |
| `full_turn_with_whisper_rs` | Real STT + real TTS 🆕 Phase 2 |

### Stress Tests

| Test | Duration | Assertion |
|------|----------|-----------|
| 100 consecutive turns | ~5 min | No panics, no leaks, all returns Sleeping |
| Rapid abort spam | 1000 force_abort calls | Idempotent, no deadlock |
| Device disconnect during playback | — | Recovery on next turn |
| Barge-in during every sentence | 50 turns | All cancel within 200ms |

### Latency Regression

Automated benchmark run on CI (or locally):
```bash
cargo bench -p kria-core --bench voice_latency
# Measures: TTFA per tier, barge-in latency, partial cadence
# Asserts: TTFA within phase budget, barge-in < 200ms
```

---

## 27. Production Hardening

### Error Recovery Matrix

| Failure | Detection | Recovery | User Feedback |
|---------|-----------|----------|---------------|
| STT timeout (>45s) | tokio::time::timeout | Abort turn → Sleeping | "I didn't catch that" |
| TTS synth fails | anyhow::Result from engine | Show text response, skip audio | Text displayed in chat |
| Playback device lost | rodio error in drain task | `healthy = false`, lazy reopen | "Audio output unavailable" |
| Capture device lost | CPAL callback stops | broadcast closes → turn fails | "Microphone disconnected" |
| LLM timeout | No tokens for >30s | Abort turn | "I'm having trouble thinking" |
| Pipeline panic | tokio task panic → JoinError | Catch in v2 loop, restart turn | "Voice restarting..." |
| VRAM exhaustion | CUDA OOM | Downgrade tier to C (CPU-only) | "Switching to CPU mode" |

### Logging Standards

```rust
// All voice modules use structured tracing:
tracing::info!(engine = stt.engine_id(), "STT stream started");
tracing::warn!(err = %e, "TTS synthesis failed");
tracing::debug!(seq = partial.seq, text = %partial.text, "partial emitted");
tracing::error!("playback device lost, marking unhealthy");
```

### Telemetry Budget

| Event | Volume | Channel |
|-------|--------|---------|
| State transitions | ~6 per turn | unbounded (low volume) |
| Partials | 2-8 per turn | unbounded (low volume) |
| Final transcript | 1 per turn | unbounded |
| Metrics | 1 per turn | unbounded |
| BargeIn | 0-1 per turn | unbounded |
| Total per turn | ~10-17 events | Well within unbounded channel capacity |

### Memory Budget (per turn)

| Allocation | Size | Lifetime |
|-----------|------|----------|
| STT audio buffer | ≤3.84MB (60s × 16kHz × f32) | Turn |
| LLM token buffer | ≤64KB (64 tokens × 1KB) | Turn |
| TTS PCM chunks | ≤480ms × 22050Hz × f32 ≈ 42KB | Per sentence |
| Sentence splitter | ≤4KB | Turn |
| Telemetry events | ~2KB per event × 17 | Turn |
| **Total per-turn overhead** | **<5MB** | Freed at turn end |

---

## 28. Final Recommended Architecture

### Architecture Diagram (Target State)

```text
┌─────────────────────────────────────────────────────────┐
│                     KRIA Desktop (Tauri)                 │
│                                                         │
│  ┌──────────┐  ┌──────────────────┐  ┌───────────────┐ │
│  │  Voice   │  │  Conversation    │  │  Voice Debug  │ │
│  │  Panel   │  │  Panel           │  │  Overlay      │ │
│  │          │  │  - user bubbles  │  │  - TTFA meter │ │
│  │ ● State  │  │  - LLM stream   │  │  - tier badge │ │
│  │ 🎤 Mic   │  │  - interrupted  │  │  - waveform   │ │
│  │ 🔊 Spkr  │  │    markers      │  │  - VAD state  │ │
│  └──────────┘  └──────────────────┘  └───────────────┘ │
│         │               │                    │          │
│         └───────────────┼────────────────────┘          │
│                         │ Tauri Event Bus               │
│                         │ voice:state/partial/final/... │
├─────────────────────────┼───────────────────────────────┤
│                         │                               │
│              ┌──────────▼──────────┐                    │
│              │  VoicePipelineV2    │                    │
│              │  (single instance)  │                    │
│              │                     │                    │
│              │  ┌─── turn_guard ──┐│                    │
│              │  │ run_turn() XOR  ││                    │
│              │  │ run_speak_turn()││                    │
│              │  └─────────────────┘│                    │
│              │                     │                    │
│              │  FSM: Sleeping →    │                    │
│              │    Listening →      │                    │
│              │    Transcribing →   │                    │
│              │    Thinking →       │                    │
│              │    Speaking →       │                    │
│              │    [BargeIn] →      │                    │
│              │    Sleeping         │                    │
│              └──────────┬──────────┘                    │
│                         │                               │
│    ┌────────────────────┼────────────────────┐          │
│    │                    │                    │          │
│    ▼                    ▼                    ▼          │
│ ┌──────┐          ┌──────────┐        ┌──────────┐    │
│ │ STT  │          │   LLM    │        │   TTS    │    │
│ │(CPU) │          │  (GPU)   │        │  (CPU)   │    │
│ │      │          │          │        │          │    │
│ │whisp.│  final   │ tokens   │  sent  │ piper-rs │    │
│ │-rs   │ ──────→  │ stream   │ ────→  │ chunked  │    │
│ └──────┘          └──────────┘        └────┬─────┘    │
│                                            │ PCM      │
│                                            ▼          │
│                                     ┌────────────┐    │
│                                     │PlaybackSink│    │
│                                     │(persistent)│    │
│                                     │   rodio    │    │
│                                     └────────────┘    │
│                                                       │
│  ┌──────────┐    ┌─────┐    ┌──────────────────────┐ │
│  │AudioCapt.│───→│ VAD │───→│ Barge-In Watcher     │ │
│  │ (CPAL)   │    │Siler│    │ (headphone/AEC mode) │ │
│  │ 16kHz    │    └─────┘    └──────────────────────┘ │
│  └──────────┘                                        │
│                                                       │
│  ┌──────────┐ EchoMode:                              │
│  │ AEC      │ half_duplex → mic muted during play    │
│  │ (opt-in) │ headphone   → VAD barge-in, no AEC    │
│  │ WebRTC   │ aec_duplex  → full AEC, full barge-in │
│  └──────────┘                                        │
└───────────────────────────────────────────────────────┘
```

### Key Architectural Properties

1. **Single pipeline instance** — one `VoicePipelineV2` per app lifetime
2. **Serial turns** — one `run_turn` at a time, enforced by `turn_guard`
3. **Bounded task count** — max 5 spawned tasks per turn, all cancel-aware
4. **Resource-separated** — STT on CPU, LLM on GPU, no contention
5. **Three echo modes** — half_duplex (safe default), headphone (free barge-in), AEC (full duplex)
6. **Observable** — every state transition, partial, final, metric emitted to Tauri event bus
7. **Deterministic shutdown** — `force_abort()` always lands at Sleeping within 250ms
8. **Phased delivery** — each phase is independently shippable and testable

### Success Criteria

| Metric | Target | Measurement |
|--------|--------|-------------|
| TTFA (S-tier, Phase 3) | ≤500ms | `VoiceMetrics.t_first_audio_out_ms` |
| TTFA (A-tier, Phase 2) | ≤800ms | `VoiceMetrics.t_first_audio_out_ms` |
| Barge-in latency | ≤50ms | VAD fire → playback silence |
| Turn mutual exclusion | 100% | No concurrent turns ever |
| Subprocess cleanup | 100% | No zombie processes |
| Memory per turn | <5MB | Profiler measurement |
| 100-turn stability | 0 panics | Stress test |
| Playback recovery | <1 turn | After device disconnect |

---

*This document supersedes `VOICE_PLANNER.md` and `VOICE_PLANNER_REVIEW.md` as the single source of truth for KRIA's voice runtime implementation.*

*End of document.*

---

## NATIVE DEPENDENCY RISKS & ECOSYSTEM MATURITY ASSESSMENT (2026-05-13)

### piper-rs Crate Status (UPDATED)

| Aspect | Assessment |
|--------|-----------|
| Crate maturity | **Early-stage** (0.1.x) |
| Upstream compatibility | Upstream still mismatched with workspace ORT/ndarray |
| Local runtime status | ✅ **Unblocked via vendor patch** (`vendor/piper-rs`) |
| API stability | Acceptable for current Phase 2 scope after local fixes |
| Maintenance risk | Medium: local patch should be revisited when upstream updates |

### Dependency Graph Resolution Applied

**Implemented fix:**
- Added workspace override: `[patch.crates-io] piper-rs = { path = "vendor/piper-rs" }`
- Patched local `piper-rs` copy for ORT 2.0 rc12 compatibility:
  - aligned `ndarray` to `0.17.2`
  - aligned `ort` to `2.0.0-rc.12`
  - updated tensor extraction calls for ORT tuple API
  - updated tensor input calls to owned arrays (`Value::from_array`)
  - updated session run paths for mutable ORT session API

**Outcome:**
- `voice-piper-rs` feature-on path compiles and runs targeted v2 pipeline tests.
- Deterministic fallback remains intact when native runtime is unavailable.

### Ongoing Operational Notes

1. Keep bindgen env set in native builds:
   - `LIBCLANG_PATH=/lib/x86_64-linux-gnu`
   - `BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include"`
2. `voice-aec` (webrtc-audio-processing) remains host-package-dependent and is not required for Phase 2 completion.
3. Revisit/remove vendor patch when upstream `piper-rs` ships native ORT 2.0 compatibility.

---

## LIVE HOTFIX TRACKER — VOICE STALL + CANCELLATION DIAGNOSTICS (2026-05-14)

### DONE
- Added bounded speech endpointing in v2 capture loop (no-speech timeout, silence-based utterance finalization, max utterance cap).
- Relaxed capture gating to always forward frames to STT while still using RMS for endpoint timing.
- Reduced speech-start/end RMS thresholds to improve low-volume mic pickup.
- Added STT→LLM pipeline diagnostics (`voice:debug` + structured tracing):
  - turn start
  - LLM route start/ok/timeout/none
  - stream request/start timeout
  - first token latency
  - token stall timeout
  - stream completion token count
  - STT final emission breadcrumbs
- Promoted speech-start/awaiting-speech markers to `INFO` level for field diagnostics.
- Suppressed expected user-stop cancellation errors from surfacing as hard UI errors after `stop_voice`.
- Added temporary UI live STT chat mirroring:
  - partial transcript appears immediately as `🎤 (live) ...`
  - final transcript upgrades the same message to `🎤 ...`
  - helps isolate STT vs LLM/TTS bottlenecks during validation.

### IN PROGRESS
- Validate whether remaining "stuck in thinking" cases are now routed to explicit `voice:debug` timeout stages.

### RUNTIME RISKS
- LLM backend swap/recovery during turn can still increase first-token latency tails.
- `xcap` future-incompat warning remains unrelated to voice path but still present in workspace build output.

### PLAYBACK/RUNTIME ISSUES DISCOVERED
- Mid-turn manual stop was previously surfacing `turn cancelled before transcription` as a false-positive user-facing error.
- Partial STT visibility in chat was insufficient for diagnosing STT-vs-LLM failure boundary in live runs.

### TEST PROGRESS
- `cargo check -p kria-desktop --features kria-core/voice-whisper-rs,kria-core/voice-piper-rs` ✅
- `ui` production build ✅ (`npm run build`)
