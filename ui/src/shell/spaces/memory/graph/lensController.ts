/**
 * lensController — render lifecycle for the 3D Knowledge Graph lens
 * (task 6.4, Req 5.4 / 16.3).
 *
 * Enforces the §5.4 render-governance HARD RULES as a small, GL-free state
 * machine so the lifecycle is unit-testable without WebGL:
 *   • Freeze to a STILL FRAME when idle/unfocused; resume the render loop ONLY
 *     during interaction or while the layout is still streaming.
 *   • When the layout settles the scene is STATIC (no perpetual render loop).
 *   • Reduced-motion → NEVER run an animation loop: render discrete still frames
 *     (on mount / on interaction) only.
 *   • Unload on Space exit: stop the loop and release the render driver.
 *
 * The controller owns WHEN to draw; the caller supplies `render()` (the GL draw
 * call) and a frame scheduler (rAF by default; injectable for tests). It never
 * touches the DOM or GL directly.
 */

export type LensRenderState =
  | "idle" // mounted but nothing scheduled yet
  | "animating" // render loop running (layout streaming or active interaction)
  | "still" // frozen: one static frame drawn, no loop
  | "stopped"; // unmounted / disposed

export interface LensControllerOptions {
  /** Draw exactly one frame (the GL render call). */
  render: () => void;
  /** Release GL/scene resources on unmount (§5.4 unload on exit). */
  dispose?: () => void;
  /** True when the user prefers reduced motion → no animation loop, ever. */
  reducedMotion: boolean;
  /** Idle delay (ms) after last activity before freezing to a still frame. */
  idleFreezeMs?: number;
  /** Frame scheduler (defaults to requestAnimationFrame). */
  raf?: (cb: (t: number) => void) => number;
  /** Frame canceller (defaults to cancelAnimationFrame). */
  caf?: (handle: number) => void;
  /** Clock (defaults to performance.now / Date.now). */
  now?: () => number;
}

const DEFAULT_IDLE_FREEZE_MS = 1500;

function defaultRaf(cb: (t: number) => void): number {
  if (typeof requestAnimationFrame === "function") return requestAnimationFrame(cb);
  return setTimeout(() => cb(defaultNow()), 16) as unknown as number;
}
function defaultCaf(handle: number): void {
  if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(handle);
  else clearTimeout(handle);
}
function defaultNow(): number {
  return typeof performance !== "undefined" && typeof performance.now === "function"
    ? performance.now()
    : Date.now();
}

export class LensController {
  private readonly opts: Required<Omit<LensControllerOptions, "dispose">> &
    Pick<LensControllerOptions, "dispose">;
  private state: LensRenderState = "stopped";
  private loopHandle: number | null = null;
  private lastActivityAt = 0;
  private layoutSettled = false;
  private frames = 0;

  constructor(options: LensControllerOptions) {
    this.opts = {
      render: options.render,
      dispose: options.dispose,
      reducedMotion: options.reducedMotion,
      idleFreezeMs: options.idleFreezeMs ?? DEFAULT_IDLE_FREEZE_MS,
      raf: options.raf ?? defaultRaf,
      caf: options.caf ?? defaultCaf,
      now: options.now ?? defaultNow,
    };
  }

  /** Current render state (for diagnostics / tests). */
  get renderState(): LensRenderState {
    return this.state;
  }

  /** Total frames drawn (for tests). */
  get frameCount(): number {
    return this.frames;
  }

  /** Whether an animation loop is currently scheduled. */
  get isLooping(): boolean {
    return this.loopHandle != null;
  }

  /**
   * Mount the lens. Under reduced-motion we draw a single still frame and stay
   * static. Otherwise we start the render loop (the layout will be streaming).
   */
  mount(): void {
    if (this.state !== "stopped") return;
    this.layoutSettled = false;
    this.lastActivityAt = this.opts.now();
    if (this.opts.reducedMotion) {
      this.drawStill();
      return;
    }
    this.state = "animating";
    this.startLoop();
  }

  /**
   * Record user interaction (orbit/zoom/focus). Resumes the render loop unless
   * reduced-motion is active, in which case we draw one discrete still frame.
   */
  noteInteraction(): void {
    if (this.state === "stopped") return;
    this.lastActivityAt = this.opts.now();
    if (this.opts.reducedMotion) {
      this.drawStill();
      return;
    }
    if (this.state !== "animating") {
      this.state = "animating";
      this.startLoop();
    }
  }

  /** A layout position batch arrived — keep animating to reflect it. */
  noteLayoutTick(): void {
    if (this.state === "stopped" || this.opts.reducedMotion) {
      // Reduced motion: draw the streamed positions as discrete still frames.
      if (this.state !== "stopped") this.drawStill();
      return;
    }
    this.layoutSettled = false;
    this.lastActivityAt = this.opts.now();
    if (this.state !== "animating") {
      this.state = "animating";
      this.startLoop();
    }
  }

  /**
   * The layout settled and STOPPED. The scene is now static; if the user isn't
   * actively interacting, freeze to a still frame (no perpetual loop, §5.4).
   */
  noteLayoutSettled(): void {
    if (this.state === "stopped") return;
    this.layoutSettled = true;
    if (this.opts.reducedMotion) {
      this.drawStill();
      return;
    }
    if (this.opts.now() - this.lastActivityAt >= this.opts.idleFreezeMs) {
      this.freeze();
    }
  }

  /** Explicitly freeze to a still frame now (kill-switch / focus lost). */
  freeze(): void {
    if (this.state === "stopped") return;
    this.stopLoop();
    this.drawStill();
  }

  /** Unmount: stop the loop and release resources (§5.4 unload on exit). */
  unmount(): void {
    if (this.state === "stopped") return;
    this.stopLoop();
    this.state = "stopped";
    this.opts.dispose?.();
  }

  // ── internals ──────────────────────────────────────────────────────────────

  private drawStill(): void {
    this.opts.render();
    this.frames += 1;
    this.state = "still";
  }

  private startLoop(): void {
    if (this.loopHandle != null) return;
    const frame = () => {
      // Guard: a stop/unmount may have raced the scheduled callback.
      if (this.state === "stopped") return;
      this.opts.render();
      this.frames += 1;

      // Idle → freeze once the layout has settled and no recent interaction.
      const idle = this.opts.now() - this.lastActivityAt >= this.opts.idleFreezeMs;
      if (this.layoutSettled && idle) {
        this.freeze();
        return;
      }
      this.loopHandle = this.opts.raf(frame);
    };
    this.loopHandle = this.opts.raf(frame);
  }

  private stopLoop(): void {
    if (this.loopHandle != null) {
      this.opts.caf(this.loopHandle);
      this.loopHandle = null;
    }
  }
}
