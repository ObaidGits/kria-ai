/**
 * Tests for memory/layout/focusManager.ts
 */
import { describe, it, expect } from 'vitest';
import {
  FOCUS_RETURN_DELAY_MS,
  createFocusGuard,
  recordFocus,
  clearFocus,
  getReturnTarget,
} from './focusManager';

// ─── Constants ────────────────────────────────────────────────────────────────

describe('FOCUS_RETURN_DELAY_MS', () => {
  it('is 0 (synchronous-next-tick via setTimeout)', () => {
    expect(FOCUS_RETURN_DELAY_MS).toBe(0);
  });
});

// ─── createFocusGuard ─────────────────────────────────────────────────────────

describe('createFocusGuard', () => {
  it('sets containerId correctly', () => {
    const guard = createFocusGuard('dialog-container');
    expect(guard.containerId).toBe('dialog-container');
  });

  it('sets lastFocusedId to null', () => {
    const guard = createFocusGuard('some-container');
    expect(guard.lastFocusedId).toBeNull();
  });

  it('creates independent guards for different containers', () => {
    const a = createFocusGuard('container-a');
    const b = createFocusGuard('container-b');
    expect(a.containerId).toBe('container-a');
    expect(b.containerId).toBe('container-b');
  });
});

// ─── recordFocus ──────────────────────────────────────────────────────────────

describe('recordFocus', () => {
  it('updates lastFocusedId', () => {
    const guard = createFocusGuard('container');
    const updated = recordFocus(guard, 'btn-close');
    expect(updated.lastFocusedId).toBe('btn-close');
  });

  it('preserves containerId', () => {
    const guard = createFocusGuard('container');
    const updated = recordFocus(guard, 'btn-close');
    expect(updated.containerId).toBe('container');
  });

  it('does not mutate the input guard (pure function)', () => {
    const guard = createFocusGuard('container');
    recordFocus(guard, 'btn-close');
    expect(guard.lastFocusedId).toBeNull();
  });

  it('can overwrite a previously recorded focus', () => {
    const guard = createFocusGuard('container');
    const step1 = recordFocus(guard, 'element-1');
    const step2 = recordFocus(step1, 'element-2');
    expect(step2.lastFocusedId).toBe('element-2');
    // step1 is unchanged
    expect(step1.lastFocusedId).toBe('element-1');
  });
});

// ─── clearFocus ───────────────────────────────────────────────────────────────

describe('clearFocus', () => {
  it('sets lastFocusedId to null', () => {
    const guard = recordFocus(createFocusGuard('container'), 'btn');
    const cleared = clearFocus(guard);
    expect(cleared.lastFocusedId).toBeNull();
  });

  it('preserves containerId', () => {
    const guard = recordFocus(createFocusGuard('container'), 'btn');
    const cleared = clearFocus(guard);
    expect(cleared.containerId).toBe('container');
  });

  it('does not mutate the input guard (pure function)', () => {
    const guard = recordFocus(createFocusGuard('container'), 'btn');
    clearFocus(guard);
    expect(guard.lastFocusedId).toBe('btn');
  });

  it('clearing an already-null guard returns null', () => {
    const guard = createFocusGuard('container');
    const cleared = clearFocus(guard);
    expect(cleared.lastFocusedId).toBeNull();
  });
});

// ─── getReturnTarget ──────────────────────────────────────────────────────────

describe('getReturnTarget', () => {
  it('returns null when no focus recorded', () => {
    const guard = createFocusGuard('container');
    expect(getReturnTarget(guard)).toBeNull();
  });

  it('returns lastFocusedId when focus has been recorded', () => {
    const guard = recordFocus(createFocusGuard('container'), 'trigger-btn');
    expect(getReturnTarget(guard)).toBe('trigger-btn');
  });

  it('returns null after clear', () => {
    const guard = clearFocus(recordFocus(createFocusGuard('container'), 'trigger-btn'));
    expect(getReturnTarget(guard)).toBeNull();
  });
});

// ─── Purity / no mutation ─────────────────────────────────────────────────────

describe('all functions are pure (no mutation)', () => {
  it('a sequence of operations does not alter the original guard', () => {
    const original = createFocusGuard('root');
    const g1 = recordFocus(original, 'a');
    const g2 = recordFocus(g1, 'b');
    const g3 = clearFocus(g2);

    // None of these operations should have mutated original
    expect(original.containerId).toBe('root');
    expect(original.lastFocusedId).toBeNull();

    // Intermediate states are also intact
    expect(g1.lastFocusedId).toBe('a');
    expect(g2.lastFocusedId).toBe('b');
    expect(g3.lastFocusedId).toBeNull();
  });
});
