/**
 * memory/scene/pathScene — Path query visual representation.
 *
 * Pure TypeScript module — no JSX, no DOM, no side effects.
 *
 * Solves the cycle-identity problem in path queries: when a semantic ID
 * appears multiple times along a path (e.g. A → B → C → A), the semantic
 * collection must contain A exactly once (stable identity), but the visual
 * representation needs A to appear at both positions.
 *
 * Solution: "visual placeholder" items for repeated positions.
 *   • First occurrence → a `semantic` visual item referencing the real ID.
 *   • Subsequent occurrences → a `placeholder` visual item with a unique
 *     visual ID, pointing back to the original semantic ID.
 *   • Navigation containers are produced via `buildNavigationContainerItem`
 *     for the placeholder slots; they carry kind='navigation-container' and
 *     are NOT included in `uniqueSemanticIds`.
 *
 * Design invariants (Task 4.6.4):
 *   1. Each semantic ID appears exactly once in `uniqueSemanticIds`.
 *   2. `visualPathItems.length === pathNodes.length`.
 *   3. First occurrence of an ID → kind.type === 'semantic'.
 *   4. Subsequent occurrences → kind.type === 'placeholder' with
 *      placeholderId = "placeholder-{originalId}-{position}".
 *   5. `isFirstOccurrence` is true for the first occurrence, false for repeats.
 *   6. Navigation containers are NOT in `uniqueSemanticIds`.
 *
 * IDs: MGD-task-4.6.4
 */

import type { SemanticSceneItem } from './semanticScene';

// ─── Input types ──────────────────────────────────────────────────────────────

/**
 * One node in an ordered path.
 *
 * `semanticId` is the actual semantic item ID (stable). `visualPosition` is
 * the 0-indexed position in the path sequence. Cycles produce duplicate
 * `semanticId` values at different `visualPosition` values.
 */
export interface PathNode {
  semanticId: string;
  visualPosition: number;
}

export interface PathSceneInput {
  /** Ordered path nodes; may include duplicate semanticIds (cycles). */
  pathNodes: PathNode[];
  graphRevision: number;
}

// ─── Visual item types ────────────────────────────────────────────────────────

/**
 * Discriminated union describing how a visual path item relates to the
 * semantic collection.
 *
 * - `semantic`: The item IS the semantic item at this path position (first
 *   occurrence). `semanticId` matches a real entry in `uniqueSemanticIds`.
 * - `placeholder`: A visual alias of a semantic item that already appeared
 *   earlier in the path (repeat/cycle). `originalId` points to the real
 *   semantic ID; `placeholderId` is a unique visual-only ID for this slot.
 */
export type VisualPathItemKind =
  | { type: 'semantic'; semanticId: string }
  | { type: 'placeholder'; originalId: string; placeholderId: string };

/**
 * One visual item in the rendered path.
 *
 * `visualId` is unique across all visual items in the path (either the
 * semantic ID for first occurrences, or the placeholderId for repeats).
 */
export interface VisualPathItem {
  visualId: string;
  position: number;
  kind: VisualPathItemKind;
  isFirstOccurrence: boolean;
}

// ─── Result type ──────────────────────────────────────────────────────────────

/**
 * Result of building a path scene.
 *
 * `uniqueSemanticIds` — each real semantic ID exactly once, in order of first
 * appearance. Use this to load semantic items from the semantic collection.
 *
 * `visualPathItems` — one entry per input PathNode (length always equals
 * pathNodes.length). First occurrences reference semantic items; repeats
 * reference placeholder navigation-container items.
 */
export interface PathSceneResult {
  /** Each ID appears exactly once. Length === count of unique IDs. */
  uniqueSemanticIds: string[];
  /** Length === pathNodes.length. */
  visualPathItems: VisualPathItem[];
}

// ─── Core builder ─────────────────────────────────────────────────────────────

/**
 * Builds the path scene representation for a cycle-safe path query.
 *
 * Pure function — same input always produces the same output.
 *
 * Algorithm:
 *   For each PathNode in order:
 *     - If semanticId is seen for the first time: emit a 'semantic' VisualPathItem
 *       and add the ID to uniqueSemanticIds.
 *     - If semanticId has been seen before: emit a 'placeholder' VisualPathItem
 *       with placeholderId = "placeholder-{semanticId}-{visualPosition}".
 */
export function buildPathScene(input: PathSceneInput): PathSceneResult {
  const seenIds = new Set<string>();
  const uniqueSemanticIds: string[] = [];
  const visualPathItems: VisualPathItem[] = [];

  for (const node of input.pathNodes) {
    const { semanticId, visualPosition } = node;

    if (!seenIds.has(semanticId)) {
      // First occurrence: real semantic item.
      seenIds.add(semanticId);
      uniqueSemanticIds.push(semanticId);

      visualPathItems.push({
        visualId: semanticId,
        position: visualPosition,
        kind: { type: 'semantic', semanticId },
        isFirstOccurrence: true,
      });
    } else {
      // Repeat occurrence: visual placeholder.
      const placeholderId = `placeholder-${semanticId}-${visualPosition}`;

      visualPathItems.push({
        visualId: placeholderId,
        position: visualPosition,
        kind: { type: 'placeholder', originalId: semanticId, placeholderId },
        isFirstOccurrence: false,
      });
    }
  }

  return { uniqueSemanticIds, visualPathItems };
}

// ─── Navigation container factory ────────────────────────────────────────────

/**
 * Builds a `SemanticSceneItem` representing a visual placeholder (alias) for
 * a repeated semantic ID in a path.
 *
 * The returned item has kind='navigation-container' and is intended as a
 * visual-only slot in the rendered path — it is NOT a semantic entity and
 * must NOT appear in `uniqueSemanticIds`.
 *
 * Properties:
 *   - id             = placeholderId
 *   - kind           = 'navigation-container'
 *   - label          = '→ {originalLabel}'
 *   - direction      = null  (not an edge)
 *   - All boolean state fields = false
 *   - provenance: all null values
 *   - validity: null intervals, isCurrentlyValid = false
 *   - graphRevision  = graphRevision (passed through)
 *   - authorityClass = 'public' (visual artifact, no authority class)
 *   - truthState     = 'placeholder'
 *   - evidenceCount  = 0
 */
export function buildNavigationContainerItem(
  placeholderId: string,
  originalId: string,
  originalLabel: string,
  graphRevision: number,
): SemanticSceneItem {
  return {
    id: placeholderId,
    kind: 'navigation-container',
    authorityClass: 'public',
    label: `→ ${originalLabel}`,
    truthState: 'placeholder',
    graphRevision,
    direction: null,
    sourceEndpointId: null,
    targetEndpointId: null,
    evidenceCount: 0,
    evidenceSummary: null,
    provenance: {
      sourceId: null,
      method: null,
      version: null,
      actorLabel: null,
    },
    validity: {
      validTimeStart: null,
      validTimeEnd: null,
      isCurrentlyValid: false,
    },
    isSelected: false,
    isFocused: false,
    isInPath: false,
    isPending: false,
    hasError: false,
  };
}
