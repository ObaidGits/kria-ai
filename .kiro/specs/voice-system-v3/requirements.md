# Requirements Document

Voice System v3 — Unified Production Pipeline

## Glossary

- **FSM**: Finite-State Machine governing the voice session lifecycle.
- **VAD**: Voice Activity Detection (Silero ONNX) — detects speech start/end.
- **Wake word**: openWakeWord-based phrase detection ("Hey Ria").
- **STT**: Speech-to-Text (faster-whisper / CTranslate2 in the Python sidecar; GPU INT8 `small` default, CPU fallback; Whisper-family, Hinglish/English).
- **TTS**: Text-to-Speech (Kokoro default, Piper fallback).
- **Barge-in**: User interrupting TTS playback by speaking.
- **TTFA**: Time To First Audio (first synthesized audio reaching the speaker).
- **AEC**: Acoustic Echo Cancellation.
- **PTT**: Push-To-Talk.
- **Turn**: One full wake/capture → STT → agent → TTS → playback cycle.
- **Tier**: Hardware capability class used to gate model/feature choices.

## V3 Revision (post faster-whisper benchmark)

This section is authoritative and supersedes earlier STT assumptions in this document where they conflict. It records the architecture decision taken after a real on-device benchmark.

### What changed and why
The original v3 plan kept STT as **in-process whisper-rs (whisper.cpp)** running the `large-v3-turbo` model. Latency forensics on the actual hardware (RTX 4050 6 GB, i7-13700HX, CPU-only whisper-rs build) proved this is the dominant latency cost: **a single large-model encode on CPU takes 7–13 s** (up to 17 s under thermal throttle), and the whisper-rs/whisper-cpp builds present cannot use the GPU. This made the in-process path unfixable within latency targets, and it also produced the `-6 failed to encode` instability and language-detect/retry amplifiers.

A controlled benchmark of STT alternatives on the same machine measured:

| Engine | Latency (~1.8 s clip) | Hinglish? | Notes |
|---|---|---|---|
| **faster-whisper `small` INT8 (GPU/CUDA)** | **~0.23 s** | **Yes** | +~460 MiB VRAM; 0% EN WER; Hindi ~0.31 s. **Chosen.** |
| faster-whisper `small` INT8 (CPU) | ~1.8 s | Yes | CPU fallback path |
| distil-large-v3 (GPU) | ~0.56 s | No (English-only) | disqualified for Hinglish |
| Moonshine | ~0.15 s | No (English-only) | disqualified for Hinglish |
| Parakeet | unrunnable on this stack | No | not Hinglish; disqualified |

**Decision:** STT moves from in-process whisper-rs (CPU) → **faster-whisper (CTranslate2) in the existing Python sidecar**, default **GPU INT8 `small`** with **CPU INT8 fallback** and **streaming partials**. Whisper-family is retained (faster-whisper is Whisper), so Hinglish/English support is preserved. Moonshine/Parakeet/distil-large-v3 remain disqualified as the default because they are English-centric, and are only ever allowed as opt-in English fast paths.

### Consequences for this spec
- **Requirement 6 (STT)** is revised below to specify the faster-whisper sidecar as the default engine.
- The following prior work/assumptions become **dead or superseded** and are scheduled for removal in the implementation plan: in-process `whisper-rs`, the v1 STT warmup, the CLI-whisper fallback as the *primary* fallback, and the whisper-rs band-aids added during stabilization (auto-language forcing, 3× encode-retry, min-window guard, partial silence-gate). The sidecar replaces the reason those existed.
- A new GPU/VRAM coordination concern appears (STT now shares the GPU with the resident LLM) and is captured in Requirement 6 and the design.

## Introduction

KRIA's voice stack currently ships two coexisting pipelines (v1 legacy + v2 streaming), with v2 active at runtime. A forensic audit (verified against code) found that voice modes do not function as configured, push-to-talk and wake word are effectively non-functional, CPU usage is extreme due to continuous large-Whisper partial decoding on non-speech audio, sessions can hang in transcription with no timeout, barge-in is unwired, several configuration knobs are silently ignored, and substantial dead/duplicate code exists.

Voice System v3 consolidates everything into a single, trait-based, production-grade pipeline. It removes the legacy v1 path and dead modules, wires the components that already exist (Silero VAD, openWakeWord), introduces a recovery/watchdog layer so no state can hang, gates the heavy STT model behind VAD/wake so idle CPU is low, fixes the text path so TTS never speaks markup or developer content, and upgrades TTS quality. The system must support Hinglish and English, run local-first, and remain extensible for future GUI Cognition integration.

