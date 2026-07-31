/**
 * GraphCanvas3D — dormant Phase 7 candidate; not mounted by shipped Memory Graph.
 *
 * No active import path reaches this component. A future integration may mount
 * it only after MGR-030 gates pass; otherwise this file and graph-only support
 * code are deleted. It wires together three isolated pieces:
 *   • the layout Web Worker (ngraph.forcelayout, off-thread, stops on settle),
 *   • the Three.js GraphScene (instanced nodes/edges, LOD labels, damped orbit),
 *   • the LensController (freeze-on-idle, resume-on-interaction, unload-on-exit,
 *     reduced-motion still frames).
 *
 * GL and the worker are DYNAMICALLY imported inside onMount so the module graph
 * (and jsdom tests / the 2D path) never pull in three or the worker. onCleanup
 * tears everything down if this dormant candidate is exercised.
 *
 * ── F4.9.6 cleanup ─────────────────────────────────────────────────────────
 * The legacy graphData global store was deleted in F4.9.6. This dormant file
 * previously referenced graphData; that reference is replaced with a local
 * stub for the legacy rendering path. The F6.2.1 SemanticInput3D spike path
 * (the primary integration target) is not affected — it never used graphData.
 *
 * ── F6.2.1 spike wiring ────────────────────────────────────────────────────
 * GraphCanvas3D accepts SemanticInput3D props (scene + capabilities + onAction).
 * When these are supplied the component is a PURE CONSUMER of the existing
 * SemanticScene — it does NOT fetch data, traverse the graph, maintain its own
 * policy state, or derive truth. The spike input is wired in but the actual 3D
 * rendering for the semantic scene is deferred to task 6.2.2+.
 *
 * This is a pre-production spike only.
 */
