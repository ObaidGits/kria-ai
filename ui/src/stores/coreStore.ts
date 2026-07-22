/**
 * Core Store — KRIA Core state machine (14+ states).
 *
 * Single source of truth for what KRIA is doing; every surface reads this.
 * Fed by domain events from the event bus (voice, agent, tool, workflow,
 * cognition, memory, approval, error). Drives the Core presence animation and
 * the tray glyph.
 *
 * ── Design (read-model, NOT an orchestrator) ────────────────────────────────
 * The Core is a *state indicator*. It observes domain events and reflects the
 * resulting state. It NEVER initiates execution, calls tools, or drives the
 * agent loop — doing so would create feedback loops. Transitions are pure and
 * deterministic: the resolved state is a function of the currently-active
 * domain activities plus a fixed precedence table.
 *
 * ── Activity-set model ──────────────────────────────────────────────────────
 * Multiple domain activities can be in-flight at once (e.g. voice listening
 * while an approval arrives). Each active activity contributes a *candidate*
 * Core state keyed by a stable source id. The resolved Core state is the
 * highest-priority candidate among all active activities; when none remain the
 * Core returns to `idle` (the resting state). This gives correct precedence
 * (blocked / error win) and correct return-to-idle without any timers or loops
 * for sustained activities.
 *
 * Requirements: 3.1 (Core states), 1.1 (global shell), 16.5 (fine-grained)
 */
import { createSignal } from "solid-js";
import { eventBus } from "./eventBus";
import type { Unsubscribe } from "./eventBus";

// ─── Core States ───────────────────────────────────────────────────────────────

/**
 * The 14+ possible states for the KRIA Core, as specified in Req 3.1.
 * Additional states can be added without breaking consumers.
 */
export type CoreState =
  | "idle"
  | "listening"
  | "thinking"
  | "planning"
  | "speaking"
  | "responding"
  | "acting"
  | "running-automation"
  | "watching"
  | "remembering"
  | "reflecting"
  | "learning"
  | "waiting"
  | "blocked"
  | "error"
  | "recovering";

/** States considered "active" (Core is doing something) */
export const ACTIVE_STATES: ReadonlySet<CoreState> = new Set([
  "listening",
  "thinking",
  "planning",
  "speaking",
  "responding",
  "acting",
  "running-automation",
  "watching",
  "remembering",
  "reflecting",
  "learning",
]);

/** States that need user attention */
export const ATTENTION_STATES: ReadonlySet<CoreState> = new Set([
  "blocked",
  "error",
  "waiting",
]);

/**
 * Precedence of each state when several domain activities are in-flight.
 * Higher wins. Attention states (blocked/error/recovering) outrank all work;
 * `idle` is the resting floor. This table is the single source of the
 * precedence rules referenced by Req 3.3 (blocked calms the Core & wins) and
 * the interruption ladder (Req 11.5 — approvals are the top interrupt).
 */
export const STATE_PRIORITY: Readonly<Record<CoreState, number>> = {
  error: 100,
  recovering: 95,
  blocked: 90,
  waiting: 60,
  speaking: 52,
  responding: 51,
  listening: 50,
  "running-automation": 46,
  acting: 44,
  planning: 40,
  thinking: 38,
  watching: 36,
  reflecting: 26,
  remembering: 24,
  learning: 22,
  idle: 0,
};

// ─── Domain Events (the bridge/stores feed these) ───────────────────────────────

export type CognitionJob =
  | "reflect"
  | "dream"
  | "consolidate"
  | "active-learning"
  | "self-improvement"
  | "entity-extraction";

/** A voice UI state as reported by the voice pipeline. */
export type VoicePhase =
  | "idle"
  | "wake_listening"
  | "listening"
  | "transcribing"
  | "thinking"
  | "speaking"
  | "interrupt"
  | "error";

/**
 * Discriminated union of every domain signal the Core reacts to. This is the
 * contract between the Tauri bridge / domain stores and the Core read-model.
 * `ingest()` maps each event into an activity mutation; the mapping table below
 * (`mapDomainEvent`) is pure and independently testable.
 */
