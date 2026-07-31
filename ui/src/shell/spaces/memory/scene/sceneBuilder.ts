/**
 * memory/scene/sceneBuilder — Pure semantic scene builder.
 *
 * Pure TypeScript module — no JSX, no DOM, no side effects.
 *
 * This module accepts raw (unvalidated) scene data and produces a canonical
 * SemanticScene. It enforces all scene-construction invariants:
 *   • Malformed items/actions are omitted with diagnostics.
 *   • Unauthorized items/actions are silently omitted (no diagnostic).
 *   • Relation endpoints are validated against the authorized item set.
 *   • Output is deterministically ordered (items by id, actions by targetItemId+kind).
 *   • No new facts are derived — all values pass through from input unchanged.
 *   • sceneHash is a stable FNV-1a-style hash of sorted ids+kinds+revisions.
 *   • Visual tokens are generated per item using a kind→shape/color lookup.
 *   • Actions whose targetItemId is not in the final valid item set are omitted.
 *
 * Design invariants: F4.6 — equal input → equal scene hash.
 *
 * IDs: MGD-003–004, MGD-026, MGD-046; MG-M09–M11.
 */

import type {
  SemanticScene,
  SemanticSceneItem,
  SemanticSceneAction,
  SemanticVisualToken,
  SemanticLayoutHint,
  SemanticSceneDiagnostic,
  SceneTokenShape,
  SceneActionKind,
  SceneItemKind,
  SceneItemDirection,
} from "./semanticScene";

// ─── Input types ──────────────────────────────────────────────────────────────

export interface RawSceneItem {
  id: string;
  kind: string;
  authorityClass: string;
  label: string | null;          // null = malformed
  truthState: string | null;     // null = malformed
  graphRevision: number | null;  // null = malformed
  direction: string | null;
  sourceEndpointId: string | null;
  targetEndpointId: string | null;
  evidenceCount: number | null;
  evidenceSummary: string | null;
  provenanceSourceId: string | null;
  provenanceMethod: string | null;
  provenanceVersion: string | null;
  provenanceActorLabel: string | null;
  validTimeStart: string | null;
  validTimeEnd: string | null;
  isCurrentlyValid: boolean;
  isSelected: boolean;
  isFocused: boolean;
  isInPath: boolean;
  isPending: boolean;
  hasError: boolean;
  isAuthorized: boolean;         // false = omit from scene
}

export interface RawSceneAction {
  targetItemId: string;
  kind: string;
  label: string | null;           // null = malformed
  isEnabled: boolean;
  isDangerous: boolean;
  requiresPreview: boolean;
  isAuthorized: boolean;          // false = omit
}

export interface SceneBuildInput {
  items: RawSceneItem[];
  actions: RawSceneAction[];
  graphRevision: number;
  layoutHint: SemanticLayoutHint;
}

export interface SceneBuildResult {
  scene: SemanticScene;
  omittedItemCount: number;       // count of items omitted (malformed or unauthorized)
  omittedActionCount: number;     // count of actions omitted
  diagnostics: SemanticSceneDiagnostic[];
}

// ─── Valid SceneActionKind set ────────────────────────────────────────────────

const VALID_ACTION_KINDS = new Set<SceneActionKind>([
  "select",
  "expand",
  "inspect",
  "path",
  "correct",
  "merge",
  "split",
  "relate",
  "forget",
  "restore",
  "delete",
  "fit",
  "back",
  "forward",
]);

function isValidActionKind(kind: string): kind is SceneActionKind {
  return VALID_ACTION_KINDS.has(kind as SceneActionKind);
}

const VALID_ITEM_KINDS = new Set<SceneItemKind>([
  "entity", "memory", "evidence", "aggregate", "relation", "goal", "source",
  "episode", "summary", "skill", "rule", "navigation-container",
]);
const VALID_DIRECTIONS = new Set<Exclude<SceneItemDirection, null>>([
  "outgoing", "incoming", "symmetric",
]);

function isValidItemKind(kind: string): kind is SceneItemKind {
  return VALID_ITEM_KINDS.has(kind as SceneItemKind);
}

function isValidDirection(direction: string | null): direction is SceneItemDirection {
  return direction === null || VALID_DIRECTIONS.has(direction as Exclude<SceneItemDirection, null>);
}

// ─── Visual token lookup tables ───────────────────────────────────────────────

const KIND_TO_SHAPE: Record<string, SceneTokenShape> = {
  entity: "circle",
  memory: "rect",
  evidence: "diamond",
  aggregate: "hexagon",
  relation: "line",
  goal: "hexagon",
  source: "diamond",
  episode: "rect",
  summary: "rect",
  skill: "hexagon",
  rule: "triangle",
  "navigation-container": "rect",
};

