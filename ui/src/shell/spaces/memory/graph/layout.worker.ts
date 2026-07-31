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
 *
 * ── F6.2.2 COMPUTE_Z extension ────────────────────────────────────────────
 * Adds a single-shot COMPUTE_Z message that maps SemanticSceneItems + vector
 * scores to a packed Float32Array of positions and echoes a Z_COMPUTED reply
 * with a transferable buffer. The worker terminates the computation if the
 * generation in the response does not match the latest seen generation, guarding
 * against stale workers that race with new scenes.
 */
import createGraph from "ngraph.graph";
import createLayout from "ngraph.forcelayout";
import {
  SettleTracker,
  type LayoutRequest,
  type LayoutResponse,
  type PositionedNode,
  type ZWorkerRequest,
  type ZWorkerResponse,
} from "./layoutSettle";
import {
  mapZValues,
  packNodePositions,
} from "./graphCanvas3DSpike";
import type { SemanticSceneItem } from "../scene/semanticScene";

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
  postMessage(message: unknown, transfer?: Transferable[]): void;
  onmessage: ((event: MessageEvent<LayoutRequest | ZWorkerRequest>) => void) | null;
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

/**
 * Latest COMPUTE_Z generation seen by this worker.
 * If a new COMPUTE_Z arrives before the previous one is replied, the old
 * result is discarded (generation mismatch guard).
 */
let latestZGeneration = -1;

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

/**
 * Post a Z_COMPUTED message with the positions Float32Array as a transferable.
 * The buffer is zero-copy transferred to the main thread.
 */
function postZComputed(message: ZWorkerResponse): void {
  ctx.postMessage(message, [message.positions.buffer]);
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

ctx.onmessage = (event: MessageEvent<LayoutRequest | ZWorkerRequest>) => {
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
    case "COMPUTE_Z": {
      // Record the latest generation — any earlier COMPUTE_Z is superseded.
      latestZGeneration = msg.generation;
      const currentGeneration = msg.generation;

      // Build the vectorScores Map from the serializable entry array.
      const vectorScores = new Map<string, number>();
      for (const entry of msg.vectorScores) {
        vectorScores.set(entry.id, entry.score);
      }

      // Reconstruct minimal SemanticSceneItem-compatible objects for mapZValues.
      // mapZValues only reads id, kind, and isInPath — we don't need the full item.
      const items = msg.items.map((item) => ({
        id: item.id,
        kind: (item.isNavigationContainer ? 'navigation-container' : 'entity') as SemanticSceneItem['kind'],
        isInPath: item.isInPath,
        // mapZValues only uses id, kind, isInPath — remaining fields are placeholders.
        authorityClass: 'personal' as const,
        label: item.id,
        truthState: 'confirmed',
        graphRevision: 0,
        direction: null,
        sourceEndpointId: null,
        targetEndpointId: null,
        evidenceCount: 0,
        evidenceSummary: null,
        provenance: { sourceId: null, method: null, version: null, actorLabel: null },
        validity: { validTimeStart: null, validTimeEnd: null, isCurrentlyValid: true },
        isSelected: false,
        isFocused: false,
        isPending: false,
        hasError: false,
      }));

      const zValues = mapZValues(items, vectorScores);
      const positions = packNodePositions(items, zValues);

      // Generation mismatch guard: discard result if a newer COMPUTE_Z arrived.
      if (currentGeneration !== latestZGeneration) {
        // Stale — do not post; the newer computation's result will follow.
        break;
      }

      postZComputed({ type: 'Z_COMPUTED', generation: currentGeneration, positions });
      break;
    }
  }
};

export {};
