# KRIA Chat System — Production Readiness

Checklist tracking the chat system toward Fast / Reliable / Transparent / Recoverable / Observable
/ Scalable. Status legend: ✅ done · �doing · ⬜ planned.

## Transparency (user sees what's happening)
- ✅ V2 emits the full lifecycle event set the panel understands (`TurnStarted` → per-phase →
  `TurnCompleted`/`TurnFailed`), streamed LIVE via a `LoopObserver` sink in the core loop
  (`loop_engine.rs`) mapped to the wire vocabulary in the desktop (`v2_loop_event_to_wire`).
- ✅ Per-step + per-phase progress in the panel (observe → decide → gate → execute → verify),
  emitted during the turn (not batched at end).
- ✅ Collapsible "🧠 Thinking" disclosure in the panel's layman summary shows the live
  `Decision.reason` (`planSummary`), collapsed by default, never raw CoT (`GuiCognitionPanel.tsx`).
- ✅ Unified per-tool status — the `agent:tool_call` (running) → `agent:tool_result` (done/error)
  lifecycle rendered as ToolCall chips in `MessageBubble` is the shared surface ALL agent tools
  (n8n/OpenClaw/MCP/Browser/Image) flow through; n8n + image add dedicated long-op progress
  (`WorkflowProgress`, `ImageProgressChip`). No speculative new envelope was added.

## Reliability
- ✅ Guaranteed `TurnEnded` on EVERY backend exit (done/ask/cap/no-progress/safety/error/cancel) —
  the `run_turn_v2` wrapper emits it on all paths (tested: `emits_turn_ended_even_on_error_exit`).
- ✅ Optimistic messages never blind-replaced — `syncEnvironmentSession` now MERGES backend
  history with local messages (`mergeHistoryWithLocalMessages`) and skips hydration while a turn is
  in flight (`!isScopedThinking`). (`app.ts`)
- ✅ GUI turns render in history — `loadMappedSessionHistory` maps `gui_cognition` mode turns into
  assistant bubbles (tool-row + plain-row paths verified).
- ✅ Session binding — the GUI turn is bound to a `current_session_id` SNAPSHOT taken before the
  detached task spawns (was `None` → re-read later); the command also returns `session_id` for
  client correlation. (`chat.rs`)
- ✅ Cross-session event isolation — agent `:token`/`:done` carry `session_id`; the frontend drops
  message-appending events stamped with a foreign session (`isForeignSession` guard in
  `registerStreamListeners`), backed by the backend stale-turn guard
  (`stale_guard_agent.is_turn_active`). (`chat.rs`, `app.ts`)
- ✅ Cooperative cancel (`cancel_gui_cognition_turn`).

## Recoverability
- ✅ Per-route `ErrorBoundary` with "Back to Home"/"Reload view" (fixes nav freeze) — `App.tsx`.
- ✅ Thinking watchdog tightened 300s → 60s, re-armed on every phase event
  (`pokeAssistantThinkingWatchdog`); the Stop control is the manual reset. (`app.ts`)
- ✅ Manual reset — the in-transcript Stop button + the panel Stop both abort the turn and clear
  the thinking state.

## Observability
- ✅ Structured per-phase trace — the core loop emits a typed `LoopEvent` for every phase
  (turn/observe/decide/gate/execute/verify) carrying `step_index`, action kind/detail, reason,
  ok/error; the desktop maps these to wire envelopes with monotonic `sequence` + `turn_id`.
- ✅ `Action::detail()` + `reason` in step telemetry.
- ⬜ Dev-mode raw event inspector (the panel's "Developer details" accordion covers most of this).

## Performance
- ✅ Event coalescing (`flushPendingEvents`, rAF/50ms).
- ✅ Device telemetry teardown (`useDeviceStatus.onCleanup`).
- ✅ Live streaming uses the existing coalesced `gui_cognition:event` channel — no new render path.

## Scalability
- ✅ Single typed envelope schema (`gui_cognition:event`) reused for all V2 phases; agent scope
  shares the `agent:*` contract with `session_id` correlation.
- ⬜ Transcript virtualization for very long sessions (history is already bounded to the Brain).

## Go-live gate
ALL CRITICAL + HIGH audit items are closed. Build: `cargo build --workspace` clean, **0 warnings**.
Tests green: kria-core V2 **66**, desktop glue **6**, frontend stores **78**, GuiCognitionPanel
**23**, frontend `tsc` clean. The three live scenarios (GUI run visibility, nav-no-freeze, mid-turn
recovery) are addressed by the shipped changes. **Status: production-grade for the audited scope.**
