/**
 * Tests for the desktop-awareness bridge + signal registry (task 3.7, Req 25).
 *
 * Covers:
 *   • OFF by default → no signals, bridge not wired (Req 25.1).
 *   • Per-source opt-in surfaces a reachable source's signals (Req 25.1/25.3).
 *   • An unavailable / unreachable source is omitted WITHOUT error (Req 25.3/25.6).
 *   • The registry declares availability / privacy tier / confidence per source
 *     (Req 25.2) and the default §25.1 catalog uses only portals/integrations,
 *     never raw scanning (Req 25.3).
 *   • Wiring: the bridge attaches to the Focus engine only while ≥1 source is
 *     opted in, and detaches when the last opts out (Req 25.1 / design §30).
 */
import { describe, expect, it, vi } from "vitest";

import {
  DEFAULT_AWARENESS_SOURCES,
  createDefaultDesktopAwarenessRegistry,
  createDesktopAwarenessRegistry,
  resolvePlatformAvailability,
  type AwarenessSourceDefinition,
} from "./desktopAwarenessBridge";
import type { AwarenessSignal } from "./homeFocusStore";

// A synthetic, fully-wired source (probe reachable + reader) for opt-in tests.
const wiredSource = (
  over: Partial<AwarenessSourceDefinition> = {},
): AwarenessSourceDefinition => ({
  id: "meeting",
  label: "Test meeting",
  purpose: "Remind about a meeting.",
  capability: "calendar",
  integration: "calendar-integration",
  availability: { wayland: "available", x11: "available" },
  confidence: 0.8,
  sourceTrust: 0.9,
  privacyTier: "sensitive",
  degradation: "No meetings without connect.",
  probe: () => true,
  read: (): AwarenessSignal[] => [
    { id: "meeting:1", capability: "calendar", priority: 80, recency: 10, voiceText: "Meeting in 20." },
  ],
  ...over,
});

// Injected wiring seam so tests never touch the real Focus engine singleton.
function harness(options: Parameters<typeof createDesktopAwarenessRegistry>[0] = {}) {
  const wire = vi.fn();
  const unwire = vi.fn();
  const registry = createDesktopAwarenessRegistry({
    platform: "wayland",
    now: () => 100,
    tauriAvailable: () => true,
    setBridge: wire,
    clearBridge: unwire,
    ...options,
  });
  return { registry, wire, unwire };
}

describe("desktop-awareness bridge — OFF by default (Req 25.1)", () => {
  it("emits no signals and does not wire the bridge before any opt-in", () => {
    const { registry, wire } = harness();
    registry.register(wiredSource());
    expect(registry.isEnabled("meeting")).toBe(false);
    expect(registry.enabledCount).toBe(0);
    expect(registry.bridge.signals()).toEqual([]);
    expect(wire).not.toHaveBeenCalled();
  });

  it("keeps the default §25.1 catalog fully OFF and contributing nothing", () => {
    const registry = createDefaultDesktopAwarenessRegistry({
      platform: "wayland",
      setBridge: vi.fn(),
      clearBridge: vi.fn(),
    });
    expect(registry.enabledCount).toBe(0);
    expect(registry.bridge.signals()).toEqual([]);
    expect(registry.list().every((s) => !s.enabled && !s.contributing)).toBe(true);
  });
});

describe("desktop-awareness bridge — per-source opt-in (Req 25.1/25.3)", () => {
  it("surfaces a reachable source's signals only after opt-in, and wires once", () => {
    const { registry, wire, unwire } = harness();
    registry.register(wiredSource());

    registry.optIn("meeting");
    expect(registry.isEnabled("meeting")).toBe(true);
    expect(wire).toHaveBeenCalledTimes(1);

    const signals = registry.bridge.signals();
    expect(signals).toHaveLength(1);
    expect(signals[0].id).toBe("meeting:1");
    // Source defaults flow onto the mapped signal (honest confidence/trust).
    expect(signals[0].confidence).toBe(0.8);
    expect(signals[0].sourceTrust).toBe(0.9);

    // Opting the last source out detaches the bridge and stops all signals.
    registry.optOut("meeting");
    expect(registry.bridge.signals()).toEqual([]);
    expect(unwire).toHaveBeenCalledTimes(1);
  });

  it("only wires once for multiple sources and unwires on the last opt-out", () => {
    const { registry, wire, unwire } = harness();
    registry.register(wiredSource({ id: "a" }));
    registry.register(wiredSource({ id: "b" }));

    registry.optIn("a");
    registry.optIn("b");
    expect(wire).toHaveBeenCalledTimes(1); // wired once (0 → ≥1), not per source

    registry.optOut("a");
    expect(unwire).not.toHaveBeenCalled(); // still one enabled → stays wired
    registry.optOut("b");
    expect(unwire).toHaveBeenCalledTimes(1); // last opt-out → detached
  });
});

