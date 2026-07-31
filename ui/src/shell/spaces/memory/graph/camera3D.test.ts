/**
 * camera3D.test.ts — F6.2.4 camera state machine unit tests.
 *
 * Validates (task 6.2.4):
 *   - Zoom clamping at ZOOM_MIN (0.25) and ZOOM_MAX (4.0) boundaries.
 *   - History: push, back, forward, bounded to HISTORY_BOUND (20).
 *   - fitVisible: returned camera contains all positions with margin.
 *   - goBack/goForward: no-ops at boundaries.
 *   - pinchZoom: clamps correctly.
 *   - isOffscreen: correct result for on/off screen positions.
 *   - listSyncState: 'scroll' when selected visible, 'reframe' when off-screen.
 *   - twoFingerPan: pan is clamped to bounds.
 *   - depthComfort: monotone, within expected range.
 *   - offscreenMarker: angle and distance are geometrically correct.
 *   - resetCamera: restores default eye/target/zoom.
 *   - handleKeyboard: 'f', 'r', ArrowLeft, ArrowRight, unknown keys.
 *
 * No DOM, no WebGL, no SolidJS rendering — pure logic tests only.
 *
 * Requirements: MGR-001, MGR-004, MGR-012, MGR-015, MGR-026; MGD-003, MGD-026.
 * Spec task: 6.2.4
 *
 * **Validates: Requirements MGR-015, MGR-026**
 */

import { describe, it, expect } from 'vitest';
import {
  ZOOM_MIN,
  ZOOM_MAX,
  HISTORY_BOUND,
  MARGIN_FRACTION,
  DEFAULT_CAMERA_3D,
  fitVisible,
  fitSelection,
  fitNeighborhood,
  resetCamera,
  pushHistory,
  goBack,
  goForward,
  handleKeyboard,
  pinchZoom,
  twoFingerPan,
  depthComfort,
  isOffscreen,
  offscreenMarker,
  listSyncState,
  type Camera3DState,
  type Camera3DSnapshot,
} from './camera3D';

// ─── Fixtures ─────────────────────────────────────────────────────────────────

/** A clean default camera with no history. */
const CAM: Camera3DState = DEFAULT_CAMERA_3D;

/** A camera at zoom=1, positioned at origin looking at [0,0,1] */
const CAM_ORIGIN: Camera3DState = {
  eye: [0, 0, 0],
  target: [0, 0, 0],
  zoom: 1.0,
  history: [],
  historyIndex: -1,
};

function makeSnapshot(overrides: Partial<Camera3DSnapshot> = {}): Camera3DSnapshot {
  return {
    eye: [0, 0, 3],
    target: [0, 0, 0],
    zoom: 1.0,
    ...overrides,
  };
}

function makePositionsMap(
  entries: Array<[string, [number, number, number]]>,
): Map<string, [number, number, number]> {
  return new Map(entries);
}

// ─── Constants ────────────────────────────────────────────────────────────────

describe('camera3D — constants', () => {
  it('ZOOM_MIN is 0.25', () => {
    expect(ZOOM_MIN).toBe(0.25);
  });

  it('ZOOM_MAX is 4.0', () => {
    expect(ZOOM_MAX).toBe(4.0);
  });

  it('HISTORY_BOUND is 20', () => {
    expect(HISTORY_BOUND).toBe(20);
  });

  it('MARGIN_FRACTION is 0.25', () => {
    expect(MARGIN_FRACTION).toBe(0.25);
  });

  it('DEFAULT_CAMERA_3D has zoom 1.0', () => {
    expect(DEFAULT_CAMERA_3D.zoom).toBe(1.0);
  });

  it('DEFAULT_CAMERA_3D has empty history', () => {
    expect(DEFAULT_CAMERA_3D.history).toEqual([]);
    expect(DEFAULT_CAMERA_3D.historyIndex).toBe(-1);
  });
});

// ─── pinchZoom ────────────────────────────────────────────────────────────────

