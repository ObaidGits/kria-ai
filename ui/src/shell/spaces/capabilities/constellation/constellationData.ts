/**
 * constellationData — the typed read-model slice feeding the Capabilities
 * Constellation lens (3D) and its 2D catalog fallback (task 8.3, Req 7.5).
 *
 * Mirrors the Memory graph's `graphData` (task 6.4) so both lenses share the
 * SAME budgeted-governance shape: capped node/edge model, focus/pin/hide view
 * state, honest loading/error/"showing N of M". The difference is the SOURCE:
 * this store reads the Capabilities catalogs from `capabilityStore` (Tools /
 * Models / Skills / Integrations) and maps them via `buildConstellation`.
 *
 * ── ARCHITECTURE INVARIANT (KRIA is the authoritative orchestrator) ──────────
 * READ / VISUALIZE ONLY. `load()` dispatches the EXISTING capability loaders
 * (dispatch-only, graceful when a service is absent, Req 20.4). focus/pin/hide
 * are VIEW state — nothing is written and NO capability is executed from the
 * lens. Selecting a tool node opens its descriptor in the shared Inspector
 * (legibility, Req 7.2); there is no materialize/write path (unlike the Memory
 * graph), and no prompt→tool shortcut. KRIA remains orchestration authority.
 */
import { createSignal } from "solid-js";
import { capabilityStore } from "../../../../stores";
import {
  applyNodeCap,
  DEFAULT_NODE_CAP,
  pruneEdges,
  type CappedNodes,
  type GraphEdge,
  type GraphModel,
  type GraphNode,
} from "../../memory/graph/graphModel";
import {
  buildConstellation,
  type ConstellationModel,
  type ConstellationNodeMeta,
} from "./constellationModel";

// ─── Signals ─────────────────────────────────────────────────────────────────

const [nodes, setNodes] = createSignal<GraphNode[]>([]);
const [edges, setEdges] = createSignal<GraphEdge[]>([]);
const [meta, setMeta] = createSignal<Map<string, ConstellationNodeMeta>>(new Map());
const [capped, setCapped] = createSignal<CappedNodes | null>(null);
const [loading, setLoading] = createSignal(false);
const [error, setError] = createSignal<string | null>(null);
const [focusedId, setFocusedId] = createSignal<string | null>(null);
/** View-state: pinned node ids (layout must not move these). */
const [pinned, setPinned] = createSignal<ReadonlySet<string>>(new Set());
/** View-state: hidden node ids (removed from the rendered set). */
const [hidden, setHidden] = createSignal<ReadonlySet<string>>(new Set());

// ─── Build from the current capabilityStore catalogs ─────────────────────────

/** Rebuild the graph from whatever the capabilityStore currently holds. */
function rebuild(cap: number = DEFAULT_NODE_CAP): void {
  const model = buildConstellation({
    capabilities: capabilityStore.capabilities(),
    models: capabilityStore.models(),
    providers: capabilityStore.providers(),
    skills: capabilityStore.skills(),
    integrations: capabilityStore.integrations(),
  });
  const capResult = applyNodeCap(model.nodes, cap);
  const shownIds = new Set(capResult.shown.map((n) => n.id));
  setNodes(capResult.shown);
  setEdges(pruneEdges(model.edges, shownIds));
  setMeta(model.meta);
  setCapped(capResult);
}

/**
 * Load the catalogs the constellation needs, then build the graph. Each loader
 * degrades independently (Req 20.4); the constellation shows whatever is
 * available. Honest loading/error state around the whole load.
 */
async function load(cap: number = DEFAULT_NODE_CAP): Promise<void> {
  setLoading(true);
  setError(null);
  setFocusedId(null);

  const results = await Promise.all([
    capabilityStore.loadTools(),
    capabilityStore.loadModels(),
    capabilityStore.loadSkills(),
    capabilityStore.loadIntegrations(),
  ]);

  rebuild(cap);
  setLoading(false);

  // If EVERY source failed, surface an honest error (empty graph otherwise).
  if (results.every((r) => !r.ok) && nodes().length === 0) {
    const first = results.find((r) => !r.ok);
    setError(first && !first.ok ? first.message : "The capability catalog is unavailable.");
  }
}

// ─── View-state interactions (focus / pin / hide) ────────────────────────────

/** Focus a node (view state only — no backend expand, unlike the Memory graph). */
function focus(id: string): void {
  setFocusedId(id);
}

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

/** Metadata for a node id (kind, inspector target, detail). */
function metaFor(id: string): ConstellationNodeMeta | undefined {
  return meta().get(id);
}

/** Reset all state (called on lens unload / Space exit). */
function reset(): void {
  setNodes([]);
  setEdges([]);
  setMeta(new Map());
  setCapped(null);
  setError(null);
  setLoading(false);
  setFocusedId(null);
  setPinned(new Set<string>());
  setHidden(new Set<string>());
}

/** Seed a constellation model directly (stories / tests). */
function seed(model: ConstellationModel, cap: number = DEFAULT_NODE_CAP): void {
  const capResult = applyNodeCap(model.nodes, cap);
  const shownIds = new Set(capResult.shown.map((n) => n.id));
  setNodes(capResult.shown);
  setEdges(pruneEdges(model.edges, shownIds));
  setMeta(model.meta);
  setCapped(capResult);
}

export const constellationData = {
  nodes,
  edges,
  meta,
  capped,
  loading,
  error,
  focusedId,
  pinned,
  hidden,

  load,
  rebuild,
  focus,
  togglePin,
  hide,
  unhideAll,
  visibleNodes,
  visibleEdges,
  model,
  metaFor,
  reset,
  seed,
} as const;
