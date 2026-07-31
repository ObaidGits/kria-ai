/**
 * Tests for memory/layout/typography.ts
 */
import { describe, it, expect } from 'vitest';
import {
  MIN_BODY_FONT_SIZE_PX,
  MIN_MAP_LABEL_FONT_SIZE_PX,
  MIN_FOCUS_RING_PX,
  meetsFontSizeRequirement,
  meetsFocusRingRequirement,
} from './typography';

// ─── Constants ────────────────────────────────────────────────────────────────

describe('constants', () => {
  it('MIN_BODY_FONT_SIZE_PX is 14', () => {
    expect(MIN_BODY_FONT_SIZE_PX).toBe(14);
  });

  it('MIN_MAP_LABEL_FONT_SIZE_PX is 12', () => {
    expect(MIN_MAP_LABEL_FONT_SIZE_PX).toBe(12);
  });

  it('MIN_FOCUS_RING_PX is 2', () => {
    expect(MIN_FOCUS_RING_PX).toBe(2);
  });
});

// ─── meetsFontSizeRequirement — body context ──────────────────────────────────

describe('meetsFontSizeRequirement (body)', () => {
  it('14px → true (exactly minimum)', () => {
    expect(meetsFontSizeRequirement(14, 'body')).toBe(true);
  });

  it('13px → false (below minimum)', () => {
    expect(meetsFontSizeRequirement(13, 'body')).toBe(false);
  });

  it('15px → true (above minimum)', () => {
    expect(meetsFontSizeRequirement(15, 'body')).toBe(true);
  });

  it('16px → true', () => {
    expect(meetsFontSizeRequirement(16, 'body')).toBe(true);
  });

  it('0px → false', () => {
    expect(meetsFontSizeRequirement(0, 'body')).toBe(false);
  });
});

// ─── meetsFontSizeRequirement — map-label context ─────────────────────────────

describe('meetsFontSizeRequirement (map-label)', () => {
  it('12px → true (exactly minimum)', () => {
    expect(meetsFontSizeRequirement(12, 'map-label')).toBe(true);
  });

  it('11px → false (below minimum)', () => {
    expect(meetsFontSizeRequirement(11, 'map-label')).toBe(false);
  });

  it('13px → true (above minimum)', () => {
    expect(meetsFontSizeRequirement(13, 'map-label')).toBe(true);
  });

  it('14px → true', () => {
    expect(meetsFontSizeRequirement(14, 'map-label')).toBe(true);
  });

  it('0px → false', () => {
    expect(meetsFontSizeRequirement(0, 'map-label')).toBe(false);
  });
});

// ─── meetsFocusRingRequirement ────────────────────────────────────────────────

describe('meetsFocusRingRequirement', () => {
  it('2px → true (exactly minimum)', () => {
    expect(meetsFocusRingRequirement(2)).toBe(true);
  });

  it('1px → false (below minimum)', () => {
    expect(meetsFocusRingRequirement(1)).toBe(false);
  });

  it('3px → true (above minimum)', () => {
    expect(meetsFocusRingRequirement(3)).toBe(true);
  });

  it('0px → false', () => {
    expect(meetsFocusRingRequirement(0)).toBe(false);
  });

  it('4px → true', () => {
    expect(meetsFocusRingRequirement(4)).toBe(true);
  });
});
