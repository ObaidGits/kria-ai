/**
 * graphCanvas3DSpikeZ.test.ts — F6.2.2 z-axis mapping and position packing unit tests.
 *
 * Validates:
 *   - computeNodeZ edge cases: null, 0.0, 1.0, -1.0 (and interior values).
 *   - mapZValues: origin=0.0, no-score=null, cycle-placeholder=null.
 *   - packNodePositions: correct Float32Array layout (stride, hasZ flag, transfer-ready).
 *
 * Critical constraint (task_6_1_2 preregistration — frozen):
 *   - Formula: z = (1 − cosine_similarity) / 2
 *   - No per-path or global rescaling.
 *   - z=0 for origin (exact), z=null for nodes without vector score.
 *   - Cycle placeholders (kind='navigation-container') always get z=null.
 *   - No numeric default for absent scores (0.5 etc. is a protocol violation).
 *
 * No DOM, no WebGL, no SolidJS rendering — pure logic tests only.
 *
 * Requirements: MGR-001, MGR-002, MGR-004, MGR-012, MGR-026; MGD-003, MGD-026, MGD-046.
 * Spec task: 6.2.2
 *
 * **Validates: Requirements MGR-001, MGR-002, MGR-004, MGR-012, MGR-026**
 */

import { describe, it, expect } from 'vitest';
import {
  computeNodeZ,
  mapZValues,
  packNodePositions,
  PACKED_NODE_STRIDE,
  type SemanticSceneItem3D,
} from './graphCanvas3DSpike';
import type { SemanticSceneItem } from '../scene/semanticScene';

// ─── Fixtures ─────────────────────────────────────────────────────────────────

function makeItem(
  id: string,
  overrides: Partial<SemanticSceneItem> = {},
): SemanticSceneItem {
  return {
    id,
    kind: 'entity',
    authorityClass: 'personal',
    label: `Label for ${id}`,
    truthState: 'confirmed',
    graphRevision: 1,
    direction: null,
    sourceEndpointId: null,
    targetEndpointId: null,
    evidenceCount: 0,
    evidenceSummary: null,
    provenance: { sourceId: null, method: null, version: null, actorLabel: null },
    validity: { validTimeStart: null, validTimeEnd: null, isCurrentlyValid: true },
    isSelected: false,
    isFocused: false,
    isInPath: false,
    isPending: false,
    hasError: false,
    ...overrides,
  };
}

// ─── computeNodeZ edge case tests ─────────────────────────────────────────────

describe('F6.2.2 computeNodeZ — edge cases', () => {
  it('returns null for null input (Unavailable — protocol: no numeric default)', () => {
    expect(computeNodeZ(null)).toBeNull();
  });

  it('returns 0.0 for cosine=1.0 (identical direction — origin anchor)', () => {
    // z = (1 - 1.0) / 2 = 0.0
    expect(computeNodeZ(1.0)).toBe(0.0);
  });

  it('returns 0.5 for cosine=0.0 (orthogonal vectors)', () => {
    // z = (1 - 0.0) / 2 = 0.5
    expect(computeNodeZ(0.0)).toBe(0.5);
  });

  it('returns 1.0 for cosine=-1.0 (maximally opposite vectors)', () => {
    // z = (1 - (-1.0)) / 2 = 2.0 / 2 = 1.0
    expect(computeNodeZ(-1.0)).toBe(1.0);
  });

  it('returns correct midpoint for cosine=0.5', () => {
    // z = (1 - 0.5) / 2 = 0.25
    expect(computeNodeZ(0.5)).toBeCloseTo(0.25, 10);
  });

  it('returns correct value for cosine=-0.5', () => {
    // z = (1 - (-0.5)) / 2 = 1.5 / 2 = 0.75
    expect(computeNodeZ(-0.5)).toBeCloseTo(0.75, 10);
  });

  it('output is within [0.0, 1.0] for all valid cosine scores', () => {
    const testScores = [-1.0, -0.9, -0.5, -0.1, 0.0, 0.1, 0.5, 0.9, 1.0];
    for (const score of testScores) {
      const z = computeNodeZ(score);
      expect(z).not.toBeNull();
      expect(z!).toBeGreaterThanOrEqual(0.0);
      expect(z!).toBeLessThanOrEqual(1.0);
    }
  });

  it('preserves monotonicity: lower cosine → higher z (semantically farther from origin)', () => {
    const z_high_cosine = computeNodeZ(0.8)!;
    const z_low_cosine = computeNodeZ(0.2)!;
    expect(z_high_cosine).toBeLessThan(z_low_cosine);
  });
});