describe('camera3D — pinchZoom', () => {
  it('scales zoom by factor', () => {
    const s = { ...CAM, zoom: 1.0 };
    const result = pinchZoom(s, 2.0);
    expect(result.zoom).toBe(2.0);
  });

  it('clamps zoom to ZOOM_MAX (4.0) when scaled beyond', () => {
    const s = { ...CAM, zoom: 3.5 };
    const result = pinchZoom(s, 2.0); // 3.5 * 2 = 7 → clamped to 4
    expect(result.zoom).toBe(ZOOM_MAX);
  });

  it('clamps zoom to ZOOM_MIN (0.25) when scaled below', () => {
    const s = { ...CAM, zoom: 0.3 };
    const result = pinchZoom(s, 0.5); // 0.3 * 0.5 = 0.15 → clamped to 0.25
    expect(result.zoom).toBe(ZOOM_MIN);
  });

  it('zoom exactly at ZOOM_MAX stays at ZOOM_MAX after upscale', () => {
    const s = { ...CAM, zoom: ZOOM_MAX };
    const result = pinchZoom(s, 1.5);
    expect(result.zoom).toBe(ZOOM_MAX);
  });

  it('zoom exactly at ZOOM_MIN stays at ZOOM_MIN after downscale', () => {
    const s = { ...CAM, zoom: ZOOM_MIN };
    const result = pinchZoom(s, 0.5);
    expect(result.zoom).toBe(ZOOM_MIN);
  });

  it('scale of 1.0 leaves zoom unchanged', () => {
    const s = { ...CAM, zoom: 1.5 };
    const result = pinchZoom(s, 1.0);
    expect(result.zoom).toBe(1.5);
  });

  it('does not mutate other fields', () => {
    const s = { ...CAM, zoom: 1.0 };
    const result = pinchZoom(s, 2.0);
    expect(result.eye).toEqual(s.eye);
    expect(result.target).toEqual(s.target);
    expect(result.history).toBe(s.history);
  });
});

// ─── twoFingerPan ─────────────────────────────────────────────────────────────

describe('camera3D — twoFingerPan', () => {
  it('pans target by (dx, dy)', () => {
    const s: Camera3DState = { ...CAM_ORIGIN, target: [0, 0, 0], eye: [0, 0, 3] };
    const result = twoFingerPan(s, 0.5, 0.3);
    expect(result.target[0]).toBeCloseTo(0.5);
    expect(result.target[1]).toBeCloseTo(0.3);
  });

  it('eye moves by same offset preserving look direction', () => {
    const s: Camera3DState = { ...CAM_ORIGIN, target: [0, 0, 0], eye: [0, 0, 3] };
    const result = twoFingerPan(s, 0.5, 0.0);
    // eye offset relative to target should be preserved
    const offsetX = result.eye[0] - result.target[0];
    const offsetY = result.eye[1] - result.target[1];
    const origOffsetX = s.eye[0] - s.target[0];
    const origOffsetY = s.eye[1] - s.target[1];
    expect(offsetX).toBeCloseTo(origOffsetX);
    expect(offsetY).toBeCloseTo(origOffsetY);
  });

  it('clamps pan so target.x does not exceed bound (1.25)', () => {
    const s: Camera3DState = { ...CAM_ORIGIN, target: [0, 0, 0] };
    const result = twoFingerPan(s, 5.0, 0); // huge dx
    expect(result.target[0]).toBeLessThanOrEqual(1.0 + MARGIN_FRACTION);
  });

  it('clamps pan so target.x does not go below -bound (-1.25)', () => {
    const s: Camera3DState = { ...CAM_ORIGIN, target: [0, 0, 0] };
    const result = twoFingerPan(s, -5.0, 0);
    expect(result.target[0]).toBeGreaterThanOrEqual(-(1.0 + MARGIN_FRACTION));
  });

  it('clamps pan so target.y does not exceed bound', () => {
    const s: Camera3DState = { ...CAM_ORIGIN, target: [0, 0, 0] };
    const result = twoFingerPan(s, 0, 5.0);
    expect(result.target[1]).toBeLessThanOrEqual(1.0 + MARGIN_FRACTION);
  });

  it('zero pan returns same eye and target', () => {
    const s: Camera3DState = { ...CAM_ORIGIN, target: [0.2, 0.3, 0], eye: [0.2, 0.3, 3] };
    const result = twoFingerPan(s, 0, 0);
    expect(result.target[0]).toBeCloseTo(0.2);
    expect(result.target[1]).toBeCloseTo(0.3);
    expect(result.eye[0]).toBeCloseTo(0.2);
    expect(result.eye[1]).toBeCloseTo(0.3);
  });
});

