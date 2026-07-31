import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GeometryMonitor, WindowGeometry } from "./windowGeometry";

// ─── Native window / dpi doubles ────────────────────────────────────────────────
// The manager talks to the desktop through Tauri's optional window APIs. We mock
// them so the exact Section-10 transition semantics can be verified without a
// live Tauri/Wayland target. Fields not modelled here (multi-monitor hotplug,
// real compositor fullscreen) are recorded as deferred platform checks in the
// task evidence note rather than faked into a false success.
const h = vi.hoisted(() => {
  const calls = {
    setFullscreen: [] as boolean[],
    setSize: [] as Array<{ width: number; height: number }>,
    setPosition: [] as Array<{ x: number; y: number }>,
  };
  const state = {
    fullscreen: false,
    position: { x: 100, y: 80 },
    size: { width: 1200, height: 800 },
    scaleFactor: 1,
    setFullscreenError: null as Error | null,
    setSizeError: null as Error | null,
    monitors: [
      { workArea: { position: { x: 0, y: 0 }, size: { width: 1920, height: 1040 } }, scaleFactor: 1 },
    ] as GeometryMonitor[],
  };
  const fakeWindow = {
    isFullscreen: async () => state.fullscreen,
    outerPosition: async () => ({ ...state.position }),
    outerSize: async () => ({ ...state.size }),
    scaleFactor: async () => state.scaleFactor,
    setFullscreen: async (value: boolean) => {
      calls.setFullscreen.push(value);
      if (state.setFullscreenError) throw state.setFullscreenError;
      state.fullscreen = value;
    },
    setSize: async (size: { width: number; height: number }) => {
      calls.setSize.push({ width: size.width, height: size.height });
      if (state.setSizeError) throw state.setSizeError;
    },
    setPosition: async (pos: { x: number; y: number }) => {
      calls.setPosition.push({ x: pos.x, y: pos.y });
    },
    onMoved: async () => () => {},
    onResized: async () => () => {},
  };
  return { calls, state, fakeWindow };
});

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => h.fakeWindow,
  availableMonitors: async () => h.state.monitors,
  currentMonitor: async () => h.state.monitors[0] ?? null,
}));

vi.mock("@tauri-apps/api/dpi", () => ({
  PhysicalSize: class {
    constructor(public width: number, public height: number) {}
  },
  PhysicalPosition: class {
    constructor(public x: number, public y: number) {}
  },
}));

import { shellStore } from "../stores/shellStore";
import {
  miniDefault,
  disposeWindowModeManager,
  initWindowModeManager,
  planNativeTransition,
} from "./windowModeManager";

const STORAGE_KEY = "kria_window_geometry_v2";
const MONITOR: GeometryMonitor = h.state.monitors[0];
const MONITORS: readonly GeometryMonitor[] = [MONITOR];

function setInternals(present: boolean) {
  const w = window as unknown as Record<string, unknown>;
  if (present) w.__TAURI_INTERNALS__ = {};
  else delete w.__TAURI_INTERNALS__;
}

/**
 * Deterministic Section-10 transition table, isolated from the async window so
 * every geometry/fullscreen outcome is provable without a desktop target.
 *
 * Validates: Requirements 10.8, 10.9, 10.10, 10.11
 */
describe("planNativeTransition — Section 10 transition table", () => {
  const savedStandard: WindowGeometry = { x: 200, y: 120, width: 1000, height: 720, scaleFactor: 1 };

  it("Standard/Mini → Immersive requests fullscreen when windowed", () => {
    expect(planNativeTransition("immersive", false, undefined, MONITORS, MONITOR)).toEqual({
      fullscreen: true,
    });
  });

  it("→ Immersive leaves fullscreen untouched when already fullscreen (no redundant native call)", () => {
    expect(planNativeTransition("immersive", true, undefined, MONITORS, MONITOR)).toEqual({});
  });

  it("Immersive → Standard exits fullscreen and restores saved Standard geometry", () => {
    const plan = planNativeTransition("standard", true, savedStandard, MONITORS, MONITOR);
    expect(plan.fullscreen).toBe(false);
    expect(plan.geometry).toEqual(savedStandard);
  });

  it("→ Standard with no saved geometry exits fullscreen but leaves geometry untouched", () => {
    expect(planNativeTransition("standard", true, undefined, MONITORS, MONITOR)).toEqual({
      fullscreen: false,
    });
  });

  it("Standard → Mini keeps windowed state and derives the Mini fallback when unsaved", () => {
    const plan = planNativeTransition("mini", false, undefined, MONITORS, MONITOR);
    expect(plan.fullscreen).toBeUndefined();
    expect(plan.geometry).toEqual(miniDefault(MONITOR));
  });

  it("Mini → Mini prefers saved geometry over the derived fallback", () => {
    const savedMini: WindowGeometry = { x: 1300, y: 150, width: 600, height: 760, scaleFactor: 1 };
    const plan = planNativeTransition("mini", false, savedMini, MONITORS, MONITOR);
    expect(plan.geometry).toEqual(savedMini);
  });

  it("Mini → Standard (windowed) with no saved Standard geometry leaves geometry untouched", () => {
    // Not fullscreen and no saved standard geometry → no fullscreen toggle and
    // no geometry request: the current windowed placement is preserved.
    expect(planNativeTransition("standard", false, undefined, MONITORS, MONITOR)).toEqual({});
  });

  it("with no available monitor still exits fullscreen but requests no geometry (native failure is not domain authority)", () => {
    expect(planNativeTransition("standard", true, savedStandard, [], null)).toEqual({ fullscreen: false });
    // Immersive stays a pure fullscreen request even with no monitor data.
    expect(planNativeTransition("immersive", false, undefined, [], null)).toEqual({ fullscreen: true });
  });
});

