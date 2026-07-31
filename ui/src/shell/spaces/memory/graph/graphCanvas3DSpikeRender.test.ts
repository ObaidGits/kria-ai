/**
 * graphCanvas3DSpikeRender.test.ts — F6.2.3 LOD, frustum culling, cap enforcement,
 * and bounded dirty label unit tests.
 *
 * Validates (task 6.2.3):
 *   - SCENE_CAPS values match the spec (design.md §10.2).
 *   - LOD level assignment by camera distance.
 *   - frustumCull returns correct visibility for nodes inside/outside frustum.
 *   - applyCaps enforces BALANCED_NODES limit with correct priority ordering.
 *   - updateDirtyLabels processes at most maxUpdatesPerFrame dirty entries.
 *
 * No DOM, no WebGL, no SolidJS rendering — pure logic tests only.
 *
 * Requirements: MGR-001, MGR-004, MGR-012, MGR-015, MGR-026; MGD-046.
 * Spec task: 6.2.3
 *
 * **Validates: Requirements MGR-015, MGR-026**
 */

import { describe, it, expect } from 'vitest';
import {
  SCENE_CAPS,
  LODLevel,
  LOD_NEAR_THRESHOLD,
  LOD_MID_THRESHOLD,
  getLODLevel,
  frustumCull,
  applyCaps,
  updateDirtyLabels,
  DEFAULT_MAX_UPDATES_PER_FRAME,
  type CameraState,
  type SceneItem,
  type LabelState,
} from './culling3D';
import { PACKED_NODE_STRIDE } from './graphCanvas3DSpike';

// ─── Fixtures ─────────────────────────────────────────────────────────────────

/**
 * Build a packed Float32Array for a list of world-space positions.
 * Mirrors packNodePositions layout without requiring semantic items.
 */
function makePositions(
  pts: Array<{ x: number; y: number; z: number; hasZ?: number }>,
): Float32Array {
  const buf = new Float32Array(pts.length * PACKED_NODE_STRIDE);
  for (let i = 0; i < pts.length; i++) {
    const p = pts[i]!;
    const base = i * PACKED_NODE_STRIDE;
    buf[base + 0] = i;           // nodeIndex
    buf[base + 1] = p.x;         // x
    buf[base + 2] = p.y;         // y
    buf[base + 3] = p.z;         // z
    buf[base + 4] = p.hasZ ?? 1; // hasZ
  }
  return buf;
}

/** Default perspective camera looking straight along +z, positioned at origin. */
const DEFAULT_CAMERA: CameraState = {
  fovY: Math.PI / 3, // 60 degrees
  aspect: 16 / 9,
  near: 0.1,
  far: 100.0,
  eye: [0, 0, 0],
  target: [0, 0, 1],
};

/** Make a minimal SceneItem. */
function makeItem(
  id: string,
  opts: Partial<SceneItem> = {},
): SceneItem {
  return {
    id,
    kind: 'node',
    isSelected: false,
    isFocused: false,
    isInPath: false,
    z: null,
    ...opts,
  };
}

/** Make a minimal LabelState. */
function makeLabel(
  itemId: string,
  opts: Partial<LabelState> = {},
): LabelState {
  return {
    itemId,
    text: `Label ${itemId}`,
    screenX: 0,
    screenY: 0,
    visible: true,
    dirty: false,
    ...opts,
  };
}

// ─── SCENE_CAPS tests ─────────────────────────────────────────────────────────

describe('F6.2.3 SCENE_CAPS — exact values from design.md §10.2', () => {
  it('BALANCED_NODES is 240', () => {
    expect(SCENE_CAPS.BALANCED_NODES).toBe(240);
  });

  it('BALANCED_EDGES is 360', () => {
    expect(SCENE_CAPS.BALANCED_EDGES).toBe(360);
  });

  it('BALANCED_LABELS is 80', () => {
    expect(SCENE_CAPS.BALANCED_LABELS).toBe(80);
  });

  it('HARD_NODES is 500', () => {
    expect(SCENE_CAPS.HARD_NODES).toBe(500);
  });

  it('HARD_EDGES is 750', () => {
    expect(SCENE_CAPS.HARD_EDGES).toBe(750);
  });

  it('HARD_LABELS is 160', () => {
    expect(SCENE_CAPS.HARD_LABELS).toBe(160);
  });

  it('BYTES_BALANCED is 524288 (512 KiB)', () => {
    expect(SCENE_CAPS.BYTES_BALANCED).toBe(512 * 1024);
  });

  it('BYTES_HARD is 2097152 (2 MiB)', () => {
    expect(SCENE_CAPS.BYTES_HARD).toBe(2 * 1024 * 1024);
  });

  it('BALANCED caps are strictly less than HARD caps', () => {
    expect(SCENE_CAPS.BALANCED_NODES).toBeLessThan(SCENE_CAPS.HARD_NODES);
    expect(SCENE_CAPS.BALANCED_EDGES).toBeLessThan(SCENE_CAPS.HARD_EDGES);
    expect(SCENE_CAPS.BALANCED_LABELS).toBeLessThan(SCENE_CAPS.HARD_LABELS);
    expect(SCENE_CAPS.BYTES_BALANCED).toBeLessThan(SCENE_CAPS.BYTES_HARD);
  });
});

