/**
 * Tests for memory/layout/pointerTargets.ts
 */
import { describe, it, expect } from 'vitest';
import {
  MIN_COARSE_TARGET_PX,
  meetsCoarseTarget,
  getCoarseTargetDimension,
} from './pointerTargets';

// ─── Constants ────────────────────────────────────────────────────────────────

describe('MIN_COARSE_TARGET_PX', () => {
  it('is 44', () => {
    expect(MIN_COARSE_TARGET_PX).toBe(44);
  });
});

// ─── meetsCoarseTarget ────────────────────────────────────────────────────────

describe('meetsCoarseTarget', () => {
  it('returns true when both dimensions are exactly 44', () => {
    expect(meetsCoarseTarget(44, 44)).toBe(true);
  });

  it('returns false when width is too small', () => {
    expect(meetsCoarseTarget(43, 44)).toBe(false);
  });

  it('returns false when height is too small', () => {
    expect(meetsCoarseTarget(44, 43)).toBe(false);
  });

  it('returns true when both dimensions are larger than minimum', () => {
    expect(meetsCoarseTarget(100, 100)).toBe(true);
  });

  it('returns false when both dimensions are below minimum', () => {
    expect(meetsCoarseTarget(20, 20)).toBe(false);
  });

  it('returns true for typical large interactive elements', () => {
    expect(meetsCoarseTarget(56, 56)).toBe(true);
    expect(meetsCoarseTarget(48, 48)).toBe(true);
  });

  it('returns false for 0x0', () => {
    expect(meetsCoarseTarget(0, 0)).toBe(false);
  });
});

// ─── getCoarseTargetDimension ─────────────────────────────────────────────────

describe('getCoarseTargetDimension', () => {
  it('returns 44 when current is below minimum (20)', () => {
    expect(getCoarseTargetDimension(20)).toBe(44);
  });

  it('returns 44 when current is exactly minimum (44)', () => {
    expect(getCoarseTargetDimension(44)).toBe(44);
  });

  it('returns current when current exceeds minimum (50)', () => {
    expect(getCoarseTargetDimension(50)).toBe(50);
  });

  it('returns 44 when current is 0', () => {
    expect(getCoarseTargetDimension(0)).toBe(44);
  });

  it('returns 44 when current is 43', () => {
    expect(getCoarseTargetDimension(43)).toBe(44);
  });

  it('returns 100 when current is 100', () => {
    expect(getCoarseTargetDimension(100)).toBe(100);
  });
});