/**
 * Mini fallback geometry: one quarter of the work area (50% × 50%), practical
 * 400×320 CSS-pixel minimums, right-margin cap 24 CSS px, monitor-anchored.
 *
 * Validates: Requirements 10.9
 */
describe("miniDefault — Mini fallback geometry", () => {
  it("derives quarter-screen right-anchored geometry with a 24px margin cap", () => {
    expect(miniDefault(MONITOR)).toEqual({ x: 936, y: 260, width: 960, height: 520, scaleFactor: 1 });
  });

  it("honours the 400×320 CSS-px minimums, scaled by the monitor scale factor", () => {
    const small: GeometryMonitor = {
      workArea: { position: { x: 0, y: 0 }, size: { width: 900, height: 600 } },
      scaleFactor: 2,
    };
    const geo = miniDefault(small);
    // 50% of 900 = 450 < 400*2=800 → clamped up to 800.
    expect(geo.width).toBe(800);
    // 50% of 600 = 300 < 320*2=640 → clamped up but bounded by work height.
    expect(geo.height).toBe(600);
    // Margin cannot exceed the remaining horizontal space.
    expect(geo.x).toBe(900 - geo.width - Math.min(24 * 2, 900 - geo.width));
  });
});

/**
 * Manager reacting to shellStore mode changes and driving native presentation
 * as an enhancement. shellStore stays the presentation authority; the native
 * layer never fabricates success.
 *
 * Validates: Requirements 10.8, 10.9, 10.10, 10.11
 */
describe("windowModeManager — native transition side effects", () => {
  beforeEach(() => {
    disposeWindowModeManager();
    window.localStorage.clear();
    h.calls.setFullscreen.length = 0;
    h.calls.setSize.length = 0;
    h.calls.setPosition.length = 0;
    h.state.fullscreen = false;
    h.state.setFullscreenError = null;
    h.state.setSizeError = null;
    h.state.position = { x: 100, y: 80 };
    h.state.size = { width: 1200, height: 800 };
    h.state.scaleFactor = 1;
    setInternals(true);
    shellStore.setWindowMode("standard");
  });

  afterEach(() => {
    disposeWindowModeManager();
    setInternals(false);
    shellStore.setWindowMode("standard");
    window.localStorage.clear();
  });

  it("requests native fullscreen when entering Immersive", async () => {
    initWindowModeManager();
    shellStore.setWindowMode("immersive");

    await vi.waitFor(() => expect(h.calls.setFullscreen).toContain(true));
    expect(shellStore.windowMode()).toBe("immersive");
  });

  it("retains the requested in-app mode and emits no false success when native fullscreen fails", async () => {
    h.state.setFullscreenError = new Error("compositor rejected fullscreen");
    initWindowModeManager();

    shellStore.setWindowMode("immersive");

    await vi.waitFor(() => expect(h.calls.setFullscreen).toContain(true));
    // In-app composition remains the requested mode (presentation authority)…
    expect(shellStore.windowMode()).toBe("immersive");
    // …and no false native-success state was recorded.
    expect(h.state.fullscreen).toBe(false);
  });

  it("captures the outgoing Standard geometry and restores the Mini fallback on Standard → Mini", async () => {
    initWindowModeManager();

    shellStore.setWindowMode("mini");

    await vi.waitFor(() => expect(h.calls.setSize.length).toBeGreaterThan(0));

    const stored = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "{}");
    expect(stored.standard).toEqual({ x: 100, y: 80, width: 1200, height: 800, scaleFactor: 1 });

    const fallback = miniDefault(MONITOR);
    expect(h.calls.setSize[h.calls.setSize.length - 1]).toEqual({ width: fallback.width, height: fallback.height });
    expect(h.calls.setPosition[h.calls.setPosition.length - 1]).toEqual({ x: fallback.x, y: fallback.y });
  });

  it("retains the requested in-app mode and emits no false success when native geometry fails", async () => {
    h.state.setSizeError = new Error("compositor rejected resize");
    initWindowModeManager();

    shellStore.setWindowMode("mini");

    // The native geometry call is attempted…
    await vi.waitFor(() => expect(h.calls.setSize.length).toBeGreaterThan(0));
    // …but a native failure never overrides presentation authority: the
    // requested in-app mode stands and nothing throws out of the operation.
    expect(shellStore.windowMode()).toBe("mini");
  });

  it("exits fullscreen and restores saved Standard geometry on Immersive → Standard", async () => {
    const savedStandard: WindowGeometry = { x: 200, y: 120, width: 1000, height: 720, scaleFactor: 1 };
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify({ standard: savedStandard }));
    h.state.fullscreen = true;
    shellStore.setWindowMode("immersive");
    initWindowModeManager();

    shellStore.setWindowMode("standard");

    await vi.waitFor(() => expect(h.calls.setSize.length).toBeGreaterThan(0));
    expect(h.calls.setFullscreen).toContain(false);
    expect(h.calls.setSize[h.calls.setSize.length - 1]).toEqual({ width: 1000, height: 720 });
    expect(h.calls.setPosition[h.calls.setPosition.length - 1]).toEqual({ x: 200, y: 120 });
  });
});
