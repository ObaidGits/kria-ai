/**
 * memory/scene/semanticScene — Canonical renderer-neutral semantic scene types.
 *
 * Pure TypeScript module — no JSX, no DOM, no side effects.
 *
 * This module is the single source of truth for the semantic scene consumed by
 * both the Canvas2D renderer and the semantic list. All types carry stable IDs,
 * policy-safe labels, authority/truth/direction/evidence/provenance/validity,
 * revision, and diagnostic information.
 *
 * Design invariants (F4.6):
 *   • Equal input → equal scene hash (deterministic).
 *   • Navigation containers are NOT edges — they are group containers only.
 *   • Only authorized actions are represented; unauthorized actions are omitted.
 *   • Selected identity is stable under aggregation.
 *   • All labels are policy-safe — no private identifiers.
 *   • No semantic inference is performed here; all values come from validated
 *     backend DTOs.
 *
 * IDs: MGR-001–002, MGR-004, MGR-012, MGR-026;
 *      MGD-003–004, MGD-026, MGD-046;
 *      MG-C03, MG-H02, MG-M09–M11, MG-O19.
 */

// ─── Scene item types ─────────────────────────────────────────────────────────

/**
 * The kind of a scene item.
 *
 * 'navigation-container' is a group container only — it is NOT an edge.
 * It represents a logical grouping of items (e.g. a cluster or expansion set)
 * and must never be rendered as a graph edge.
 */
export type SceneItemKind =
  | 'entity'
  | 'memory'
  | 'evidence'
  | 'aggregate'
  | 'relation'
  | 'goal'
  | 'source'
  | 'episode'
  | 'summary'
  | 'skill'
  | 'rule'
  | 'navigation-container'; // not an edge — group container only

/**
 * Edge direction for relation items.
 * null for non-edge (non-relation) items.
 */
export type SceneItemDirection =
  | 'outgoing'
  | 'incoming'
  | 'symmetric'
  | null; // null for non-edge items

/**
 * Authority class classifying the ownership/origin of a scene item.
 * Extensible via the string fallback.
 */
export type AuthorityClass =
  | 'personal'
  | 'work'
  | 'public'
  | string; // extensible

/**
 * Policy-safe provenance record.
 * actorLabel is a display label — never a raw user identifier.
 */
export interface SceneItemProvenance {
  sourceId: string | null;
  method: string | null;
  version: string | null;
  /** Policy-safe label — no private data. */
  actorLabel: string | null;
}

/**
 * Valid-time interval for a scene item.
 * Dates are ISO 8601 strings or null if unbounded.
 */
export interface SceneItemValidity {
  validTimeStart: string | null;
  validTimeEnd: string | null;
  isCurrentlyValid: boolean;
}

/**
 * One semantic item in the scene.
 *
 * Items are renderer-neutral: the same item type is consumed by Canvas2D,
 * the semantic list, and the inspector. IDs are stable across queries,
 * aggregation, and scene rebuilds.
 */
export interface SemanticSceneItem {
  /** Stable semantic ID — never changes under aggregation or re-query. */
  id: string;
  kind: SceneItemKind;
  authorityClass: AuthorityClass;
  /** Policy-safe label — omits private identifiers. */
  label: string;
  /** Current truth state — exact from backend. */
  truthState: string;
  /** Graph revision when this item was loaded. */
  graphRevision: number;
  /** Direction (null for non-relation items). */
  direction: SceneItemDirection;
  /** Source endpoint ID for relation items. */
  sourceEndpointId: string | null;
  /** Target endpoint ID for relation items. */
  targetEndpointId: string | null;
  /** Evidence count. */
  evidenceCount: number;
  /** Summary of evidence — policy-safe. */
  evidenceSummary: string | null;
  /** Provenance — policy-safe. */
  provenance: SceneItemProvenance;
  /** Validity interval. */
  validity: SceneItemValidity;
  /** Whether this item is selected in the scene. */
  isSelected: boolean;
  /** Whether this item is the current focus. */
  isFocused: boolean;
  /** Whether this item is in the current path. */
  isInPath: boolean;
  /** Whether this item has a pending (unconfirmed) state. */
  isPending: boolean;
  /** Whether this item has an error state. */
  hasError: boolean;
}

// ─── Scene actions ────────────────────────────────────────────────────────────

/**
 * The kind of a scene action.
 *
 * Only authorized actions appear in a SemanticScene; unauthorized actions
 * are omitted entirely rather than shown as disabled.
 */
export type SceneActionKind =
  | 'select'
  | 'expand'
  | 'inspect'
  | 'path'
  | 'correct'
  | 'merge'
  | 'split'
  | 'relate'
  | 'forget'
  | 'restore'
  | 'delete'
  | 'fit'
  | 'back'
  | 'forward';

/**
 * One authorized action for a scene item.
 *
 * isEnabled=false means the capability is present but not currently operable
 * (e.g. nothing to undo). isDangerous=true triggers a confirmation preview.
 * requiresPreview=true means the UI must show a diff before committing.
 */
export interface SemanticSceneAction {
  /** ID matching the item this action targets. */
  targetItemId: string;
  kind: SceneActionKind;
  /** Policy-safe label. */
  label: string;
  /** false = capability not authorized for current state */
  isEnabled: boolean;
  isDangerous: boolean;
  requiresPreview: boolean;
}

