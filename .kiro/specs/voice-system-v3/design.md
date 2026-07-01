# Design — Voice System v3

## Overview

Voice System v3 replaces the dual v1/v2 stack with a single, layered, trait-based pipeline owned by one finite-state machine (FSM) and protected by a recovery/watchdog layer. The heavy STT model is gated behind VAD/wake so idle CPU is near-zero. STT is moved out of the in-process whisper-rs (CPU-only, 7–13 s/decode on this hardware) into the existing **Python sidecar running faster-whisper (CTranslate2)**, default **GPU INT8 `small`** (~0.23 s measured) with **CPU INT8 fallback** and **streaming partials**; Whisper-family is retained so Hinglish/English is preserved. Text destined for TTS is sanitized before sentence splitting so markup and developer/structured content are never vocalized. TTS is upgraded to a higher-quality local engine (Kokoro) with Piper retained as a guaranteed fallback. Every exposed configuration knob maps to a real runtime effect or is removed. An optional, unprivileged wake daemon provides cold-start always-on without embedding heavy work outside the app.

This design is derived from a code-verified forensic audit, three architecture critique iterations, and a fourth revision after a real on-device STT benchmark (documented in the planning report and in the requirements "V3 Revision" section). It deliberately keeps the genuinely valuable parts of the current v2 code (FSM skeleton, sentence splitter, playback sink, Silero VAD, openWakeWord backend, `normalize_for_tts`) and discards the rest, including the in-process whisper-rs STT path and its stabilization band-aids.

## Architecture

### Layered component model

```
AudioCaptureLayer (cpal, blocking thread)
   → PreprocessLayer (noise suppression + AEC[feature])
   → VadLayer (Silero ONNX): speech start/end + barge-in signal
   → WakeLayer (openWakeWord): active only in Idle/wake mode
   → SessionLayer (FSM): owns transitions, modes, cancellation
       → SttLayer (trait): SidecarFasterWhisper(default, GPU INT8 small / CPU fallback) | EnglishFast[opt]
       → AgentLayer: model router (+ optional tool-aware path)
       → SpeechTextSanitizer → SentenceSplitter
       → TtsLayer (trait): Kokoro(default) | Piper(fallback)
       → PlaybackLayer (rodio, hard-abort)
   TelemetryLayer  (observes all layers → Tauri events)
   RecoveryLayer   (per-state watchdog/timeouts; wraps SessionLayer)
   ConfigLayer     (single typed config; turn-boundary hot reload)
   ExtensionLayer  (optional unprivileged wake daemon → IPC)
```

### Why these choices (traceability)
- **faster-whisper sidecar is the default STT** (Whisper-family → Hinglish-capable per Req 6) because it is the only measured engine that is both sub-second on this hardware (GPU INT8 `small` ≈ 0.23 s) and multilingual; in-process whisper-rs (CPU, 7–13 s) is removed. Moonshine/Parakeet/distil-large-v3 are English-centric and allowed only as opt-in English fast paths.
- **Kokoro becomes default TTS** (Apache-2.0, ONNX via existing `ort`, Hindi + English, more natural than Piper); Piper remains the fallback (Req 7).
- **Silero VAD becomes the single VAD** (already vendored, currently wasted); inline RMS heuristic is removed (Req 3, 5).
- **openWakeWord is wired** (already implemented, models on disk) rather than replaced (Req 2, 11).
- **RecoveryLayer** is new and is the guarantee against stuck states (Req 4).

## State Machine

```
                ┌─────────┐
                │  Idle   │◀────────────── stop_voice / error-recover / turn-complete(PTT)
                └────┬────┘
   PTT key / wake / continuous-arm
                     ▼
                ┌──────────┐  VAD speech start   ┌───────────┐
                │Listening │────────────────────▶│ Capturing │
                └────┬─────┘                      └────┬──────┘
        no-speech timeout│                            │ VAD endpoint / max-utterance
                     ▼   │                            ▼
                  Idle ◀─┘                      ┌──────────────┐  empty/low-conf
                                                │ Transcribing │──────────────▶ Idle
                                                └──────┬───────┘
                                                  final│  [watchdog timeout → Error]
                                                       ▼
                                                ┌──────────┐  first token  ┌──────────┐
                                                │ Thinking │──────────────▶│ Speaking │
                                                └──────────┘ [timeout→Error]└────┬─────┘
                                                                                 │
                              barge-in (VAD speech start + AEC/headphone)         │ playback done
                                                       ◀─────────────────────────┤
                                                ┌───────────┐                     │
                                                │ Interrupt │──▶ Listening        ▼
                                                └───────────┘            Idle | Listening(continuous)

  Error ──(emit voice:error, recover)──▶ Idle
```