export type CoreDomainEvent =
  | { kind: "voice"; state: VoicePhase }
  | {
      kind: "agent";
      phase: "start" | "thinking" | "planning" | "streaming" | "done" | "error";
      sessionId?: string;
      message?: string;
    }
  | { kind: "tool"; phase: "start" | "done"; callId?: string; name?: string }
  | { kind: "gui-cognition"; phase: "start" | "watching" | "acting" | "done"; sessionId?: string }
  | { kind: "workflow"; phase: "start" | "progress" | "done" | "failed"; workflowId?: string }
  | { kind: "cognition"; job: CognitionJob; phase: "start" | "done" }
  | { kind: "memory"; op: "updated" | "deleted" | "remembering" }
  | { kind: "approval"; phase: "request" | "resolved"; id?: string; reason?: string }
  | { kind: "waiting"; phase: "start" | "done"; id?: string; reason?: string }
  | { kind: "error"; phase: "raised" | "recovering" | "cleared"; message?: string }
  | { kind: "reset" };

/** A mutation to the activity set derived from a domain event. */
export type ActivityOp =
  | { op: "begin"; source: string; state: CoreState; reason?: string; error?: string }
  | { op: "pulse"; source: string; state: CoreState; ttlMs: number }
  | { op: "end"; source: string }
  | { op: "clear" }
  | { op: "noop" };

/** Default lifetime for momentary "pulse" activities (memory writes, etc.). */
const PULSE_TTL_MS = 1500;

const COGNITION_STATE: Readonly<Record<CognitionJob, CoreState>> = {
  reflect: "reflecting",
  dream: "reflecting",
  consolidate: "remembering",
  "active-learning": "learning",
  "self-improvement": "learning",
  "entity-extraction": "remembering",
};

/**
 * Pure mapping: domain event → activity-set mutation.
 *
 * This is the authoritative event→state mapping table. It is deterministic and
 * side-effect free so it can be unit-tested in isolation.
 */
export function mapDomainEvent(event: CoreDomainEvent): ActivityOp {
  switch (event.kind) {
    case "voice": {
      const source = "voice";
      switch (event.state) {
        case "wake_listening":
        case "listening":
        case "interrupt":
          return { op: "begin", source, state: "listening" };
        case "transcribing":
        case "thinking":
          return { op: "begin", source, state: "thinking" };
        case "speaking":
          return { op: "begin", source, state: "speaking" };
        case "idle":
        case "error":
          // Voice errors surface as notifications; the Core simply stands down.
          return { op: "end", source };
      }
      return { op: "noop" };
    }

    case "agent": {
      const source = `agent:${event.sessionId ?? "default"}`;
      switch (event.phase) {
        case "start":
        case "thinking":
          return { op: "begin", source, state: "thinking" };
        case "planning":
          return { op: "begin", source, state: "planning" };
        case "streaming":
          // Text token streaming is a distinct phase from voice TTS `speaking`.
          return { op: "begin", source, state: "responding" };
        case "error":
          // Route through the dedicated error activity so precedence applies.
          return { op: "begin", source: "error", state: "error", error: event.message };
        case "done":
          return { op: "end", source };
      }
      return { op: "noop" };
    }

    case "tool": {
      const source = `tool:${event.callId ?? event.name ?? "default"}`;
      return event.phase === "start"
        ? { op: "begin", source, state: "acting" }
        : { op: "end", source };
    }

    case "gui-cognition": {
      const source = `gui:${event.sessionId ?? "default"}`;
      switch (event.phase) {
        case "start":
        case "watching":
          return { op: "begin", source, state: "watching" };
        case "acting":
          return { op: "begin", source, state: "acting" };
        case "done":
          return { op: "end", source };
      }
      return { op: "noop" };
    }

    case "workflow": {
      const source = `workflow:${event.workflowId ?? "default"}`;
      return event.phase === "start" || event.phase === "progress"
        ? { op: "begin", source, state: "running-automation" }
        : { op: "end", source };
    }

    case "cognition": {
      const source = `cognition:${event.job}`;
      return event.phase === "start"
        ? { op: "begin", source, state: COGNITION_STATE[event.job] }
        : { op: "end", source };
    }

    case "memory":
      // Memory writes are momentary — pulse the "remembering" state briefly.
      return { op: "pulse", source: "memory", state: "remembering", ttlMs: PULSE_TTL_MS };

    case "approval": {
      const source = `approval:${event.id ?? "default"}`;
      return event.phase === "request"
        ? { op: "begin", source, state: "blocked", reason: event.reason }
        : { op: "end", source };
    }

    case "waiting": {
      const source = `waiting:${event.id ?? "default"}`;
      return event.phase === "start"
        ? { op: "begin", source, state: "waiting", reason: event.reason }
        : { op: "end", source };
    }

    case "error":
      if (event.phase === "raised") return { op: "begin", source: "error", state: "error", error: event.message };
      if (event.phase === "recovering") return { op: "begin", source: "error", state: "recovering" };
      return { op: "end", source: "error" };

    case "reset":
      return { op: "clear" };
  }
  return { op: "noop" };
}

