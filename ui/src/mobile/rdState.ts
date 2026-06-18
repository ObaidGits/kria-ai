/**
 * Pure session state machine for the in-app remote desktop.
 *
 * The state is a function of real control-plane / signaling / WebRTC events
 * (plus a bounded reconnect backoff). The view renders a human-readable label
 * and an optional user action per state, so the UI never shows an unexplained
 * indefinite spinner.
 */

export type RdState =
  | { tag: "idle" }
  | { tag: "requesting" }
  | { tag: "awaiting_approval"; description: string; sessionId: string }
  | { tag: "connecting" }
  | { tag: "negotiating" }
  | { tag: "establishing" }
  | { tag: "connected" }
  | { tag: "reconnecting"; attempt: number; nextRetryMs: number }
  | { tag: "disconnected"; reason: string }
  | { tag: "error"; message: string };

export type RdEvent =
  | { type: "request" }
  | { type: "request_ok"; description: string; sessionId: string }
  | { type: "request_fail"; message: string }
  | { type: "confirm" }
  | { type: "confirm_fail"; message: string }
  | { type: "ws_open" }
  | { type: "offer" }
  | { type: "ice_checking" }
  | { type: "track" }
  | { type: "ice_connected" }
  | { type: "ice_disconnected" }
  | { type: "ice_failed" }
  | { type: "ws_close" }
  | { type: "server_error"; message: string }
  | { type: "reconnect_give_up"; reason: string }
  | { type: "manual_reconnect" }
  | { type: "stop" };

/** Reconnect backoff schedule (ms). Index = attempt-1; last value caps. */
export const BACKOFF_MS = [500, 1000, 2000, 4000, 4000] as const;
export const MAX_RECONNECT_ATTEMPTS = BACKOFF_MS.length;

/** Transient states that must escalate if they make no progress (ms). */
export const WATCHDOG_MS = 12_000;

const backoffFor = (attempt: number): number =>
  BACKOFF_MS[Math.min(attempt, BACKOFF_MS.length) - 1] ?? BACKOFF_MS[BACKOFF_MS.length - 1];

/** Pure reducer: (state, event) → next state. */
export function rdReduce(state: RdState, ev: RdEvent): RdState {
  // Stop always returns to idle.
  if (ev.type === "stop") return { tag: "idle" };
  // A fatal server error wins from any state.
  if (ev.type === "server_error") return { tag: "error", message: ev.message };

  switch (ev.type) {
    case "request":
      return { tag: "requesting" };
    case "request_ok":
      return { tag: "awaiting_approval", description: ev.description, sessionId: ev.sessionId };
    case "request_fail":
      return { tag: "error", message: ev.message };
    case "confirm":
      return { tag: "connecting" };
    case "confirm_fail":
      return { tag: "error", message: ev.message };
    case "ws_open":
      // ws open during connecting (or a reconnect) → negotiating.
      return { tag: "negotiating" };
    case "offer":
      return { tag: "negotiating" };
    case "ice_checking":
      // Don't downgrade an already-connected session.
      return state.tag === "connected" ? state : { tag: "establishing" };
    case "track":
      // Track alone keeps us establishing until ICE confirms; if ICE already
      // connected we'd be here via ice_connected.
      return state.tag === "connected" ? state : { tag: "establishing" };
    case "ice_connected":
      return { tag: "connected" };
    case "ice_failed":
      return { tag: "error", message: "media connection failed" };
    case "ice_disconnected":
    case "ws_close": {
      // Transient drop while the session should be live → reconnecting.
      if (state.tag === "idle" || state.tag === "error" || state.tag === "disconnected") {
        return state;
      }
      const attempt = state.tag === "reconnecting" ? state.attempt + 1 : 1;
      if (attempt > MAX_RECONNECT_ATTEMPTS) {
        return { tag: "disconnected", reason: "lost connection" };
      }
      return { tag: "reconnecting", attempt, nextRetryMs: backoffFor(attempt) };
    }
    case "reconnect_give_up":
      return { tag: "disconnected", reason: ev.reason };
    case "manual_reconnect":
      return { tag: "connecting" };
    default:
      return state;
  }
}

/** Whether the state represents an intended-active session (drives reconnect). */
export function isLiveIntent(s: RdState): boolean {
  return (
    s.tag === "connecting" ||
    s.tag === "negotiating" ||
    s.tag === "establishing" ||
    s.tag === "connected" ||
    s.tag === "reconnecting"
  );
}

/** Transient states subject to the watchdog escalation. */
export function isTransient(s: RdState): boolean {
  return s.tag === "connecting" || s.tag === "negotiating" || s.tag === "establishing";
}

/** Human-readable label for a state. */
export function stateLabel(s: RdState): string {
  switch (s.tag) {
    case "idle":
      return "Not connected";
    case "requesting":
      return "Requesting session…";
    case "awaiting_approval":
      return "Waiting for approval on the laptop…";
    case "connecting":
      return "Connecting…";
    case "negotiating":
      return "Negotiating stream…";
    case "establishing":
      return "Establishing media…";
    case "connected":
      return "Connected";
    case "reconnecting":
      return `Reconnecting… (attempt ${s.attempt})`;
    case "disconnected":
      return `Disconnected — ${s.reason}`;
    case "error":
      return s.message;
  }
}

export type RdAction = "retry" | "cancel" | "reconnect" | "none";

/** Suggested user action for a state. */
export function stateAction(s: RdState): RdAction {
  switch (s.tag) {
    case "error":
      return "retry";
    case "disconnected":
      return "reconnect";
    case "connecting":
    case "negotiating":
    case "establishing":
    case "requesting":
      return "cancel";
    default:
      return "none";
  }
}
