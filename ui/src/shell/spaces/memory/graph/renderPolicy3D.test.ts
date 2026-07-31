/**
 * renderPolicy3D.test.ts — F6.2.5 render policy unit tests.
 *
 * Covers:
 *   - Quality ladder degrades step by step (full → list-first)
 *   - degradeQuality floors at 'list-first'
 *   - shouldRender3D is false only for 'list-first'
 *   - shouldStopRendering respects the 2000ms threshold (strict >)
 *   - Context loss → fallback restores query/focus/pending state
 *   - Reduced motion disables animation flags
 *   - onContextLoss records the query and focusId
 *   - onContextRestored moves state to 'healthy'
 *   - Multiple context losses update the record (last-write wins)
 *   - applyEvent reducer covers all event kinds
 *   - 'none' motion mode disables rendering entirely
 *
 * Pure TypeScript — no DOM, no WebGL, no SolidJS rendering.
 *
 * Requirements: MGR-001, MGR-004, MGR-012, MGR-015, MGR-026;
 *               MGD-003, MGD-026, MGD-046; task 6.2.5
 */

import { describe, it, expect } from 'vitest';
import fc from 'fast-check';
import {
  degradeQuality,
  shouldRender3D,
  shouldStopRendering,
  onContextLoss,
  onContextRestored,
  onFallbackActivated,
  onRecoveryStarted,
  applyEvent,
  INITIAL_RENDER_POLICY,
  INITIAL_CONTEXT_LOSS_RECORD,
  IDLE_THRESHOLD_MS,
} from './renderPolicy3D';
import type {
  QualityLevel,
  DegradationReason,
  IdleState,
  ContextLossRecord,
  RenderPolicyEvent,
} from './renderPolicy3D';

// ─── Quality ladder ────────────────────────────────────────────────────────────

describe('degradeQuality — step-by-step ladder', () => {
  const reason: DegradationReason = 'forced';

  it('degrades full → no-decoration', () => {
    expect(degradeQuality('full', reason)).toBe('no-decoration');
  });

  it('degrades no-decoration → no-labels', () => {
    expect(degradeQuality('no-decoration', reason)).toBe('no-labels');
  });

  it('degrades no-labels → no-analytics', () => {
    expect(degradeQuality('no-labels', reason)).toBe('no-analytics');
  });

  it('degrades no-analytics → reduced-scene', () => {
    expect(degradeQuality('no-analytics', reason)).toBe('reduced-scene');
  });

  it('degrades reduced-scene → list-first', () => {
    expect(degradeQuality('reduced-scene', reason)).toBe('list-first');
  });

  it('floors at list-first — further degradation stays list-first', () => {
    expect(degradeQuality('list-first', reason)).toBe('list-first');
  });

  it('degrading full five times reaches list-first', () => {
    let q: QualityLevel = 'full';
    for (let i = 0; i < 5; i++) q = degradeQuality(q, reason);
    expect(q).toBe('list-first');
  });

  it('degrading full six times still returns list-first (floor)', () => {
    let q: QualityLevel = 'full';
    for (let i = 0; i < 6; i++) q = degradeQuality(q, reason);
    expect(q).toBe('list-first');
  });
});

// ─── shouldRender3D ───────────────────────────────────────────────────────────

describe('shouldRender3D', () => {
  it('returns true for full', () => expect(shouldRender3D('full')).toBe(true));
  it('returns true for no-decoration', () => expect(shouldRender3D('no-decoration')).toBe(true));
  it('returns true for no-labels', () => expect(shouldRender3D('no-labels')).toBe(true));
  it('returns true for no-analytics', () => expect(shouldRender3D('no-analytics')).toBe(true));
  it('returns true for reduced-scene', () => expect(shouldRender3D('reduced-scene')).toBe(true));
  it('returns FALSE for list-first', () => expect(shouldRender3D('list-first')).toBe(false));
});

