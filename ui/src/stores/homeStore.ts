/**
 * Home Store — the homepage presence state machine + homepage-local UI state.
 *
 * Owns the explicit homepage macro state machine required by Req 30.1:
 *
 *     rest ↔ engaged ↔ reading ↔ mode-transition ↔ companion ↔ blocked
 *
 * plus the homepage-local UI slices from design §13.1 (view mode, dock reveal,
 * orbit engagement, reading-mode flag, companion state, render mode). This is
 * PURE UI state: it performs no orchestration, no tool calls, no sends, and NO
 * domain-store writes. Cross-store communication is via the typed event bus only
 * (it emits `home:state-changed`; it never reaches into another store).
 *
 * ── Authority invariants (Req 29 / Req 30.3, guardrails.md "Never") ──────────
 * • `coreStore` is the SOLE authority for Core state. `homeStore` NEVER writes
 *   it — it only snapshots a Core state value into its preserved shared context
 *   (a plain value, carried across transitions). The Focus engine's `coreHint`
 *   is advisory only and is never written back to `coreStore`; the guardrail
 *   lint (`scripts/guardrail-lint.mjs`) statically enforces this on this file
 *   and the Focus engine files.
 * • Explicit user actions (mode, navigation) win; the machine models the
 *   resulting UI state, it does not decide domain behavior.
 *
 * ── Focus management (Req 30.1) ──────────────────────────────────────────────
 * Each macro state has a canonical focus target. Two states are *transient
 * overlays* — `blocked` and `mode-transition` — and capture a return state so
 * focus (and the resting macro state) is restored when they clear. The Core is
 * the continuity anchor during `mode-transition` (Req 13.2).
 *
 * ── Shared-state preservation (Req 30.1 / Req 13.3) ──────────────────────────
 * The active thread, a snapshot of the Core emotional state, the composer draft,
 * and the Focus subject id are held in `sharedContext` and are NEVER cleared by
 * a state transition. Switching modes / entering reading / going companion all
 * preserve this context by construction (transitions only touch macro state +
 * local slices, never the shared context).
 *
 * Requirements: 30.1, 30.2, 30.3, 13.3, 16.5
 */
import { createSignal, batch } from "solid-js";
import { eventBus } from "./eventBus";
// Type-only import: homeStore snapshots a Core state VALUE; it never imports or
// calls a coreStore mutator (authority invariant, Req 30.3).
import type { CoreState } from "./coreStore";

// ─── Macro states ────────────────────────────────────────────────────────────

/**
 * The explicit homepage state machine (Req 30.1). Transient overlay states
 * (`blocked`, `mode-transition`) capture a return state and restore it on exit.
 */
export type HomeState =
  | "rest" // resting presence; Core is the focal point, nothing engaged
  | "engaged" // user is interacting (composer focused / orbit engaged)
  | "reading" // post-first-send depth-recession reading mode
  | "mode-transition" // transient: switching view mode; Core is continuity anchor
  | "companion" // condensed cross-application ember presence
  | "blocked"; // interruptibility-blocked / approval-blocking context

/** Canonical focus target for each macro state (Req 21.1 / 30.1). */
export type HomeFocusTarget =
  | "core"
  | "composer"
  | "message-stream"
  | "companion-ember"
  | "approval";

/** View modes (design §13.1 / Req 13.1). Distinct from shellStore's legacy
 * `WindowMode` triplet, which task 8.1 reconciles into this canonical set. */
export type HomeViewMode = "immersive" | "standard" | "mini" | "companion";

/** Resolved render path (design §13.1). Capability gate resolves `auto`. */
export type HomeRenderMode = "2d" | "3d" | "auto";

/** Edge anchor for the floating companion ember. */
export type EdgeAnchor = "top-left" | "top-right" | "bottom-left" | "bottom-right";

