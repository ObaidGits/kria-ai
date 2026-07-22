/**
 * Pinned-item ownership for the Command Palette (Req 14.1, 14.3).
 *
 * The hybrid navigation model (design.md §7) makes the Command Palette the
 * single owner of global search, recent, AND pinned items — NOT standing
 * homepage UI (Req 14.3). Recents live in `recents.ts`; pins live here.
 *
 * Pinning is deliberately a *thin palette-scoped wrapper* over the shared,
 * bounded, persisted adaptive-ranking module's `palette` zone (the same store
 * that already backs recent-use promotion via `recencyBoost`). Delegating keeps
 * one persistence surface and one ranking pipeline: `searchItems` calls
 * `rankPaletteCandidates`, which promotes pinned ids ahead of unpinned ones
 * (bounded, ordering-only — a pin never hides or invokes an item, Req 19.1).
 *
 * This module adds NO UI. It exposes the pin contract so the palette (and only
 * the palette) owns pinned navigation targets; a feature Space can pin/unpin a
 * target id through this API without any standing homepage affordance.
 */
import {
  isAdaptivePinned,
  setAdaptivePinned,
  resetAdaptiveSuggestions,
} from "../adaptive";

/** The adaptive zone that backs palette pins (shared with recents ranking). */
const PALETTE_ZONE = "palette" as const;

/** Whether a palette item id is currently pinned by the user. */
export function isPinned(id: string): boolean {
  if (!id) return false;
  return isAdaptivePinned(PALETTE_ZONE, id);
}

/** Pin a palette item id so it ranks ahead of unpinned matches. */
export function pinItem(id: string): void {
  if (!id) return;
  setAdaptivePinned(PALETTE_ZONE, id, true);
}

/** Unpin a previously pinned palette item id. */
export function unpinItem(id: string): void {
  if (!id) return;
  setAdaptivePinned(PALETTE_ZONE, id, false);
}

/** Toggle the pinned state of a palette item id. Returns the new state. */
export function togglePin(id: string): boolean {
  if (!id) return false;
  const next = !isPinned(id);
  setAdaptivePinned(PALETTE_ZONE, id, next);
  return next;
}

/**
 * Reset the palette adaptive zone (pins + recency ranking). Pins and recents
 * share one persisted zone, so this clears both — primarily used by tests and
 * the palette's existing "Reset ranking" control.
 */
export function clearPins(): void {
  resetAdaptiveSuggestions(PALETTE_ZONE);
}