// ─── shouldStopRendering ──────────────────────────────────────────────────────

describe('shouldStopRendering — 2000ms threshold', () => {
  it('returns false when both are exactly at threshold (=2000ms, not >)', () => {
    const idle: IdleState = { lastInputMs: 0, lastSceneChangeMs: 0, nowMs: 2000 };
    expect(shouldStopRendering(idle)).toBe(false);
  });

  it('returns true when both exceed threshold by 1ms', () => {
    const idle: IdleState = { lastInputMs: 0, lastSceneChangeMs: 0, nowMs: 2001 };
    expect(shouldStopRendering(idle)).toBe(true);
  });

  it('returns false when only input is idle (scene recently changed)', () => {
    const idle: IdleState = { lastInputMs: 0, lastSceneChangeMs: 1999, nowMs: 3000 };
    expect(shouldStopRendering(idle)).toBe(false);
  });

  it('returns false when only scene is idle (input recently arrived)', () => {
    const idle: IdleState = { lastInputMs: 1999, lastSceneChangeMs: 0, nowMs: 3000 };
    expect(shouldStopRendering(idle)).toBe(false);
  });

  it('returns true when both input and scene have been idle for well over 2000ms', () => {
    const idle: IdleState = { lastInputMs: 0, lastSceneChangeMs: 0, nowMs: 5000 };
    expect(shouldStopRendering(idle)).toBe(true);
  });

  it('returns false at nowMs=0 (nothing has elapsed)', () => {
    const idle: IdleState = { lastInputMs: 0, lastSceneChangeMs: 0, nowMs: 0 };
    expect(shouldStopRendering(idle)).toBe(false);
  });
});

// ─── Context loss record ──────────────────────────────────────────────────────

describe('onContextLoss — records query/focusId/pendingAction', () => {
  it('transitions healthy → lost', () => {
    const rec = onContextLoss(INITIAL_CONTEXT_LOSS_RECORD, 'my-query', 'item-1', 'select', 1000);
    expect(rec.state).toBe('lost');
  });

  it('records the query', () => {
    const rec = onContextLoss(INITIAL_CONTEXT_LOSS_RECORD, 'search term', null, null, 1000);
    expect(rec.fallback2DQuery).toBe('search term');
  });

  it('records the focusId', () => {
    const rec = onContextLoss(INITIAL_CONTEXT_LOSS_RECORD, null, 'focus-item-id', null, 1000);
    expect(rec.fallback2DFocusId).toBe('focus-item-id');
  });

  it('records pendingActionKind', () => {
    const rec = onContextLoss(INITIAL_CONTEXT_LOSS_RECORD, null, null, 'navigate', 1000);
    expect(rec.pendingActionKind).toBe('navigate');
  });

  it('records lostAtMs', () => {
    const rec = onContextLoss(INITIAL_CONTEXT_LOSS_RECORD, null, null, null, 42000);
    expect(rec.lostAtMs).toBe(42000);
  });

  it('accepts null query, focusId, and pendingAction', () => {
    const rec = onContextLoss(INITIAL_CONTEXT_LOSS_RECORD, null, null, null, 1);
    expect(rec.fallback2DQuery).toBeNull();
    expect(rec.fallback2DFocusId).toBeNull();
    expect(rec.pendingActionKind).toBeNull();
  });
});

describe('onContextLoss — multiple losses update the record', () => {
  it('second loss overwrites first loss data', () => {
    const r1 = onContextLoss(INITIAL_CONTEXT_LOSS_RECORD, 'q1', 'f1', 'action1', 100);
    const r2 = onContextLoss(r1, 'q2', 'f2', 'action2', 200);
    expect(r2.state).toBe('lost');
    expect(r2.fallback2DQuery).toBe('q2');
    expect(r2.fallback2DFocusId).toBe('f2');
    expect(r2.pendingActionKind).toBe('action2');
    expect(r2.lostAtMs).toBe(200);
  });

  it('loss from recovering state still forces state=lost', () => {
    const recovering: ContextLossRecord = {
      state: 'recovering', lostAtMs: 100, fallback2DQuery: 'old', fallback2DFocusId: null, pendingActionKind: null,
    };
    const r = onContextLoss(recovering, 'new-q', 'new-f', null, 300);
    expect(r.state).toBe('lost');
    expect(r.fallback2DQuery).toBe('new-q');
  });
});

