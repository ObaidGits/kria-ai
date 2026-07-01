# Implementation Plan — Voice System v3

## Overview

Implementation is organized into sequential, independently shippable waves. Each wave is build-passing and reversible (behind a feature flag/config where feasible). Waves 1–2 stop the active defects (stuck states, CPU), Waves 3–5 restore/upgrade functionality, Wave 6 removes the legacy/dead code only after the single pipeline is proven, and Waves 7–9 finish config integrity, UX, and the optional daemon. Do not begin implementation until this spec is approved.

## Task Dependency Graph

```json
{
  "waves": [
    { "id": "wave-0", "name": "Safety net & baseline", "dependsOn": [] },
    { "id": "wave-1", "name": "Recovery layer & FSM hardening", "dependsOn": ["wave-0"] },
    { "id": "wave-2", "name": "VAD-gated STT (CPU fix)", "dependsOn": ["wave-1"] },
    { "id": "wave-3", "name": "Modes (PTT/continuous/wake)", "dependsOn": ["wave-2"] },
    { "id": "wave-4", "name": "Speech-safe text + barge-in", "dependsOn": ["wave-3"] },
    { "id": "wave-5", "name": "TTS upgrade (Kokoro)", "dependsOn": ["wave-4"] },
    { "id": "wave-6", "name": "Dead code removal & unification", "dependsOn": ["wave-1", "wave-2", "wave-3", "wave-4", "wave-5"] },
    { "id": "wave-7", "name": "Configuration integrity", "dependsOn": ["wave-6"] },
    { "id": "wave-8", "name": "Frontend UX", "dependsOn": ["wave-7"] },
    { "id": "wave-9", "name": "Optional wake daemon", "dependsOn": ["wave-3"] }
  ],
  "criticalPath": ["wave-0", "wave-1", "wave-2", "wave-3", "wave-4", "wave-5", "wave-6"]
}
```

Critical path: 0 → 1 → 2 → 3 → 4 → 5 → 6. Waves 7–8 follow 6; Wave 9 may proceed in parallel after Wave 3 but ships disabled.

## Tasks

Implementation is organized into sequential waves. Each wave is independently shippable, build-passing, and reversible. Do not begin implementation until this spec is approved.

---

## Wave 0 — Safety net & baseline

- [x] 0.1 Establish voice test harness and baseline metrics
  - Synthetic-audio live harness (`tests/stt_sidecar_live.rs`) + runtime per-turn metrics (`voice/turn_diagnostics.rs`). **CI baseline lane added** (`.github/workflows/voice-ci.yml`): runs voice unit/contract tests, wake-daemon tests, config validation, sidecar `py_compile`, and frontend typecheck/build on voice-path changes (no A/V hardware).
  - _Requirements: 10.1, 10.4, 12.2_

- [x] 0.2 Snapshot current contracts
  - Contract test added: `voice_runtime_helpers::contract_tests` locks the `voice:*` event-name mapping (state/partial/transcript/error/busy/playback/interruption) so any change is intentional (Req 12.1). 2/2 pass.
  - _Requirements: 12.1_

**Objectives:** measurable baseline + guardrails before refactor.
**Acceptance:** harness runs in CI; baseline numbers recorded; contract test green.
**Validation:** `cargo test -p kria-core` voice suite runs; harness reproduces a stuck-state case.
**Rollback:** test-only additions; remove harness.
**Test plan:** harness self-test on synthetic audio.
**Completion:** baseline doc committed; contract test passing.

---

## Wave 1 — Recovery layer & FSM hardening (stop the bleeding)

- [x] 1.1 Introduce RecoveryLayer watchdog around the session FSM
  - Implemented as a per-turn watchdog (cancels the shared turn token after `max_turn_ms`) rather than a separate layer type — see Decision Log. Added to `run_turn` and `run_speak_turn`.
  - _Requirements: 4.1, 4.2, 4.3, 4.4_

- [x] 1.2 Add hard timeout to STT final join and turn-total budget
  - `transcribe_timeout_ms` (60 s default) wraps the STT-final join; `max_turn_ms` (120 s) bounds the whole turn. Both env-overridable.
  - _Requirements: 4.1, 4.3_

- [x] 1.3 Guarantee bounded `stop_voice`/abort to Idle
  - Existing `force_abort` verified; recovery paths now also finalise to Sleeping.
  - _Requirements: 4.5_

- [x] 1.4 Extend FSM + telemetry to 8 explicit states (no Transcribing/Thinking collapse at event boundary)
  - Done: `v2_telemetry_to_event` now emits granular states (`idle/listening/transcribing/thinking/speaking/interrupt`) + `wake_listening`/`error` via dedicated events; frontend renders each distinctly. Locked by the Wave 0.2 contract test.
  - _Requirements: 9.1, 10.1_

**Objectives:** no state can hang; UI can see true state.
**Acceptance:** every Req-4 timeout path returns to Idle in tests; FSM emits distinct states.
**Validation:** integration tests force STT/agent stalls and assert recovery; manual stuck-state case no longer hangs.
**Rollback:** feature flag `voice_recovery_v3`; disable to revert to prior behavior.
**Test plan:** transition-table unit tests + stall-injection integration tests.
**Completion:** stuck-Listening and stuck-Thinking reproductions resolved; CI green.

---

## Wave 2 — CPU fix: VAD-gated STT (Silero) + remove inline RMS

- [ ] 2.1 Promote Silero VAD to the single VadLayer (endpoint + barge-in signal)
  - DEFERRED: replaces live endpoint behavior; requires audio-device runtime verification.
  - _Requirements: 3.3, 5.1_

- [ ] 2.2 Remove inline RMS VAD from the run loop; honor configured silence/threshold
  - DEFERRED: endpoint-timing behavior change, not runtime-verifiable here (attempted + reverted to avoid unverified regression).
  - _Requirements: 3.3, 8.1_

- [x] 2.3 Gate STT so no inference runs in Idle/wake-listening or on silence
  - Energy gate added to whisper-rs partial path: windows below `PARTIAL_SILENCE_RMS` are never decoded (the primary idle/quiet CPU fix). Unit-tested.
  - _Requirements: 3.1, 3.3_

- [x] 2.4 Replace 4 Hz rolling-window partials with optional, rate-capped, tier-gated partial pass (advisory only)
  - Partials now honor `enable_partial_transcripts` config and are forced off on tier C; silence-gated. (Full streaming_decoder scheduler swap deferred.)
  - _Requirements: 3.2, 3.4, 6.3_

**Objectives:** near-zero idle CPU; bounded partial cost.
**Acceptance:** idle CPU within target; partials disabled by default on low-RAM tier; config flag honored.
**Validation:** performance test asserts idle CPU drop vs Wave 0 baseline; verify no STT calls on silence (log assertion).
**Rollback:** flag `voice_vad_gating_v3`.
**Test plan:** VAD endpoint unit tests; CPU perf test; partial-rate cap test.
**Completion:** measured idle CPU reduction; partial cadence capped.