// ─── LOD level tests ──────────────────────────────────────────────────────────

describe('F6.2.3 getLODLevel — assignment by camera distance', () => {
  it('returns NEAR for distance 0.0 (at camera eye)', () => {
    expect(getLODLevel(0.0)).toBe(LODLevel.NEAR);
  });

  it('returns NEAR for distance < 2.0 (e.g. 1.99)', () => {
    expect(getLODLevel(1.99)).toBe(LODLevel.NEAR);
  });

  it('returns NEAR for distance exactly 0.001', () => {
    expect(getLODLevel(0.001)).toBe(LODLevel.NEAR);
  });

  it('returns MID for distance exactly at LOD_NEAR_THRESHOLD (2.0)', () => {
    expect(getLODLevel(LOD_NEAR_THRESHOLD)).toBe(LODLevel.MID);
  });

  it('returns MID for distance 3.5 (between 2.0 and 5.0)', () => {
    expect(getLODLevel(3.5)).toBe(LODLevel.MID);
  });

  it('returns MID for distance exactly at LOD_MID_THRESHOLD (5.0)', () => {
    expect(getLODLevel(LOD_MID_THRESHOLD)).toBe(LODLevel.MID);
  });

  it('returns FAR for distance just above LOD_MID_THRESHOLD (5.001)', () => {
    expect(getLODLevel(5.001)).toBe(LODLevel.FAR);
  });

  it('returns FAR for distance 50.0', () => {
    expect(getLODLevel(50.0)).toBe(LODLevel.FAR);
  });

  it('LOD boundaries are exactly 2.0 and 5.0 (frozen per spec)', () => {
    expect(LOD_NEAR_THRESHOLD).toBe(2.0);
    expect(LOD_MID_THRESHOLD).toBe(5.0);
  });
});

// ─── frustumCull tests ────────────────────────────────────────────────────────

describe('F6.2.3 frustumCull — visibility for nodes inside/outside frustum', () => {
  it('returns empty array for empty positions buffer', () => {
    const result = frustumCull(new Float32Array(0), DEFAULT_CAMERA);
    expect(result).toEqual([]);
  });

  it('returns array of length equal to node count', () => {
    const positions = makePositions([
      { x: 0, y: 0, z: 5 },
      { x: 0.1, y: 0, z: 5 },
    ]);
    const result = frustumCull(positions, DEFAULT_CAMERA);
    expect(result).toHaveLength(2);
  });

  it('node directly in front of camera (on forward axis) is visible', () => {
    // Camera at origin, looking at +z. Node at (0, 0, 5) is on the forward axis.
    const positions = makePositions([{ x: 0, y: 0, z: 5 }]);
    const result = frustumCull(positions, DEFAULT_CAMERA);
    expect(result[0]).toBe(true);
  });

  it('node behind the camera (negative z) is not visible', () => {
    // Camera looking at +z. Node at (0, 0, -5) is behind.
    const positions = makePositions([{ x: 0, y: 0, z: -5 }]);
    const result = frustumCull(positions, DEFAULT_CAMERA);
    expect(result[0]).toBe(false);
  });

  it('node beyond far plane is not visible', () => {
    // DEFAULT_CAMERA.far = 100. Node at z=200 is beyond far.
    const positions = makePositions([{ x: 0, y: 0, z: 200 }]);
    const result = frustumCull(positions, DEFAULT_CAMERA);
    expect(result[0]).toBe(false);
  });

  it('node between near and far planes, on axis, is visible', () => {
    const positions = makePositions([{ x: 0, y: 0, z: 50 }]);
    const result = frustumCull(positions, DEFAULT_CAMERA);
    expect(result[0]).toBe(true);
  });

  it('node far outside lateral frustum is not visible', () => {
    // At depth z=10, with fovY=60°, aspect=16/9, the horizontal FOV is wide.
    // A node at x=50, y=0, z=10 is far outside any reasonable frustum.
    const positions = makePositions([{ x: 50, y: 0, z: 10 }]);
    const result = frustumCull(positions, DEFAULT_CAMERA);
    expect(result[0]).toBe(false);
  });

  it('degenerate camera (eye === target) returns all false', () => {
    const degenCamera: CameraState = {
      ...DEFAULT_CAMERA,
      eye: [0, 0, 0],
      target: [0, 0, 0],
    };
    const positions = makePositions([
      { x: 0, y: 0, z: 5 },
      { x: 1, y: 0, z: 5 },
    ]);
    const result = frustumCull(positions, degenCamera);
    expect(result[0]).toBe(false);
    expect(result[1]).toBe(false);
  });

  it('multiple nodes: correctly discriminates visible vs culled', () => {
    const camera: CameraState = {
      fovY: Math.PI / 2, // 90°
      aspect: 1,
      near: 0.1,
      far: 50,
      eye: [0, 0, 0],
      target: [0, 0, 1],
    };
    const positions = makePositions([
      { x: 0, y: 0, z: 5 },    // on-axis, visible
      { x: 0, y: 0, z: 200 },  // beyond far, invisible
      { x: 0, y: 0, z: -1 },   // behind camera, invisible
    ]);
    const result = frustumCull(positions, camera);
    expect(result[0]).toBe(true);
    expect(result[1]).toBe(false);
    expect(result[2]).toBe(false);
  });
});