// ─── pushHistory / goBack / goForward ────────────────────────────────────────

describe('camera3D — pushHistory', () => {
  it('starts with empty history; pushHistory adds an entry', () => {
    const s = CAM; // historyIndex=-1, history=[]
    const next = makeSnapshot({ zoom: 2.0 });
    const result = pushHistory(s, next);
    expect(result.history).toHaveLength(1);
    expect(result.historyIndex).toBe(0);
    expect(result.zoom).toBe(2.0);
  });

  it('records the previous state as the history entry', () => {
    const s: Camera3DState = { ...CAM, zoom: 1.5, historyIndex: -1, history: [] };
    const next = makeSnapshot({ zoom: 2.0 });
    const result = pushHistory(s, next);
    expect(result.history[0]!.zoom).toBe(1.5);
  });

  it('new state eye/target/zoom is applied to the returned state', () => {
    const next: Camera3DSnapshot = { eye: [1, 2, 3], target: [0, 0, 0], zoom: 3.0 };
    const result = pushHistory(CAM, next);
    expect(result.eye).toEqual([1, 2, 3]);
    expect(result.zoom).toBe(3.0);
  });

  it('pushHistory discards future entries after historyIndex', () => {
    // Build a state with history=[snap0, snap1], index=0 (we're "at" snap0 after going back)
    const snap0: Camera3DSnapshot = { eye: [0,0,1], target: [0,0,0], zoom: 1.0 };
    const snap1: Camera3DSnapshot = { eye: [0,0,2], target: [0,0,0], zoom: 1.5 };
    const s: Camera3DState = {
      eye: snap0.eye, target: snap0.target, zoom: snap0.zoom,
      history: [snap0, snap1],
      historyIndex: 0,
    };
    const newSnap: Camera3DSnapshot = { eye: [5,5,5], target: [0,0,0], zoom: 2.5 };
    const result = pushHistory(s, newSnap);
    // snap1 (the future) should be gone; only snap0 + previous live state remain
    // live state snap0 is the "current" → pushed to history
    // future snap1 is discarded
    expect(result.history.some(h => h.zoom === 1.5)).toBe(false);
  });

  it('history is bounded to HISTORY_BOUND (20) entries', () => {
    let s = CAM;
    // Push 25 entries
    for (let i = 0; i < 25; i++) {
      s = pushHistory(s, makeSnapshot({ zoom: 1 + i * 0.1 }));
    }
    expect(s.history.length).toBeLessThanOrEqual(HISTORY_BOUND);
  });

  it('oldest entry is dropped when exceeding bound', () => {
    // Establish a known first zoom
    let s: Camera3DState = { ...CAM, zoom: 99.0, historyIndex: -1, history: [] };
    // Push HISTORY_BOUND + 1 transitions so the first one (zoom=99) gets evicted
    for (let i = 0; i < HISTORY_BOUND + 1; i++) {
      s = pushHistory(s, makeSnapshot({ zoom: i + 1.0 }));
    }
    // history should not contain zoom=99
    expect(s.history.some(h => h.zoom === 99.0)).toBe(false);
    expect(s.history.length).toBeLessThanOrEqual(HISTORY_BOUND);
  });
});

