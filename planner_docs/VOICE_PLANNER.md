# KRIA Voice Pipeline — Architecture Analysis & Re-Architecture Plan

> **Created:** 2026-05-13
> **Author:** Principal Voice Systems Architect (Cascade)
> **Status:** Planning / Pre-Implementation
> **Target:** Siri/Gemini/ChatGPT Voice-class local-first assistant UX

---

## 1. Executive Summary

KRIA's voice pipeline has a solid architectural foundation with a well-designed v2 streaming skeleton, but **critical gaps prevent it from delivering assistant-grade voice UX**. The system currently behaves as a sophisticated push-to-talk transcription tool rather than a real-time conversational assistant.

**Key findings:**
- v1 is the default runtime; v2 is scaffolded but not operational end-to-end
- All STT backends shell out to CLI processes (cold-load per utterance)
- All TTS backends shell out to CLI processes (sentence-blocking)
- No true streaming STT — partials spawn redundant whisper-cpp subprocesses
- Barge-in architecture is correctly designed but untestable without real engines
- Wake word, AEC, piper-rs, whisper-rs all compile to stubs/bail
- Playback creates a new OutputStream per chunk (no persistent sink)
- Audio capture uses busy-polling with `try_recv` + 10ms sleep in v1
- VRAM budget (6 GB RTX 4050) makes concurrent whisper+LLM infeasible on GPU
- ~2-5 second perceived latency from speech-end to first audio out (estimated)

**Bottom line:** The concurrency skeleton (v2) is architecturally sound. The problem is that every engine node in the pipeline is either a stub or a CLI subprocess. Until in-process engines are wired, the system cannot achieve <500ms TTFA.

---

## 2. Current Pipeline Audit

### 2.1 v1 Pipeline (`VoicePipeline`)

| Aspect | Status | Issue |
|--------|--------|-------|
| Capture → VAD → STT → Agent → TTS → Playback | Working | Sequential, blocking |
| STT | `whisper-cpp` CLI subprocess | Cold-loads model per call |
| TTS | `piper` CLI subprocess | Synthesizes entire response, then plays |
| VAD | Silero ONNX (good) | Properly integrated |
| Partials | Disabled by default | Each partial spawns fresh whisper-cpp |
| Barge-in | None | Mic muted during TTS via atomic bool |
| Echo prevention | `mic_muted` atomic flag | 300ms unmute delay — no real AEC |
| State machine | 4 states (Idle/Listening/Processing/Speaking) | No BargeIn/Transcribing/Thinking |

### 2.2 v2 Pipeline (`VoicePipelineV2`)

| Aspect | Status | Issue |
|--------|--------|-------|
| Concurrency skeleton | Complete | Well-designed CancellationToken chain |
| FSM | 6 states | Correct for assistant UX |
| Barge-in | Architecturally complete | Tested with stubs; ≤50ms target |
| Sentence splitter | Working | Good abbreviation/Hinglish handling |
| STT engine | `CliWhisperStt` stub | Buffers entire utterance, shells out |
| TTS engine | `CliPiperTts` stub | Synthesizes whole sentence, one chunk |
| `WhisperRsStt` | Compiles, bails at runtime | "not yet wired in" |
| `PiperRsTts` | Compiles, bails at runtime | "not yet wired in" |
| `SidecarStt` | Compiles, bails at runtime | "streaming not yet implemented" |
| AEC | Passthrough stub | WebRTC APM not wired |
| Wake word | Compiles, loads models if present | Feature-gated, models not shipped |
| Post-edit | Decision logic works | LLM `correct()` call not wired in pipeline |
| Playback sink | Works with stubs | Real rodio integration fragile |

### 2.3 Tauri Integration

| Aspect | Status | Issue |
|--------|--------|-------|
| `start_voice` | Builds v1 always, optionally hot-swaps v2 | Dual-path complexity |
| `start_voice_v2_loop` | Continuous capture → run_turn loop | Drops chunks during Speaking (echo gate) |
| `voice_v2_speak` | Speak-only turn from text prompt | Works but LLM closure is ad-hoc |
| Telemetry pump | Maps v2 events → Tauri events | Working |