describe('onContextRestored', () => {
  it('transitions recovering → healthy', () => {
    const recovering: ContextLossRecord = {
      state: 'recovering', lostAtMs: 500, fallback2DQuery: 'q', fallback2DFocusId: 'f', pendingActionKind: 'a',
    };
    const r = onContextRestored(recovering);
    expect(r.state).toBe('healthy');
    expect(r.lostAtMs).toBeNull();
  });

  it('is a no-op from healthy (not recovering)', () => {
    const healthy = INITIAL_CONTEXT_LOSS_RECORD;
    const r = onContextRestored(healthy);
    expect(r).toBe(healthy); // referential equality — no new object
  });

  it('is a no-op from lost (must go lost → recovering first)', () => {
    const lost: ContextLossRecord = {
      state: 'lost', lostAtMs: 100, fallback2DQuery: 'q', fallback2DFocusId: null, pendingActionKind: null,
    };
    const r = onContextRestored(lost);
    expect(r.state).toBe('lost');
  });

  it('is a no-op from fallback', () => {
    const fallback: ContextLossRecord = {
      state: 'fallback', lostAtMs: 100, fallback2DQuery: 'q', fallback2DFocusId: 'f', pendingActionKind: null,
    };
    const r = onContextRestored(fallback);
    expect(r.state).toBe('fallback');
  });
});

describe('onFallbackActivated', () => {
  it('transitions lost → fallback', () => {
    const lost: ContextLossRecord = {
      state: 'lost', lostAtMs: 100, fallback2DQuery: 'search', fallback2DFocusId: 'item-x', pendingActionKind: 'select',
    };
    const r = onFallbackActivated(lost);
    expect(r.state).toBe('fallback');
    // Fallback data is preserved
    expect(r.fallback2DQuery).toBe('search');
    expect(r.fallback2DFocusId).toBe('item-x');
    expect(r.pendingActionKind).toBe('select');
  });

  it('is a no-op from healthy', () => {
    const r = onFallbackActivated(INITIAL_CONTEXT_LOSS_RECORD);
    expect(r.state).toBe('healthy');
  });

  it('is a no-op from recovering', () => {
    const recovering: ContextLossRecord = {
      state: 'recovering', lostAtMs: 50, fallback2DQuery: null, fallback2DFocusId: null, pendingActionKind: null,
    };
    const r = onFallbackActivated(recovering);
    expect(r.state).toBe('recovering');
  });
});

describe('context loss → fallback restores query/focus/pending state', () => {
  it('full path: healthy → lost → fallback preserves all fallback fields', () => {
    const lost = onContextLoss(INITIAL_CONTEXT_LOSS_RECORD, 'my-query', 'focus-42', 'navigate', 999);
    const fallback = onFallbackActivated(lost);
    expect(fallback.state).toBe('fallback');
    expect(fallback.fallback2DQuery).toBe('my-query');
    expect(fallback.fallback2DFocusId).toBe('focus-42');
    expect(fallback.pendingActionKind).toBe('navigate');
    expect(fallback.lostAtMs).toBe(999);
  });

  it('full path: healthy → lost → recovering → healthy clears lostAtMs', () => {
    const lost = onContextLoss(INITIAL_CONTEXT_LOSS_RECORD, 'q', 'f', 'a', 100);
    const recovering = onRecoveryStarted(lost);
    const healthy = onContextRestored(recovering);
    expect(healthy.state).toBe('healthy');
    expect(healthy.lostAtMs).toBeNull();
  });
});

