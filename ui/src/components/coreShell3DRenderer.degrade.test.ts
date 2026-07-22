/**
 * coreShell3DRenderer degrade-wiring tests (task 7.2, Req 17.3 / 20.4).
 *
 * Verifies the resolver-driven budget & degradation the renderer now owns:
 *   - the shed ladder sheds effects in the EXACT mandated order
 *     particles → filament → parallax → (last) breath, then raises frameDrop;
 *   - the render loop is fps-capped to the 30–45 window (bounded frame interval);
 *   - the loop pauses on window blur and resumes on focus (idle-quiet at rest).
 *
 * Validates: Requirements 17.3, 20.4
 */
import { describe, it, expect, vi, afterEach } from "vitest";
import {
  CORE_FPS_CAP,
  CORE_FRAME_INTERVAL_MS,
  CORE_SHED_ORDER,
  createCoreShellRenderer,
  createShedController,
} from "./coreShell3DRenderer";

// A minimal mock WebGL context: every method is a no-op returning a truthy
// handle; getExtension yields the lose-context stub (mirrors the sibling test).
function makeMockGl() {
  const loseContext = vi.fn();
  const getExtension = vi.fn((name: string) =>
    name === "WEBGL_lose_context" ? { loseContext } : null,
  );
  const gl = new Proxy(
    {},
    {
      get(_t, prop) {
        if (prop === "getExtension") return getExtension;
        return () => ({});
      },
    },
  ) as unknown as WebGLRenderingContext;
  return { gl, loseContext };
}

function makeCanvas(gl: WebGLRenderingContext) {
  return {
    getContext: vi.fn(() => gl),
    style: { getPropertyValue: () => "" },
    width: 0,
    height: 0,
    clientWidth: 32,
    clientHeight: 32,
  } as unknown as HTMLCanvasElement;
}

/** A fake window that records listeners and can dispatch them synchronously. */
function makeFakeWindow() {
  const listeners: Record<string, Array<() => void>> = {};
  return {
    addEventListener: (type: string, cb: () => void) => {
      (listeners[type] ??= []).push(cb);
    },
    removeEventListener: (type: string, cb: () => void) => {
      listeners[type] = (listeners[type] ?? []).filter((f) => f !== cb);
    },
    dispatch: (type: string) => {
      (listeners[type] ?? []).slice().forEach((f) => f());
    },
    count: (type: string) => (listeners[type] ?? []).length,
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

// ── Shed ladder: particles → filament → parallax → (last) breath → frameDrop ─
describe("createShedController — sheds effects in the mandated order (Req 17.3)", () => {
  it("declares the exact shed order", () => {
    expect(CORE_SHED_ORDER).toEqual(["particles", "filament", "parallax", "breath"]);
  });

  it("sheds one effect per sustained-slow step, breath last, then frame-drops", () => {
    // sustainSamples: 1 → each slow sample acts immediately (deterministic).
    const shed = createShedController({ sustainSamples: 1, shedFps: 30, recoverFps: 43 });
    expect(shed.state()).toEqual({ level: 0, frameDrop: false });
    expect(shed.shedEffects()).toEqual([]);

    shed.sample(20); // step 1 → particles
    expect(shed.shedEffects()).toEqual(["particles"]);

    shed.sample(20); // step 2 → filament detail
    expect(shed.shedEffects()).toEqual(["particles", "filament"]);

    shed.sample(20); // step 3 → parallax
    expect(shed.shedEffects()).toEqual(["particles", "filament", "parallax"]);

    shed.sample(20); // step 4 → breath (LAST)
    expect(shed.shedEffects()).toEqual(["particles", "filament", "parallax", "breath"]);
    expect(shed.state().frameDrop).toBe(false);

    // Ladder exhausted, still slow → frame-drop (host degrades to 2D).
    const after = shed.sample(20);
    expect(after.frameDrop).toBe(true);
    expect(after.level).toBe(CORE_SHED_ORDER.length);
  });

  it("requires SUSTAINED slowness — a single slow frame does not shed", () => {
    const shed = createShedController({ sustainSamples: 30 });
    shed.sample(10); // one slow sample
    expect(shed.state().level).toBe(0);
    shed.sample(60); // recovered → resets the streak
    shed.sample(10);
    expect(shed.state().level).toBe(0);
  });

  it("recovers shed effects (and clears frame-drop first) when comfortably fast", () => {
    const shed = createShedController({ sustainSamples: 1, shedFps: 30, recoverFps: 43 });
    for (let i = 0; i < 5; i++) shed.sample(20); // exhaust ladder + frame-drop
    expect(shed.state()).toEqual({ level: 4, frameDrop: true });

    shed.sample(60); // fast → clears frame-drop first
    expect(shed.state()).toEqual({ level: 4, frameDrop: false });

    shed.sample(60); // fast → restore breath
    expect(shed.shedEffects()).toEqual(["particles", "filament", "parallax"]);
  });
});

// ── fps cap: bounded inter-frame interval (30–45 window) ────────────────────
describe("createCoreShellRenderer — fps cap bounds the frame interval (Req 17.3)", () => {
  it("paces to CORE_FPS_CAP by default (interval === 1000/45)", () => {
    const { gl } = makeMockGl();
    const r = createCoreShellRenderer(makeCanvas(gl), "idle", { autoPauseOnBlur: false });
    expect(r).not.toBeNull();
    expect(r!.frameIntervalMs()).toBeCloseTo(CORE_FRAME_INTERVAL_MS, 5);
    // Never faster than the 45fps cap (interval never below 1000/45).
    expect(r!.frameIntervalMs()).toBeGreaterThanOrEqual(1000 / CORE_FPS_CAP);
    r!.dispose();
  });

  it("clamps a too-high requested fps down to the 45fps cap", () => {
    const { gl } = makeMockGl();
    const r = createCoreShellRenderer(makeCanvas(gl), "idle", {
      autoPauseOnBlur: false,
      maxFps: 120,
    });
    expect(r!.frameIntervalMs()).toBeCloseTo(1000 / CORE_FPS_CAP, 5);
    r!.dispose();
  });
});

// ── Pause on window blur / resume on focus (idle-quiet at rest) ─────────────
describe("createCoreShellRenderer — pauses the loop on blur, resumes on focus (Req 17.3)", () => {
  it("pauses on blur and resumes on focus while still 'running'", () => {
    const { gl } = makeMockGl();
    const win = makeFakeWindow();
    const r = createCoreShellRenderer(makeCanvas(gl), "idle", {
      window: win as unknown as Window,
    });
    expect(r).not.toBeNull();

    r!.start();
    expect(r!.isRunning()).toBe(true);
    expect(r!.isPaused()).toBe(false);
    expect(win.count("blur")).toBe(1); // listener wired

    win.dispatch("blur");
    expect(r!.isPaused()).toBe(true); // loop paused — no perpetual rAF at rest
    expect(r!.isRunning()).toBe(true); // still 'running', just paused

    win.dispatch("focus");
    expect(r!.isPaused()).toBe(false); // resumed

    r!.dispose();
    expect(win.count("blur")).toBe(0); // listeners cleaned up on dispose
  });

  it("starts paused when the document is already hidden", () => {
    const { gl } = makeMockGl();
    const win = makeFakeWindow();
    const r = createCoreShellRenderer(makeCanvas(gl), "idle", {
      window: win as unknown as Window,
      document: { addEventListener() {}, removeEventListener() {}, hidden: true } as unknown as Document,
    });
    r!.start();
    expect(r!.isPaused()).toBe(true);
    r!.dispose();
  });
});
