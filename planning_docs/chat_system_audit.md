# KRIA Chat System — Production Audit (Phase A: Evidence)

Status: evidence-based investigation. Every claim cites code (`file:line`). Opinions are
marked OPINION. This is the input to the redesign + implementation.

## 0. How a chat turn actually flows today

### Normal (LLM) turn
1. `ChatView.handleSubmit` → `sendMessage(text)` (`ui/src/stores/app.ts:1536`).
2. `sendMessage`: if `isThinking("assistant")` → prompt is **queued** (`enqueueScopedPrompt`), not sent.
   Else `ensureScopedSessionActive` → append optimistic `userMsg` → `setScopedThinking(true)` →
   `armAssistantThinkingWatchdog()` → `invoke("send_message" | "send_manual_tool_message")`.
3. Frontend does **not poll**. It `listen()`s to Tauri push events:
   `agent:thinking|token|tool_call|tool_result|done`, plus `gui_cognition:event`
   (`app.ts:3465`+, `app.ts:3800`).
4. Backend streams `agent:token` incrementally → assistant bubble grows. `agent:done` →
   `setThinking(false)` + `loadSessions()` + `drainScopedPromptQueue` (`app.ts:3492`).

### GUI Cognition turn
1. Same `sendMessage` path; `manualProfile.mode_id = "gui_cognition"` → `send_manual_tool_message`.
2. Backend `chat.rs:~355` detects `is_gui_cognition_override()` → runs the turn in a **detached
   spawned task** and returns `{status:"processing"}` immediately. A `GuiCognitionDoneGuard`
   emits `{prefix}:done` on every exit.
3. Task → `desktop_gui_cognition_command_capture_streamed(msg, state, None, "agent", ...)` →
   `run_gui_cognition_v2`. NOTE: `session_id_override = None` → backend resolves
   `app_state.current_session_id` itself.
4. `run_gui_cognition_v2`: emits `agent:thinking` up front, then a bounded loop
   `observe → decide → gate → execute → re-observe` (max_steps 12, no_progress_limit 2,
   QwenBrain decide timeout 20s, observe timeout default 300s). Per step it emits
   `gui_cognition:event {type:"V2Step", step_index, action, action_detail, ok, ...}`.
5. End: the batch (`agent:thinking`,`{prefix}:token`=reply,`{prefix}:tool_result`,`{prefix}:done`)
   is returned and emitted by `chat.rs` AFTER the turn completes.

## 1. CRITICAL — V2 ↔ frontend event contract is broken (root cause of "empty chat")

**Evidence**
- Backend V2 emits only `{type:"V2Step", ...}` and an initial `agent:thinking`
  (`commands/gui_cognition.rs`, `run_gui_cognition_v2`). It never emits `TurnStarted` or any
  rich lifecycle event.
- Frontend gate: `shouldAcceptEnvelope` (`guiCognitionSession.ts:419`) returns `true` ONLY for
  `TurnStarted`; otherwise `if (!state.active.turnId) return false;`. The reducer
  `handleGuiCognitionEvent` (`:448`) starts a session ONLY on `TurnStarted` (`:454`), and its
  `switch` handles V1 vocabulary (`ObservationStarted`, `PlanProposed`, `ExecutionStarted`,
  `VerificationCompleted`, `HitlRequired`, `WorkflowRunCompleted`, …). There is **no `V2Step`
  case** and **no `TurnStarted` from V2**.

**Consequence (verified by tracing):** under V2 (the current default) every `gui_cognition:event`
is rejected → `activeGuiCognitionSession()` stays `null` → `GuiCognitionPanel` (rendered via
`<Show when={activeGuiCognitionSession()}>` in `ChatView.tsx`) NEVER mounts. The user sees only
the generic "Running GUI cognition workflow" thinking row (`ChatView.tsx`) and the single
end-of-turn `agent:token` reply. The entire rich GUI-cognition UI is dead code under V2.

**This is the #1 cause of "prompt diya, sirf Thinking + result action dikha, chat empty."**

## 2. HIGH — GUI reply is batched, not streamed