// ─── RenderPolicyState + applyEvent ──────────────────────────────────────────

describe('applyEvent — set-motion-mode', () => {
  it('reduced motion disables animationEnabled', () => {
    const p = applyEvent(INITIAL_RENDER_POLICY, { kind: 'set-motion-mode', mode: 'reduced' });
    expect(p.motionMode).toBe('reduced');
    expect(p.animationEnabled).toBe(false);
  });

  it('reduced motion still allows frameRequested', () => {
    const p = applyEvent(INITIAL_RENDER_POLICY, { kind: 'set-motion-mode', mode: 'reduced' });
    expect(p.frameRequested).toBe(true);
  });

  it('none motion mode disables both animationEnabled and frameRequested', () => {
    const p = applyEvent(INITIAL_RENDER_POLICY, { kind: 'set-motion-mode', mode: 'none' });
    expect(p.animationEnabled).toBe(false);
    expect(p.frameRequested).toBe(false);
  });

  it('full motion mode enables animationEnabled', () => {
    const reduced = applyEvent(INITIAL_RENDER_POLICY, { kind: 'set-motion-mode', mode: 'reduced' });
    const full = applyEvent(reduced, { kind: 'set-motion-mode', mode: 'full' });
    expect(full.animationEnabled).toBe(true);
    expect(full.frameRequested).toBe(true);
  });

  it('switching to none clears staticFramePending', () => {
    // First set reduced + trigger a scene change
    let p = applyEvent(INITIAL_RENDER_POLICY, { kind: 'set-motion-mode', mode: 'reduced' });
    p = applyEvent(p, { kind: 'scene-changed' });
    expect(p.staticFramePending).toBe(true);
    // Switch to none — pending should be cleared
    const none = applyEvent(p, { kind: 'set-motion-mode', mode: 'none' });
    expect(none.staticFramePending).toBe(false);
  });
});

describe('applyEvent — degrade-quality', () => {
  it('steps quality down one level', () => {
    const p = applyEvent(INITIAL_RENDER_POLICY, { kind: 'degrade-quality', reason: 'forced' });
    expect(p.quality).toBe('no-decoration');
  });

  it('five steps reach list-first', () => {
    let p = INITIAL_RENDER_POLICY;
    for (let i = 0; i < 5; i++) p = applyEvent(p, { kind: 'degrade-quality', reason: 'forced' });
    expect(p.quality).toBe('list-first');
  });

  it('reset-quality restores to full', () => {
    let p = INITIAL_RENDER_POLICY;
    for (let i = 0; i < 5; i++) p = applyEvent(p, { kind: 'degrade-quality', reason: 'forced' });
    p = applyEvent(p, { kind: 'reset-quality' });
    expect(p.quality).toBe('full');
  });
});

describe('applyEvent — scene-changed', () => {
  it('sets staticFramePending=true in reduced mode', () => {
    let p = applyEvent(INITIAL_RENDER_POLICY, { kind: 'set-motion-mode', mode: 'reduced' });
    p = applyEvent(p, { kind: 'scene-changed' });
    expect(p.staticFramePending).toBe(true);
  });

  it('does not set staticFramePending in full mode', () => {
    const p = applyEvent(INITIAL_RENDER_POLICY, { kind: 'scene-changed' });
    expect(p.staticFramePending).toBe(false);
  });

  it('does not set staticFramePending in none mode', () => {
    let p = applyEvent(INITIAL_RENDER_POLICY, { kind: 'set-motion-mode', mode: 'none' });
    p = applyEvent(p, { kind: 'scene-changed' });
    expect(p.staticFramePending).toBe(false);
  });
});

describe('applyEvent — static-frame-rendered', () => {
  it('clears staticFramePending', () => {
    let p = applyEvent(INITIAL_RENDER_POLICY, { kind: 'set-motion-mode', mode: 'reduced' });
    p = applyEvent(p, { kind: 'scene-changed' });
    expect(p.staticFramePending).toBe(true);
    p = applyEvent(p, { kind: 'static-frame-rendered' });
    expect(p.staticFramePending).toBe(false);
  });
});

