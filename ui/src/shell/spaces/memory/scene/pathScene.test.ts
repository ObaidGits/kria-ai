/**
 * pathScene.test.ts — Unit tests for buildPathScene and buildNavigationContainerItem.
 *
 * Pure TypeScript / vitest — no JSX, no DOM.
 *
 * Covers all invariants from Task 4.6.4:
 *   1. Each semantic ID appears exactly once in uniqueSemanticIds.
 *   2. visualPathItems.length === pathNodes.length.
 *   3. First occurrence → kind.type === 'semantic'.
 *   4. Subsequent occurrences → kind.type === 'placeholder' with
 *      placeholderId = "placeholder-{originalId}-{position}".
 *   5. isFirstOccurrence = true for first, false for repeats.
 *   6. Navigation containers are NOT in uniqueSemanticIds.
 */

import { describe, it, expect } from 'vitest';
import {
  buildPathScene,
  buildNavigationContainerItem,
  type PathNode,
  type PathSceneInput,
} from './pathScene';

// ─── Helper ───────────────────────────────────────────────────────────────────

function nodes(...ids: string[]): PathNode[] {
  return ids.map((semanticId, index) => ({ semanticId, visualPosition: index }));
}

function input(pathNodes: PathNode[], graphRevision = 1): PathSceneInput {
  return { pathNodes, graphRevision };
}

// ─── buildPathScene ───────────────────────────────────────────────────────────