`agent:token` for a GUI turn fires once, only after the whole (often 60–120s) turn finishes
(`chat.rs` emits `capture.events` post-await). During the turn the transcript shows nothing from
the assistant. Combined with #1, the chat looks frozen/empty for the entire run.

## 3. HIGH — Optimistic messages can be wiped by hydration

- `syncEnvironmentSession` replaces the whole list when `!hasMessages`:
  `updateScopedMessages(scope, () => mapped)` (`app.ts:1176`).
- Session switch / clear paths call `setAssistantMessages([])` (`app.ts:3344`, `:3401`).
- `agent:done` → `loadSessions()` (`app.ts:3493`).
If any env-sync/session-switch fires around a GUI turn, the optimistic `userMsg` + the streamed
reply are replaced by backend history. GUI turns are stored via `store_turn` with mode
`"gui_cognition"` and a JSON-string reply; if `loadMappedSessionHistory` does not map that into a
normal user/assistant bubble, reloaded GUI turns vanish → empty transcript.

## 4. HIGH — Detached task has no turn correlation

`send_manual_tool_message` returns `{status:"processing"}`; the turn runs in a detached task whose
events are GLOBAL Tauri emits (not scoped to a request/turn id the frontend tracks). If the user
switches session/tab mid-turn, the reply/`done` land in whatever scope is "current" → wrong-session
routing or dropped reply. `session_id_override = None` adds a create→switch race window.

## 5. HIGH — No `ErrorBoundary` (root cause of "nav stuck")

**Evidence:** `App.tsx:1` imports `Suspense`/`lazy` but NOT `ErrorBoundary`; no `ErrorBoundary`
anywhere in `App.tsx`. Routed views are lazy: `DeviceMatrix` (VM Management), `N8nDashboard`,
`AnalyticsDashboard`, `TestRunnerDashboard`. If any throws during render, Solid has no boundary to
recover → the reactive subtree wedges and subsequent `navigate("home")` (`App.tsx:406`) cannot
re-render Home. This matches "VM Management/Dashboard ke baad Home pe stuck."

## 6. MEDIUM — `isThinking` can stick

Watchdog `ASSISTANT_THINKING_WATCHDOG_MS = 300_000` (5 min, `app.ts`). If the detached GUI task
loses `agent:done` (panic between await and emit, or a non-mapped terminal), input stays disabled
up to 5 min. The `gui_cognition:event` terminal safety-net in `app.ts:3800` cannot help because
(per #1) those envelopes are rejected before reaching lifecycle logic.

## 7. MEDIUM — Event flooding / render cost

`gui_cognition:event` is coalesced via `flushPendingEvents` (rAF/50ms, `app.ts:313`) — good. But
device telemetry (`useDeviceStatus`) runs SSE + WS + heartbeat globally (App comment: even when the
matrix isn't visible). Teardown is correct (`useDeviceStatus.ts:1134 onCleanup`), but constant
background work + GUI streaming raises main-thread pressure. OPINION: contributes to jank, not the
hard freeze (#5 is the freeze).

## 8. MEDIUM — Observability gaps

No turn_id surfaced to the client; no structured per-phase trace; GUI step telemetry only carries
action + ok, not phase/timing/reason. Hard to debug "kahan atka" from logs.

## Severity-ranked summary

| # | Issue | Severity | User symptom |
|---|-------|----------|--------------|
| 1 | V2↔frontend event contract broken | CRITICAL | empty chat, no steps, only "Thinking" |
| 2 | GUI reply batched not streamed | HIGH | feels hung during the whole turn |
| 3 | optimistic messages wiped on hydration | HIGH | prompt/reply disappear |
| 4 | detached task, no turn correlation | HIGH | wrong-session / dropped reply on switch |
| 5 | no ErrorBoundary | HIGH | nav stuck after VM/Dashboard |
| 6 | isThinking stick (late watchdog) | MEDIUM | input frozen up to 5 min |
| 7 | background telemetry + render cost | MEDIUM | jank |
| 8 | weak observability | MEDIUM | hard to debug |