// ─── mapZValues tests ─────────────────────────────────────────────────────────

describe('F6.2.2 mapZValues — origin node', () => {
  it('origin node (first isInPath=true) receives z=0.0 exactly', () => {
    const items: SemanticSceneItem[] = [
      makeItem('a', { isInPath: false }),
      makeItem('origin', { isInPath: true }),
      makeItem('c', { isInPath: true }),
    ];
    // origin has a non-zero score — but z must be 0.0 exactly
    const scores = new Map([['origin', 0.7], ['a', 0.5], ['c', 0.3]]);
    const result = mapZValues(items, scores);

    expect(result.get('origin')).toBe(0.0);
  });

  it('origin is determined by FIRST isInPath=true item, not by score or position', () => {
    const items: SemanticSceneItem[] = [
      makeItem('x', { isInPath: true }),
      makeItem('y', { isInPath: true }),
    ];
    const scores = new Map([['x', 0.9], ['y', 0.6]]);
    const result = mapZValues(items, scores);

    // First item with isInPath=true is origin → z=0
    expect(result.get('x')).toBe(0.0);
    // Second item uses its score
    expect(result.get('y')).toBeCloseTo((1 - 0.6) / 2, 10);
  });

  it('falls back to first item as origin when no item has isInPath=true', () => {
    const items: SemanticSceneItem[] = [
      makeItem('first', { isInPath: false }),
      makeItem('second', { isInPath: false }),
    ];
    const scores = new Map([['first', 0.8], ['second', 0.4]]);
    const result = mapZValues(items, scores);

    expect(result.get('first')).toBe(0.0);
    expect(result.get('second')).toBeCloseTo((1 - 0.4) / 2, 10);
  });
});

describe('F6.2.2 mapZValues — no-score nodes', () => {
  it('returns null for items absent from vectorScores (Unavailable)', () => {
    const items: SemanticSceneItem[] = [
      makeItem('origin', { isInPath: true }),
      makeItem('no-score'),
    ];
    const scores = new Map<string, number>(); // no score for 'no-score'
    const result = mapZValues(items, scores);

    expect(result.get('no-score')).toBeNull();
  });

  it('null is NOT defaulted to 0.5 or any other numeric value (protocol enforcement)', () => {
    const items: SemanticSceneItem[] = [makeItem('origin', { isInPath: true }), makeItem('absent')];
    const scores = new Map<string, number>();
    const result = mapZValues(items, scores);

    const absentZ = result.get('absent');
    expect(absentZ).toBeNull();
    expect(absentZ).not.toBe(0.5);
    expect(absentZ).not.toBe(0.0);
    expect(typeof absentZ).not.toBe('number');
  });

  it('correctly maps items with scores and leaves missing items as null', () => {
    const items: SemanticSceneItem[] = [
      makeItem('origin', { isInPath: true }),
      makeItem('has-score'),
      makeItem('no-score'),
    ];
    const scores = new Map([['origin', 1.0], ['has-score', 0.4]]);
    const result = mapZValues(items, scores);

    expect(result.get('origin')).toBe(0.0);
    expect(result.get('has-score')).toBeCloseTo((1 - 0.4) / 2, 10);
    expect(result.get('no-score')).toBeNull();
  });
});

