# KRIA Voice Runtime

## Purpose

The voice subsystem provides bounded speech interaction for KRIA. It captures
microphone audio, detects speech, transcribes user input, hands committed text
to the normal orchestration path, and plays assistant speech back through TTS.

Voice is an interaction layer. It does not own policy, tool permission,
workflow planning, or final execution truth.

Primary implementation areas:
- `crates/kria-core/src/voice/`
- `crates/kria-desktop/src/commands/voice.rs`
- `crates/kria-desktop/src/commands/voice_runtime_helpers.rs`
- `crates/kria-desktop/src/commands/voice_diagnostics.rs`
- `docs/evaluations/voice-validation.md`

## Runtime Shape

KRIA currently carries two voice paths:

| Path | Role | Notes |
|---|---|---|
| v1 pipeline | Compatibility/default path | Capture, VAD, CLI Whisper STT, CLI Piper TTS, playback |
| v2 pipeline | Streaming path when `voice.engine = "v2"` and runtime is available | Streaming turn loop, sentence playback, hard abort/barge-in support, optional native backends by cargo feature |

`start_voice` validates binaries and model files, reloads config, rebuilds the
pipeline, and hot-swaps to v2 when the config requests it and v2 can be built.
If the v2 swap fails, KRIA continues with the v1 pipeline and logs the fallback.

## Voice Flow

Current v1 flow:

```text
start_voice
  -> reload config
  -> verify whisper-cpp/main and piper
  -> verify STT/TTS model files
  -> build VoicePipeline
  -> capture audio
  -> VAD
  -> STT final transcript
  -> AgentLoop turn
  -> TTS
  -> playback
  -> listening loop
```

Current v2 continuous flow:

```text
start_voice
  -> build or hot-swap ActivePipeline::Streaming
  -> start audio capture forwarder
  -> emit telemetry to UI
  -> run_turn loop
  -> wake/capture/STT
  -> committed transcript
  -> route model backend
  -> stream LLM tokens
  -> sentence splitter
  -> TTS chunks
  -> playback with hard abort/barge-in
```

Voice-originated turns still use normal model routing and normal tool safety
when they reach the agent runtime. The v2 helper intentionally filters search
tools from voice interactions to reduce accidental aggressive browsing during
spoken use.

## Core Components

| Component | File | Contract |
|---|---|---|
| Voice module exports | `crates/kria-core/src/voice/mod.rs` | Public voice runtime surface |
| v1 pipeline | `crates/kria-core/src/voice/pipeline.rs` | Capture, VAD, STT, TTS, playback state/events |
| v2 pipeline | `crates/kria-core/src/voice/v2/pipeline.rs` | Streaming turn execution and state |
| Runtime bridge | `crates/kria-core/src/voice/runtime_bridge.rs` | Coordinates FSMs and telemetry; not an orchestrator |
| Transcript authority | `crates/kria-core/src/voice/transcript_authority.rs` | Single transcript truth state machine |
| Turn ownership | `crates/kria-core/src/voice/turn_ownership.rs` | Conversational floor ownership and invalidation actions |
| Pre-commit policy | `crates/kria-core/src/voice/pre_commit_policy.rs` | Blocks LLM/tool/file/network actions before utterance commit |
| Runtime telemetry | `crates/kria-core/src/voice/runtime_telemetry.rs` | TTFA, queue pressure, latency, degradation |
| Tier resolver | `crates/kria-core/src/voice/tier.rs` | Hardware/config to voice tier profile |
| Capture/playback | `crates/kria-core/src/voice/capture.rs`, `crates/kria-core/src/voice/playback.rs` | Mic and speaker IO |
| STT/TTS | `crates/kria-core/src/voice/stt.rs`, `crates/kria-core/src/voice/tts.rs` | CLI-compatible speech engines |
| Audio enhancement | `crates/kria-core/src/voice/audio_enhance.rs` | Echo and spectral noise gates |
| Desktop commands | `crates/kria-desktop/src/commands/voice.rs` | Start/stop/status/v2 speak/abort commands |
| Diagnostics | `crates/kria-desktop/src/commands/voice_diagnostics.rs` | v2 status and audio transcription debug helpers |

## Authority Model

Transcript authority states:

```text
S0Idle
  -> S1Speculative
  -> S2Stabilizing
  -> S3Committed
  -> S4RefinedFinal
```

Rules:
- partial transcripts are provisional,
- execution uses the S3 committed transcript,
- refinement may change the visible string after commit but must not rewrite
  the executed transcript,
- generation mismatch drops stale partials,
- Whisper refinement is not transcript authority.

Turn ownership states:

```text
Idle
  -> Listening
  -> Processing
  -> Speaking
  -> Idle
```

Interruption states:

```text
Speaking
  -> Interrupting
  -> Listening

any active state
  -> Cancelling
  -> Idle
```

Turn ownership emits invalidation actions such as:
- cancel current turn token,
- increment generation,
- flush audio/partial queues,
- cancel pending refinement,
- stop TTS,
- stop LLM stream,
- notify sidecar generation change,
- reset transcript authority.

The FSMs emit required actions; runtime code executes them. They do not make
tool or policy decisions.

## Pre-Commit Safety

Before `UtteranceCommitted`, the host must not run:
- LLM generation,
- tool calls,
- filesystem writes,
- network actions.

Allowed pre-commit actions are intentionally small and reversible:
- stop TTS,
- cancel turn,
- mute mic,
- reduce volume.

This prevents unstable partial transcripts from causing side effects.

After commit, the transcript is treated as normal user input and goes through
the same orchestration, policy, HITL, target authority, and tool execution path
as typed input.

## Configuration

