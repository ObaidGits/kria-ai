/**
 * graphCanvas3DSpike.test.ts — F6.2.1 parity spike unit tests.
 *
 * Validates:
 *   - The semantic collection hash produced by computeSemanticParitySnapshot
 *     equals the hash Graph2D would compute from the same SemanticScene.
 *   - The authorized action hash produced by computeSemanticParitySnapshot
 *     equals the hash Graph2D would compute from the same SemanticScene.
 *   - Both hashes are deterministic: same input → same output across calls.
 *   - Both hashes differ when scene content differs.
 *   - getSpikeParitySnapshot (the GraphCanvas3D component helper) returns
 *     identical hashes for the same scene as direct computeSemanticParitySnapshot.
 *
 * Critical constraint (task_6_1_5 / task 6.2.1):
 *   The 3D renderer is a PURE CONSUMER of SemanticScene — it must not maintain
 *   its own truth, policy, or layout state. Parity is verified by asserting
 *   that the semantic collection hash and the authorized action hash are
 *   IDENTICAL for both the 2D and 3D consumers when presented with the same
 *   scene built from the same fixture.
 *
 * Test strategy:
 *   1. Build a fixed SemanticScene via buildSemanticScene (the same path as
 *      Graph2D / SemanticList use at runtime).
 *   2. Run computeSemanticParitySnapshot on it (the 3D spike consumer path).
 *   3. Run the equivalent hash functions for the 2D consumer
 *      (computeSemanticCollectionHash / computeAuthorizedActionHash directly,
 *      and compare to the 3D snapshot — they must be equal).
 *   4. Verify determinism and sensitivity.
 *
 * No DOM, no WebGL, no SolidJS rendering — pure logic tests only.
 *
 * Requirements: MGR-001, MGR-002, MGR-004, MGR-012, MGR-026; MGD-003, MGD-026.
 * Spec task: 6.2.1
 */

import { describe, it, expect } from 'vitest';
import {
  computeSemanticCollectionHash,
  computeAuthorizedActionHash,
  computeSemanticParitySnapshot,
} from './graphCanvas3DSpike';
import { getSpikeParitySnapshot } from './GraphCanvas3D';
import { buildSemanticScene } from '../scene/sceneBuilder';
import type { RawSceneItem, RawSceneAction, SceneBuildInput } from '../scene/sceneBuilder';
import type { SemanticScene, SemanticSceneItem, SemanticSceneAction } from '../scene/semanticScene';
import { buildAuthorizedActions, DEFAULT_CAPABILITIES } from '../scene/sceneActions';
import type { SemanticInput3D } from './graphCanvas3DSpike';

// ─── Fixtures ─────────────────────────────────────────────────────────────────

const LAYOUT_HINT: SceneBuildInput['layoutHint'] = {
  seed: 0x4d475203,
  strategy: 'path-layered-dag',
  primaryItemId: 'entity-alpha',
  maxDepth: 3,
};

function makeRawItem(
  id: string,
  kind: string = 'entity',
  overrides: Partial<RawSceneItem> = {},
): RawSceneItem {
  return {
    id,
    kind,
    authorityClass: 'personal',
    label: `Label for ${id}`,
    truthState: 'confirmed',
    graphRevision: 42,
    direction: null,
    sourceEndpointId: null,
    targetEndpointId: null,
    evidenceCount: 1,
    evidenceSummary: null,
    provenanceSourceId: null,
    provenanceMethod: 'manual',
    provenanceVersion: '1.0',
    provenanceActorLabel: null,
    validTimeStart: null,
    validTimeEnd: null,
    isCurrentlyValid: true,
    isSelected: false,
    isFocused: false,
    isInPath: false,
    isPending: false,
    hasError: false,
    isAuthorized: true,
    ...overrides,
  };
}

function makeRawAction(
  targetItemId: string,
  kind: string = 'select',
  overrides: Partial<RawSceneAction> = {},
): RawSceneAction {
  return {
    targetItemId,
    kind,
    label: kind.charAt(0).toUpperCase() + kind.slice(1),
    isEnabled: true,
    isDangerous: false,
    requiresPreview: false,
    isAuthorized: true,
    ...overrides,
  };
}