import { For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { Button, EmptyState } from "../../../../kit";
import { degradeToTwoD } from "../../../../platform/renderMode";
import type { GraphScene as GraphSceneType, ScreenLabel } from "./GraphScene";
import type { LensController as LensControllerType } from "./lensController";
import type { LayoutRequest, LayoutResponse } from "./layoutSettle";
import type { SemanticInput3D } from "./graphCanvas3DSpike";
import {
  computeSemanticParitySnapshot,
  sceneToGraphModel,
  computeDeterministicPositions,
} from "./graphCanvas3DSpike";
import type { GraphNode, GraphEdge, PredictedLink, CappedNodes } from "./graphModel";
import "./kria-graph.css";

// ── Legacy graphData stub (F4.9.6) ─────────────────────────────────────────
// The graphData global store was deleted in F4.9.6. This dormant component
// is inaccessible from the shipped product until F6. The legacy rendering path
// below references the old store API; the stubs keep the file compilable.
// The F6.2.1 spike (SemanticInput3D path) is unaffected — it never used graphData.
const graphData = {
  focusedId: () => null as string | null,
  pinned: () => new Set<string>(),
  hidden: () => ({ size: 0 }),
  capped: () => null as CappedNodes | null,
  loading: () => false,
  error: () => null as string | null,
  predicted: () => [] as PredictedLink[],
  visibleNodes: () => [] as GraphNode[],
  visibleEdges: () => [] as GraphEdge[],
  model: () => ({ nodes: [] as GraphNode[], edges: [] as GraphEdge[] }),
  expand: async (_id: string): Promise<void> => { /* dormant */ },
  togglePin: (_id: string): void => { /* dormant */ },
  hide: (_id: string): void => { /* dormant */ },
  unhideAll: (): void => { /* dormant */ },
  materializePrediction: async (_targetId: string): Promise<{ ok: true } | { ok: false; message: string }> =>
    ({ ok: false, message: "dormant" }),
} as const;

export interface GraphCanvas3DProps {
  /** Reduced-motion posture → controller renders discrete still frames only. */
  static?: boolean;
  /**
   * F6.2.1 spike: optional SemanticScene consumer input.
   *
   * When present, GraphCanvas3D acts as a pure SemanticScene consumer
   * (identical contract to Graph2D).  The component does NOT fetch data,
   * maintain policy state, or derive truth from this input.
   *
   * The actual 3D semantic rendering is deferred to task 6.2.2+.
   * For now, receiving this input is sufficient for the parity spike.
   */
  spike?: SemanticInput3D;
}

/**
 * Compute the semantic parity snapshot for the spike input.
 *
 * This is a pure function call — no rendering.  It is exported so that tests
 * can assert that the snapshot equals the one produced by Graph2D for the same
 * SemanticScene.
 *
 * Returns null when no spike input is provided (legacy graphData path only).
 */
export function getSpikeParitySnapshot(props: Pick<GraphCanvas3DProps, 'spike'>) {
  if (!props.spike) return null;
  return computeSemanticParitySnapshot(props.spike.scene);
}

export function GraphCanvas3D(props: GraphCanvas3DProps) {
  let canvas!: HTMLCanvasElement;
  let stage!: HTMLDivElement;
  let scene: GraphSceneType | null = null;
  let controller: LensControllerType | null = null;
  let worker: Worker | null = null;
  let resizeObserver: ResizeObserver | null = null;

  const [labels, setLabels] = createSignal<ScreenLabel[]>([]);
  const [ready, setReady] = createSignal(false);
  const [spikeSelectedId, setSpikeSelectedId] = createSignal<string | null>(null);

  // ── Task 6.2.2: spike-mode graph model ──────────────────────────────────
  // When `props.spike` is supplied the component is a pure SemanticScene
  // consumer: nodes/edges are derived from the scene, NOT from the dormant
  // graphData stub. This is the path that actually renders.
  const isSpikeMode = () => props.spike != null;

  const spikeModel = () => {
    const s = props.spike;
    if (!s) return { nodes: [] as GraphNode[], edges: [] as GraphEdge[] };
    return sceneToGraphModel(s.scene);
  };

  /** The active node set — spike scene when present, else the dormant stub. */
  const activeNodes = (): GraphNode[] =>
    isSpikeMode() ? spikeModel().nodes : graphData.visibleNodes();

  /** The active edge set — spike scene when present, else the dormant stub. */
  const activeEdges = (): GraphEdge[] =>
    isSpikeMode() ? spikeModel().edges : graphData.visibleEdges();

  /** The active focused id — spike selection when present, else the stub. */
  const activeFocusedId = (): string | null =>
    isSpikeMode() ? spikeSelectedId() : graphData.focusedId();

  /** Refresh the HTML label overlay from the scene's projection. */
  function refreshLabels() {
    if (!scene) return;
    setLabels(scene.computeLabels(activeFocusedId(), stage.clientWidth, stage.clientHeight));
  }

  function postToWorker(msg: LayoutRequest) {
    worker?.postMessage(msg);
  }

  /** (Re)start the layout for the current visible model.
   *
   * In spike mode we FIRST apply deterministic Fibonacci-sphere positions so
   * geometry is immediately visible, then hand the graph to the worker for
   * optional force-refinement. The scene is therefore never blank while the
   * worker spins up (or if the worker is unavailable entirely). */
  function startLayout() {
    const nodes = activeNodes();
    const edges = activeEdges();

    // Immediate deterministic layout — the scene renders on the next frame.
    if (nodes.length > 0) {
      scene?.applyPositions(computeDeterministicPositions(nodes));
    }

    postToWorker({
      type: "start",
      dimensions: 3,
      nodes: nodes.map((n) => ({ id: n.id })),
      links: edges
        .filter((e) => !e.predicted)
        .map((e) => ({ source: e.source, target: e.target })),
    });
  }

  let disposed = false;
  onCleanup(() => {
    disposed = true;
  });

  // Keep the scene synchronized when async loads, refreshes, expands, or
  // hide/unhide actions change the reactive graph model after mount.
  createEffect(() => {
    const nextNodes = activeNodes();
    const nextEdges = activeEdges();
    // The worker is optional in spike mode — the deterministic layout inside
    // startLayout() means we can render without it.
    if (!scene || !controller) return;

    scene.setGraph(nextNodes, nextEdges);
    startLayout();
    controller.noteInteraction();
  });

  onMount(async () => {
    try {
      // Dynamic imports keep three + the worker out of the 2D / test paths.
      const [{ GraphScene }, { LensController }] = await Promise.all([
        import("./GraphScene"),
        import("./lensController"),
      ]);
      // Reduced-motion may have unmounted this branch while imports resolved.
      if (disposed) return;

      scene = new GraphScene(canvas);
      scene.setGraph(activeNodes(), activeEdges());

      controller = new LensController({
        reducedMotion: !!props.static,
        render: () => {
          scene?.render();
          refreshLabels();
        },
        dispose: () => scene?.dispose(),
      });

      // Orbit/zoom is an interaction (resumes the loop); a node click selects it.
      scene.setOnInteraction(() => controller?.noteInteraction());
      scene.setOnPick((id) => {
        scene?.setSelected(id);
        if (isSpikeMode()) {
          // Spike mode: selection is local presentation state and the action is
          // dispatched through the SAME typed callback Graph2D uses. The 3D
          // renderer never fetches data or expands the graph itself.
          setSpikeSelectedId(id);
          props.spike?.onAction(id, "select");
          controller?.noteInteraction();
          return;
        }
        // Legacy dormant path (graphData stub — always empty).
        void graphData.expand(id).then(() => {
          scene?.setGraph(activeNodes(), activeEdges());
          scene?.setSelected(id);
          startLayout();
          controller?.noteInteraction();
        });
      });

      // Spin up the layout worker for optional force-refinement. Failure is
      // non-fatal in spike mode because deterministic positions already render.
      try {
        worker = new Worker(new URL("./layout.worker.ts", import.meta.url), { type: "module" });
        worker.onmessage = (event: MessageEvent<LayoutResponse>) => {
          const msg = event.data;
          if (msg.type === "tick") {
            scene?.applyPositions(msg.positions);
            controller?.noteLayoutTick();
          } else if (msg.type === "settled") {
            scene?.applyPositions(msg.positions);
            controller?.noteLayoutSettled();
          }
        };
        worker.onerror = () => {
          // Worker failure only degrades force-refinement; the deterministic
          // layout stays on screen. Only degrade to 2D outside spike mode.
          if (!disposed && !isSpikeMode()) {
            degradeToTwoD("2D fallback: 3D graph layout failed");
          }
        };
      } catch {
        worker = null; // deterministic layout still renders
      }

      controller.mount();
      startLayout();
      setReady(true);

      // Keep the drawing buffer sized to the stage; a resize is an interaction.
      resizeObserver = new ResizeObserver(() => {
        if (!scene) return;
        scene.resize(stage.clientWidth, stage.clientHeight);
        controller?.noteInteraction();
      });
      resizeObserver.observe(stage);
    } catch {
      if (!disposed) degradeToTwoD("2D fallback: 3D graph could not start");
    }
  });

  onCleanup(() => {
    resizeObserver?.disconnect();
    resizeObserver = null;
    // Stop + terminate the worker (no perpetual simulation after exit).
    postToWorker({ type: "stop" });
    worker?.terminate();
    worker = null;
    // Unmount the controller (stops loop, disposes the scene).
    controller?.unmount();
    controller = null;
    scene = null;
  });

  function resetView() {
    scene?.resetView();
    controller?.noteInteraction();
  }

  function pinFocused() {
    const id = activeFocusedId();
    if (!id) return;
    if (isSpikeMode()) return; // pin is not an authorized spike action
    graphData.togglePin(id);
    controller?.noteInteraction();
  }

  function hideFocused() {
    const id = activeFocusedId();
    if (!id) return;
    if (isSpikeMode()) {
      // Spike mode: clear selection rather than mutating the scene (the 3D
      // renderer must not remove authority items from the shared scene).
      setSpikeSelectedId(null);
      scene?.setSelected(null);
      controller?.noteInteraction();
      return;
    }
    graphData.hide(id);
    scene?.setGraph(activeNodes(), activeEdges());
    startLayout();
    controller?.noteInteraction();
  }

  async function materialize(target: string) {
    if (isSpikeMode()) return; // predictions are not part of the spike contract
    const res = await graphData.materializePrediction(target);
    if (res.ok) {
      scene?.setGraph(activeNodes(), activeEdges());
      startLayout();
      controller?.noteInteraction();
    }
  }

  return (
    <div class="kria-graph">
      <div class="kria-graph__toolbar">
        <Button variant="secondary" size="sm" onClick={resetView} disabled={!ready()}>
          Reset view
        </Button>
        <Show when={!isSpikeMode()}>
          <Button variant="secondary" size="sm" onClick={pinFocused} disabled={!activeFocusedId()}>
            {activeFocusedId() && graphData.pinned().has(activeFocusedId()!) ? "Unpin" : "Pin"}
          </Button>
        </Show>
        <Button variant="secondary" size="sm" onClick={hideFocused} disabled={!activeFocusedId()}>
          {isSpikeMode() ? "Clear selection" : "Hide"}
        </Button>
        <Show when={!isSpikeMode() && graphData.hidden().size > 0}>
          <Button variant="ghost" size="sm" onClick={() => graphData.unhideAll()}>
            Unhide all ({graphData.hidden().size})
          </Button>
        </Show>
        <Show when={!isSpikeMode() && graphData.capped()}>
          {(c) => <span class="kria-graph__count">{c().label}</span>}
        </Show>
        <Show when={isSpikeMode()}>
          <span class="kria-graph__count">
            {activeNodes().length} node{activeNodes().length === 1 ? "" : "s"}
            {activeEdges().length > 0 ? `, ${activeEdges().length} edges` : ""}
          </span>
        </Show>
      </div>

      <div class="kria-graph__stage" ref={stage}>
        <canvas class="kria-graph__canvas" ref={canvas} />

        <Show when={!isSpikeMode() && graphData.loading()}>
          <div class="kria-graph__stage-state" role="status" aria-live="polite">
            Loading knowledge graph…
          </div>
        </Show>
        <Show when={!isSpikeMode() && graphData.error()}>
          <div class="kria-graph__stage-state">
            <EmptyState
              icon="alert-triangle"
              title="Couldn't load the knowledge graph"
              description={graphData.error() ?? "The graph service is unavailable."}
            />
          </div>
        </Show>
        {/* Empty state — only when the ACTIVE model has no nodes. In spike mode
            this reflects the SemanticScene, not the dormant stub. */}
        <Show
          when={
            (isSpikeMode() || (!graphData.loading() && !graphData.error())) &&
            activeNodes().length === 0
          }
        >
          <div class="kria-graph__stage-state">
            <EmptyState
              icon="network"
              title="No graph yet"
              description="No extracted knowledge entities exist yet. Add memories or run entity extraction from Cognition."
            />
          </div>
        </Show>

        <div class="kria-graph__labels" aria-hidden="true">
          <For each={labels()}>
            {(l) => (
              <span
                class="kria-graph__label"
                data-focused={activeFocusedId() === l.id ? "true" : "false"}
                style={{ left: `${l.x}px`, top: `${l.y}px` }}
              >
                {l.label}
              </span>
            )}
          </For>
        </div>

        {/* Predicted-links panel is legacy-only — not part of the spike contract. */}
        <Show when={!isSpikeMode() && graphData.focusedId()}>
          <aside class="kria-graph__panel" aria-label="Focused entity">
            <h4 class="kria-graph__panel-title">Predicted links</h4>
            <Show
              when={graphData.predicted().length > 0}
              fallback={<p class="kria-graph__pred">No predicted links.</p>}
            >
              <For each={graphData.predicted()}>
                {(p) => (
                  <div class="kria-graph__pred">
                    <span>{p.label}</span>
                    <span>score {p.score.toFixed(2)}</span>
                    <Button variant="ghost" size="sm" onClick={() => void materialize(p.target)}>
                      Materialize
                    </Button>
                  </div>
                )}
              </For>
            </Show>
          </aside>
        </Show>
      </div>
    </div>
  );
}

export default GraphCanvas3D;
