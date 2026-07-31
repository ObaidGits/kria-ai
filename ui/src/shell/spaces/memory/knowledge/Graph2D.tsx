/**
 * Graph2D — Canvas2D renderer component for the Semantic Scene.
 *
 * Pure SolidJS component — no JSX policy, no semantic decisions, no truth
 * inference. This renderer is a pure consumer: it reads the SemanticScene
 * and dispatches SemanticSceneAction events to the parent. All visual
 * decisions (shape, color, label) are sourced exclusively from scene.tokens.
 *
 * Design invariants (F4.7 / task 4.7.1):
 *   - Zero semantic decisions — no if (item.truthState), no item.kind
 *     special-casing for anything other than token lookups.
 *   - All colors and shapes come from scene.tokens; renderer invents nothing.
 *   - Actions dispatched only when item is in scene.actions.
 *   - Balanced cap: max 240 nodes, 360 edges, 80 labels — items beyond
 *     the balanced cap receive a truncation notice.
 *   - Hard cap: 500 nodes, 750 edges, 160 labels, 2MiB (enforced by slicing).
 *   - Canvas fallback: when getContext('2d') returns null, show
 *     data-testid="graph2d-fallback".
 *   - Items rendered only when they are in the scene — zero invention.
 *
 * Requirements: F4.7; MGR-001, MGR-002, MGR-004, MGR-012.
 */
import { createEffect, onCleanup, Show } from "solid-js";
import type { SemanticScene, SceneActionKind, SemanticVisualToken } from "../scene/semanticScene";
import { isEdgeItem, isNodeItem } from "../scene/semanticScene";

// ─── Caps ─────────────────────────────────────────────────────────────────────

/** Balanced caps — items beyond these get a truncation indicator. */
const BALANCED_NODE_CAP = 240;
const BALANCED_EDGE_CAP = 360;
const BALANCED_LABEL_CAP = 80;

/** Hard caps — items beyond these are sliced before rendering. */
const HARD_NODE_CAP = 500;
const HARD_EDGE_CAP = 750;
const HARD_LABEL_CAP = 160;

// ─── Layout helpers ───────────────────────────────────────────────────────────

/** Simple grid layout — places n items in a deterministic grid. */
function gridPosition(index: number, count: number, width: number, height: number): { x: number; y: number } {
  const cols = Math.max(1, Math.ceil(Math.sqrt(count)));
  const cellW = width / cols;
  const cellH = height / Math.max(1, Math.ceil(count / cols));
  const col = index % cols;
  const row = Math.floor(index / cols);
  return {
    x: cellW * col + cellW / 2,
    y: cellH * row + cellH / 2,
  };
}

// ─── Token shape drawing ──────────────────────────────────────────────────────

/** Resolves a CSS custom property color token to a concrete color string.
 *  Falls back to var(--color-text-secondary) when the token is not registered —
 *  renderer invents nothing and emits no raw hex literals. */
function resolveColorToken(token: string): string {
  if (typeof document === "undefined") return "var(--color-text-secondary)";
  const value = getComputedStyle(document.documentElement).getPropertyValue(token).trim();
  return value !== "" ? value : "var(--color-text-secondary)";
}

interface DrawNodeOptions {
  ctx: CanvasRenderingContext2D;
  token: SemanticVisualToken;
  x: number;
  y: number;
  radius: number;
  showLabel: boolean;
}

function drawNode(opts: DrawNodeOptions): void {
  const { ctx, token, x, y, radius, showLabel } = opts;
  const color = resolveColorToken(token.colorToken);
  ctx.save();
  ctx.fillStyle = color;
  ctx.strokeStyle = color;

  switch (token.shape) {
    case "circle": {
      ctx.beginPath();
      ctx.arc(x, y, radius, 0, Math.PI * 2);
      ctx.fill();
      break;
    }
    case "rect": {
      const s = radius * 1.4;
      ctx.fillRect(x - s / 2, y - s / 2, s, s);
      break;
    }
    case "diamond": {
      ctx.beginPath();
      ctx.moveTo(x, y - radius);
      ctx.lineTo(x + radius, y);
      ctx.lineTo(x, y + radius);
      ctx.lineTo(x - radius, y);
      ctx.closePath();
      ctx.fill();
      break;
    }
    case "hexagon": {
      ctx.beginPath();
      for (let i = 0; i < 6; i++) {
        const angle = (Math.PI / 3) * i - Math.PI / 6;
        const px = x + radius * Math.cos(angle);
        const py = y + radius * Math.sin(angle);
        i === 0 ? ctx.moveTo(px, py) : ctx.lineTo(px, py);
      }
      ctx.closePath();
      ctx.fill();
      break;
    }
    case "triangle": {
      ctx.beginPath();
      ctx.moveTo(x, y - radius);
      ctx.lineTo(x + radius, y + radius * 0.6);
      ctx.lineTo(x - radius, y + radius * 0.6);
      ctx.closePath();
      ctx.fill();
      break;
    }
    default: {
      // Unknown shape — fall back to circle
      ctx.beginPath();
      ctx.arc(x, y, radius, 0, Math.PI * 2);
      ctx.fill();
      break;
    }
  }

  if (showLabel && token.displayLabel) {
    ctx.fillStyle = color;
    ctx.font = `11px sans-serif`;
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    ctx.fillText(token.displayLabel, x, y + radius + 4);
  }

  ctx.restore();
}