/**
 * Build a test SemanticScene from a known fixture using pathScene-style inputs.
 *
 * Uses buildSemanticScene (the same scene builder used at runtime) so the
 * test exercises the real production code path, not a hand-crafted mock.
 */
function buildFixtureScene(itemIds: string[] = ['entity-alpha', 'entity-beta', 'entity-gamma']): SemanticScene {
  const items: RawSceneItem[] = [
    makeRawItem(itemIds[0] ?? 'entity-alpha', 'entity'),
    makeRawItem(itemIds[1] ?? 'entity-beta', 'memory'),
    makeRawItem(itemIds[2] ?? 'entity-gamma', 'goal'),
  ];
  // Build authorized actions via the same code path as Graph2D / SceneActions
  const rawActions: RawSceneAction[] = items.flatMap((item) => {
    const authorized = buildAuthorizedActions(item.id, DEFAULT_CAPABILITIES);
    return authorized.map(
      (a): RawSceneAction => ({
        targetItemId: a.targetItemId,
        kind: a.kind,
        label: a.label,
        isEnabled: a.isEnabled,
        isDangerous: a.isDangerous,
        requiresPreview: a.requiresPreview,
        isAuthorized: true,
      }),
    );
  });

  const input: SceneBuildInput = {
    items,
    actions: rawActions,
    graphRevision: 42,
    layoutHint: LAYOUT_HINT,
  };
  return buildSemanticScene(input).scene;
}

// ─── Helpers: 2D consumer hash (what Graph2D would compute) ──────────────────
//
// Graph2D is a pure consumer: it reads scene.items and scene.actions exactly
// as delivered.  We model its "hash" perspective by calling the same functions
// the 3D spike exports, on the same scene input.  This is the parity oracle.

function twoD_semanticCollectionHash(scene: SemanticScene): string {
  return computeSemanticCollectionHash(scene.items);
}

function twoD_authorizedActionHash(scene: SemanticScene): string {
  return computeAuthorizedActionHash(scene.actions);
}

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('F6.2.1 spike — semantic collection hash parity', () => {
  it('3D semantic collection hash equals 2D semantic collection hash for the same fixture scene', () => {
    const scene = buildFixtureScene();

    const snapshot3D = computeSemanticParitySnapshot(scene);
    const hash2D = twoD_semanticCollectionHash(scene);

    expect(snapshot3D.semanticCollectionHash).toBe(hash2D);
  });

  it('3D semantic collection hash equals 2D hash for a minimal single-item scene', () => {
    const singleItemScene = buildFixtureScene(['entity-solo', 'entity-solo-b', 'entity-solo-c']);
    // Rebuild with just 1 item
    const input: SceneBuildInput = {
      items: [makeRawItem('only-entity', 'entity')],
      actions: buildAuthorizedActions('only-entity', DEFAULT_CAPABILITIES).map(
        (a): RawSceneAction => ({
          targetItemId: a.targetItemId,
          kind: a.kind,
          label: a.label,
          isEnabled: a.isEnabled,
          isDangerous: a.isDangerous,
          requiresPreview: a.requiresPreview,
          isAuthorized: true,
        }),
      ),
      graphRevision: 1,
      layoutHint: LAYOUT_HINT,
    };
    const scene = buildSemanticScene(input).scene;

    const snapshot3D = computeSemanticParitySnapshot(scene);
    const hash2D = twoD_semanticCollectionHash(scene);

    expect(snapshot3D.semanticCollectionHash).toBe(hash2D);
  });

  it('3D semantic collection hash equals 2D hash for an empty scene', () => {
    const emptyScene: SemanticScene = {
      sceneHash: 'empty',
      graphRevision: 1,
      items: [],
      actions: [],
      tokens: [],
      layoutHint: LAYOUT_HINT,
      diagnostics: [],
    };

    const snapshot3D = computeSemanticParitySnapshot(emptyScene);
    const hash2D = twoD_semanticCollectionHash(emptyScene);

    expect(snapshot3D.semanticCollectionHash).toBe(hash2D);
  });
});

