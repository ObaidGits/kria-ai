/**
 * CoreShell3D resolver-wiring tests (task 7.2, Req 20.3 / 20.4).
 *
 * These verify the END-TO-END wiring between the render-mode resolver
 * (`platform/coreRenderMode.ts`) and the 3D Core: with NO explicit `enabled`
 * prop the component follows the live resolver decision, so:
 *   - a gate pass flips it to 3D (one WebGL surface mounts);
 *   - every degrade trigger (reduced-motion, no-WebGL, low-power, failed-gate,
 *     runtime frame-drop) flips it back to the first-class 2D Core AND tears the
 *     WebGL context down (dispose) — no reload;
 *   - jsdom / no-WebGL stays 2D.
 *
 * Validates: Requirements 20.3, 20.4
 */
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import { render, cleanup } from "@solidjs/testing-library";
import { CoreShell3D } from "./CoreShell3D";
import { CORE_STATE_LABELS } from "./CorePresence";
import type { CapabilitySnapshot } from "../platform/capabilities";
import {
  initCoreRenderMode,
  setCoreGatePassed,
  setCoreLowPower,
  setCoreReducedMotion,
  reportCoreFrameDrop,
  coreRenderMode,
} from "../platform/coreRenderMode";

// Renderer-factory mock: returns a fake live renderer so we can exercise the
// 3D-mounted path without a GPU (jsdom has no WebGL).
const h = vi.hoisted(() => ({ impl: null as null | ((...args: unknown[]) => unknown) }));
vi.mock("./coreShell3DRenderer", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./coreShell3DRenderer")>();
  return {
    ...actual,
    createCoreShellRenderer: (...args: unknown[]) => (h.impl ? h.impl(...args) : null),
  };
});

function fakeRenderer(overrides: Record<string, unknown> = {}) {
  return {
    gl: {},
    setState: vi.fn(),
    resize: vi.fn(),
    start: vi.fn(),
    stop: vi.fn(),
    pause: vi.fn(),
    resume: vi.fn(),
    isPaused: vi.fn(() => false),
    renderFrame: vi.fn(),
    isRunning: vi.fn(() => true),
    frameIntervalMs: vi.fn(() => 1000 / 45),
    detailLevel: vi.fn(() => 0),
    shedEffects: vi.fn(() => []),
    dispose: vi.fn(),
    ...overrides,
  };
}

function caps(overrides: Partial<CapabilitySnapshot> = {}): CapabilitySnapshot {
  return {
    webglTier: "webgl2",
    hasWebGL: true,
    prefersReducedMotion: false,
    supportsBackdropFilter: true,
    probe: null,
    ...overrides,
  };
}

beforeEach(() => {
  // A fully 3D-capable device with the Core-3D gate passed → enable3D true.
  initCoreRenderMode(caps(), "auto");
  setCoreGatePassed(true);
  h.impl = () => fakeRenderer();
});

afterEach(() => {
  cleanup();
  h.impl = null;
  // Reset the global resolver store to a safe posture between tests.
  initCoreRenderMode(caps(), "auto");
});

describe("CoreShell3D — follows the resolver (no explicit enabled prop)", () => {
  it("gate pass flips the Core to 3D (one WebGL surface mounts)", () => {
    // Start 2D (gate not yet passed), then pass the gate → upgrades to 3D.
    initCoreRenderMode(caps(), "auto"); // gate reset to failed
    const r = fakeRenderer();
    h.impl = () => r;
    const { container } = render(() => <CoreShell3D state="idle" />);
    expect(coreRenderMode().enable3D).toBe(false);
    expect(container.querySelector("canvas")).toBeNull(); // 2D first

    setCoreGatePassed(true); // gate passes
    expect(coreRenderMode().enable3D).toBe(true);
    expect(container.querySelector('[data-render="3d"]')).not.toBeNull();
    expect(container.querySelector("canvas")).not.toBeNull();
    expect(r.start).toHaveBeenCalled();
  });

  it.each<[string, () => void]>([
    ["reduced-motion", () => setCoreReducedMotion(true)],
    ["low-power", () => setCoreLowPower(true)],
    ["failed-gate", () => setCoreGatePassed(false)],
    ["frame-drop", () => reportCoreFrameDrop(true)],
  ])("auto-degrades to the 2D Core and tears down the context on %s", (_trigger, fire) => {
    const r = fakeRenderer();
    h.impl = () => r;
    const { container, getByRole } = render(() => <CoreShell3D state="thinking" />);
    // Mounted in 3D first.
    expect(container.querySelector("canvas")).not.toBeNull();
    expect(r.dispose).not.toHaveBeenCalled();

    fire(); // fire the degrade trigger

    // Resolver flipped to 2D → the 3D Core tore down (context released) and the
    // first-class 2D Core renders in its place — no reload.
    expect(coreRenderMode().enable3D).toBe(false);
    expect(r.dispose).toHaveBeenCalledTimes(1);
    expect(container.querySelector("canvas")).toBeNull();
    expect(getByRole("img").getAttribute("aria-label")).toBe(CORE_STATE_LABELS.thinking);
  });
});

describe("CoreShell3D — jsdom / no-WebGL device stays 2D", () => {
  it("renders the 2D Core (no canvas) when the device has no WebGL", () => {
    initCoreRenderMode(caps({ hasWebGL: false, webglTier: "none" }), "auto");
    setCoreGatePassed(true); // even a 'passed' gate cannot beat the no-webgl trigger
    const { container, getByRole } = render(() => <CoreShell3D state="idle" />);
    expect(coreRenderMode().enable3D).toBe(false);
    expect(container.querySelector("canvas")).toBeNull();
    expect(getByRole("img").getAttribute("aria-label")).toBe(CORE_STATE_LABELS.idle);
  });
});