describe('F6.2.2 mapZValues — cycle placeholder nodes', () => {
  it('navigation-container items always receive z=null regardless of score presence', () => {
    const items: SemanticSceneItem[] = [
      makeItem('origin', { isInPath: true }),
      makeItem('placeholder', { kind: 'navigation-container' }),
    ];
    // Even if the map contains a score for the placeholder, z must be null
    const scores = new Map([['origin', 1.0], ['placeholder', 0.8]]);
    const result = mapZValues(items, scores);

    expect(result.get('placeholder')).toBeNull();
  });

  it('navigation-container items receive z=null even when absent from scores', () => {
    const items: SemanticSceneItem[] = [
      makeItem('origin', { isInPath: true }),
      makeItem('nav', { kind: 'navigation-container' }),
    ];
    const scores = new Map<string, number>();
    const result = mapZValues(items, scores);

    expect(result.get('nav')).toBeNull();
  });

  it('navigation-container z=null is NOT the originating node z value (no borrowing)', () => {
    const items: SemanticSceneItem[] = [
      makeItem('real', { isInPath: true }),
      makeItem('cycle-repeat', { kind: 'navigation-container' }),
    ];
    const scores = new Map([['real', 1.0]]);
    const result = mapZValues(items, scores);

    // real is origin → z=0.0; cycle-repeat must NOT borrow z=0.0 from real
    expect(result.get('real')).toBe(0.0);
    expect(result.get('cycle-repeat')).toBeNull(); // null, not 0.0
  });
});

describe('F6.2.2 mapZValues — general correctness', () => {
  it('returns a Map with an entry for every item in the array', () => {
    const items: SemanticSceneItem[] = [
      makeItem('a', { isInPath: true }),
      makeItem('b'),
      makeItem('c'),
      makeItem('d', { kind: 'navigation-container' }),
    ];
    const scores = new Map([['a', 1.0], ['b', 0.6]]);
    const result = mapZValues(items, scores);

    expect(result.size).toBe(4);
    expect(result.has('a')).toBe(true);
    expect(result.has('b')).toBe(true);
    expect(result.has('c')).toBe(true);
    expect(result.has('d')).toBe(true);
  });

  it('handles empty items array', () => {
    const result = mapZValues([], new Map());
    expect(result.size).toBe(0);
  });

  it('handles single item (origin only)', () => {
    const items: SemanticSceneItem[] = [makeItem('solo', { isInPath: true })];
    const scores = new Map([['solo', 1.0]]);
    const result = mapZValues(items, scores);

    expect(result.get('solo')).toBe(0.0);
  });
});

// ─── packNodePositions tests ──────────────────────────────────────────────────

