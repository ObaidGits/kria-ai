# KRIA Voice Runtime — Real-World Assistant Validation

> **Purpose:** Permanent runtime validation tracker for KRIA as a daily-use desktop assistant
> **Start Date:** 2026-05-13
> **Scope:** UX feel, conversational smoothness, workflow usefulness, stability, operator trust
> **Goal:** Determine whether KRIA is genuinely useful, not just architecturally sound

---

## 1. Validation Approach

### Core Questions

1. **Does KRIA actually feel assistant-like?**
   - Or still feel robotic / command-focused / latency-obvious?

2. **Where does interaction still feel awkward?**
   - Specific latency ranges, state transitions, error modes?

3. **What destroys user trust fastest?**
   - False activations? Dangerous commands? Interruption failures?

4. **What workflow behaviors feel magical/useful?**
   - When does voice actually save time vs. slow things down?

5. **What latency ranges are actually acceptable in practice?**
   - <200ms? <500ms? <1s? Context-dependent?

6. **Which failures are dangerous vs. merely annoying?**
   - Safety-critical bugs vs. polish issues?

7. **Does voice interaction improve productivity or slow it down?**
   - Honest assessment per workflow type.

8. **Which runtime issues matter MOST in real use?**
   - Not benchmark worst-case, but 90th-percentile weekday?

### Severity Classification

| Level | Definition | Example | Action |
|-------|-----------|---------|--------|
| **🔴 CRITICAL** | Dangerous or breaks trust immediately | False activation that runs commands; barge-in doesn't stop playback | Fix before claiming assistant-ready |
| **🟠 HIGH** | Severe UX degradation; frequently noticed | Interruption latency >500ms; freezes >2s; repeated failures | Fix before release |
| **🟡 MEDIUM** | Noticeable but tolerable; occasional friction | Partial transcript flicker; occasional device recovery lag | Polish post-release |
| **🟢 LOW** | Minor polish; barely noticed | Slight TTS artifact; rare state timeout | Track for future |

---

## 2. Validation Scenarios

### Coding Workflows
- [ ] Voice command to open editor, navigate file, insert code
- [ ] Interrupt mid-synthesis while thinking about next edit
- [ ] Chain 3+ commands in rapid succession
- [ ] Mixed voice + keyboard editing
- [ ] Error recovery (misheard command, wrong file)

### Browser/Research Workflows
- [ ] Search web, read results, ask follow-ups
- [ ] Interrupt response mid-stream
- [ ] Navigate between tabs via voice
- [ ] Back button / navigation commands
- [ ] Long multi-turn conversation about search results

### Terminal/Automation Workflows
- [ ] Build project, report status, interrupt on failure
- [ ] Run tests, ask for help reading output
- [ ] Dangerous commands (file operations) — safety?
- [ ] Error interpretation
- [ ] Re-run / retry patterns

### File Management Workflows
- [ ] List files, ask for details, open one
- [ ] Rename/move files via voice
- [ ] Undo accidental operations
- [ ] Search files by properties
- [ ] Bulk operations

### Interruption-Heavy Sessions
- [ ] Rapid barge-in while assistant is speaking
- [ ] Barge-in during synthesis (mid-chunk)
- [ ] Barge-in during microphone input (restart capture)
- [ ] Multiple rapid interrupts
- [ ] Interrupt then immediate new command

### Long Conversations
- [ ] 30-min+ conversation (stability)
- [ ] Context accumulation (token growth)
- [ ] Partial transcript stability over time
- [ ] Memory leaks (CPU/RAM creep)
- [ ] Device connection stability

### Noisy Environment
- [ ] Coffee shop / public space
- [ ] Background music / TV
- [ ] Multiple speakers nearby
- [ ] False activations?
- [ ] Transcription quality

### Headphone Mode
- [ ] Unplug/replug during conversation
- [ ] Switch headphones
- [ ] Device recovery behavior
- [ ] Echo prevention quality
- [ ] Latency with headphones

### Dangerous Edge Cases
- [ ] Commands that could delete data
- [ ] Commands that could expose secrets
- [ ] Misheard commands (safety implications)
- [ ] Barge-in safety (what if interrupted wrong moment?)
- [ ] Hallucinated actions