### State ownership & timeouts (RecoveryLayer)
| State | Entry | Exit | Timeout / failure |
|---|---|---|---|
| Idle | stop/turn-complete | wake/PTT/arm | none |
| Listening | armed | VAD start / no-speech timeout | no-speech → Idle |
| Capturing | VAD start | VAD endpoint / max-utterance | max-utterance → Transcribing |
| Transcribing | capture end | final / empty | hard timeout → Error |
| Thinking | final | first token | route+stream timeout → Error |
| Speaking | first token | playback done / barge-in | bounded playback; stall → Error |
| Interrupt | barge-in | → Listening | bounded |
| Error | any failure | → Idle | always recovers |

## Components and Interfaces

### SttLayer
```
trait Stt {
  fn engine_id(&self) -> &'static str;
  async fn start_stream(self: Arc<Self>, pcm_rx, partial_tx) -> StreamHandle;
}
```
- `SidecarFasterWhisperStt` (default): a thin Rust client over the existing Python sidecar (JSON-RPC bridge). The sidecar loads faster-whisper / CTranslate2 once and keeps it warm. Default model `small`, INT8, `device=cuda` when a GPU with free VRAM is present, else `device=cpu` INT8. Final decode runs on the VAD-bounded utterance; streaming partials are emitted by the sidecar, advisory-only, rate-capped, and tier-gated off on low RAM. Language left to Whisper auto-detect (Hinglish/English).
- `EnglishFastStt` (opt-in, e.g. Parakeet/Moonshine/distil-large-v3 ONNX): only when the user enables an English fast path; never replaces the faster-whisper default.
- **GPU/VRAM coordination:** the sidecar negotiates VRAM with the resident LLM. `small` INT8 (~+460 MiB) fits the ~1 GB free headroom; the client refuses to request a larger model on GPU when the LLM is resident, and downgrades to CPU INT8 instead. The model is loaded once and kept warm, not reloaded per turn.
- **Fallback chain:** GPU faster-whisper → CPU faster-whisper → typed error + recover (never hang). The sidecar liveness is health-checked; a dead sidecar yields a typed STT error, not a stuck Transcribing state.
- **Removed:** in-process `WhisperStt`/whisper-rs, `CliWhisperStt` as the primary fallback, `SidecarStt` (old), the v1 STT warmup, and the whisper-rs stabilization band-aids (auto-language forcing, 3× encode retry, min-window guard, partial silence-gate) — the sidecar supersedes the conditions that required them.

### TtsLayer
```
trait Tts {
  fn engine_id(&self) -> &'static str;
  fn sample_rate(&self) -> TtsSampleRate;
  async fn synthesize_sentence(self, sentence, pcm_tx, abort_rx) -> Result<()>;
}
```
- `KokoroTts` (default): in-process via `ort`, streamed PCM chunks.
- `PiperTts` (fallback): existing CLI/`ort` path.
- All engines receive already-sanitized text.

### SpeechTextSanitizer
- Runs BEFORE `SentenceSplitter` (fixes the split-before-normalize defect).
- Extends `normalize_for_tts`: strip code fences/inline code, markdown, bullets/headers, emoji, raw URLs, AND tool-call/JSON/structured scaffolding; collapse whitespace.
- Agent voice path either (a) routes through a tool-aware parser that never streams tool syntax to TTS, or (b) sanitizes structured tokens out before splitting.

### VadLayer (Silero)
- Single VAD instance feeds both endpointing (Capturing→Transcribing) and barge-in (Speaking→Interrupt).
- Configurable silence/threshold values are honored at runtime (Req 8).

### WakeLayer (openWakeWord)
- Spawned with a broadcast `AudioChunk` subscription in Idle/wake mode; emits wake events into the FSM.
- Feature-gated build flag shipped enabled where models are present; disabled cleanly otherwise.

### SessionLayer (FSM)
- Owns the per-turn `CancellationToken`, mutual exclusion (one active turn), and mode semantics.
- Contains no engine-specific logic; engines are injected as `Arc<dyn Stt/Tts>`.

### RecoveryLayer
- Wraps the FSM; assigns each state a deadline; on breach emits typed `voice:error` and forces Idle.
- Watchdog also covers external stalls (device lost, model hang).

### ConfigLayer
- One typed `VoiceConfig`; validated on load; precedence env > user > default > code.
- Hot reload queued to turn boundary; mid-turn changes deferred.
- Removes/garbage-collects knobs that cannot map to runtime.

### TelemetryLayer
- Emits a structured event per transition; per-turn metrics (TTFA, STT latency, total turn).
- Stops collapsing Transcribing+Thinking into "processing" at the event boundary.

### ExtensionLayer (optional wake daemon)
- Separate unprivileged process: VAD + openWakeWord only.
- IPC (local socket) → main app `wake` signal → launch/wake + start session.
- Visible mic indicator + explicit permission; in-app wake remains the fallback.