---

## 3. Runtime Architecture Analysis

### 3.1 Threading Model

```
v1:
  std::thread (capture loop) ──→ mpsc ──→ tokio task (VAD + STT)
  STT: tokio::process::Command (subprocess per call)
  TTS: tokio::process::Command (subprocess per call)
  Playback: spawn_blocking (rodio is sync)

v2:
  std::thread (capture) ──→ mpsc ──→ broadcast ──→ tokio tasks
  STT: tokio::spawn (wraps CLI subprocess internally)
  TTS: tokio::spawn (wraps CLI subprocess internally)
  Playback: tokio::spawn drain loop → spawn_blocking for rodio
```

**Issues:**
- v1 capture thread uses `try_recv` + 10ms sleep — wastes CPU and adds up to 10ms jitter
- No dedicated audio processing thread with real-time priority
- CLI subprocesses add 200-800ms cold-load latency per invocation
- `spawn_blocking` for playback adds task scheduler overhead
- No thread pinning or priority elevation for audio-critical paths

### 3.2 Channel Topology

```
v1: unbounded_channel (capture → VAD task) — unbounded = OOM risk under load
v2: broadcast(128) for capture fan-out — good
    mpsc(64) for STT PCM — good
    unbounded for partials/telemetry — acceptable (low volume)
    mpsc(4) for TTS PCM → playback — tight, may cause backpressure stalls
```

### 3.3 Cancellation Chain (v2)

```
CancellationToken (per-turn root)
  ├── capture_task clone → stops forwarding audio
  ├── partial_pump clone → stops forwarding partials
  ├── tts_task clone → stops sentence loop
  │     └── bridge → watch<bool> → Tts::synthesize_sentence abort_rx
  └── force_abort → playback.abort() + turn_cancel.cancel()
```

This is **architecturally correct** and well-designed. The ≤50ms barge-in target is achievable once real engines replace stubs.

---

## 4. Voice UX Gap Analysis

| Gap | Current Behavior | Target Behavior | Severity |
|-----|-----------------|-----------------|----------|
| Response start delay | 2-5s (STT cold-load + LLM TTFT + TTS cold-load) | <500ms (S-tier) | **Critical** |
| No streaming STT | Buffers entire utterance → single CLI call | Rolling-window partial every 500ms | **Critical** |
| No streaming TTS | Entire sentence synthesized → one big chunk | Incremental PCM chunks during synthesis | **High** |
| Sentence-serial playback | Synthesize sentence N, play, then synthesize N+1 | Overlap: synthesize N+1 while playing N | **High** |
| No continuous listening | Discrete turns with 300ms inter-turn silence | Always-on VAD with contextual gating | **High** |
| Push-to-talk default | Must click button or hotkey to start | Wake word + continuous mode as primary | **High** |
| Echo cancellation | Mic-mute flag with 300ms delay | Real AEC (WebRTC APM or DSP-based) | **Medium** |
| Turn-taking | Fixed VAD silence timeout (500ms default) | Adaptive silence detection based on context | **Medium** |
| No conversational continuity | Each turn is independent | Multi-turn context window for voice | **Low** |

---

## 5. Latency Bottleneck Analysis

### Current estimated latency breakdown (v1, single turn):

| Stage | Estimated Latency | Bottleneck |
|-------|------------------|------------|
| VAD speech-end detection | 500-1000ms | `vad_silence_ms` default = 500ms |
| Audio → temp WAV write | 5-20ms | Disk I/O |
| whisper-cpp cold-load | 800-2000ms | Model load per subprocess |
| whisper-cpp inference | 500-3000ms | Depends on utterance length |
| Agent LLM TTFT | 500-2000ms | Model inference |
| Agent LLM full response | 2000-8000ms | Token generation |
| piper CLI cold-load | 200-500ms | Model load per subprocess |
| piper synthesis | 200-1000ms | Entire response at once |
| Playback open stream | 50-200ms | `OutputStream::try_default()` per chunk |
| Playback drain | Duration of audio | Blocking `sink.sleep_until_end()` |
| **Total (speech-end → first audio)** | **~2500-8000ms** | **Unacceptable** |