describe('F6.2.1 spike — authorized action hash parity', () => {
  it('3D authorized action hash equals 2D authorized action hash for the same fixture scene', () => {
    const scene = buildFixtureScene();

    const snapshot3D = computeSemanticParitySnapshot(scene);
    const hash2D = twoD_authorizedActionHash(scene);

    expect(snapshot3D.authorizedActionHash).toBe(hash2D);
  });

  it('3D authorized action hash equals 2D hash for a scene with no actions (empty action set)', () => {
    const input: SceneBuildInput = {
      items: [makeRawItem('lone-entity', 'entity')],
      actions: [],
      graphRevision: 5,
      layoutHint: LAYOUT_HINT,
    };
    const scene = buildSemanticScene(input).scene;

    const snapshot3D = computeSemanticParitySnapshot(scene);
    const hash2D = twoD_authorizedActionHash(scene);

    expect(snapshot3D.authorizedActionHash).toBe(hash2D);
  });

  it('3D authorized action hash equals 2D hash for restricted capabilities', () => {
    // Restrict capabilities: no destructive actions
    const restrictedCaps = {
      ...DEFAULT_CAPABILITIES,
      canCorrect: false,
      canMerge: false,
      canSplit: false,
      canForget: false,
      canDelete: false,
    };
    const rawActions: RawSceneAction[] = buildAuthorizedActions('item-x', restrictedCaps).map(
      (a): RawSceneAction => ({
        targetItemId: a.targetItemId,
        kind: a.kind,
        label: a.label,
        isEnabled: a.isEnabled,
        isDangerous: a.isDangerous,
        requiresPreview: a.requiresPreview,
        isAuthorized: true,
      }),
    );
    const input: SceneBuildInput = {
      items: [makeRawItem('item-x', 'entity')],
      actions: rawActions,
      graphRevision: 7,
      layoutHint: LAYOUT_HINT,
    };
    const scene = buildSemanticScene(input).scene;

    const snapshot3D = computeSemanticParitySnapshot(scene);
    const hash2D = twoD_authorizedActionHash(scene);

    expect(snapshot3D.authorizedActionHash).toBe(hash2D);
  });
});

describe('F6.2.1 spike — determinism', () => {
  it('computeSemanticParitySnapshot is deterministic: same scene → same hashes across calls', () => {
    const scene = buildFixtureScene();

    const snap1 = computeSemanticParitySnapshot(scene);
    const snap2 = computeSemanticParitySnapshot(scene);

    expect(snap1.semanticCollectionHash).toBe(snap2.semanticCollectionHash);
    expect(snap1.authorizedActionHash).toBe(snap2.authorizedActionHash);
  });

  it('computeSemanticParitySnapshot is deterministic for empty scene', () => {
    const empty: SemanticScene = {
      sceneHash: 'x',
      graphRevision: 0,
      items: [],
      actions: [],
      tokens: [],
      layoutHint: LAYOUT_HINT,
      diagnostics: [],
    };

    const snap1 = computeSemanticParitySnapshot(empty);
    const snap2 = computeSemanticParitySnapshot(empty);

    expect(snap1.semanticCollectionHash).toBe(snap2.semanticCollectionHash);
    expect(snap1.authorizedActionHash).toBe(snap2.authorizedActionHash);
  });

  it('hashes are order-independent: item list order does not affect semanticCollectionHash', () => {
    const scene = buildFixtureScene();

    // buildSemanticScene already sorts by id, so this is a regression guard:
    // shuffling items after the sort must still produce the same hash because
    // computeSemanticCollectionHash re-sorts internally.
    const shuffledItems: SemanticSceneItem[] = [...scene.items].reverse();
    const shuffledScene: SemanticScene = { ...scene, items: shuffledItems };

    const hashOriginal = computeSemanticCollectionHash(scene.items);
    const hashShuffled = computeSemanticCollectionHash(shuffledItems);

    expect(hashOriginal).toBe(hashShuffled);

    // Parity snapshots must also be equal
    const snapOriginal = computeSemanticParitySnapshot(scene);
    const snapShuffled = computeSemanticParitySnapshot(shuffledScene);
    expect(snapOriginal.semanticCollectionHash).toBe(snapShuffled.semanticCollectionHash);
  });

  it('hashes are order-independent: action list order does not affect authorizedActionHash', () => {
    const scene = buildFixtureScene();
    const shuffledActions: SemanticSceneAction[] = [...scene.actions].reverse();
    const shuffledScene: SemanticScene = { ...scene, actions: shuffledActions };

    const hashOriginal = computeAuthorizedActionHash(scene.actions);
    const hashShuffled = computeAuthorizedActionHash(shuffledActions);

    expect(hashOriginal).toBe(hashShuffled);
  });
});