// ─── State Transition Rules (advisory, manual path only) ────────────────────────

/**
 * Valid state transitions. Undefined key = any transition allowed from that state.
 * This is advisory — the manual `setState` logs warnings on invalid transitions
 * but doesn't block. The event-fed path resolves from the activity set and
 * bypasses this check (it is always a valid derived state).
 */
const VALID_TRANSITIONS: Partial<Record<CoreState, readonly CoreState[]>> = {
  idle: ["listening", "thinking", "planning", "speaking", "responding", "acting", "running-automation", "watching", "remembering", "reflecting", "learning", "waiting", "blocked", "error"],
  listening: ["idle", "thinking", "speaking", "responding", "blocked", "error"],
  thinking: ["idle", "planning", "speaking", "responding", "acting", "blocked", "error", "waiting"],
  planning: ["idle", "thinking", "responding", "acting", "blocked", "error"],
  speaking: ["idle", "listening", "thinking", "responding", "error"],
  responding: ["idle", "listening", "thinking", "error"],
  acting: ["idle", "thinking", "blocked", "error", "waiting"],
  "running-automation": ["idle", "acting", "blocked", "error", "waiting"],
  watching: ["idle", "acting", "thinking", "error"],
  remembering: ["idle", "error"],
  reflecting: ["idle", "learning", "remembering", "error"],
  learning: ["idle", "error"],
  waiting: ["idle", "thinking", "acting", "blocked", "error"],
  blocked: ["idle", "waiting", "error", "recovering"],
  error: ["idle", "recovering"],
  recovering: ["idle", "error"],
};

// ─── Signals ───────────────────────────────────────────────────────────────────

const [state, setStateSignal] = createSignal<CoreState>("idle");
const [previousState, setPreviousState] = createSignal<CoreState>("idle");
const [stateTimestamp, setStateTimestamp] = createSignal<number>(Date.now());
const [errorMessage, setErrorMessage] = createSignal<string | null>(null);
const [blockReason, setBlockReason] = createSignal<string | null>(null);

// ─── Activity Set (non-reactive; drives the resolved signal) ────────────────────

interface Activity {
  state: CoreState;
  reason?: string;
  error?: string;
  /** Wall-clock expiry for pulse activities; undefined = sustained. */
  expiresAt?: number;
  /** Timer handle for pulse expiry, so it can be cleared on reset/end. */
  timer?: ReturnType<typeof setTimeout>;
}

const activities = new Map<string, Activity>();

// ─── Derived ───────────────────────────────────────────────────────────────────

/** Whether the Core is actively doing something */
const isActive = () => ACTIVE_STATES.has(state());

/** Whether the Core needs user attention (blocked/error/waiting) */
const needsAttention = () => ATTENTION_STATES.has(state());

/** Whether the Core is idle */
const isIdle = () => state() === "idle";

/** Duration in current state. */
const stateDuration = () => Date.now() - stateTimestamp();

// ─── Core mutation (shared by manual + event-fed paths) ─────────────────────────

/**
 * Apply a resolved Core state. Central mutation used by both the manual
 * `setState` escape hatch and the event-fed resolver. Emits
 * "core:state-changed" only on an actual state change, and keeps the
 * error/block metadata coherent with the current state.
 */
