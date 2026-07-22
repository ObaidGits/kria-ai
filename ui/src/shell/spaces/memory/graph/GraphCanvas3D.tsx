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
 */
import { For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { Button, EmptyState } from "../../../../kit";
import { degradeToTwoD } from "../../../../platform/renderMode";
import { graphData } from "./graphData";
import type { GraphScene as GraphSceneType, ScreenLabel } from "./GraphScene";
import type { LensController as LensControllerType } from "./lensController";
import type { LayoutRequest, LayoutResponse } from "./layoutSettle";

export interface GraphCanvas3DProps {
  /** Reduced-motion posture → controller renders discrete still frames only. */
  static?: boolean;
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

  /** Refresh the HTML label overlay from the scene's projection. */
  function refreshLabels() {
    if (!scene) return;
    setLabels(scene.computeLabels(graphData.focusedId(), stage.clientWidth, stage.clientHeight));
  }

  function postToWorker(msg: LayoutRequest) {
    worker?.postMessage(msg);
  }

  /** (Re)start the layout for the current visible model. */
  function startLayout() {
    const model = graphData.model();
    postToWorker({
      type: "start",
      dimensions: 3,
      nodes: model.nodes.map((n) => ({ id: n.id })),
      links: model.edges
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
    const nextNodes = graphData.visibleNodes();
    const nextEdges = graphData.visibleEdges();
    if (!scene || !worker || !controller) return;

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
      scene.setGraph(graphData.visibleNodes(), graphData.visibleEdges());

      controller = new LensController({
        reducedMotion: !!props.static,
        render: () => {
          scene?.render();
          refreshLabels();
        },
        dispose: () => scene?.dispose(),
      });

      // Orbit/zoom is an interaction (resumes the loop); a node click focuses +
      // expands it (read-only view action), then re-layouts the enlarged graph.
      scene.setOnInteraction(() => controller?.noteInteraction());
      scene.setOnPick((id) => {
        scene?.setSelected(id);
        void graphData.expand(id).then(() => {
          scene?.setGraph(graphData.visibleNodes(), graphData.visibleEdges());
          scene?.setSelected(id);
          startLayout();
          controller?.noteInteraction();
        });
      });

      // Spin up the layout worker (Vite resolves the URL at build time).
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
        if (!disposed) degradeToTwoD("2D fallback: 3D graph layout failed");
      };

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
    const id = graphData.focusedId();
    if (!id) return;
    graphData.togglePin(id);
    controller?.noteInteraction();
  }

  function hideFocused() {
    const id = graphData.focusedId();
    if (!id) return;
    graphData.hide(id);
    scene?.setGraph(graphData.visibleNodes(), graphData.visibleEdges());
    startLayout();
    controller?.noteInteraction();
  }

  async function materialize(target: string) {
    const res = await graphData.materializePrediction(target);
    if (res.ok) {
      scene?.setGraph(graphData.visibleNodes(), graphData.visibleEdges());
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
        <Button variant="secondary" size="sm" onClick={pinFocused} disabled={!graphData.focusedId()}>
          {graphData.focusedId() && graphData.pinned().has(graphData.focusedId()!) ? "Unpin" : "Pin"}
        </Button>
        <Button variant="secondary" size="sm" onClick={hideFocused} disabled={!graphData.focusedId()}>
          Hide
        </Button>
        <Show when={graphData.hidden().size > 0}>
          <Button variant="ghost" size="sm" onClick={() => graphData.unhideAll()}>
            Unhide all ({graphData.hidden().size})
          </Button>
        </Show>
        <Show when={graphData.capped()}>
          {(c) => <span class="kria-graph__count">{c().label}</span>}
        </Show>
      </div>

      <div class="kria-graph__stage" ref={stage}>
        <canvas class="kria-graph__canvas" ref={canvas} />

        <Show when={graphData.loading()}>
          <div class="kria-graph__stage-state" role="status" aria-live="polite">
            Loading knowledge graph…
          </div>
        </Show>
        <Show when={graphData.error()}>
          <div class="kria-graph__stage-state">
            <EmptyState
              icon="alert-triangle"
              title="Couldn't load the knowledge graph"
              description={graphData.error() ?? "The graph service is unavailable."}
            />
          </div>
        </Show>
        <Show
          when={
            !graphData.loading() &&
            !graphData.error() &&
            graphData.visibleNodes().length === 0
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
                data-focused={graphData.focusedId() === l.id ? "true" : "false"}
                style={{ left: `${l.x}px`, top: `${l.y}px` }}
              >
                {l.label}
              </span>
            )}
          </For>
        </div>

        <Show when={graphData.focusedId()}>
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