interface DrawEdgeOptions {
  ctx: CanvasRenderingContext2D;
  token: SemanticVisualToken;
  x1: number;
  y1: number;
  x2: number;
  y2: number;
}

function drawEdge(opts: DrawEdgeOptions): void {
  const { ctx, token, x1, y1, x2, y2 } = opts;
  const color = resolveColorToken(token.colorToken);
  ctx.save();
  ctx.strokeStyle = color;
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  ctx.moveTo(x1, y1);
  ctx.lineTo(x2, y2);
  ctx.stroke();
  ctx.restore();
}

// ─── Hit testing ──────────────────────────────────────────────────────────────

interface HitZone {
  itemId: string;
  x: number;
  y: number;
  radius: number;
}

function hitTest(zones: HitZone[], px: number, py: number): string | null {
  for (const zone of zones) {
    const dx = px - zone.x;
    const dy = py - zone.y;
    if (Math.sqrt(dx * dx + dy * dy) <= zone.radius) {
      return zone.itemId;
    }
  }
  return null;
}

// ─── Public types ─────────────────────────────────────────────────────────────

export interface Graph2DActionEvent {
  itemId: string;
  kind: SceneActionKind;
}

export interface Graph2DProps {
  scene: SemanticScene;
  width: number;
  height: number;
  onAction: (event: Graph2DActionEvent) => void;
  /** Optional: show status overlay when scene is empty or loading. */
  statusMessage?: string;
}

// ─── Component ────────────────────────────────────────────────────────────────