## Frontend interface (contracts)
- Keep command names: `start_voice`, `stop_voice`, `voice_transcribe_uploaded_audio`, `list_audio_devices`; wire `voice_v2_speak`/`voice_v2_abort` to UI controls.
- Event contract extended (additive): `voice:state` carries the full FSM state string (`idle|listening|capturing|transcribing|thinking|speaking|interrupt|error`).
- Partial transcripts advisory in overlay only; debug breadcrumbs behind a dev flag (not in chat).
- New UI: distinct state labels, mic-level meter, latency/health indicators, mode switch reflecting real behavior, onboarding wake test.

## Data Models
- `VoiceSessionState` extended to the 8 states above.
- `FinalTranscript { text, language, confidence, duration_ms, engine }` (unchanged).
- `PartialTranscript { text, seq, confidence, engine }` (advisory).
- `VoiceTelemetry` extended with explicit per-state events + per-turn metrics.

## Error Handling
- Typed errors per layer (capture/vad/wake/stt/tts/playback/config).
- Every error path resolves to Idle via RecoveryLayer; UI shows actionable message.
- Fallbacks: STT engine → GPU faster-whisper → CPU faster-whisper → typed error/recover (no hang); TTS engine → Piper; wake daemon → in-app wake; AEC absent → no voice-barge-in claim.

### Sidecar & GPU risks (new in V3 revision)
- **Sidecar process is now on the STT critical path.** Mitigation: warm load at voice start, health-check/liveness probe, hard STT timeout (RecoveryLayer) so a dead/slow sidecar surfaces a typed error and returns to Idle rather than hanging.
- **IPC latency/serialization.** PCM is streamed to the sidecar; the bridge must avoid per-chunk JSON overhead for audio (binary/length-prefixed frames or a shared ring), keeping the GPU-decode win (~0.23 s) from being eaten by transport.
- **VRAM contention with the resident LLM (~1 GB free).** Mitigation: `small` INT8 only on GPU; refuse larger GPU models when LLM resident; CPU INT8 fallback; never reload per turn.
- **Cold start.** First sidecar model load is multi-second; load happens at session start (not first utterance) and is covered by a startup state, not the turn watchdog.
- **Python/dependency footprint.** faster-whisper + CTranslate2 + CUDA libs add to the sidecar; gate behind the sidecar's existing optional-service contract (voice degrades to CPU/typed-error, never hard-fails the app).

## Testing Strategy
- Unit: sanitizer (markup/JSON/emoji cases), sentence splitter ordering, VAD endpoint, FSM transition table, timeout/watchdog behavior, config precedence.
- Integration: synthetic-audio turn (wake→capture→STT→agent stub→TTS→playback), barge-in cancel latency, no-speech timeout, stuck-state recovery, mode behavior (PTT/continuous/wake).
- Performance: idle CPU (must be near-zero with STT gated), partial-rate cap, TTFA budget per tier.
- Regression: command/event contract stability; engine fallback paths.
- Manual/live: device-loss recovery, headphone vs half-duplex barge-in, Hinglish accuracy spot-check.

## Correctness Properties

### Property 1: No-hang invariant
For every reachable FSM state there exists a bounded-time transition to Idle (enforced by RecoveryLayer deadlines). No state is terminal except Idle.
**Validates: Requirements 4.1, 4.2, 4.3, 4.5**

### Property 2: Single-turn mutual exclusion
At most one active turn exists at any time; a second turn request is rejected, never interleaved.
**Validates: Requirements 1.1, 4.5**

### Property 3: STT gating invariant
STT model inference occurs only while in Capturing (or rate-capped partials during Capturing); never in Idle, Listening(wake), or on detected silence.
**Validates: Requirements 3.1, 3.2, 3.3, 2.4**

### Property 4: Cancellation propagation
A single per-turn cancellation token, when cancelled, halts STT, agent stream, TTS synthesis, and playback within a bounded latency.
**Validates: Requirements 4.5, 5.2, 7.5**

### Property 5: Speech-safe output
Any text reaching a TTS engine has passed the sanitizer; tool-call/structured/markup tokens are never synthesized.
**Validates: Requirements 7.1, 7.3**

### Property 6: Config consistency
Runtime behavior always corresponds to the loaded config; config changes apply only at turn boundaries (never mid-turn).
**Validates: Requirements 8.1, 8.2, 8.3**

### Property 7: Contract stability
Tauri command names and `voice:*` event names remain backward-compatible; state additions are additive string values.
**Validates: Requirements 12.1, 9.1**

### Property 8: Fallback totality
Each engine layer (STT, TTS, wake) has a defined fallback so the pipeline remains functional when a preferred engine is unavailable.
**Validates: Requirements 6.2, 6.5, 7.4, 11.4**