### Verified constraints driving this spec
- Hinglish + English are mandatory → a Whisper-family engine remains the transcription default (Moonshine/Parakeet/distil-large-v3 are English-centric and are optional opt-in fast paths only).
- Latency first, accuracy second → the default STT engine must meet a sub-second target on this hardware; the only measured engine that is both sub-second AND Hinglish-capable is **faster-whisper `small` INT8 on GPU** (~0.23 s). In-process whisper-rs on CPU (7–13 s) does not meet this and is removed.
- Local-first, GPU-preferred with CPU fallback → faster-whisper runs GPU INT8 by default and falls back to CPU INT8 when no GPU/VRAM is available; no cloud dependency for the default path.
- STT now shares the GPU with the resident LLM → VRAM coordination is a first-class requirement (the `small` model fits the ~1 GB headroom; larger models must not be loaded alongside the LLM).
- One implementation per concern → no competing pipelines, and a single STT engine abstraction.

## Requirements

### Requirement 1: Single unified pipeline
**User Story:** As a maintainer, I want exactly one voice pipeline, so that there is no duplicate or dead code to maintain.

#### Acceptance Criteria
1. WHEN the application builds THEN the system SHALL contain exactly one runtime voice pipeline implementation.
2. WHEN voice is started THEN the system SHALL NOT construct or execute any legacy v1 pipeline.
3. IF a module under `voice/` is not referenced by the active pipeline THEN the system SHALL have that module removed from the codebase.
4. WHEN a developer searches for STT/TTS engines THEN the system SHALL expose each engine only behind a single `Stt`/`Tts` trait abstraction.

### Requirement 2: Functional voice modes
**User Story:** As a user, I want push-to-talk, continuous, and wake-word modes to behave as labeled, so that the mode I select is the mode I get.

#### Acceptance Criteria
1. WHEN `mode = "push_to_talk"` AND the configured key is pressed THEN the system SHALL open the mic and begin Listening, and WHEN released/toggled-off THEN the system SHALL end capture and transcribe.
2. WHEN `mode = "continuous"` THEN the system SHALL auto re-arm Listening after each completed turn without user action.
3. WHEN `mode = "wake_word"` AND the wake phrase is detected THEN the system SHALL transition from Idle to Listening.
4. WHILE in `wake_word` Idle state the system SHALL run only VAD and wake detection and SHALL NOT run the STT model.
5. WHEN an unknown mode value is configured THEN the system SHALL log a warning and fall back to a defined default mode.

### Requirement 3: Low idle CPU
**User Story:** As a user, I want low CPU usage when I am not speaking, so that voice can stay enabled without draining my machine.

#### Acceptance Criteria
1. WHILE in Idle or wake-listening state the system SHALL NOT execute STT model inference.
2. WHILE capturing speech the system SHALL run at most one partial STT inference per configured cadence, and partial inference SHALL be disabled by default on low-RAM tiers.
3. WHEN no speech is detected THEN the system SHALL NOT perform STT decoding on the silence.
4. WHEN partial transcription is disabled by configuration THEN the system SHALL honor that setting at runtime.

### Requirement 4: No stuck states (recovery layer)
**User Story:** As a user, I want voice to always recover, so that it never freezes in Listening or Thinking.

#### Acceptance Criteria
1. WHEN the system enters Transcribing THEN the system SHALL enforce a hard timeout, and IF exceeded THEN the system SHALL emit an error and return to Idle.
2. WHEN the system enters Thinking THEN the system SHALL enforce route and stream timeouts, and IF exceeded THEN the system SHALL emit an error and return to Idle.
3. WHEN any turn exceeds a maximum total duration THEN the system SHALL abort the turn and return to Idle.
4. WHEN a capture, STT, TTS, or playback failure occurs THEN the system SHALL surface a typed error event and recover to a known state.
5. WHEN `stop_voice` is invoked THEN the system SHALL abort the active turn and reach Idle within a bounded time.

### Requirement 5: Barge-in and interruption
**User Story:** As a user, I want to interrupt KRIA while it is speaking, so that conversations feel natural.

#### Acceptance Criteria
1. WHEN AEC is available (feature enabled) OR headphone mode is active AND the user speaks while the system is Speaking THEN the system SHALL cancel playback and transition to Listening.
2. WHEN barge-in fires THEN playback SHALL stop within a bounded latency target.
3. IF AEC is unavailable AND the mode is half-duplex THEN the system SHALL NOT claim voice barge-in is active and SHALL rely on the Stop control instead.
4. WHEN `voice.barge_in.enabled = false` THEN the system SHALL NOT cancel playback on detected speech.

### Requirement 6: Streaming STT (Hinglish + English) via faster-whisper sidecar
**User Story:** As a user, I want fast, accurate transcription in Hinglish and English, so that I can speak naturally with sub-second latency.