describe('camera3D — goBack', () => {
  it('no-op when history is empty (historyIndex=-1)', () => {
    const result = goBack(CAM);
    expect(result).toBe(CAM);
  });

  it('no-op when at the first history entry (historyIndex=0)', () => {
    const s: Camera3DState = {
      ...CAM,
      history: [makeSnapshot({ zoom: 1.0 })],
      historyIndex: 0,
    };
    const result = goBack(s);
    expect(result).toBe(s);
  });

  it('moves to previous snapshot when index > 0', () => {
    const snap0: Camera3DSnapshot = { eye: [0,0,1], target: [0,0,0], zoom: 1.0 };
    const snap1: Camera3DSnapshot = { eye: [0,0,2], target: [0,0,0], zoom: 2.0 };
    const s: Camera3DState = {
      eye: snap1.eye, target: snap1.target, zoom: snap1.zoom,
      history: [snap0, snap1],
      historyIndex: 1,
    };
    const result = goBack(s);
    expect(result.zoom).toBe(1.0);
    expect(result.historyIndex).toBe(0);
  });

  it('preserves history array reference', () => {
    const snap0: Camera3DSnapshot = { eye: [0,0,1], target: [0,0,0], zoom: 1.0 };
    const snap1: Camera3DSnapshot = { eye: [0,0,2], target: [0,0,0], zoom: 2.0 };
    const s: Camera3DState = {
      eye: snap1.eye, target: snap1.target, zoom: snap1.zoom,
      history: [snap0, snap1],
      historyIndex: 1,
    };
    const result = goBack(s);
    expect(result.history).toBe(s.history);
  });
});

describe('camera3D — goForward', () => {
  it('no-op when history is empty (historyIndex=-1)', () => {
    const result = goForward(CAM);
    expect(result).toBe(CAM);
  });

  it('no-op when already at the last entry', () => {
    const snap0: Camera3DSnapshot = { eye: [0,0,1], target: [0,0,0], zoom: 1.0 };
    const s: Camera3DState = {
      eye: snap0.eye, target: snap0.target, zoom: snap0.zoom,
      history: [snap0],
      historyIndex: 0,
    };
    const result = goForward(s);
    expect(result).toBe(s);
  });

  it('moves to next snapshot when index < last', () => {
    const snap0: Camera3DSnapshot = { eye: [0,0,1], target: [0,0,0], zoom: 1.0 };
    const snap1: Camera3DSnapshot = { eye: [0,0,2], target: [0,0,0], zoom: 2.0 };
    const s: Camera3DState = {
      eye: snap0.eye, target: snap0.target, zoom: snap0.zoom,
      history: [snap0, snap1],
      historyIndex: 0,
    };
    const result = goForward(s);
    expect(result.zoom).toBe(2.0);
    expect(result.historyIndex).toBe(1);
  });

  it('back then forward returns to the same zoom', () => {
    const snap0: Camera3DSnapshot = { eye: [0,0,1], target: [0,0,0], zoom: 1.0 };
    const snap1: Camera3DSnapshot = { eye: [0,0,2], target: [0,0,0], zoom: 2.0 };
    const s: Camera3DState = {
      eye: snap1.eye, target: snap1.target, zoom: snap1.zoom,
      history: [snap0, snap1],
      historyIndex: 1,
    };
    const afterBack = goBack(s);
    const afterForward = goForward(afterBack);
    expect(afterForward.zoom).toBe(2.0);
  });
});

// ─── fitVisible ───────────────────────────────────────────────────────────────