---

## 3. DAILY VALIDATION LOG

> Record date, scenario, findings, severity, notes

### 2026-05-13 — Initial Baseline Audit

**Date:** 2026-05-13
**Session Duration:** Baseline collection + lightweight scenario walk-through
**Environment:** Linux desktop, RTX 4050 6GB, 16GB RAM, PipeWire active, Pulse inactive
**Build:** Desktop release mode, voice features default (feature-off piper-rs, whisper-rs primary STT code present)
**Status:** IN PROGRESS

#### Known Baseline (from proxy benchmarks)

**Latency baseline (v2 pipeline, feature-off, warm runs, no CPU stress):**
- Barge-in stop latency: avg **585–590ms**, p95 **588–594ms**, worst **596–620ms** ⚠️ (>500ms target)
- Happy-path turn latency: avg **604–620ms**, p95 **612–619ms**, worst **619–646ms** ⚠️
- Partial streaming: avg **635–640ms**
- Under CPU stress (synthetic load): worst-case **1400–2200ms** 🔴 (degradation severe)

**Current bottleneck:** runtime polish + live percentile validation (native `piper-rs` path is now unblocked via local vendor patch, but real-device TTFA traces are still pending).

**TTFA expectation (realistic):** ~1–1.2s in practice (cold CLI TTS load + synthesis time), not <800ms target.

**Runtime state:** All 35 v2 unit tests passing. Default-path stable. Feature-on `voice-piper-rs` compile/tests now pass with patched local vendor crate; live-device percentile capture still required.

#### Known Issues (from prior validation)

🔴 **CRITICAL:**
- None reported yet (initial audit phase)

🟠 **HIGH:**
- barge-in stop latency >500ms (not assistant-grade)
- High CPU load causes p95 tail to widen 2–3x
- Long-running sessions (2+h) untested for stability

🟡 **MEDIUM:**
- Partial transcript stability under heavy load
- Device switching (headphone/speaker) behavior untested
- False activation risk unknown
- Recovery from playback device disconnection untested

**Next steps:** Run live scenarios to surface real UX issues.

---

## 4. UX FAILURES

*Behaviors that make interaction feel awkward/slow/robotic*

| Issue | Frequency | Severity | Details | Impact | Workaround |
|-------|-----------|----------|---------|--------|-----------|
| No "thinking" indicator during LLM processing | Every response | 🟡 MEDIUM | 1–2s silence while LLM generates; user unsure if listening | Feels stuck or broken | Rely on UI spinner (may not be visible) |
| TTFA >1s even on simple queries | Every turn | 🟠 HIGH | Noticeable delay before response begins | Kills conversational feel | None; architectural limit |
| False busy rejections (overlap corner cases) | Occasional | 🟡 MEDIUM | Edge cases may reject legitimate quick re-invocation | Annoying error state | Wait & retry manually |
| Partial transcript churning on heavy CPU | Heavy load | 🟡 MEDIUM | UI partial updates flicker/redraw excessively | Distracting, looks janky | Reduce background load |
| No error message for failed TTS | Occasional | 🟠 HIGH | Playback fails silently; user gets no feedback | Confusing ("did it work?") | Check system audio, restart |
| Headphone unplug mid-turn | Edge case | 🟠 HIGH | Device switch during synthesis may cause playback loss | Audio cuts out | Plug back in or restart voice mode |
| Wake word sensitivity unknown | Unknown | 🔴 CRITICAL | No data on false activation rate | Could trigger unexpectedly | Untested; deferred wake feature |

---

## 5. INTERRUPTION FAILURES

*Barge-in / cancellation issues that violate expectations*

| Issue | Frequency | Severity | Details | Impact | Reproducible |
|-------|-----------|----------|---------|--------|---|
| Barge-in audio lag | Every barge-in session | 🔴 CRITICAL | TTS playback continues ~500ms after spoken interrupt | User says "stop", hears assistant for 500ms more | Yes, consistent |
| Playback not invalidated | Unknown | 🟠 HIGH | Stale chunk may resume after new command | Conversation feels confused | Needs testing |
| Rapid re-interrupt causes state confusion | Unknown | 🟠 HIGH | Back-to-back interrupts may cause FSM race | Unpredictable behavior | Needs testing |
| Restart sensitivity (speech detection during barge-in) | Unknown | 🟡 MEDIUM | Does new input trigger new STT capture immediately? | May cause overlapping partial captures | Needs testing |

