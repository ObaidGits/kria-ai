# KRIA Voice Pipeline

> **Last Updated:** 2026-05-13
> **Status:** Production (v1 default, v2 streaming scaffolded)

---

## Overview

KRIA supports voice-first interaction with speech-to-text (STT), text-to-speech (TTS), wake-word detection, and voice activity detection (VAD). The voice pipeline is designed for sub-500 ms time-to-first-audio (TTFA) on simple commands.

Two runtime architectures coexist:
- **v1** (`VoicePipeline`) — turn-based: capture → transcribe → respond → synthesize. Default.
- **v2** (`VoicePipelineV2`) — streaming with sentence-level playback, hard barge-in, and persistent in-process orchestration. Scaffolded; falls back to v1 CLI engines when native backends are not compiled.

---

## Architecture

### v1 Pipeline (Default)

```
Microphone → AudioCapture ──→ STT (whisper-cpp CLI) → AgentLoop
                                      ↓                    ↓
                                VAD (silero_vad.onnx)      TTS (piper CLI)
                                      ↓                    ↓
                                Wake Word (optional)  →  Speakers
```

### v2 Pipeline (Streaming)

```text
         ┌──────────────────────────────┐
         │   AudioCapture (broadcast)   │
         └──────┬───────────────────────┘
                │ AudioChunk (16 kHz mono f32)
                ▼                       ▼
          ┌───────────┐         ┌───────────────┐
          │  STT chan │         │  VAD watcher  │
          └─────┬─────┘         └───────┬───────┘
                │ FinalTranscript       │ SpeechStart while Speaking
                ▼                       │
          ┌───────────┐                 │
          │ LLM token │                 │
          │  stream   │                 │
          └─────┬─────┘                 │
                ▼                       │
          ┌───────────────┐             │
          │ SentenceSplit │             │
          └─────┬─────────┘             │
                ▼                       │
          ┌───────────┐ ◄─────────────┘
          │ TTS / Play│    (CancellationToken)
          └───────────┘
```

Hard barge-in semantics: when VAD reports `SpeechStart` while `Speaking`, a single `CancellationToken::cancel()` propagates to TTS synthesis, playback drain, LLM token stream, and sentence splitter — all within the same scheduler tick.

---

## Components

| Component | v1 Backend | v2 Backend | Purpose |
|-----------|------------|------------|---------|
| **STT** | `whisper-cpp` CLI | `whisper-rs` / `CliWhisperStt` / `SidecarStt` | Speech to text |
| **TTS** | `piper` CLI | `piper-rs` / `CliPiperTts` | Text to speech |
| **Wake Word** | — | `openWakeWord` ONNX | Hands-free activation |
| **VAD** | `silero_vad.onnx` | `silero_vad.onnx` | Voice activity detection |
| **AEC** | — | WebRTC APM (feature-gated) | Acoustic echo cancellation |
| **Post-Edit** | — | `HinglishPostEditor` | Hinglish transcript fix-pass |

---

## Configuration

```toml
[voice]
enabled = true
mode = "push_to_talk"          # "push_to_talk" | "continuous"
stt_model = "auto"             # "auto" | "ggml-large-v3-turbo-q5_0.bin" | "ggml-medium-q5_0.bin" | "ggml-small-q5_1.bin"
stt_engine = "auto"            # "auto" | "whisper-rs-cuda" | "whisper-rs" | "piper-rs" | ...
tts_voice = "en_US-ljspeech-high"
tts_engine = "auto"
vad_silence_ms = 500
energy_threshold = 0.02
mic_device = "auto"            # "auto" | device name
speaker_device = "auto"
push_to_talk_key = "ctrl+space"
language = "auto"              # "auto" | "en" | "hi" | ...
partial_update_ms = 2000
noise_suppression_mode = "off" # "off" | "light" | "aggressive"
follow_system_default_mic = true
follow_system_default_speaker = true

[voice.wake_word]
enabled = false
model_path = ""                # "" defaults to "hey_ria.onnx"
sensitivity = 0.5
aliases = ["ria", "kria"]

[voice.aec]
enabled = false
aggressiveness = "medium"      # "low" | "medium" | "high"

[voice.post_edit]
enabled = true
mode = "on_low_confidence"     # "always" | "on_low_confidence"
timeout_ms = 0                 # 0 = tier default
model = "qwen2.5-3b"
```

### Resolution order for `stt_model` and `stt_engine`

1. Explicit config value (non-empty, non-"auto")
2. `"auto"` → resolved from detected hardware tier
3. Legacy `"ggml-base.en.bin"` → silently upgraded to tier-appropriate model

---

## Hardware Tiers & Model Selection

`VoiceTier` (S / A / C) is derived from `HardwareTier` (High / Performance / Standard / Lite):