describe('camera3D — fitVisible', () => {
  it('returns state unchanged for empty positions', () => {
    const result = fitVisible(CAM, []);
    expect(result).toBe(CAM);
  });

  it('positions the camera target at the centroid of positions', () => {
    const positions: Array<[number, number, number]> = [
      [-1, -1, 0],
      [1, 1, 0],
    ];
    const result = fitVisible(CAM, positions);
    expect(result.target[0]).toBeCloseTo(0);
    expect(result.target[1]).toBeCloseTo(0);
  });

  it('camera target is within bounding box when single position given', () => {
    const positions: Array<[number, number, number]> = [[0.5, 0.5, 0.0]];
    const result = fitVisible(CAM, positions);
    expect(result.target[0]).toBeCloseTo(0.5);
    expect(result.target[1]).toBeCloseTo(0.5);
  });

  it('zoom is within [ZOOM_MIN, ZOOM_MAX]', () => {
    const positions: Array<[number, number, number]> = [
      [-10, -10, 0],
      [10, 10, 0],
    ];
    const result = fitVisible(CAM, positions);
    expect(result.zoom).toBeGreaterThanOrEqual(ZOOM_MIN);
    expect(result.zoom).toBeLessThanOrEqual(ZOOM_MAX);
  });

  it('large scene is still within zoom bounds', () => {
    const positions: Array<[number, number, number]> = [
      [-100, -100, -50],
      [100, 100, 50],
    ];
    const result = fitVisible(CAM, positions);
    expect(result.zoom).toBeGreaterThanOrEqual(ZOOM_MIN);
    expect(result.zoom).toBeLessThanOrEqual(ZOOM_MAX);
  });

  it('eye is placed in front of the target (eye.z > target.z)', () => {
    const positions: Array<[number, number, number]> = [
      [-1, -1, 0],
      [1, 1, 0],
    ];
    const result = fitVisible(CAM, positions);
    expect(result.eye[2]).toBeGreaterThan(result.target[2]);
  });

  it('target and eye share same x, y (looking straight on)', () => {
    const positions: Array<[number, number, number]> = [
      [0.2, 0.3, 0],
      [0.8, 0.9, 0],
    ];
    const result = fitVisible(CAM, positions);
    expect(result.eye[0]).toBeCloseTo(result.target[0]);
    expect(result.eye[1]).toBeCloseTo(result.target[1]);
  });
});

describe('camera3D — fitSelection', () => {
  it('returns state unchanged when selectedIds is empty', () => {
    const positions = makePositionsMap([['a', [0,0,0]], ['b', [1,1,0]]]);
    const result = fitSelection(CAM, positions, []);
    expect(result).toBe(CAM);
  });

  it('returns state unchanged when selected id not in positions', () => {
    const positions = makePositionsMap([['a', [0,0,0]]]);
    const result = fitSelection(CAM, positions, ['unknown']);
    expect(result).toBe(CAM);
  });

  it('fits to selected nodes only', () => {
    const positions = makePositionsMap([
      ['a', [0, 0, 0]],
      ['b', [1, 1, 0]],
      ['far', [100, 100, 0]],
    ]);
    const result = fitSelection(CAM, positions, ['a', 'b']);
    // Centroid of a and b is [0.5, 0.5, 0]
    expect(result.target[0]).toBeCloseTo(0.5);
    expect(result.target[1]).toBeCloseTo(0.5);
  });
});

describe('camera3D — fitNeighborhood', () => {
  it('returns state unchanged when neighborIds is empty', () => {
    const positions = makePositionsMap([['a', [0,0,0]]]);
    const result = fitNeighborhood(CAM, positions, []);
    expect(result).toBe(CAM);
  });

  it('fits to neighborhood nodes', () => {
    const positions = makePositionsMap([
      ['n1', [-1, 0, 0]],
      ['n2', [1, 0, 0]],
    ]);
    const result = fitNeighborhood(CAM, positions, ['n1', 'n2']);
    expect(result.target[0]).toBeCloseTo(0);
  });
});

// ─── resetCamera ──────────────────────────────────────────────────────────────

describe('camera3D — resetCamera', () => {
  it('restores default eye', () => {
    const s: Camera3DState = { ...CAM, eye: [99, 99, 99] };
    const result = resetCamera(s);
    expect(result.eye).toEqual(DEFAULT_CAMERA_3D.eye);
  });

  it('restores default target', () => {
    const s: Camera3DState = { ...CAM, target: [5, 5, 5] };
    const result = resetCamera(s);
    expect(result.target).toEqual(DEFAULT_CAMERA_3D.target);
  });

  it('restores default zoom', () => {
    const s: Camera3DState = { ...CAM, zoom: 3.0 };
    const result = resetCamera(s);
    expect(result.zoom).toBe(DEFAULT_CAMERA_3D.zoom);
  });

  it('preserves history', () => {
    const snap = makeSnapshot();
    const s: Camera3DState = { ...CAM, history: [snap], historyIndex: 0 };
    const result = resetCamera(s);
    expect(result.history).toEqual([snap]);
    expect(result.historyIndex).toBe(0);
  });
});

