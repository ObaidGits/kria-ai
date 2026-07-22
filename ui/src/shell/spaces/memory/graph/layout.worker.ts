/**
 * layout.worker — dormant Phase 7 graph-layout worker candidate.
 *
 * No shipped Memory Graph route starts this worker. It remains isolated for
 * never block the UI (Req 16). It streams node positions as it converges, then
 * STOPS when the layout settles (SettleTracker) → the scene becomes static with
 * NO perpetual simulation (§5.4 hard rule). The main thread (GraphScene) applies
 * the streamed positions to the instanced meshes.
 *
 * This file is worker-only (imports the worker `self`); its settle DECISION and
 * message protocol live in ./layoutSettle.ts, which is unit-tested directly.
 * The worker is intentionally thin around that tested core.
 */
import createGraph from "ngraph.graph";
import createLayout from "ngraph.forcelayout";
import {
  SettleTracker,
  type LayoutRequest,
  type LayoutResponse,
  type PositionedNode,
} from "./layoutSettle";

// ngraph's layout type is loosely typed; keep a minimal local shape.
interface NgraphLayout {
  step(): void;
  getForceVectorLength?(): number;
  getNodePosition(id: string): { x: number; y: number; z?: number };
  pinNode(node: unknown, isPinned: boolean): void;
  setNodePosition(id: string, x: number, y: number, z?: number): void;
  dispose?(): void;
}

/**
 * Minimal worker-scope surface (avoids pulling the whole "webworker" lib, which
 * conflicts with the DOM lib this project compiles against). Only the members
 * the worker uses are typed.
 */
interface WorkerScope {
  postMessage(message: unknown): void;
  onmessage: ((event: MessageEvent<LayoutRequest>) => void) | null;
  setTimeout(handler: () => void, ms: number): number;
  clearTimeout(id: number): void;
}

const ctx = self as unknown as WorkerScope;

let layout: NgraphLayout | null = null;
let graph: ReturnType<typeof createGraph> | null = null;
let tracker: SettleTracker | null = null;
let rafHandle: number | null = null;
let dims: 2 | 3 = 3;
let nodeIds: string[] = [];

/** Read every node's current position into a serializable batch. */
function readPositions(): PositionedNode[] {
  if (!layout) return [];
  return nodeIds.map((id) => {
    const p = layout!.getNodePosition(id);
    return { id, x: p.x, y: p.y, z: p.z ?? 0 };
  });
}

function post(message: LayoutResponse): void {
  ctx.postMessage(message);
}

/** One animation slice: step the layout, stream positions, stop on settle. */
function pump(): void {
  if (!layout || !tracker) return;

  // A few physics steps per frame keeps convergence quick without starving the
  // worker's message loop.
  const STEPS_PER_FRAME = 2;
  let movement = 0;
  for (let i = 0; i < STEPS_PER_FRAME; i++) {
    layout.step();
    movement = layout.getForceVectorLength ? layout.getForceVectorLength() : 0;
    if (tracker.step(movement)) break;
  }

  const positions = readPositions();

  if (tracker.settled) {
    post({ type: "settled", step: tracker.stepCount, reason: tracker.reason ?? "quiet", positions });
    stopPump();
    return;
  }

  post({ type: "tick", step: tracker.stepCount, positions });
  schedule();
}

function schedule(): void {
  if (typeof requestAnimationFrame === "function") {
    rafHandle = requestAnimationFrame(pump);
  } else {
    // Worker environments without rAF fall back to a macrotask.
    rafHandle = ctx.setTimeout(pump, 16) as unknown as number;
  }
}

function stopPump(): void {
  if (rafHandle != null) {
    if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(rafHandle);
    else ctx.clearTimeout(rafHandle);
    rafHandle = null;
  }
}

function dispose(): void {
  stopPump();
  layout?.dispose?.();
  layout = null;
  graph = null;
  tracker = null;
  nodeIds = [];
}

ctx.onmessage = (event: MessageEvent<LayoutRequest>) => {
  const msg = event.data;
  switch (msg.type) {
    case "start": {
      dispose();
      dims = msg.dimensions ?? 3;
      graph = createGraph();
      nodeIds = msg.nodes.map((n) => n.id);
      for (const n of msg.nodes) graph.addNode(n.id);
      for (const l of msg.links) {
        // Guard against links to nodes not in the batch.
        if (graph.getNode(l.source) && graph.getNode(l.target)) {
          graph.addLink(l.source, l.target);
        }
      }
      layout = createLayout(graph, { dimensions: dims }) as unknown as NgraphLayout;
      // Apply initial pins.
      for (const n of msg.nodes) {
        if (n.pinned) {
          layout.setNodePosition(n.id, n.pinned.x, n.pinned.y, n.pinned.z);
          layout.pinNode(graph.getNode(n.id), true);
        }
      }
      tracker = new SettleTracker({
        epsilon: msg.settle?.epsilon ?? 0.01,
        quietStepsRequired: msg.settle?.quietStepsRequired ?? 8,
        maxSteps: msg.settle?.maxSteps ?? 600,
      });
      schedule();
      break;
    }
    case "pin": {
      if (!layout || !graph) return;
      const node = graph.getNode(msg.id);
      if (!node) return;
      if (msg.position) {
        layout.setNodePosition(msg.id, msg.position.x, msg.position.y, msg.position.z);
        layout.pinNode(node, true);
      } else {
        layout.pinNode(node, false);
      }
      // Re-heat: a pin/unpin during interaction resumes stepping briefly.
      if (tracker) {
        tracker.reset();
        if (rafHandle == null) schedule();
      }
      break;
    }
    case "stop": {
      dispose();
      break;
    }
  }
};

export {};
