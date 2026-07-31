/**
 * Tests for memory/layout/motionTokens.ts
 */
import { describe, it, expect } from 'vitest';
import {
  MOTION,
  getMotionDuration,
  isWithinHardMax,
  isWithinReducedMotionMax,
} from './motionTokens';
import type { MotionToken } from './motionTokens';

// ─── MOTION constants ─────────────────────────────────────────────────────────

describe('MOTION constants', () => {
  it('FOCUS_MS is 80', () => {
    expect(MOTION.FOCUS_MS).toBe(80);
  });

  it('SELECTION_MS is 120', () => {
    expect(MOTION.SELECTION_MS).toBe(120);
  });

  it('INSPECTOR_MS is 180', () => {
    expect(MOTION.INSPECTOR_MS).toBe(180);
  });

  it('CAMERA_MS is 220', () => {
    expect(MOTION.CAMERA_MS).toBe(220);
  });

  it('SCENE_MS is 300', () => {
    expect(MOTION.SCENE_MS).toBe(300);
  });

  it('TEMPORAL_MS is 320', () => {
    expect(MOTION.TEMPORAL_MS).toBe(320);
  });

  it('INFERRED_STORED_MS is 240', () => {
    expect(MOTION.INFERRED_STORED_MS).toBe(240);
  });

  it('STATUS_MS is 120', () => {
    expect(MOTION.STATUS_MS).toBe(120);
  });

  it('HARD_MAX_MS is 400', () => {
    expect(MOTION.HARD_MAX_MS).toBe(400);
  });

  it('REDUCED_MOTION_MAX_MS is 80', () => {
    expect(MOTION.REDUCED_MOTION_MAX_MS).toBe(80);
  });
});

// ─── getMotionDuration ────────────────────────────────────────────────────────

describe('getMotionDuration', () => {
  const cases: Array<[MotionToken, number]> = [
    ['FOCUS_MS', 80],
    ['SELECTION_MS', 120],
    ['INSPECTOR_MS', 180],
    ['CAMERA_MS', 220],
    ['SCENE_MS', 300],
    ['TEMPORAL_MS', 320],
    ['INFERRED_STORED_MS', 240],
    ['STATUS_MS', 120],
    ['HARD_MAX_MS', 400],
    ['REDUCED_MOTION_MAX_MS', 80],
  ];

  for (const [token, expected] of cases) {
    it(`getMotionDuration(${token}) → ${expected}`, () => {
      expect(getMotionDuration(token)).toBe(expected);
    });
  }
});

// ─── isWithinHardMax ──────────────────────────────────────────────────────────

describe('isWithinHardMax', () => {
  it('400ms → true (exactly at hard max)', () => {
    expect(isWithinHardMax(400)).toBe(true);
  });

  it('401ms → false (exceeds hard max)', () => {
    expect(isWithinHardMax(401)).toBe(false);
  });

  it('0ms → true', () => {
    expect(isWithinHardMax(0)).toBe(true);
  });

  it('320ms → true', () => {
    expect(isWithinHardMax(320)).toBe(true);
  });

  it('500ms → false', () => {
    expect(isWithinHardMax(500)).toBe(false);
  });
});

// ─── isWithinReducedMotionMax ─────────────────────────────────────────────────

describe('isWithinReducedMotionMax', () => {
  it('80ms → true (exactly at reduced-motion max)', () => {
    expect(isWithinReducedMotionMax(80)).toBe(true);
  });

  it('81ms → false (exceeds reduced-motion max)', () => {
    expect(isWithinReducedMotionMax(81)).toBe(false);
  });

  it('0ms → true', () => {
    expect(isWithinReducedMotionMax(0)).toBe(true);
  });

  it('79ms → true', () => {
    expect(isWithinReducedMotionMax(79)).toBe(true);
  });

  it('120ms → false', () => {
    expect(isWithinReducedMotionMax(120)).toBe(false);
  });
});

// ─── Invariant: all MOTION values are > 0 and ≤ HARD_MAX_MS ─────────────────

describe('MOTION value invariants', () => {
  const tokens = Object.keys(MOTION) as MotionToken[];

  it('all token values are greater than 0', () => {
    for (const token of tokens) {
      expect(MOTION[token]).toBeGreaterThan(0);
    }
  });

  it('all token values are ≤ HARD_MAX_MS (400)', () => {
    for (const token of tokens) {
      expect(MOTION[token]).toBeLessThanOrEqual(MOTION.HARD_MAX_MS);
    }
  });
});
