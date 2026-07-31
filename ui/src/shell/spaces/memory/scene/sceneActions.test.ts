/**
 * sceneActions.test.ts — Vitest unit tests for sceneActions.ts
 *
 * Pure TS, no DOM, no side effects.
 * Tests all capability authorization, action building, and scene-level action behavior.
 */

import { describe, it, expect } from 'vitest';
import {
  ALL_ACTION_KINDS,
  DEFAULT_CAPABILITIES,
  isActionAuthorized,
  buildAuthorizedActions,
  buildSceneLevelActions,
  type SceneCapabilities,
} from './sceneActions';
import type { SceneActionKind } from './semanticScene';

// ─── ALL_ACTION_KINDS ─────────────────────────────────────────────────────────

describe('ALL_ACTION_KINDS', () => {
  it('contains all 14 action kinds', () => {
    const expected: SceneActionKind[] = [
      'select', 'expand', 'inspect', 'path',
      'correct', 'merge', 'split', 'relate',
      'forget', 'restore', 'delete',
      'fit', 'back', 'forward',
    ];
    expect(ALL_ACTION_KINDS).toHaveLength(14);
    for (const kind of expected) {
      expect(ALL_ACTION_KINDS).toContain(kind);
    }
  });
});

// ─── isActionAuthorized ───────────────────────────────────────────────────────

describe('isActionAuthorized', () => {
  const allFalse: SceneCapabilities = {
    canCorrect: false,
    canMerge: false,
    canSplit: false,
    canRelate: false,
    canForget: false,
    canRestore: false,
    canDelete: false,
    canNavigatePath: false,
    canFitView: false,
    canNavigateHistory: false,
  };

  it('select is always true regardless of capabilities', () => {
    expect(isActionAuthorized('select', allFalse)).toBe(true);
    expect(isActionAuthorized('select', DEFAULT_CAPABILITIES)).toBe(true);
  });

  it('expand is always true regardless of capabilities', () => {
    expect(isActionAuthorized('expand', allFalse)).toBe(true);
    expect(isActionAuthorized('expand', DEFAULT_CAPABILITIES)).toBe(true);
  });

  it('inspect is always true regardless of capabilities', () => {
    expect(isActionAuthorized('inspect', allFalse)).toBe(true);
    expect(isActionAuthorized('inspect', DEFAULT_CAPABILITIES)).toBe(true);
  });

  it('path requires canNavigatePath', () => {
    expect(isActionAuthorized('path', { ...allFalse, canNavigatePath: true })).toBe(true);
    expect(isActionAuthorized('path', allFalse)).toBe(false);
  });

  it('correct requires canCorrect', () => {
    expect(isActionAuthorized('correct', { ...allFalse, canCorrect: true })).toBe(true);
    expect(isActionAuthorized('correct', allFalse)).toBe(false);
  });

  it('merge requires canMerge', () => {
    expect(isActionAuthorized('merge', { ...allFalse, canMerge: true })).toBe(true);
    expect(isActionAuthorized('merge', allFalse)).toBe(false);
  });

  it('split requires canSplit', () => {
    expect(isActionAuthorized('split', { ...allFalse, canSplit: true })).toBe(true);
    expect(isActionAuthorized('split', allFalse)).toBe(false);
  });

  it('relate requires canRelate', () => {
    expect(isActionAuthorized('relate', { ...allFalse, canRelate: true })).toBe(true);
    expect(isActionAuthorized('relate', allFalse)).toBe(false);
  });

  it('forget requires canForget', () => {
    expect(isActionAuthorized('forget', { ...allFalse, canForget: true })).toBe(true);
    expect(isActionAuthorized('forget', allFalse)).toBe(false);
  });

  it('restore requires canRestore', () => {
    expect(isActionAuthorized('restore', { ...allFalse, canRestore: true })).toBe(true);
    expect(isActionAuthorized('restore', allFalse)).toBe(false);
  });

  it('delete requires canDelete', () => {
    expect(isActionAuthorized('delete', { ...allFalse, canDelete: true })).toBe(true);
    expect(isActionAuthorized('delete', allFalse)).toBe(false);
  });

  it('fit requires canFitView', () => {
    expect(isActionAuthorized('fit', { ...allFalse, canFitView: true })).toBe(true);
    expect(isActionAuthorized('fit', allFalse)).toBe(false);
  });

  it('back requires canNavigateHistory', () => {
    expect(isActionAuthorized('back', { ...allFalse, canNavigateHistory: true })).toBe(true);
    expect(isActionAuthorized('back', allFalse)).toBe(false);
  });

  it('forward requires canNavigateHistory', () => {
    expect(isActionAuthorized('forward', { ...allFalse, canNavigateHistory: true })).toBe(true);
    expect(isActionAuthorized('forward', allFalse)).toBe(false);
  });
});