function applyState(next: CoreState, meta?: { error?: string | null; blockReason?: string | null }): void {
  const current = state();

  if (current === next) {
    // No transition, but refresh metadata (e.g. a newer block reason).
    if (meta?.error !== undefined) setErrorMessage(meta.error);
    if (meta?.blockReason !== undefined) setBlockReason(meta.blockReason);
    return;
  }

  setPreviousState(current);
  setStateSignal(next);
  setStateTimestamp(Date.now());

  if (meta?.error !== undefined) setErrorMessage(meta.error);
  if (meta?.blockReason !== undefined) setBlockReason(meta.blockReason);
  if (next !== "error" && next !== "recovering") setErrorMessage(null);
  if (next !== "blocked") setBlockReason(null);

  eventBus.emit("core:state-changed", { state: next, previous: current });
}

/**
 * Recompute the resolved Core state from the active activity set and apply it.
 * Deterministic: highest STATE_PRIORITY wins; ties resolve to the
 * earliest-inserted activity (Map insertion order). Empty set → idle.
 */
function resolve(): void {
  let best: { source: string; activity: Activity } | null = null;
  const now = Date.now();

  for (const [source, activity] of activities) {
    if (activity.expiresAt !== undefined && activity.expiresAt <= now) continue;
    if (best === null || STATE_PRIORITY[activity.state] > STATE_PRIORITY[best.activity.state]) {
      best = { source, activity };
    }
  }

  if (best === null) {
    applyState("idle");
    return;
  }

  applyState(best.activity.state, {
    error: best.activity.error ?? null,
    blockReason: best.activity.reason ?? null,
  });
}

function clearActivityTimers(): void {
  for (const activity of activities.values()) {
    if (activity.timer) clearTimeout(activity.timer);
  }
}

function applyOp(mutation: ActivityOp): void {
  switch (mutation.op) {
    case "begin": {
      const prev = activities.get(mutation.source);
      if (prev?.timer) clearTimeout(prev.timer);
      activities.set(mutation.source, {
        state: mutation.state,
        reason: mutation.reason,
        error: mutation.error,
      });
      break;
    }
    case "pulse": {
      const prev = activities.get(mutation.source);
      if (prev?.timer) clearTimeout(prev.timer);
      const timer = setTimeout(() => {
        activities.delete(mutation.source);
        resolve();
      }, mutation.ttlMs);
      activities.set(mutation.source, {
        state: mutation.state,
        expiresAt: Date.now() + mutation.ttlMs,
        timer,
      });
      break;
    }
    case "end": {
      const prev = activities.get(mutation.source);
      if (prev?.timer) clearTimeout(prev.timer);
      activities.delete(mutation.source);
      break;
    }
    case "clear":
      clearActivityTimers();
      activities.clear();
      break;
    case "noop":
      return;
  }
  resolve();
}

// ─── Public: event-fed ingest ───────────────────────────────────────────────────

/**
 * Feed a domain event into the Core state machine. Pure, deterministic, bounded:
 * updates the activity set then re-resolves the Core state. This is how the
 * bridge and domain stores drive the Core — the Core never calls back out.
 */
function ingest(event: CoreDomainEvent): void {
  applyOp(mapDomainEvent(event));
}

// ─── Public: manual escape hatch (kept for direct control + tests) ──────────────

/**
 * Transition the Core to a new state directly. Emits "core:state-changed" on the
 * bus. Logs a warning (dev only) on unusual transitions but does NOT block.
 *
 * Prefer `ingest()` for event-driven state; `setState` is a manual override.
 */
function setState(next: CoreState, meta?: { error?: string; blockReason?: string }): void {
  const current = state();
  if (current === next) return;

  if (import.meta.env?.DEV) {
    const allowed = VALID_TRANSITIONS[current];
    if (allowed && !allowed.includes(next)) {
      console.warn(
        `[coreStore] Unusual transition: ${current} → ${next}. Expected one of:`,
        allowed
      );
    }
  }

  applyState(next, meta);
}

/** Convenience: transition to idle */
function goIdle(): void {
  setState("idle");
}

/** Convenience: mark as blocked (directs attention to Approval Center per Req 3.3) */
function setBlocked(reason: string): void {
  setState("blocked", { blockReason: reason });
}