// ─── applyCaps tests ──────────────────────────────────────────────────────────

describe('F6.2.3 applyCaps — BALANCED_NODES limit with priority ordering', () => {
  it('returns empty result for empty input', () => {
    const result = applyCaps([], [], new Map());
    expect(result.visibleNodes).toEqual([]);
    expect(result.visibleEdges).toEqual([]);
    expect(result.visibleLabels).toEqual([]);
    expect(result.truncated).toBe(false);
    expect(result.truncationReason).toBeNull();
  });

  it('passes through items under cap without truncation', () => {
    const items: SceneItem[] = [
      makeItem('n1', { z: 0.1 }),
      makeItem('n2', { z: 0.2 }),
    ];
    const vis = [true, true];
    const result = applyCaps(items, vis, new Map([['n1', 0.1], ['n2', 0.2]]));
    expect(result.visibleNodes).toContain('n1');
    expect(result.visibleNodes).toContain('n2');
    expect(result.truncated).toBe(false);
  });

  it('enforces BALANCED_NODES limit when node count exceeds cap', () => {
    // Create BALANCED_NODES + 5 nodes, all with low priority.
    const items: SceneItem[] = Array.from({ length: SCENE_CAPS.BALANCED_NODES + 5 }, (_, i) =>
      makeItem(`n${i}`, { kind: 'node', z: i * 0.001 }),
    );
    const vis = new Array(items.length).fill(true);
    const result = applyCaps(items, vis, new Map());
    expect(result.visibleNodes).toHaveLength(SCENE_CAPS.BALANCED_NODES);
    expect(result.truncated).toBe(true);
    expect(result.truncationReason).not.toBeNull();
  });

  it('selected items are included before non-selected items', () => {
    // Fill to just under the cap with non-selected items, then add one selected item.
    const nonSelected: SceneItem[] = Array.from(
      { length: SCENE_CAPS.BALANCED_NODES },
      (_, i) => makeItem(`ns${i}`, { kind: 'node', z: 0.5 }),
    );
    const selected = makeItem('selected-node', { kind: 'node', isSelected: true, z: 0.9 });
    const items = [...nonSelected, selected];
    const vis = new Array(items.length).fill(true);
    const result = applyCaps(items, vis, new Map());

    // The selected node must be in the result despite being last in the input.
    expect(result.visibleNodes).toContain('selected-node');
    expect(result.visibleNodes).toHaveLength(SCENE_CAPS.BALANCED_NODES);
  });

  it('focused items are included before non-focused items', () => {
    const nonFocused: SceneItem[] = Array.from(
      { length: SCENE_CAPS.BALANCED_NODES },
      (_, i) => makeItem(`nf${i}`, { kind: 'node', z: 0.5 }),
    );
    const focused = makeItem('focused-node', { kind: 'node', isFocused: true, z: 0.9 });
    const items = [...nonFocused, focused];
    const vis = new Array(items.length).fill(true);
    const result = applyCaps(items, vis, new Map());

    expect(result.visibleNodes).toContain('focused-node');
    expect(result.visibleNodes).toHaveLength(SCENE_CAPS.BALANCED_NODES);
  });

  it('path items are included before regular items', () => {
    const regular: SceneItem[] = Array.from(
      { length: SCENE_CAPS.BALANCED_NODES },
      (_, i) => makeItem(`r${i}`, { kind: 'node', z: 0.5 }),
    );
    const pathItem = makeItem('path-node', { kind: 'node', isInPath: true, z: 0.9 });
    const items = [...regular, pathItem];
    const vis = new Array(items.length).fill(true);
    const result = applyCaps(items, vis, new Map());

    expect(result.visibleNodes).toContain('path-node');
    expect(result.visibleNodes).toHaveLength(SCENE_CAPS.BALANCED_NODES);
  });

  it('closer z-value (smaller) has higher priority than farther z-value', () => {
    // Two nodes: one close (z=0.1), one far (z=0.9). Cap is 1.
    // The close one should be chosen.
    const items: SceneItem[] = [
      makeItem('far-node', { kind: 'node', z: 0.9 }),
      makeItem('close-node', { kind: 'node', z: 0.1 }),
    ];
    // Provide only 1 slot by using a cap trick: fill up with a neutral item then see which wins.
    // We test priority directly: with 2 items and cap=1, close-node should win.
    // Use a synthetic 1-slot test by wrapping applyCaps with a smaller array.
    const oneSlot = [
      makeItem('far-node', { kind: 'node', z: 0.9 }),
      makeItem('close-node', { kind: 'node', z: 0.1 }),
    ];
    const vis = [true, true];
    // Fill up remaining slots with BALANCED_NODES-1 other items all at z=0.5.
    const filler: SceneItem[] = Array.from(
      { length: SCENE_CAPS.BALANCED_NODES - 1 },
      (_, i) => makeItem(`fill${i}`, { kind: 'node', z: 0.5 }),
    );
    const allItems = [...filler, ...oneSlot];
    const allVis = new Array(allItems.length).fill(true);
    const result = applyCaps(allItems, allVis, new Map());

    // Result has exactly BALANCED_NODES items. close-node should be in; far-node should not.
    expect(result.visibleNodes).toContain('close-node');
    expect(result.visibleNodes).not.toContain('far-node');
  });

  it('pre-culled (visibility=false) items are excluded before cap enforcement', () => {
    const items: SceneItem[] = [
      makeItem('culled', { kind: 'node', z: 0.0 }),
      makeItem('visible', { kind: 'node', z: 0.5 }),
    ];
    const vis = [false, true]; // 'culled' is pre-culled
    const result = applyCaps(items, vis, new Map());

    expect(result.visibleNodes).not.toContain('culled');
    expect(result.visibleNodes).toContain('visible');
  });

  it('visibleLabels does not exceed BALANCED_LABELS', () => {
    const items: SceneItem[] = Array.from(
      { length: SCENE_CAPS.BALANCED_LABELS + 10 },
      (_, i) => makeItem(`n${i}`, { kind: 'node', z: i * 0.001 }),
    );
    const vis = new Array(items.length).fill(true);
    const result = applyCaps(items, vis, new Map());
    expect(result.visibleLabels.length).toBeLessThanOrEqual(SCENE_CAPS.BALANCED_LABELS);
  });

  it('truncationReason is null when not truncated', () => {
    const items: SceneItem[] = [makeItem('a'), makeItem('b')];
    const result = applyCaps(items, [true, true], new Map());
    expect(result.truncationReason).toBeNull();
  });

  it('truncationReason is a non-empty string when truncated', () => {
    const items: SceneItem[] = Array.from(
      { length: SCENE_CAPS.BALANCED_NODES + 1 },
      (_, i) => makeItem(`n${i}`),
    );
    const vis = new Array(items.length).fill(true);
    const result = applyCaps(items, vis, new Map());
    expect(result.truncationReason).not.toBeNull();
    expect(typeof result.truncationReason).toBe('string');
    expect((result.truncationReason as string).length).toBeGreaterThan(0);
  });
});

