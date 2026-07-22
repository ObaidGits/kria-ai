/**
 * ConstellationCanvas3D — the 3D branch of the Capabilities Constellation lens
 * (task 8.3, §5.4). Mounted ONLY when the shared capability gate has enabled 3D
 * (see ConstellationLens).
 *
 * REUSES the Memory graph's budgeted GL infrastructure verbatim (task 6.4) so
 * both 3D lenses share ONE governance story:
 *   • the same layout Web Worker (ngraph.forcelayout, off-thread, stops on
 *     settle) — imported from ../../memory/graph/layout.worker,
 *   • the same Three.js `GraphScene` (instanced nodes/edges, LOD labels, damped
 *     constrained orbit, reset-view, frustum culling, community color, centrality
 *     size),
 *   • the same `LensController` (freeze-on-idle, resume-on-interaction,
 *     unload-on-exit, reduced-motion still frames).
 *
 * The ONLY behavioural difference from the Memory graph: this lens is
 * READ/VISUALIZE ONLY. A node click focuses it and — for a tool node — opens
 * its descriptor in the shared Inspector (Req 7.2). There is NO materialize /
 * backend write and NO capability execution. GL + the worker are DYNAMICALLY
 * imported inside onMount so the 2D path / jsdom tests never pull in three.
 */
import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { Button } from "../../../../kit";
import { shellStore } from "../../../../stores";
import type { GraphScene as GraphSceneType, ScreenLabel } from "../../memory/graph/GraphScene";
import type { LensController as LensControllerType } from "../../memory/graph/lensController";
import type { LayoutRequest, LayoutResponse } from "../../memory/graph/layoutSettle";
import { constellationData } from "./constellationData";
import { labelForKind } from "./constellationModel";

export interface ConstellationCanvas3DProps {
  /** Reduced-motion posture → controller renders discrete still frames only. */
  static?: boolean;
}

export function ConstellationCanvas3D(props: ConstellationCanvas3DProps) {
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
    setLabels(
      scene.computeLabels(constellationData.focusedId(), stage.clientWidth, stage.clientHeight),
    );
  }

  function postToWorker(msg: LayoutRequest) {
    worker?.postMessage(msg);
  }

  /** (Re)start the layout for the current visible model. */
  function startLayout() {
    const model = constellationData.model();
    postToWorker({
      type: "start",
      dimensions: 3,
      nodes: model.nodes.map((n) => ({ id: n.id })),
      links: model.edges.map((e) => ({ source: e.source, target: e.target })),
    });
  }

  /** Open the descriptor for a focused tool node in the shared Inspector. */
  function inspectIfDescriptor(id: string) {
    const m = constellationData.metaFor(id);
    if (m?.hasDescriptor && m.providerId && m.capabilityId) {
      // Node click focus lands on the canvas, not a semantic control — hand the
      // stable Capabilities region as the Focus_Return_Owner (§20.3/§20.4).
      shellStore.openInspector(
        "capability",
        id,
        {
          providerId: m.providerId,
          capabilityId: m.capabilityId,
          name: m.name,
        },
        { regionSelector: '[data-space="capabilities"]' },
      );
    }
  }

  let disposed = false;
  onCleanup(() => {
    disposed = true;
  });

  onMount(async () => {
    // Dynamic imports keep three + the worker out of the 2D / test paths.
    const [{ GraphScene }, { LensController }] = await Promise.all([
      import("../../memory/graph/GraphScene"),
      import("../../memory/graph/lensController"),
    ]);
    // Reduced-motion may have unmounted this branch while imports resolved.
    if (disposed) return;

    scene = new GraphScene(canvas);
    scene.setGraph(constellationData.visibleNodes(), constellationData.visibleEdges());

    controller = new LensController({
      reducedMotion: !!props.static,
      render: () => {
        scene?.render();
        refreshLabels();
      },
      dispose: () => scene?.dispose(),
    });

    // Orbit/zoom is an interaction (resumes the loop); a node click focuses it
    // (read-only view action) and opens a tool's descriptor in the Inspector.
    scene.setOnInteraction(() => controller?.noteInteraction());
    scene.setOnPick((id) => {
      scene?.setSelected(id);
      constellationData.focus(id);
      inspectIfDescriptor(id);
      controller?.noteInteraction();
    });

    // Spin up the SHARED layout worker (Vite resolves the URL at build time).
    worker = new Worker(new URL("../../memory/graph/layout.worker.ts", import.meta.url), {
      type: "module",
    });
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
    const id = constellationData.focusedId();
    if (!id) return;
    constellationData.togglePin(id);
    controller?.noteInteraction();
  }

  function hideFocused() {
    const id = constellationData.focusedId();
    if (!id) return;
    constellationData.hide(id);
    scene?.setGraph(constellationData.visibleNodes(), constellationData.visibleEdges());
    startLayout();
    controller?.noteInteraction();
  }

  const focusedKind = () => {
    const id = constellationData.focusedId();
    return id ? constellationData.metaFor(id)?.kind : undefined;
  };

  return (
    <div class="kria-graph">
      <div class="kria-graph__toolbar">
        <Button variant="secondary" size="sm" onClick={resetView} disabled={!ready()}>
          Reset view
        </Button>
        <Button
          variant="secondary"
          size="sm"
          onClick={pinFocused}
          disabled={!constellationData.focusedId()}
        >
          {constellationData.focusedId() &&
          constellationData.pinned().has(constellationData.focusedId()!)
            ? "Unpin"
            : "Pin"}
        </Button>
        <Button
          variant="secondary"
          size="sm"
          onClick={hideFocused}
          disabled={!constellationData.focusedId()}
        >
          Hide
        </Button>
        <Show when={constellationData.hidden().size > 0}>
          <Button variant="ghost" size="sm" onClick={() => constellationData.unhideAll()}>
            Unhide all ({constellationData.hidden().size})
          </Button>
        </Show>
        <Show when={constellationData.capped()}>
          {(c) => <span class="kria-graph__count">{c().label}</span>}
        </Show>
      </div>

      <div class="kria-graph__stage" ref={stage}>
        <canvas class="kria-graph__canvas" ref={canvas} />

        <div class="kria-graph__labels" aria-hidden="true">
          <For each={labels()}>
            {(l) => (
              <span
                class="kria-graph__label"
                data-focused={constellationData.focusedId() === l.id ? "true" : "false"}
                style={{ left: `${l.x}px`, top: `${l.y}px` }}
              >
                {l.label}
              </span>
            )}
          </For>
        </div>

        <Show when={constellationData.focusedId()}>
          {(focused) => (
            <aside class="kria-graph__panel" aria-label="Focused node">
              <h4 class="kria-graph__panel-title">
                {constellationData.metaFor(focused())?.name ?? focused()}
              </h4>
              <p class="kria-graph__pred">
                <span>{focusedKind() ? labelForKind(focusedKind()!) : "Node"}</span>
              </p>
              <Show when={constellationData.metaFor(focused())?.detail}>
                <p class="kria-graph__pred">
                  <span>{constellationData.metaFor(focused())!.detail}</span>
                </p>
              </Show>
              <Show when={constellationData.metaFor(focused())?.hasDescriptor}>
                <Button variant="ghost" size="sm" onClick={() => inspectIfDescriptor(focused())}>
                  Open descriptor
                </Button>
              </Show>
            </aside>
          )}
        </Show>
      </div>
    </div>
  );
}

export default ConstellationCanvas3D;
