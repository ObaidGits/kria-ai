/**
 * graphModel — pure, framework/GL-free logic for the Knowledge Graph lens
 * (task 6.4, Req 5.4 / 16.3).
 *
 * This module holds EVERY non-GL decision the 3D lens (and its 2D fallback)
 * depend on, so the maths is unit-testable under jsdom (WebGL itself cannot run
 * there — the surrounding logic can and must):
 *   • data → graph mapping (memory_graph_* payloads → typed nodes/edges),
 *   • node/element caps ("showing N of M" — §5.4 hard rule),
 *   • centrality → node size, community → color-token index,
 *   • LOD tiering + label-set selection (labels only for focused/near set),
 *   • frustum/bounds culling selection,
 *   • the auto-degrade decision (heavy load / reduced-motion / no-WebGL).
 *
 * ARCHITECTURE (KRIA runtime authority): this is a presentation read-model. It
 * only SHAPES memory-graph data the runtime already returns; it never mutates
 * memory or creates edges. "Predicted links" reflect backend predictions —
 * materializing one is a backend relationship write (the lens dispatches the
 * existing command; this module just models the view).
 *
 * Colors are referenced by DESIGN-TOKEN CSS-variable NAME (never a raw hex), so
 * the GL layer resolves them from the live theme at runtime (zero raw color →
 * token-lint clean, dark/light parity). Accent is reserved for SELECTION only
 * (§5.4), so the community palette deliberately excludes the accent token.
 */

// ─── Typed graph model ──────────────────────────────────────────────────────

/** A knowledge-graph node in the view read-model. */
export interface GraphNode {
  /** Stable entity id (backend canonical id). */
  id: string;
  /** Human-readable label (rendered as text; never HTML). */
  label: string;
  /**
   * Community index from community detection, or -1 when the node belongs to no
   * detected community. Drives the community color (§5.4 community color).
   */
  community: number;
  /** Centrality proxy (degree). Drives node size (§5.4 centrality size). */
  centrality: number;
}

/** A knowledge-graph edge in the view read-model. */
export interface GraphEdge {
  source: string;
  target: string;
  /** Relationship type label, when known. */
  relType?: string;
  /**
   * True for a BACKEND-PREDICTED link (not yet a real relationship). The lens
   * renders these distinctly and can "materialize" one via the existing
   * relationship-create command (backend write — never invented here).
   */
  predicted?: boolean;
}

/** A backend link prediction surfaced for a focused node (§5.4 predicted-link). */
export interface PredictedLink {
  target: string;
  label: string;
  score: number;
}

