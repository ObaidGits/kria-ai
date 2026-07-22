/**
 * Mode-Transition Coordinator — the single semantic entry point for switching
 * the canonical View Mode axis (Immersive / Standard / Mini / Companion) with
 * the Core as the continuity anchor and full shared-state preservation
 * (task 8.2; design §8.1; Requirements 13.2, 13.3, 13.5).
 *
 * ── Why a coordinator ────────────────────────────────────────────────────────
 * Three things must happen together, in order, for a mode switch to be
 * *continuous* rather than a hard reload:
 *   1. Shared state (active thread, Core-state snapshot, Composer draft, bound
 *      Focus subject) is captured into `homeStore.sharedContext` BEFORE anything
 *      changes, so it survives the switch by construction (Req 13.3).
 *   2. The homepage enters the transient `mode-transition` macro state, whose
 *      canonical focus target is the Core — the Core morphs/relocates rather
 *      than disappearing, i.e. it is the continuity anchor (Req 13.2).
 *   3. The native window presentation (`shellStore.setWindowMode`, applied by
 *      `windowModeManager`: fullscreen/geometry) changes, then the transition
 *      settles and the staged View Mode is applied + the prior stable macro
 *      state is restored (`homeStore.completeModeTransition`).
 *
 * Callers (palette `cmd.mode.*`, `WindowModeSwitch`, the Immersive→Standard
 * Escape path) all funnel through {@link requestWindowMode}, so EVERY
 * keyboard/palette/AT trigger (Req 13.5) is continuous and state-preserving —
 * never a bare `shellStore.setWindowMode` that would skip the continuity anchor.
 *
 * ── Authority invariants (Req 29 / 30.3, guardrails.md "Never") ──────────────
 * • `coreStore` is the SOLE authority for Core state. This coordinator only
 *   READS `coreStore.state()` to snapshot it into the preserved shared context;
 *   it NEVER writes `coreStore`. The Core stays visually continuous precisely
 *   because nothing resets it across the switch.
 * • Mode switching is presentation-only: no domain store (converse/approval/…)
 *   is reset, so conversations persist across all modes (Req 13.4).
 *
 * ── Reduced-motion (Req 17.4 / 20.3, guardrails.md "Always") ─────────────────
 * Under `prefers-reduced-motion` (OS query or the global kill-switch's
 * `data-reduced-motion="on"` stamp) the "continuous" animation degrades to an
 * instant switch: the transition settles synchronously with no dwell and no
 * cross-fade, while still preserving every piece of shared state.
 */
import { shellStore, type WindowMode } from "../stores/shellStore";
import { homeStore } from "../stores/homeStore";
import { coreStore } from "../stores/coreStore";
import { converseStore } from "../stores/converseStore";
import { homeFocusStore } from "../stores/homeFocusStore";
import { REDUCED_MOTION_QUERY } from "../platform/motion";

/**
 * Continuous mode-transition dwell (ms). Kept short and in the presence-motion
 * band so the Core reads as morphing/relocating, not reloading. Reduced-motion
 * collapses this to 0 (instant switch).
 */
export const MODE_TRANSITION_MS = 260;

export interface RequestWindowModeOptions {
  /** Override the continuous-animation dwell (ms). Tests inject 0/custom. */
  durationMs?: number;
  /**
   * Force the reduced-motion instant switch regardless of environment. When
   * omitted the environment is probed via {@link prefersReducedMotion}.
   */
  reducedMotion?: boolean;
}

/** Handle for the in-flight settle timer, so a new request can supersede it. */
let pendingTimer: ReturnType<typeof setTimeout> | undefined;

/**
 * Synchronous reduced-motion probe. Prefers the global controller's
 * `data-reduced-motion="on"` root stamp (kill-switch OR OS reduction, whichever
 * won), falling back to the OS media query directly. Safe on jsdom/no-DOM.
 */
export function prefersReducedMotion(): boolean {
  if (typeof document !== "undefined" && document.documentElement?.dataset.reducedMotion === "on") {
    return true;
  }
  if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
    try {
      return window.matchMedia(REDUCED_MOTION_QUERY).matches;
    } catch {
      // jsdom without a matchMedia implementation — treat as motion-allowed.
    }
  }
  return false;
}