---

## 6. ASSISTANT FEEL ISSUES

*Behaviors that don't match human assistant expectations*

| Issue | Category | Severity | Details | Expectation vs. Reality |
|-------|----------|----------|---------|------------------------|
| Barge-in latency | Responsiveness | 🔴 CRITICAL | 585ms response feels like lag, not immediate attention | Assistant should interrupt within <200ms; KRIA takes >500ms |
| CLI subprocess startup | Latency | 🟠 HIGH | Cold TTS load (~100–300ms) plus synthesis adds up | Should feel like thought-to-speech, not command-to-subprocess |
| Partial transcript flicker | UX stability | 🟡 MEDIUM | Heavy CPU load may cause UI redraw churn | Should feel calm, word-by-word streaming |
| Interruption audio lag | Interruption quality | 🔴 CRITICAL | TTS continues playing 500ms after barge-in spoken | Should stop immediately on new speech |
| Long pause during thinking | Feedback | 🟡 MEDIUM | No audio/visual feedback during LLM inference | Should have "thinking" indicator or subtle audio cue |
| No spoken confirmation | Acknowledgment | 🟡 MEDIUM | Voice commands execute silently; no "got it" | Human assistants confirm understanding |
| Busy state rejection | Error handling | 🟡 MEDIUM | If turn overlaps, just fails; no retry suggestion | Should say "let me finish" or suggest retry |

---

## 7. LATENCY OBSERVATIONS

*Timing measurements and feel in context*

| Scenario | Measured | Acceptable? | Feels Like | Notes |
|----------|----------|------------|-----------|-------|
| Barge-in response time | 585–590ms avg, 620ms worst | ❌ No | Sluggish, delayed | Exceeds <500ms human assistant target |
| STT first-partial latency | ~635–640ms avg | ❌ Slow | Detectable pause | CLI subprocess load time |
| LLM first-token latency | Unknown | ? | ? | Depends on model/prompt size |
| TTS first-audio latency | ~100–300ms | ? | Acceptable | CLI piper subprocess; feature-on path blocked |
| Total TTFA (realistic) | ~1–1.2s | ❌ Not assistant-grade | Feels robotic | CLI fallback; <800ms target unmet |
| Interruption stop latency | ~585ms (equals barge-in) | ❌ Too slow | Feels laggy | Audio continues after barge-in spoken |
| CPU stress worst-case (p95) | ~1400–2200ms | 🔴 Unacceptable | Freezing | Multitasking scenarios feel very sluggish |

---

## 8. LONG SESSION STABILITY

*Extended conversation behavior (>30min)*

| Issue | Occurred | Severity | Details | Session Impact |
|-------|----------|----------|---------|---|
| (TBD) | | | | |

---

## 9. CPU/RAM OBSERVATIONS

*Resource behavior during normal operation*

| Metric | Idle | During Input | During Synthesis | Peak | Notes |
|--------|------|----------|---------|------|-------|
| CPU % | TBD | TBD | TBD | TBD | |
| RAM (MB) | TBD | TBD | TBD | TBD | |
| GPU VRAM | TBD | TBD | TBD | TBD | |

---

## 10. AUTOMATION RISKS

*Cases where automated actions could cause problems*

| Risk | Likelihood | Severity | Details | Mitigation | Safe? |
|------|-----------|----------|---------|-----------|-------|
| False wake-word activation | TBD | ? | | | |
| Misheard dangerous command | TBD | ? | | | |
| Unintended file operation | TBD | ? | | | |

---

## 11. HALLUCINATION RISKS

*LLM generating false information or nonsensical actions*

| Case | Observed | Severity | Details | Pattern |
|------|----------|----------|---------|---------|
| (TBD) | | | | |

---

## 12. DANGEROUS EXECUTION CASES

*Commands that could be misinterpreted or cause harm*