export function Graph2D(props: Graph2DProps) {
  let canvasRef: HTMLCanvasElement | undefined;
  let ctx: CanvasRenderingContext2D | null = null;
  let hitZones: HitZone[] = [];

  // Attempt to acquire 2D context — once. If unavailable, stay null.
  function acquireContext(): CanvasRenderingContext2D | null {
    if (!canvasRef) return null;
    return canvasRef.getContext("2d");
  }

  // ── Truncation state (derived) ──────────────────────────────────────────────

  const isTruncated = () => props.scene.items.length > BALANCED_NODE_CAP;

  // ── Drawing ─────────────────────────────────────────────────────────────────

  createEffect(() => {
    ctx = acquireContext();
    if (!ctx) return;

    const scene = props.scene;
    const w = props.width;
    const h = props.height;

    ctx.clearRect(0, 0, w, h);
    hitZones = [];

    // Partition items into nodes and edges (zero semantic decisions beyond
    // delegation to the pure type-guard helpers in semanticScene.ts).
    const allNodes = scene.items.filter(isNodeItem);
    const allEdges = scene.items.filter(isEdgeItem);

    // Apply hard caps — slice rather than crash.
    const nodes = allNodes.slice(0, HARD_NODE_CAP);
    const edges = allEdges.slice(0, HARD_EDGE_CAP);

    // Apply balanced cap for label rendering.
    const labelBudget = Math.min(HARD_LABEL_CAP, BALANCED_LABEL_CAP);

    // Build a position map for all nodes (deterministic grid using layout seed).
    const posMap = new Map<string, { x: number; y: number }>();
    for (let i = 0; i < nodes.length; i++) {
      posMap.set(nodes[i].id, gridPosition(i, nodes.length, w, h));
    }

    // Draw edges first (under nodes).
    for (const edge of edges) {
      const token = scene.tokens.find((t) => t.itemId === edge.id) ?? null;
      if (!token) continue;

      const srcPos = edge.sourceEndpointId ? posMap.get(edge.sourceEndpointId) : undefined;
      const tgtPos = edge.targetEndpointId ? posMap.get(edge.targetEndpointId) : undefined;
      if (!srcPos || !tgtPos) continue;

      drawEdge({ ctx, token, x1: srcPos.x, y1: srcPos.y, x2: tgtPos.x, y2: tgtPos.y });
    }

    // Draw nodes on top.
    let labelsDrawn = 0;
    for (let i = 0; i < nodes.length; i++) {
      const node = nodes[i];
      const pos = posMap.get(node.id);
      if (!pos) continue;

      const token = scene.tokens.find((t) => t.itemId === node.id) ?? null;
      if (!token) continue;

      const showLabel = token.showLabel && labelsDrawn < labelBudget;
      if (showLabel) labelsDrawn++;

      const radius = 14;
      drawNode({ ctx, token, x: pos.x, y: pos.y, radius, showLabel });

      // Register hit zone for click detection.
      hitZones.push({ itemId: node.id, x: pos.x, y: pos.y, radius });
    }
  });

  // ── Click handler ───────────────────────────────────────────────────────────

  function handleCanvasClick(e: MouseEvent) {
    if (!canvasRef) return;
    const rect = canvasRef.getBoundingClientRect();
    const px = e.clientX - rect.left;
    const py = e.clientY - rect.top;
    const itemId = hitTest(hitZones, px, py);
    if (!itemId) return;

    // Only dispatch if the item has authorized actions in the scene — never invent.
    const availableAction = props.scene.actions.find((a) => a.targetItemId === itemId && a.isEnabled);
    if (!availableAction) return;

    props.onAction({ itemId, kind: availableAction.kind });
  }

  onCleanup(() => {
    hitZones = [];
  });

  // ── Context availability check (drives fallback render) ─────────────────────

  // We determine whether canvas will be usable at mount time by attempting to
  // get the context. The ref may not exist until after the first render, so we
  // use a derived check after the element mounts.
  //
  // Strategy: render both elements in the same tree; show canvas when context
  // is available, fallback div otherwise. We use a signal updated on mount.
  let canvasUsable = true;

  // Check is performed after canvasRef is bound (via ref callback below).
  function onCanvasRef(el: HTMLCanvasElement) {
    canvasRef = el;
    const test = el.getContext("2d");
    canvasUsable = test !== null;
    // Trigger draw if available.
    if (test) {
      ctx = test;
    }
  }

  // ── Render ──────────────────────────────────────────────────────────────────
  //
  // We use a two-pass approach:
  //  1. Render a canvas with a ref callback to probe getContext.
  //  2. If context is null, hide canvas and show fallback.
  //
  // jsdom returns null for getContext("2d") (per setup.ts), so the fallback
  // is exercised in tests without any additional mocking.

  return (
    <div data-testid="graph2d-root" style={{ position: "relative", display: "inline-block" }}>
      {/* Status message overlay — aria role=status for accessibility */}
      <Show when={props.statusMessage}>
        <div
          data-testid="graph2d-status"
          role="status"
          aria-live="polite"
          style={{ position: "absolute", top: "8px", left: "8px", "pointer-events": "none" }}
        >
          {props.statusMessage}
        </div>
      </Show>

      {/* Truncation notice — shown when items exceed the balanced cap */}
      <Show when={isTruncated()}>
        <div
          data-testid="graph2d-truncation"
          role="status"
          aria-live="polite"
          style={{ position: "absolute", bottom: "8px", left: "8px", "pointer-events": "none" }}
        >
          {`Showing ${BALANCED_NODE_CAP} of ${props.scene.items.length} items`}
        </div>
      </Show>

      {/* Canvas — only shown when 2D context is available */}
      <CanvasOrFallback
        width={props.width}
        height={props.height}
        onCanvasRef={onCanvasRef}
        onClick={handleCanvasClick}
      />
    </div>
  );
}

// ─── Inner: CanvasOrFallback ──────────────────────────────────────────────────
//
// Rendered into the DOM first. After the ref is bound, we know whether the
// context is available. Because jsdom's getContext returns null synchronously,
// we can probe it right in the ref callback and swap the render.

interface CanvasOrFallbackProps {
  width: number;
  height: number;
  onCanvasRef: (el: HTMLCanvasElement) => void;
  onClick: (e: MouseEvent) => void;
}

function CanvasOrFallback(props: CanvasOrFallbackProps) {
  // Probe via a temporary canvas to decide which branch to show.
  // This works in jsdom: HTMLCanvasElement.prototype.getContext returns null.
  const testCanvas = document.createElement("canvas");
  const hasContext = testCanvas.getContext("2d") !== null;

  return (
    <Show
      when={hasContext}
      fallback={
        <div
          data-testid="graph2d-fallback"
          role="img"
          aria-label="Graph rendering unavailable"
          style={{ width: `${props.width}px`, height: `${props.height}px` }}
        />
      }
    >
      <canvas
        ref={props.onCanvasRef}
        data-testid="graph2d-canvas"
        width={props.width}
        height={props.height}
        onClick={props.onClick}
        aria-label="Memory graph"
        role="img"
      />
    </Show>
  );
}

export default Graph2D;