/**
 * The currently-bound Focus subject id (Voice Line / ACS share one subject by
 * construction, so either yields the same id). `null` when the homepage rests.
 * Read-only — the Focus engine is a pure read-model (Req 30.3).
 */
function currentFocusSubjectId(): string | null {
  const frame = homeFocusStore.frame();
  return frame.voiceLine?.subjectId ?? frame.acs?.subjectId ?? null;
}

/**
 * Snapshot the live shared state into `homeStore.sharedContext`. This is the
 * preservation guarantee for a transition: the values are copied BEFORE the
 * switch, and `homeStore` transitions never touch the shared context, so the
 * active thread, Core-state snapshot, draft, and Focus subject all survive
 * (Req 13.3 / 30.1). Exposed for deterministic unit coverage.
 */
export function captureSharedContext(): void {
  homeStore.updateSharedContext({
    threadId: converseStore.activeThreadId(),
    // A copied VALUE — coreStore stays the authority; never written back.
    coreState: coreStore.state(),
    draft: converseStore.composerDraft().text,
    focusSubjectId: currentFocusSubjectId(),
  });
}

/**
 * Settle any in-flight transition immediately: cancel the pending dwell timer
 * and, if the machine is still in `mode-transition`, complete it (apply the
 * staged View Mode + restore the prior stable macro state / resolve to
 * companion). Idempotent; safe to call when nothing is pending.
 */
export function settlePendingModeTransition(): void {
  if (pendingTimer !== undefined) {
    clearTimeout(pendingTimer);
    pendingTimer = undefined;
  }
  if (homeStore.state() === "mode-transition") {
    homeStore.completeModeTransition();
  }
}

/**
 * Request a continuous switch to `target`. Returns `false` (no-op) when already
 * in that mode. Otherwise: settles any prior in-flight transition, captures
 * shared state, enters the Core-anchored `mode-transition` state, applies the
 * native window presentation, then settles — instantly under reduced-motion or
 * after {@link MODE_TRANSITION_MS} otherwise.
 *
 * This is the ONLY sanctioned way to change the View Mode from the UI so every
 * trigger (keyboard/palette/AT) is continuous and preserves shared state.
 */
export function requestWindowMode(target: WindowMode, opts?: RequestWindowModeOptions): boolean {
  if (target === shellStore.windowMode()) return false;

  // A prior transition still settling? Finish it first so the machine is in a
  // stable state — beginning a transition from `mode-transition` is a no-op.
  settlePendingModeTransition();

  // 1) Preserve live shared state BEFORE anything changes (Req 13.3).
  captureSharedContext();

  // 2) Core becomes the continuity anchor: macro state → `mode-transition`,
  //    focus target → `core`, target View Mode staged (Req 13.2).
  homeStore.beginModeTransition(target);

  // 3) Native presentation: fullscreen/geometry via windowModeManager. This is
  //    presentation-only and never resets a domain store (conversations persist).
  shellStore.setWindowMode(target);

  // 4) Settle. Reduced-motion → instant, state still preserved (Req 17.4/20.3).
  const reduced = opts?.reducedMotion ?? prefersReducedMotion();
  const duration = reduced ? 0 : opts?.durationMs ?? MODE_TRANSITION_MS;

  if (duration <= 0 || typeof setTimeout === "undefined") {
    settlePendingModeTransition();
  } else {
    pendingTimer = setTimeout(() => {
      pendingTimer = undefined;
      homeStore.completeModeTransition();
    }, duration);
  }
  return true;
}

/**
 * Sync `homeStore.viewMode` to the shell's resolved window mode at boot so the
 * two never start out of step (the shell restores its mode from storage while
 * `homeStore` defaults to "standard"). Presentation-only; touches no domain
 * state and starts no transition.
 */
export function syncViewModeFromShell(): void {
  const mode = shellStore.windowMode();
  if (homeStore.viewMode() !== mode) homeStore.setViewMode(mode);
}
