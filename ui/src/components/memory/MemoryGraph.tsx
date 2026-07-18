// Production knowledge-graph engine (memory-upgrade P4).
//
// A real force-directed graph over the MemorySystem graph APIs: force
// simulation, community coloring, degree-scaled nodes, neighbor expansion,
// link-prediction overlay, node drag, zoom/pan, selection + inspection, and
// relationship creation. Every node/edge is real backend data — no mocks.

import { Component, For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { memoryStore, LIVE_EVENT, type LinkPrediction } from "../../stores/memory";
import "../../styles/memory.css";

interface SimNode {
  id: string;
  label: string;
  degree: number;
  community: number;
  x: number;
  y: number;
  vx: number;
  vy: number;
  pinned: boolean;
}

interface SimEdge {
  source: string;
  target: string;
  type?: string;
  predicted?: boolean;
}

const WIDTH = 960;
const HEIGHT = 600;
const COMMUNITY_COLORS = [
  "#3b82f6", "#22c55e", "#f59e0b", "#ec4899", "#a855f7",
  "#14b8a6", "#ef4444", "#eab308", "#06b6d4", "#8b5cf6",
];

const MemoryGraph: Component = () => {
  const [nodes, setNodes] = createSignal<SimNode[]>([]);
  const [edges, setEdges] = createSignal<SimEdge[]>([]);
  const [selected, setSelected] = createSignal<string | null>(null);
  const [predictions, setPredictions] = createSignal<LinkPrediction[]>([]);
  const [zoom, setZoom] = createSignal(1);
  const [pan, setPan] = createSignal({ x: 0, y: 0 });
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [query, setQuery] = createSignal("");
  const [relType, setRelType] = createSignal("related_to");
  const [showPredicted, setShowPredicted] = createSignal(true);

  let raf = 0;
  let alpha = 0;
  let dragId: string | null = null;
  let panning = false;
  let panStart = { x: 0, y: 0, px: 0, py: 0 };

  const maxDegree = () => nodes().reduce((m, n) => Math.max(m, n.degree), 1);
  const nodeById = (id: string) => nodes().find((n) => n.id === id);

  // ── Force simulation (repulsion + spring + centering) ──
  function tick() {
    const ns = nodes();
    if (ns.length === 0 || alpha < 0.005) {
      raf = 0;
      return;
    }
    const es = edges();
    const cx = WIDTH / 2;
    const cy = HEIGHT / 2;
    // repulsion
    for (let i = 0; i < ns.length; i++) {
      const a = ns[i];
      if (a.pinned) continue;
      for (let j = i + 1; j < ns.length; j++) {
        const b = ns[j];
        let dx = a.x - b.x;
        let dy = a.y - b.y;
        let d2 = dx * dx + dy * dy;
        if (d2 < 1) d2 = 1;
        const f = (4000 * alpha) / d2;
        const dist = Math.sqrt(d2);
        const fx = (dx / dist) * f;
        const fy = (dy / dist) * f;
        a.vx += fx;
        a.vy += fy;
        b.vx -= fx;
        b.vy -= fy;
      }
      // centering
      a.vx += (cx - a.x) * 0.002 * alpha;
      a.vy += (cy - a.y) * 0.002 * alpha;
    }
    // springs
    for (const e of es) {
      const a = nodeById(e.source);
      const b = nodeById(e.target);
      if (!a || !b) continue;
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const dist = Math.sqrt(dx * dx + dy * dy) || 1;
      const target = 90;
      const f = (dist - target) * 0.02 * alpha;
      const fx = (dx / dist) * f;
      const fy = (dy / dist) * f;
      if (!a.pinned) { a.vx += fx; a.vy += fy; }
      if (!b.pinned) { b.vx -= fx; b.vy -= fy; }
    }
    // integrate
    for (const n of ns) {
      if (n.pinned) { n.vx = 0; n.vy = 0; continue; }
      n.vx *= 0.85;
      n.vy *= 0.85;
      n.x += Math.max(-15, Math.min(15, n.vx));
      n.y += Math.max(-15, Math.min(15, n.vy));
    }
    alpha *= 0.98;
    setNodes([...ns]);
    raf = requestAnimationFrame(tick);
  }

  function reheat() {
    alpha = 1;
    if (!raf) raf = requestAnimationFrame(tick);
  }

  async function loadGraph() {
    setLoading(true);
    setError(null);
    setPredictions([]);
    setSelected(null);
    try {
      const [centrality, comms] = await Promise.all([
        memoryStore.api.graphCentrality(80),
        memoryStore.api.graphCommunities(),
      ]);
      const communityOf = new Map<string, number>();
      comms.communities.forEach((members, idx) => {
        for (const id of members) communityOf.set(id, idx);
      });
      const built: SimNode[] = centrality.nodes.map((c, i) => {
        const angle = (2 * Math.PI * i) / Math.max(1, centrality.nodes.length);
        return {
          id: c.entity,
          label: c.display_name,
          degree: c.degree,
          community: communityOf.get(c.entity) ?? -1,
          x: WIDTH / 2 + 200 * Math.cos(angle),
          y: HEIGHT / 2 + 200 * Math.sin(angle),
          vx: 0,
          vy: 0,
          pinned: false,
        };
      });
      setNodes(built);
      setEdges([]);
      reheat();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  async function select(id: string) {
    setSelected(id);
    try {
      const [rels, preds] = await Promise.all([
        memoryStore.api.graphRelationships(id),
        memoryStore.api.graphPredictLinks(id, 8),
      ]);
      setPredictions(preds.predictions);
      const known = new Set(nodes().map((n) => n.id));
      const origin = nodeById(id);
      const newNodes: SimNode[] = [];
      const addNode = (nid: string, label: string) => {
        if (known.has(nid)) return;
        known.add(nid);
        newNodes.push({
          id: nid,
          label,
          degree: 1,
          community: -1,
          x: (origin?.x ?? WIDTH / 2) + (Math.random() - 0.5) * 120,
          y: (origin?.y ?? HEIGHT / 2) + (Math.random() - 0.5) * 120,
          vx: 0,
          vy: 0,
          pinned: false,
        });
      };
      const relEdges: SimEdge[] = rels.map((r) => {
        addNode(r.source_id, r.source_id.slice(0, 8));
        addNode(r.target_id, r.target_id.slice(0, 8));
        return { source: r.source_id, target: r.target_id, type: r.rel_type };
      });
      const predEdges: SimEdge[] = preds.predictions.map((p) => {
        addNode(p.target, p.display_name);
        return { source: id, target: p.target, type: "predicted", predicted: true };
      });
      if (newNodes.length) setNodes([...nodes(), ...newNodes]);
      setEdges((prev) => {
        const seen = new Set(prev.map((e) => `${e.source}->${e.target}`));
        const merged = [...prev];
        for (const e of [...relEdges, ...predEdges]) {
          const k = `${e.source}->${e.target}`;
          if (!seen.has(k)) { seen.add(k); merged.push(e); }
        }
        return merged;
      });
      reheat();
    } catch (e) {
      setError(String(e));
    }
  }

  async function runSearch() {
    const q = query().trim();
    if (!q) return loadGraph();
    setLoading(true);
    setError(null);
    try {
      const ents = await memoryStore.api.graphSearch(q);
      setNodes(
        ents.map((e, i) => {
          const angle = (2 * Math.PI * i) / Math.max(1, ents.length);
          return {
            id: e.id,
            label: e.display_name,
            degree: 1,
            community: -1,
            x: WIDTH / 2 + 180 * Math.cos(angle),
            y: HEIGHT / 2 + 180 * Math.sin(angle),
            vx: 0,
            vy: 0,
            pinned: false,
          };
        }),
      );
      setEdges([]);
      reheat();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  async function materializePrediction(targetId: string) {
    const src = selected();
    if (!src) return;
    try {
      await memoryStore.api.graphCreateRelationship(src, targetId, relType(), 0.7);
      // Promote the predicted edge to a real edge.
      setEdges((prev) =>
        prev.map((e) =>
          e.source === src && e.target === targetId ? { ...e, predicted: false, type: relType() } : e,
        ),
      );
      setPredictions((prev) => prev.filter((p) => p.target !== targetId));
    } catch (e) {
      setError(String(e));
    }
  }

  // ── Pointer handling: node drag vs canvas pan ──
  function svgPoint(e: MouseEvent): { x: number; y: number } {
    const svg = e.currentTarget as SVGSVGElement;
    const rect = svg.getBoundingClientRect();
    const sx = (e.clientX - rect.left) / rect.width * WIDTH;
    const sy = (e.clientY - rect.top) / rect.height * HEIGHT;
    return { x: (sx - pan().x) / zoom(), y: (sy - pan().y) / zoom() };
  }

  // Live: reload the graph when relationship/entity/memory changes arrive,
  // unless the user is actively dragging (avoid yanking the layout).
  const onLive = (e: Event) => {
    const kinds = (e as CustomEvent<{ kinds: string[] }>).detail?.kinds ?? [];
    if (dragId) return;
    if (kinds.some((k) => ["relationship", "entity", "created", "deleted"].includes(k))) {
      const sel = selected();
      void (async () => {
        await loadGraph();
        if (sel) void select(sel);
      })();
    }
  };

  onMount(() => {
    void loadGraph();
    if (typeof window !== "undefined") window.addEventListener(LIVE_EVENT, onLive);
  });
  onCleanup(() => {
    if (raf) cancelAnimationFrame(raf);
    if (typeof window !== "undefined") window.removeEventListener(LIVE_EVENT, onLive);
  });

  return (
    <div class="mem-graph">
      <div class="mem-graph-toolbar">
        <input
          class="mem-input"
          placeholder="Search entities…"
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
          onKeyDown={(e) => e.key === "Enter" && runSearch()}
        />
        <button class="mem-btn" onClick={runSearch}>Search</button>
        <button class="mem-btn" onClick={loadGraph}>Reset</button>
        <button class="mem-btn" onClick={() => setZoom((z) => Math.min(3, z + 0.2))}>+</button>
        <button class="mem-btn" onClick={() => setZoom((z) => Math.max(0.3, z - 0.2))}>−</button>
        <button class="mem-btn" onClick={() => { setZoom(1); setPan({ x: 0, y: 0 }); }}>Fit</button>
        <button class="mem-btn" onClick={() => setShowPredicted((v) => !v)}>
          {showPredicted() ? "Hide predicted" : "Show predicted"}
        </button>
        <button class="mem-btn" onClick={reheat}>Re-layout</button>
        <span class="mem-muted">{nodes().length} nodes · {edges().length} edges</span>
      </div>
      <Show when={error()}><div class="mem-error">{error()}</div></Show>
      <Show when={loading()}><div class="mem-muted mem-graph-loading">Loading graph…</div></Show>
      <Show when={!loading() && nodes().length === 0}>
        <div class="mem-empty">No entities yet. Run entity extraction (Cognition tab), then reset.</div>
      </Show>

      <div class="mem-graph-stage">
        <svg
          class="mem-graph-svg"
          viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
          onWheel={(e) => { e.preventDefault(); setZoom((z) => Math.max(0.3, Math.min(3, z - Math.sign(e.deltaY) * 0.1))); }}
          onMouseDown={(e) => { panning = true; panStart = { x: e.clientX, y: e.clientY, px: pan().x, py: pan().y }; }}
          onMouseUp={() => { panning = false; dragId = null; }}
          onMouseLeave={() => { panning = false; dragId = null; }}
          onMouseMove={(e) => {
            if (dragId) {
              const p = svgPoint(e);
              setNodes(nodes().map((n) => (n.id === dragId ? { ...n, x: p.x, y: p.y, pinned: true } : n)));
              return;
            }
            if (panning) {
              setPan({ x: panStart.px + (e.clientX - panStart.x), y: panStart.py + (e.clientY - panStart.y) });
            }
          }}
        >
          <g transform={`translate(${pan().x} ${pan().y}) scale(${zoom()})`}>
            <For each={edges()}>
              {(edge) => {
                if (edge.predicted && !showPredicted()) return null;
                const s = nodeById(edge.source);
                const t = nodeById(edge.target);
                return (
                  <Show when={s && t}>
                    <line
                      x1={s!.x} y1={s!.y} x2={t!.x} y2={t!.y}
                      stroke={edge.predicted ? "#a855f7" : "#3f4a5a"}
                      stroke-width={edge.predicted ? 1 : 1.5}
                      stroke-dasharray={edge.predicted ? "4 4" : undefined}
                    />
                  </Show>
                );
              }}
            </For>
            <For each={nodes()}>
              {(node) => (
                <g
                  transform={`translate(${node.x} ${node.y})`}
                  style={{ cursor: "pointer" }}
                  onMouseDown={(e) => { e.stopPropagation(); dragId = node.id; }}
                  onClick={(e) => { e.stopPropagation(); if (dragId === null) void select(node.id); }}
                >
                  <circle
                    r={selected() === node.id ? 16 : 8 + Math.min(12, (node.degree / maxDegree()) * 12)}
                    fill={node.community >= 0 ? COMMUNITY_COLORS[node.community % COMMUNITY_COLORS.length] : "#64748b"}
                    stroke={selected() === node.id ? "#fbbf24" : node.pinned ? "#22c55e" : "#111827"}
                    stroke-width={selected() === node.id ? 3 : node.pinned ? 2 : 1}
                  />
                  <text x="0" y="-16" text-anchor="middle" class="mem-graph-label">{node.label}</text>
                </g>
              )}
            </For>
          </g>
        </svg>

        <Show when={selected()}>
          <aside class="mem-graph-inspector">
            <h4>{nodeById(selected()!)?.label ?? selected()}</h4>
            <div class="mem-muted">degree {nodeById(selected()!)?.degree ?? 0} · community {nodeById(selected()!)?.community ?? "—"}</div>
            <div class="mem-graph-inspector-actions">
              <button class="mem-btn" onClick={() => setNodes(nodes().map((n) => n.id === selected() ? { ...n, pinned: !n.pinned } : n))}>
                {nodeById(selected()!)?.pinned ? "Unpin" : "Pin"}
              </button>
              <button class="mem-btn" onClick={() => { setNodes(nodes().filter((n) => n.id !== selected())); setSelected(null); }}>
                Hide
              </button>
            </div>
            <div class="mem-graph-rel-form">
              <input class="mem-input" value={relType()} onInput={(e) => setRelType(e.currentTarget.value)} placeholder="relationship type" />
            </div>
            <h5>Predicted links</h5>
            <Show when={predictions().length > 0} fallback={<div class="mem-empty">None</div>}>
              <For each={predictions()}>
                {(p) => (
                  <div class="mem-graph-pred">
                    <span>{p.display_name}</span>
                    <span class="mem-muted">score {p.score.toFixed(2)}</span>
                    <button class="mem-btn" onClick={() => materializePrediction(p.target)}>Create</button>
                  </div>
                )}
              </For>
            </Show>
          </aside>
        </Show>
      </div>
    </div>
  );
};

export default MemoryGraph;