const KIND_TO_COLOR: Record<string, string> = {
  entity: "--color-entity",
  memory: "--color-memory",
  evidence: "--color-success-solid",
  aggregate: "--color-accent-secondary",
  relation: "--color-relation",
  goal: "--color-accent-secondary",
  source: "--color-warning-solid",
  episode: "--color-accent-secondary",
  summary: "--color-text-secondary",
  skill: "--color-success-solid",
  rule: "--color-danger-solid",
  "navigation-container": "--color-text-muted",
};

// ─── FNV-1a-style deterministic hash ─────────────────────────────────────────

/**
 * FNV-1a 32-bit hash over a UTF-16 string.
 *
 * Stable across runs — only depends on input characters, not on runtime state.
 * Returns a hex string prefixed with "fnv1a-".
 */
function fnv1aHash(input: string): string {
  let hash = 0x811c9dc5; // FNV offset basis (32-bit)
  for (let i = 0; i < input.length; i++) {
    hash ^= input.charCodeAt(i);
    // Multiply by FNV prime (32-bit): 0x01000193
    // JavaScript bitwise ops work on 32-bit signed integers; >>> 0 coerces to uint32.
    hash = (Math.imul(hash, 0x01000193) >>> 0);
  }
  return "fnv1a-" + hash.toString(16).padStart(8, "0");
}

// ─── Builder core ─────────────────────────────────────────────────────────────

/**
 * Builds a canonical SemanticScene from raw input.
 *
 * Pure function — same input always produces the same output.
 */
