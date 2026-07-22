/**
 * layoutSettle — pure settle/stop detection for dormant graph layout support.
 *
 * No shipped Memory Graph route consumes this protocol. A future MGR-030
 * the scene then becomes static (no perpetual simulation). ngraph.forcelayout
 * itself lives in the worker (browser/worker-only); this module holds the
 * settle DECISION so it is unit-testable without a Worker or WebGL.
 *
 * A layout is "settled" when the total movement per step stays below an epsilon
 * for a number of consecutive steps, OR a hard max-step ceiling is hit (so a
 * pathological graph can never spin forever).
 */

export interface SettleConfig {
  /** Movement magnitude below which a single step counts as "quiet". */
  epsilon: number;
  /** Consecutive quiet steps required to declare the layout settled. */
  quietStepsRequired: number;
  /** Absolute ceiling on steps — the layout always stops by here. */
  maxSteps: number;
}

export const DEFAULT_SETTLE: SettleConfig = {
  epsilon: 0.01,
  quietStepsRequired: 8,
  maxSteps: 600,
};

/**
 * Tracks layout progress step-by-step and decides when to STOP. Feed it the
 * per-step movement (e.g. ngraph's `getForceVectorLength()` or accumulated
 * position delta); it reports settled once movement is quiet long enough or the
 * step ceiling is reached.
 */
export class SettleTracker {
  private readonly config: SettleConfig;
  private steps = 0;
  private quietRun = 0;
  private done = false;
  private settleReason: "quiet" | "max-steps" | null = null;

  constructor(config: SettleConfig = DEFAULT_SETTLE) {
    this.config = config;
  }

  /** Steps taken so far. */
  get stepCount(): number {
    return this.steps;
  }

  /** Whether the layout has settled/stopped. */
  get settled(): boolean {
    return this.done;
  }

  /** Why it settled ("quiet" = converged, "max-steps" = ceiling), else null. */
  get reason(): "quiet" | "max-steps" | null {
    return this.settleReason;
  }

  /**
   * Record one layout step's movement magnitude and return whether the layout
   * is now settled. Idempotent once settled (always returns true).
   */
  step(movement: number): boolean {
    if (this.done) return true;
    this.steps += 1;

    if (Number.isFinite(movement) && Math.abs(movement) < this.config.epsilon) {
      this.quietRun += 1;
    } else {
      this.quietRun = 0;
    }

    if (this.quietRun >= this.config.quietStepsRequired) {
      this.done = true;
      this.settleReason = "quiet";
    } else if (this.steps >= this.config.maxSteps) {
      this.done = true;
      this.settleReason = "max-steps";
    }
    return this.done;
  }

  /** Reset for a fresh layout run (e.g. after new data / re-layout). */
  reset(): void {
    this.steps = 0;
    this.quietRun = 0;
    this.done = false;
    this.settleReason = null;
  }
}

// ─── Worker message protocol (shared by worker + controller) ─────────────────

/** A node the layout should position (id + optional pin coords). */
export interface LayoutNodeInput {
  id: string;
  /** Pinned world position — the layout must not move a pinned node (§5.4). */
  pinned?: { x: number; y: number; z: number };
}

/** A link the layout should honor. */
export interface LayoutLinkInput {
  source: string;
  target: string;
}

/** Main-thread → worker: start/replace a layout. */
export interface LayoutStartMessage {
  type: "start";
  nodes: LayoutNodeInput[];
  links: LayoutLinkInput[];
  /** Layout in 2 or 3 dimensions (3 for the 3D lens). */
  dimensions?: 2 | 3;
  settle?: Partial<SettleConfig>;
}

/** Main-thread → worker: pin/unpin a node during interaction. */
export interface LayoutPinMessage {
  type: "pin";
  id: string;
  position: { x: number; y: number; z: number } | null;
}

/** Main-thread → worker: stop and dispose the current layout. */
export interface LayoutStopMessage {
  type: "stop";
}

export type LayoutRequest = LayoutStartMessage | LayoutPinMessage | LayoutStopMessage;

/** A single node's resolved position. */
export interface PositionedNode {
  id: string;
  x: number;
  y: number;
  z: number;
}

/** Worker → main-thread: a batch of positions (streamed during layout). */
export interface LayoutTickMessage {
  type: "tick";
  step: number;
  positions: PositionedNode[];
}

/** Worker → main-thread: the layout settled and STOPPED (scene now static). */
export interface LayoutSettledMessage {
  type: "settled";
  step: number;
  reason: "quiet" | "max-steps";
  positions: PositionedNode[];
}

export type LayoutResponse = LayoutTickMessage | LayoutSettledMessage;
