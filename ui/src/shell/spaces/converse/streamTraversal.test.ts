import { describe, it, expect } from "vitest";
import {
  computePageScrollTop,
  isEditableTarget,
  resolveTraversalIntent,
  shouldPerformFocusRestore,
  shouldQueueFocusRestore,
  shouldRevealFocusedRow,
} from "./streamTraversal";

// Task 9.6 (design §21 UIE-M-005; Req 10.11, 12.11, 19.1–19.7).
// jsdom has no real scroll layout, so the traversal CONTRACT is exercised as
// deterministic pure logic here; MessageStream.test.tsx covers the wiring.

describe("resolveTraversalIntent — key → action (Req 10.11)", () => {
  it("Home jumps to top", () => {
    expect(resolveTraversalIntent({ key: "Home", editableFocus: false })).toEqual({ kind: "top" });
  });

  it("End jumps to bottom", () => {
    expect(resolveTraversalIntent({ key: "End", editableFocus: false })).toEqual({
      kind: "bottom",
    });
  });

  it("PageUp / PageDown page by ±1", () => {
    expect(resolveTraversalIntent({ key: "PageUp", editableFocus: false })).toEqual({
      kind: "page",
      direction: -1,
    });
    expect(resolveTraversalIntent({ key: "PageDown", editableFocus: false })).toEqual({
      kind: "page",
      direction: 1,
    });
  });

  it("does not trap Tab (passes through)", () => {
    expect(resolveTraversalIntent({ key: "Tab", editableFocus: false })).toEqual({ kind: "none" });
  });

  it("does not hijack arrows / Enter belonging to child controls", () => {
    for (const key of ["ArrowUp", "ArrowDown", "Enter", " ", "a"]) {
      expect(resolveTraversalIntent({ key, editableFocus: false })).toEqual({ kind: "none" });
    }
  });

  it("defers ALL keys when focus is inside an editable control (a11y, Req 12.11)", () => {
    for (const key of ["Home", "End", "PageUp", "PageDown"]) {
      expect(resolveTraversalIntent({ key, editableFocus: true })).toEqual({ kind: "none" });
    }
  });
});

describe("computePageScrollTop — bounded page scroll (no trap)", () => {
  it("pages down by one client height", () => {
    expect(computePageScrollTop(0, 400, 2000, 1)).toBe(400);
  });

  it("pages up by one client height", () => {
    expect(computePageScrollTop(800, 400, 2000, -1)).toBe(400);
  });

  it("clamps at the bottom (max = scrollHeight - clientHeight)", () => {
    expect(computePageScrollTop(1500, 400, 2000, 1)).toBe(1600);
    expect(computePageScrollTop(1600, 400, 2000, 1)).toBe(1600);
  });

  it("clamps at the top (never negative)", () => {
    expect(computePageScrollTop(100, 400, 2000, -1)).toBe(0);
    expect(computePageScrollTop(0, 400, 2000, -1)).toBe(0);
  });

  it("handles non-finite inputs safely", () => {
    expect(computePageScrollTop(NaN, NaN, NaN, 1)).toBe(0);
  });
});

describe("shouldRevealFocusedRow — focus reveal (no focused-but-invisible trap)", () => {
  const viewportTop = 500;
  const viewportHeight = 400; // visible band [500, 900]

  it("reveals a row not currently rendered (no geometry)", () => {
    expect(shouldRevealFocusedRow(null, viewportTop, viewportHeight)).toBe(true);
  });

  it("reveals a row above the visible band", () => {
    expect(shouldRevealFocusedRow({ start: 100, end: 180 }, viewportTop, viewportHeight)).toBe(true);
  });

  it("reveals a row below the visible band", () => {
    expect(shouldRevealFocusedRow({ start: 1000, end: 1080 }, viewportTop, viewportHeight)).toBe(
      true,
    );
  });

  it("reveals a row only partially visible (bottom edge cut off)", () => {
    expect(shouldRevealFocusedRow({ start: 850, end: 950 }, viewportTop, viewportHeight)).toBe(true);
  });

  it("does NOT reveal a fully-visible row (won't fight follow/stick)", () => {
    expect(shouldRevealFocusedRow({ start: 520, end: 600 }, viewportTop, viewportHeight)).toBe(
      false,
    );
  });
});

// Task 11.6 (gap G5; design §16 Accessibility Plan; Req 16.4, 12.11).
// Focus must survive a focused row being virtualized away and its DOM node
// reused for another message — but must NEVER fight an intentional focus move.
describe("shouldQueueFocusRestore — queue only unmount-caused focus loss", () => {
  it("queues when focus dropped to nothing and the focused row unmounted", () => {
    expect(
      shouldQueueFocusRestore({
        relatedTargetPresent: false,
        lastFocusedId: "m5",
        lastFocusedRowStillRendered: false,
      }),
    ).toBe(true);
  });

  it("does NOT queue an intentional focus move (relatedTarget present)", () => {
    expect(
      shouldQueueFocusRestore({
        relatedTargetPresent: true,
        lastFocusedId: "m5",
        lastFocusedRowStillRendered: false,
      }),
    ).toBe(false);
  });

  it("does NOT queue when the focused row is still rendered (focus moved within it)", () => {
    expect(
      shouldQueueFocusRestore({
        relatedTargetPresent: false,
        lastFocusedId: "m5",
        lastFocusedRowStillRendered: true,
      }),
    ).toBe(false);
  });

  it("does NOT queue when nothing was focused", () => {
    expect(
      shouldQueueFocusRestore({
        relatedTargetPresent: false,
        lastFocusedId: null,
        lastFocusedRowStillRendered: false,
      }),
    ).toBe(false);
  });
});

describe("shouldPerformFocusRestore — replay once the row is back", () => {
  it("restores when the pending message is rendered again and focus is still lost", () => {
    expect(
      shouldPerformFocusRestore({ pendingId: "m5", isRendered: true, focusInsideViewport: false }),
    ).toBe(true);
  });

  it("does NOT restore while the pending message row is not yet mounted", () => {
    expect(
      shouldPerformFocusRestore({ pendingId: "m5", isRendered: false, focusInsideViewport: false }),
    ).toBe(false);
  });

  it("does NOT steal focus back once it already landed inside the viewport", () => {
    expect(
      shouldPerformFocusRestore({ pendingId: "m5", isRendered: true, focusInsideViewport: true }),
    ).toBe(false);
  });

  it("does nothing when there is no pending restore", () => {
    expect(
      shouldPerformFocusRestore({ pendingId: null, isRendered: true, focusInsideViewport: false }),
    ).toBe(false);
  });
});

describe("isEditableTarget", () => {
  it("detects input/textarea/select", () => {
    for (const tag of ["input", "textarea", "select"]) {
      const el = document.createElement(tag);
      expect(isEditableTarget(el)).toBe(true);
    }
  });

  it("detects contenteditable", () => {
    const el = document.createElement("div");
    el.setAttribute("contenteditable", "true");
    // jsdom does not compute isContentEditable from the attribute; force it.
    Object.defineProperty(el, "isContentEditable", { value: true });
    expect(isEditableTarget(el)).toBe(true);
  });

  it("returns false for a plain element / null", () => {
    expect(isEditableTarget(document.createElement("button"))).toBe(false);
    expect(isEditableTarget(null)).toBe(false);
  });
});
