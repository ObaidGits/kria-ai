/**
 * NodeCanvas — the lightweight, in-house 2D node canvas for the workflow
 * builder (task 7.3, Req 6.4). DOM nodes over an SVG edge layer — NOT the 3D
 * graph engine (that is Memory-only, design.md §6.3). No heavy graph/docking
 * library.
 *
 * Capabilities (kept deliberately simple, Req 6.4):
 *   • Render workflow nodes (absolutely-positioned DOM) + their connections
 *     (SVG lines with an arrowhead).
 *   • Add nodes from the palette (drag-and-drop onto the canvas; click-to-add
 *     lives in NodePalette).
 *   • Move a node — pointer drag, or arrow keys while its button is focused
 *     (Req 17.1 keyboard-operable).
 *   • Connect ports — a per-node "Connect" control starts a connection; picking
 *     a target node completes it (Esc cancels). Keyboard-reachable.
 *   • Select a node — clicking/activating its button selects it and opens the
 *     shared node Inspector (Req 6.3).
 *
 * Accessibility: the canvas is a labelled group; every node is a real
 * `<button>` with an accessible name, focus-visible, and keyboard move/select;
 * selection is announced via `aria-pressed`. A live region narrates connect
 * mode. Reduced-motion is honored (no transitions; CSS).
 *
 * Presentation-only: all edits mutate LOCAL draft state on `automationStore`.
 *
 * Requirements: 6.3, 6.4, 17.1
 */
import { createSignal, For, Show, onCleanup } from "solid-js";
import { Icon } from "../../../components/Icon";
import { IconButton } from "../../../kit";
import { NODE_PALETTE, automationStore } from "../../../stores";
import type { BuilderNode } from "../../../stores";
import "./builder.css";

const NODE_W = 156;
const NODE_H = 60;
const MOVE_STEP = 20;

function iconFor(kind: string): string {
  return NODE_PALETTE.find((p) => p.kind === kind)?.icon ?? "workflow";
}

function center(node: BuilderNode): { x: number; y: number } {
  return { x: node.x + NODE_W / 2, y: node.y + NODE_H / 2 };
}