| Command Class | Risk | Observed Issues | Safeguard Status | Notes |
|------|------|---------|---------|-------|
| File deletion | TBD | | | |
| Sensitive data exposure | TBD | | | |
| Unintended API calls | TBD | | | |

---

## 13. USER TRUST FAILURES

*Moments when user would stop trusting KRIA*

| Issue | Occurred | Frequency | Impact | Recovery |
|-------|----------|-----------|--------|----------|
| (TBD) | | | | |

---

## 14. VOICE NATURALNESS ISSUES

*TTS quality, pacing, intonation, emotion mismatch*

| Issue | Frequency | Severity | Details | Detectable by Ear |
|-------|-----------|----------|---------|---------|
| (TBD) | | | | |

---

## 15. WORKFLOW SUCCESS CASES

*When KRIA genuinely feels useful / faster than GUI*

| Workflow | Task | Feeling | Time Saved | Repeatability |
|----------|------|---------|-----------|---|
| (TBD) | | | | |

---

## 16. WORKFLOW FAILURE CASES

*When voice interaction slows things down or feels awkward*

| Workflow | Task | Failure Mode | Time Lost | Why Voice Failed |
|----------|------|--------|----------|---------|
| (TBD) | | | | |

---

## 17. DEVICE ISSUES

*Hardware integration, audio capture, playback quirks*

| Issue | Device | Frequency | Severity | Details | Reproducible |
|-------|--------|-----------|----------|---------|---|
| (TBD) | | | | | |

---

## 18. PIPEWIRE/PLAYBACK ISSUES

*Audio system integration, device switching, recovery*

| Issue | Frequency | Severity | Details | Workaround | Reproducible |
|-------|-----------|----------|---------|-----------|---|
| (TBD) | | | | | |

---

## 19. RECOVERY FAILURES

*When runtime errors aren't recovered gracefully*

| Error | Frequency | Severity | Recovery Time | Silent vs. Visible | Dangerous |
|-------|-----------|----------|---|---|---|
| (TBD) | | | | | |

---

## 20. FALSE ACTIVATION EVENTS

*Wake word triggered when not intended*

| Context | Frequency | Severity | Audio Details | Impact |
|---------|-----------|----------|---------|--------|
| (TBD) | | | | |

---

## 21. BARGE-IN QUALITY

*Interruption feel, latency, completion behavior*

| Aspect | Measured | Quality | Notes |
|--------|----------|---------|-------|
| Latency to stop | TBD | | |
| Stop immediacy | TBD | | |
| Resume smoothness | TBD | | |
| Restart sensitivity | TBD | | |

---

## 22. THINKING/WAITING UX

*Visual/audio feedback while assistant is processing*

| State | Current Feedback | Quality | Improvement |
|-------|---------|---------|---------|
| Listening | TBD | | |
| Thinking (LLM) | TBD | | |
| Speaking | TBD | | |
| Interrupted | TBD | | |

---

## 23. REAL PRODUCT VALUE

*Honest assessment: is KRIA actually useful?*

**Value Proposition Assessment:**

| Dimension | Rating | Evidence | Blocker |
|-----------|--------|----------|---------|
| Faster than GUI for common tasks? | ❌ No | TTFA ~1–1.2s vs. 100–200ms for mouse click | Latency kills speed advantage |
| Feels assistant-like? | ❌ No | 585ms barge-in latency + continued playback feels robotic | Responsiveness too slow for conversational feel |
| Reliable enough to depend on? | ⚠️ Partial | Core v2 pipeline passes tests; live validation incomplete | Unknown device/recovery edge cases |
| Improves workflow or distracts? | ⚠️ Unknown | Preliminary assessment: adds friction more than speed | Needs extended real-world testing |
| Would you recommend to others? | ❌ No (currently) | Not assistant-grade; too slow for daily use as primary interface | Requires native piper-rs or architecture rethink |

**Honest Verdict (Preliminary 2026-05-13):**

KRIA is **architecturally sound but latency-limited**. The v2 runtime orchestration is correct (tests passing, interruption logic solid), but the **CLI subprocess TTS path is the ceiling**. Users will notice:

1. **Initial response latency is obvious** (~1s+ for simple queries)
2. **Barge-in feels sluggish** (600ms response vs. 150ms human assistant)
3. **Audio continues after interruption** (500ms+ lag to stop)
4. **No feedback during thinking** (confusing 1–2s silences)