// ─── handleKeyboard ───────────────────────────────────────────────────────────

describe('camera3D — handleKeyboard', () => {
  const positions: Array<[number, number, number]> = [
    [-1, -1, 0],
    [1, 1, 0],
  ];

  it("'f' key calls fitVisible (target at centroid)", () => {
    const result = handleKeyboard(CAM, 'f', positions, []);
    expect(result.target[0]).toBeCloseTo(0);
    expect(result.target[1]).toBeCloseTo(0);
  });

  it("'r' key calls resetCamera (zoom is 1.0)", () => {
    const s = { ...CAM, zoom: 3.5 };
    const result = handleKeyboard(s, 'r', positions, []);
    expect(result.zoom).toBe(1.0);
    expect(result.eye).toEqual(DEFAULT_CAMERA_3D.eye);
  });

  it("'ArrowLeft' is a no-op when no history", () => {
    const result = handleKeyboard(CAM, 'ArrowLeft', positions, []);
    expect(result).toBe(CAM);
  });

  it("'ArrowRight' is a no-op when no history", () => {
    const result = handleKeyboard(CAM, 'ArrowRight', positions, []);
    expect(result).toBe(CAM);
  });

  it("'ArrowLeft' navigates back when history exists", () => {
    const snap0: Camera3DSnapshot = { eye: [0,0,1], target: [0,0,0], zoom: 1.1 };
    const snap1: Camera3DSnapshot = { eye: [0,0,2], target: [0,0,0], zoom: 2.2 };
    const s: Camera3DState = {
      eye: snap1.eye, target: snap1.target, zoom: snap1.zoom,
      history: [snap0, snap1], historyIndex: 1,
    };
    const result = handleKeyboard(s, 'ArrowLeft', positions, []);
    expect(result.zoom).toBe(1.1);
  });

  it("'ArrowRight' navigates forward when at middle of history", () => {
    const snap0: Camera3DSnapshot = { eye: [0,0,1], target: [0,0,0], zoom: 1.1 };
    const snap1: Camera3DSnapshot = { eye: [0,0,2], target: [0,0,0], zoom: 2.2 };
    const s: Camera3DState = {
      eye: snap0.eye, target: snap0.target, zoom: snap0.zoom,
      history: [snap0, snap1], historyIndex: 0,
    };
    const result = handleKeyboard(s, 'ArrowRight', positions, []);
    expect(result.zoom).toBe(2.2);
  });

  it('unknown key returns state unchanged', () => {
    const result = handleKeyboard(CAM, 'x', positions, []);
    expect(result).toBe(CAM);
  });

  it('unknown key Escape returns state unchanged', () => {
    const result = handleKeyboard(CAM, 'Escape', positions, []);
    expect(result).toBe(CAM);
  });
});

// ─── depthComfort ─────────────────────────────────────────────────────────────

