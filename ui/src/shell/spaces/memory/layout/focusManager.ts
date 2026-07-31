/**
 * memory/layout/focusManager — Focus management constants and pure state functions.
 *
 * Pure TypeScript module — no DOM, no side effects.
 *
 * Manages focus guard state for dialogs, drawers, and sheets:
 * initial focus, containment, Escape handling, initiator restoration,
 * and per-window focus isolation under patch/remove races.
 *
 * IDs: MGR-013–016, MGR-022; MGD-013–014; MG-H11–H12.
 */

// ─── Constants ────────────────────────────────────────────────────────────────

/**
 * Focus return delay in milliseconds.
 * Implemented as setTimeout(fn, 0) — synchronous-next-tick via the event loop.
 * This ensures the DOM has settled before attempting focus restoration.
 */
export const FOCUS_RETURN_DELAY_MS = 0;

// ─── FocusGuard interface ─────────────────────────────────────────────────────

/**
 * Immutable focus guard state for a managed container (dialog, drawer, sheet).
 *
 * containerId    — the ID of the container element being managed
 * lastFocusedId  — the ID of the element that last had focus within or
 *                  before the container; null if not yet recorded
 */
export interface FocusGuard {
  containerId: string;
  lastFocusedId: string | null;
}

// ─── Pure state functions ─────────────────────────────────────────────────────

/**
 * Creates a new FocusGuard for the given container ID.
 * lastFocusedId starts as null.
 */
export function createFocusGuard(containerId: string): FocusGuard {
  return { containerId, lastFocusedId: null };
}

/**
 * Returns a new FocusGuard with lastFocusedId set to focusedId.
 * Does not mutate the input guard.
 */
export function recordFocus(guard: FocusGuard, focusedId: string): FocusGuard {
  return { ...guard, lastFocusedId: focusedId };
}

/**
 * Returns a new FocusGuard with lastFocusedId set to null.
 * Does not mutate the input guard.
 */
export function clearFocus(guard: FocusGuard): FocusGuard {
  return { ...guard, lastFocusedId: null };
}

/**
 * Returns the element ID that focus should return to when the container closes.
 * Returns null when no focus has been recorded.
 */
export function getReturnTarget(guard: FocusGuard): string | null {
  return guard.lastFocusedId;
}