### Target latency breakdown (v2, optimized):

| Stage | Target | How |
|-------|--------|-----|
| VAD speech-end | 300-500ms | Tuned silence timeout |
| STT final transcript | 200-400ms | In-process whisper-rs, model pre-loaded |
| Post-edit (if needed) | 0-250ms | Concurrent with first LLM tokens |
| LLM first token | 100-300ms | Streaming, model hot |
| Sentence split + TTS first sentence | 100-200ms | In-process piper-rs, model pre-loaded |
| Playback start | 10-30ms | Persistent rodio sink |
| **Total TTFA** | **~400-900ms** | **Assistant-grade** |

---

## 6. Real-Time Streaming Analysis

### STT Streaming (Current: Non-existent)

The `Stt` trait defines `start_stream(pcm_rx, partial_tx)` — correct interface. But:
- `CliWhisperStt`: buffers ALL audio, writes WAV, shells out at end. Zero streaming.
- `WhisperRsStt`: bail!("not yet wired in")
- `SidecarStt`: bail!("streaming not yet implemented")
- v1 partials: disabled by default because each partial spawns a fresh subprocess

**Fix:** Wire `whisper-rs` with a rolling 2.5s window, emit partials every 500ms, run final pass at speech-end.

### TTS Streaming (Current: Sentence-blocking)

The `Tts` trait defines `synthesize_sentence(sentence, pcm_tx, abort_rx)`. But:
- `CliPiperTts`: spawns piper subprocess, waits for completion, sends ONE chunk
- `PiperRsTts`: bail!("not yet wired in")
- No sub-sentence chunking — entire sentence must finish before playback starts

**Fix:** Wire `sonata-synth` (piper-rs) with incremental phoneme→PCM emission (~120ms chunks).

### LLM Streaming (Current: Working)

The `chat_stream` path in `start_voice_v2_loop` correctly streams tokens into the sentence splitter. This is the one streaming stage that works.

---

## 7. STT Weaknesses

1. **CLI subprocess per utterance** — cold-loads 1.6GB model every time
2. **No model persistence** — whisper context is not kept in memory
3. **Warmup is a hack** — transcribes 1s of silence at startup, still cold on real calls
4. **Confidence is heuristic** — `estimate_confidence()` uses token count + latency, not whisper logprobs
5. **Hinglish support is prompt-only** — no fine-tuned model, no language-specific beam search
6. **GPU contention** — whisper-cpp CUDA + LLM CUDA on 6GB VRAM = OOM risk
7. **No streaming** — entire utterance buffered before processing
8. **Temp file overhead** — f32 PCM → i16 WAV → disk → whisper-cpp → parse stdout
9. **90s timeout** — far too long for interactive voice; should be 10-15s max

---

## 8. TTS Weaknesses

1. **CLI subprocess per sentence** — cold-loads piper model every time
2. **Single output file** — writes to `/tmp/kria_tts_output.wav` (race condition if concurrent)
3. **No streaming synthesis** — entire sentence → WAV → read → parse → PCM → play
4. **Playback creates new OutputStream per chunk** — `open_output_stream()` called every time
5. **Markdown normalization** — rebuilds regex objects on every call (should be `Lazy<Regex>`)
6. **No voice caching** — common phrases ("Sure!", "Done.") re-synthesized every time
7. **Fixed prosody** — no SSML, no emphasis, no speed adaptation for conversational context
8. **22050 Hz only** — no sample-rate negotiation with output device

---

## 9. VAD / Wake Word Weaknesses

### VAD
- Silero VAD integration is **good** — proper ONNX inference, LSTM state management
- `min_speech_chunks = 3` (300ms) — reasonable, but not adaptive
- Silence timeout is configurable but static — no context-aware endpointing
- `max_prob` across windows — may cause false positives on short noise spikes
- Mutex-locked ONNX session — blocks the capture callback if inference is slow

