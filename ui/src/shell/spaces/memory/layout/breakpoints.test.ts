/**
 * Tests for memory/layout/breakpoints.ts
 */
import { describe, it, expect } from 'vitest';
import {
  BREAKPOINT_LARGE,
  BREAKPOINT_MEDIUM,
  NAV_WIDTH_LARGE,
  NAV_WIDTH_MEDIUM,
  WORKSPACE_MIN_WIDTH,
  INSPECTOR_WIDTH_LARGE,
  INSPECTOR_WIDTH_MEDIUM,
  getLayoutMode,
  getNavWidth,
  getInspectorWidth,
} from './breakpoints';

// ─── Constants ────────────────────────────────────────────────────────────────

describe('breakpoint constants', () => {
  it('BREAKPOINT_LARGE is 1200', () => {
    expect(BREAKPOINT_LARGE).toBe(1200);
  });

  it('BREAKPOINT_MEDIUM is 800', () => {
    expect(BREAKPOINT_MEDIUM).toBe(800);
  });

  it('NAV_WIDTH_LARGE is 240', () => {
    expect(NAV_WIDTH_LARGE).toBe(240);
  });

  it('NAV_WIDTH_MEDIUM is 72', () => {
    expect(NAV_WIDTH_MEDIUM).toBe(72);
  });

  it('WORKSPACE_MIN_WIDTH is 560', () => {
    expect(WORKSPACE_MIN_WIDTH).toBe(560);
  });

  it('INSPECTOR_WIDTH_LARGE is 360', () => {
    expect(INSPECTOR_WIDTH_LARGE).toBe(360);
  });

  it('INSPECTOR_WIDTH_MEDIUM is 320', () => {
    expect(INSPECTOR_WIDTH_MEDIUM).toBe(320);
  });
});

// ─── getLayoutMode ────────────────────────────────────────────────────────────

describe('getLayoutMode', () => {
  it('returns large at exactly 1200px', () => {
    expect(getLayoutMode(1200)).toBe('large');
  });

  it('returns large above 1200px', () => {
    expect(getLayoutMode(1440)).toBe('large');
    expect(getLayoutMode(1920)).toBe('large');
    expect(getLayoutMode(2560)).toBe('large');
  });

  it('returns medium at exactly 800px', () => {
    expect(getLayoutMode(800)).toBe('medium');
  });

  it('returns medium between 800 and 1199px', () => {
    expect(getLayoutMode(801)).toBe('medium');
    expect(getLayoutMode(1000)).toBe('medium');
    expect(getLayoutMode(1199)).toBe('medium');
  });

  it('returns small below 800px', () => {
    expect(getLayoutMode(799)).toBe('small');
    expect(getLayoutMode(600)).toBe('small');
    expect(getLayoutMode(320)).toBe('small');
    expect(getLayoutMode(0)).toBe('small');
  });
});

// ─── getNavWidth ──────────────────────────────────────────────────────────────

describe('getNavWidth', () => {
  it('returns 240 for large mode', () => {
    expect(getNavWidth('large')).toBe(240);
  });

  it('returns 72 for medium mode', () => {
    expect(getNavWidth('medium')).toBe(72);
  });

  it('returns 0 for small mode', () => {
    expect(getNavWidth('small')).toBe(0);
  });
});

// ─── getInspectorWidth ────────────────────────────────────────────────────────

describe('getInspectorWidth', () => {
  it('returns 360 for large mode', () => {
    expect(getInspectorWidth('large')).toBe(360);
  });

  it('returns 320 for medium mode', () => {
    expect(getInspectorWidth('medium')).toBe(320);
  });

  it('returns 0 for small mode (full-height sheet)', () => {
    expect(getInspectorWidth('small')).toBe(0);
  });
});
