/**
 * Dormant lens-controller utility tests. They verify lifecycle mechanics only;
 * they do not prove a Memory Graph renderer mount or shipped 3D capability.
 * behavior is deterministic without WebGL or real rAF.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { LensController } from "./lensController";

interface Harness {
  controller: LensController;
  advance(ms: number): void;
  flushFrame(): void;
  pendingFrames(): number;
  renders(): number;
  disposes(): number;
}

function makeHarness(reducedMotion: boolean, idleFreezeMs = 1000): Harness {
  let time = 0;
  let nextId = 1;
  const pending = new Map<number, (t: number) => void>();
  let renderCount = 0;
  let disposeCount = 0;

  const controller = new LensController({
    reducedMotion,
    idleFreezeMs,
    render: () => {
      renderCount += 1;
    },
    dispose: () => {
      disposeCount += 1;
    },
    raf: (cb) => {
      const id = nextId++;
      pending.set(id, cb);
      return id;
    },
    caf: (id) => {
      pending.delete(id);
    },
    now: () => time,
  });

  return {
    controller,
    advance: (ms) => {
      time += ms;
    },
    flushFrame: () => {
      const next = [...pending.keys()].sort((a, b) => a - b)[0];
      if (next == null) return;
      const cb = pending.get(next)!;
      pending.delete(next);
      cb(time);
    },
    pendingFrames: () => pending.size,
    renders: () => renderCount,
    disposes: () => disposeCount,
  };
}

describe("LensController — reduced-motion (no animation loop, §5.4)", () => {
  let h: Harness;
  beforeEach(() => {
    h = makeHarness(true);
  });

  it("draws a single still frame on mount and starts no loop", () => {
    h.controller.mount();
    expect(h.controller.renderState).toBe("still");
    expect(h.controller.isLooping).toBe(false);
    expect(h.renders()).toBe(1);
    expect(h.pendingFrames()).toBe(0);
  });

  it("draws a discrete still frame on interaction (still no loop)", () => {
    h.controller.mount();
    h.controller.noteInteraction();
    expect(h.controller.isLooping).toBe(false);
    expect(h.renders()).toBe(2);
  });

  it("draws streamed layout ticks as discrete still frames", () => {
    h.controller.mount();
    h.controller.noteLayoutTick();
    expect(h.controller.isLooping).toBe(false);
    expect(h.renders()).toBe(2);
  });
});

describe("LensController — animated (loop + freeze, §5.4)", () => {
  let h: Harness;
  beforeEach(() => {
    h = makeHarness(false, 1000);
  });

  it("starts the render loop on mount", () => {
    h.controller.mount();
    expect(h.controller.renderState).toBe("animating");
    expect(h.controller.isLooping).toBe(true);
    expect(h.pendingFrames()).toBe(1);
  });

  it("keeps looping while the layout is still streaming", () => {
    h.controller.mount();
    h.controller.noteLayoutTick();
    h.flushFrame();
    expect(h.controller.isLooping).toBe(true);
    expect(h.renders()).toBeGreaterThanOrEqual(1);
  });

  it("freezes to a still frame when idle after the layout settles", () => {
    h.controller.mount();
    // Layout settles while activity is recent → does NOT freeze yet.
    h.controller.noteLayoutSettled();
    expect(h.controller.renderState).toBe("animating");
    // Idle time elapses; the next frame observes idle+settled → freeze.
    h.advance(1500);
    h.flushFrame();
    expect(h.controller.renderState).toBe("still");
    expect(h.controller.isLooping).toBe(false);
    expect(h.pendingFrames()).toBe(0);
  });

  it("freezes immediately if the layout settles after an idle gap", () => {
    h.controller.mount();
    h.advance(1500); // no interaction for a while
    h.controller.noteLayoutSettled();
    expect(h.controller.renderState).toBe("still");
    expect(h.controller.isLooping).toBe(false);
  });

  it("resumes the loop on interaction after freezing", () => {
    h.controller.mount();
    h.advance(1500);
    h.controller.noteLayoutSettled(); // frozen
    expect(h.controller.isLooping).toBe(false);
    h.controller.noteInteraction();
    expect(h.controller.renderState).toBe("animating");
    expect(h.controller.isLooping).toBe(true);
  });
});

describe("LensController — unload on exit (§5.4)", () => {
  it("stops the loop and disposes resources on unmount", () => {
    const h = makeHarness(false);
    h.controller.mount();
    expect(h.controller.isLooping).toBe(true);
    h.controller.unmount();
    expect(h.controller.renderState).toBe("stopped");
    expect(h.controller.isLooping).toBe(false);
    expect(h.disposes()).toBe(1);
    expect(h.pendingFrames()).toBe(0);
  });

  it("ignores lifecycle calls after unmount", () => {
    const h = makeHarness(false);
    h.controller.mount();
    h.controller.unmount();
    const rendersAfter = h.renders();
    h.controller.noteInteraction();
    h.controller.noteLayoutTick();
    h.controller.noteLayoutSettled();
    expect(h.controller.renderState).toBe("stopped");
    expect(h.renders()).toBe(rendersAfter);
  });
});
