# KRIA Voice Runtime & Streaming STT — **FINAL** architecture specification (v1.0)

**Status:** Implementation-grade · **Architecture-loop CLOSED** for v1.0 — further changes require ADR and version bump.

**Audience:** Engineers implementing voice v2, sidecar STT, and UI transcript layers.

**Hardware target (normative):** RTX **4050 Laptop 6 GB VRAM**, **16 GB** system RAM, **local-first**, Linux primary.

**Last updated:** 2026-05-14

---

## 0. Document control

| Item | Rule |
|------|------|
| **Normative language** | Sections marked **SHALL** / **MUST NOT** are binding for v1.0 implementations claiming compliance. |
| **Anti-goals** | No speculative planner execution, no autonomous tool calls before commit, no multi-agent STT routers, no unbounded adaptive ML without explicit caps in this doc. |
| **Change process** | Any deviation SHALL be recorded as **ADR** + bump doc to **v1.1**. |

---

## 1. Scope & assistant UX goals

### 1.1 In scope

- Streaming partial hypotheses, utterance commit, optional Whisper refine, UI reconciliation, sidecar IPC, cancellation, backpressure, bounded CPU/GPU behaviour.

### 1.2 Out of scope (v1.0)

- Cloud ASR, continuous speaker diarisation for meetings, custom silicon tuning, proprietary parity claims.

### 1.3 Assistant UX goals (non-normative but design drivers)

**Trust > raw ms:** stable partials, bounded rollback, no silent world mutation before `UtteranceCommitted`.

---

## 2. Reference hardware & capacity principle

| Resource | Principle |
|----------|-----------|
| **6 GB VRAM** | **MUST NOT** assume two large GPU residents (LLM + large STT) without **VoiceBorrow** FSM (§15). Default: **CPU streamer + short GPU Whisper** per committed utterance. |
| **16 GB RAM** | All PCM ring buffers SHALL have **hard byte caps** (§8). |

---

## 3. Target runtime architecture (frozen for v1.0)

```text
Mic → (optional AEC) → VAD (profile from §13) → bounded chunk fanout
          │
          ├─► Streaming ASR sidecar (CPU INT8 ONNX) ──► partials ──► Transcript Authority FSM (§6)
          │
          └─► On UtteranceCommitted ──► Whisper medium multilingual (CUDA), ≤1 decode/utterance
                    └──► reconcile (§7) ──► agent + UI committed tier
```

**Whisper rolling partials (`KRIA_WHISPER_PARTIAL=1`) MUST NOT** be used as a strategic streaming path; **MAY** remain debug-only.

---

## 4. Runtime invariants (SHALL hold)

| ID | Invariant |
|----|-----------|
| R1 | Every voice turn has exactly one **`session_id`** (UUID string). All IPC messages **SHALL** carry it. |
| R2 | **`generation`** (u64, host-owned, monotonic per `session_id`) **SHALL** increment on any cancel/restart; consumer **MUST** drop partials where `generation` mismatches current. |
| R3 | **No** LLM tool execution, filesystem mutation, or network side-effect **SHALL** occur before **`UtteranceCommitted`** except **whitelist** local actions (§14). |
| R4 | Visible user transcript **SHALL** be owned by exactly one state at a time (§6). |
| R5 | Sidecar process **SHALL** be supervised (§10); stale IPC **SHALL** be cleaned on restart. |
| R6 | PCM ingress to sidecar **SHALL** respect **backpressure** (§8); unbounded `mpsc::unbounded` for STT path **MUST NOT** be used for sidecar-bound audio in compliant builds. |

---

## 5. IPC v0.1 — streaming STT sidecar (normative)

### 5.1 Transport

- **SHALL:** `AF_UNIX` **stream** socket, path `${XDG_RUNTIME_DIR}/kria/stt-streamer.sock` (fallback `/tmp/kria-stt-${UID}.sock`).
- **MAY:** stdio NDJSON for bring-up only; **production compliance** requires socket v0.1.

### 5.2 Framing

- **SHALL:** messages are **length-prefixed JSON**: `u32_be_len` + UTF-8 JSON body. Max body **256 KiB**. Oversized **MUST** close connection with error.

### 5.3 Session lifecycle

