# KRIA Voice Runtime v2 — P3 Implementation Tracker

**Status:** IN_PROGRESS  
**Spec:** `ENHANCED_STT.md` v1.0 (frozen)  
**Phase:** P3 — Production Integration + Real Runtime Wiring  
**Started:** 2026-05-15  
**Engineer:** Principal Rust Runtime Engineer

---

## 0. Production Runtime Audit Results

### Already Real (No Work Needed):
- ✅ CPAL microphone capture (device enum, failure recovery, default-change)
- ✅ rodio playback (dedicated worker thread, device selection)
- ✅ GPU lease coordination (GpuLeaseManager shared voice↔LLM)
- ✅ Tauri event wiring (state, transcripts, errors → frontend)
- ✅ VAD (Silero ONNX + energy threshold)
- ✅ v2 pipeline (streaming sentence playback, barge-in, wake-word)
- ✅ Config plumbing (VoiceConfig with all fields)
- ✅ Whisper warmup at startup
- ✅ v2 hot-swap (v1→v2 on config change)

### Needs P3 Integration:
- ⚠️ P2 FSMs not wired into v2 pipeline (transcript_authority, turn_ownership)
- ⚠️ Sidecar IPC not connected to real process
- ⚠️ Runtime telemetry not exposed to UI
- ⚠️ GPU lease FSM (§15 VoiceBorrow) not implemented
- ⚠️ Device hotplug recovery not fully exercised
- ⚠️ Runtime diagnostics panel not available

### P3 Strategy:
Since the production runtime is already substantially real, P3 focuses on:
1. Wiring P2 FSMs into the live v2 pipeline
2. GPU lease coordination for voice (§15 VoiceBorrow)
3. Runtime diagnostics + telemetry overlay
4. Production validation harness

---

## 1. P3 Implementation Plan

### Phase P3.1: FSM Integration Layer ✅ DONE
- [x] RuntimeBridge struct (coordinator, not orchestrator)
- [x] Wire TranscriptAuthorityFsm into bridge
- [x] Wire TurnOwnershipFsm into bridge
- [x] Wire RuntimeTelemetry (TTFA, latency histograms, queue monitors)
- [x] Wire WorkerBudget (Whisper: max 1 concurrent)
- [x] Wire DegradationLevel (skip_refinement hooks)
- [x] RuntimeLoadSnapshot generation
- [x] Full turn lifecycle test
- [x] Queue pressure monitoring
- [x] Degradation auto-update
- [x] Bridge reset for new sessions
- [x] 10 integration tests passing
- [x] All 265 voice tests passing

### Phase P3.2: GPU Lease Coordination (§15)
- [ ] VoiceBorrow state in GpuLeaseManager
- [ ] Whisper CUDA lease acquisition
- [ ] LLM ngl reduction during voice
- [ ] Lease release on turn complete
- [ ] Contention telemetry

### Phase P3.3: Runtime Diagnostics
- [ ] Tauri command for runtime snapshot
- [ ] Queue depth reporting
- [ ] TTFA reporting
- [ ] Interruption latency reporting
- [ ] Device status reporting
- [ ] Degradation level reporting

### Phase P3.4: Production Validation
- [ ] Long-session stability test
- [ ] Device hotplug validation
- [ ] Rapid barge-in validation
- [ ] Queue saturation validation
- [ ] TTFA measurement

---

## 2. Risks & Findings

### R-P3-001: Sidecar Binary Not Available
**Status:** ACCEPTABLE  
**Impact:** IPC transport tested in isolation only  
**Mitigation:** Protocol logic complete; real sidecar deferred to P3.5+

### R-P3-002: GPU Lease Contention
**Status:** MONITORING  
**Impact:** Whisper CUDA + LLM may contend on 6GB VRAM  
**Mitigation:** VoiceBorrow FSM + worker budget enforcement

---

## 3. Change Log

### 2026-05-15 (Initial)
- P3 tracking document created
- Production runtime audit completed
- Strategy defined: wire P2 FSMs into existing real runtime

### 2026-05-15 (P3.1 Complete)
- RuntimeBridge implementation (coordinator pattern)
- TranscriptAuthorityFsm wired via process_transcript_event()
- TurnOwnershipFsm wired via process_turn_event()
- TTFA tracking (record_ttfa, overrun detection)
- Interrupt/cancel latency histograms
- Queue pressure monitoring (audio + partial)
- Whisper worker budget enforcement (max 1)
- Degradation level auto-update
- RuntimeLoadSnapshot for diagnostics
- Full turn lifecycle integration test
- 10 new tests
- All 265 voice tests passing (255 + 10 P3.1)

---

*End of VOICE_P3_IMPLEMENTATION.md*