#### Acceptance Criteria
1. WHEN transcribing THEN the system SHALL use **faster-whisper (CTranslate2) running in the Python sidecar** as the default Hinglish/English engine, NOT in-process whisper-rs.
2. WHEN a CUDA GPU with sufficient free VRAM is available THEN the system SHALL run the `small` model with INT8 quantization on GPU as the default; WHEN GPU/VRAM is unavailable THEN the system SHALL fall back to `small` INT8 on CPU and log the fallback.
3. WHEN speech ends THEN the system SHALL produce a single authoritative final transcript.
4. WHEN partials are enabled THEN the sidecar SHALL emit advisory streaming partial transcripts that are never treated as authoritative.
5. WHEN the sidecar is unavailable (not started, crashed, or model missing) THEN the system SHALL surface a typed error and recover, and MAY fall back to a CPU engine; the system SHALL NOT hang waiting on the sidecar.
6. WHILE STT runs on GPU THEN the system SHALL respect VRAM coordination with the resident LLM and SHALL NOT load an STT model that exceeds the available headroom (default `small` ≈ +460 MiB fits ~1 GB free).
7. WHERE an English-only fast-path engine is enabled by the user (e.g. Moonshine/Parakeet/distil-large-v3) THEN the system SHALL allow it as an opt-in without removing the faster-whisper default.

### Requirement 7: Streaming TTS, speech-safe text
**User Story:** As a user, I want KRIA to speak clean natural responses, so that it never reads markup, punctuation symbols, or developer/system content.

#### Acceptance Criteria
1. WHEN the agent produces response tokens THEN the system SHALL sanitize text (strip markdown, code fences/inline code, tool-call/JSON scaffolding, emoji, raw URLs) BEFORE sentence splitting.
2. WHEN sanitized text is split into sentences THEN the system SHALL synthesize each sentence as it completes (streaming playback).
3. WHEN the agent emits a tool-call or structured/developer payload THEN the system SHALL NOT vocalize that payload.
4. WHEN a high-quality TTS engine is available THEN the system SHALL use it as the primary engine with a guaranteed fallback engine.
5. WHEN TTS is interrupted THEN synthesis SHALL stop without leaking queued audio.

### Requirement 8: Configuration integrity
**User Story:** As a user, I want every voice setting to take effect, so that the UI reflects real runtime behavior.

#### Acceptance Criteria
1. WHEN a voice setting is exposed in the UI THEN changing it SHALL change runtime behavior, OR the setting SHALL be removed.
2. WHEN configuration is changed THEN the system SHALL apply changes at a turn boundary and SHALL NOT corrupt an in-flight turn.
3. WHEN configuration is loaded THEN the system SHALL apply a documented precedence order (env > user config > default config > code default).
4. WHEN a setting is ignored by the active engine/tier THEN the system SHALL indicate this in the UI rather than silently ignoring it.

### Requirement 9: Accurate frontend state
**User Story:** As a user, I want the UI to show what voice is actually doing, so that it never appears stuck or unfinished.

#### Acceptance Criteria
1. WHEN the backend FSM changes state THEN the UI SHALL distinguish Listening, Capturing, Transcribing, Thinking, Speaking, Interrupt, and Error.
2. WHEN partial transcripts arrive THEN the UI SHALL render them as advisory (non-authoritative) and SHALL NOT write debug breadcrumbs into the chat transcript by default.
3. WHEN an error or latency spike occurs THEN the UI SHALL surface a health/latency indicator.
4. WHEN voice is stopped THEN the UI SHALL return to Idle.

### Requirement 10: Telemetry and observability
**User Story:** As an operator, I want enough telemetry to diagnose voice failures, so that issues are debuggable in the field.

#### Acceptance Criteria
1. WHEN any FSM transition occurs THEN the system SHALL emit a structured telemetry event.
2. WHEN STT/TTS/playback/VAD/wake operations run THEN the system SHALL log latency and outcome.
3. WHEN a failure occurs THEN the system SHALL log a typed, actionable error.
4. WHEN a turn completes THEN the system SHALL emit per-turn metrics (TTFA, STT latency, total turn time).

### Requirement 11: Optional wake daemon (extension)
**User Story:** As a power user, I want optional always-on wake-up even when the app is closed, so that I can summon KRIA hands-free, without compromising security or resources.

#### Acceptance Criteria
1. WHERE the optional wake daemon is enabled THEN it SHALL run unprivileged and run only VAD + wake detection (no STT/TTS/LLM).
2. WHEN the wake phrase is detected by the daemon THEN it SHALL signal the main app via IPC to launch/wake and start a session.
3. WHILE the daemon has mic access THEN the system SHALL present a visible indicator and an explicit permission flow.
4. IF the daemon is disabled or unavailable THEN in-app wake SHALL continue to function.

### Requirement 12: Non-regression and safety
**User Story:** As a maintainer, I want the migration to be safe and reversible, so that removing v1 does not break shipping behavior.

#### Acceptance Criteria
1. WHEN v3 lands THEN existing Tauri command names and voice event names SHALL remain stable contracts unless explicitly versioned.
2. WHEN a wave is merged THEN the build SHALL pass and voice SHALL function at least as well as before that wave.
3. IF a wave regresses voice THEN the change SHALL be revertible behind a feature flag or config toggle.