/** Convenience: mark as error */
function setError(message: string): void {
  setState("error", { error: message });
}

/** Reset to initial state (clears the activity set too). */
function reset(): void {
  clearActivityTimers();
  activities.clear();
  setStateSignal("idle");
  setPreviousState("idle");
  setStateTimestamp(Date.now());
  setErrorMessage(null);
  setBlockReason(null);
}

// ─── Event-bus wiring ───────────────────────────────────────────────────────────

let subscriptions: Unsubscribe[] = [];

/**
 * Subscribe the Core state machine to the typed event bus. Idempotent; returns a
 * dispose function. Only the events the bridge currently emits are wired here;
 * richer domain signals (tool calls, gui-cognition, cognition jobs) reach the
 * Core via `ingest()` as the bridge maps them.
 *
 * NOTE: never subscribe to "core:state-changed" — the Core emits it, so doing so
 * would create a feedback loop (architecture invariant).
 */
function initCoreStateMachine(): Unsubscribe {
  if (subscriptions.length > 0) return disposeCoreStateMachine;

  subscriptions = [
    eventBus.on("voice:state-changed", (p) => {
      ingest({ kind: "voice", state: (p.state || "idle") as VoicePhase });
    }),
    // Both the turn bracket (thinking→done) and streaming tokens are coalesced
    // on the SAME rAF queue so their emission order is preserved. Otherwise a
    // trailing rAF-batched token could re-`begin` the agent activity AFTER a
    // synchronously-processed `done` already ended it, leaving the Core stuck in
    // `responding` (Stop button + statusline never returning to idle).
    eventBus.on(
      "converse:thinking-changed",
      (p) => {
        ingest({ kind: "agent", phase: p.thinking ? "thinking" : "done", sessionId: p.sessionId });
      },
      "raf"
    ),
    eventBus.on(
      "converse:token",
      (p) => {
        ingest({ kind: "agent", phase: "streaming", sessionId: p.sessionId });
      },
      "raf"
    ),
    eventBus.on("approval:request", (p) => {
      ingest({ kind: "approval", phase: "request", id: p.id });
    }),
    eventBus.on("approval:resolved", (p) => {
      ingest({ kind: "approval", phase: "resolved", id: p.id });
    }),
    eventBus.on("automation:workflow-started", (p) => {
      ingest({ kind: "workflow", phase: "start", workflowId: p.workflowId });
    }),
    eventBus.on("automation:workflow-completed", (p) => {
      ingest({ kind: "workflow", phase: p.success ? "done" : "failed", workflowId: p.workflowId });
    }),
    eventBus.on("memory:updated", () => {
      ingest({ kind: "memory", op: "updated" });
    }),
    eventBus.on("memory:deleted", () => {
      ingest({ kind: "memory", op: "deleted" });
    }),
    // Cognition jobs → Core running state (reflecting/remembering/learning,
    // Req 5.6 / 3.1). The Cognition controls emit these on trigger/completion;
    // the Core merely reflects them (COGNITION_STATE mapping), never drives.
    eventBus.on("memory:cognition-started", (p) => {
      ingest({ kind: "cognition", job: p.job as CognitionJob, phase: "start" });
    }),
    eventBus.on("memory:cognition-completed", (p) => {
      ingest({ kind: "cognition", job: p.job as CognitionJob, phase: "done" });
    }),
  ];

  return disposeCoreStateMachine;
}

/** Detach all event-bus subscriptions wired by `initCoreStateMachine`. */
function disposeCoreStateMachine(): void {
  for (const unsub of subscriptions) unsub();
  subscriptions = [];
}

// ─── Export ────────────────────────────────────────────────────────────────────

export const coreStore = {
  // Signals (read-only)
  state,
  previousState,
  stateTimestamp,
  errorMessage,
  blockReason,

  // Derived
  isActive,
  needsAttention,
  isIdle,
  stateDuration,

  // Event-fed machine
  ingest,
  initCoreStateMachine,
  disposeCoreStateMachine,

  // Manual escape hatch
  setState,
  goIdle,
  setBlocked,
  setError,
  reset,
} as const;