---

## Wave 3 — Modes made real (PTT / continuous / wake)

- [x] 3.1 Implement mode dispatch in session start (read `voice.mode`)
  - Implemented in `start_voice_v2_loop`: continuous / push_to_talk / wake_word now behave distinctly (previously all collapsed to continuous). Desktop build verified.
  - _Requirements: 2.1, 2.2, 2.3, 2.5_

- [x] 3.2 Wire push-to-talk key gating (mic open only while engaged)
  - True hold-to-talk implemented: backend `voice_ptt_release` Tauri command → `VoicePipelineV2::signal_ptt_finalize()` → capture loop ends the utterance immediately (checked per-chunk + every 250 ms), instead of waiting for VAD silence. Frontend press-and-hold button in `ChatView` (pointer + keyboard accessible) calls `voicePttPress`/`voicePttRelease`; `voicePttActive` reflected in the overlay. TODO: bind the configured global shortcut (`push_to_talk_key`) for app-wide PTT; live mic validation in QA.
  - _Requirements: 2.1_

- [x] 3.3 Wire openWakeWord into the loop for wake mode (Idle runs VAD+wake only)
  - `voice-wake-oww` feature enabled by default; wake detector spawned + each turn gated on a wake event (no STT runs while waiting). RUNTIME-VERIFIED: 3 oww models load (active=true), no false-fire on silence. Actual "Hey Ria" firing needs a human (synthetic TTS phrase scored 0.0 — see Runtime Validation Log).
  - _Requirements: 2.3, 2.4, 11.4_

- [x] 3.4 Implement continuous auto re-arm after each turn
  - Preserved/confirmed as the continuous-mode path.
  - _Requirements: 2.2_

## Runtime Validation Log (live, on-device)

Executed via `crates/kria-core/tests/voice_silence_gate_live.rs` against the real `ggml-large-v3-turbo-q5_0` model and the openWakeWord models, using Piper-synthesized speech (no human input):

- **CPU/silence (Scenario B):** 4 s of silence → `partials_on_silence=0`, final transcript `""`; total elapsed dropped 21.6 s → 4.0 s after the silence final-decode gate. Confirms the engine performs ZERO Whisper inference on silence. ✅
- **STT on real speech (Scenario A, STT portion):** Piper "what time is it today" → final transcript `"What time is it today?"` (production-default path, partials off). ✅
- **Wake model load (Scenario E setup):** detector `active=true`; no false-fire on silence. ✅
- **DISCOVERED BUG:** partials-ON path hits a whisper.cpp `failed to encode` / blank-final concurrency issue (partial+final share one `WhisperContext`). Production default keeps partials OFF (config `enable_partial_transcripts=false`, now honored), so the shipping path is unaffected and proven correct. Partials must stay disabled until this is fixed.
- **DISCOVERED + FIXED:** Whisper hallucinated text on silence ("The best way to do that…"); now suppressed by the final silence gate → empty transcript, turn bails cleanly.

### Needs human voice (cannot be done autonomously)
Real "Hello KRIA" full round-trip timing, real "Hey Ria" wake firing, barge-in (Scenario F), and live PTT/continuous behavior require a person speaking into the mic. Synthetic TTS "hey ria" did not trip openWakeWord (score 0.0) — likely TTS-vs-human pronunciation mismatch; verify by speaking "Hey Ria".

**Objectives:** the selected mode behaves as labeled.
**Acceptance:** each mode passes its behavioral test; unknown mode warns + defaults.
**Validation:** integration tests per mode; manual wake test ("Hey Ria").
**Rollback:** flag `voice_modes_v3`; fall back to continuous-only.
**Test plan:** per-mode behavioral tests + wake-detection test with shipped models.
**Completion:** PTT, continuous, and wake all verified.

---

## Wave 4 — Speech-safe text + barge-in

- [x] 4.1 Move sanitizer BEFORE sentence splitting; extend to code/JSON/emoji/URLs
  - `normalize_for_tts` extended: strips backticks (incl. unbalanced fence fragments), brace/bracket/angle scaffolding (tool-call/JSON), and emoji/pictographs. Unit-tested. (Per-sentence application already in the TTS path; reorder-before-split fully addressed once the agent path is refactored in 4.2.)
  - _Requirements: 7.1, 7.3_

- [x] 4.2 Ensure voice agent path never streams tool-call/structured tokens to TTS
  - The voice path already uses a slim conversational prompt (no tool catalog). Additionally, every sentence/tail is now sanitized with `normalize_for_tts` at the pipeline synth call sites (engine-agnostic, both `run_turn` and `run_speak_turn`) so markup/JSON/tool-call/emoji never reach ANY TTS backend, and empty-after-sanitize sentences are skipped.
  - _Requirements: 7.3_