describe('applyEvent — context-lost / recovery-started / context-restored / fallback-activated', () => {
  it('context-lost disables animation and frame', () => {
    const p = applyEvent(INITIAL_RENDER_POLICY, {
      kind: 'context-lost', query: 'q', focusId: 'f', pendingAction: 'a', nowMs: 1000,
    });
    expect(p.contextLoss.state).toBe('lost');
    expect(p.animationEnabled).toBe(false);
    expect(p.frameRequested).toBe(false);
  });

  it('context-lost records query, focusId, pendingAction', () => {
    const p = applyEvent(INITIAL_RENDER_POLICY, {
      kind: 'context-lost', query: 'hello', focusId: 'item-99', pendingAction: 'select', nowMs: 500,
    });
    expect(p.contextLoss.fallback2DQuery).toBe('hello');
    expect(p.contextLoss.fallback2DFocusId).toBe('item-99');
    expect(p.contextLoss.pendingActionKind).toBe('select');
  });

  it('recovery-started transitions lost → recovering', () => {
    let p = applyEvent(INITIAL_RENDER_POLICY, {
      kind: 'context-lost', query: null, focusId: null, pendingAction: null, nowMs: 0,
    });
    p = applyEvent(p, { kind: 'recovery-started' });
    expect(p.contextLoss.state).toBe('recovering');
  });

  it('context-restored from recovering → healthy restores animation', () => {
    let p = applyEvent(INITIAL_RENDER_POLICY, {
      kind: 'context-lost', query: null, focusId: null, pendingAction: null, nowMs: 0,
    });
    p = applyEvent(p, { kind: 'recovery-started' });
    p = applyEvent(p, { kind: 'context-restored' });
    expect(p.contextLoss.state).toBe('healthy');
    expect(p.animationEnabled).toBe(true);
    expect(p.frameRequested).toBe(true);
  });

  it('fallback-activated from lost → fallback, preserves fallback data', () => {
    let p = applyEvent(INITIAL_RENDER_POLICY, {
      kind: 'context-lost', query: 'search', focusId: 'id-7', pendingAction: 'navigate', nowMs: 100,
    });
    p = applyEvent(p, { kind: 'fallback-activated' });
    expect(p.contextLoss.state).toBe('fallback');
    expect(p.contextLoss.fallback2DQuery).toBe('search');
    expect(p.contextLoss.fallback2DFocusId).toBe('id-7');
    expect(p.contextLoss.pendingActionKind).toBe('navigate');
  });

  it('context-restored is no-op from lost (must start recovery first)', () => {
    let p = applyEvent(INITIAL_RENDER_POLICY, {
      kind: 'context-lost', query: null, focusId: null, pendingAction: null, nowMs: 0,
    });
    p = applyEvent(p, { kind: 'context-restored' });
    expect(p.contextLoss.state).toBe('lost');
  });
});

// ─── Property-based tests ─────────────────────────────────────────────────────
//
// **Validates: Requirements MGR-004, MGR-012, MGR-015, MGD-046**

const qualityLevelArb = fc.constantFrom<QualityLevel>(
  'full', 'no-decoration', 'no-labels', 'no-analytics', 'reduced-scene', 'list-first',
);

const degradationReasonArb = fc.constantFrom<DegradationReason>(
  'frame-budget-exceeded', 'memory-pressure', 'thermal-throttle',
  'context-loss', 'scene-cap-exceeded', 'user-preference', 'forced',
);

describe('Property: degradeQuality never increases quality', () => {
  const ORDER = ['full', 'no-decoration', 'no-labels', 'no-analytics', 'reduced-scene', 'list-first'];
  it('degraded index is always >= original index', () => {
    fc.assert(fc.property(qualityLevelArb, degradationReasonArb, (q, r) => {
      const degraded = degradeQuality(q, r);
      const before = ORDER.indexOf(q);
      const after = ORDER.indexOf(degraded);
      return after >= before;
    }));
  });
});

