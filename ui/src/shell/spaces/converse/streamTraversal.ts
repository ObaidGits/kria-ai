/**
 * streamTraversal — pure keyboard/focus traversal contract for the virtualized
 * conversation viewport (design.md §21 UIE-M-005; task 9.6, gap G7).
 *
 * The 9.1 inventory found the stream viewport had `onScroll` + follow but NO
 * keyboard traversal contract and was not keyboard-focusable → an unproven
 * focus/scroll trap. This module owns the DETERMINISTIC decision logic so the
 * behavior is jsdom-testable (jsdom has no real scroll layout): MessageStream
 * wires these pure functions to the virtualizer + scroll element.
 *
 * Contract (each key → behavior):
 *   • Home       → jump to first message (top); follow/stick released.
 *   • End        → jump to latest message (bottom); follow/stick re-engaged.
 *   • PageUp     → scroll up ≈ one viewport height (bounded at 0).
 *   • PageDown   → scroll down ≈ one viewport height (bounded at max).
 *   • Tab        → NOT intercepted (moves focus out naturally — no focus trap;
 *                  this is a non-modal reading region, not a dialog).
 *   • any key while focus is inside an editable control → NOT intercepted
 *                  (never hijack a text field's caret / a child action's keys).
 *   • wheel / touchpad → native `overflow-y:auto` (no handler, no preventDefault
 *                  → no scroll-chaining trap). Documented, not code here.
 *
 * Focus reveal: when a control inside an off-screen (not fully visible) message
 * receives focus, the viewport reveals that row so the focused control is
 * visible — no "focused but invisible" trap. Revealing a non-last message
 * releases stick; End re-engages it.
 *
 * Requirements: 10.11, 12.11, 19.1–19.7 (design §21 UIE-M-005 Acceptance).
 */

/** A resolved traversal action for a keydown on the viewport. */
export type TraversalIntent =
  | { kind: "none" }
  | { kind: "top" }
  | { kind: "bottom" }
  | { kind: "page"; direction: -1 | 1 };

export interface TraversalContext {
  /** `KeyboardEvent.key`. */
  key: string;
  /**
   * Whether focus is inside an editable control (input/textarea/select/
   * contenteditable). When true we defer ALL keys to that control — Page/Home/
   * End must move its caret, never hijack the scroll (a11y, Req 12.11).
   */
  editableFocus: boolean;
}

/**
 * Resolve what a keydown should do. Only Page/Home/End act, and only when focus
 * is not inside an editable control. Everything else (Tab, arrows, Enter, …)
 * returns `none` so it passes through untouched — no focus trap, no key hijack.
 */
export function resolveTraversalIntent(ctx: TraversalContext): TraversalIntent {
  if (ctx.editableFocus) return { kind: "none" };
  switch (ctx.key) {
    case "Home":
      return { kind: "top" };
    case "End":
      return { kind: "bottom" };
    case "PageUp":
      return { kind: "page", direction: -1 };
    case "PageDown":
      return { kind: "page", direction: 1 };
    default:
      return { kind: "none" };
  }
}

/**
 * Next `scrollTop` after a Page key: current ± one viewport height, bounded to
 * `[0, scrollHeight - clientHeight]` so Page keys never overscroll past the
 * ends (no bounce, no trap). Non-finite inputs collapse to a safe 0.
 */
export function computePageScrollTop(
  current: number,
  clientHeight: number,
  scrollHeight: number,
  direction: -1 | 1,
): number {
  const safeCurrent = Number.isFinite(current) ? current : 0;
  const page = Number.isFinite(clientHeight) ? clientHeight : 0;
  const max = Math.max(0, (Number.isFinite(scrollHeight) ? scrollHeight : 0) - page);
  const next = safeCurrent + direction * page;
  return Math.min(max, Math.max(0, next));
}

/**
 * Whether a focused message's row must be revealed (scrolled into view). A row
 * with no geometry (not currently rendered) is always revealed by index; a
 * rendered row is revealed only when it is not fully within the visible band
 * `[viewportTop, viewportTop + viewportHeight]` — so focusing an already-visible
 * message never fights the follow/stick logic.
 */
export function shouldRevealFocusedRow(
  focusedRow: { start: number; end: number } | null,
  viewportTop: number,
  viewportHeight: number,
): boolean {
  if (!focusedRow) return true;
  const top = Number.isFinite(viewportTop) ? viewportTop : 0;
  const bottom = top + (Number.isFinite(viewportHeight) ? viewportHeight : 0);
  return focusedRow.start < top || focusedRow.end > bottom;
}

/**
 * Whether an element (typically a keydown target) is an editable control whose
 * keys must not be hijacked by the viewport traversal handler.
 */
export function isEditableTarget(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  const tag = el.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  return el.isContentEditable === true;
}

// ── Focus preservation across virtualization row reuse (task 11.6, gap G5) ────
// The virtualizer only mounts the visible window (+overscan): a focused message
// row can UNMOUNT when the user scrolls it out of view, and its DOM node can be
// REUSED for a different message. When that happens the browser drops focus to
// `document.body` (a "focused but no longer present" loss) instead of returning
// it to the message when the row comes back. These two pure decisions let
// MessageStream queue a restore only for that specific loss and replay it once
// the owning message's row is rendered again — without ever fighting an
// intentional focus move (Tab-away, click elsewhere) or the reveal/follow logic.
// Requirements: 16.4, 12.11 (design §16 Accessibility Plan; UIE-M-007/008/009).

/**
 * Whether a `focusout` from the viewport represents focus LOST because the
 * focused message row unmounted (virtualized away), and therefore a restore
 * should be queued for that message id.
 *
 * A restore is queued ONLY when all hold:
 *   • `relatedTargetPresent` is false — focus moved to nothing (unmount drop),
 *     not to another element (an intentional Tab-away / click keeps focus and
 *     must never be yanked back).
 *   • `lastFocusedId` is a known message — something in a row had focus.
 *   • `lastFocusedRowStillRendered` is false — that row is gone from the DOM, so
 *     the loss is an unmount, not a focus move within a still-present row.
 */
export function shouldQueueFocusRestore(params: {
  relatedTargetPresent: boolean;
  lastFocusedId: string | null;
  lastFocusedRowStillRendered: boolean;
}): boolean {
  const { relatedTargetPresent, lastFocusedId, lastFocusedRowStillRendered } = params;
  if (relatedTargetPresent) return false;
  if (lastFocusedId == null) return false;
  if (lastFocusedRowStillRendered) return false;
  return true;
}

/**
 * Whether a queued focus restore should now be performed. Restores focus to the
 * remembered message's row ONLY when:
 *   • `pendingId` names a message awaiting restore,
 *   • `isRendered` — that message's row is mounted again (reused/revealed), and
 *   • `focusInsideViewport` is false — focus is still dropped (e.g. on body); if
 *     focus already landed somewhere valid inside the viewport we must not steal
 *     it back.
 */
export function shouldPerformFocusRestore(params: {
  pendingId: string | null;
  isRendered: boolean;
  focusInsideViewport: boolean;
}): boolean {
  const { pendingId, isRendered, focusInsideViewport } = params;
  if (pendingId == null) return false;
  if (focusInsideViewport) return false;
  return isRendered;
}
