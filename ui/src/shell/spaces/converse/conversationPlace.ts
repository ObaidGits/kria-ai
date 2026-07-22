/**
 * Conversation place — the SINGLE anchor-based restoration owner for the
 * virtualized message stream (design.md §20 Place tolerance, §21 IU-10 /
 * UIE-M-005; task 9.3, gaps G2/G3/G4).
 *
 * Why this exists (the core UIE-M-005 violation from the 9.1 inventory):
 * three paths used to write `.kria-stream__viewport.scrollTop` in raw pixels —
 * P-A (mode `capturePlace`), P-B (approval `capturePlace`), and P-C
 * (MessageStream follow). For a `@tanstack/solid-virtual` list with DYNAMIC
 * `measureElement` row heights, raw `scrollTop` does not map back to the same
 * message after a reversible transition (`getTotalSize()`/row `start` recompute)
 * → "shell scroll restoration can compete with virtualization" (UIE-M-005
 * Technical impact). §21 IU-10 requires "coordinate ONE restoration path".
 *
 * Contract this module owns:
 *   • ONE owner of the conversation viewport's restoration. MessageStream (which
 *     holds the virtualizer instance) registers an imperative handle here via
 *     `registerConversationPlaceOwner`; it is the only writer of that viewport.
 *   • Anchor + offset, NOT raw px: capture `{ activeThreadId, anchorMessageId,
 *     offsetWithinItem, atBottom }` — the topmost visible message (or the focused
 *     message if a message action holds focus) and the intra-item offset
 *     (viewport-top minus that row's top). Restore by `scrollToIndex(indexOf(
 *     anchorMessageId))` then adjusting by `offsetWithinItem`, landing within
 *     tolerance = min(one rendered item height, 24 CSS px) (§20 Place tolerance).
 *   • atBottom reconciliation (stick vs anchor): if the user was at the bottom
 *     (follow tail), restore preserves bottom; otherwise restore lands on the
 *     anchor and never force-scrolls to bottom.
 *   • A begin/end coordinator so overlapping transitions (a Window Mode change
 *     coinciding with a pending approval) capture ONCE and restore the stream
 *     EXACTLY ONCE — never double-restore (§21 IU-10 "one restoration path").
 *
 * P-A (AppShell mode place) and P-B (approvalPlace) DEFER the conversation
 * viewport to this owner and keep restoring only focus + caret + the
 * non-virtualized lane scrollers (threads/Work/Context/Inspector). The stream
 * viewport is excluded from `collectScrollables` (see placePreservation.ts) so
 * they no longer blanket-restore it in raw px.
 *
 * Pure module (no framework coupling); the anchor math is exported as pure
 * functions so it is deterministic under jsdom (which lacks real layout).
 *
 * Requirements: 10.4, 11.6, 11.7, 15.x, 16.4, 21.6–21.8 (design §20, §21 IU-10)
 */

/** Marks the scroll element this module owns; used to exclude it elsewhere. */
export const CONVERSATION_SCROLL_OWNER_ATTR = "data-scroll-owner";
export const CONVERSATION_SCROLL_OWNER_VALUE = "conversation";
/** Class of the conversation viewport (belt-and-braces exclusion match). */
export const CONVERSATION_VIEWPORT_CLASS = "kria-stream__viewport";
/** Design §20 hard ceiling for the place tolerance (CSS px). */
export const PLACE_TOLERANCE_MAX_PX = 24;

/** A restorable conversation anchor (message-id + intra-item offset). */
export interface ConversationAnchor {
  /** Active thread the anchor belongs to (guards cross-thread restores). */
  activeThreadId: string | null;
  /** Topmost visible (or focused) message id; null when the stream is empty. */
  anchorMessageId: string | null;
  /** Intra-item offset: viewport-top minus the anchor row's top (px, ≥ 0). */
  offsetWithinItem: number;
  /** Whether the user was pinned to the bottom (follow tail) at capture. */
  atBottom: boolean;
  /** The anchor row's measured height at capture (for the tolerance ceiling). */
  anchorItemHeight: number;
}

/** A rendered virtual row, reduced to what the anchor math needs. */
export interface RenderedRow {
  index: number;
  id: string;
  /** Row top within the scrolled content (virtualizer `start`). */
  start: number;
  /** Row bottom within the scrolled content (virtualizer `end`). */
  end: number;
}

/** The imperative handle MessageStream registers as the single owner. */
export interface ConversationPlaceOwner {
  /** Capture the current anchor, or null when there is nothing to anchor. */
  capture(): ConversationAnchor | null;
  /** Restore a previously captured anchor (no-op for null/foreign thread). */
  restore(anchor: ConversationAnchor | null): void;
}

// ─── Pure anchor math (deterministic, jsdom-safe) ────────────────────────────

/**
 * Place tolerance for a landed restore: min(one rendered item height, 24 CSS
 * px), whichever is smaller (design §20). A non-positive/NaN item height falls
 * back to the 24px ceiling so the tolerance is always meaningful.
 */
export function placeTolerancePx(renderedItemHeight: number): number {
  if (!Number.isFinite(renderedItemHeight) || renderedItemHeight <= 0) {
    return PLACE_TOLERANCE_MAX_PX;
  }
  return Math.min(renderedItemHeight, PLACE_TOLERANCE_MAX_PX);
}

/** Whether a landed scroll position is within tolerance of the target (§20). */
export function isWithinPlaceTolerance(
  landedTop: number,
  targetTop: number,
  renderedItemHeight: number,
): boolean {
  return Math.abs(landedTop - targetTop) <= placeTolerancePx(renderedItemHeight);
}

/**
 * Pick the anchor row for a viewport: the row that covers the viewport top
 * (`start <= viewportTop < end`); else the first row starting at/after the
 * viewport top; else the last rendered row. Returns null when no rows render.
 */
