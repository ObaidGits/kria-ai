import { onCleanup, onMount, Show } from "solid-js";
import { IconButton } from "../../../../kit";
import { lensRenderMode } from "../../../../platform/renderMode";
import { eventBus, type Unsubscribe } from "../../../../stores/eventBus";
import { openDetachedSurface, windowPresentation } from "../../../../windowing/detachableSurfaces";
import { graphData } from "./graphData";
import { MemoryUniverse } from "./MemoryUniverse";
import "./KnowledgeGraphLens.css";

/** Immersive deterministic Memory Graph. SVG stays available without WebGL. */
export default function KnowledgeGraphLens() {
  let unsubscribeMemory: Unsubscribe | null = null;
  let refreshTimer: ReturnType<typeof setTimeout> | null = null;

  onMount(() => {
    void graphData.load();
    unsubscribeMemory = eventBus.on("memory:updated", ({ kind }) => {
      if (kind && !["relationship", "entity", "created", "updated", "deleted"].includes(kind)) return;
      if (refreshTimer) clearTimeout(refreshTimer);
      refreshTimer = setTimeout(() => {
        refreshTimer = null;
        void graphData.load();
      }, 250);
    });
  });

  onCleanup(() => {
    unsubscribeMemory?.();
    if (refreshTimer) clearTimeout(refreshTimer);
    graphData.reset();
  });

  return (
    <div class="kria-graph">
      <Show when={windowPresentation.surface() !== "lens"}>
        <div class="kria-graph__shell-tools">
          <IconButton icon="monitor" label="Detach Memory Graph" onClick={() => void openDetachedSurface("lens", "memory")} />
        </div>
      </Show>
      <MemoryUniverse static={lensRenderMode().isStatic} />
    </div>
  );
}