describe('camera3D — depthComfort', () => {
  it('z=0 returns 1.0 (minimum comfortable depth)', () => {
    expect(depthComfort(0)).toBe(1.0);
  });

  it('z=1 returns 6.0 (maximum depth)', () => {
    expect(depthComfort(1)).toBe(6.0);
  });

  it('z=0.5 returns 3.5 (midpoint)', () => {
    expect(depthComfort(0.5)).toBeCloseTo(3.5);
  });

  it('is monotonically increasing', () => {
    const d0 = depthComfort(0.0);
    const d1 = depthComfort(0.25);
    const d2 = depthComfort(0.5);
    const d3 = depthComfort(0.75);
    const d4 = depthComfort(1.0);
    expect(d1).toBeGreaterThan(d0);
    expect(d2).toBeGreaterThan(d1);
    expect(d3).toBeGreaterThan(d2);
    expect(d4).toBeGreaterThan(d3);
  });

  it('result is always >= 0.5 (near-plane floor)', () => {
    for (const z of [0, 0.1, 0.5, 0.9, 1.0]) {
      expect(depthComfort(z)).toBeGreaterThanOrEqual(0.5);
    }
  });

  it('clamps input < 0 to z=0 behaviour', () => {
    expect(depthComfort(-0.5)).toBe(depthComfort(0));
  });

  it('clamps input > 1 to z=1 behaviour', () => {
    expect(depthComfort(2.0)).toBe(depthComfort(1));
  });
});

// ─── isOffscreen ──────────────────────────────────────────────────────────────

describe('camera3D — isOffscreen', () => {
  /** Camera at zoom=1, target at origin. viewport half-extents = 1.0 / 1.0 = 1.0 */
  const cam: Camera3DState = { ...CAM_ORIGIN, zoom: 1.0 };

  it('position at the camera target is on-screen', () => {
    expect(isOffscreen(cam, [0, 0, 0])).toBe(false);
  });

  it('position within viewport half-extent is on-screen', () => {
    expect(isOffscreen(cam, [0.5, 0.5, 0])).toBe(false);
  });

  it('position exactly at the viewport edge is on-screen', () => {
    // half-extent = 1.0 / zoom(1.0) = 1.0; |dx| = 1.0 exactly → not off-screen
    expect(isOffscreen(cam, [1.0, 0, 0])).toBe(false);
  });

  it('position just beyond viewport edge in x is off-screen', () => {
    expect(isOffscreen(cam, [1.01, 0, 0])).toBe(true);
  });

  it('position far off to the right is off-screen', () => {
    expect(isOffscreen(cam, [5, 0, 0])).toBe(true);
  });

  it('position far below is off-screen', () => {
    expect(isOffscreen(cam, [0, -5, 0])).toBe(true);
  });

  it('higher zoom makes viewport smaller (more things off-screen)', () => {
    const zoomedIn: Camera3DState = { ...cam, zoom: 4.0 };
    // half-extent = 1/4 = 0.25; position at [0.3, 0, 0] is off-screen at zoom=4
    expect(isOffscreen(zoomedIn, [0.3, 0, 0])).toBe(true);
    // but it's on-screen at zoom=1
    expect(isOffscreen(cam, [0.3, 0, 0])).toBe(false);
  });

  it('lower zoom makes viewport larger (more things on-screen)', () => {
    const zoomedOut: Camera3DState = { ...cam, zoom: 0.25 };
    // half-extent = 1/0.25 = 4.0; position at [3, 0, 0] is on-screen at zoom=0.25
    expect(isOffscreen(zoomedOut, [3, 0, 0])).toBe(false);
    // but off-screen at zoom=1
    expect(isOffscreen(cam, [3, 0, 0])).toBe(true);
  });

  it('z-coordinate does not affect on/off-screen result', () => {
    // Same x,y but different z — should give same result
    expect(isOffscreen(cam, [0.5, 0.5, 0])).toBe(
      isOffscreen(cam, [0.5, 0.5, 100]),
    );
  });
});

// ─── offscreenMarker ─────────────────────────────────────────────────────────