| HardwareTier | VoiceTier | STT Model | STT Engine | TTS Engine | TTFA Budget |
|--------------|-----------|-----------|------------|------------|-------------|
| High | S | `ggml-large-v3-turbo-q5_0.bin` | `whisper-rs-cuda` | `piper-rs` | 500 ms |
| Performance | S | `ggml-large-v3-turbo-q5_0.bin` | `whisper-rs-cuda` | `piper-rs` | 500 ms |
| Standard | A | `ggml-large-v3-turbo-q5_0.bin` | `whisper-rs-cuda` | `piper-rs` | 800 ms |
| Lite | C | `ggml-small-q5_1.bin` | `whisper-rs` | `piper-rs` | 1200 ms |

Override precedence:
1. `KRIA_TIER` environment variable
2. `config.hardware.tier`
3. Cached `hardware_tier.json`
4. Fresh `detect_hardware()`

---

## Model Path Resolution

Model files are resolved through `resolve_model_file()` in `voice_runtime_helpers.rs`:

1. **Managed location**: `KRIA_MODELS_DIR/<subdir>/` or `~/.kria/models/<subdir>/`
2. **Workspace fallback**: walks up from CWD looking for `models/<subdir>/` — covers Tauri dev runs where `download_models.py` places files under the project root
3. Returns the primary path even if missing, so callers can emit a clear error

| Subsystem | Subdir | Default File |
|-----------|--------|--------------|
| STT | `stt` | tier-dependent `.bin` |
| TTS | `piper` | `{voice}.onnx` |
| VAD | `vad` | `silero_vad.onnx` |
| Wake Word | `wake` | `hey_ria.onnx` |

### Environment Variables

| Variable | Purpose |
|----------|---------|
| `KRIA_MODELS_DIR` | Override the base models directory |
| `KRIA_TIER` | Override hardware tier (`lite`, `standard`, `performance`, `high`) |
| `KRIA_REDETECT` / `KRIA_REDETECT_HARDWARE` | Force fresh hardware detection |

---

## Cargo Features (v2 Native Backends)

All default **OFF** except the pure-Rust scaffolding:

| Feature | Description |
|---------|-------------|
| `voice-whisper-rs` | In-process whisper.cpp via `whisper-rs` FFI |
| `voice-whisper-cuda` | CUDA backend for whisper-rs |
| `voice-whisper-vulkan` | Vulkan backend for whisper-rs |
| `voice-piper-rs` | In-process Piper via `sonata-synth` / `ort` |
| `voice-aec` | WebRTC APM echo cancellation (adds clang+cmake) |
| `voice-wake-oww` | openWakeWord ONNX wake-word detector |

With **no features enabled**, v2 still compiles and works via CLI fallback engines (`CliWhisperStt`, `CliPiperTts`).

---

## STT Backends (v2)

### `CliWhisperStt` (always available)
Wraps the v1 `SpeechToText` binary path. Buffers the entire utterance, writes a temp WAV, shells out to `whisper-cpp`. No partial transcripts. Used as the default fallback.

### `WhisperRsStt` (feature `voice-whisper-rs`)
In-process whisper.cpp via FFI. Intended for streaming with 2.5 s rolling window and 500 ms partial cadence. Currently scaffolded — requires `whisper-rs` in `Cargo.toml` to activate.

### `SidecarStt` (always available)
Proxies to the Python sidecar (`faster-whisper`). Not yet wired for streaming — falls back to `CliWhisperStt`.

### Hinglish Initial Prompt
All whisper backends receive a Hinglish-aware initial prompt:
> "User speaks Hinglish — a code-switch mix of Hindi and English in Latin script... Preserve Latin spellings of Hindi words. Do not transliterate to Devanagari."

---

## TTS Backends (v2)

### `CliPiperTts` (always available)
Wraps the v1 `TextToSpeech` CLI path. Synthesizes the whole sentence then pushes one big PCM chunk. Sample rate: 22,050 Hz.

### `PiperRsTts` (feature `voice-piper-rs`)
In-process Piper via `sonata-synth` over the existing `ort` ONNX runtime. Currently scaffolded.

---

## Wake Word Detection

When `voice-wake-oww` is enabled, the `openWakeWord` 3-model stack runs on the mic stream:

```text
16 kHz mono audio
    │
    ▼
melspectrogram.onnx    (audio → log-mel features, 32 bins)
    │
    ▼
embedding_model.onnx   (76 mel frames → 96-dim embedding)
    │
    ▼
hey_ria.onnx           (16 embeddings → keyword score 0..1)
    │
    ▼
score ≥ sensitivity → WakeWordEvent("hey ria", score, "oww")
```

Buffering invariants:
- **Audio buffer**: 1280 samples (= 80 ms @ 16 kHz) per mel step
- **Mel buffer**: 76 frames per embedding window, stride 8
- **Embedding buffer**: capped at 16; once full, each new embedding triggers keyword inference

Without the feature, `WakeWordDetector::disabled()` is a no-op passthrough.

---

## AEC (Acoustic Echo Cancellation)

Behind the `voice-aec` feature. Wraps `webrtc-audio-processing` (vendored C, BSD-3). When disabled or the feature is off, `AecProcessor::passthrough()` returns frames unchanged.