/**
 * Shared context preserved across EVERY transition (Req 13.3 / 30.1). Held as
 * plain snapshot values — `coreState` is a copied value, not a live coreStore
 * handle, so the authority invariant (Req 30.3) holds.
 */
export interface HomeSharedContext {
  /** Active conversation thread id (owned by converseStore; snapshotted here). */
  threadId: string | null;
  /** Snapshot of the Core emotional state; coreStore stays the authority. */
  coreState: CoreState | null;
  /** Composer draft text (per-thread draft lives in converseStore). */
  draft: string;
  /** Currently-bound Focus subject id (owned by homeFocusStore). */
  focusSubjectId: string | null;
}

// ─── Transition table ────────────────────────────────────────────────────────

/**
 * Valid transitions. Self-transitions are always ignored (no-op). `reading` is
 * only reachable via `engaged` (first send) or from a transient overlay
 * restoring it — never directly from `rest` (the user must engage first). The
 * transient overlays (`blocked`, `mode-transition`) are reachable from every
 * stable state and can restore any stable state.
 */
export const VALID_HOME_TRANSITIONS: Readonly<Record<HomeState, readonly HomeState[]>> = {
  rest: ["engaged", "mode-transition", "companion", "blocked"],
  engaged: ["rest", "reading", "mode-transition", "companion", "blocked"],
  reading: ["rest", "engaged", "mode-transition", "companion", "blocked"],
  "mode-transition": ["rest", "engaged", "reading", "companion", "blocked"],
  companion: ["rest", "engaged", "reading", "mode-transition", "blocked"],
  blocked: ["rest", "engaged", "reading", "mode-transition", "companion"],
};

/** Canonical focus target per state. Core anchors `mode-transition` (Req 13.2). */
export const HOME_FOCUS_TARGET: Readonly<Record<HomeState, HomeFocusTarget>> = {
  rest: "core",
  engaged: "composer",
  reading: "message-stream",
  "mode-transition": "core",
  companion: "companion-ember",
  blocked: "approval",
};

/** Stable (non-overlay) states a transient overlay can restore to. */
const STABLE_STATES: ReadonlySet<HomeState> = new Set<HomeState>([
  "rest",
  "engaged",
  "reading",
  "companion",
]);

// ─── Signals ───────────────────────────────────────────────────────────────

const [state, setStateSignal] = createSignal<HomeState>("rest");
const [previousState, setPreviousState] = createSignal<HomeState>("rest");

const [viewMode, setViewModeSignal] = createSignal<HomeViewMode>("standard");
const [renderMode, setRenderModeSignal] = createSignal<HomeRenderMode>("auto");
const [dockRevealed, setDockRevealedSignal] = createSignal(false);
const [dockPinned, setDockPinnedSignal] = createSignal(false);
const [orbitEngaged, setOrbitEngagedSignal] = createSignal(false);
const [companionBrightened, setCompanionBrightenedSignal] = createSignal(false);
const [companionPosition, setCompanionPositionSignal] = createSignal<EdgeAnchor | undefined>(
  undefined,
);

const [sharedContext, setSharedContextSignal] = createSignal<HomeSharedContext>({
  threadId: null,
  coreState: null,
  draft: "",
  focusSubjectId: null,
});

// ─── Transient-overlay return bookkeeping (focus + macro-state restore) ───────

/** State to restore when `blocked` clears. */
let blockedReturnState: HomeState = "rest";
/** State to restore when `mode-transition` completes. */
let modeReturnState: HomeState = "rest";
/** View mode staged by `beginModeTransition`, applied on `completeModeTransition`. */
let pendingViewMode: HomeViewMode | null = null;

// ─── Derived ─────────────────────────────────────────────────────────────────

/** Reading Mode flag (design §13.1) — derived from the macro state. */
const readingMode = () => state() === "reading";

/** Composed companion slice (design §13.1). `active` is derived from the state. */
const companion = () => ({
  active: state() === "companion",
  brightened: companionBrightened(),
  position: companionPosition(),
});