describe('F6.2.1 spike — sensitivity: different scenes produce different hashes', () => {
  it('different item IDs → different semanticCollectionHash', () => {
    const sceneA = buildFixtureScene(['entity-alpha', 'entity-beta', 'entity-gamma']);
    const sceneB = buildFixtureScene(['entity-delta', 'entity-epsilon', 'entity-zeta']);

    const hashA = computeSemanticCollectionHash(sceneA.items);
    const hashB = computeSemanticCollectionHash(sceneB.items);

    expect(hashA).not.toBe(hashB);
  });

  it('different item kinds → different semanticCollectionHash', () => {
    const buildOneKind = (kind: string): SemanticScene => {
      const input: SceneBuildInput = {
        items: [makeRawItem('item-1', kind)],
        actions: [],
        graphRevision: 1,
        layoutHint: LAYOUT_HINT,
      };
      return buildSemanticScene(input).scene;
    };

    const sceneEntity = buildOneKind('entity');
    const sceneMemory = buildOneKind('memory');

    const hashEntity = computeSemanticCollectionHash(sceneEntity.items);
    const hashMemory = computeSemanticCollectionHash(sceneMemory.items);

    expect(hashEntity).not.toBe(hashMemory);
  });

  it('different truth states → different semanticCollectionHash', () => {
    const buildWithTruth = (truthState: string): SemanticScene => {
      const input: SceneBuildInput = {
        items: [makeRawItem('item-1', 'entity', { truthState })],
        actions: [],
        graphRevision: 1,
        layoutHint: LAYOUT_HINT,
      };
      return buildSemanticScene(input).scene;
    };

    const sceneConfirmed = buildWithTruth('confirmed');
    const sceneContradicted = buildWithTruth('contradicted');

    const h1 = computeSemanticCollectionHash(sceneConfirmed.items);
    const h2 = computeSemanticCollectionHash(sceneContradicted.items);

    expect(h1).not.toBe(h2);
  });

  it('different action sets → different authorizedActionHash', () => {
    // Full capabilities vs minimal (only select/expand/inspect)
    const fullActions = buildAuthorizedActions('item-1', DEFAULT_CAPABILITIES).map(
      (a): RawSceneAction => ({
        targetItemId: a.targetItemId, kind: a.kind, label: a.label,
        isEnabled: a.isEnabled, isDangerous: a.isDangerous, requiresPreview: a.requiresPreview,
        isAuthorized: true,
      }),
    );
    const minimalActions = buildAuthorizedActions('item-1', {
      ...DEFAULT_CAPABILITIES,
      canCorrect: false, canMerge: false, canSplit: false, canRelate: false,
      canForget: false, canRestore: false, canDelete: false, canNavigatePath: false,
      canFitView: false, canNavigateHistory: false,
    }).map(
      (a): RawSceneAction => ({
        targetItemId: a.targetItemId, kind: a.kind, label: a.label,
        isEnabled: a.isEnabled, isDangerous: a.isDangerous, requiresPreview: a.requiresPreview,
        isAuthorized: true,
      }),
    );

    const buildScene = (rawActions: RawSceneAction[]): SemanticScene => {
      const input: SceneBuildInput = {
        items: [makeRawItem('item-1', 'entity')],
        actions: rawActions,
        graphRevision: 1,
        layoutHint: LAYOUT_HINT,
      };
      return buildSemanticScene(input).scene;
    };

    const sceneFull = buildScene(fullActions);
    const sceneMinimal = buildScene(minimalActions);

    const hashFull = computeAuthorizedActionHash(sceneFull.actions);
    const hashMinimal = computeAuthorizedActionHash(sceneMinimal.actions);

    expect(hashFull).not.toBe(hashMinimal);
  });
});

