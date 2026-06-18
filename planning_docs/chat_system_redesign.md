# KRIA Chat System — Redesign (Phases B–C: Critical Review + Alternatives)

For each top issue: challenge the finding, compare current vs better vs best, pick the strongest
option achievable inside the current KRIA architecture (Tauri events + SolidJS stores + spawned
backend turns). No SSE/socket rewrite is justified — see §0.

## 0. Transport decision (challenged)

**Question:** would SSE / a multiplexed stream / an aggregator materially improve UX over the
current Tauri `emit`/`listen`?
- Tauri events are already an in-process push channel with sub-ms latency; the gaps are *contract*
  and *granularity*, not transport.
- SSE/WS would add a second transport, reconnection logic, and ordering concerns for ZERO latency
  win in-process.
**Decision:** keep Tauri events. Fix the *contract* (what is emitted) and *correlation* (turn_id),
add per-phase granularity. Introduce a single typed envelope + a sequence/turn guard (already
present for gui_cognition; extend to the agent scope).

## 1. CRITICAL — V2 ↔ frontend event contract

**Challenge:** options to reconcile the divergence.
- **A (current):** broken — V2 emits `V2Step`, frontend wants `TurnStarted`+rich vocabulary.
- **B (adapt frontend):** add a `V2Step` case + accept first V2 envelope as session start. Cheap,
  but the rich panel (observation/plan/target/verify fields) stays empty — under-uses the UI.
- **C (BEST): backend emits the lifecycle envelopes the panel already understands, mapped from the
  V2 loop phases.** V2's phases map cleanly:
  ```
  loop start            → TurnStarted
  Sight.observe start   → ObservationStarted
  Sight.observe done    → ObservationCompleted { active_window, source }
  Brain.decide done     → PlanProposed/RouteConfirmed { action, reason }   (reason = thinking)
  SafetyGate            → SafetyEvaluation { status }   (+ HitlRequired when Deny-needs-approval)
  Hands.execute start   → ExecutionStarted { action_kind, action_detail }
  Hands.execute done    → ExecutionCompleted/Failed { ok, error }
  re-observe/verify     → VerificationCompleted { status: verified|verification_failed }
  loop end              → TurnEnded { status }   (NEW terminal — drives panel close + thinking off)
  ```
  This revives the existing panel + lifecycle + the per-step/phase visibility the user asked for,
  with NO frontend rewrite. Add ONE new event the frontend lacks: explicit `TurnEnded`.

**Chosen: C.** Strongest reliability (single source of truth = backend phases), best UX (full
panel + steps + reason), low frontend churn. Keep a thin `V2Step`→adapter only as a fallback.

## 2. GUI reply streaming + transcript presence

- **Current:** reply only at end; transcript empty during turn.
- **Best:** on `TurnStarted`, frontend inserts a live assistant "GUI run" bubble that mirrors the
  panel's current phase/step + `reason` (thinking) as it streams; on `TurnEnded` it finalizes with
  the summary. Backend also emits a short `agent:token` progress line per meaningful step (optional;
  the panel already conveys detail, so keep transcript line concise to avoid noise).
**Chosen:** live bubble bound to the gui-cognition session + concise per-step progress; full detail
in the (now-working) panel.

## 3. Optimistic message integrity

- **Current:** list-replace on hydration can wipe optimistic msgs.
- **Best:** never blind-replace. Give every optimistic message a stable `client_id`; hydration
  MERGES by `client_id`/server id (de-dupe), never overwrites an in-flight turn's messages. GUI
  turns persisted with mode `gui_cognition` MUST be mapped into a renderable user+assistant pair by
  `loadMappedSessionHistory`.
**Chosen:** merge-by-id hydration + GUI-turn history mapping.

## 4. Turn correlation

- **Best:** `send_manual_tool_message`/`send_message` RETURN a `turn_id`; backend stamps every
  `agent:*`/`gui_cognition:*` event with that `turn_id` + `session_id`; frontend applies an event
  only if `turn_id` matches the scope's active turn AND `session_id` matches current. Pass the
  frontend's real `session_id` into the spawned GUI task (kill `None` race).
**Chosen:** turn_id correlation + explicit session binding.

## 5. ErrorBoundary

- **Best:** wrap each routed `<Show>` body (Home/Dashboard/VM/Settings) in a Solid `ErrorBoundary`
  with a fallback (error text + "Reload view" that resets local state and re-navigates). A crash in
  one view never wedges navigation.
**Chosen:** per-route ErrorBoundary. (Cheapest high-impact fix for the nav-stuck bug.)

## 6. Thinking UX

- **Best:** the `Decision.reason` (already sanitized, never raw CoT) is the "thinking". Show it as a
  collapsed, expandable "🧠 Thinking" line per step in the panel + the live bubble. For reasoning
  models, stream `<think>` to a separate `agent:reasoning` channel (today it is stripped by
  `sanitize_json_object_content`) — collapsed by default, opt-in expand. Never dump raw CoT.
**Chosen:** sanitized reason inline (collapsible) now; reasoning-channel later.

## 7. Recovery layer

- **Best:** a single `TurnEnded` guarantee (backend emits on every exit incl. error/panic) +
  frontend watchdog reduced to ~45–60s with re-arm on any phase event + an explicit "stuck? reset"
  affordance. `cancel_gui_cognition_turn` already cooperative.
**Chosen:** guaranteed terminal event + tighter watchdog + manual reset.

## Prioritization (UX × Reliability ÷ Cost)

1. **#1 contract fix (backend lifecycle events + `TurnEnded`)** — unblocks panel, steps, thinking.
2. **#5 per-route ErrorBoundary** — fixes nav freeze; tiny cost.
3. **#2 live transcript bubble + per-step streaming** — kills "empty/hang".
4. **#6 collapsible thinking (reason)** — rides on #1.
5. **#3 optimistic-merge + GUI history mapping**, **#4 turn_id**, **#7 watchdog/reset** — next.

Implementation will start with 1, 2, 5, 6 (one coherent change) since they share the event path.
