import { describe, it, expect } from "vitest";
import {
  createCameraHistory,
  pushEntry,
  goBack,
  goForward,
  canGoBack,
  canGoForward,
  fitItems,
  type CameraHistory,
  type CameraHistoryEntry,
} from "./cameraHistory";
import { type CameraState, type ViewportSize, ZOOM_MIN, ZOOM_MAX } from "./camera";

// ─── Fixtures ─────────────────────────────────────────────────────────────────

const CAM_A: CameraState = { x: 100, y: 100, zoom: 1 };
const CAM_B: CameraState = { x: 200, y: 200, zoom: 1.5 };
const CAM_C: CameraState = { x: 300, y: 300, zoom: 2 };

const ENTRY_A: CameraHistoryEntry = { camera: CAM_A, queryId: "q1", revision: 1 };
const ENTRY_B: CameraHistoryEntry = { camera: CAM_B, queryId: "q2", revision: 2 };
const ENTRY_C: CameraHistoryEntry = { camera: CAM_C, queryId: null, revision: null };

const VP: ViewportSize = { width: 800, height: 600 };

// ─── createCameraHistory ──────────────────────────────────────────────────────

describe("createCameraHistory", () => {
  it("creates empty entries and currentIndex=-1", () => {
    const h = createCameraHistory();
    expect(h.entries).toHaveLength(0);
    expect(h.currentIndex).toBe(-1);
  });
});

// ─── pushEntry ────────────────────────────────────────────────────────────────

describe("pushEntry", () => {
  it("adds an entry and advances currentIndex", () => {
    const h0 = createCameraHistory();
    const h1 = pushEntry(h0, ENTRY_A);
    expect(h1.entries).toHaveLength(1);
    expect(h1.currentIndex).toBe(0);
    expect(h1.entries[0]).toEqual(ENTRY_A);
  });

  it("appends further entries and advances index", () => {
    const h = pushEntry(pushEntry(createCameraHistory(), ENTRY_A), ENTRY_B);
    expect(h.entries).toHaveLength(2);
    expect(h.currentIndex).toBe(1);
    expect(h.entries[1]).toEqual(ENTRY_B);
  });

  it("does not mutate the original history", () => {
    const h0 = createCameraHistory();
    pushEntry(h0, ENTRY_A);
    expect(h0.entries).toHaveLength(0);
    expect(h0.currentIndex).toBe(-1);
  });

  it("when at non-latest, truncates forward entries before appending", () => {
    // Build history A → B → C, then go back to A
    let h = pushEntry(pushEntry(pushEntry(createCameraHistory(), ENTRY_A), ENTRY_B), ENTRY_C);
    // h.currentIndex = 2
    const { history: hBack1 } = goBack(h);      // index = 1 (at B)
    const { history: hBack2 } = goBack(hBack1); // index = 0 (at A)

    // Push D from index 0 — should discard B and C
    const ENTRY_D: CameraHistoryEntry = { camera: { x: 400, y: 400, zoom: 1 }, queryId: "d", revision: 4 };
    const hNew = pushEntry(hBack2, ENTRY_D);

    expect(hNew.entries).toHaveLength(2);
    expect(hNew.entries[0]).toEqual(ENTRY_A);
    expect(hNew.entries[1]).toEqual(ENTRY_D);
    expect(hNew.currentIndex).toBe(1);
  });
});

// ─── goBack ───────────────────────────────────────────────────────────────────

describe("goBack", () => {
  it("moves currentIndex back and returns the entry", () => {
    const h = pushEntry(pushEntry(createCameraHistory(), ENTRY_A), ENTRY_B);
    const { history, entry } = goBack(h);
    expect(history.currentIndex).toBe(0);
    expect(entry).toEqual(ENTRY_A);
  });

  it("returns null when already at the start (index 0)", () => {
    const h = pushEntry(createCameraHistory(), ENTRY_A);
    const { history, entry } = goBack(h);
    expect(entry).toBeNull();
    expect(history.currentIndex).toBe(0); // unchanged
  });

  it("returns null for empty history", () => {
    const h = createCameraHistory();
    const { history, entry } = goBack(h);
    expect(entry).toBeNull();
    expect(history.currentIndex).toBe(-1);
  });
});

// ─── goForward ────────────────────────────────────────────────────────────────

