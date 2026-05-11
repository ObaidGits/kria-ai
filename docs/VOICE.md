# KRIA Voice Pipeline

> **Last Updated:** 2026-05-11
> **Status:** Production

---

## Overview

KRIA supports voice-first interaction with speech-to-text (STT), text-to-speech (TTS), wake word detection, and voice activity detection (VAD). The voice pipeline is designed for sub-500ms latency on simple commands.

---

## Components

| Component | Backend | Purpose |
|-----------|---------|---------|
| **STT** | whisper.cpp / whisper-rs | Speech to text |
| **TTS** | Piper / in-process | Text to speech |
| **Wake Word** | Custom detector | Hands-free activation |
| **VAD** | webrtc-audio-processing | Voice activity detection |

---

## Audio Pipeline

```
Microphone → VAD → STT → AgentLoop → TTS → Speakers
                ↓
           Wake Word Detector
```

---

## Configuration

```toml
[voice]
stt_model = "whisper-small"
tts_voice = "en_US-lessac-medium"
sample_rate = 16000
vad_threshold = 0.5
wake_word = "kria"
wake_word_sensitivity = 0.8
```

---

## Latency Targets

| Stage | Target |
|-------|--------|
| Wake word detection | < 100ms |
| STT transcription | < 500ms |
| Agent response | < 2s |
| TTS synthesis | < 300ms |

---

## STT Model Selection

| Model | Params | VRAM | WER (Clean) | Latency (GPU) |
|-------|--------|------|-------------|---------------|
| small.en | 244M | ~0.5 GB | 7.7% | ~0.15s |
| medium.en | 769M | ~1.5 GB | 5.8% | ~0.3s |
| large-v3-turbo | 809M | ~1.6 GB | 5.2% | ~0.4s |
| distil-large-v3 | 756M | ~1.8 GB | 5.7% | ~0.12s |

**Recommended:** `medium.en` on GPU for best accuracy/speed balance.

---

## Audio Preprocessing

### High-Pass Filter

Remove low-frequency noise (fan rumble, 50-300 Hz):

```python
import scipy.signal as signal
b, a = signal.butter(4, 300, btype='high', fs=16000)
audio = signal.lfilter(b, a, audio).astype(np.int16)
```

### Automatic Gain Control (AGC)

Normalize audio to consistent level for Whisper:

```python
def agc(audio, target_db=-20):
    peak = np.max(np.abs(audio.astype(np.float32)))
    if peak < 100:
        return audio
    target_peak = 32768 * (10 ** (target_db / 20))
    gain = target_peak / peak
    return (audio.astype(np.float32) * gain).clip(-32768, 32767).astype(np.int16)
```

---

## VRAM Budget with Voice

| Component | VRAM |
|-----------|------|
| Whisper medium.en | 1.5 GB |
| LLM (Qwen2.5-VL-7B) | 2.5-4.7 GB |
| CUDA overhead | 0.5 GB |
| **Total** | 4.5-6.5 GB |

On 6GB VRAM GPUs, use CPU for LLM when voice is active, or use smaller STT model.

---

## Alternative STT: faster-whisper

[faster-whisper](https://github.com/SYSTRAN/faster-whisper) uses CTranslate2 for 2-4x faster inference:

```python
from faster_whisper import WhisperModel

model = WhisperModel("medium.en", device="cuda", compute_type="float16")

segments, info = model.transcribe(
    audio,
    language="en",
    beam_size=5,
    vad_filter=True,
)
```

Advantages:
- Built-in Silero VAD
- Better beam search
- `float16` compute for 2x speedup

---

## Alternative STT: NVIDIA Parakeet

[Parakeet TDT](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2) offers state-of-the-art accuracy:

| Feature | Value |
|---------|-------|
| Params | 600M |
| WER (LibriSpeech) | 2.9% |
| VRAM | ~1.5 GB |
| Streaming | Yes |

**Key advantage:** Transducer models don't hallucinate like encoder-decoder models.

---

## Hallucination Prevention

Whisper can produce hallucinations like "(wind howling)" or "[BLANK_AUDIO]". Mitigations:

1. **Use VAD** — Only transcribe when speech detected
2. **High-pass filter** — Remove low-frequency noise
3. **Initial prompt** — Provide context to guide transcription
4. **Temperature control** — Use `temperature=0` for deterministic output
5. **Switch to Parakeet** — Transducer models don't hallucinate

---

## Streaming ASR

For real-time sub-200ms latency:

```python
# WebSocket-based streaming
@app.websocket("/ws/transcribe")
async def ws_transcribe(ws: WebSocket):
    await ws.accept()
    while True:
        audio_chunk = await ws.receive_bytes()
        text = model.transcribe_stream(audio_chunk)
        if text:
            await ws.send_json({"text": text, "is_final": False})
```

---

## Source Files

- `crates/kria-core/src/voice/` — Rust voice modules
- `kria-modules/` — Python audio preprocessing
- `config/default.toml` — Voice configuration
