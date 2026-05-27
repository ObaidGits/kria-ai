# KRIA Voice Overview

## 1. Purpose

The voice subsystem provides bounded real-time speech interaction for KRIA turns. It handles audio capture, ASR, turn framing, transcript authority, and speech output while preserving central orchestration control.

Responsibilities:
- Convert live audio to structured transcript signals.
- Bridge voice events into orchestrated turn lifecycle.
- Manage TTS output timing relative to runtime state.
- Preserve transcript authority and interruption semantics.

Non-goals:
- Voice runtime does not replace orchestrator policy/authority.
- Voice pipeline does not directly execute dangerous side effects.

## 2. Architecture Overview

Primary implementation:
- `crates/kria-core/src/voice/mod.rs`
- `crates/kria-core/src/voice/v2/mod.rs`
- `crates/kria-core/src/voice/runtime_bridge.rs`
- `crates/kria-core/src/voice/transcript_authority.rs`

Architecture:
1. Audio ingress and segmentation.
2. ASR/transcript normalization and authority handling.
3. Bridge to turn orchestration (start/update/finalize/cancel).
4. TTS egress controlled by runtime state and interruption rules.

## 3. Runtime Execution Flow

1. Session starts with voice runtime initialized and bridge connected.
2. Incoming speech produces partial/final transcript events.
3. Transcript authority resolves accepted text for orchestration input.
4. Orchestrator processes turn; tool/provider routes execute normally.
5. Response is rendered through TTS, with interrupt/cancel handling.
6. Session continues with bounded memory and explicit lifecycle state.

Authority boundaries:
- Voice provides interaction substrate and state signals.
- Orchestrator owns execution decisions and side effects.

## 4. Core Components

| Component | Location | Contract |
|---|---|---|
| Voice runtime | `voice/mod.rs`, `voice/v2/mod.rs` | Session lifecycle and audio pipeline |
| Runtime bridge | `voice/runtime_bridge.rs` | Connects voice events to orchestrator turns |
| Transcript authority | `voice/transcript_authority.rs` | Canonical transcript acceptance and conflict handling |
| Interruption controls | `voice/*` | User/barge-in and cancel semantics for live sessions |

Invariants:
- A turn uses one authoritative finalized transcript input.
- Interruption events must propagate deterministically.
- Voice session state transitions are explicit and bounded.

## 5. Integration Contracts

| Integration | Contract |
|---|---|
| Orchestration | Voice submits turn input; orchestration decides actions |
| Providers | ASR/TTS/model providers are backend services under orchestration policy |
| Tools | Voice can trigger tool-eligible turns but cannot bypass tool gates |
| Memory | Transcript/output artifacts flow through memory governance |
| OpenClaw/n8n/MCP | Voice does not grant substrate authority; only influences turn intent |
| Hardware | Audio device and compute constraints shape QoS and fallback behavior |
| Safety | Risk policies apply identically to voice-originated turns |
| GUI/Browser | Voice-triggered automation still uses standard tool/safety paths |

## 6. Failure Handling & Recovery

- ASR degradation: fallback to partial/noisy transcript handling with clarification prompts.
- Device/input failure: transition session state and surface recoverable error.
- TTS failure: degrade to text response path.
- Interruption storms: coalesce/cancel to preserve deterministic turn state.

Recovery:
- Maintain session continuity when possible.
- Prefer explicit recovery transitions over hidden resets.

## 7. Performance & Constraints

Constraints:
- End-to-end latency depends on ASR, provider response, and TTS phases.
- Real-time interaction requires low jitter in bridge/event handling.
- Device limitations (mic quality, sample rate, CPU) materially affect quality.

Tradeoffs:
- Lower latency settings can reduce ASR accuracy.
- More robust filtering/post-processing can increase delay.

## 8. Security & Safety

Trust boundaries:
- Microphone/audio sources are untrusted input.
- Transcript text is treated as user input and policy-governed.

Controls:
- Voice-originated instructions still pass policy/HITL for dangerous actions.
- Transcript authority limits injection via unstable partial fragments.
- Session controls enforce explicit start/stop/cancel semantics.

## 9. Observability

Capture:
- ASR latency, transcript confidence/error rates.
- Turn handoff timings (voice->orchestration->voice).
- Interrupt/cancel counts and causes.
- TTS latency/failure distributions.

Evaluation:
- Validate real-world voice behavior through dedicated runbooks in `docs/evaluations/`.

## 10. Runtime Invariants

Core invariants:
1. Voice runtime is bounded, deterministic, and cancellation-correct.
2. Transcript authority is explicit: partials are provisional, final transcript is authoritative.
3. Barge-in cancellation must propagate through STT/LLM/TTS/playback as one chain.
4. UX polish logic may change timing only, never semantic content.
5. Voice execution cannot bypass platform safety, policy, or tool governance.

Resource and stability invariants:
- Runtime behavior must honor hardware tier and lease constraints.
- Partial update cadence must remain bounded to avoid UI/event spam.
- Long-session health requires queue-depth and latency observability.
- Degraded mode behavior must be visible and deterministic.

Engine invariants:
- Native in-process engines are preferred when compiled and available.
- CLI fallback must remain functional and explicitly observable.
- Engine selection must be deterministic: config, then tier, then fallback chain.
- Missing runtime dependencies must fail clearly, not silently.

Validation invariants:
- Real-world assistant validation is mandatory for production claims.
- Regression criteria include interruption latency, transcript stability, and trust-critical failure modes.
- Voice changes must preserve this architecture contract.

## 11. Configuration and Operations

Configuration priority order:
1. Explicit user config values.
2. Hardware-tier derived defaults.
3. Runtime fallback chain.

Key controls:
- `voice.mode`, `voice.stt_engine`, `voice.tts_engine`
- VAD controls: `vad_silence_ms`, thresholds
- Partial cadence controls
- Device-follow behavior for mic/speaker changes

Production operations:
- Prefer native voice backends when available for latency and stability.
- Keep fallback CLI engines available for compatibility.
- Monitor interruption latency, partial flicker, and queue buildup.
- Keep degraded mode explicit in user-visible events and logs.

Common failure patterns:

| Pattern | Symptom | Operational Response |
|---|---|---|
| Engine cold start overhead | High first-response latency | Warm startup path, persistent context where supported |
| Device churn | Lost mic/speaker stream | Enable default-device follow and recovery handlers |
| Over-aggressive partial cadence | UI instability/flicker | Enforce bounded cadence and coalescing |
| Barge-in inconsistency | Speech overlap or delayed stop | Validate cancellation propagation chain end-to-end |

Validation runbook:
- Use `docs/evaluations/voice-validation.md` as the acceptance runbook for runtime quality and trust behavior before production promotion.

## 12. Future Evolution

1. Improve transcript stability and confidence-aware turn gating.
2. Add stronger QoS adaptation under constrained hardware.
3. Expand deterministic interruption policies for long responses.
4. Keep voice as bounded interaction layer under orchestration authority.