describe('buildPathScene', () => {
  // ── Empty path ─────────────────────────────────────────────────────────────

  it('empty path → empty uniqueSemanticIds and visualPathItems', () => {
    const result = buildPathScene(input([]));
    expect(result.uniqueSemanticIds).toEqual([]);
    expect(result.visualPathItems).toEqual([]);
  });

  // ── Single item ────────────────────────────────────────────────────────────

  it('single item path → one uniqueSemanticId, isFirstOccurrence=true', () => {
    const result = buildPathScene(input(nodes('A')));

    expect(result.uniqueSemanticIds).toEqual(['A']);
    expect(result.visualPathItems).toHaveLength(1);

    const item = result.visualPathItems[0];
    expect(item.isFirstOccurrence).toBe(true);
    expect(item.kind.type).toBe('semantic');
    if (item.kind.type === 'semantic') {
      expect(item.kind.semanticId).toBe('A');
    }
    expect(item.visualId).toBe('A');
    expect(item.position).toBe(0);
  });

  // ── Path with no repeats ───────────────────────────────────────────────────

  it('path with no repeats → all first occurrences, all IDs in uniqueSemanticIds', () => {
    const result = buildPathScene(input(nodes('A', 'B', 'C')));

    expect(result.uniqueSemanticIds).toEqual(['A', 'B', 'C']);
    expect(result.visualPathItems).toHaveLength(3);

    for (const item of result.visualPathItems) {
      expect(item.isFirstOccurrence).toBe(true);
      expect(item.kind.type).toBe('semantic');
    }
  });

  // ── visualPathItems.length invariant ──────────────────────────────────────

  it('visualPathItems.length === pathNodes.length always', () => {
    for (const ids of [[], ['A'], ['A', 'B'], ['A', 'B', 'A'], ['X', 'Y', 'Z', 'X', 'Y']]) {
      const pathNodes = nodes(...ids);
      const result = buildPathScene(input(pathNodes));
      expect(result.visualPathItems).toHaveLength(pathNodes.length);
    }
  });

  // ── uniqueSemanticIds count ────────────────────────────────────────────────

  it('uniqueSemanticIds.length === count of distinct IDs (not total path length)', () => {
    // A → B → C → A: 4 nodes but only 3 distinct IDs
    const result = buildPathScene(input(nodes('A', 'B', 'C', 'A')));
    expect(result.uniqueSemanticIds).toHaveLength(3);
    expect(result.uniqueSemanticIds).toEqual(['A', 'B', 'C']);
  });

  it('each semantic ID appears exactly once in uniqueSemanticIds (cycle with more repeats)', () => {
    // A → B → A → B → A: 5 nodes, 2 distinct IDs
    const result = buildPathScene(input(nodes('A', 'B', 'A', 'B', 'A')));
    expect(result.uniqueSemanticIds).toHaveLength(2);
    // No duplicates
    expect(new Set(result.uniqueSemanticIds).size).toBe(2);
  });

  // ── Path with one cycle ────────────────────────────────────────────────────

  it('path with one cycle → first occurrence is semantic, second is placeholder', () => {
    // A → B → C → A
    const result = buildPathScene(input(nodes('A', 'B', 'C', 'A')));

    // First A (position 0) → semantic
    const firstA = result.visualPathItems[0];
    expect(firstA.isFirstOccurrence).toBe(true);
    expect(firstA.kind.type).toBe('semantic');
    if (firstA.kind.type === 'semantic') {
      expect(firstA.kind.semanticId).toBe('A');
    }
    expect(firstA.visualId).toBe('A');

    // Second A (position 3) → placeholder
    const secondA = result.visualPathItems[3];
    expect(secondA.isFirstOccurrence).toBe(false);
    expect(secondA.kind.type).toBe('placeholder');
    if (secondA.kind.type === 'placeholder') {
      expect(secondA.kind.originalId).toBe('A');
      expect(secondA.kind.placeholderId).toBe('placeholder-A-3');
    }
    expect(secondA.visualId).toBe('placeholder-A-3');
    expect(secondA.position).toBe(3);
  });

  // ── Multiple cycles of same ID ─────────────────────────────────────────────

  it('multiple cycles of same ID → each repeat gets unique placeholderId', () => {
    // A → B → A → B → A (positions 0..4)
    const result = buildPathScene(input(nodes('A', 'B', 'A', 'B', 'A')));

    // Position 0: first A → semantic
    expect(result.visualPathItems[0].isFirstOccurrence).toBe(true);
    expect(result.visualPathItems[0].kind.type).toBe('semantic');

    // Position 2: second A → placeholder-A-2
    const repeat1 = result.visualPathItems[2];
    expect(repeat1.isFirstOccurrence).toBe(false);
    expect(repeat1.kind.type).toBe('placeholder');
    if (repeat1.kind.type === 'placeholder') {
      expect(repeat1.kind.placeholderId).toBe('placeholder-A-2');
      expect(repeat1.kind.originalId).toBe('A');
    }
    expect(repeat1.visualId).toBe('placeholder-A-2');

    // Position 4: third A → placeholder-A-4
    const repeat2 = result.visualPathItems[4];
    expect(repeat2.isFirstOccurrence).toBe(false);
    expect(repeat2.kind.type).toBe('placeholder');
    if (repeat2.kind.type === 'placeholder') {
      expect(repeat2.kind.placeholderId).toBe('placeholder-A-4');
      expect(repeat2.kind.originalId).toBe('A');
    }
    expect(repeat2.visualId).toBe('placeholder-A-4');

    // Placeholder IDs must be unique
    const placeholderIds = result.visualPathItems
      .filter((i) => !i.isFirstOccurrence)
      .map((i) => i.visualId);
    expect(new Set(placeholderIds).size).toBe(placeholderIds.length);
  });

  // ── Placeholder kind.originalId points to first occurrence ────────────────

  it('placeholder kind.originalId always points to first-occurrence semantic ID', () => {
    // X → Y → Z → Y → X
    const result = buildPathScene(input(nodes('X', 'Y', 'Z', 'Y', 'X')));

    // Second Y at position 3
    const secondY = result.visualPathItems[3];
    expect(secondY.kind.type).toBe('placeholder');
    if (secondY.kind.type === 'placeholder') {
      expect(secondY.kind.originalId).toBe('Y');
    }

    // Second X at position 4
    const secondX = result.visualPathItems[4];
    expect(secondX.kind.type).toBe('placeholder');
    if (secondX.kind.type === 'placeholder') {
      expect(secondX.kind.originalId).toBe('X');
    }

    // Both original IDs are in uniqueSemanticIds
    expect(result.uniqueSemanticIds).toContain('X');
    expect(result.uniqueSemanticIds).toContain('Y');
    expect(result.uniqueSemanticIds).toContain('Z');
    expect(result.uniqueSemanticIds).toHaveLength(3);
  });

  // ── Placeholder placeholderId is unique per position ──────────────────────

  it('placeholderId is unique per position even for different IDs', () => {
    // A → B → A → B: positions 0,1,2,3
    const result = buildPathScene(input(nodes('A', 'B', 'A', 'B')));

    const placeholders = result.visualPathItems.filter((i) => !i.isFirstOccurrence);
    expect(placeholders).toHaveLength(2);

    const ids = placeholders.map((p) => p.visualId);
    expect(ids).toContain('placeholder-A-2');
    expect(ids).toContain('placeholder-B-3');
    expect(new Set(ids).size).toBe(2);
  });

  // ── isFirstOccurrence correctness ─────────────────────────────────────────

  it('isFirstOccurrence is correct for all items in a mixed path', () => {
    // P → Q → P → R → Q → P  (positions 0..5)
    const pathNodes: PathNode[] = [
      { semanticId: 'P', visualPosition: 0 },
      { semanticId: 'Q', visualPosition: 1 },
      { semanticId: 'P', visualPosition: 2 },
      { semanticId: 'R', visualPosition: 3 },
      { semanticId: 'Q', visualPosition: 4 },
      { semanticId: 'P', visualPosition: 5 },
    ];
    const result = buildPathScene(input(pathNodes));

    const expected = [true, true, false, true, false, false];
    result.visualPathItems.forEach((item, i) => {
      expect(item.isFirstOccurrence).toBe(expected[i]);
    });

    expect(result.uniqueSemanticIds).toEqual(['P', 'Q', 'R']);
  });

  // ── Navigation containers are NOT in uniqueSemanticIds ────────────────────

  it('placeholder visual IDs are not in uniqueSemanticIds', () => {
    // A → B → A
    const result = buildPathScene(input(nodes('A', 'B', 'A')));

    // Only real semantic IDs should be in uniqueSemanticIds
    expect(result.uniqueSemanticIds).toEqual(['A', 'B']);
    expect(result.uniqueSemanticIds).not.toContain('placeholder-A-2');
  });

  // ── position is preserved from PathNode.visualPosition ────────────────────

  it('position matches the PathNode.visualPosition for all items', () => {
    // Non-contiguous positions (caller controls visualPosition)
    const pathNodes: PathNode[] = [
      { semanticId: 'A', visualPosition: 0 },
      { semanticId: 'B', visualPosition: 5 },
      { semanticId: 'A', visualPosition: 10 },
    ];
    const result = buildPathScene(input(pathNodes));

    expect(result.visualPathItems[0].position).toBe(0);
    expect(result.visualPathItems[1].position).toBe(5);
    expect(result.visualPathItems[2].position).toBe(10);

    // Placeholder ID uses the visualPosition from PathNode
    const placeholder = result.visualPathItems[2];
    if (placeholder.kind.type === 'placeholder') {
      expect(placeholder.kind.placeholderId).toBe('placeholder-A-10');
    }
  });
});