// ─── DEFAULT_CAPABILITIES ─────────────────────────────────────────────────────

describe('DEFAULT_CAPABILITIES', () => {
  it('select passes with default capabilities', () => {
    expect(isActionAuthorized('select', DEFAULT_CAPABILITIES)).toBe(true);
  });

  it('expand passes with default capabilities', () => {
    expect(isActionAuthorized('expand', DEFAULT_CAPABILITIES)).toBe(true);
  });

  it('inspect passes with default capabilities', () => {
    expect(isActionAuthorized('inspect', DEFAULT_CAPABILITIES)).toBe(true);
  });

  it('all item-level actions are authorized with default capabilities', () => {
    const itemKinds: SceneActionKind[] = [
      'select', 'expand', 'inspect', 'path',
      'correct', 'merge', 'split', 'relate',
      'forget', 'restore', 'delete',
    ];
    for (const kind of itemKinds) {
      expect(isActionAuthorized(kind, DEFAULT_CAPABILITIES)).toBe(true);
    }
  });
});

// ─── buildAuthorizedActions ───────────────────────────────────────────────────

describe('buildAuthorizedActions', () => {
  const ITEM_ID = 'item-abc-123';

  it('includes select, expand, inspect, and other enabled actions with all caps true', () => {
    const actions = buildAuthorizedActions(ITEM_ID, DEFAULT_CAPABILITIES);
    const kinds = actions.map((a) => a.kind);
    expect(kinds).toContain('select');
    expect(kinds).toContain('expand');
    expect(kinds).toContain('inspect');
    expect(kinds).toContain('correct');
    expect(kinds).toContain('merge');
    expect(kinds).toContain('split');
    expect(kinds).toContain('relate');
    expect(kinds).toContain('forget');
    expect(kinds).toContain('restore');
    expect(kinds).toContain('delete');
    expect(kinds).toContain('path');
  });

  it('excludes correct when canCorrect=false', () => {
    const caps: SceneCapabilities = { ...DEFAULT_CAPABILITIES, canCorrect: false };
    const actions = buildAuthorizedActions(ITEM_ID, caps);
    const kinds = actions.map((a) => a.kind);
    expect(kinds).not.toContain('correct');
    // Other actions unaffected
    expect(kinds).toContain('select');
    expect(kinds).toContain('merge');
  });

  it('excludes delete when canDelete=false but includes others', () => {
    const caps: SceneCapabilities = { ...DEFAULT_CAPABILITIES, canDelete: false };
    const actions = buildAuthorizedActions(ITEM_ID, caps);
    const kinds = actions.map((a) => a.kind);
    expect(kinds).not.toContain('delete');
    expect(kinds).toContain('select');
    expect(kinds).toContain('expand');
    expect(kinds).toContain('inspect');
    expect(kinds).toContain('forget');
    expect(kinds).toContain('merge');
  });

  it('does NOT include fit, back, or forward in item-level actions', () => {
    const actions = buildAuthorizedActions(ITEM_ID, DEFAULT_CAPABILITIES);
    const kinds = actions.map((a) => a.kind);
    expect(kinds).not.toContain('fit');
    expect(kinds).not.toContain('back');
    expect(kinds).not.toContain('forward');
  });

  it('correct has isDangerous=true and requiresPreview=true', () => {
    const actions = buildAuthorizedActions(ITEM_ID, DEFAULT_CAPABILITIES);
    const correct = actions.find((a) => a.kind === 'correct');
    expect(correct).toBeDefined();
    expect(correct!.isDangerous).toBe(true);
    expect(correct!.requiresPreview).toBe(true);
  });

  it('merge has isDangerous=true and requiresPreview=true', () => {
    const actions = buildAuthorizedActions(ITEM_ID, DEFAULT_CAPABILITIES);
    const merge = actions.find((a) => a.kind === 'merge');
    expect(merge).toBeDefined();
    expect(merge!.isDangerous).toBe(true);
    expect(merge!.requiresPreview).toBe(true);
  });

  it('split has isDangerous=true and requiresPreview=true', () => {
    const actions = buildAuthorizedActions(ITEM_ID, DEFAULT_CAPABILITIES);
    const split = actions.find((a) => a.kind === 'split');
    expect(split).toBeDefined();
    expect(split!.isDangerous).toBe(true);
    expect(split!.requiresPreview).toBe(true);
  });

  it('forget has isDangerous=true and requiresPreview=true', () => {
    const actions = buildAuthorizedActions(ITEM_ID, DEFAULT_CAPABILITIES);
    const forget = actions.find((a) => a.kind === 'forget');
    expect(forget).toBeDefined();
    expect(forget!.isDangerous).toBe(true);
    expect(forget!.requiresPreview).toBe(true);
  });

  it('delete has isDangerous=true and requiresPreview=true', () => {
    const actions = buildAuthorizedActions(ITEM_ID, DEFAULT_CAPABILITIES);
    const del = actions.find((a) => a.kind === 'delete');
    expect(del).toBeDefined();
    expect(del!.isDangerous).toBe(true);
    expect(del!.requiresPreview).toBe(true);
  });

  it('select has isDangerous=false', () => {
    const actions = buildAuthorizedActions(ITEM_ID, DEFAULT_CAPABILITIES);
    const select = actions.find((a) => a.kind === 'select');
    expect(select).toBeDefined();
    expect(select!.isDangerous).toBe(false);
  });

  it('expand has isDangerous=false', () => {
    const actions = buildAuthorizedActions(ITEM_ID, DEFAULT_CAPABILITIES);
    const expand = actions.find((a) => a.kind === 'expand');
    expect(expand).toBeDefined();
    expect(expand!.isDangerous).toBe(false);
  });

  it('inspect has isDangerous=false', () => {
    const actions = buildAuthorizedActions(ITEM_ID, DEFAULT_CAPABILITIES);
    const inspect = actions.find((a) => a.kind === 'inspect');
    expect(inspect).toBeDefined();
    expect(inspect!.isDangerous).toBe(false);
  });

  it('excludeKinds removes specific kinds from the result', () => {
    const actions = buildAuthorizedActions(ITEM_ID, DEFAULT_CAPABILITIES, {
      excludeKinds: ['merge', 'split'],
    });
    const kinds = actions.map((a) => a.kind);
    expect(kinds).not.toContain('merge');
    expect(kinds).not.toContain('split');
    // Other kinds still present
    expect(kinds).toContain('select');
    expect(kinds).toContain('correct');
  });

  it('all returned actions have targetItemId matching the provided itemId', () => {
    const actions = buildAuthorizedActions(ITEM_ID, DEFAULT_CAPABILITIES);
    for (const action of actions) {
      expect(action.targetItemId).toBe(ITEM_ID);
    }
  });

  it('all returned actions have isEnabled=true', () => {
    const actions = buildAuthorizedActions(ITEM_ID, DEFAULT_CAPABILITIES);
    for (const action of actions) {
      expect(action.isEnabled).toBe(true);
    }
  });

  it('returns empty array when all capability flags are false (only always-authorized remain)', () => {
    const noOp: SceneCapabilities = {
      canCorrect: false,
      canMerge: false,
      canSplit: false,
      canRelate: false,
      canForget: false,
      canRestore: false,
      canDelete: false,
      canNavigatePath: false,
      canFitView: false,
      canNavigateHistory: false,
    };
    const actions = buildAuthorizedActions(ITEM_ID, noOp);
    const kinds = actions.map((a) => a.kind);
    // select/expand/inspect are always authorized
    expect(kinds).toContain('select');
    expect(kinds).toContain('expand');
    expect(kinds).toContain('inspect');
    // All capability-gated ones are absent
    expect(kinds).not.toContain('path');
    expect(kinds).not.toContain('correct');
    expect(kinds).not.toContain('merge');
    expect(kinds).not.toContain('split');
    expect(kinds).not.toContain('relate');
    expect(kinds).not.toContain('forget');
    expect(kinds).not.toContain('restore');
    expect(kinds).not.toContain('delete');
  });

  it('action order follows ALL_ACTION_KINDS (minus scene-level) order', () => {
    const actions = buildAuthorizedActions(ITEM_ID, DEFAULT_CAPABILITIES);
    const itemKindsInOrder: SceneActionKind[] = [
      'select', 'expand', 'inspect', 'path',
      'correct', 'merge', 'split', 'relate',
      'forget', 'restore', 'delete',
    ];
    const returnedKinds = actions.map((a) => a.kind);
    expect(returnedKinds).toEqual(itemKindsInOrder);
  });
});