describe("desktop-awareness bridge — omit unavailable without error (Req 25.3/25.6)", () => {
  it("omits an opted-in source that is platform-unavailable", () => {
    const { registry } = harness({ platform: "wayland" });
    registry.register(
      wiredSource({ id: "x11only", availability: { wayland: "unavailable", x11: "available" } }),
    );
    registry.optIn("x11only");
    expect(() => registry.bridge.signals()).not.toThrow();
    expect(registry.bridge.signals()).toEqual([]);
    expect(registry.status("x11only")?.contributing).toBe(false);
  });

  it("omits an opted-in but unreachable (declared-but-unwired) source", () => {
    const { registry } = harness();
    registry.register(wiredSource({ id: "unwired", probe: undefined, read: undefined }));
    registry.optIn("unwired");
    expect(registry.bridge.signals()).toEqual([]);
    expect(registry.status("unwired")?.reachable).toBe(false);
  });

  it("degrades a throwing probe/reader to omission rather than propagating", () => {
    const { registry } = harness();
    registry.register(
      wiredSource({ id: "boom", probe: () => { throw new Error("probe failed"); } }),
    );
    registry.register(
      wiredSource({
        id: "reader-boom",
        read: () => { throw new Error("read failed"); },
      }),
    );
    registry.optIn("boom");
    registry.optIn("reader-boom");
    expect(() => registry.bridge.signals()).not.toThrow();
    expect(registry.bridge.signals()).toEqual([]);
  });
});

describe("desktop-awareness signal registry — declared metadata (Req 25.2)", () => {
  it("declares source, Wayland/X11 availability, confidence, privacy tier, degradation", () => {
    for (const def of DEFAULT_AWARENESS_SOURCES) {
      expect(def.id).toBeTruthy();
      expect(def.purpose.length).toBeGreaterThan(0); // plain-language purpose (25.3)
      expect(["low", "medium", "sensitive"]).toContain(def.privacyTier);
      expect(def.confidence).toBeGreaterThanOrEqual(0);
      expect(def.confidence).toBeLessThanOrEqual(1);
      expect(["available", "restricted", "unavailable"]).toContain(def.availability.wayland);
      expect(["available", "restricted", "unavailable"]).toContain(def.availability.x11);
      expect(def.degradation.length).toBeGreaterThan(0);
    }
  });

  it("prefers portals/integrations/system over raw scanning (Req 25.3)", () => {
    const allowed = new Set([
      "calendar-integration",
      "editor-integration",
      "mpris",
      "xdg-portal",
      "pipewire-portal",
      "system",
      "file-watch",
    ]);
    for (const def of DEFAULT_AWARENESS_SOURCES) {
      expect(allowed.has(def.integration)).toBe(true);
    }
  });

  it("notes the Wayland restriction on active-app / window (design §25.1)", () => {
    const activeApp = DEFAULT_AWARENESS_SOURCES.find((s) => s.id === "active-app");
    expect(activeApp?.availability.wayland).toBe("restricted");
  });

  it("resolves platform availability per session type, permissive when unknown", () => {
    const def = wiredSource({ availability: { wayland: "unavailable", x11: "available" } });
    expect(resolvePlatformAvailability(def, "wayland")).toBe("unavailable");
    expect(resolvePlatformAvailability(def, "x11")).toBe("available");
    // Unknown platform takes the more permissive of the two so detection gaps
    // never hide a source — the probe still gates real contribution.
    expect(resolvePlatformAvailability(def, "unknown")).toBe("available");
  });
});