describe("goForward", () => {
  it("moves currentIndex forward and returns the entry", () => {
    const h0 = pushEntry(pushEntry(createCameraHistory(), ENTRY_A), ENTRY_B);
    const { history: hBack } = goBack(h0);
    const { history, entry } = goForward(hBack);
    expect(history.currentIndex).toBe(1);
    expect(entry).toEqual(ENTRY_B);
  });

  it("returns null when already at the latest entry", () => {
    const h = pushEntry(pushEntry(createCameraHistory(), ENTRY_A), ENTRY_B);
    const { history, entry } = goForward(h);
    expect(entry).toBeNull();
    expect(history.currentIndex).toBe(1); // unchanged
  });

  it("returns null for empty history", () => {
    const h = createCameraHistory();
    const { history, entry } = goForward(h);
    expect(entry).toBeNull();
    expect(history.currentIndex).toBe(-1);
  });
});

// ─── canGoBack / canGoForward ─────────────────────────────────────────────────

describe("canGoBack", () => {
  it("false for empty history", () => {
    expect(canGoBack(createCameraHistory())).toBe(false);
  });

  it("false when at the first entry", () => {
    const h = pushEntry(createCameraHistory(), ENTRY_A);
    expect(canGoBack(h)).toBe(false);
  });

  it("true when there is an entry before current", () => {
    const h = pushEntry(pushEntry(createCameraHistory(), ENTRY_A), ENTRY_B);
    expect(canGoBack(h)).toBe(true);
  });
});

describe("canGoForward", () => {
  it("false for empty history", () => {
    expect(canGoForward(createCameraHistory())).toBe(false);
  });

  it("false when at the latest entry", () => {
    const h = pushEntry(pushEntry(createCameraHistory(), ENTRY_A), ENTRY_B);
    expect(canGoForward(h)).toBe(false);
  });

  it("true when there is an entry after current", () => {
    const h0 = pushEntry(pushEntry(createCameraHistory(), ENTRY_A), ENTRY_B);
    const { history } = goBack(h0);
    expect(canGoForward(history)).toBe(true);
  });
});

// ─── fitItems ─────────────────────────────────────────────────────────────────

describe("fitItems", () => {
  it("returns camera unchanged when itemPositions is empty", () => {
    const cam: CameraState = { x: 250, y: 300, zoom: 1.8 };
    const result = fitItems(cam, VP, new Map());
    expect(result).toEqual(cam);
  });

  it("centres the camera on a single item", () => {
    const cam: CameraState = { x: 0, y: 0, zoom: 1 };
    const positions = new Map([["a", { x: 500, y: 400 }]]);
    const result = fitItems(cam, VP, positions);
    expect(result.x).toBe(500);
    expect(result.y).toBe(400);
  });

  it("centres on the bounding box of multiple items", () => {
    const cam: CameraState = { x: 0, y: 0, zoom: 1 };
    const positions = new Map([
      ["a", { x: 0, y: 0 }],
      ["b", { x: 200, y: 0 }],
      ["c", { x: 200, y: 100 }],
      ["d", { x: 0, y: 100 }],
    ]);
    const result = fitItems(cam, VP, positions);
    expect(result.x).toBe(100); // (0 + 200) / 2
    expect(result.y).toBe(50);  // (0 + 100) / 2
  });

  it("adjusts zoom so all items fit within the viewport", () => {
    const cam: CameraState = { x: 0, y: 0, zoom: 1 };
    // Items span 400×300 world units; viewport is 800×600
    // With 10% padding: padW=720, padH=540 → zoomX=720/400=1.8, zoomY=540/300=1.8
    const positions = new Map([
      ["a", { x: 0, y: 0 }],
      ["b", { x: 400, y: 300 }],
    ]);
    const result = fitItems(cam, VP, positions);
    expect(result.zoom).toBeCloseTo(1.8, 5);
  });

  it("clamps computed zoom to [ZOOM_MIN, ZOOM_MAX]", () => {
    const cam: CameraState = { x: 0, y: 0, zoom: 1 };

    // Very spread items → zoom would be tiny → clamped to ZOOM_MIN
    const widePositions = new Map([
      ["a", { x: 0, y: 0 }],
      ["b", { x: 100000, y: 100000 }],
    ]);
    const wide = fitItems(cam, VP, widePositions);
    expect(wide.zoom).toBeGreaterThanOrEqual(ZOOM_MIN);

    // Very close items → zoom would be huge → clamped to ZOOM_MAX
    const tinyPositions = new Map([
      ["a", { x: 0, y: 0 }],
      ["b", { x: 1, y: 1 }],
    ]);
    const tiny = fitItems(cam, VP, tinyPositions);
    expect(tiny.zoom).toBeLessThanOrEqual(ZOOM_MAX);
  });

  it("does not mutate the input camera", () => {
    const cam: CameraState = { x: 50, y: 60, zoom: 1.2 };
    const positions = new Map([["a", { x: 300, y: 200 }]]);
    fitItems(cam, VP, positions);
    expect(cam).toEqual({ x: 50, y: 60, zoom: 1.2 });
  });
});