| Phase | Host → Sidecar | Sidecar → Host |
|-------|----------------|------------------|
| Open | `{"type":"hello","proto":"0.1","session_id":"UUID","sample_rate":16000,"generation":0}` | `{"type":"hello_ack","capabilities":["partial"],"max_chunk_samples":4000}` |
| Stream | `{"type":"audio","session_id","generation","seq":N,"pcm":[f32,...]}` — **SHALL** `len(pcm) ≤ 4000` (250 ms @ 16 kHz mono) | `{"type":"partial","session_id","generation","seq":N,"text":"...","stable":false}` |
| Heartbeat | `{"type":"ping","ts_ms":...}` every **5 s** during active session | `{"type":"pong","ts_ms":...}` within **1 s** |
| Close | `{"type":"bye","session_id","generation"}` | `{"type":"bye_ack"}` then sidecar **SHALL** exit **0** within **500 ms** |
| Error | — | `{"type":"error","code":"...","fatal":bool}` |

### 5.4 Reconnect & stale cleanup

- Host **SHALL** treat **3 consecutive** missing `pong` or socket error as **fatal** for that sidecar instance: **SIGKILL** child, **unlink** socket path, **increment** `generation`, **sleep** backoff (§10), respawn.
- Sidecar on `hello` with unknown `session_id` **SHALL** reset internal decoder state.
- On `generation` change mid-session, sidecar **SHALL** flush internal streaming buffers and **MUST NOT** emit further `partial` for old generation.

### 5.5 Bounded queue semantics (sidecar ingress)

| Parameter | Value |
|-----------|-------|
| **Max unprocessed audio bytes** | **8_388_608** (8 MiB ≈ 131 s f32 mono @16kHz theoretical max buffer; implementer **SHALL** enforce drop-oldest or **pause** host capture when exceeded — **prefer pause** to avoid word loss). |
| **Max pending JSON messages (host→sidecar)** | **64** — full: host **SHALL** block or drop **oldest non-audio** (never drop `audio` without policy: **block** with timeout 50 ms then telemetry `stt_backpressure`). |

### 5.6 Shutdown semantics

- **SIGTERM** to sidecar: flush `partial` if any, send `bye_ack`, exit 0.
- **SIGKILL**: host **MUST** assume decoder state lost; new `hello` required.

---

## 6. Transcript authority lifecycle (single source of truth)

### 6.1 States (UI-visible string ownership)

| State | Owner of visible caption | Enter | Exit |
|-------|-------------------------|-------|------|
| **S0_Idle** | *(empty)* | no session | `SpeechStart` |
| **S1_Speculative** | **Streamer** partial text only | first `partial` with `stable=false` | `stable=true` **OR** timer **T_stab=220 ms** same prefix |
| **S2_Stabilizing** | **Streamer** (frozen prefix + volatile tail) | `stable=true` from sidecar **or** timer satisfied | `UtteranceCommitted` (VAD commit) |
| **S3_Committed** | **Streamer** snapshot `Ts` frozen | VAD commit + debounce complete | Whisper refine result `W` available |
| **S4_RefinedFinal** | **Reconciled** string per §7 | after §7 applied | next turn `generation++` **OR** user `UndoRefine` |

**MUST NOT** display Whisper text directly to user before **S4** unless §7 chooses **pass-through** (Whisper prefix-extends `Ts`).

### 6.2 Authority transition rules

1. **S1→S2:** sidecar sets `stable:true` **OR** host applies **prefix hold** rule: same word-prefix for **2** consecutive partials ≥ **120 ms** apart.  
2. **S2→S3:** VAD `EndCandidate` + tail padding **SHALL** complete; host emits `UtteranceCommitted {Ts, session_id, generation}`.  
3. **S3→S4:** only reconciliation engine (§7) mutates visible string; **atomic** UI swap.

---

## 7. Reconciliation algorithm — Whisper refine vs streamer (normative, bounded, deterministic)

**Inputs:** `Ts` (string, committed streamer snapshot), `W` (Whisper output string).  
**Normalisation:** Unicode NFKC; collapse internal whitespace to single space; trim.

**Procedure (SHALL implement in order):**