// ─── Visual tokens ────────────────────────────────────────────────────────────

/**
 * Shape token for a visual item.
 * 'line' is used for edge/relation items.
 */
export type SceneTokenShape =
  | 'circle'
  | 'rect'
  | 'diamond'
  | 'hexagon'
  | 'triangle'
  | 'line'; // for edges

/**
 * Visual token for one scene item.
 *
 * Visual tokens are renderer hints — they do NOT carry semantic meaning.
 * Colors reference CSS custom properties; icons are identifiers, never
 * rendered glyphs or unicode characters.
 */
export interface SemanticVisualToken {
  /** Item ID this token applies to. */
  itemId: string;
  shape: SceneTokenShape;
  /** CSS custom property name for color, e.g. "--color-entity". */
  colorToken: string;
  /** Icon identifier — never a unicode char or rendered glyph. */
  iconId: string | null;
  /** Text label to show in the map (may differ from item.label for space). */
  displayLabel: string;
  /** Whether to show the label at current LOD. */
  showLabel: boolean;
}

// ─── Layout hints ─────────────────────────────────────────────────────────────

/**
 * Named layout strategy for the scene.
 *
 * The strategy is a hint to the renderer — the renderer may ignore or
 * adapt it, but must not change the semantic meaning of the scene.
 */
export type LayoutStrategy =
  | 'search-treemap-grid'
  | 'ego-radial-rings'
  | 'path-layered-dag'
  | 'temporal-lanes'
  | 'goal-source-grouped-lane';

/**
 * Deterministic layout hint for the scene.
 *
 * seed is a query-hash/revision derived value that ensures the same
 * input always produces the same layout (equal input → equal scene hash).
 */
export interface SemanticLayoutHint {
  /**
   * Query-hash/revision deterministic seed — ensures same input → same layout.
   */
  seed: number;
  strategy: LayoutStrategy;
  /** Primary item ID (e.g. ego node for radial, start for path). */
  primaryItemId: string | null;
  /** Maximum depth/rings for radial/DAG strategies. */
  maxDepth: number | null;
}

// ─── Diagnostics ─────────────────────────────────────────────────────────────

/** Severity level for a scene diagnostic. */
export type SceneDiagnosticLevel = 'info' | 'warning' | 'error';

/**
 * One diagnostic message about scene construction.
 *
 * Diagnostics are informational — they do NOT alter the scene content.
 * Messages never contain private content.
 */
export interface SemanticSceneDiagnostic {
  level: SceneDiagnosticLevel;
  /** Human-readable message — never contains private content. */
  message: string;
  /** Optional reference to the item that caused this diagnostic. */
  relatedItemId: string | null;
}

// ─── Complete scene ───────────────────────────────────────────────────────────

/**
 * The complete canonical renderer-neutral semantic scene.
 *
 * sceneHash is a deterministic hash of the scene's full content. Two scenes
 * built from identical inputs must produce equal hashes; this is the parity
 * oracle for map/list action/item consistency.
 */
export interface SemanticScene {
  /** Deterministic hash of the complete scene — used for parity checks. */
  sceneHash: string;
  /** Graph revision this scene was built from. */
  graphRevision: number;
  /** All semantic items. */
  items: SemanticSceneItem[];
  /** All authorized actions per item. */
  actions: SemanticSceneAction[];
  /** Visual tokens per item. */
  tokens: SemanticVisualToken[];
  /** Layout hint for this scene. */
  layoutHint: SemanticLayoutHint;
  /** Any diagnostics about scene construction (no private content). */
  diagnostics: SemanticSceneDiagnostic[];
}

// ─── Type guard helpers (pure functions) ─────────────────────────────────────

/**
 * Returns true when the item is a navigation container.
 *
 * Navigation containers are group containers — they are NOT edges. They
 * must never be rendered as graph edges or traversed as relation endpoints.
 */
export function isNavigationContainer(item: SemanticSceneItem): boolean {
  return item.kind === 'navigation-container';
}

/**
 * Returns true when the item is an edge (relation with a non-null direction).
 *
 * An item is an edge when its kind is 'relation' AND it carries a non-null
 * direction. A 'relation' item with a null direction is not a directed edge
 * in the rendered graph.
 */
export function isEdgeItem(item: SemanticSceneItem): boolean {
  return item.kind === 'relation' && item.direction !== null;
}

/**
 * Returns true when the item is a node (not a relation and not a navigation
 * container).
 *
 * Node items represent atomic entities in the graph and are rendered as
 * distinct visual nodes — circles, hexagons, etc. — never as lines.
 */
export function isNodeItem(item: SemanticSceneItem): boolean {
  return item.kind !== 'relation' && item.kind !== 'navigation-container';
}

/**
 * Returns all authorized actions targeting the given itemId.
 *
 * Returns an empty array when no actions exist for the itemId.
 * The returned array is a new array (not a slice of the scene's actions array).
 */
export function getActionsForItem(
  scene: SemanticScene,
  itemId: string,
): SemanticSceneAction[] {
  return scene.actions.filter((action) => action.targetItemId === itemId);
}

/**
 * Returns the visual token for the given itemId, or null if none exists.
 */
export function getTokenForItem(
  scene: SemanticScene,
  itemId: string,
): SemanticVisualToken | null {
  return scene.tokens.find((token) => token.itemId === itemId) ?? null;
}
