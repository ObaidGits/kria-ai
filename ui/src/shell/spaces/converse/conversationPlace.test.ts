/**
 * conversationPlace tests (task 9.3; design §20 Place tolerance, §21 IU-10 /
 * UIE-M-005).
 *
 * Proves the SINGLE anchor-based restoration owner:
 *   • anchor math: topmost/focused message + intra-item offset;
 *   • tolerance = min(one rendered item, 24 CSS px);
 *   • atBottom reconciliation (stick vs anchor);
 *   • single-owner registry + delegation;
 *   • the begin/end coordinator restores the stream EXACTLY ONCE even when a
 *     mode change coincides with a pending approval.
 *
 * jsdom lacks real layout, so the owner is stubbed and the anchor logic is
 * asserted deterministically (indexOf + offset + tolerance), per the task.
 */
import { describe, it, expect, afterEach } from "vitest";
import {
  PLACE_TOLERANCE_MAX_PX,
  beginConversationPlace,
  captureConversationPlace,
  computeConversationAnchor,
  endConversationPlace,
  isWithinPlaceTolerance,
  pickAnchorRow,
  placeTolerancePx,
  registerConversationPlaceOwner,
  resolveConversationRestore,
  restoreConversationPlace,
  __conversationRestoreCount,
  __resetConversationPlace,
  type ConversationAnchor,
  type ConversationPlaceOwner,
  type RenderedRow,
} from "./conversationPlace";

afterEach(() => __resetConversationPlace());

const rows: RenderedRow[] = [
  { index: 0, id: "m0", start: 0, end: 100 },
  { index: 1, id: "m1", start: 100, end: 260 }, // 160px tall
  { index: 2, id: "m2", start: 260, end: 300 }, // 40px tall
  { index: 3, id: "m3", start: 300, end: 420 },
];

describe("place tolerance (design §20: min(one rendered item, 24px))", () => {
  it("caps a tall item at the 24px ceiling", () => {
    expect(placeTolerancePx(160)).toBe(PLACE_TOLERANCE_MAX_PX);
  });

  it("uses the item height when it is smaller than 24px", () => {
    expect(placeTolerancePx(16)).toBe(16);
  });

  it("falls back to the 24px ceiling for a non-positive/NaN height", () => {
    expect(placeTolerancePx(0)).toBe(24);
    expect(placeTolerancePx(NaN)).toBe(24);
  });

  it("accepts a landed position within tolerance and rejects one outside", () => {
    // 160px item → tolerance 24px.
    expect(isWithinPlaceTolerance(120, 100, 160)).toBe(true);
    expect(isWithinPlaceTolerance(130, 100, 160)).toBe(false);
    // 40px item → tolerance 24px (item > 24 → capped).
    expect(isWithinPlaceTolerance(283, 260, 40)).toBe(true);
    expect(isWithinPlaceTolerance(285, 260, 40)).toBe(false);
  });
});

describe("pickAnchorRow", () => {
  it("returns the row covering the viewport top", () => {
    expect(pickAnchorRow(rows, 150)?.id).toBe("m1");
  });

  it("returns the first row at/after the top when none covers it", () => {
    // viewportTop exactly at a boundary (start of m2)
    expect(pickAnchorRow(rows, 260)?.id).toBe("m2");
  });

  it("returns null for an empty stream", () => {
    expect(pickAnchorRow([], 0)).toBeNull();
  });
});

describe("computeConversationAnchor", () => {
  it("anchors on the topmost visible message + intra-item offset", () => {
    const anchor = computeConversationAnchor({
      activeThreadId: "t1",
      rows,
      viewportTop: 140,
      atBottom: false,
    });
    expect(anchor.anchorMessageId).toBe("m1");
    expect(anchor.offsetWithinItem).toBe(40); // 140 - 100
    expect(anchor.anchorItemHeight).toBe(160);
    expect(anchor.atBottom).toBe(false);
    expect(anchor.activeThreadId).toBe("t1");
  });

  it("prefers a focused message over the topmost visible row", () => {
    const anchor = computeConversationAnchor({
      activeThreadId: "t1",
      rows,
      viewportTop: 140, // topmost would be m1
      atBottom: false,
      focusedMessageId: "m3",
    });
    expect(anchor.anchorMessageId).toBe("m3");
    expect(anchor.offsetWithinItem).toBe(0); // 140 - 300 clamped to >= 0
  });

  it("records atBottom and a null anchor for an empty stream", () => {
    const anchor = computeConversationAnchor({
      activeThreadId: null,
      rows: [],
      viewportTop: 0,
      atBottom: true,
    });
    expect(anchor.anchorMessageId).toBeNull();
    expect(anchor.atBottom).toBe(true);
  });
});