1. If `norm(W) == norm(Ts)` → visible unchanged; `reconcile=identical`.  
2. If `norm(W)` **starts with** `norm(Ts)` as substring at index 0 (prefix) → visible = `W`; `reconcile=prefix_extend`. **Rollback cap** §7.1 still applies to **character growth** beyond **+120** chars → cap extra suffix preview with "…" **MUST NOT** apply to agent-bound string (agent gets full `W`).  
3. Else tokenise `Ts` and `W` on whitespace → arrays `t[]`, `w[]` with **max 64 tokens** each (truncate tail with marker token `#TRUNC`).  
4. Compute **word-level Levenshtein** `d` on truncated arrays. Let `r = d / max(len(t),1)`.  
   - If `r ≤ 0.25` **and** `|len(W)-len(Ts)| ≤ 40` chars → visible = `W`; `reconcile=replace_bounded`.  
   - Else → visible **remains `Ts`** for user; agent payload **SHALL** include `{user_visible: Ts, whisper: W, reconcile: rejected}`; `reconcile=reject`.

### 7.1 Rollback / flicker caps (UI)

| Cap | Value |
|-----|-------|
| Max visible character change in one atomic swap | **min(40, ceil(0.15 × max(len(Ts),len(W))))** |
| If rule (2) would exceed cap | Apply prefix_extend only up to cap; remainder as tooltip / “tap to expand” **MAY** be deferred to v1.1 |

### 7.2 Flicker prevention (SHALL)

- **MUST NOT** emit more than **15** visible transcript updates per second (host coalescer).  
- **MUST NOT** show empty string flashes: suppress updates where `trim(text).is_empty()`.

---

## 8. Runtime backpressure policy (normative numbers)

| Resource | Cap | On exceed |
|----------|-----|-----------|
| **Utterance duration** | **120 s** hard stop → force `UtteranceCommitted` + telemetry `utterance_truncated` | |
| **Host→sidecar pending audio** | **8 MiB** (see §5.5) | block send ≤50 ms then `voice:degraded` telemetry |
| **Partials queue (host UI)** | **64** entries | coalesce / drop intermediate |
| **Coalescer output rate** | **4–15 Hz** adaptive: floor **4 Hz**, ceiling **15 Hz** (lengthen when prefix-stable ≥ 120 ms) | lengthen when stable |
| **Overload** | CPU process `%` > **90** for **2 s** (optional host metric) | raise coalesce ceiling to **200 ms**, **skip** optional refine if policy flag `fast_mode` |

---

## 9. Thread & scheduler budgets (normative guidance)

| Subsystem | Budget rule |
|-----------|--------------|
| **ONNX Runtime intra-op** | **SHALL** set `intra_op_num_threads` ≤ **2** for streamer sidecar on ≤8P-cores; ≤**4** on >8P. |
| **ONNX inter-op** | **1** unless profiling shows headroom. |
| **Whisper decode** (`spawn_blocking` pool) | **SHALL** cap concurrent Whisper jobs = **1** per process (existing mutex pattern). |
| **Tokio blocking pool** | Whisper + heavy IO **SHALL NOT** exceed **512** concurrent blocking tasks from voice alone (use dedicated pool if needed). |
| **UI thread** | Audio/visual updates **SHALL** go through coalescer; **MUST NOT** attach raw per-chunk handlers. |

---

## 10. Sidecar supervision & restart (complete policy)

| Parameter | Value |
|-----------|-------|
| **Restart backoff** | base **100 ms**, ×2 capped **5 s** |
| **Max restarts** | **5** per **60 s** rolling window |
| **After max exceeded** | **MUST** disable streamer for **120 s**, fall back to **Whisper-only** path + user toast `voice_stt_degraded` |
| **Stale socket** | **unlink** before bind; **fcntl** close-on-exec |
| **Health** | heartbeat §5.3 |

---

## 11. Cancellation & stale partial invalidation

- On `CancellationToken` cancel for turn: host **SHALL** `generation += 1`, send sidecar `{"type":"cancel","session_id","generation"}` (optional if socket dead), **clear** partial queue, UI → **S0_Idle** or listening state per pipeline FSM.  
- **MUST NOT** apply any `partial` with `generation` < current.

---

## 12. Device recovery semantics (audio)