/** Whether the machine is in a transient overlay (blocked / mode-transition). */
const isTransient = () => state() === "blocked" || state() === "mode-transition";

/** The canonical focus target for the current state (Req 21.1 / 30.1). */
const focusTarget = (): HomeFocusTarget => HOME_FOCUS_TARGET[state()];

/** Whether `to` is a valid transition target from the current state. */
const canTransition = (to: HomeState): boolean =>
  to !== state() && VALID_HOME_TRANSITIONS[state()].includes(to);

// ─── Core transition primitive ────────────────────────────────────────────────

/**
 * Apply a macro-state transition. Deterministic and validated: an invalid or
 * self transition is a no-op (a warning is logged in dev, matching coreStore's
 * advisory philosophy — the UI never throws mid-interaction). Emits
 * `home:state-changed` only on an actual change. NEVER touches `sharedContext`
 * (shared-state preservation is by construction, Req 30.1 / 13.3).
 */
function transition(next: HomeState): boolean {
  const current = state();
  if (current === next) return false;

  if (!VALID_HOME_TRANSITIONS[current].includes(next)) {
    if (import.meta.env?.DEV) {
      console.warn(
        `[homeStore] Invalid transition: ${current} → ${next}. Allowed:`,
        VALID_HOME_TRANSITIONS[current],
      );
    }
    return false;
  }

  setPreviousState(current);
  setStateSignal(next);
  eventBus.emit("home:state-changed", { state: next, previous: current });
  return true;
}

// ─── Public actions ────────────────────────────────────────────────────────

/** Engage from rest/reading/companion (composer focus / orbit engagement). */
function engage(): boolean {
  return transition("engaged");
}

/** Return to resting presence. */
function rest(): boolean {
  return transition("rest");
}

/**
 * Enter Reading Mode (first send → depth-recession, Req 11.1). Reachable from
 * `engaged`; also honored from a transient overlay restoring reading.
 */
function enterReading(): boolean {
  return transition("reading");
}

/** Leave Reading Mode back to engaged (composer regains focus). */
function exitReading(): boolean {
  return transition("engaged");
}

/**
 * Begin a continuous view-mode transition (Req 13.2/13.3). Captures the current
 * stable state to restore, stages the target view mode (applied on completion),
 * and enters the transient `mode-transition` state (Core is the continuity
 * anchor). Shared context is untouched and therefore preserved.
 */
function beginModeTransition(target: HomeViewMode): boolean {
  const current = state();
  modeReturnState = STABLE_STATES.has(current) ? current : "rest";
  pendingViewMode = target;
  return transition("mode-transition");
}

/**
 * Complete the staged view-mode transition. Applies the pending view mode and
 * restores the captured stable macro state — unless the target view mode is
 * `companion`, in which case it resolves into the `companion` macro state.
 */
function completeModeTransition(): boolean {
  const target = pendingViewMode;
  let restored: boolean;
  batch(() => {
    if (target) setViewModeSignal(target);
    if (target === "companion") {
      restored = transition("companion");
    } else {
      restored = transition(modeReturnState);
    }
  });
  pendingViewMode = null;
  return restored!;
}

/**
 * Enter Companion Mode (condensed cross-application ember, Req 15.1). Inherits
 * the Core emotional state via the preserved shared context (never a coreStore
 * write). Optionally anchors the ember to a screen edge.
 */
function enterCompanion(position?: EdgeAnchor): boolean {
  const changed = transition("companion");
  if (changed && position) setCompanionPositionSignal(position);
  return changed;
}

/** Return from Companion Mode to resting presence and clear brightening. */
function exitCompanion(): boolean {
  const changed = transition("rest");
  if (changed) setCompanionBrightenedSignal(false);
  return changed;
}

/** Brighten / dim the companion ember (only for meaningful needs, Req 15.2). */
function setCompanionBrightened(brightened: boolean): void {
  setCompanionBrightenedSignal(brightened);
}