### Wake Word
- openWakeWord 3-model stack is **well-designed**
- Feature-gated, models not shipped by default — effectively disabled
- 500ms cooldown after fire — may miss rapid re-invocations
- No false-positive rejection beyond raw score threshold
- No integration with v2 `run_turn` loop — `force_wake("auto")` is called unconditionally

---

## 10. Interruption / Barge-In Analysis

### v2 Barge-In (Architecturally Sound)

The `CancellationToken` chain is correctly designed:
1. VAD `SpeechStart` while `Speaking` → `turn_cancel.cancel()`
2. TTS task observes via bridge → `abort_rx.changed()` → stops synthesis
3. Playback drain task observes → `rx.close()` → drops queued audio
4. 250ms grace period, then hard abort

**But:**
- Not testable end-to-end without real engines (stubs complete instantly)
- Echo gate (dropping chunks during Speaking) means VAD never sees speech during playback — barge-in can only fire if echo leaks through
- No AEC means barge-in would fire on KRIA's own voice without the echo gate
- The echo gate and barge-in are **mutually exclusive** — this is a fundamental design conflict

**Resolution:** AEC must be operational before barge-in can work in practice. Without AEC, the echo gate must stay, and barge-in only works via push-to-talk or external trigger.

### v1 Barge-In (None)

v1 has zero barge-in capability. `mic_muted` is set during the entire TTS playback.

---

## 11. Audio Runtime Orchestration Review