| Event | Behaviour |
|-------|-----------|
| **Mic disconnect** | Pause capture; UI `mic_lost`; **SHALL NOT** increment `generation` unless user presses Retry. On reconnect same device → resume. |
| **Bluetooth switch** | If device ID changes → **new** `session_id` **recommended**; if same session, **generation++** **SHALL**. |
| **Fallback selection** | **SHALL** use CPAL default enumeration order: prefer last working device from prefs JSON in `~/.kria/device.json` if present. |
| **Recovery timeout** | If no device **30 s** → exit voice session to sleep + telemetry. |

**VAD profiles (§13)** — **SHALL** be selectable: `quiet` | `normal` | `noisy` (fixed parameter sets in config; **MUST NOT** require ML self-adaptation in v1.0).

---

## 13. VAD environment profiles (fixed tables)

| Profile | Silero threshold (if used) | Min speech ms | Tail padding ms | Notes |
|---------|---------------------------|---------------|-----------------|-------|
| `quiet` | 0.55 | 120 | 280 | stricter end |
| `normal` | 0.50 | 150 | 400 | default |
| `noisy` | 0.45 | 200 | 550 | fewer false ends |

Host **SHALL** expose user toggle; auto-switch **MUST NOT** in v1.0 (avoid “smart” creep).

---

## 14. Early intent (whitelist only)

**Before `UtteranceCommitted`, host MAY:**

- Update speculative UI caption.  
- Execute: **`StopTts`**, **`CancelTurn`**, **`MicMute`**, **`VolumeDown`** (local, reversible).

**MUST NOT:** LLM completion, tool calls, file writes, network.

---

## 15. GPU lease FSM (3-state, bounded)

| State | Meaning |
|-------|---------|
| `LlmPrimary` | LLM holds GPU lease as today. |
| `VoiceBorrow` | Voice requests **exclusive** Whisper CUDA window; orchestrator **SHALL** lower `ngl` or pause LLM per existing watchdog policy — **exact ngl drop table is orchestrator-owned**; this doc only requires **declared** transition hooks. |
| `Recovering` | Existing GPU lease recovery; **MUST NOT** start new Whisper GPU job until idle or timeout **30 s**. |

**MUST NOT** run Whisper CUDA + full LLM layers without passing through `VoiceBorrow` policy on **≤6 GB** cards.

---

## 16. Evaluation methodology (reproducible)

### 16.1 Hinglish set

- **SHALL** maintain **≥200** utterances, Latin script mixed Hindi–English, recorded **16 kHz mono**, SNR variants: clean, +keyboard noise, +fan loop.  
- **Labels:** single reference transcript string.

### 16.2 Metrics

| Metric | Definition |
|--------|------------|
| **WER** | Word error rate vs reference on **final** string after §7. |
| **Partial stability** | Fraction of partial updates that are **prefix extensions** of previous (target ≥ **0.85** on English subset). |
| **Commit latency** | `speech_end` → `UtteranceCommitted` p50/p95. |
| **Refine latency** | commit → `S4` p50/p95. |
| **Flicker rate** | count of UI updates where visible string **edit distance** > **6** chars vs previous / total updates (target ≤ **0.05**). |
| **Rollback rate** | fraction of turns where §7 yields `rejected` (track for tuning). |

### 16.3 TTFA (voice output)

- Already tracked in `VoiceMetrics` / tier budgets — **SHALL** log alongside STT metrics.

---

## 17. Observability (lightweight)

**SHALL** emit structured events (tracing or JSONL): `stt_session_start`, `stt_partial`, `stt_commit`, `stt_refine_done`, `stt_reconcile_result`, `stt_sidecar_restart`, `stt_backpressure`, `audio_device_lost`.

**MAY:** optional Tauri debug panel subscribing to same bus — not blocking v1.0.

---

## 18. Phased implementation — **frozen acceptance**

| Phase | Deliverable | Acceptance (all SHALL pass) |
|-------|---------------|------------------------------|
| **P0** | Metrics schema §16 + §17 events + policy §14 code review | Can produce JSONL trace for one session with all timestamps. |
| **P1** | Whisper CUDA medium + VRAM-aware defaults | p95 refine latency **≤** baseline large-CPU on reference clips by **≥40%**; **0** tool calls before commit in tests. |
| **P2** | Sidecar IPC **§5** compliant + §6–§7 in UI host | Sidecar passes conformance harness (fake host) + restart storm test §10. |

