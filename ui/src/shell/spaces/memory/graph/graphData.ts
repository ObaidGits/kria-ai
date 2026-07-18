/**
 * graphData — the typed knowledge-graph read-model slice (task 6.4, Req 5.4).
 *
 * A minimal, feature-scoped reactive store that feeds the Knowledge Graph lens
 * (3D) and its 2D fallback from the EXISTING `memory_graph_*` Tauri commands via
 * the graceful bridge. It holds the capped node/edge model, per-focus predicted
 * links, and honest loading/error/"showing N of M" state.
 *
 * ARCHITECTURE (KRIA runtime authority): pure read-model + view-state.
 *   • load / expand only READ backend graph data (centrality, communities,
 *     relationships, predictions).
 *   • focus/expand/pin/hide are VIEW state (pinned/hidden id sets) — they never
 *     mutate memory.
 *   • materializePrediction dispatches the EXISTING
 *     `memory_graph_create_relationship` command (the backend performs the
 *     write); the view then REFLECTS the now-real relationship. The lens never
 *     fabricates an edge on its own.
 *
 * No orchestration, no prompt→tool. All shaping/caps/colors live in graphModel.
 */
import { createSignal } from "solid-js";
import { bridgeInvoke } from "../../../../bridge/invoke";
import {
  applyNodeCap,
  buildCommunityIndex,
  DEFAULT_NODE_CAP,
  mapCentralityToNodes,
  mapPredictions,
  mapRelationshipsToEdges,
  pruneEdges,
  type CappedNodes,
  type CentralityNodeLike,
  type GraphEdge,
  type GraphModel,
  type GraphNode,
  type PredictedLink,
  type PredictionLike,
  type RelationshipLike,
} from "./graphModel";

// ─── Raw command payloads ────────────────────────────────────────────────────

interface CentralityPayload {
  nodes: CentralityNodeLike[];
  count: number;
}
interface CommunitiesPayload {
  communities: string[][];
  count: number;
}
interface PredictLinksPayload {
  predictions: PredictionLike[];
  count: number;
}

// ─── Signals ─────────────────────────────────────────────────────────────────

const [nodes, setNodes] = createSignal<GraphNode[]>([]);
const [edges, setEdges] = createSignal<GraphEdge[]>([]);
const [predicted, setPredicted] = createSignal<PredictedLink[]>([]);
const [capped, setCapped] = createSignal<CappedNodes | null>(null);
const [loading, setLoading] = createSignal(false);
const [error, setError] = createSignal<string | null>(null);
const [focusedId, setFocusedId] = createSignal<string | null>(null);
/** View-state: pinned node ids (layout must not move these). */
const [pinned, setPinned] = createSignal<ReadonlySet<string>>(new Set());
/** View-state: hidden node ids (removed from the rendered set). */
const [hidden, setHidden] = createSignal<ReadonlySet<string>>(new Set());
let loadGeneration = 0;

// ─── Load / expand (READ-only) ───────────────────────────────────────────────

function isCurrentLoad(generation: number): boolean {
  return generation === loadGeneration;
}


function failMessage(message: string, command: string): string {
  return message?.trim() ? message : `Graph command '${command}' failed`;
}

/**
 * Load the top-N graph view (centrality + communities) and apply the node cap.
 * Honest loading/error state; on failure the model is left empty with a message.
 */
async function load(cap: number = DEFAULT_NODE_CAP): Promise<void> {
  const generation = ++loadGeneration;
  setLoading(true);
  setError(null);
  setPredicted([]);
  setFocusedId(null);

  const [centralityRes, communitiesRes] = await Promise.all([
    bridgeInvoke<CentralityPayload>("memory_graph_centrality", { limit: cap }),
    bridgeInvoke<CommunitiesPayload>("memory_graph_communities"),
  ]);

  if (!isCurrentLoad(generation)) return;
  if (!centralityRes.ok) {
    setLoading(false);
    setError(failMessage(centralityRes.message, "memory_graph_centrality"));
    setNodes([]);
    setEdges([]);
    setCapped(null);
    return;
  }

  const communities = communitiesRes.ok ? communitiesRes.data.communities : [];
  const communityIndex = buildCommunityIndex(communities);
  const allNodes = mapCentralityToNodes(centralityRes.data.nodes ?? [], communityIndex);
  const capResult = applyNodeCap(allNodes, cap);

  setNodes(capResult.shown);
  setCapped(capResult);
  setEdges([]);
  setLoading(false);
}

/**
 * Expand a focused node: fetch its relationships + predicted links (READ), add
 * any newly-referenced nodes, and merge de-duplicated edges. Focus + expand are
 * §5.4 interactions expressed as view state; nothing is written to memory.
 */