describe("resolveConversationRestore (atBottom reconciliation, §20/§21)", () => {
  const indexOf = (id: string) => rows.findIndex((r) => r.id === id);

  it("captured-at-bottom → restore scrolls to bottom", () => {
    const anchor: ConversationAnchor = {
      activeThreadId: "t1",
      anchorMessageId: "m1",
      offsetWithinItem: 40,
      atBottom: true,
      anchorItemHeight: 160,
    };
    expect(resolveConversationRestore(anchor, indexOf)).toEqual({ kind: "bottom" });
  });

  it("captured-mid → restore lands on the anchor index + offset (not bottom)", () => {
    const anchor: ConversationAnchor = {
      activeThreadId: "t1",
      anchorMessageId: "m1",
      offsetWithinItem: 40,
      atBottom: false,
      anchorItemHeight: 160,
    };
    expect(resolveConversationRestore(anchor, indexOf)).toEqual({
      kind: "anchor",
      index: 1,
      offsetWithinItem: 40,
      anchorItemHeight: 160,
    });
  });

  it("noop when the anchored message no longer exists (removal → task 9.4)", () => {
    const anchor: ConversationAnchor = {
      activeThreadId: "t1",
      anchorMessageId: "gone",
      offsetWithinItem: 10,
      atBottom: false,
      anchorItemHeight: 96,
    };
    expect(resolveConversationRestore(anchor, indexOf)).toEqual({ kind: "noop" });
  });

  it("noop for a null anchor", () => {
    expect(resolveConversationRestore(null, indexOf)).toEqual({ kind: "noop" });
  });
});

/** A minimal owner stub that records capture/restore calls. */
function stubOwner(anchor: ConversationAnchor | null): ConversationPlaceOwner & {
  captures: number;
  restores: Array<ConversationAnchor | null>;
} {
  const state = {
    captures: 0,
    restores: [] as Array<ConversationAnchor | null>,
    capture() {
      state.captures += 1;
      return anchor;
    },
    restore(a: ConversationAnchor | null) {
      state.restores.push(a);
    },
  };
  return state;
}

describe("single-owner registry", () => {
  it("delegates capture/restore to the registered owner", () => {
    const anchor: ConversationAnchor = {
      activeThreadId: "t1",
      anchorMessageId: "m2",
      offsetWithinItem: 5,
      atBottom: false,
      anchorItemHeight: 40,
    };
    const owner = stubOwner(anchor);
    registerConversationPlaceOwner(owner);

    expect(captureConversationPlace()).toBe(anchor);
    restoreConversationPlace(anchor);
    expect(owner.restores).toEqual([anchor]);
  });

  it("no-ops safely when no owner is registered", () => {
    expect(captureConversationPlace()).toBeNull();
    expect(() => restoreConversationPlace(null)).not.toThrow();
  });

  it("disposer clears only the current owner", () => {
    const a = stubOwner(null);
    const dispose = registerConversationPlaceOwner(a);
    const b = stubOwner(null);
    registerConversationPlaceOwner(b); // replaces a
    dispose(); // must NOT clear b
    captureConversationPlace();
    expect(b.captures).toBe(1);
  });
});

describe("coordinator: restore EXACTLY ONCE under overlapping transitions (§21 IU-10)", () => {
  it("mode-change coinciding with a pending approval restores the stream once", () => {
    const anchor: ConversationAnchor = {
      activeThreadId: "t1",
      anchorMessageId: "m1",
      offsetWithinItem: 40,
      atBottom: false,
      anchorItemHeight: 160,
    };
    const owner = stubOwner(anchor);
    registerConversationPlaceOwner(owner);

    // Overlap: mode-changing begins, approval becomes pending, mode settles,
    // then the approval queue clears.
    beginConversationPlace(); // P-A mode-changing
    beginConversationPlace(); // P-B approval pending (nested)
    endConversationPlace(); // P-A mode-changed (still one transition open)
    expect(owner.restores).toHaveLength(0); // not restored yet
    endConversationPlace(); // P-B queue cleared → the single restore

    expect(owner.captures).toBe(1); // captured once (outermost begin only)
    expect(owner.restores).toEqual([anchor]); // restored exactly once
    expect(__conversationRestoreCount()).toBe(1);
  });

  it("a lone transition still captures once and restores once", () => {
    const owner = stubOwner(null);
    registerConversationPlaceOwner(owner);
    beginConversationPlace();
    endConversationPlace();
    expect(owner.captures).toBe(1);
    expect(owner.restores).toHaveLength(1);
  });

  // Task 9.5 clearance-competition guard: whichever transition opens first, the
  // shell (mode/approval) never competes with the single owner — the stream is
  // captured once and restored exactly once (no double-touch of the viewport).
  it("approval-first then mode-change (reverse overlap) still restores the stream exactly once", () => {
    const anchor: ConversationAnchor = {
      activeThreadId: "t1",
      anchorMessageId: "m2",
      offsetWithinItem: 12,
      atBottom: false,
      anchorItemHeight: 96,
    };
    const owner = stubOwner(anchor);
    registerConversationPlaceOwner(owner);

    beginConversationPlace(); // P-B approval pending (opens first this time)
    beginConversationPlace(); // P-A mode-changing (nested)
    endConversationPlace(); // approval clears (one still open)
    expect(owner.restores).toHaveLength(0);
    endConversationPlace(); // mode settles → the single restore

    expect(owner.captures).toBe(1);
    expect(owner.restores).toEqual([anchor]);
    expect(__conversationRestoreCount()).toBe(1);
  });
});