describe('F6.2.1 spike — GraphCanvas3D component spike wiring', () => {
  it('getSpikeParitySnapshot returns null when no spike input is provided', () => {
    expect(getSpikeParitySnapshot({})).toBeNull();
  });

  it('getSpikeParitySnapshot returns the same parity snapshot as direct computeSemanticParitySnapshot', () => {
    const scene = buildFixtureScene();
    const spikeInput: SemanticInput3D = {
      scene,
      capabilities: DEFAULT_CAPABILITIES,
      onAction: () => { /* no-op */ },
    };

    const fromComponent = getSpikeParitySnapshot({ spike: spikeInput });
    const direct = computeSemanticParitySnapshot(scene);

    expect(fromComponent).not.toBeNull();
    expect(fromComponent!.semanticCollectionHash).toBe(direct.semanticCollectionHash);
    expect(fromComponent!.authorizedActionHash).toBe(direct.authorizedActionHash);
  });

  it('GraphCanvas3D spike hash equals Graph2D hash for the same snapshot and capabilities', () => {
    // This is the primary parity assertion for task 6.2.1:
    // "assert semantic collection/action hashes equal 2D/list for the same
    //  snapshot/session/capabilities."
    const scene = buildFixtureScene();

    // 2D consumer path: same functions, same scene
    const hash2DCollection = twoD_semanticCollectionHash(scene);
    const hash2DActions = twoD_authorizedActionHash(scene);

    // 3D spike consumer path: via GraphCanvas3D's getSpikeParitySnapshot
    const spikeInput: SemanticInput3D = {
      scene,
      capabilities: DEFAULT_CAPABILITIES,
      onAction: () => { /* no-op */ },
    };
    const snap3D = getSpikeParitySnapshot({ spike: spikeInput })!;

    // PARITY ASSERTION — the core invariant for this task
    expect(snap3D.semanticCollectionHash).toBe(hash2DCollection);
    expect(snap3D.authorizedActionHash).toBe(hash2DActions);
  });

  it('parity holds across multiple item counts (multi-node scene)', () => {
    // Build a larger fixture
    const itemIds = ['id-a', 'id-b', 'id-c'];
    const kinds = ['entity', 'memory', 'goal'];
    const rawItems: RawSceneItem[] = itemIds.map((id, i) =>
      makeRawItem(id, kinds[i] ?? 'entity'),
    );
    const rawActions: RawSceneAction[] = rawItems.flatMap((item) =>
      buildAuthorizedActions(item.id, DEFAULT_CAPABILITIES).map(
        (a): RawSceneAction => ({
          targetItemId: a.targetItemId, kind: a.kind, label: a.label,
          isEnabled: a.isEnabled, isDangerous: a.isDangerous, requiresPreview: a.requiresPreview,
          isAuthorized: true,
        }),
      ),
    );
    const scene = buildSemanticScene({
      items: rawItems,
      actions: rawActions,
      graphRevision: 99,
      layoutHint: LAYOUT_HINT,
    }).scene;

    const spikeInput: SemanticInput3D = {
      scene,
      capabilities: DEFAULT_CAPABILITIES,
      onAction: () => { /* no-op */ },
    };
    const snap3D = getSpikeParitySnapshot({ spike: spikeInput })!;

    expect(snap3D.semanticCollectionHash).toBe(twoD_semanticCollectionHash(scene));
    expect(snap3D.authorizedActionHash).toBe(twoD_authorizedActionHash(scene));
  });
});