Primary `VoiceConfig` fields:

| Field | Meaning |
|---|---|
| `voice.enabled` | Enables voice features |
| `voice.mode` | Interaction mode, for example push-to-talk/headphone behavior |
| `voice.engine` | `v1` or `v2` |
| `voice.tier` | `auto`, `s`, `a`, or `c` |
| `voice.stt_model` | STT model file |
| `voice.tts_voice` | Piper voice name |
| `voice.stt_engine` | `auto`, `whisper-rs`, `whisper-cuda`, or sidecar-oriented engine |
| `voice.tts_engine` | `auto`, `piper-cli`, or `piper-rs` |
| `voice.language` | STT language hint |
| `voice.vad_silence_ms` | Silence threshold for end-of-utterance |
| `voice.energy_threshold` | VAD energy threshold |
| `voice.partial_update_ms` | Partial transcript cadence |
| `voice.enable_partial_transcripts` | Enables live partials; off by default for v1 CLI backend |
| `voice.follow_system_default_mic` | Restart capture when default mic changes |
| `voice.follow_system_default_speaker` | Follow default speaker output |
| `voice.noise_suppression_mode` | Audio noise suppression mode |
| `voice.wake_word` | Wake-word model, sensitivity, aliases |
| `voice.aec` | Optional acoustic echo cancellation config |
| `voice.barge_in` | Barge-in debounce/settings |
| `voice.post_edit` | Transcript post-edit/fix-pass config |

Tier defaults:

| Voice tier | Hardware mapping | TTFA budget |
|---|---|---:|
| S | Performance/high hardware | 500 ms |
| A | Standard hardware | 800 ms |
| C | Lite/CPU-constrained hardware | 1200 ms |

`VoiceTierProfile` resolves engine choices, model names, AEC aggressiveness,
post-edit behavior, and TTFA budget from config plus detected hardware.

## v2 Backends And Features

The v2 module compiles without native-heavy features and can use CLI fallback
engines. Native features are optional:

| Feature | Effect |
|---|---|
| `voice-whisper-rs` | Whisper Rust backend |
| `voice-whisper-cuda` | CUDA support for Whisper backend |
| `voice-whisper-vulkan` | Vulkan support for Whisper backend |
| `voice-piper-rs` | In-process Piper-compatible TTS |
| `voice-aec` | WebRTC APM acoustic echo cancellation |
| `voice-wake-oww` | openWakeWord detector |

`voice_v2_status` reports resolved tier, engine choices, compiled features,
wake-word model paths, and model presence. It is the first diagnostic command to
run when v2 behavior is unclear.

## Desktop Events

Voice commands emit Tauri events used by the UI:

| Event | Meaning |
|---|---|
| `voice:state` | `idle`, `listening`, `processing`, or `speaking` |
| `voice:partial_transcript` | Provisional transcript text |
| `voice:transcript` | Final committed transcript |
| `voice:error` | Runtime error |
| `voice:io_mode` | Headphone/half-duplex mode |
| `voice:debug` | Diagnostic stage details |
| `voice:v2_telemetry` | Raw v2 telemetry payload |
| `voice:busy` | Runtime rejected a new entry while busy |
| `voice:playback_failure` | Playback failed |
| `voice:playback_recovered` | Playback recovered |
| `voice:interruption` | Barge-in or cancel event |

## Observability

Tracked signals include:
- TTFA p50/p95/p99 and overrun rate,
- interrupt and cancel latency,
- audio queue pressure,
- partial queue pressure,
- Whisper worker utilization,
- total turns,
- total interruptions,
- total barge-ins,
- degradation level.

Degradation levels:
- none: normal,
- light: skip optional refinement,
- moderate: skip refinement and reduce partial pressure,
- heavy: emergency mode, skip optional processing.

## Operational Checks

Before production validation:
- `whisper-cpp` or `main` binary is on `PATH`,
- `piper` binary is on `PATH`,
- configured STT model exists under the resolved model directory,
- configured Piper voice model exists,
- mic and speaker devices are valid or set to `auto`,
- v2 status reports expected compiled features and model paths,
- barge-in and stop commands cancel TTS promptly,
- voice-originated tool actions still trigger normal policy/HITL gates.

Useful commands/surfaces:
- `start_voice`,
- `stop_voice`,
- `get_voice_status`,
- `voice_v2_status`,
- `voice_v2_speak`,
- `voice_v2_abort`,
- `voice_transcribe_audio_file`,
- `voice_transcribe_uploaded_audio`.

## Failure Handling

| Failure | Expected behavior |
|---|---|
| Missing Whisper/Piper binary | `start_voice` returns clear setup error |
| Missing model file | `start_voice` returns model path error |
| Mic capture failure | Emit `voice:error`, stop active state |
| TTS failure | Degrade to text/event error and continue when possible |
| LLM route timeout | Spoken response explains timeout/no backend |
| Barge-in | Stop TTS and transition back to listening |
| Stop voice | Set inactive, abort v2 turn, stop v1 pipeline, emit idle |
| Queue pressure | Telemetry updates degradation level |

## Production Invariants

1. Voice partials are never execution authority.
2. A side effect cannot run before utterance commit.
3. Voice input cannot bypass orchestration, policy, HITL, target authority, or
   verifier rules.
4. Exactly one conversational owner is active at a time.
5. Barge-in and stop must invalidate stale audio, transcript, LLM, and TTS work.
6. Runtime behavior must be observable through events and telemetry.
7. Missing dependencies must fail clearly, not silently.
8. Production claims require the voice validation runbook.

Use `docs/evaluations/voice-validation.md` for acceptance testing before
claiming production readiness.