// ─── updateDirtyLabels tests ──────────────────────────────────────────────────

describe('F6.2.3 updateDirtyLabels — bounded dirty label processing', () => {
  it('returns an array of the same length as input', () => {
    const labels: LabelState[] = [
      makeLabel('a', { dirty: true }),
      makeLabel('b', { dirty: false }),
    ];
    const result = updateDirtyLabels(labels, 16);
    expect(result).toHaveLength(labels.length);
  });

  it('processes at most maxUpdatesPerFrame dirty entries per call (exact limit)', () => {
    const labels: LabelState[] = Array.from({ length: 32 }, (_, i) =>
      makeLabel(`l${i}`, { dirty: true, screenX: i * 100, screenY: 0 }),
    );
    const maxPerFrame = 10;
    const result = updateDirtyLabels(labels, maxPerFrame);

    const processedCount = result.filter((l) => !l.dirty).length;
    expect(processedCount).toBe(maxPerFrame);
  });

  it('remaining dirty labels beyond maxUpdatesPerFrame stay dirty', () => {
    const labels: LabelState[] = Array.from({ length: 20 }, (_, i) =>
      makeLabel(`l${i}`, { dirty: true, screenX: i * 100, screenY: 0 }),
    );
    const maxPerFrame = 5;
    const result = updateDirtyLabels(labels, maxPerFrame);

    const stillDirty = result.filter((l) => l.dirty).length;
    expect(stillDirty).toBe(20 - maxPerFrame);
  });

  it('does not mutate the original input array', () => {
    const original: LabelState[] = [
      makeLabel('a', { dirty: true, screenX: 0, screenY: 0 }),
    ];
    const originalDirty = original[0]!.dirty;
    updateDirtyLabels(original, 16);
    // Original must be unchanged.
    expect(original[0]!.dirty).toBe(originalDirty);
  });

  it('clears the dirty flag on processed labels', () => {
    const labels: LabelState[] = [
      makeLabel('a', { dirty: true, screenX: 0, screenY: 0 }),
    ];
    const result = updateDirtyLabels(labels, 16);
    expect(result[0]!.dirty).toBe(false);
  });

  it('non-dirty labels are returned unchanged', () => {
    const label = makeLabel('clean', { dirty: false, screenX: 99, screenY: 42, visible: true });
    const result = updateDirtyLabels([label], 16);
    expect(result[0]!.dirty).toBe(false);
    expect(result[0]!.visible).toBe(true);
    expect(result[0]!.screenX).toBe(99);
    expect(result[0]!.screenY).toBe(42);
  });

  it('uses default maxUpdatesPerFrame of 16 when not specified', () => {
    const labels: LabelState[] = Array.from({ length: 32 }, (_, i) =>
      makeLabel(`l${i}`, { dirty: true, screenX: i * 100, screenY: 0 }),
    );
    const result = updateDirtyLabels(labels);
    const processedCount = result.filter((l) => !l.dirty).length;
    expect(processedCount).toBe(DEFAULT_MAX_UPDATES_PER_FRAME);
  });

  it('DEFAULT_MAX_UPDATES_PER_FRAME is 16', () => {
    expect(DEFAULT_MAX_UPDATES_PER_FRAME).toBe(16);
  });

  it('non-overlapping dirty labels are all marked visible after processing', () => {
    // Each label is spaced 200px apart — no overlap (LABEL_WIDTH_PX is 80).
    const labels: LabelState[] = Array.from({ length: 5 }, (_, i) =>
      makeLabel(`l${i}`, { dirty: true, screenX: i * 200, screenY: 0 }),
    );
    const result = updateDirtyLabels(labels, 5);
    for (const label of result) {
      expect(label.visible).toBe(true);
    }
  });

  it('overlapping dirty labels: first-accepted wins (second is hidden)', () => {
    // Two labels at the same position — first is accepted, second collides.
    const labels: LabelState[] = [
      makeLabel('first', { dirty: true, screenX: 10, screenY: 10 }),
      makeLabel('second', { dirty: true, screenX: 10, screenY: 10 }),
    ];
    const result = updateDirtyLabels(labels, 16);
    expect(result[0]!.visible).toBe(true);
    expect(result[1]!.visible).toBe(false);
  });

  it('processes zero dirty labels when maxUpdatesPerFrame is 0', () => {
    const labels: LabelState[] = [
      makeLabel('a', { dirty: true, screenX: 0, screenY: 0 }),
    ];
    const result = updateDirtyLabels(labels, 0);
    // Nothing processed — label stays dirty.
    expect(result[0]!.dirty).toBe(true);
  });

  it('clean visible labels block dirty labels at the same position (collision)', () => {
    // A clean visible label occupies a position; a dirty label at the same position should collide.
    const labels: LabelState[] = [
      makeLabel('clean', { dirty: false, visible: true, screenX: 0, screenY: 0 }),
      makeLabel('dirty', { dirty: true, screenX: 0, screenY: 0 }),
    ];
    const result = updateDirtyLabels(labels, 16);
    // dirty label collides with clean label — should be invisible.
    expect(result[1]!.visible).toBe(false);
    expect(result[1]!.dirty).toBe(false);
  });
});