// ─── buildNavigationContainerItem ─────────────────────────────────────────────

describe('buildNavigationContainerItem', () => {
  const PLACEHOLDER_ID = 'placeholder-node-42-7';
  const ORIGINAL_ID = 'node-42';
  const ORIGINAL_LABEL = 'My Node';
  const GRAPH_REVISION = 17;

  it('id equals placeholderId', () => {
    const item = buildNavigationContainerItem(
      PLACEHOLDER_ID,
      ORIGINAL_ID,
      ORIGINAL_LABEL,
      GRAPH_REVISION,
    );
    expect(item.id).toBe(PLACEHOLDER_ID);
  });

  it('kind is navigation-container', () => {
    const item = buildNavigationContainerItem(
      PLACEHOLDER_ID,
      ORIGINAL_ID,
      ORIGINAL_LABEL,
      GRAPH_REVISION,
    );
    expect(item.kind).toBe('navigation-container');
  });

  it('label starts with "→ " and contains the originalLabel', () => {
    const item = buildNavigationContainerItem(
      PLACEHOLDER_ID,
      ORIGINAL_ID,
      ORIGINAL_LABEL,
      GRAPH_REVISION,
    );
    expect(item.label).toMatch(/^→ /);
    expect(item.label).toBe(`→ ${ORIGINAL_LABEL}`);
  });

  it('label "→ " prefix works with empty original label', () => {
    const item = buildNavigationContainerItem(PLACEHOLDER_ID, ORIGINAL_ID, '', GRAPH_REVISION);
    expect(item.label).toBe('→ ');
  });

  it('direction is null (not an edge)', () => {
    const item = buildNavigationContainerItem(
      PLACEHOLDER_ID,
      ORIGINAL_ID,
      ORIGINAL_LABEL,
      GRAPH_REVISION,
    );
    expect(item.direction).toBeNull();
  });

  it('all boolean state fields are false', () => {
    const item = buildNavigationContainerItem(
      PLACEHOLDER_ID,
      ORIGINAL_ID,
      ORIGINAL_LABEL,
      GRAPH_REVISION,
    );
    expect(item.isSelected).toBe(false);
    expect(item.isFocused).toBe(false);
    expect(item.isInPath).toBe(false);
    expect(item.isPending).toBe(false);
    expect(item.hasError).toBe(false);
  });

  it('graphRevision is passed through', () => {
    const item = buildNavigationContainerItem(
      PLACEHOLDER_ID,
      ORIGINAL_ID,
      ORIGINAL_LABEL,
      GRAPH_REVISION,
    );
    expect(item.graphRevision).toBe(GRAPH_REVISION);
  });

  it('graphRevision zero is preserved', () => {
    const item = buildNavigationContainerItem(PLACEHOLDER_ID, ORIGINAL_ID, ORIGINAL_LABEL, 0);
    expect(item.graphRevision).toBe(0);
  });

  it('provenance has all null values', () => {
    const item = buildNavigationContainerItem(
      PLACEHOLDER_ID,
      ORIGINAL_ID,
      ORIGINAL_LABEL,
      GRAPH_REVISION,
    );
    expect(item.provenance.sourceId).toBeNull();
    expect(item.provenance.method).toBeNull();
    expect(item.provenance.version).toBeNull();
    expect(item.provenance.actorLabel).toBeNull();
  });

  it('validity has null intervals and isCurrentlyValid=false', () => {
    const item = buildNavigationContainerItem(
      PLACEHOLDER_ID,
      ORIGINAL_ID,
      ORIGINAL_LABEL,
      GRAPH_REVISION,
    );
    expect(item.validity.validTimeStart).toBeNull();
    expect(item.validity.validTimeEnd).toBeNull();
    expect(item.validity.isCurrentlyValid).toBe(false);
  });

  it('sourceEndpointId and targetEndpointId are null', () => {
    const item = buildNavigationContainerItem(
      PLACEHOLDER_ID,
      ORIGINAL_ID,
      ORIGINAL_LABEL,
      GRAPH_REVISION,
    );
    expect(item.sourceEndpointId).toBeNull();
    expect(item.targetEndpointId).toBeNull();
  });

  it('evidenceCount is 0', () => {
    const item = buildNavigationContainerItem(
      PLACEHOLDER_ID,
      ORIGINAL_ID,
      ORIGINAL_LABEL,
      GRAPH_REVISION,
    );
    expect(item.evidenceCount).toBe(0);
  });

  it('evidenceSummary is null', () => {
    const item = buildNavigationContainerItem(
      PLACEHOLDER_ID,
      ORIGINAL_ID,
      ORIGINAL_LABEL,
      GRAPH_REVISION,
    );
    expect(item.evidenceSummary).toBeNull();
  });

  it('produces distinct items for distinct placeholder IDs', () => {
    const item1 = buildNavigationContainerItem('ph-1', 'node-A', 'Node A', 5);
    const item2 = buildNavigationContainerItem('ph-2', 'node-A', 'Node A', 5);
    expect(item1.id).toBe('ph-1');
    expect(item2.id).toBe('ph-2');
    expect(item1.id).not.toBe(item2.id);
  });
});