/** The full, uncapped graph the lens works from. */
export interface GraphModel {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

// ─── Raw backend payload shapes (memory_graph_* commands) ────────────────────
// Mirror the façade payloads (snake_case) consumed via the bridge. Kept minimal
// and local so the mapping is self-contained and testable.

export interface CentralityNodeLike {
  entity: string;
  display_name: string;
  degree: number;
}

export interface RelationshipLike {
  source_id: string;
  target_id: string;
  rel_type?: string;
}

export interface PredictionLike {
  target: string;
  display_name: string;
  score: number;
}

// ─── data → graph mapping ────────────────────────────────────────────────────

/**
 * Build a community-index lookup from the `memory_graph_communities` payload
 * (an array of communities, each a list of member entity ids).
 */
export function buildCommunityIndex(communities: readonly (readonly string[])[]): Map<string, number> {
  const index = new Map<string, number>();
  communities.forEach((members, communityId) => {
    for (const entityId of members) {
      // First community wins if an entity appears twice (deterministic).
      if (!index.has(entityId)) index.set(entityId, communityId);
    }
  });
  return index;
}

/**
 * Map centrality nodes + community assignments into typed {@link GraphNode}s.
 * Nodes with no community assignment get community -1 (neutral color).
 */
export function mapCentralityToNodes(
  centrality: readonly CentralityNodeLike[],
  communityIndex: Map<string, number>,
): GraphNode[] {
  return centrality.map((c) => ({
    id: c.entity,
    label: c.display_name,
    community: communityIndex.get(c.entity) ?? -1,
    centrality: Number.isFinite(c.degree) && c.degree > 0 ? c.degree : 0,
  }));
}

/** Map backend relationships into typed real (non-predicted) edges. */
export function mapRelationshipsToEdges(rels: readonly RelationshipLike[]): GraphEdge[] {
  return rels.map((r) => ({
    source: r.source_id,
    target: r.target_id,
    relType: r.rel_type,
    predicted: false,
  }));
}

/** Map backend link predictions into typed {@link PredictedLink}s. */
export function mapPredictions(preds: readonly PredictionLike[]): PredictedLink[] {
  return preds.map((p) => ({ target: p.target, label: p.display_name, score: p.score }));
}

// ─── Node/element cap — "showing N of M" (§5.4 hard rule) ────────────────────

/** Default cap on rendered nodes (top-N by relevance/centrality, §5.4). */
export const DEFAULT_NODE_CAP = 300;

export interface CappedNodes {
  /** The nodes to actually render (top-N by centrality, capped). */
  shown: GraphNode[];
  /** How many are shown. */
  shownCount: number;
  /** Total available before the cap. */
  total: number;
  /** True when the cap actually elided nodes. */
  capped: boolean;
  /** Visible, human-readable "showing N of M" label (§5.4). */
  label: string;
}

/**
 * Apply the node cap: keep the top-N nodes by centrality (relevance) and report
 * an honest "showing N of M". Ties break by id for determinism. A cap ≤0 is
 * treated as "no cap".
 */
export function applyNodeCap(nodes: readonly GraphNode[], cap: number = DEFAULT_NODE_CAP): CappedNodes {
  const total = nodes.length;
  const effectiveCap = cap > 0 ? cap : total;
  const sorted = [...nodes].sort((a, b) =>
    b.centrality !== a.centrality ? b.centrality - a.centrality : a.id.localeCompare(b.id),
  );
  const shown = sorted.slice(0, effectiveCap);
  const shownCount = shown.length;
  const capped = shownCount < total;
  return {
    shown,
    shownCount,
    total,
    capped,
    label: capped ? `Showing ${shownCount} of ${total}` : `Showing all ${total}`,
  };
}

/**
 * Keep only edges whose BOTH endpoints survive a node set (e.g. after capping).
 * Prevents dangling edges pointing at elided nodes.
 */
export function pruneEdges(edges: readonly GraphEdge[], nodeIds: ReadonlySet<string>): GraphEdge[] {
  return edges.filter((e) => nodeIds.has(e.source) && nodeIds.has(e.target));
}

// ─── Centrality → node size ──────────────────────────────────────────────────

export interface NodeSizeRange {
  min: number;
  max: number;
}

/** Default node size range (world units) for the 3D scene. */
export const DEFAULT_NODE_SIZE: NodeSizeRange = { min: 0.6, max: 2.4 };

/**
 * Scale a node's size from its centrality, normalized against the current max
 * centrality (§5.4 centrality size). Uses a sqrt curve so a few very-central
 * hubs don't dwarf everything. Falls back to the min size when max is 0.
 */
export function nodeSizeForCentrality(
  centrality: number,
  maxCentrality: number,
  range: NodeSizeRange = DEFAULT_NODE_SIZE,
): number {
  if (maxCentrality <= 0) return range.min;
  const norm = Math.sqrt(Math.max(0, centrality) / maxCentrality); // 0..1
  return range.min + norm * (range.max - range.min);
}

/** Max centrality across a node set (for size normalization). */
export function maxCentrality(nodes: readonly GraphNode[]): number {
  return nodes.reduce((m, n) => Math.max(m, n.centrality), 0);
}

// ─── Community → color-token index ───────────────────────────────────────────

/**
 * Community color palette, referenced by design-token CSS-variable NAME (never
 * raw hex → token-lint clean, theme-aware). Deliberately EXCLUDES the accent
 * token — accent is reserved for selection (§5.4). The GL layer resolves these
 * to concrete colors from the live theme via getComputedStyle.
 */
export const COMMUNITY_COLOR_TOKENS: readonly string[] = [
  "--color-info-solid",
  "--color-success-solid",
  "--color-warning-solid",
  "--color-danger-solid",
  "--color-info-text",
  "--color-success-text",
  "--color-warning-text",
  "--color-danger-text",
];

/** Token for nodes with no community (community -1). */
export const NEUTRAL_NODE_COLOR_TOKEN = "--color-text-secondary";

/** Token for the selected node / focus highlight (accent = selection only). */
export const SELECTION_COLOR_TOKEN = "--color-accent-default";

/** Token for a real relationship edge. */
export const EDGE_COLOR_TOKEN = "--color-border-default";

/** Token for a predicted (not-yet-real) edge. */
export const PREDICTED_EDGE_COLOR_TOKEN = "--color-warning-solid";

/**
 * Resolve a community index to a color-token variable name. -1 (or negative)
 * maps to the neutral token; others wrap around the palette by modulo.
 */
export function communityColorToken(community: number): string {
  if (community < 0) return NEUTRAL_NODE_COLOR_TOKEN;
  return COMMUNITY_COLOR_TOKENS[community % COMMUNITY_COLOR_TOKENS.length];
}

// ─── LOD + label-set selection ───────────────────────────────────────────────

export type LODTier = "near" | "mid" | "far";

export interface LODThresholds {
  /** Distance below which a node is "near" (full detail + eligible for label). */
  near: number;
  /** Distance below which a node is "mid" (simplified); beyond → "far". */
  mid: number;
}

export const DEFAULT_LOD: LODThresholds = { near: 18, mid: 40 };

/** Classify a node's LOD tier from its camera distance (§5.4 LOD). */
export function computeLOD(distance: number, thresholds: LODThresholds = DEFAULT_LOD): LODTier {
  if (distance <= thresholds.near) return "near";
  if (distance <= thresholds.mid) return "mid";
  return "far";
}

export interface LabelCandidate {
  id: string;
  distance: number;
}

/** Default cap on simultaneously rendered labels (perf + legibility). */
export const DEFAULT_MAX_LABELS = 20;

/**
 * Select which node ids show a text label (§5.4: labels only for the focused
 * and near set). The focused node is ALWAYS labelled; otherwise the nearest
 * nodes within the "near" LOD band are labelled up to `maxLabels`.
 */
export function selectLabelSet(
  candidates: readonly LabelCandidate[],
  focusedId: string | null,
  thresholds: LODThresholds = DEFAULT_LOD,
  maxLabels: number = DEFAULT_MAX_LABELS,
): Set<string> {
  const labels = new Set<string>();
  if (focusedId) labels.add(focusedId);

  const near = candidates
    .filter((c) => c.id !== focusedId && c.distance <= thresholds.near)
    .sort((a, b) => a.distance - b.distance);

  for (const c of near) {
    if (labels.size >= maxLabels) break;
    labels.add(c.id);
  }
  return labels;
}

// ─── Frustum / bounds culling ────────────────────────────────────────────────

export interface Vec2 {
  x: number;
  y: number;
}

export interface ViewBounds {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

/** Whether a projected 2D position lies within the (padded) view bounds. */
export function isInViewBounds(pos: Vec2, bounds: ViewBounds, pad = 0): boolean {
  return (
    pos.x >= bounds.minX - pad &&
    pos.x <= bounds.maxX + pad &&
    pos.y >= bounds.minY - pad &&
    pos.y <= bounds.maxY + pad
  );
}

/**
 * Select the ids of nodes visible within the view bounds (frustum cull, §5.4).
 * `projected` maps node id → its projected 2D position. Nodes without a
 * projection are treated as culled.
 */
export function cullNodes(
  nodeIds: readonly string[],
  projected: ReadonlyMap<string, Vec2>,
  bounds: ViewBounds,
  pad = 0,
): Set<string> {
  const visible = new Set<string>();
  for (const id of nodeIds) {
    const pos = projected.get(id);
    if (pos && isInViewBounds(pos, bounds, pad)) visible.add(id);
  }
  return visible;
}

// ─── Auto-degrade decision (§5.4) ────────────────────────────────────────────

export interface DegradeInputs {
  /** WebGL present on the device. */
  hasWebGL: boolean;
  /** User prefers reduced motion. */
  reducedMotion: boolean;
  /**
   * Recent interaction FPS samples (most recent last). Sustained low FPS while
   * interacting signals the device can't keep the lens above budget.
   */
  recentFps: readonly number[];
  /** Optional external "heavy model load" signal from the runtime. */
  heavyModelLoad?: boolean;
}

export interface DegradeDecision {
  degrade: boolean;
  reason: string;
}

/** Minimum sustained interaction FPS before we auto-degrade (§5.6 ≥30 or degrade). */
export const DEGRADE_MIN_FPS = 24;
/** How many consecutive low-FPS samples constitute "sustained". */
export const DEGRADE_LOW_FPS_WINDOW = 3;

/**
 * Decide whether the 3D lens must auto-degrade to its 2D representation
 * (§5.4 hard rule). Degrades when WebGL is absent, reduced-motion is on, the
 * runtime reports heavy model load, or interaction FPS is sustained below the
 * floor. Pure → unit-testable; the controller acts on the boolean.
 */
export function evaluateDegrade(inputs: DegradeInputs): DegradeDecision {
  if (!inputs.hasWebGL) return { degrade: true, reason: "no WebGL: 2D fallback" };
  if (inputs.reducedMotion) return { degrade: true, reason: "reduced-motion: 2D static fallback" };
  if (inputs.heavyModelLoad) return { degrade: true, reason: "heavy model load: 2D fallback" };

  const recent = inputs.recentFps.slice(-DEGRADE_LOW_FPS_WINDOW);
  if (
    recent.length >= DEGRADE_LOW_FPS_WINDOW &&
    recent.every((fps) => fps > 0 && fps < DEGRADE_MIN_FPS)
  ) {
    return {
      degrade: true,
      reason: `sustained low FPS (<${DEGRADE_MIN_FPS}): 2D fallback`,
    };
  }
  return { degrade: false, reason: "within budget: 3D retained" };
}