describe('F6.2.2 packNodePositions — buffer layout', () => {
  it('returns a Float32Array with correct length (items × stride)', () => {
    const items: SemanticSceneItem[] = [makeItem('a'), makeItem('b'), makeItem('c')];
    const zValues = new Map([['a', 0.0], ['b', 0.5], ['c', null]]);
    const buf = packNodePositions(items, zValues);

    expect(buf).toBeInstanceOf(Float32Array);
    expect(buf.length).toBe(items.length * PACKED_NODE_STRIDE);
  });

  it('returns empty Float32Array for empty items', () => {
    const buf = packNodePositions([], new Map());
    expect(buf).toBeInstanceOf(Float32Array);
    expect(buf.length).toBe(0);
  });

  it('nodeIndex field (offset 0) is the 0-based array index', () => {
    const items: SemanticSceneItem[] = [makeItem('x'), makeItem('y'), makeItem('z')];
    const zValues = new Map([['x', 0.2], ['y', 0.4], ['z', 0.6]]);
    const buf = packNodePositions(items, zValues);

    expect(buf[0 * PACKED_NODE_STRIDE + 0]).toBe(0); // item 0
    expect(buf[1 * PACKED_NODE_STRIDE + 0]).toBe(1); // item 1
    expect(buf[2 * PACKED_NODE_STRIDE + 0]).toBe(2); // item 2
  });

  it('hasZ field (offset 4) is 1.0 for items with a z value', () => {
    const items: SemanticSceneItem[] = [makeItem('has'), makeItem('none')];
    const zValues = new Map([['has', 0.3], ['none', null]]);
    const buf = packNodePositions(items, zValues);

    expect(buf[0 * PACKED_NODE_STRIDE + 4]).toBe(1.0); // has z
    expect(buf[1 * PACKED_NODE_STRIDE + 4]).toBe(0.0); // no z
  });

  it('hasZ=0.0 items have z=0.0 in the buffer (not the actual z value)', () => {
    const items: SemanticSceneItem[] = [makeItem('null-z')];
    const zValues = new Map<string, number | null>([['null-z', null]]);
    const buf = packNodePositions(items, zValues);

    const z = buf[0 * PACKED_NODE_STRIDE + 3];
    const hasZ = buf[0 * PACKED_NODE_STRIDE + 4];

    expect(hasZ).toBe(0.0);
    expect(z).toBe(0.0); // stored as 0.0 sentinel; renderer checks hasZ first
  });

  it('z field (offset 3) matches the mapped z value for items with hasZ=1.0', () => {
    const items: SemanticSceneItem[] = [makeItem('a'), makeItem('b')];
    const zValues = new Map([['a', 0.0], ['b', 0.75]]);
    const buf = packNodePositions(items, zValues);

    expect(buf[0 * PACKED_NODE_STRIDE + 3]).toBeCloseTo(0.0, 6);
    expect(buf[0 * PACKED_NODE_STRIDE + 4]).toBe(1.0);

    expect(buf[1 * PACKED_NODE_STRIDE + 3]).toBeCloseTo(0.75, 6);
    expect(buf[1 * PACKED_NODE_STRIDE + 4]).toBe(1.0);
  });

  it('x and y fields (offsets 1, 2) are deterministic for the same id', () => {
    const items: SemanticSceneItem[] = [makeItem('deterministic-id')];
    const zValues = new Map([['deterministic-id', 0.5]]);

    const buf1 = packNodePositions(items, zValues);
    const buf2 = packNodePositions(items, zValues);

    const x1 = buf1[0 * PACKED_NODE_STRIDE + 1];
    const y1 = buf1[0 * PACKED_NODE_STRIDE + 2];
    const x2 = buf2[0 * PACKED_NODE_STRIDE + 1];
    const y2 = buf2[0 * PACKED_NODE_STRIDE + 2];

    expect(x1).toBe(x2);
    expect(y1).toBe(y2);
  });

  it('x and y are in [-1, 1] range', () => {
    const ids = ['alpha', 'beta', 'gamma', 'delta', 'epsilon'];
    const items: SemanticSceneItem[] = ids.map((id) => makeItem(id));
    const zValues = new Map(ids.map((id) => [id, 0.5] as [string, number | null]));
    const buf = packNodePositions(items, zValues);

    for (let i = 0; i < items.length; i++) {
      const x = buf[i * PACKED_NODE_STRIDE + 1]!;
      const y = buf[i * PACKED_NODE_STRIDE + 2]!;
      expect(x).toBeGreaterThanOrEqual(-1.0);
      expect(x).toBeLessThanOrEqual(1.0);
      expect(y).toBeGreaterThanOrEqual(-1.0);
      expect(y).toBeLessThanOrEqual(1.0);
    }
  });

  it('different ids produce different x/y positions (no collision for typical ids)', () => {
    const items: SemanticSceneItem[] = [makeItem('node-a'), makeItem('node-b')];
    const zValues = new Map([['node-a', 0.3], ['node-b', 0.3]]);
    const buf = packNodePositions(items, zValues);

    const xA = buf[0 * PACKED_NODE_STRIDE + 1];
    const yA = buf[0 * PACKED_NODE_STRIDE + 2];
    const xB = buf[1 * PACKED_NODE_STRIDE + 1];
    const yB = buf[1 * PACKED_NODE_STRIDE + 2];

    // Different ids must produce different positions
    expect(xA === xB && yA === yB).toBe(false);
  });

  it('buffer is backed by an ArrayBuffer (transferable — not SharedArrayBuffer)', () => {
    const items: SemanticSceneItem[] = [makeItem('t')];
    const zValues = new Map([['t', 0.1]]);
    const buf = packNodePositions(items, zValues);

    expect(buf.buffer).toBeInstanceOf(ArrayBuffer);
    expect(buf.buffer).not.toBeInstanceOf(SharedArrayBuffer);
  });

  it('full integration: origin node z=0.0 in packed buffer', () => {
    const items: SemanticSceneItem[] = [
      makeItem('origin', { isInPath: true }),
      makeItem('far', { isInPath: false }),
    ];
    const scores = new Map([['origin', 1.0], ['far', -0.5]]);
    const zValues = mapZValues(items, scores);
    const buf = packNodePositions(items, zValues);

    // origin: index=0, hasZ=1.0, z=0.0
    expect(buf[0 * PACKED_NODE_STRIDE + 3]).toBe(0.0);
    expect(buf[0 * PACKED_NODE_STRIDE + 4]).toBe(1.0);

    // far: z = (1 - (-0.5)) / 2 = 0.75, hasZ=1.0
    expect(buf[1 * PACKED_NODE_STRIDE + 3]).toBeCloseTo(0.75, 6);
    expect(buf[1 * PACKED_NODE_STRIDE + 4]).toBe(1.0);
  });

  it('full integration: null z → hasZ=0 in packed buffer', () => {
    const items: SemanticSceneItem[] = [
      makeItem('origin', { isInPath: true }),
      makeItem('placeholder', { kind: 'navigation-container' }),
      makeItem('no-score'),
    ];
    const scores = new Map<string, number>(); // no scores at all
    const zValues = mapZValues(items, scores);
    const buf = packNodePositions(items, zValues);

    // origin: z=0.0, hasZ=1.0 (origin always gets z=0 exactly)
    expect(buf[0 * PACKED_NODE_STRIDE + 3]).toBe(0.0);
    expect(buf[0 * PACKED_NODE_STRIDE + 4]).toBe(1.0);

    // placeholder: z=null → hasZ=0
    expect(buf[1 * PACKED_NODE_STRIDE + 4]).toBe(0.0);

    // no-score: z=null → hasZ=0
    expect(buf[2 * PACKED_NODE_STRIDE + 4]).toBe(0.0);
  });
});

// ─── SemanticSceneItem3D interface type test ──────────────────────────────────

describe('F6.2.2 SemanticSceneItem3D — interface extends SemanticSceneItem', () => {
  it('SemanticSceneItem3D can be constructed and z_value is present', () => {
    // Type-level test: ensure the interface is correctly declared.
    const item3D: SemanticSceneItem3D = {
      ...makeItem('i3d', { isInPath: true }),
      z_value: 0.0,
    };
    expect(item3D.z_value).toBe(0.0);
  });

  it('SemanticSceneItem3D z_value accepts null for Unavailable', () => {
    const item3D: SemanticSceneItem3D = {
      ...makeItem('i3d-null'),
      z_value: null,
    };
    expect(item3D.z_value).toBeNull();
  });

  it('SemanticSceneItem3D inherits all SemanticSceneItem fields', () => {
    const base = makeItem('base-item', { isInPath: true, isSelected: true });
    const item3D: SemanticSceneItem3D = { ...base, z_value: 0.5 };

    // Inherited fields are present and correct
    expect(item3D.id).toBe('base-item');
    expect(item3D.isInPath).toBe(true);
    expect(item3D.isSelected).toBe(true);
    expect(item3D.z_value).toBe(0.5);
  });
});