**REJECTED for v1.0:** speculative planner, TTS audio backchannel, ML VAD auto-profile, continuous LID model, dynamic model farms.

---

## 19. Issue disposition — final review table (this review round)

| # | Issue | Verdict | Fix optimality | Final prescription | When |
|---|-------|---------|----------------|-------------------|------|
| 1 | IPC abstract | **VALID** | Original “add IPC” correct | **§5** is now binding | **NOW** (spec); **P2** (code) |
| 2 | Single source of truth | **VALID** | Good | **§6** FSM | **P2** |
| 3 | Bounded memory vague | **VALID** | Good | **§5.5 + §8** numbers | **P2** |
| 4 | Sidecar restart incomplete | **VALID** | exponential backoff correct | **§10** | **P2** |
| 5 | Stale partials after cancel | **VALID** | `generation` correct | **§4 R2, §11** | **P2** |
| 6 | Diff algorithm ambiguous | **VALID** | suffix-only was ambiguous | **§7** word-Levenshtein bounded | **P2** |
| 7 | Audio hotplug | **VALID** | Good | **§12** | **LATER** (P3 acceptable) |
| 8 | VAD profiles | **VALID** | Good | **§13** fixed tables | **NOW** config |
| 9 | Thread budget | **VALID** | Good | **§9** | **P2** |
| 10 | TTFA UX tiers | **OVERSTATED** as blocker | SLO labels useful | Map to existing tier TTFA budgets + **degraded** flag in telemetry | **LATER** |
|11 | Eval methodology | **VALID** | Good | **§16** | **P0** ongoing |
|12 | Dashboard | **LOW** | optional | **§17** | **LATER** |

---

## 20. Consolidated disposition — prior review cycles (abridged)

- **Whisper rolling partials:** **REJECT** strategic use.  
- **Dual GPU fat residents:** **REJECT** on 6 GB without `VoiceBorrow`.  
- **Cascade without §6–§7:** **REJECT**.  
- **InterruptionIntentClass framework:** **REJECT**; small enum **LATER** optional.  
- **Three-tier confidence UI:** **MERGED** into §6 **S1/S2**.  
- **Speculative execution:** **REJECT** (§14 whitelist).

---

## 21. **FINAL GATE — Is this document implementation-grade?**

### Answer: **YES** — with a **narrow blocking list**

This document is **stable enough to stop architecture-loop refinement** and begin **focused implementation** for:

| Phase | Safe to start **immediately** |
|-------|------------------------------|
| **P0** | Instrumentation §16–§17, whitelist policy §14, VAD profiles §13, reconciliation **unit tests** from §7 fixtures. |
| **P1** | Whisper CUDA medium path, VRAM tier defaults, `UtteranceCommitted` gating audits. |
| **P2** | Sidecar **only** after internal **IPC conformance harness** (golden host) against §5 is green — **do not** ship socket parser without harness. |

### **ONLY** remaining items that would **block declaring “done”** for full streaming assistant (not blocking starting P0/P1):

1. **IPC conformance harness** automated tests (not spec text — **code**).  
2. **UI integration** of §6 state machine (can stub until P2).  
3. **§12 device recovery** — **MAY** ship P2 without Bluetooth edge cases if documented “known limitation” until P3.

### Explicit statement

> **Architecture refinement for v1.0 is CLOSED.** Subsequent work SHALL be implementation, tests, and measured tuning against §16. Any architecture change SHALL require **ADR + v1.1** of this document.

---

## Appendix A — Code map (informative)

- `crates/kria-core/src/voice/v2/stt.rs`, `pipeline.rs`, `vad.rs`, `tier.rs`  
- `crates/kria-core/Cargo.toml` — `voice-whisper-cuda`, `voice-whisper-rs`  
- GPU orchestration: `crates/kria-core/src/llm/orchestrator/*`, `resource/gpu_lease.rs`

---

## Appendix B — Glossary

| Term | Meaning |
|------|---------|
| `UtteranceCommitted` | Host event after VAD end + padding + debounce; freezes `Ts`. |
| `generation` | Monotonic invalidation counter for partial stale drops. |
| `Ts` / `W` | Streamer committed text vs Whisper output (§7). |

---

*End of ENHANCED_STT.md v1.0*
