/**
 * ConstellationLens — the Capabilities Space "Constellation" segment (task 8.3,
 * Req 7.5 / 16.3 / 17.5). Mounted by CapabilitiesSpace for the `constellation`
 * segment. The Capabilities analogue of the Memory KnowledgeGraphLens (6.4/6.5).
 *
 * 2D-FIRST, 3D as a CAPABILITY-GATED ENHANCEMENT (design.md §11.2, task 0.6 —
 * 3D is opt-in on WebKitGTK). This entry uses the SAME shared render-mode gate
 * and 3D governance as the Memory graph:
 *   1. Loads the constellation read-model (constellationData) from the existing
 *      capability catalogs — needed by BOTH representations.
 *   2. Decides whether to even ATTEMPT 3D via the shared degrade ladder
 *      (evaluateDegrade: no-WebGL / reduced-motion / heavy load → stay 2D).
 *   3. If eligible, runs the on-device §11.3 G2 probe ONCE and applies it to the
 *      SHARED render-mode gate. Only a PASSING probe flips the gate into 3D;
 *      until then the mandatory 2D catalog fallback renders. Under jsdom /
 *      WebKitGTK software-raster the probe returns null → stays 2D (accepted).
 *   4. Renders through <LensRenderMode>: `twoD` = ConstellationFallback (always
 *      available, the DEFAULT), `threeD` = ConstellationCanvas3D (mounted only
 *      when enable3D).
 *   5. Unloads the read-model on exit (§5.4 unload on Space exit).
 *
 * The GL scene + worker live only inside ConstellationCanvas3D (dynamically
 * imported), so this entry and the 2D path never touch WebGL.
 *
 * ARCHITECTURE: READ/VISUALIZE ONLY — no capability execution, no writes; a tool
 * node opens its descriptor in the shared Inspector (Req 7.2). KRIA remains the
 * orchestration authority; the lens is pure legibility.
 */
import { onCleanup, onMount, Show } from "solid-js";
import { LensRenderMode } from "../../../../platform/LensRenderMode";
import { LensModeToggle } from "../../../../platform/LensModeToggle";
import { applyProbeResult, degradeToTwoD, lensRenderMode } from "../../../../platform/renderMode";
import { runG2Probe } from "../../../../prototypes/gateProbes";
import { evaluateDegrade } from "../../memory/graph/graphModel";
import { constellationData } from "./constellationData";
import { ConstellationFallback } from "./ConstellationFallback";
import { ConstellationCanvas3D } from "./ConstellationCanvas3D";
import { IconButton } from "../../../../kit";
import { openDetachedSurface, windowPresentation } from "../../../../windowing/detachableSurfaces";
import "../../memory/graph/KnowledgeGraphLens.css";
import "./constellation.css";

export default function ConstellationLens() {
  onMount(() => {
    // Load the constellation read-model for whichever representation renders.
    void constellationData.load();

    // Decide whether 3D is even worth attempting on this device (shared degrade
    // ladder). WebGL absence / reduced-motion / heavy load keep us 2D.
    const snapshot = lensRenderMode().snapshot;
    const decision = evaluateDegrade({
      hasWebGL: snapshot.hasWebGL,
      reducedMotion: snapshot.prefersReducedMotion,
      recentFps: [],
    });
    if (decision.degrade) {
      degradeToTwoD(decision.reason);
      return;
    }

    // Eligible: run the §11.3 G2 probe once; only a passing probe enables 3D.
    void runG2Probe({ nodeCount: 1500, frames: 90 })
      .then((probe) => applyProbeResult(probe))
      .catch(() => applyProbeResult(null));
  });

  // Unload on Space exit (§5.4): clear the read-model. ConstellationCanvas3D
  // disposes its own GL scene + worker in its onCleanup.
  onCleanup(() => constellationData.reset());

  return (
    <div class="kria-graph">
      <div class="kria-graph__toolbar">
        <LensModeToggle label="Constellation view mode" />
        <Show when={windowPresentation.surface() !== "lens"}>
          <IconButton icon="monitor" label="Detach constellation lens"
            onClick={() => void openDetachedSurface("lens", "capabilities")} />
        </Show>
      </div>
      <LensRenderMode
        twoD={(s) => <ConstellationFallback static={s.isStatic} reason={s.reason} />}
        threeD={(s) => <ConstellationCanvas3D static={s.isStatic} />}
      />
    </div>
  );
}