/**
 * Re-anchor the companion ember to a screen corner (design §9 "optional
 * reposition/nudge", Req 15.3). Independent of the macro state so the ember can
 * be nudged while already in Companion mode (unlike {@link enterCompanion},
 * which only sets the position on an actual transition).
 */
function setCompanionPosition(position: EdgeAnchor): void {
  setCompanionPositionSignal(position);
}

/**
 * Enter the interruptibility-blocked / approval-blocking overlay (Req 26.2).
 * Captures the current stable state so focus + macro state can be restored when
 * the block clears. Only RED approvals should surface here (enforced by the
 * Focus engine, not this store).
 */
function enterBlocked(): boolean {
  const current = state();
  blockedReturnState = STABLE_STATES.has(current) ? current : "rest";
  return transition("blocked");
}

/** Clear the blocked overlay and restore the captured stable state + focus. */
function exitBlocked(): boolean {
  return transition(blockedReturnState);
}

// ─── Local UI slice setters (design §13.1) ────────────────────────────────────

function setViewMode(mode: HomeViewMode): void {
  setViewModeSignal(mode);
}

function setRenderMode(mode: HomeRenderMode): void {
  setRenderModeSignal(mode);
}

/** Reveal / hide the Hidden Dock (edge/Alt/⌘K/pin/AT-focus, Req 7.1). */
function setDockRevealed(revealed: boolean): void {
  // A pinned dock stays revealed; only an explicit unpin can hide it.
  if (!revealed && dockPinned()) return;
  setDockRevealedSignal(revealed);
}

function setDockPinned(pinned: boolean): void {
  batch(() => {
    setDockPinnedSignal(pinned);
    if (pinned) setDockRevealedSignal(true);
  });
}

function setOrbitEngaged(engaged: boolean): void {
  setOrbitEngagedSignal(engaged);
}

// ─── Shared-context preservation (Req 30.1 / 13.3) ─────────────────────────────

/**
 * Merge a partial update into the preserved shared context. This is the ONLY
 * writer of shared context; transitions never touch it, so the active thread,
 * Core-state snapshot, draft, and Focus subject survive every state change.
 */
function updateSharedContext(patch: Partial<HomeSharedContext>): void {
  setSharedContextSignal((prev) => ({ ...prev, ...patch }));
}

// ─── Reset (tests + hard cutover) ──────────────────────────────────────────────

function reset(): void {
  batch(() => {
    setStateSignal("rest");
    setPreviousState("rest");
    setViewModeSignal("standard");
    setRenderModeSignal("auto");
    setDockRevealedSignal(false);
    setDockPinnedSignal(false);
    setOrbitEngagedSignal(false);
    setCompanionBrightenedSignal(false);
    setCompanionPositionSignal(undefined);
    setSharedContextSignal({ threadId: null, coreState: null, draft: "", focusSubjectId: null });
  });
  blockedReturnState = "rest";
  modeReturnState = "rest";
  pendingViewMode = null;
}

// ─── Export ────────────────────────────────────────────────────────────────

export const homeStore = {
  // Read-only signals / derived
  state,
  previousState,
  viewMode,
  renderMode,
  dockRevealed,
  dockPinned,
  orbitEngaged,
  companion,
  readingMode,
  sharedContext,
  isTransient,
  focusTarget,
  canTransition,

  // State-machine actions
  transition,
  engage,
  rest,
  enterReading,
  exitReading,
  beginModeTransition,
  completeModeTransition,
  enterCompanion,
  exitCompanion,
  setCompanionBrightened,
  setCompanionPosition,
  enterBlocked,
  exitBlocked,

  // Local UI slice setters
  setViewMode,
  setRenderMode,
  setDockRevealed,
  setDockPinned,
  setOrbitEngaged,

  // Shared-context preservation
  updateSharedContext,

  // Lifecycle
  reset,
} as const;