- [x] 4.3 Wire barge-in: VAD speech-start during Speaking → cancel playback → Listening (AEC/headphone gated)
  - Added `VoicePipelineV2::request_barge_in()` (cancels the turn token only when `Speaking`; `run_turn`'s `turn.cancelled()` branch then sets `BargeIn` + `abort_root` → stops TTS/playback/LLM in one tick). A session-level barge-in watcher in `start_voice_v2_loop` runs an energy VAD over the capture broadcast and triggers it on sustained speech. Half-duplex naturally suppresses mic during playback (forwarder drops chunks), so voice barge-in is effective only in headphone/AEC mode (Req 5.1/5.3). NEEDS HUMAN VALIDATION (live mic + headphones) — see TODO.
  - _Requirements: 5.1, 5.2, 5.3, 5.4_

- [x] 4.4 Honor `barge_in.enabled`; suppress false barge-in claim when AEC unavailable in half-duplex
  - Watcher is only spawned when `voice.barge_in.enabled` is true; debounced by `barge_in.min_speech_ms`; threshold from `voice.energy_threshold`. Half-duplex suppression handled upstream (no voice-barge-in claim without headphone/AEC).
  - _Requirements: 5.3, 5.4_

**Objectives:** clean speech; working interruption.
**Acceptance:** sanitizer test corpus passes (no markup/JSON/emoji spoken); barge-in cancels within latency target.
**Validation:** unit sanitizer tests; barge-in latency integration test (AEC/headphone).
**Rollback:** flags `voice_sanitizer_v3`, `voice_bargein_v3`.
**Test plan:** sanitizer corpus; barge-in cancel-latency test; half-duplex no-claim test.
**Completion:** TTS never reads markup/dev content; barge-in verified where supported.

---

## Wave 5 — TTS upgrade (Kokoro primary, Piper fallback)

- [x] 5.1 Add `KokoroTts` engine behind `Tts` trait (streamed chunks)
  - `KokoroTts` in `voice/v2/tts.rs` talks to a new Kokoro sidecar (`sidecars/kria-tts/main.py`, stdlib `http.server`, KPipeline, 24 kHz f32 PCM binary response). New launcher `voice/v2/tts_sidecar.rs` (URL `KRIA_TTS_SIDECAR_URL` default `:8766`, health probe, spawn, warm-up). Selected via `tts_engine = "kokoro"`.
  - _Requirements: 7.2, 7.4_

- [x] 5.2 Make Piper the guaranteed fallback; engine selection tier/config aware
  - Builder constructs the Piper engine first, then wraps it as the `KokoroTts` fallback only when `tts_engine = "kokoro"`. Any Kokoro failure (sidecar down / model missing / synth error) transparently delegates to Piper per sentence — selecting Kokoro never breaks audio. Default remains Piper.
  - _Requirements: 7.4_

- [x] 5.3 Ensure interruption stops synthesis without leaking queued audio
  - `KokoroTts::synthesize_sentence` races the sidecar request against `abort_rx.changed()`; on abort it drops the in-flight synth and emits nothing (same abort contract as the Piper engines).
  - _Requirements: 7.5_

**Objectives:** higher-quality, multilingual (incl. Hindi) TTS with safe fallback.
**Acceptance:** Kokoro active when available; Piper fallback on failure; interruption clean.
**Validation:** Build ✅ (core + desktop); Python sidecars `py_compile` ✅; `tts_sidecar` unit tests ✅. **Runtime VERIFIED** — kokoro 0.9.4 installed in a dedicated py3.12 venv (`sidecars/kria-tts/venv`); sidecar `/health` reports `model_loaded:true`, `/synthesize` returns real 24 kHz f32 PCM (3.55 s for a test line, peak 0.308). `espeakng-loader` bundles espeak-ng (no sudo needed).
**Rollback:** `tts_engine` defaults to Piper; Kokoro is opt-in (`tts_engine = "kokoro"`).
**Completion:** Kokoro engine + sidecar + fallback wired AND runtime-verified. Enable with `voice.tts_engine = "kokoro"`.

### Wave 5 — Progress
- **Completed (code + runtime):** 5.1, 5.2, 5.3 — Kokoro installed & serving in `sidecars/kria-tts/venv` (py3.12); English synthesis verified end-to-end.
- **Fixed (red-dot bug):** the TTS launcher `resolve_python` was preferring the repo `.venv` (py3.14, no kokoro) → sidecar reported `model_loaded:false` (red dot) and fell back to Piper. Now prefers the dedicated `sidecars/kria-tts/venv` first. Also added dead-child self-heal (respawn on exit) to both STT/TTS launchers.
- **Remaining:** human audio-quality A/B + Hindi (`KRIA_TTS_LANG=h`) spot-check (QA).
- **Files modified:** `sidecars/kria-tts/main.py` (new), `sidecars/kria-tts/requirements.txt` (new), `crates/kria-core/src/voice/v2/tts.rs`, `crates/kria-core/src/voice/v2/tts_sidecar.rs` (new), `crates/kria-core/src/voice/v2/mod.rs`, `crates/kria-core/src/config.rs`, `crates/kria-desktop/src/commands/voice_runtime_helpers.rs`.

## TODO — human/runtime validation (final QA phase)
- **Wave 5 Kokoro: INSTALLED & VERIFIED.** kokoro 0.9.4 is set up in `sidecars/kria-tts/venv` (py3.12) and serves real 24 kHz audio. To activate: set `voice.tts_engine = "kokoro"`. Remaining: human audio-quality A/B + Hindi (`KRIA_TTS_LANG=h`) spot-check.
- **Wave 3.2 PTT / 4.3 barge-in / 5 Kokoro audio:** require a human with mic/headphones/speaker — defer to QA.
- **Wake firing ("Hey Ria"):** real human phrase (synthetic TTS scored 0.0).

---

## Wave 6 — Dead code removal & unification

- [ ] 6.1 Remove v1 `VoicePipeline` and the v1 event-forwarder command block
  - DEFERRED (gated): v1 is the active rollback while v2+sidecar is proven. Removal requires the single pipeline to pass human mic QA (Req 12.2/12.3). See TODO.
  - _Requirements: 1.1, 1.2_

- [x] 6.2 Remove dead modules (old `SidecarStt`, …)
  - Removed the superseded `SidecarStt` stub (only ever `bail!`ed; replaced by `SidecarFasterWhisperStt`), its impl + unit test, and updated module docs. Build + `voice::v2::stt` tests green. (Other dead modules — refiner/post-edit/reconcile, unreferenced `voice/*` — to be swept with 6.1 after QA.)
  - _Requirements: 1.3_

- [ ] 6.3 Consolidate STT/TTS behind single trait abstractions; delete duplicate warmup ownership
  - PARTIAL: STT/TTS already flow through single `Stt`/`Tts` traits; the v1 whisper warmup in `build_voice_pipeline` is the remaining duplicate, removed with 6.1/6.5 post-QA.
  - _Requirements: 1.4_

- [ ] 6.4 Collapse `ActivePipeline` enum to a single pipeline type
  - DEFERRED (gated): depends on 6.1 (v1 removal).
  - _Requirements: 1.1_

- [ ] 6.5 Remove in-process whisper-rs STT and the v1 STT warmup
  - DEFERRED (gated): whisper-rs is the explicit STT rollback the user asked to keep until Wave 6 is QA-proven. Remove once the faster-whisper sidecar passes live QA.
  - _Requirements: 1.1, 1.3_

- [ ] 6.6 Remove whisper-rs stabilization band-aids
  - DEFERRED (gated): tied to 6.5.
  - _Requirements: 1.3_

**Why deferred:** Wave 6's destructive removals eliminate the rollback path before the new single pipeline is runtime-proven. The spec's own dependency states removal "occurs only after Waves 1–5 prove the single pipeline," and several proofs need human mic/headphone/speaker QA. Executing now would violate reversibility (Req 12.3) and risk a large unverifiable change. The safe, unreferenced removal (6.2) was done.

### Wave 6 — Progress
- **Completed:** 6.2 (old `SidecarStt` stub removed).
- **Deferred (gated on QA):** 6.1, 6.3, 6.4, 6.5, 6.6.
- **Files modified:** `crates/kria-core/src/voice/v2/stt.rs`, `crates/kria-core/src/voice/v2/mod.rs`.

---

## Wave 7 — Configuration integrity

- [x] 7.1 Audit every voice setting; remove or wire each to a real runtime effect
  - `VoiceConfig::validate()` added (unit-tested): flags unknown `mode`/`stt_engine`/`tts_engine`, out-of-range `confidence_threshold`/`wake_word.sensitivity`, `barge_in.min_speech_ms=0`, `energy_threshold<=0`, Kokoro dependency, and wake_word/mode mismatch. Surfaced as `config_warnings` in `voice_v2_status` + frontend `voiceConfigWarnings`. Engine + sidecar health also surfaced.
  - _Requirements: 8.1, 8.4_

- [~] 7.2 Implement turn-boundary hot reload (no mid-turn corruption)
  - Barge-in (`barge_in.enabled`/`min_speech_ms`/`energy_threshold`) now re-read from live config at each Speaking-episode boundary — toggling takes effect next turn without restart. Per-turn LLM context (prompt/memory) already re-read each turn. REMAINING: mode change to/from `wake_word` still requires restart (wake-detector lifecycle) — documented TODO.
  - _Requirements: 8.2_

- [ ] 7.3 Document and enforce precedence (env > user > default > code)
  - TODO: env overrides exist (`KRIA_TIER`, `KRIA_STT_*`, `KRIA_TTS_*`, `KRIA_VOICE_*`); central precedence doc/enforcement for all voice knobs still pending.
  - _Requirements: 8.3_

### Wave 7 — Progress
- **Completed:** config integrity validation (7.1) + engine/sidecar/health observability; barge-in turn-boundary hot reload (7.2 partial).
- **Remaining:** mode-change hot reload (wake re-spawn), precedence doc (7.3).
- **Files modified:** `crates/kria-core/src/config.rs` (validate + tests), `voice_diagnostics.rs` (config_warnings), `voice_runtime_helpers.rs` (barge-in live config).

## Voice Observability & Telemetry (Wave 7)

Structured per-turn telemetry + failure tracking, exposed via the
`voice_turn_diagnostics` Tauri command and surfaced in the overlay.

- **New module `voice/turn_diagnostics.rs`** (unit-tested): bounded ring buffer (64 turns) of `VoiceTurnRecord` with typed `TurnOutcome` (completed / empty_transcript / barge_in / busy / timeout / error) and `FailureClass` classification (stt_empty, stt_sidecar_unavailable, stt_decode, stt_timeout, llm_routing, llm_timeout, tts_synthesis, model_unavailable, gpu_lease, capture, playback, turn_timeout, cancelled, other). `aggregate()` → counts + e2e p50/p95 + top failure.
- **Recording**: the desktop telemetry pump records a turn on `Metrics` (completed/empty), `Error` (error/timeout), and `BusyRejected`, tagging the resolved STT/TTS engine + transcript length.
- **Derived latencies on `VoiceMetrics`** (only when actually measured — no placeholders): `stt_latency_ms`, `partial_latency_ms`, `llm_ttft_ms`, `llm_completion_ms`, `tts_gen_ms`, `tts_total_ms`, `playback_start_ms`, `end_to_end_ms`. Marks: mic_capture, vad_trigger/endpoint, stt_first_token, llm_first_token, **llm_complete**, tts_first_chunk, **tts_complete**, first_partial, final, post_edit, first_audio_out.
- **Failure-answering**: classification + `reason` string answer why a turn failed/timed out/returned empty/STT-empty/TTS-failed/model-unavailable/GPU-lease-failed/cancelled.
- **Frontend**: overlay shows aggregate health (`turns ok`, e2e p50, failed count, top failure) + sidecar dots + engines + config warnings; `voice_turn_diagnostics` exposes the full record list for a future detail panel.

### Honestly NOT yet instrumented (no placeholder emitted)
- Wake/PTT/barge-in *failure* reasons surface via the generic error path + `voice:wake`/state events; dedicated wake-miss / PTT-miss counters are a follow-up.
- Per-sentence TTS timing (only first-chunk + total-span are marked, not each sentence).

---

## Wave 8 — Frontend UX

- [x] 8.1 Render full FSM states with distinct labels + mic-level meter
  - Backend `v2_telemetry_to_event` now emits granular states (`transcribing`, `thinking`, `interrupt`, plus `wake_listening`, `error`) instead of collapsing to `processing`. Frontend `VoiceUiState` extended; `VoiceOverlay` shows a distinct label + accent per state, animated waveform while the mic is hot, and per-state CSS. (Mic-level meter uses the existing waveform animation; a true RMS meter is a follow-up TODO.)
  - _Requirements: 9.1_

- [x] 8.2 Advisory partials in overlay; move debug breadcrumbs behind dev flag (out of chat)
  - Partials now land in a separate `voicePartialTranscript` signal and render dimmed + italic in the overlay (non-authoritative), cleared on commit/listen/idle. The committed transcript is rendered distinctly. (Chat-mirror debug breadcrumb behind a dev flag = remaining TODO.)
  - _Requirements: 9.2_

- [x] 8.3 Latency/health indicators + mode switch reflecting real behavior
  - Overlay shows IO mode (headphone/half-duplex), TTFA, playback health, interruption reason, and **STT/TTS sidecar health dots + engine names** sourced from `voice_v2_status` (`refreshVoiceStatus` on start + wake). Wake state surfaced via `voice:wake` flash + `wake_listening` state.
  - _Requirements: 9.3, 8.4_

- [x] 8.4 Voice onboarding + accessibility (aria-live, keyboard PTT/stop) + live mic meter
  - Mic-level meter end-to-end (backend `voice:mic_level` RMS → overlay meter). **Onboarding wizard added** (`VoiceOnboarding.tsx`): 3 steps — mic test + live meter + device list (links to Settings), wake-word guidance, engines/health + config-warning summary; opened via the overlay ⚙ button (`openVoiceOnboarding`), completion persisted to localStorage. Accessibility: overlay `role="status" aria-live="polite"`, keyboard PTT (Space/Enter hold) + labeled stop, dialog `aria-modal`.
  - _Requirements: 9.1, 9.4_

**Objectives:** UI never appears stuck/unfinished.
**Acceptance:** state labels match backend; partials advisory; health/engines visible.
**Validation:** `tsc --noEmit` clean + `vite build` ✅. Visual/UX walkthrough = QA.
**Rollback:** UI-only; revert components.
**Completion (code):** 8.1/8.2/8.3 done; 8.4 partial (onboarding wizard TODO).

### Wave 8 — Progress
- **Completed:** 8.1, 8.2, 8.3.
- **In Progress:** 8.4 (a11y baseline done; onboarding wizard TODO).
- **Files modified:** `crates/kria-desktop/src/commands/voice_runtime_helpers.rs` (granular states + `voice:wake`), `ui/src/stores/app.ts` (state type, signals, listeners, PTT + status fns, exports), `ui/src/components/VoiceOverlay.tsx`, `ui/src/components/ChatView.tsx` (PTT hold button), `ui/src/styles/base.css`.

---

## Wave 9 — Optional wake daemon (extension)

- [x] 9.1 Build unprivileged daemon: VAD + openWakeWord only
  - New crate `crates/kria-wake-daemon` (added to workspace). Runs unprivileged; reuses `kria-core` `WakeWordDetector` + `AudioCapture` — NO STT/TTS/LLM. Config via env (`KRIA_WAKE_MODEL`, `KRIA_WAKE_SENSITIVITY`, `KRIA_WAKE_SOCK`, `KRIA_WAKE_LAUNCH`, `KRIA_WAKE_MIC`). Exits cleanly when the detector is inactive (in-app wake remains the fallback).
  - _Requirements: 11.1_

- [x] 9.2 IPC wake signal → launch/wake app + start session
  - `ipc.rs`: newline-delimited JSON `WakeSignal` over AF_UNIX (app is listener, daemon connects — no privilege needed). On wake: delivers the signal to the app socket; if no listener, launches the app via `KRIA_WAKE_LAUNCH` (cold-start path). Debounced 1.5 s. Socket roundtrip + no-listener fallback + `drain_lines` unit-tested (6/6).
  - _Requirements: 11.2_

- [x] 9.3 Visible mic indicator + explicit permission; in-app wake remains fallback
  - Daemon logs a prominent "🎙️ microphone is being monitored (wake-only)" banner at start (Req 11.3). Default-off / opt-in (separate process the user must launch); disabling it leaves in-app wake working (Req 11.4).
  - _Requirements: 11.3, 11.4_

**Objectives:** safe cold-start always-on, optional.
**Validation:** `cargo test -p kria-wake-daemon` 6/6 (IPC encode/decode, drain, socket roundtrip, no-listener). Live mic wake firing = QA. Build ✅.
**Rollback:** ship disabled by default; daemon is a separate binary the user opts into. Removing it does not affect in-app voice.
**Completion:** daemon + IPC + launch fallback + mic banner + **app-side warm-path socket listener** implemented & unit-tested.

### Wave 9 — Progress
- **Completed:** 9.1, 9.2, 9.3 + app-side warm-path listener (`commands/wake_listener.rs`: binds the socket, parses `WakeSignal`, emits `voice:external_wake` → frontend auto-starts a session). Cold-start via `KRIA_WAKE_LAUNCH` also works.
- **Remaining:** live mic wake-firing validation (QA only).
- **Files:** `crates/kria-wake-daemon/{Cargo.toml,src/main.rs,src/ipc.rs}` (new), `crates/kria-desktop/src/commands/wake_listener.rs` (new) + setup spawn + `libc` dep, `ui/src/stores/app.ts` (`voice:external_wake`), workspace `Cargo.toml`.

---

## Notes

## Cross-wave completion criteria
- Each wave: build passes, no contract break (Wave 0.2), voice functions at least as well as before that wave (Req 12.2).
- Each wave is behind a flag/config where feasible for reversibility (Req 12.3).
- v1/dead-code removal (Wave 6) occurs only after Waves 1–5 prove the single pipeline.

## Reality Reconciliation — Round 2 (post human runtime findings)

Reclassified against real usage. States: COMPLETE / IN PROGRESS / BLOCKED / NEEDS HUMAN VALIDATION / FAILED.

| Issue | Area | Status | Fix shipped this round | Verification |
|---|---|---|---|---|
| 3 | Lifecycle: stuck-after-restart | **COMPLETE (code) / NEEDS HUMAN VALIDATION** | Session epoch (`begin_new_session`/`current_session`) kills stale loop+forwarder on restart; `start_voice_v2_loop` force-aborts prior turn at entry; telemetry converted mpsc→**broadcast** so each session re-subscribes (UI no longer goes dead on 2nd session) | build + 323 unit tests; live restart needs human |
| 7 | Context overflow → stuck Thinking | **COMPLETE** | Voice now uses a slim conversational system prompt with NO tool catalog (the v2 voice path never executed tools anyway) | desktop build |
| 1+2 | Endpointing: 40-50 s waits / must speak loud | **COMPLETE (code) / NEEDS HUMAN VALIDATION** | Silero VAD wired into v2 capture endpoint (replaces fixed-RMS); RMS kept only as fallback; `set_vad_model_path` from `models/vad/silero_vad.onnx` | Silero engine RUNTIME-VALIDATED on synthetic speech (SpeechStart+SpeechEnd); integrated room behavior needs human |
| 8 (partial) | TTS speaks tool/JSON/markup | **IMPROVED** | slim prompt + sanitizer (round 1) stop most unwanted content; robotic voice (Kokoro) still pending | — |
| 4 | Push-to-talk | **IN PROGRESS** | one-shot semantics only; true hold-to-talk key gating still missing | — |
| 5 | Wake firing | **NEEDS HUMAN VALIDATION** | models load + no false-fire proven; synthetic "Hey Ria" scored 0.0 — real human phrase unverified | — |
| 6 | GPU cold-start / model unavailable | **BLOCKED (needs investigation + live LLM)** | not addressed this round | — |

### Discovered + fixed (runtime)
- whisper-rs **partials-ON concurrency bug** (`failed to encode` / blank final) → partials stay default-OFF (config honored); loud warning added.
- whisper **hallucination on silence** → final silence-gate returns empty.
- whisper **partial sub-1s window** `failed to encode` → min-window guard added.

### Still open / next
- Issue 4 hold-to-talk (needs global-shortcut + frontend signal).
- Issue 6 GPU lease/model-swap investigation (needs live LLM run).
- Issue 8 Kokoro TTS (external model download required — BLOCKER).
- Full live round-trip ("Hello KRIA"), restart cycle, and wake firing — require a human at the mic.

## Reality Reconciliation — Round 3 (real session log 2026-06-19 10:21-10:25)

The log proved the previous "fixes" were not the whole story. **The dominant failure is `whisper-rs: failed to encode` (error -6)**, which cascades: STT fails → no LLM → no TTS → "nothing works".

- **Turn 1 actually worked** end-to-end (STT "Ria, mera CPU" → LLM 14 tokens → piper synthesized 2.98 s audio), confirming the chain is sound when STT succeeds. Matches the user's "just once it ran."
- **Reproduced locally** with a new multi-turn test (`whisper_rs_multi_turn_reuse`): one shared engine, 3 sequential decodes of the same speech → turn 0 OK, turns 1-2 `failed to encode (-6)`. Root cause = (a) cached `WhisperContext` cannot be reused across decodes, and (b) an additional *transient* CPU encode failure even with a fresh context (same audio fails on some turns, passes on others).
- **FIX (validated 3/3):** `WhisperContext` is now created fresh per decode AND the decode retries up to 3× on encode failure. Multi-turn test: 3/3 turns now transcribe correctly.
- **Also confirmed/kept:** the `auto` language fix (explicit `set_language("auto")`), the GPU-lease removal (TTS), Silero endpointing.

### Still open (post-fix, need rebuild + human)
- **Latency:** large-v3-turbo on CPU is ~12 s/decode (whisper-rs built CPU-only; LLM uses CUDA). Retry worsens worst case. Follow-up: smaller STT model or `voice-whisper-cuda`.
- **Endpoint too eager:** turn 1 cut at "Ria, mera CPU" (Silero fired after ~1 s pause). Needs a longer end-silence / min-utterance.
- **Wake mode requires a mic click — user is correct that this defeats the purpose.** Current design only starts the loop on `start_voice`. True wake needs background always-on VAD+wake (auto-start in wake mode, or the wake daemon). Separate, larger change.

## V3 Revision — Round 4 (post faster-whisper benchmark)

A real on-device STT benchmark (RTX 4050 6 GB, i7-13700HX) proved the in-process whisper-rs (CPU) path is the dominant latency cost (7–13 s/decode, up to 17 s throttled) and cannot use the GPU. The only measured engine that is BOTH sub-second AND Hinglish-capable is **faster-whisper `small` INT8 on GPU (~0.23 s)**. This round moves STT into the existing Python sidecar (faster-whisper / CTranslate2) and retires the whisper-rs path plus its stabilization band-aids. See requirements "V3 Revision" section and design "Sidecar & GPU risks".

### Wave reclassification

| Wave / Task | Disposition | Reason |
|---|---|---|
| Wave 0 (harness, contracts) | **KEEP** | Still the baseline/guardrail; harness must add a sidecar-STT path + GPU/CPU latency baselines. |
| Wave 1 (recovery/watchdog) | **KEEP** | Unchanged; now also guards sidecar liveness/cold-start (see A3). |
| Wave 2.3/2.4 (silence-gate, partial config) | **MODIFY** | Silence-gate + min-window were whisper-rs band-aids → become **DELETE** once sidecar lands (Wave 6). VAD endpointing (2.1/2.2) is KEEP. |
| Wave 3 (modes) | **KEEP** | Engine-agnostic. |
| Wave 4 (sanitizer/barge-in) | **KEEP** | Engine-agnostic. |
| Wave 5 (Kokoro TTS) | **KEEP** | Unchanged. |
| Wave 6 (dead-code removal) | **EXPAND** | Now also removes in-process whisper-rs, v1 STT warmup, CLI-whisper primary fallback, auto-lang forcing, 3× encode retry, min-window guard, partial silence-gate. |
| In-process `WhisperStt` / whisper-rs | **DELETE / REPLACE** | Replaced by `SidecarFasterWhisperStt`. |
| Whisper-rs band-aids (auto-lang, retry, min-window, silence-gate from Round 2/3) | **DELETE** | Sidecar removes the conditions that required them. |
| **Wave A (sidecar faster-whisper STT)** | **NEW** | Default STT engine; impact-first. |
| **Wave A2 (streaming partials)** | **NEW** | Advisory partials from the sidecar. |
| **Wave A3 (VRAM coordination + liveness)** | **NEW** | GPU sharing with resident LLM; cold-start/health. |

### Revised impact-first implementation order

`Wave 0 → Wave 1 → Wave A → Wave A3 → Wave 3 → Wave 4 → Wave A2 → Wave 5 → Wave 6 → Wave 7 → Wave 8 → Wave 9`

Rationale: Wave A (sidecar STT) is now the single highest-impact change (removes ~95% of the measured latency) and should land right after recovery hardening, before mode/sanitizer/TTS polish. The old Wave 2 (VAD-gated whisper-rs CPU fix) is demoted: VAD endpointing (2.1/2.2) stays, but the CPU-gating band-aids (2.3/2.4) are superseded by the sidecar and removed in Wave 6.

---

## Wave A — Sidecar faster-whisper STT (NEW, default engine)

- [x] A.1 Add a faster-whisper STT module to the Python sidecar
  - Created `sidecars/kria-stt/main.py` (stdlib `http.server`, no web framework) + `requirements.txt`. Loads CTranslate2 faster-whisper once, kept warm; default `small`, `int8` (→ `int8_float16` on GPU), `device=auto` (CUDA when free VRAM ≥ 700 MiB, else CPU). `POST /transcribe` accepts raw f32-LE PCM (binary), returns `{text, language, confidence, duration_ms, engine, device}`; `GET /health` reports loaded model/device. RUNTIME-VERIFIED: loads on CUDA, transcribed "What time is it today?" in ~0.14 s round-trip.
  - _Requirements: 6.1, 6.2, 6.3_

- [x] A.2 Add `SidecarFasterWhisperStt` Rust client behind the `Stt` trait
  - `SidecarFasterWhisperStt` in `voice/v2/stt.rs` buffers the VAD-bounded utterance and POSTs raw f32-LE PCM (binary, no per-chunk JSON). Made the DEFAULT engine in `build_v2_with_cli_engines` (selected for `auto`/`faster-whisper`/`sidecar`); tier default updated to `faster-whisper`. whisper-rs kept as explicit rollback. RUNTIME-VERIFIED via `tests/stt_sidecar_live.rs`: 328 ms, correct transcript, `engine=faster-whisper`.
  - _Requirements: 6.1, 6.2, 6.5, 1.4_

- [x] A.3 Fallback + no-hang on sidecar unavailability
  - `voice/v2/stt_sidecar.rs` resolves URL (`KRIA_STT_SIDECAR_URL`, default `:8765`), health-probes, and best-effort spawns the Python process (kill-on-drop, single warm instance). On sidecar failure the client falls back to the always-available CLI whisper path; with no fallback it returns a typed error (no hang). Pipeline watchdog (Wave 1) bounds the call.
  - _Requirements: 6.5, 4.1, 8.4_

**Objectives:** sub-second Hinglish/English STT; remove the CPU encode bottleneck.
**Acceptance:** GPU `small` INT8 decode ≤ ~0.5 s on a ~2 s utterance; CPU fallback functional; Hinglish + English transcribe correctly.
**Validation:** live test 328 ms (vs ~13 s whisper-rs baseline); silence→empty verified; sidecar-down → CLI fallback (code path reuses tested v1 CLI).
**Rollback:** config `stt_engine = "whisper-rs"` (kept until Wave 6); engine routing in `build_v2_with_cli_engines`.
**Test plan:** sidecar unit test (EN clip), Rust client integration test, fallback/down test.
**Completion:** sidecar faster-whisper is the default; latency target met; fallback wired.

---

## Wave A2 — Streaming partials from the sidecar (NEW)

- [x] A2.1 Emit advisory streaming partials from the sidecar during capture
  - `SidecarFasterWhisperStt` now drives cadence-based rolling-buffer decodes (600 ms cadence) during capture, emitting advisory `PartialTranscript`s (one in flight at a time, silence-gated). Honors `enable_partial_transcripts`; forced OFF on tier C by the builder. The Python sidecar serializes overlapping partial+final requests with a `threading.Lock` so the authoritative final is never raced. RUNTIME-VERIFIED via `tests/stt_sidecar_live.rs::partials_stream_during_capture`: emitted "What time?" → "What time is it today?" then the authoritative final, all `engine=faster-whisper`.
  - _Requirements: 6.4, 3.2, 3.4_

- [~] A2.2 Surface partials in the overlay as advisory (reuse Wave 8 contract)
  - PARTIAL: the backend now emits partials through the existing `partial_tx` → telemetry path; final UI advisory rendering is tracked under Wave 8 (frontend).
  - _Requirements: 9.2_

**Objectives:** responsive partial feedback without affecting the authoritative final.
**Acceptance:** partials advisory only; disabled by default on low-RAM tier; final transcript unchanged by partials.
**Validation:** live partial-cadence test passed (2 partials + correct final); authoritative-final invariant held.
**Rollback:** config `enable_partial_transcripts=false` (default).
**Completion:** backend partials advisory and rate-capped; final integrity preserved. UI surface = Wave 8.

### Wave A2 — Progress
- **Completed:** A2.1 (backend streaming partials, thread-safe sidecar, tier/config gating).
- **In Progress:** A2.2 UI advisory rendering (frontend, folded into Wave 8).
- **Validation:** Build ✅ (core + desktop). Live test ✅ 3/3 (partials + final). Runtime ✅ (warm final 145 ms).
- **Files modified:** `sidecars/kria-stt/main.py` (threading.Lock), `crates/kria-core/src/voice/v2/stt.rs`, `crates/kria-core/src/voice/v2/mod.rs`, `crates/kria-core/tests/stt_sidecar_live.rs`.

---

## Wave A3 — GPU/VRAM coordination + sidecar liveness (NEW)

- [x] A3.1 VRAM coordination with the resident LLM
  - Sidecar `_resolve_device` probes free VRAM (torch `mem_get_info`); selects CUDA for `small` INT8 only when free ≥ `KRIA_STT_MIN_FREE_VRAM` (default 700 MiB), else CPU INT8 — never OOMs the resident LLM. Model loaded once and kept warm (no per-turn reload). Explicit `KRIA_STT_DEVICE=cuda|cpu` honored.
  - _Requirements: 6.2, 6.6_

- [x] A3.2 Sidecar liveness/health + cold-start handling
  - `stt_sidecar::warm_up()` is spawned at voice-session start (in `start_voice_v2_loop`) so the model loads up front, not on the first utterance; skipped when whisper-rs is explicitly selected. `ensure_ready` health-probes with a 30 s cold-load window; warm calls return immediately. Cold start is covered by this startup warm-up, not the per-turn watchdog.
  - _Requirements: 6.5, 4.1, 4.4_

**Objectives:** STT shares the GPU safely with the LLM; no cold-start hangs.
**Acceptance:** `small` chosen on GPU only within headroom; CPU fallback otherwise; no per-turn reload; cold start does not trip the turn watchdog.
**Validation:** device-resolution + CPU-fallback paths implemented and load-verified (CUDA selected with headroom); warm-up wired at session start.
**Rollback:** `KRIA_STT_DEVICE=cpu` forces CPU; engine rollback to whisper-rs.
**Completion:** VRAM coordination + liveness implemented and build-verified.

### Wave A / A3 — Progress
- **Completed:** A.1, A.2, A.3, A3.1, A3.2 (faster-whisper sidecar is the default STT, GPU INT8 small + CPU fallback, binary PCM transport, liveness + spawn + warm-up, CLI fallback no-hang).
- **In Progress:** —
- **Blocked:** —
- **Deferred:** streaming partials → Wave A2; removal of whisper-rs + band-aids → Wave 6 (kept as rollback per instructions).
- **Validation:** Build ✅ (`cargo check -p kria-core -p kria-desktop`). Tests ✅ (`stt`/`stt_sidecar`/`tier` unit tests; live `stt_sidecar_live` 2/2). Runtime ✅ (sidecar on CUDA, 328 ms Rust round-trip, silence→empty).
- **Files modified:** `sidecars/kria-stt/main.py` (new), `sidecars/kria-stt/requirements.txt` (new), `crates/kria-core/src/voice/v2/stt.rs`, `crates/kria-core/src/voice/v2/stt_sidecar.rs` (new), `crates/kria-core/src/voice/v2/mod.rs`, `crates/kria-core/src/voice/tier.rs`, `crates/kria-core/src/config.rs`, `crates/kria-desktop/src/commands/voice_runtime_helpers.rs`, `crates/kria-core/tests/stt_sidecar_live.rs` (new).

---

## Wave 6 (expanded) — STT removal addendum

In addition to the existing Wave 6 tasks, after Wave A is proven:

- [ ] 6.5 Remove in-process whisper-rs STT and the v1 STT warmup
  - Delete `WhisperRsStt`/in-process whisper path, the CLI-whisper primary fallback, and the v1 warmup ownership
  - _Requirements: 1.1, 1.3_

- [ ] 6.6 Remove whisper-rs stabilization band-aids
  - Delete auto-language forcing, the 3× encode-retry, the min-window guard, and the partial silence-gate (Round 2/3 fixes) — superseded by the sidecar
  - _Requirements: 1.3_

**Note:** these removals occur ONLY after Wave A + A3 prove the sidecar path end-to-end, preserving reversibility (the in-process path stays selectable via `stt_engine` until then).

## Implementation Reconciliation — Round 5 (autonomous backend completion)

Verified from code + build (`cargo build` full workspace green; targeted voice tests green; live STT sidecar tests green).

| Wave | Status | Notes |
|---|---|---|
| Wave A (sidecar STT) | **COMPLETE** | default engine, GPU INT8 small + CPU fallback, binary PCM, CLI fallback. Live 145–328 ms. |
| Wave A3 (VRAM + liveness) | **COMPLETE** | device coordination, spawn, warm-up at session start. |
| Wave A2 (streaming partials) | **COMPLETE (backend)** | cadence partials, thread-safe sidecar; live 2-partial test. UI = Wave 8. |
| Wave 3 (modes) | 3.1/3.3/3.4 COMPLETE; **3.2 PARTIAL** | true hold-to-talk needs frontend global shortcut (QA). |
| Wave 4 (speech-safe + barge-in) | **COMPLETE (backend)** | 4.1/4.2 sanitizer engine-agnostic at synth sites; 4.3/4.4 barge-in watcher + `request_barge_in`. Live mic = QA. |
| Wave 5 (Kokoro TTS) | **COMPLETE (code) / runtime BLOCKED** | engine + sidecar + Piper fallback wired; needs `kokoro` install + model download. |
| Wave 6 (dead-code removal) | 6.2 COMPLETE; **rest DEFERRED (QA-gated)** | old `SidecarStt` removed; v1/whisper-rs removal gated on single-pipeline QA proof (Req 12.3). |
| Wave 7 (config integrity) | **PARTIAL** | new sidecar engines + health surfaced in `voice_v2_status`; stale note fixed. Full knob audit + turn-boundary hot-reload remain. |
| Wave 8 (frontend UX) | **NOT STARTED** | partials/state/health now emitted by backend; UI rendering pending (frontend session). |
| Wave 9 (wake daemon) | **NOT STARTED** | net-new unprivileged daemon crate (frontend/daemon session). |

### Remaining work (next sessions)
- **Frontend (Wave 8 + 3.2):** render 8 FSM states + advisory partials + latency/health (consume `voice_v2_status` `stt_sidecar`/`tts_sidecar`); global-shortcut hold-to-talk key signal.
- **Wave 9:** unprivileged VAD+wake daemon + IPC wake signal.
- **Wave 7:** per-setting effect audit + turn-boundary hot reload + precedence enforcement/doc.
- **Wave 6 (post-QA):** remove v1 `VoicePipeline`, whisper-rs + band-aids, collapse `ActivePipeline`.

### Genuine blockers
- **Kokoro runtime:** external download/install required (`kokoro` pip + weights + `espeak-ng`).
- **Wave 6 destructive removal:** requires live mic QA proof of the single pipeline (reversibility, Req 12.3).
- **Live audio validations (3.2 PTT, 4.3 barge-in, 5 audio, wake firing):** require human mic/headphones/speaker.

## Implementation Reconciliation — Round 6 (full non-destructive completion)

All implementable, non-destructive waves are now code-complete with checks/tests.
Only Wave 6 (destructive legacy removal) is intentionally deferred until live QA
proves the new pipeline (rollback paths preserved per instruction).

| Wave / Task | Status | Tests |
|---|---|---|
| 0.1 harness/baseline | PARTIAL (live harness exists; CI baseline lane TODO) | live STT harness |
| 0.2 contract snapshot | **COMPLETE** | `contract_tests` 2/2 |
| 1.1–1.3 recovery/watchdog | COMPLETE | pipeline suite |
| 1.4 granular FSM states | **COMPLETE** | contract_tests |
| 2.x VAD-gated STT | superseded by Wave A sidecar (band-aids removed in Wave 6, gated) | — |
| 3.1/3.3/3.4 modes | COMPLETE | — |
| 3.2 hold-to-talk | **COMPLETE** (backend finalize + FE hold btn) | build |
| 4.1–4.4 sanitize + barge-in | COMPLETE | pipeline suite |
| A / A2 / A3 STT sidecar | COMPLETE | live + unit |
| 5 Kokoro TTS | CODE-COMPLETE (runtime download-gated) | tts_sidecar unit |
| 6 dead-code removal | **DEFERRED (QA-gated)** except 6.2 done | — |
| 7.1 config validation | **COMPLETE** | `voice_validate_tests` |
| 7.2 turn-boundary hot reload | **COMPLETE** (barge-in + mode) | build |
| 7.3 precedence (env>user>default>code) | **COMPLETE** | env-override tests |
| Observability / telemetry | **COMPLETE** | `turn_diagnostics` + `metrics` |
| 8.1/8.2/8.3 frontend states/partials/health | COMPLETE | tsc + vite |
| 8.4 mic meter + a11y | **COMPLETE** (onboarding wizard TODO) | tsc + vite |
| 9 wake daemon | **COMPLETE** | `kria-wake-daemon` 6/6 |

### Test summary (this round)
- `cargo test -p kria-core --lib voice::` → **329 passed**.
- `cargo test -p kria-core --lib config::voice_validate_tests` → 5 passed.
- `cargo test -p kria-desktop contract_tests` → 2 passed.
- `cargo test -p kria-wake-daemon` → 6 passed.
- `cargo build` (full workspace) ✅ · `tsc --noEmit` ✅ · `vite build` ✅.

### Genuine blockers / QA-gated (unchanged)
- Wave 6 destructive removal — needs live mic QA proof of the single pipeline (Req 12.3); rollback preserved.
- Kokoro runtime — external `kokoro` install + model download.
- Live A/V validations (PTT, barge-in, wake firing, Kokoro audio, onboarding walkthrough) — require human mic/headphones/speaker.
- CI audio-fixture baseline lane (0.1), app-side wake socket listener (warm auto-start), onboarding wizard — non-blocking follow-ups.

## Implementation Reconciliation — Round 7 (follow-ups closed + cleanup)

All previously-listed non-blocking follow-ups are now implemented with tests.

| Follow-up | Status | Verification |
|---|---|---|
| App-side wake socket listener (warm auto-start) | **DONE** | `kria-desktop wake_listener` 3/3 + setup spawn + frontend `voice:external_wake` |
| Onboarding wizard (mic test, device list, wake guide, health) | **DONE** | tsc + vite build; opened via overlay ⚙ |
| CI audio-fixture baseline lane (0.1) | **DONE** | `.github/workflows/voice-ci.yml` (rust/sidecar/frontend jobs) |
| Code cleanup | **DONE** | `cargo fmt` on touched crates; new crates warning-free |

### Final test/build status
- `cargo build` full workspace ✅
- `cargo test -p kria-core --lib voice::` → **329 passed**
- `cargo test -p kria-wake-daemon` → 6 passed
- `cargo test -p kria-desktop` voice (contract) → 2 passed; wake_listener → 3 passed
- `config::voice_validate_tests` → 5 passed
- `tsc --noEmit` ✅ · `vite build` ✅

### Remaining (intentionally NOT done — require human QA / external resources)
- **Wave 6 destructive removal** (v1 `VoicePipeline`, in-process whisper-rs, band-aids, `ActivePipeline` collapse): KEPT as rollback. Per Req 12.3 + your standing instruction, this only proceeds AFTER live mic QA proves the faster-whisper sidecar single pipeline. Removing it now would delete the safety net for an A/V-unvalidated path.
- **Kokoro runtime activation**: needs `kokoro` pip install + model weight download + `espeak-ng` (code-complete; Piper fallback active).
- **Live A/V validation**: PTT hold, barge-in cancel, "Hey Ria" wake firing, Kokoro audio quality, onboarding walkthrough — require a human with mic/headphones/speaker.

These are the ONLY open items and each is a genuine blocker (destructive-needs-proof / external-download / human-A-V). Everything implementable and verifiable without audio hardware is complete and green.

## Round 8 — live-usage bug fixes (user QA feedback)

1. **STT hallucinated on silence ("Don't translate to Devanagari") + low accuracy.**
   Root cause: faster-whisper echoed the long `INITIAL_PROMPT` (its tail was literally "Do not transliterate to Devanagari") on silence/noise, and `vad_filter` was OFF.
   Fix (`sidecars/kria-stt/main.py`): shortened `INITIAL_PROMPT`; enabled `vad_filter=True` (+ vad_parameters); added `no_speech_threshold`/`log_prob_threshold`/`compression_ratio_threshold`, `temperature=0`, and a hallucination/echo + high-no-speech guard that returns empty; raised default `beam_size` 1→5 for accuracy. VERIFIED: zeros/room-noise → empty; real speech transcribes; GPU latency still ~0.1–0.2 s.
3. **Chat flooded with debug, no assistant reply shown.**
   Fix: removed all `voice:debug`/partial "(live)" chat injection (debug now console-only; partials live only in the overlay). Backend now emits `voice:assistant_text` (accumulated LLM reply) → frontend appends it as a normal assistant message; committed user utterance appended as a normal user message. Chat now reads like a normal conversation.
4. **PTT button confusing in continuous mode.**
   Fix: the "Hold to talk" button is now shown ONLY when `voice.mode = "push_to_talk"` (exposed `mode` via `voice_v2_status`), with clearer label ("🎙 Hold to talk" / "🔴 Release to send").
2/Wake + extension: models present; requires `mode = "wake_word"` (config) and/or running the `kria-wake-daemon` binary — documented for the user (not a code bug).

Builds: `cargo build -p kria-desktop` ✅ · `tsc` ✅ · `vite build` ✅.