describe('camera3D — offscreenMarker', () => {
  const cam: Camera3DState = { ...CAM_ORIGIN, target: [0, 0, 0], zoom: 1.0 };

  it('position to the right has angle ≈ 0', () => {
    const { angle } = offscreenMarker(cam, [5, 0, 0]);
    expect(angle).toBeCloseTo(0);
  });

  it('position above has angle ≈ π/2', () => {
    const { angle } = offscreenMarker(cam, [0, 5, 0]);
    expect(angle).toBeCloseTo(Math.PI / 2);
  });

  it('position to the left has angle ≈ π', () => {
    const { angle } = offscreenMarker(cam, [-5, 0, 0]);
    expect(Math.abs(angle)).toBeCloseTo(Math.PI);
  });

  it('position below has angle ≈ -π/2', () => {
    const { angle } = offscreenMarker(cam, [0, -5, 0]);
    expect(angle).toBeCloseTo(-Math.PI / 2);
  });

  it('distance is Euclidean distance from target to position in x,y', () => {
    const { distance } = offscreenMarker(cam, [3, 4, 99]);
    expect(distance).toBeCloseTo(5); // 3-4-5 triangle
  });

  it('position at target has distance 0', () => {
    const { distance } = offscreenMarker(cam, [0, 0, 0]);
    expect(distance).toBeCloseTo(0);
  });
});

// ─── listSyncState ────────────────────────────────────────────────────────────

describe('camera3D — listSyncState', () => {
  /** Camera at zoom=1, target at origin; half-extents = 1.0 */
  const cam: Camera3DState = { ...CAM_ORIGIN, zoom: 1.0 };

  it("returns 'none' when selectedId is null", () => {
    const positions = makePositionsMap([['a', [0, 0, 0]]]);
    const result = listSyncState(cam, null, positions);
    expect(result.action).toBe('none');
    expect(result.itemId).toBeNull();
  });

  it("returns 'none' when selectedId is empty string", () => {
    const positions = makePositionsMap([['a', [0, 0, 0]]]);
    const result = listSyncState(cam, '', positions);
    expect(result.action).toBe('none');
    expect(result.itemId).toBeNull();
  });

  it("returns 'none' when selectedId is not in positions", () => {
    const positions = makePositionsMap([['a', [0, 0, 0]]]);
    const result = listSyncState(cam, 'missing', positions);
    expect(result.action).toBe('none');
    expect(result.itemId).toBeNull();
  });

  it("returns 'scroll' when selected node is on-screen", () => {
    // Position [0.5, 0.5, 0] is within half-extent 1.0 of target [0,0,0]
    const positions = makePositionsMap([['node-a', [0.5, 0.5, 0]]]);
    const result = listSyncState(cam, 'node-a', positions);
    expect(result.action).toBe('scroll');
    expect(result.itemId).toBe('node-a');
  });

  it("returns 'scroll' with itemId equal to selectedId", () => {
    const positions = makePositionsMap([['my-node', [0.1, 0.1, 0]]]);
    const result = listSyncState(cam, 'my-node', positions);
    expect(result.itemId).toBe('my-node');
  });

  it("returns 'reframe' when selected node is off-screen", () => {
    // Position [5, 5, 0] is beyond half-extent 1.0
    const positions = makePositionsMap([['far-node', [5, 5, 0]]]);
    const result = listSyncState(cam, 'far-node', positions);
    expect(result.action).toBe('reframe');
    expect(result.itemId).toBe('far-node');
  });

  it("returns 'reframe' with itemId equal to selectedId when off-screen", () => {
    const positions = makePositionsMap([['off', [10, 10, 0]]]);
    const result = listSyncState(cam, 'off', positions);
    expect(result.itemId).toBe('off');
  });

  it("zoomed-in camera: previously on-screen node is now 'reframe'", () => {
    // At zoom=4, half-extent = 0.25; position at [0.5, 0.5, 0] is off-screen
    const zoomedCam: Camera3DState = { ...cam, zoom: 4.0 };
    const positions = makePositionsMap([['n', [0.5, 0.5, 0]]]);
    const result = listSyncState(zoomedCam, 'n', positions);
    expect(result.action).toBe('reframe');
  });

  it("zoomed-out camera: previously off-screen node is now 'scroll'", () => {
    // At zoom=0.25, half-extent = 4.0; position at [3, 0, 0] is on-screen
    const zoomedOutCam: Camera3DState = { ...cam, zoom: 0.25 };
    const positions = makePositionsMap([['n', [3, 0, 0]]]);
    const result = listSyncState(zoomedOutCam, 'n', positions);
    expect(result.action).toBe('scroll');
  });
});