### Capture
- CPAL integration is solid — proper device resolution, fallback, pause/resume
- 100ms chunk size is appropriate for VAD (matches Silero's 512-sample window at 16kHz)
- Noise suppression is minimal (single-pole HPF + soft gate) — adequate for quiet rooms
- Device-change detection via polling `should_restart_for_default_change` — works but inelegant

### Playback
- **Critical flaw:** `AudioPlayer::play_samples()` opens a new `OutputStream` + `Sink` every call
- `sink.sleep_until_end()` blocks the spawn_blocking thread until audio finishes
- No persistent output stream — adds 50-200ms latency per chunk
- No volume control, fade-in/out, or ducking
- Speaker device resolution mirrors mic resolution (good)

### Orchestration
- v1: capture thread → async VAD task → STT call → agent call → TTS call → playback call
  - Entirely sequential, no overlap
- v2: capture → broadcast → concurrent STT + VAD → sentence-split → TTS → playback
  - Correct architecture, but all nodes are stubs

---

## 12. Threading / Concurrency Risks

| Risk | Location | Severity |
|------|----------|----------|
| `try_recv` + 10ms sleep busy-loop | v1 capture thread | Medium — wastes CPU |
| `StdMutex` in CPAL callback | capture.rs buffer + noise_suppression | Low — short critical sections |
| `StdMutex` on Silero ONNX session | vad.rs | Medium — ONNX inference blocks callback |
| `tokio::Mutex` on `PlaybackSink` | v2 pipeline | Low — short holds |
| `tokio::Mutex` on `AecProcessor` | v2 pipeline | Low — passthrough is instant |
| `Arc<Mutex<Option<MetricsBuilder>>>` | v2 pipeline | Low — metadata only |
| Unbounded channel for capture chunks | v1 pipeline | Medium — OOM under sustained speech |
| broadcast(128) channel | v2 pipeline | Low — lagged frames are skipped |
| No real-time thread priority | All audio threads | Medium — scheduler may preempt |

---

## 13. Device Management Issues

1. **No hot-plug detection** — device changes detected by polling, not OS events
2. **No audio device health monitoring** — `has_failed()` only catches callback errors
3. **Speaker selection per-chunk** — v1 resolves output device on every `play_samples` call
4. **No fallback chain** — if preferred device fails, falls back to default once; no retry
5. **No Bluetooth/USB disconnect handling** — capture thread will restart but may lose audio
6. **No sample rate negotiation** — hardcoded 16kHz capture, 22050Hz playback

---

## 14. Real-Time Conversational Flow Issues

1. **No overlap between listening and speaking** — echo gate blocks all capture during TTS
2. **300ms inter-turn silence** — hardcoded, adds noticeable delay between turns
3. **No "thinking" audio feedback** — silence between transcript and first audio
4. **No conversational filler** — no "hmm", "let me check" while LLM generates
5. **No turn-continuation** — user can't add "and also..." before KRIA responds
6. **No multi-turn voice context** — each turn rebuilds full message history
7. **VAD endpointing is context-blind** — same silence timeout for questions vs. dictation
8. **No prosody-aware TTS** — responses sound robotic regardless of content

---

## 15. Re-Architecture: Target Data Flow

### 15.1 Target Pipeline (Assistant-Grade)

```text
                    ┌────────────────────────────────────┐
                    │ AudioCapture (CPAL, 16kHz, RT prio)│
                    └───────┬────────────────────────────┘
                            │ f32 PCM (10ms frames)
                            ▼
                    ┌───────────────┐    ┌──────────────┐
                    │ AEC (WebRTC)  │◄───│ Render ref   │
                    └───────┬───────┘    └──────┬───────┘
                            │                    │
                    ┌───────┴───────┐            │
                    │ Fan-out       │            │
                    ├───────┬───────┤            │
                    ▼       ▼       ▼            │
               ┌────────┐ ┌───┐ ┌──────┐        │
               │Wake Det│ │VAD│ │STT   │        │
               │(always)│ │   │ │(hot) │        │
               └────┬───┘ └─┬─┘ └──┬───┘        │
                    │       │      │             │
                    │   ┌───┴──────┴──┐          │
                    │   │ Turn Arbiter│          │
                    │   └──────┬──────┘          │
                    │          │ FinalTranscript  │
                    │          ▼                  │
                    │   ┌────────────┐           │
                    │   │ Post-Edit  │           │
                    │   │ (optional) │           │
                    │   └──────┬─────┘           │
                    │          │                  │
                    │          ▼                  │
                    │   ┌────────────┐           │
                    │   │ LLM Stream │           │
                    │   └──────┬─────┘           │
                    │          │ tokens           │
                    │          ▼                  │
                    │   ┌─────────────┐          │
                    │   │ Sentence    │          │
                    │   │ Splitter    │          │
                    │   └──────┬──────┘          │
                    │          │ sentences        │
                    │          ▼                  │
                    │   ┌─────────────┐          │
                    │   │ TTS (hot)   │──PCM───►│
                    │   └──────┬──────┘          │
                    │          │ PCM chunks       │
                    │          ▼                  │
                    │   ┌─────────────┐          │
                    │   │ Playback    │──render──┘
                    │   │ (persistent)│
                    │   └─────────────┘
                    │
                    └── Barge-in: VAD(SpeechStart) while Speaking
                         → CancellationToken.cancel()
                         → TTS stops, Playback flushes, LLM dropped
```

### 15.2 Key Architecture Changes

| Change | Why | Complexity |
|--------|-----|------------|
| Persistent STT context (whisper-rs) | Eliminate 800-2000ms cold-load | **High** |
| Persistent TTS context (piper-rs/sonata) | Eliminate 200-500ms cold-load | **High** |
| Persistent playback sink (rodio) | Eliminate 50-200ms stream open | **Medium** |
| AEC integration | Enable simultaneous capture + playback | **High** |
| Streaming STT with rolling window | Enable real-time partials | **High** |
| TTS sub-sentence chunking | Reduce first-chunk latency | **Medium** |
| Sentence N+1 prefetch | Overlap TTS synthesis with playback | **Medium** |
| Wake word always-on | Enable hands-free activation | **Low** (already scaffolded) |
| Adaptive VAD endpointing | Context-aware turn boundaries | **Medium** |

---

## 16. Streaming STT Strategy

### 16.1 Rolling Window Architecture

```
Audio stream: ──────────────────────────────────────►
                 │ 2.5s window │
                      │ 2.5s window │  (stride 500ms)
                           │ 2.5s window │
                                │ ... │

Each window → whisper-rs inference → partial transcript
Final window (at VAD speech-end) → final transcript
```

### 16.2 Implementation Plan

1. Add `whisper-rs` as optional dependency in `kria-core/Cargo.toml`
2. Initialize `WhisperContext` once at pipeline start, keep in `Arc`
3. Use `WhisperState::new()` per inference pass (lightweight — shares model weights)
4. Ring buffer of last 2.5s of audio (40,000 samples at 16kHz)
5. Every 500ms, copy ring buffer → run inference on blocking thread pool
6. Emit partial transcript via `partial_tx`
7. At VAD speech-end, run final pass on complete utterance with `beam_size=5`
8. Confidence from whisper logprobs (available via whisper-rs API)

### 16.3 GPU Strategy

- **Default (6GB VRAM):** STT on CPU (4 threads), LLM on GPU
- **8GB+ VRAM:** STT on GPU, LLM on GPU, time-multiplexed via `GpuLeaseManager`
- **No GPU:** Both on CPU, expect 1200ms+ TTFA (C-tier budget)

---

## 17. Streaming TTS Strategy

### 17.1 Sub-Sentence Chunking

Current `CliPiperTts` waits for entire sentence → one big PCM vector.

Target: Piper-rs emits PCM in ~120ms chunks as phonemes are synthesized:
```
"Hello, how can I help you today?"
  → chunk 1: "Hello," PCM (120ms)     → to playback immediately
  → chunk 2: "how can I" PCM (120ms)  → to playback
  → chunk 3: "help you" PCM (120ms)   → to playback
  → chunk 4: "today?" PCM (120ms)     → to playback
```

### 17.2 Sentence Prefetch Pipeline

While sentence N is playing, synthesize sentence N+1:
```
Time:     0ms     200ms    400ms    600ms    800ms    1000ms
Synth:   [sent1 ████████]
Play:            [sent1 ██████████████████████]
Synth:                   [sent2 ████████]
Play:                                         [sent2 ██████████]
```

This is already partially supported by the v2 `run_turn` loop's `for sentence in ...` design — the sentence splitter emits sentences as they complete, and each is synthesized immediately. The missing piece is **intra-sentence** streaming.

---

## 18. Playback Sink Re-Architecture

### 18.1 Current Issues

```rust
// v1: AudioPlayer::play_samples — creates new stream EVERY call
let (_stream, handle) = rodio::OutputStream::try_default()?;
let sink = rodio::Sink::try_new(&handle)?;
sink.append(source);
sink.sleep_until_end(); // BLOCKS
```

### 18.2 Target: Persistent Sink

```rust
// Target: One OutputStream for entire session
struct PersistentPlayback {
    _stream: OutputStream,          // kept alive for session lifetime
    sink: Sink,                      // reusable
    aec_ref_tx: UnboundedSender<Vec<f32>>,  // AEC render reference
}

impl PersistentPlayback {
    fn push_chunk(&self, pcm: Vec<f32>) { ... }  // non-blocking
    fn flush(&self) { ... }                        // drain queue
    fn abort(&self) { ... }                        // clear + stop
}
```

The v2 `PlaybackSink` already has this architecture sketched — it creates one sink via `begin_session` and pushes chunks. The issue is that `begin_session` creates a **new** `OutputStream` each time. Fix: hoist the `OutputStream` to session lifetime.

---

## 19. AEC Integration Plan

### 19.1 Why AEC is Critical

Without AEC, the system must choose between:
- **Echo gate** (current): Mute mic during playback → no barge-in
- **No gate**: Barge-in works but fires on KRIA's own voice → chaos

AEC enables both: the mic stays live during playback, but KRIA's voice is subtracted from the capture signal, so only the user's voice triggers VAD.

### 19.2 Integration Points

1. `AudioCapture` produces raw frames → `AecProcessor::process_capture_frame()`
2. `PlaybackSink` sends render reference → `AecProcessor::push_render_frame()`
3. AEC output → VAD + STT (clean signal)
4. The v2 pipeline already has `aec_ref_tx` wiring in `PlaybackSink`

### 19.3 Configuration

```toml
[voice.aec]
enabled = true
aggressiveness = "medium"  # "low" | "medium" | "high"
```

The `AecProcessor` struct and `voice-aec` feature gate already exist. What's needed:
- Wire `webrtc-audio-processing` crate in `Cargo.toml`
- Ensure capture and playback use matching sample rates (16kHz)
- Resample playback reference from 22050Hz → 16kHz before feeding AEC

---

## 20. Implementation Roadmap

### Phase 1: Make v2 Operational (2-3 weeks)

**Goal:** v2 pipeline works end-to-end with CLI engines at v1 parity.

| Task | Files | Priority |
|------|-------|----------|
| Fix v2 `run_turn` to work with `CliWhisperStt` + `CliPiperTts` | `pipeline.rs` | **P0** |
| Make `start_voice_v2_loop` the default when `voice.engine = "v2"` | `voice.rs`, `voice_runtime_helpers.rs` | **P0** |
| Persistent `OutputStream` in `PlaybackSink` | `v2/playback.rs` | **P0** |
| Integration test: text prompt → LLM → TTS → playback | new test file | **P0** |
| Wire post-edit into `run_turn` Transcribing state | `pipeline.rs`, `post_edit.rs` | **P1** |
| Add v2 telemetry to Tauri devtools panel | `voice_diagnostics.rs` | **P1** |

### Phase 2: In-Process Engines (3-4 weeks)

**Goal:** Eliminate CLI subprocess overhead, enable streaming.

| Task | Files | Priority |
|------|-------|----------|
| Wire `whisper-rs` crate in `Cargo.toml` | `kria-core/Cargo.toml` | **P0** |
| Implement `WhisperRsStt::start_stream` with rolling window | `v2/stt.rs` | **P0** |
| Wire `sonata-synth` / `ort` for Piper in-process | `kria-core/Cargo.toml` | **P0** |
| Implement `PiperRsTts::synthesize_sentence` with chunked emission | `v2/tts.rs` | **P0** |
| Sentence prefetch: synthesize N+1 while playing N | `pipeline.rs` | **P1** |
| VRAM budget enforcement: STT CPU / LLM GPU split | `tier.rs`, `stt.rs` | **P1** |

### Phase 3: Real-Time UX (2-3 weeks)

**Goal:** Barge-in, AEC, wake word, adaptive endpointing.

| Task | Files | Priority |
|------|-------|----------|
| Wire WebRTC APM in `voice-aec` feature | `aec.rs`, `Cargo.toml` | **P0** |
| Remove echo gate, enable full-duplex | `voice_runtime_helpers.rs` | **P0** |
| End-to-end barge-in test | new test file | **P0** |
| Ship wake word models, enable by default | `download_models.py`, `default.toml` | **P1** |
| Adaptive VAD endpointing (question-aware) | `vad.rs` | **P1** |
| Thinking audio feedback ("hmm"/"let me check") | `pipeline.rs`, `tts.rs` | **P2** |
| Conversational filler during LLM generation | `pipeline.rs` | **P2** |

### Phase 4: Polish (2 weeks)

**Goal:** Production-ready voice UX.

| Task | Files | Priority |
|------|-------|----------|
| TTFA telemetry dashboard in UI | frontend | **P1** |
| Voice phrase caching (common TTS outputs) | `tts.rs` | **P1** |
| Audio device hot-plug detection | `capture.rs`, `playback.rs` | **P1** |
| Real-time thread priority for audio paths | `capture.rs` | **P2** |
| Bluetooth/USB disconnect recovery | `capture.rs`, `playback.rs` | **P2** |
| Multi-language voice selection | `tts.rs`, config | **P2** |

---

## 21. Risk Register

| Risk | Impact | Mitigation |
|------|--------|------------|
| `whisper-rs` CUDA segfault on Linux | Blocks Phase 2 | Test on CI; CPU fallback always available |
| `sonata-synth` / `ort` version conflict | Blocks Phase 2 | Pin ort version shared with VAD/wake |
| WebRTC APM build complexity (clang+cmake) | Blocks Phase 3 | Feature-gate; passthrough fallback |
| 6GB VRAM OOM with concurrent whisper+LLM | Degrades UX | GPU lease manager + CPU fallback |
| Rodio API changes (0.20 → future) | Minor | Pin version in Cargo.toml |
| CPAL device enumeration failures | Degrades UX | Graceful fallback + user notification |
| Silero VAD false positives on keyboard/fan | Bad UX | Combine with wake word for continuous mode |

---

## 22. Source File Index

| File | Purpose | Lines | Status |
|------|---------|-------|--------|
| `voice/v2/pipeline.rs` | v2 orchestrator (FSM, run_turn, barge-in) | 784 | Scaffold — needs real engines |
| `voice/v2/stt.rs` | Stt trait + CliWhisperStt/SidecarStt/WhisperRsStt | 285 | CliWhisper works; others bail |
| `voice/v2/tts.rs` | Tts trait + CliPiperTts/PiperRsTts | ~250 | CliPiper works; PiperRs bails |
| `voice/v2/playback.rs` | PlaybackSink with barge-in support | 185 | Works with stubs |
| `voice/v2/sentence.rs` | SentenceSplitter for streaming LLM | 204 | Working |
| `voice/v2/aec.rs` | AecProcessor (WebRTC APM wrapper) | 144 | Passthrough only |
| `voice/v2/post_edit.rs` | HinglishPostEditor | 253 | Decision logic works; not wired in pipeline |
| `voice/v2/wake.rs` | WakeWordDetector (openWakeWord ONNX) | ~300 | Feature-gated, compiles |
| `voice/v2/mod.rs` | v2 module root, build_v2_with_cli_engines | ~100 | Working |
| `voice/stt.rs` | v1 SpeechToText (whisper-cpp CLI) | 307 | Working (used by CliWhisperStt) |
| `voice/tts.rs` | v1 TextToSpeech (piper CLI) | 324 | Working (used by CliPiperTts) |
| `voice/capture.rs` | AudioCapture (CPAL) | 367 | Working |
| `voice/playback.rs` | AudioPlayer (rodio) | 134 | Working but creates new stream per call |
| `voice/vad.rs` | VoiceActivityDetector (Silero ONNX) | 333 | Working |
| `voice/tier.rs` | VoiceTier profiles | 248 | Working |
| `voice/metrics.rs` | Turn telemetry + TTFA tracking | 207 | Working |
| `voice/mod.rs` | Module declarations + re-exports | 19 | Working |
| `commands/voice.rs` | Tauri commands (start/stop/speak) | 750 | Working |
| `commands/voice_runtime_helpers.rs` | Pipeline builders + v2 loop | 547 | Working |
| `commands/voice_diagnostics.rs` | v2 status diagnostic | ~100 | Working |

---

## 23. Key Constants & Thresholds

| Constant | Value | Location |
|----------|-------|----------|
| Capture sample rate | 16,000 Hz | `capture.rs` |
| TTS sample rate | 22,050 Hz | `tts.rs` |
| VAD chunk size | 512 samples (32ms) | `vad.rs` |
| Capture chunk size | ~1600 samples (100ms) | `capture.rs` |
| Sentence terminators | `.!?;` (not `:`) | `sentence.rs` |
| Min sentence length | 12 chars | `sentence.rs` |
| Confidence threshold (post-edit) | 0.55 | `post_edit.rs` |
| Barge-in grace period | 250ms | `pipeline.rs` |
| Inter-turn silence | 300ms | `voice_runtime_helpers.rs` |
| GPU lease timeout | 120s idle, 15s acquire | `voice_runtime_helpers.rs` |
| Whisper warmup timeout | 120s | `voice_runtime_helpers.rs` |
| STT command timeout | 45s | `voice_runtime_helpers.rs` |
| TTFA budget (S/A/C) | 500/800/1200ms | `tier.rs` |
| Post-edit timeout (S/A/C) | 3000/5000/0ms | `tier.rs` |
| Broadcast channel capacity | 128 chunks | `voice_runtime_helpers.rs` |
| LLM token channel capacity | 64 tokens | `voice.rs` / `voice_runtime_helpers.rs` |
| TTS PCM channel capacity | 4 chunks | `playback.rs` |

---

*End of Voice Pipeline Architecture Analysis & Re-Architecture Plan*