Settings mapped from `[voice.aec]`:
- `aggressiveness`: `"low"` | `"medium"` | `"high"`

---

## Post-Edit / Hinglish Fixer

`HinglishPostEditor` runs a tiny local LLM (default `qwen2.5-3b`) to clean obvious spelling/spacing errors in Hinglish transcripts. Triggered only when:
- Whisper confidence < 0.55, **or**
- Transcript contains Hinglish markers (`kya`, `hai`, `karo`, `mujhe`, ...), **or**
- `mode = "always"`

Behind an explicit timeout — if the LLM doesn't answer in time, the original transcript is used. TTFA budget is never sacrificed.

---

## Latency Targets (TTFA)

| Stage | Target |
|-------|--------|
| Wake word detection | < 100 ms |
| STT transcription | < 500 ms |
| Agent response (first token) | < 2 s |
| TTS synthesis (first sentence) | < 300 ms |
| **Total TTFA** | **S: 500 ms | A: 800 ms | C: 1200 ms** |

---

## VRAM Budget with Voice

| Component | VRAM |
|-----------|------|
| Whisper `ggml-large-v3-turbo-q5_0.bin` | ~1.6 GB |
| LLM (Qwen2.5-VL-7B) | 2.5–4.7 GB |
| CUDA overhead | 0.5 GB |
| **Total** | 4.5–6.5 GB |

On 6 GB VRAM GPUs, use CPU for LLM when voice is active, or switch to a smaller STT model.

---

## Model Download

Run the provided script to download tier-appropriate models:

```bash
# Download all models for detected tier
python scripts/download_models.py

# Or specify a tier explicitly
python scripts/download_models.py --tier lite      # small STT model
python scripts/download_models.py --tier standard   # medium STT model
python scripts/download_models.py --tier performance # large-v3-turbo STT model
```

Models are placed in `models/<subsystem>/` under the project root. The runtime resolves them from either `~/.kria/models/` (managed) or the workspace directory (dev fallback).

---

## Hallucination Prevention

Whisper can produce hallucinations like "(wind howling)" or "[BLANK_AUDIO]". Mitigations:

1. **VAD** — Only transcribe when speech is detected
2. **Hinglish initial prompt** — Guides transcription for code-switched Hindi/English
3. **Post-edit fixer** — Cleans obvious errors without sacrificing latency
4. **Temperature control** — Whisper uses deterministic settings

---

## Source Files

| Path | Purpose |
|------|---------|
| `crates/kria-core/src/voice/stt.rs` | v1 `SpeechToText` (whisper-cpp CLI wrapper) |
| `crates/kria-core/src/voice/tts.rs` | v1 `TextToSpeech` (piper CLI wrapper) |
| `crates/kria-core/src/voice/capture.rs` | CPAL-based audio capture (`AudioCapture`, `AudioChunk`) |
| `crates/kria-core/src/voice/playback.rs` | Rodio-based audio output (`AudioPlayer`) |
| `crates/kria-core/src/voice/vad.rs` | Silero VAD integration |
| `crates/kria-core/src/voice/tier.rs` | `VoiceTier`, `VoiceTierProfile`, tier-to-model mapping |
| `crates/kria-core/src/voice/v2/mod.rs` | v2 module root, `build_v2_with_cli_engines`, `CompiledFeatures` |
| `crates/kria-core/src/voice/v2/pipeline.rs` | `VoicePipelineV2`, `run_turn`, barge-in, state machine |
| `crates/kria-core/src/voice/v2/stt.rs` | `Stt` trait + `CliWhisperStt` / `SidecarStt` / `WhisperRsStt` |
| `crates/kria-core/src/voice/v2/tts.rs` | `Tts` trait + `CliPiperTts` / `PiperRsTts` |
| `crates/kria-core/src/voice/v2/wake.rs` | `WakeWordDetector` (openWakeWord ONNX stack) |
| `crates/kria-core/src/voice/v2/aec.rs` | `AecProcessor` (WebRTC APM wrapper) |
| `crates/kria-core/src/voice/v2/post_edit.rs` | `HinglishPostEditor` |
| `crates/kria-core/src/voice/v2/sentence.rs` | Sentence splitter for streaming playback |
| `crates/kria-core/src/voice/v2/playback.rs` | `PlaybackSink` with hard-abort barge-in support |
| `crates/kria-desktop/src/commands/voice.rs` | Tauri commands: `start_voice`, `stop_voice` |
| `crates/kria-desktop/src/commands/voice_runtime_helpers.rs` | `build_voice_pipeline`, `build_v2_pipeline`, `resolve_model_file`, `start_voice_v2_loop` |
| `crates/kria-desktop/src/commands/voice_diagnostics.rs` | `voice_v2_status` diagnostic command |
| `crates/kria-core/src/platform/paths.rs` | `KriaPaths` — model directory resolution (`KRIA_MODELS_DIR` env override) |
| `config/default.toml` | Default voice configuration |
| `scripts/download_models.py` | Model downloader |
