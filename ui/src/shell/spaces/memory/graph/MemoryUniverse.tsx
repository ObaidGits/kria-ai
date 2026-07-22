import { createMemo, createSignal, For, Show } from "solid-js";
import { Icon } from "../../../../components/Icon";
import { MemoryGraphFallback } from "./MemoryGraphFallback";
import { graphData } from "./graphData";
import {
  buildUniverse,
  curvedPath,
  MEMORY_CATEGORIES,
  UNIVERSE_CENTER,
  type UniverseHub,
  type UniverseMemory,
} from "./memoryUniverseModel";

interface CameraState { scale: number; x: number; y: number }
type Selection = { kind: "core" } | { kind: "hub"; id: string } | { kind: "memory"; id: string };

const DEFAULT_CAMERA: CameraState = { scale: 1, x: 0, y: 0 };

export function MemoryUniverse(props: { static?: boolean }) {
  const [selection, setSelection] = createSignal<Selection>({ kind: "core" });
  const [hoveredId, setHoveredId] = createSignal<string | null>(null);
  const [camera, setCamera] = createSignal<CameraState>(DEFAULT_CAMERA);
  const [query, setQuery] = createSignal("");
  const [showInspector, setShowInspector] = createSignal(false);
  const [showList, setShowList] = createSignal(false);
  const [lens, setLens] = createSignal<"navigation" | "relationships" | "predictions">("navigation");
  let svg!: SVGSVGElement;
  let dragging = false;
  let dragOrigin = { x: 0, y: 0 };
  let cameraOrigin = DEFAULT_CAMERA;

  const model = createMemo(() => buildUniverse(graphData.visibleNodes(), graphData.visibleEdges()));
  const selectedMemory = createMemo(() => {
    const current = selection();
    return current.kind === "memory" ? model().memories.find((memory) => memory.id === current.id) : undefined;
  });
  const selectedHub = createMemo(() => {
    const current = selection();
    const categoryId = current.kind === "hub" ? current.id : selectedMemory()?.categoryId;
    return categoryId ? model().hubs.find((hub) => hub.id === categoryId) : undefined;
  });
  const selectedRelationships = createMemo(() => {
    const memory = selectedMemory();
    return memory
      ? model().relationships.filter((edge) => edge.source === memory.id || edge.target === memory.id)
      : [];
  });
  const searchMatch = (memory: UniverseMemory) => !query().trim() || memory.label.toLowerCase().includes(query().trim().toLowerCase());
  const worldTransform = createMemo(() => {
    const value = camera();
    return `translate(${value.x} ${value.y}) scale(${value.scale})`;
  });

  function framePoint(x: number, y: number, scale: number): void {
    setCamera({
      scale,
      x: UNIVERSE_CENTER.x - x * scale,
      y: UNIVERSE_CENTER.y - y * scale,
    });
  }

  function resetCamera(): void {
    setCamera(DEFAULT_CAMERA);
  }

  function selectCore(): void {
    setSelection({ kind: "core" });
    setShowInspector(false);
    resetCamera();
  }

  function selectHub(hub: UniverseHub): void {
    setSelection({ kind: "hub", id: hub.id });
    setShowInspector(true);
    framePoint(hub.x, hub.y, 1.2);
  }

  async function selectMemory(memory: UniverseMemory, focus = false): Promise<void> {
    setSelection({ kind: "memory", id: memory.id });
    setShowInspector(true);
    framePoint(memory.x, memory.y, focus ? 1.9 : 1.48);
    await graphData.expand(memory.id);
  }

  function activate(event: KeyboardEvent, action: () => void): void {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    action();
  }

  function zoom(delta: number): void {
    setCamera((current) => {
      const scale = Math.min(2.4, Math.max(0.66, current.scale * delta));
      const ratio = scale / current.scale;
      return {
        scale,
        x: UNIVERSE_CENTER.x - (UNIVERSE_CENTER.x - current.x) * ratio,
        y: UNIVERSE_CENTER.y - (UNIVERSE_CENTER.y - current.y) * ratio,
      };
    });
  }

  function pointerDown(event: PointerEvent): void {
    if (event.button !== 0) return;
    dragging = true;
    dragOrigin = { x: event.clientX, y: event.clientY };
    cameraOrigin = camera();
    svg.setPointerCapture?.(event.pointerId);
  }

  function pointerMove(event: PointerEvent): void {
    if (!dragging) return;
    const bounds = svg.getBoundingClientRect();
    const sx = 1100 / Math.max(1, bounds.width);
    const sy = 720 / Math.max(1, bounds.height);
    setCamera({
      ...cameraOrigin,
      x: cameraOrigin.x + (event.clientX - dragOrigin.x) * sx,
      y: cameraOrigin.y + (event.clientY - dragOrigin.y) * sy,
    });
  }

  function pointerUp(event: PointerEvent): void {
    dragging = false;
    svg.releasePointerCapture?.(event.pointerId);
  }

  const selectionCategory = () => selectedHub()?.id;
  const selectedMemoryId = () => {
    const current = selection();
    return current.kind === "memory" ? current.id : null;
  };
  const memoryDimmed = (memory: UniverseMemory) =>
    !searchMatch(memory) || (selection().kind !== "core" && selectionCategory() !== memory.categoryId);

  return (
    <section
      class="memory-universe"
      classList={{ "memory-universe--static": !!props.static }}
      data-lens={lens()}
      aria-label="KRIA Memory Graph"
    >
      <div class="memory-universe__status" aria-live="polite">
        <div><strong>Memory Graph</strong><span>{model().memories.length} {model().memories.length === 1 ? "memory" : "memories"} shown</span></div>
      </div>

      <div class="memory-universe__search">
        <Icon name="search" />
        <input
          aria-label="Filter visible memories"
          placeholder="Filter this view…"
          value={query()}
          onInput={(event) => setQuery(event.currentTarget.value)}
        />
        <kbd>⌘K</kbd>
      </div>

      <nav class="memory-universe__toolbar" aria-label="Memory graph controls">
        <button classList={{ active: lens() === "navigation" }} onClick={() => setLens("navigation")}>
          <Icon name="network" /> Generated facets
        </button>
        <button classList={{ active: lens() === "relationships" }} onClick={() => setLens("relationships")}>
          <Icon name="git-branch" /> Relationships
        </button>
        <button classList={{ active: timeline() }} onClick={() => setTimeline((value) => !value)}>
          <Icon name="clock" /> Timeline
        </button>
        <button classList={{ active: lens() === "predictions" }} onClick={() => setLens("predictions")}>
          <Icon name="sparkles" /> Predicted links
        </button>
        <button onClick={() => setShowList(true)} title="Open accessible memory list"><Icon name="list-checks" /></button>
      </nav>

      <div class="memory-universe__view-controls" aria-label="Camera controls">
        <button onClick={() => zoom(1.18)} aria-label="Zoom in">+</button>
        <button onClick={() => zoom(0.84)} aria-label="Zoom out">−</button>
        <button onClick={resetCamera} aria-label="Center graph"><Icon name="maximize-2" /></button>
        <button onClick={() => { setSelection({ kind: "core" }); resetCamera(); }} aria-label="Auto arrange"><Icon name="rotate-ccw" /></button>
      </div>

      <svg
        ref={svg}
        class="memory-universe__scene"
        viewBox="0 0 1100 720"
        role="img"
        aria-label="Radial 2D memory view centered on current focus"
        onPointerDown={pointerDown}
        onPointerMove={pointerMove}
        onPointerUp={pointerUp}
        onPointerCancel={pointerUp}
        onWheel={(event) => { event.preventDefault(); zoom(event.deltaY < 0 ? 1.1 : 0.9); }}
      >
        <defs>
          <radialGradient id="core-fill">
            <stop offset="0" stop-color="var(--color-text-primary)" />
            <stop offset="0.18" stop-color="var(--color-accent-hover)" />
            <stop offset="0.52" stop-color="var(--color-info-solid)" />
            <stop offset="1" stop-color="var(--color-accent-secondary)" stop-opacity="0.08" />
          </radialGradient>
          <radialGradient id="hub-fill">
            <stop offset="0" stop-color="currentColor" stop-opacity="0.32" />
            <stop offset="0.52" stop-color="currentColor" stop-opacity="0.1" />
            <stop offset="1" stop-color="currentColor" stop-opacity="0" />
          </radialGradient>
          <linearGradient id="edge-flow" x1="0" x2="1">
            <stop offset="0" stop-color="var(--color-accent-secondary)" stop-opacity="0.2" />
            <stop offset="0.48" stop-color="var(--color-accent-default)" />
            <stop offset="1" stop-color="var(--color-info-solid)" stop-opacity="0.15" />
          </linearGradient>
          <filter id="core-glow" x="-180%" y="-180%" width="460%" height="460%">
            <feGaussianBlur stdDeviation="12" result="blur" />
            <feColorMatrix in="blur" type="matrix" values="0 0 0 0 0.2  0 0 0 0 0.72  0 0 0 0 1  0 0 0 1 0" />
            <feMerge><feMergeNode /><feMergeNode in="SourceGraphic" /></feMerge>
          </filter>
          <filter id="node-glow" x="-120%" y="-120%" width="340%" height="340%">
            <feGaussianBlur stdDeviation="4" result="blur" />
            <feMerge><feMergeNode in="blur" /><feMergeNode in="SourceGraphic" /></feMerge>
          </filter>
        </defs>

        <g class="memory-universe__world" transform={worldTransform()}>
          <g class="memory-universe__relationship-layer" role="group" aria-label="Authority relationships">
            <For each={model().relationships}>{(edge) => (
              <path
                d={curvedPath(edge.sourceNode.x, edge.sourceNode.y, edge.targetNode.x, edge.targetNode.y, 0.08)}
                data-authority-class={edge.predicted ? "inferred" : "stored"}
                classList={{ inferred: !!edge.predicted, stored: !edge.predicted, selected: selectedMemoryId() === edge.source || selectedMemoryId() === edge.target }}
              >
                <title>{edge.predicted ? "Inferred candidate" : "Stored relationship"}: {edge.relType ?? "type unavailable"} — {edge.sourceNode.label} to {edge.targetNode.label}</title>
              </path>
            )}</For>
          </g>

          <g class="memory-universe__navigation-facets" role="group" aria-label="Generated navigation facets">
          <For each={model().hubs}>{(hub) => (
            <g class={`memory-universe__cluster tone-${hub.tone}`} data-category={hub.id}>

              <For each={hub.memories}>{(memory, index) => (
                <g
                  class="memory-universe__memory"
                  classList={{
                    selected: selectedMemoryId() === memory.id,
                    hovered: hoveredId() === memory.id,
                    dimmed: memoryDimmed(memory),
                    major: index() < 4,
                  }}
                  transform={`translate(${memory.x} ${memory.y})`}
                  role="button"
                  tabindex="0"
                  aria-label={`${memory.label}, ${hub.label} memory`}
                  onPointerDown={(event) => event.stopPropagation()}
                  onMouseEnter={() => setHoveredId(memory.id)}
                  onMouseLeave={() => setHoveredId(null)}
                  onClick={() => void selectMemory(memory)}
                  onDblClick={() => void selectMemory(memory, true)}
                  onKeyDown={(event) => activate(event, () => void selectMemory(memory))}
                >
                  <circle class="memory-universe__memory-halo" r={memory.radius + 7} />
                  <circle class="memory-universe__memory-shell" r={memory.radius} />
                  <circle class="memory-universe__memory-light" cx={-memory.radius * 0.25} cy={-memory.radius * 0.3} r={Math.max(1.4, memory.radius * 0.28)} />
                  <Show when={index() < 4}>
                    <Icon class="memory-universe__memory-icon" name={hub.icon} size={memory.radius * 1.15} x={-memory.radius * 0.575} y={-memory.radius * 0.575} />
                  </Show>
                  <text class="memory-universe__memory-label" y={memory.radius + 15}>{memory.label}</text>
                </g>
              )}</For>

              <g
                class="memory-universe__hub"
                classList={{ selected: selectionCategory() === hub.id }}
                transform={`translate(${hub.x} ${hub.y})`}
                role="button"
                tabindex="0"
                data-authority-class={hub.authorityClass}
                data-generated={hub.generated ? "true" : "false"}
                aria-label={`Generated navigation facet ${hub.label}, ${hub.total} memories`}
                onPointerDown={(event) => event.stopPropagation()}
                onClick={() => selectHub(hub)}
                onDblClick={() => { selectHub(hub); framePoint(hub.x, hub.y, 1.65); }}
                onKeyDown={(event) => activate(event, () => selectHub(hub))}
              >
                <circle class="memory-universe__hub-aura" r="47" />
                <circle class="memory-universe__hub-hull" r="72" />
                <circle class="memory-universe__hub-rim" r="29" />
                <circle class="memory-universe__hub-body" r="24" />
                <Icon class="memory-universe__hub-icon" name={hub.icon} size={25} x={-12.5} y={-12.5} />
                <text class="memory-universe__hub-title" y="49">{hub.label}</text>
                <text class="memory-universe__hub-count" y="66">Generated facet · {hub.total} memories</text>
              </g>
            </g>
          )}</For>
          </g>

          <g
            class="memory-universe__core"
            transform={`translate(${UNIVERSE_CENTER.x} ${UNIVERSE_CENTER.y})`}
            role="button"
            tabindex="0"
            aria-label="Current memory focus"
            onPointerDown={(event) => event.stopPropagation()}
            onClick={selectCore}
            onKeyDown={(event) => activate(event, selectCore)}
          >
            <circle class="memory-universe__core-field" r="86" />
            <circle class="memory-universe__core-orbit memory-universe__core-orbit--outer" r="59" />
            <circle class="memory-universe__core-orbit memory-universe__core-orbit--middle" r="45" />
            <circle class="memory-universe__core-orbit memory-universe__core-orbit--inner" r="31" />
            <circle class="memory-universe__core-shell" r="23" />
            <circle class="memory-universe__core-light" r="11" />
            <circle class="memory-universe__core-specular" cx="-4" cy="-5" r="4" />
            <text class="memory-universe__core-title" y="76">CURRENT FOCUS</text>
            <text class="memory-universe__core-subtitle" y="94">Visible memory view</text>
            <text class="memory-universe__core-meta" y="109">{model().memories.length} {model().memories.length === 1 ? "memory" : "memories"} shown</text>
          </g>
        </g>
      </svg>

      <div class="memory-universe__legend" aria-label="Graph legend">
        <span><i class="legend-memory" /> Memory</span>
        <span><i class="legend-navigation" /> Generated navigation facet</span>
        <Show when={model().relationships.some((edge) => !edge.predicted)}>
          <span><i class="legend-stored" /> Stored relationship</span>
        </Show>
        <Show when={model().relationships.some((edge) => edge.predicted)}>
          <span><i class="legend-inferred" /> Inferred candidate</span>
        </Show>
      </div>

      <Show when={graphData.loading()}>
        <div class="memory-universe__loading" role="status"><span /> Mapping memory universe…</div>
      </Show>
      <Show when={graphData.error()}>
        <div class="memory-universe__notice" role="status">
          Some graph data could not be loaded. Only currently shown data is available.
        </div>
      </Show>

      <Show when={showInspector()}>
        <MemoryDetailsPanel
          memory={selectedMemory()}
          hub={selectedHub()}
          relationships={selectedRelationships().length}
          onClose={() => setShowInspector(false)}
          onFocus={() => {
            const memory = selectedMemory();
            const hub = selectedHub();
            if (memory) framePoint(memory.x, memory.y, 1.9);
            else if (hub) framePoint(hub.x, hub.y, 1.55);
          }}
        />
      </Show>

      <Show when={showList()}>
        <div class="memory-universe__list-panel" role="dialog" aria-modal="true" aria-label="Accessible memory list">
          <div class="memory-universe__list-header">
            <div><span>ACCESSIBLE VIEW</span><strong>Memory index</strong></div>
            <button onClick={() => setShowList(false)} aria-label="Close memory list"><Icon name="x" /></button>
          </div>
          <MemoryGraphFallback static={props.static} reason="Accessible table view" />
        </div>
      </Show>
    </section>
  );
}