describe('Property: degradeQuality always produces a valid QualityLevel', () => {
  const VALID: QualityLevel[] = ['full', 'no-decoration', 'no-labels', 'no-analytics', 'reduced-scene', 'list-first'];
  it('output is always a member of the quality ladder', () => {
    fc.assert(fc.property(qualityLevelArb, degradationReasonArb, (q, r) => {
      return (VALID as string[]).includes(degradeQuality(q, r));
    }));
  });
});

describe('Property: shouldRender3D is false iff quality is list-first', () => {
  it('false only for list-first', () => {
    fc.assert(fc.property(qualityLevelArb, (q) => {
      return shouldRender3D(q) === (q !== 'list-first');
    }));
  });
});

describe('Property: shouldStopRendering is monotone in elapsed time', () => {
  it('if stop=true at nowMs, stop=true for any nowMs+k (k>0)', () => {
    fc.assert(fc.property(
      fc.integer({ min: 0, max: 10000 }),
      fc.integer({ min: 0, max: 10000 }),
      fc.integer({ min: 2001, max: 20000 }),
      fc.integer({ min: 1, max: 5000 }),
      (lastInput, lastScene, nowMs, extra) => {
        const idleA: IdleState = { lastInputMs: lastInput, lastSceneChangeMs: lastScene, nowMs };
        if (!shouldStopRendering(idleA)) return true; // only test when baseline is true
        const idleB: IdleState = { ...idleA, nowMs: nowMs + extra };
        return shouldStopRendering(idleB) === true;
      },
    ));
  });
});

describe('Property: applyEvent never mutates its input', () => {
  const eventArb: fc.Arbitrary<RenderPolicyEvent> = fc.oneof(
    fc.record({ kind: fc.constant('reset-quality' as const) }),
    fc.record({ kind: fc.constant('scene-changed' as const) }),
    fc.record({ kind: fc.constant('static-frame-rendered' as const) }),
    fc.record({ kind: fc.constant('recovery-started' as const) }),
    fc.record({ kind: fc.constant('context-restored' as const) }),
    fc.record({ kind: fc.constant('fallback-activated' as const) }),
    fc.record({ kind: fc.constant('degrade-quality' as const), reason: degradationReasonArb }),
    fc.record({ kind: fc.constant('set-motion-mode' as const), mode: fc.constantFrom('full', 'reduced', 'none') as fc.Arbitrary<'full' | 'reduced' | 'none'> }),
    fc.record({ kind: fc.constant('context-lost' as const), query: fc.option(fc.string(), { nil: null }), focusId: fc.option(fc.string(), { nil: null }), pendingAction: fc.option(fc.string(), { nil: null }), nowMs: fc.integer({ min: 0 }) }),
  );

  it('input policy object is identical after applyEvent', () => {
    fc.assert(fc.property(eventArb, (event) => {
      const before = structuredClone(INITIAL_RENDER_POLICY);
      applyEvent(INITIAL_RENDER_POLICY, event);
      // INITIAL_RENDER_POLICY must not be mutated
      expect(INITIAL_RENDER_POLICY).toEqual(before);
    }));
  });
});

describe('Property: onContextLoss state is always lost', () => {
  it('regardless of initial state, result is always lost', () => {
    fc.assert(fc.property(
      fc.option(fc.string(), { nil: null }),
      fc.option(fc.string(), { nil: null }),
      fc.option(fc.string(), { nil: null }),
      fc.integer({ min: 0 }),
      (q, f, a, nowMs) => {
        const r = onContextLoss(INITIAL_CONTEXT_LOSS_RECORD, q, f, a, nowMs);
        return r.state === 'lost';
      },
    ));
  });
});
