/**
 * sceneParityCheck.test.ts — Vitest unit tests for scene parity/purity helpers.
 *
 * Pure TypeScript — no DOM, no JSX, no side effects.
 * Task: F4.6.6
 */

import { describe, it, expect } from 'vitest';
import type {
  SemanticScene,
  SemanticSceneItem,
  SemanticSceneAction,
  SemanticVisualToken,
} from './semanticScene';
import type { SceneItemKind } from './semanticScene';
import {
  checkItemIdUniqueness,
  checkActionTargetValidity,
  checkNavigationContainerPurity,
  checkEdgeEndpointCompleteness,
  checkTokenItemValidity,
  checkNoUnknownKinds,
  checkActionUniqueness,
  runAllParityChecks,
} from './sceneParityCheck';

// ─── Fixture helpers ──────────────────────────────────────────────────────────

function makeItem(
  id: string,
  kind: SceneItemKind = 'entity',
  overrides: Partial<SemanticSceneItem> = {},
): SemanticSceneItem {
  return {
    id,
    kind,
    authorityClass: 'personal',
    label: `Label for ${id}`,
    truthState: 'asserted',
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

function makeEdgeItem(
  id: string,
  sourceEndpointId: string | null,
  targetEndpointId: string | null,
): SemanticSceneItem {
  return makeItem(id, 'relation', {
    direction: 'outgoing',
    sourceEndpointId,
    targetEndpointId,
  });
}

function makeAction(
  targetItemId: string,
  kind: SemanticSceneAction['kind'] = 'select',
): SemanticSceneAction {
  return {
    targetItemId,
    kind,
    label: `${kind} action`,
    isEnabled: true,
    isDangerous: false,
    requiresPreview: false,
  };
}

function makeToken(itemId: string): SemanticVisualToken {
  return {
    itemId,
    shape: 'circle',
    colorToken: '--color-entity',
    iconId: null,
    displayLabel: itemId,
    showLabel: true,
  };
}

function makeScene(
  items: SemanticSceneItem[] = [],
  actions: SemanticSceneAction[] = [],
  tokens: SemanticVisualToken[] = [],
): SemanticScene {
  return {
    sceneHash: 'test-hash',
    graphRevision: 1,
    items,
    actions,
    tokens,
    layoutHint: {
      seed: 0,
      strategy: 'ego-radial-rings',
      primaryItemId: null,
      maxDepth: null,
    },
    diagnostics: [],
  };
}

// ─── checkItemIdUniqueness ────────────────────────────────────────────────────

describe('checkItemIdUniqueness', () => {
  it('passes for items with unique IDs', () => {
    const scene = makeScene([makeItem('a'), makeItem('b'), makeItem('c')]);
    const result = checkItemIdUniqueness(scene);
    expect(result.passed).toBe(true);
    expect(result.violations).toHaveLength(0);
  });

  it('fails for duplicate item IDs and reports the ID', () => {
    const scene = makeScene([makeItem('a'), makeItem('b'), makeItem('a')]);
    const result = checkItemIdUniqueness(scene);
    expect(result.passed).toBe(false);
    expect(result.violations).toHaveLength(1);
    expect(result.violations[0]).toContain('"a"');
  });
});

// ─── checkActionTargetValidity ────────────────────────────────────────────────

describe('checkActionTargetValidity', () => {
  it('passes when all actions reference valid item IDs', () => {
    const scene = makeScene(
      [makeItem('x'), makeItem('y')],
      [makeAction('x'), makeAction('y')],
    );
    const result = checkActionTargetValidity(scene);
    expect(result.passed).toBe(true);
    expect(result.violations).toHaveLength(0);
  });

  it('fails when an action targets a non-existent item ID', () => {
    const scene = makeScene(
      [makeItem('x')],
      [makeAction('ghost')],
    );
    const result = checkActionTargetValidity(scene);
    expect(result.passed).toBe(false);
    expect(result.violations[0]).toContain('"ghost"');
  });
});

// ─── checkNavigationContainerPurity ──────────────────────────────────────────

describe('checkNavigationContainerPurity', () => {
  it('passes when navigation containers have no actions', () => {
    const nav = makeItem('nav1', 'navigation-container');
    const entity = makeItem('e1');
    const scene = makeScene([nav, entity], [makeAction('e1')]);
    const result = checkNavigationContainerPurity(scene);
    expect(result.passed).toBe(true);
    expect(result.violations).toHaveLength(0);
  });

  it('fails when a navigation container has actions', () => {
    const nav = makeItem('nav1', 'navigation-container');
    const scene = makeScene([nav], [makeAction('nav1', 'expand')]);
    const result = checkNavigationContainerPurity(scene);
    expect(result.passed).toBe(false);
    expect(result.violations[0]).toContain('"nav1"');
    expect(result.violations[0]).toContain('"expand"');
  });
});

// ─── checkEdgeEndpointCompleteness ────────────────────────────────────────────

describe('checkEdgeEndpointCompleteness', () => {
  it('passes when edge endpoints exist in the scene', () => {
    const src = makeItem('src');
    const tgt = makeItem('tgt');
    const edge = makeEdgeItem('edge1', 'src', 'tgt');
    const scene = makeScene([src, tgt, edge]);
    const result = checkEdgeEndpointCompleteness(scene);
    expect(result.passed).toBe(true);
    expect(result.violations).toHaveLength(0);
  });

  it('fails when an edge has null sourceEndpointId', () => {
    const tgt = makeItem('tgt');
    const edge = makeEdgeItem('edge1', null, 'tgt');
    const scene = makeScene([tgt, edge]);
    const result = checkEdgeEndpointCompleteness(scene);
    expect(result.passed).toBe(false);
    expect(result.violations.some((v) => v.includes('null sourceEndpointId'))).toBe(true);
    expect(result.violations.some((v) => v.includes('"edge1"'))).toBe(true);
  });

  it('fails when an edge references a missing endpoint ID', () => {
    const src = makeItem('src');
    const edge = makeEdgeItem('edge1', 'src', 'missing-tgt');
    const scene = makeScene([src, edge]);
    const result = checkEdgeEndpointCompleteness(scene);
    expect(result.passed).toBe(false);
    expect(result.violations.some((v) => v.includes('"missing-tgt"'))).toBe(true);
  });
});

// ─── checkTokenItemValidity ───────────────────────────────────────────────────

describe('checkTokenItemValidity', () => {
  it('passes when all tokens reference valid item IDs', () => {
    const scene = makeScene([makeItem('a')], [], [makeToken('a')]);
    const result = checkTokenItemValidity(scene);
    expect(result.passed).toBe(true);
    expect(result.violations).toHaveLength(0);
  });

  it('fails when a token references a missing item ID', () => {
    const scene = makeScene([makeItem('a')], [], [makeToken('phantom')]);
    const result = checkTokenItemValidity(scene);
    expect(result.passed).toBe(false);
    expect(result.violations[0]).toContain('"phantom"');
  });
});

// ─── checkNoUnknownKinds ──────────────────────────────────────────────────────

describe('checkNoUnknownKinds', () => {
  it('passes for all known kinds', () => {
    const items = [
      makeItem('e', 'entity'),
      makeItem('m', 'memory'),
      makeItem('r', 'relation'),
      makeItem('g', 'goal'),
      makeItem('s', 'source'),
      makeItem('ep', 'episode'),
      makeItem('su', 'summary'),
      makeItem('sk', 'skill'),
      makeItem('ru', 'rule'),
      makeItem('nc', 'navigation-container'),
    ];
    const scene = makeScene(items);
    const result = checkNoUnknownKinds(scene);
    expect(result.passed).toBe(true);
    expect(result.violations).toHaveLength(0);
  });

  it('fails when kind is an empty string (cast to simulate unknown)', () => {
    // Simulate a malformed item arriving over the wire with an unknown kind.
    const item = makeItem('bad') as unknown as SemanticSceneItem;
    (item as unknown as Record<string, unknown>)['kind'] = '';
    const scene = makeScene([item]);
    const result = checkNoUnknownKinds(scene);
    expect(result.passed).toBe(false);
    expect(result.violations[0]).toContain('(empty string)');
  });

  it('fails when kind is an unknown value (cast to simulate unknown)', () => {
    const item = makeItem('bad2') as unknown as SemanticSceneItem;
    (item as unknown as Record<string, unknown>)['kind'] = 'alien-kind';
    const scene = makeScene([item]);
    const result = checkNoUnknownKinds(scene);
    expect(result.passed).toBe(false);
    expect(result.violations[0]).toContain('"alien-kind"');
    expect(result.violations[0]).toContain('"bad2"');
  });
});

// ─── checkActionUniqueness ────────────────────────────────────────────────────

describe('checkActionUniqueness', () => {
  it('passes for unique (targetItemId, kind) pairs', () => {
    const scene = makeScene(
      [makeItem('a'), makeItem('b')],
      [makeAction('a', 'select'), makeAction('a', 'expand'), makeAction('b', 'select')],
    );
    const result = checkActionUniqueness(scene);
    expect(result.passed).toBe(true);
    expect(result.violations).toHaveLength(0);
  });

  it('fails for duplicate (targetItemId, kind) pairs', () => {
    const scene = makeScene(
      [makeItem('a')],
      [makeAction('a', 'select'), makeAction('a', 'select')],
    );
    const result = checkActionUniqueness(scene);
    expect(result.passed).toBe(false);
    expect(result.violations[0]).toContain('"a"');
    expect(result.violations[0]).toContain('"select"');
  });
});

// ─── runAllParityChecks ───────────────────────────────────────────────────────

describe('runAllParityChecks', () => {
  it('passes for a fully valid scene', () => {
    const src = makeItem('src');
    const tgt = makeItem('tgt');
    const edge = makeEdgeItem('edge1', 'src', 'tgt');
    const nav = makeItem('nav1', 'navigation-container');
    const scene = makeScene(
      [src, tgt, edge, nav],
      [makeAction('src', 'select'), makeAction('tgt', 'inspect')],
      [makeToken('src'), makeToken('tgt')],
    );
    const result = runAllParityChecks(scene);
    expect(result.passed).toBe(true);
    expect(result.violations).toHaveLength(0);
  });

  it('fails and aggregates violations when any check fails', () => {
    // Duplicate item ID (fails checkItemIdUniqueness)
    const scene = makeScene([makeItem('dup'), makeItem('dup')]);
    const result = runAllParityChecks(scene);
    expect(result.passed).toBe(false);
    expect(result.violations.length).toBeGreaterThan(0);
    // Violation should be prefixed with the check name
    expect(result.violations.some((v) => v.startsWith('[ItemIdUniqueness]'))).toBe(true);
  });
});