export function NodeCanvas() {
  let canvasRef: HTMLDivElement | undefined;
  // The node a pending connection starts from (connect mode), or null.
  const [connectingFrom, setConnectingFrom] = createSignal<string | null>(null);
  // Live-region message for connect mode (Req 17.2).
  const [liveMessage, setLiveMessage] = createSignal("");
  // Pointer-drag bookkeeping.
  let dragId: string | null = null;
  let dragOffset = { x: 0, y: 0 };

  const nodes = () => automationStore.builderNodes();
  const edges = () => automationStore.builderEdges();
  const selectedId = () => automationStore.selectedNodeId();

  const nameOf = (id: string) => nodes().find((n) => n.id === id)?.name ?? id;

  function cancelConnect() {
    if (connectingFrom()) {
      setConnectingFrom(null);
      setLiveMessage("Connection cancelled.");
    }
  }

  function activateNode(node: BuilderNode) {
    const from = connectingFrom();
    if (from && from !== node.id) {
      automationStore.connectNodes(from, node.id);
      setLiveMessage(`Connected ${nameOf(from)} to ${node.name}.`);
      setConnectingFrom(null);
      return;
    }
    automationStore.selectNode(node.id);
  }

  function startConnect(node: BuilderNode) {
    setConnectingFrom(node.id);
    setLiveMessage(`Connecting from ${node.name}. Pick a target node, or press Escape to cancel.`);
  }

  // ── Pointer drag to move ─────────────────────────────────────────────────
  function onNodePointerDown(e: PointerEvent, node: BuilderNode) {
    // Only start a drag from the node body (not its action buttons).
    if ((e.target as HTMLElement).closest("button")?.dataset.nodeAction) return;
    dragId = node.id;
    const rect = canvasRef?.getBoundingClientRect();
    dragOffset = {
      x: e.clientX - (rect?.left ?? 0) - node.x,
      y: e.clientY - (rect?.top ?? 0) - node.y,
    };
  }

  function onCanvasPointerMove(e: PointerEvent) {
    if (!dragId || !canvasRef) return;
    const rect = canvasRef.getBoundingClientRect();
    const x = Math.max(0, e.clientX - rect.left - dragOffset.x);
    const y = Math.max(0, e.clientY - rect.top - dragOffset.y);
    automationStore.moveNode(dragId, x, y);
  }

  function endDrag() {
    dragId = null;
  }

  // ── Keyboard move / select / connect / delete ────────────────────────────
  function onNodeKeyDown(e: KeyboardEvent, node: BuilderNode) {
    switch (e.key) {
      case "ArrowLeft":
        e.preventDefault();
        automationStore.moveNode(node.id, Math.max(0, node.x - MOVE_STEP), node.y);
        break;
      case "ArrowRight":
        e.preventDefault();
        automationStore.moveNode(node.id, node.x + MOVE_STEP, node.y);
        break;
      case "ArrowUp":
        e.preventDefault();
        automationStore.moveNode(node.id, node.x, Math.max(0, node.y - MOVE_STEP));
        break;
      case "ArrowDown":
        e.preventDefault();
        automationStore.moveNode(node.id, node.x, node.y + MOVE_STEP);
        break;
      case "Delete":
      case "Backspace":
        e.preventDefault();
        automationStore.removeNode(node.id);
        setLiveMessage(`Removed ${node.name}.`);
        break;
      case "Escape":
        cancelConnect();
        break;
      // Enter/Space activate the button natively → onClick (select/connect).
    }
  }

  // ── Palette drag-and-drop onto the canvas ────────────────────────────────
  function onDrop(e: DragEvent) {
    e.preventDefault();
    const kind = e.dataTransfer?.getData("application/x-kria-node");
    if (!kind || !canvasRef) return;
    const rect = canvasRef.getBoundingClientRect();
    automationStore.addNode(kind, {
      x: Math.max(0, e.clientX - rect.left - NODE_W / 2),
      y: Math.max(0, e.clientY - rect.top - NODE_H / 2),
    });
  }

  const onKeyDownGlobal = (e: KeyboardEvent) => {
    if (e.key === "Escape") cancelConnect();
  };
  document.addEventListener("keydown", onKeyDownGlobal);
  onCleanup(() => document.removeEventListener("keydown", onKeyDownGlobal));

  return (
    <div
      ref={canvasRef}
      class="kria-nb-canvas"
      role="group"
      aria-label="Workflow node canvas"
      data-connecting={connectingFrom() ? "true" : undefined}
      onPointerMove={onCanvasPointerMove}
      onPointerUp={endDrag}
      onPointerLeave={endDrag}
      onDragOver={(e) => {
        e.preventDefault();
        if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
      }}
      onDrop={onDrop}
    >
      <p class="kria-nb-canvas__live" role="status" aria-live="polite">
        {liveMessage()}
      </p>

      {/* Edge layer (SVG, behind the nodes). Decorative — the connections are
          also listed textually in the node Inspector for AT. */}
      <svg class="kria-nb-canvas__edges" aria-hidden="true">
        <defs>
          <marker
            id="kria-nb-arrow"
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="7"
            markerHeight="7"
            orient="auto-start-reverse"
          >
            <path d="M 0 0 L 10 5 L 0 10 z" fill="var(--color-border-strong)" />
          </marker>
        </defs>
        <For each={edges()}>
          {(edge) => {
            const src = () => nodes().find((n) => n.id === edge.source);
            const dst = () => nodes().find((n) => n.id === edge.target);
            return (
              <Show when={src() && dst()}>
                <line
                  class="kria-nb-canvas__edge"
                  x1={center(src()!).x}
                  y1={center(src()!).y}
                  x2={center(dst()!).x}
                  y2={center(dst()!).y}
                  marker-end="url(#kria-nb-arrow)"
                />
              </Show>
            );
          }}
        </For>
      </svg>

      {/* Node layer */}
      <For each={nodes()}>
        {(node) => (
          <div
            class="kria-nb-node"
            classList={{
              "kria-nb-node--selected": selectedId() === node.id,
              "kria-nb-node--connect-source": connectingFrom() === node.id,
            }}
            style={{ left: `${node.x}px`, top: `${node.y}px`, width: `${NODE_W}px` }}
            data-node-id={node.id}
            onPointerDown={(e) => onNodePointerDown(e, node)}
          >
            <button
              type="button"
              class="kria-nb-node__body"
              aria-pressed={selectedId() === node.id}
              aria-label={
                connectingFrom() && connectingFrom() !== node.id
                  ? `Connect to ${node.name}`
                  : `${node.name} node — select to configure`
              }
              onClick={() => activateNode(node)}
              onKeyDown={(e) => onNodeKeyDown(e, node)}
            >
              <span class="kria-nb-node__icon" aria-hidden="true">
                <Icon name={iconFor(node.kind)} size={15} />
              </span>
              <span class="kria-nb-node__name">{node.name}</span>
            </button>
            <div class="kria-nb-node__actions">
              <IconButton
                icon="git-branch"
                label={`Connect from ${node.name}`}
                size="sm"
                variant="ghost"
                data-node-action="connect"
                onClick={() => startConnect(node)}
              />
              <IconButton
                icon="trash-2"
                label={`Remove ${node.name}`}
                size="sm"
                variant="ghost"
                data-node-action="remove"
                onClick={() => automationStore.removeNode(node.id)}
              />
            </div>
          </div>
        )}
      </For>

      <Show when={nodes().length === 0}>
        <div class="kria-nb-canvas__empty">
          <Icon name="workflow" size={28} aria-hidden="true" />
          <p>Add nodes from the palette to start building. Click a node or drag it here.</p>
        </div>
      </Show>

      <Show when={connectingFrom()}>
        <div class="kria-nb-canvas__connect-banner" role="status">
          <Icon name="git-branch" size={14} aria-hidden="true" />
          <span>Connecting from {nameOf(connectingFrom()!)} — pick a target node.</span>
          <IconButton icon="x" label="Cancel connection" size="sm" onClick={cancelConnect} />
        </div>
      </Show>
    </div>
  );
}

export default NodeCanvas;