// ─── buildSceneLevelActions ───────────────────────────────────────────────────

describe('buildSceneLevelActions', () => {
  it('returns fit, back, and forward when all relevant caps are true', () => {
    const actions = buildSceneLevelActions(DEFAULT_CAPABILITIES);
    const kinds = actions.map((a) => a.kind);
    expect(kinds).toContain('fit');
    expect(kinds).toContain('back');
    expect(kinds).toContain('forward');
  });

  it('excludes back and forward when canNavigateHistory=false', () => {
    const caps: SceneCapabilities = { ...DEFAULT_CAPABILITIES, canNavigateHistory: false };
    const actions = buildSceneLevelActions(caps);
    const kinds = actions.map((a) => a.kind);
    expect(kinds).not.toContain('back');
    expect(kinds).not.toContain('forward');
    expect(kinds).toContain('fit');
  });

  it('excludes fit when canFitView=false', () => {
    const caps: SceneCapabilities = { ...DEFAULT_CAPABILITIES, canFitView: false };
    const actions = buildSceneLevelActions(caps);
    const kinds = actions.map((a) => a.kind);
    expect(kinds).not.toContain('fit');
    expect(kinds).toContain('back');
    expect(kinds).toContain('forward');
  });

  it('returns empty array when both canFitView and canNavigateHistory are false', () => {
    const caps: SceneCapabilities = {
      ...DEFAULT_CAPABILITIES,
      canFitView: false,
      canNavigateHistory: false,
    };
    const actions = buildSceneLevelActions(caps);
    expect(actions).toHaveLength(0);
  });

  it('scene-level actions do NOT have a targetItemId property', () => {
    const actions = buildSceneLevelActions(DEFAULT_CAPABILITIES);
    for (const action of actions) {
      expect(action).not.toHaveProperty('targetItemId');
    }
  });

  it('scene-level actions all have isEnabled=true', () => {
    const actions = buildSceneLevelActions(DEFAULT_CAPABILITIES);
    for (const action of actions) {
      expect(action.isEnabled).toBe(true);
    }
  });

  it('scene-level actions all have isDangerous=false', () => {
    const actions = buildSceneLevelActions(DEFAULT_CAPABILITIES);
    for (const action of actions) {
      expect(action.isDangerous).toBe(false);
    }
  });

  it('scene-level actions do NOT include item-level kinds', () => {
    const actions = buildSceneLevelActions(DEFAULT_CAPABILITIES);
    const kinds = actions.map((a) => a.kind);
    const itemKinds: SceneActionKind[] = [
      'select', 'expand', 'inspect', 'path',
      'correct', 'merge', 'split', 'relate',
      'forget', 'restore', 'delete',
    ];
    for (const kind of itemKinds) {
      expect(kinds).not.toContain(kind);
    }
  });
});