export function pickAnchorRow(rows: RenderedRow[], viewportTop: number): RenderedRow | null {
  if (rows.length === 0) return null;
  const ordered = [...rows].sort((a, b) => a.start - b.start);
  const covering = ordered.find((r) => r.start <= viewportTop && viewportTop < r.end);
  if (covering) return covering;
  const below = ordered.find((r) => r.start >= viewportTop);
  return below ?? ordered[ordered.length - 1];
}

export interface AnchorInputs {
  activeThreadId: string | null;
  rows: RenderedRow[];
  viewportTop: number;
  atBottom: boolean;
  /** Message with focus inside the stream, if a message action holds focus. */
  focusedMessageId?: string | null;
}

/**
 * Compute the conversation anchor. A focused message (a message action holding
 * focus) takes precedence over the topmost visible row so the user's active
 * message is what is restored; otherwise the topmost visible row is the anchor.
 * `offsetWithinItem` is clamped to ≥ 0.
 */
export function computeConversationAnchor(input: AnchorInputs): ConversationAnchor {
  const { activeThreadId, rows, viewportTop, atBottom, focusedMessageId } = input;

  const focusedRow =
    focusedMessageId != null ? rows.find((r) => r.id === focusedMessageId) ?? null : null;
  const anchorRow = focusedRow ?? pickAnchorRow(rows, viewportTop);

  if (!anchorRow) {
    return {
      activeThreadId,
      anchorMessageId: null,
      offsetWithinItem: 0,
      atBottom,
      anchorItemHeight: 0,
    };
  }

  return {
    activeThreadId,
    anchorMessageId: anchorRow.id,
    offsetWithinItem: Math.max(0, viewportTop - anchorRow.start),
    atBottom,
    anchorItemHeight: Math.max(0, anchorRow.end - anchorRow.start),
  };
}

/** A deterministic restore plan the owner executes against the virtualizer. */
export type RestorePlan =
  | { kind: "noop" }
  | { kind: "bottom" }
  | { kind: "anchor"; index: number; offsetWithinItem: number; anchorItemHeight: number };

/**
 * Resolve the restore plan from an anchor (atBottom reconciliation, §20/§21):
 *   • atBottom → scroll to bottom (preserve follow tail).
 *   • anchored mid-thread with a resolvable message → land on that index +
 *     offset (never force-scroll to bottom).
 *   • otherwise (no anchor, or the anchored message no longer exists — e.g.
 *     removed while away) → noop; target-removal fallback is task 9.4.
 */
export function resolveConversationRestore(
  anchor: ConversationAnchor | null | undefined,
  indexOfMessage: (id: string) => number,
): RestorePlan {
  if (!anchor) return { kind: "noop" };
  if (anchor.atBottom) return { kind: "bottom" };
  if (anchor.anchorMessageId == null) return { kind: "noop" };
  const index = indexOfMessage(anchor.anchorMessageId);
  if (index < 0) return { kind: "noop" };
  return {
    kind: "anchor",
    index,
    offsetWithinItem: anchor.offsetWithinItem,
    anchorItemHeight: anchor.anchorItemHeight,
  };
}

// ─── Single-owner registry ───────────────────────────────────────────────────

let owner: ConversationPlaceOwner | null = null;

/**
 * Register the single conversation viewport restoration owner (MessageStream).
 * Returns a disposer that clears it only if still current (safe under
 * remount/hot-reload). Registering a second owner replaces the first — there is
 * only ever ONE writer of the stream viewport.
 */
export function registerConversationPlaceOwner(next: ConversationPlaceOwner): () => void {
  owner = next;
  return () => {
    if (owner === next) owner = null;
  };
}

/** Capture the conversation anchor from the registered owner (null if none). */
export function captureConversationPlace(): ConversationAnchor | null {
  return owner ? owner.capture() : null;
}

/** Restore the conversation anchor through the registered owner. */
export function restoreConversationPlace(anchor: ConversationAnchor | null): void {
  owner?.restore(anchor);
}

// ─── Overlapping-transition coordinator (restore exactly once) ───────────────
//
// P-A (mode) and P-B (approval) both delegate the conversation viewport here.
// A Window Mode transition can coincide with a pending approval, so both would
// otherwise capture and restore the stream. This depth-counted coordinator
// captures ONCE (on the outermost begin, before any disruption) and restores
// EXACTLY ONCE (when the last transition settles), satisfying §21 IU-10's "one
// restoration path". A late begin while a capture is already held does not
// re-capture (the earliest place is the truthful pre-disruption place).

let captureDepth = 0;
let pendingSnapshot: ConversationAnchor | null = null;
let restoreCount = 0;

/**
 * Begin a coordinated conversation-place transition. Captures the anchor once,
 * on the first (outermost) concurrent begin.
 */
export function beginConversationPlace(): void {
  if (captureDepth === 0) pendingSnapshot = captureConversationPlace();
  captureDepth += 1;
}

/**
 * End a coordinated conversation-place transition. When the last concurrent
 * transition ends, restore the captured anchor exactly once.
 */
export function endConversationPlace(): void {
  if (captureDepth === 0) return;
  captureDepth -= 1;
  if (captureDepth > 0) return;
  const snap = pendingSnapshot;
  pendingSnapshot = null;
  restoreCount += 1;
  restoreConversationPlace(snap);
}

/** Test-only: how many times the coordinator has issued a restore. */
export function __conversationRestoreCount(): number {
  return restoreCount;
}

/** Test-only: reset coordinator + owner state between cases. */
export function __resetConversationPlace(): void {
  owner = null;
  captureDepth = 0;
  pendingSnapshot = null;
  restoreCount = 0;
}
