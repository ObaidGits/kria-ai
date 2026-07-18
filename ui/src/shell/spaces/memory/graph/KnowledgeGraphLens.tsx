/**
 * KnowledgeGraphLens — the Memory Space "Knowledge Graph" region (task 6.4,
 * Req 5.4 / 16.3). Mounted by MemorySpace for the `knowledgegraph` segment.
 *
 * 2D-FIRST, 3D as a CAPABILITY-GATED ENHANCEMENT (design.md §11.2). This entry:
 *   1. Loads the graph read-model (graphData) from the existing memory_graph_*
 *      commands — needed by BOTH representations.
 *   2. Decides whether to even ATTEMPT 3D via the shared degrade ladder
 *      (evaluateDegrade: no-WebGL / reduced-motion / heavy load → stay 2D).
 *   3. If eligible, runs the on-device §11.3 G2 probe ONCE and applies the
 *      result to the shared render-mode gate. Only a PASSING probe flips the
 *      gate into 3D (renderMode.ts); until/unless that happens the mandatory 2D
 *      fallback renders. Under jsdom / WebKitGTK software-raster the probe
 *      returns null → the lens stays 2D (an accepted outcome, not a failure).
 *   4. Renders through <LensRenderMode>: `twoD` = MemoryGraphFallback (always
 *      available), `threeD` = GraphCanvas3D (mounted only when enable3D).
 *   5. Unloads the read-model on exit (§5.4 unload on Space exit).
 *
 * The GL scene + worker live only inside GraphCanvas3D (dynamically imported),
 * so this entry and the 2D path never touch WebGL — the surrounding logic is
 * fully testable under jsdom.
 */
import { onCleanup, onMount, Show } from "solid-js";
import { LensRenderMode } from "../../../../platform/LensRenderMode";
import { LensModeToggle } from "../../../../platform/LensModeToggle";
import { applyProbeResult, degradeToTwoD, lensRenderMode } from "../../../../platform/renderMode";
import { runG2Probe } from "../../../../prototypes/gateProbes";
import { graphData } from "./graphData";
import { evaluateDegrade } from "./graphModel";
import { MemoryGraphFallback } from "./MemoryGraphFallback";
import { GraphCanvas3D } from "./GraphCanvas3D";
import { IconButton } from "../../../../kit";
import { openDetachedSurface, windowPresentation } from "../../../../windowing/detachableSurfaces";
import { eventBus, type Unsubscribe } from "../../../../stores/eventBus";
import "./KnowledgeGraphLens.css";

export default function KnowledgeGraphLens() {
  let unsubscribeMemory: Unsubscribe | null = null;
  let refreshTimer: ReturnType<typeof setTimeout> | null = null;

  onMount(() => {
    // Load the graph read-model for whichever representation renders.
    void graphData.load();
    unsubscribeMemory = eventBus.on("memory:updated", ({ kind }) => {
      if (kind && !["relationship", "entity", "created", "updated", "deleted"].includes(kind)) return;
      if (refreshTimer) clearTimeout(refreshTimer);
      refreshTimer = setTimeout(() => {
        refreshTimer = null;
        void graphData.load();
      }, 250);
    });

    // Decide whether 3D is even worth attempting on this device (shared degrade
    // ladder). WebGL absence / reduced-motion / heavy load keep us 2D.
    const snapshot = lensRenderMode().snapshot;
    const decision = evaluateDegrade({
      hasWebGL: snapshot.hasWebGL,
      reducedMotion: snapshot.prefersReducedMotion,
      recentFps: [],
    });
    if (decision.degrade) {
      // Ensure the gate reflects the 2D posture (idempotent when already 2D).
      degradeToTwoD(decision.reason);
      return;
    }

    // Eligible: run the §11.3 G2 probe once; only a passing probe enables 3D.
    // runG2Probe resolves null when WebGL can't actually render (→ stays 2D).
    void runG2Probe({ nodeCount: 1500, frames: 90 })
      .then((probe) => applyProbeResult(probe))
      .catch(() => applyProbeResult(null));
  });

  // Unload on Space exit (§5.4): clear the read-model. GraphCanvas3D disposes
  // its own GL scene + worker in its onCleanup.
  onCleanup(() => {
    unsubscribeMemory?.();
    if (refreshTimer) clearTimeout(refreshTimer);
    graphData.reset();
  });

  return (
    <div class="kria-graph">
      <div class="kria-graph__toolbar">
        <LensModeToggle label="Knowledge graph view mode" />
        <Show when={windowPresentation.surface() !== "lens"}>
          <IconButton icon="monitor" label="Detach knowledge graph lens"
            onClick={() => void openDetachedSurface("lens", "memory")} />
        </Show>
      </div>
      <LensRenderMode
        twoD={(s) => <MemoryGraphFallback static={s.isStatic} reason={s.reason} />}
        threeD={(s) => <GraphCanvas3D static={s.isStatic} />}
      />
    </div>
  );
}