export function buildSemanticScene(input: SceneBuildInput): SceneBuildResult {
  const diagnostics: SemanticSceneDiagnostic[] = [];
  let omittedItemCount = 0;
  let omittedActionCount = 0;

  // ── Step 1: Collect authorized item IDs (before malformed check) ──
  // We need this to validate relation endpoints against the authorized set.
  const authorizedIds = new Set<string>(
    input.items
      .filter((raw) => raw.isAuthorized)
      .map((raw) => raw.id),
  );

  // ── Step 2: Validate and build items ──────────────────────────────
  const validItems: SemanticSceneItem[] = [];

  for (const raw of input.items) {
    // Unauthorized: silently omit — no diagnostic.
    if (!raw.isAuthorized) {
      omittedItemCount++;
      continue;
    }

    // Required fields and closed enums are validated before entering the scene.
    if (
      raw.label === null ||
      raw.truthState === null ||
      raw.graphRevision === null ||
      !isValidItemKind(raw.kind) ||
      !isValidDirection(raw.direction) ||
      (raw.kind !== "relation" && raw.direction !== null)
    ) {
      omittedItemCount++;
      diagnostics.push({
        level: "warning",
        message: `Item '${raw.id}' omitted: malformed required field or unsupported kind/direction.`,
        relatedItemId: raw.id,
      });
      continue;
    }

    // Relation endpoint validation: for relation items with non-null direction,
    // both sourceEndpointId and targetEndpointId must be present and appear
    // in the authorized items set.
    if (raw.kind === "relation" && raw.direction !== null) {
      const srcMissing =
        raw.sourceEndpointId === null ||
        !authorizedIds.has(raw.sourceEndpointId);
      const tgtMissing =
        raw.targetEndpointId === null ||
        !authorizedIds.has(raw.targetEndpointId);

      if (srcMissing || tgtMissing) {
        omittedItemCount++;
        diagnostics.push({
          level: "warning",
          message: `Relation '${raw.id}' omitted: endpoint(s) missing or not in authorized item set.`,
          relatedItemId: raw.id,
        });
        continue;
      }
    }

    // Build the canonical item — pass through values, derive nothing.
    const item: SemanticSceneItem = {
      id: raw.id,
      kind: raw.kind,
      authorityClass: raw.authorityClass,
      label: raw.label,
      truthState: raw.truthState,
      graphRevision: raw.graphRevision,
      direction: raw.direction,
      sourceEndpointId: raw.sourceEndpointId,
      targetEndpointId: raw.targetEndpointId,
      evidenceCount: raw.evidenceCount ?? 0,
      evidenceSummary: raw.evidenceSummary,
      provenance: {
        sourceId: raw.provenanceSourceId,
        method: raw.provenanceMethod,
        version: raw.provenanceVersion,
        actorLabel: raw.provenanceActorLabel,
      },
      validity: {
        validTimeStart: raw.validTimeStart,
        validTimeEnd: raw.validTimeEnd,
        isCurrentlyValid: raw.isCurrentlyValid,
      },
      isSelected: raw.isSelected,
      isFocused: raw.isFocused,
      isInPath: raw.isInPath,
      isPending: raw.isPending,
      hasError: raw.hasError,
    };

    validItems.push(item);
  }

  // ── Step 3: Revalidate edges against the final valid node collection ──
  // An authorized but malformed endpoint must not allow a dangling relation to
  // survive. Navigation containers and relation records cannot be endpoints.
  const finalNodeIds = new Set(
    validItems
      .filter((item) => item.kind !== "relation" && item.kind !== "navigation-container")
      .map((item) => item.id),
  );
  const endpointCompleteItems = validItems.filter((item) => {
    if (item.kind !== "relation" || item.direction === null) return true;
    const complete =
      item.sourceEndpointId !== null &&
      item.targetEndpointId !== null &&
      finalNodeIds.has(item.sourceEndpointId) &&
      finalNodeIds.has(item.targetEndpointId);
    if (!complete) {
      omittedItemCount++;
      diagnostics.push({
        level: "warning",
        message: `Relation '${item.id}' omitted: endpoint did not survive item validation.`,
        relatedItemId: item.id,
      });
    }
    return complete;
  });

  const sortedItems = [...endpointCompleteItems].sort((a, b) =>
    a.id < b.id ? -1 : a.id > b.id ? 1 : 0,
  );

  // Build a set of final valid item IDs for action filtering.
  const finalItemIds = new Set<string>(sortedItems.map((i) => i.id));

  // ── Step 4: Validate and build actions ────────────────────────────
  const validActions: SemanticSceneAction[] = [];

  for (const raw of input.actions) {
    // Unauthorized: silently omit — no diagnostic.
    if (!raw.isAuthorized) {
      omittedActionCount++;
      continue;
    }

    // Malformed: label must be non-null; kind must be a valid SceneActionKind.
    if (raw.label === null || !isValidActionKind(raw.kind)) {
      omittedActionCount++;
      diagnostics.push({
        level: "warning",
        message: `Action '${raw.kind}' for item '${raw.targetItemId}' omitted: malformed (null label or invalid kind).`,
        relatedItemId: raw.targetItemId,
      });
      continue;
    }

    // Actions whose targetItemId is not in the final valid item set are omitted.
    if (!finalItemIds.has(raw.targetItemId)) {
      omittedActionCount++;
      continue;
    }

    const action: SemanticSceneAction = {
      targetItemId: raw.targetItemId,
      kind: raw.kind,
      label: raw.label,
      isEnabled: raw.isEnabled,
      isDangerous: raw.isDangerous,
      requiresPreview: raw.requiresPreview,
    };

    validActions.push(action);
  }

  // ── Step 5: Deterministic ordering — sort actions by (targetItemId, kind) ──
  const sortedActions = [...validActions].sort((a, b) => {
    if (a.targetItemId < b.targetItemId) return -1;
    if (a.targetItemId > b.targetItemId) return 1;
    if (a.kind < b.kind) return -1;
    if (a.kind > b.kind) return 1;
    return 0;
  });

  // ── Step 6: Visual tokens ─────────────────────────────────────────
  const tokens: SemanticVisualToken[] = sortedItems.map((item): SemanticVisualToken => ({
    itemId: item.id,
    shape: KIND_TO_SHAPE[item.kind] ?? "rect",
    colorToken: KIND_TO_COLOR[item.kind] ?? "--color-entity",
    iconId: null,
    displayLabel: item.label,
    showLabel: true,
  }));

  // ── Step 7: Compute full semantic parity hash ─────────────────────
  // Property insertion order is fixed here and arrays are already sorted.
  // Labels, states, endpoints, evidence, actions, tokens, and layout changes
  // therefore invalidate parity instead of silently retaining an old hash.
  const hashInput = JSON.stringify({
    graphRevision: input.graphRevision,
    items: sortedItems,
    actions: sortedActions,
    tokens,
    layoutHint: input.layoutHint,
  });
  const sceneHash = fnv1aHash(hashInput);

  // ── Step 8: Assemble final scene ──────────────────────────────────
  const scene: SemanticScene = {
    sceneHash,
    graphRevision: input.graphRevision,
    items: sortedItems,
    actions: sortedActions,
    tokens,
    layoutHint: input.layoutHint,
    diagnostics,
  };

  return {
    scene,
    omittedItemCount,
    omittedActionCount,
    diagnostics,
  };
}