async function expand(entityId: string): Promise<void> {
  setFocusedId(entityId);
  const [relsRes, predsRes] = await Promise.all([
    bridgeInvoke<RelationshipLike[]>("memory_graph_relationships", { entityId }),
    bridgeInvoke<PredictLinksPayload>("memory_graph_predict_links", { entityId, limit: 8 }),
  ]);

  if (relsRes.ok) {
    const known = new Set(nodes().map((n) => n.id));
    const additions: GraphNode[] = [];
    for (const r of relsRes.data) {
      for (const [id] of [[r.source_id], [r.target_id]] as const) {
        if (!known.has(id)) {
          known.add(id);
          additions.push({ id, label: id.slice(0, 8), community: -1, centrality: 1 });
        }
      }
    }
    if (additions.length) setNodes((prev) => [...prev, ...additions]);
    const relEdges = mapRelationshipsToEdges(relsRes.data);
    setEdges((prev) => mergeEdges(prev, relEdges));
  }

  if (predsRes.ok) {
    const preds = mapPredictions(predsRes.data.predictions ?? []);
    setPredicted(preds);
    // Predicted edges are rendered distinctly from `focusedId` → each target.
    const predEdges: GraphEdge[] = preds.map((p) => ({
      source: entityId,
      target: p.target,
      relType: "predicted",
      predicted: true,
    }));
    setEdges((prev) => mergeEdges(prev, predEdges));
  }
}

function mergeEdges(existing: readonly GraphEdge[], incoming: readonly GraphEdge[]): GraphEdge[] {
  const seen = new Set(existing.map((e) => `${e.source}->${e.target}`));
  const merged = [...existing];
  for (const e of incoming) {
    const key = `${e.source}->${e.target}`;
    if (!seen.has(key)) {
      seen.add(key);
      merged.push(e);
    }
  }
  return merged;
}

// ─── Materialize a predicted link (backend WRITE via existing command) ───────

/**
 * Materialize a predicted link into a real relationship (§5.4 predicted-link
 * materialize). Dispatches the EXISTING `memory_graph_create_relationship`
 * command — the BACKEND performs the write. On success the view REFLECTS it by
 * promoting the predicted edge to a real edge and dropping the prediction.
 */
async function materializePrediction(
  targetId: string,
  relType = "related_to",
  strength = 0.7,
): Promise<{ ok: true } | { ok: false; message: string }> {
  const source = focusedId();
  if (!source) return { ok: false, message: "No focused node to link from" };
  const res = await bridgeInvoke<string>("memory_graph_create_relationship", {
    sourceId: source,
    targetId,
    relType,
    strength,
  });
  if (!res.ok) return { ok: false, message: failMessage(res.message, "memory_graph_create_relationship") };

  setEdges((prev) =>
    prev.map((e) =>
      e.source === source && e.target === targetId
        ? { ...e, predicted: false, relType }
        : e,
    ),
  );
  setPredicted((prev) => prev.filter((p) => p.target !== targetId));
  return { ok: true };
}

// ─── View-state interactions (pin / hide) ────────────────────────────────────

function togglePin(id: string): void {
  setPinned((prev) => {
    const next = new Set<string>(prev);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return next;
  });
}

function hide(id: string): void {
  setHidden((prev) => {
    const next = new Set<string>(prev);
    next.add(id);
    return next;
  });
  if (focusedId() === id) setFocusedId(null);
}

function unhideAll(): void {
  setHidden(new Set<string>());
}

/** The rendered node set (model minus hidden). */
function visibleNodes(): GraphNode[] {
  const h = hidden();
  return nodes().filter((n) => !h.has(n.id));
}

/** The rendered edge set (pruned to visible nodes). */
function visibleEdges(): GraphEdge[] {
  const visibleIds = new Set(visibleNodes().map((n) => n.id));
  return pruneEdges(edges(), visibleIds);
}

/** Snapshot of the current model (visible nodes + edges). */
function model(): GraphModel {
  return { nodes: visibleNodes(), edges: visibleEdges() };
}

/** Reset all state (called on lens unload / Space exit). */
function reset(): void {
  loadGeneration += 1;
  setNodes([]);
  setEdges([]);
  setPredicted([]);
  setCapped(null);
  setError(null);
  setLoading(false);
  setFocusedId(null);
  setPinned(new Set<string>());
  setHidden(new Set<string>());
}

/** Seed nodes/edges directly (stories / tests / fallback rendering). */
function seed(model: GraphModel, capResult?: CappedNodes): void {
  setNodes(model.nodes);
  setEdges(model.edges);
  if (capResult) setCapped(capResult);
}

export const graphData = {
  nodes,
  edges,
  predicted,
  capped,
  loading,
  error,
  focusedId,
  pinned,
  hidden,

  load,
  expand,
  materializePrediction,
  togglePin,
  hide,
  unhideAll,
  visibleNodes,
  visibleEdges,
  model,
  reset,
  seed,
} as const;