**Verdict:** Not ready for daily assistant use. Useful as a tool for specific workflows (file search, terminal help), but will **not replace mouse/keyboard** for speed. Requires either:
- Native piper-rs unblocked + TTFA <800ms achieved, OR
- Different architecture (persistent inference, speculative synthesis) — but user explicitly forbade this

**Recommendation:** Mark as **experimental/research tool** until latency target is met.

---

## 24. TOP HIGH-IMPACT FIXES

*Ranked by effort + impact on daily usability*

| Fix | Effort | Impact | ROI | Owner | Status |
|-----|--------|--------|-----|-------|--------|
| **Complete live feature-on TTFA capture (`KRIA_VOICE_LIVE=1`)** | 2–3h | 🔴 CRITICAL | Converts implementation completion into real assistant-grade evidence | Core | In progress |
| **Add "thinking" audio/visual indicator during LLM inference** | 1–2h | 🟠 HIGH | Removes silent-pause confusion; improves perceived responsiveness | UX | Easy |
| **Improve barge-in stop latency from 585ms to <300ms** | 2–3h | 🟠 HIGH | Requires reducing TTS chunk size or presynthesis strategy (user rejected) | Core | Blocked on architecture |
| **Add error feedback for playback failures** | 1h | 🟡 MEDIUM | Tells user what went wrong (audio device, permission, etc.) | UX | Easy |
| **Test & harden device switching (headphone/speaker)** | 2h | 🟡 MEDIUM | Improves reliability in real multitasking scenario | Testing | Easy |
| **Measure false activation rate for wake word** | 1h | 🟡 MEDIUM | Determine if feature can be enabled safely | Testing | Easy |
| **Profile CPU load under heavy background work** | 1h | 🟡 MEDIUM | Understand degradation profile; set realistic expectations | Testing | Easy |

**Honest assessment:** Native TTS compile blocker is closed; now the remaining gap is **real-world latency/UX evidence** (live percentile capture + interruption smoothness under load). Easy polish fixes improve feel, but assistant-grade claim still needs live TTFA proof.

---

## 25. REJECTED IDEAS

*Architectural / feature ideas that don't belong in this phase*

- Do not implement speculative pre-synthesis or parallel synthesis branches to mask latency
- Do not add emotional AI, voice personality, or conversational filler
- Do not redesign orchestration to claim assistant-grade without proving on live path
- Do not add full duplex/AEC as workaround for latency issues
- Do not add autonomous background agents or hidden task loops
- Do not implement unbounded conversation memory (context explosion)
- Do not add multimodal (vision/text) until voice is solid
- Do not optimize for benchmark scores; optimize for 90th-percentile weekday use

---

## 26. VALIDATION SCHEDULE

### Phase 1: Initial Audit (2026-05-13)
- [ ] Establish baseline TTFA, latency ranges
- [ ] Identify top 5 UX friction points
- [ ] Classify severity of known issues
- [ ] Map workflows where voice helps vs. hurts

### Phase 2: Deep Validation (2026-05-15)
- [ ] Run coding workflows (1+ hours)
- [ ] Run browser workflows (1+ hours)
- [ ] Run interruption-heavy tests (30+ mins)
- [ ] Measure CPU/RAM under load
- [ ] Test device recovery scenarios

### Phase 3: Stress & Edge Cases (2026-05-17)
- [ ] Long session (2+ hours)
- [ ] Noisy environment simulation
- [ ] Rapid-fire commands (10+ per min)
- [ ] Dangerous command patterns
- [ ] Recovery from failure states

### Phase 4: Synthesis & Recommendations (2026-05-20)
- [ ] Honest product verdict
- [ ] Top blockers vs. polish items
- [ ] Release readiness assessment
- [ ] Recommend next phase (Phase 3 or stabilization)

---

## 27. NOTES & EMERGING PATTERNS

*Key insights, trends, unexpected behaviors*

(TBD as validation progresses)

---

*This document is the permanent source of truth for KRIA voice runtime real-world validation.*
*Update daily as validation progresses. Be brutally honest.*

*End of document.*