function MemoryDetailsPanel(props: {
  memory?: UniverseMemory;
  hub?: UniverseHub;
  relationships: number;
  onClose: () => void;
  onFocus: () => void;
}) {
  const title = () => props.memory?.label ?? props.hub?.label ?? "Memory focus";
  const category = () => props.hub?.label ?? "Visible memories";
  const summary = () => props.memory
    ? `${category()} memory with ${props.relationships} visible relationship${props.relationships === 1 ? "" : "s"} in this view.`
    : `${props.hub?.total ?? 0} memories are grouped in this generated navigation facet. Facet membership is not a stored relationship.`;

  return (
    <aside class="memory-inspector" aria-label={`Details for ${title()}`}>
      <header>
        <div class="memory-inspector__eyebrow"><span /> GRAPH DETAILS</div>
        <button onClick={props.onClose} aria-label="Close details panel"><Icon name="x" /></button>
      </header>
      <div class={`memory-inspector__identity tone-${props.hub?.tone ?? "blue"}`}>
        <span class="memory-inspector__identity-icon"><Icon name={props.hub?.icon ?? "brain"} /></span>
        <div><h2>{title()}</h2><p>Generated navigation facet: {category()} · current view</p></div>
      </div>

      <section>
        <h3>Summary</h3>
        <p>{summary()}</p>
      </section>

      <section>
        <h3>Relationships <em>{props.relationships}</em></h3>
        <div class="memory-inspector__chips">
          <Show when={props.hub}><span>generated navigation facet: {category()}</span></Show>
          <Show when={props.relationships > 0}><span>{props.relationships} visible link{props.relationships === 1 ? "" : "s"}</span></Show>
          <Show when={graphData.predicted().length > 0}><span>{graphData.predicted().length} predicted link{graphData.predicted().length === 1 ? "" : "s"}</span></Show>
          <Show when={props.relationships === 0}><span>No visible links</span></Show>
        </div>
      </section>

      <Show when={graphData.predicted().length > 0}>
        <section>
          <h3>Predicted links</h3>
          <For each={graphData.predicted().slice(0, 3)}>{(prediction) => (
            <div class="memory-inspector__prediction">
              <div><strong>{prediction.label}</strong><span>Relative score {prediction.score.toFixed(2)}</span></div>
              <button onClick={() => void graphData.materializePrediction(prediction.target)}>Link</button>
            </div>
          )}</For>
        </section>
      </Show>

      <section>
        <h3>Suggested actions</h3>
        <button class="memory-inspector__action" onClick={props.onFocus}><Icon name="maximize-2" /> Focus this facet</button>
        <Show when={props.memory}>
          <button class="memory-inspector__action" onClick={() => graphData.togglePin(props.memory!.id)}>
            <Icon name="pin" /> {graphData.pinned().has(props.memory!.id) ? "Release position" : "Pin memory"}
          </button>
        </Show>
      </section>
    </aside>
  );
}

export default MemoryUniverse;
