/**
 * Privacy-model enforcement tests for the desktop-awareness registry (task 3.8,
 * Req 25.4/25.5). Covers:
 *   • register() structurally rejects forbidden capture / non-allowlisted kinds.
 *   • The default §25.1 catalog is entirely local (all-local processing).
 *   • Signals are ephemeral by default; nothing is rememberable until the user
 *     opts a source into memory, and opting out (source or memory) re-ephemeralizes.
 *   • rememberableSignals() is the memory gate: only opted-in-to-memory sources.
 */
import { describe, expect, it, vi } from "vitest";
import {
  DEFAULT_AWARENESS_SOURCES,
  createDesktopAwarenessRegistry,
  type AwarenessSourceDefinition,
} from "./desktopAwarenessBridge";
import { assertAllLocalIntegrations, ForbiddenCaptureError } from "./awarenessPrivacy";
import type { AwarenessSignal } from "./homeFocusStore";

const wiredSource = (over: Partial<AwarenessSourceDefinition> = {}): AwarenessSourceDefinition => ({
  id: "media",
  label: "Music playing",
  purpose: "See what is playing.",
  capability: "desktop",
  integration: "mpris",
  availability: { wayland: "available", x11: "available" },
  confidence: 0.9,
  sourceTrust: 0.85,
  privacyTier: "low",
  degradation: "No MPRIS → nothing.",
  probe: () => true,
  read: (): AwarenessSignal[] => [
    { id: "media:1", capability: "desktop", priority: 20, recency: 10, voiceText: "Now playing." },
  ],
  ...over,
});

function harness() {
  return createDesktopAwarenessRegistry({
    platform: "wayland",
    now: () => 100,
    tauriAvailable: () => true,
    setBridge: vi.fn(),
    clearBridge: vi.fn(),
  });
}

describe("registry — forbidden capture rejection (Req 25.4)", () => {
  it("rejects registering a keylogging source", () => {
    const registry = harness();
    expect(() =>
      registry.register(wiredSource({ id: "evil", integration: "keylog" as never })),
    ).toThrow(ForbiddenCaptureError);
    // Rejected source never enters the registry.
    expect(registry.status("evil")).toBeUndefined();
  });

  it("rejects clipboard / screen-content / file-history / scanning kinds", () => {
    const registry = harness();
    for (const integration of [
      "clipboard-capture",
      "screen-content-capture",
      "file-history",
      "browsing-history",
      "window-scan",
      "process-scan",
    ]) {
      expect(() =>
        registry.register(wiredSource({ id: integration, integration: integration as never })),
      ).toThrow(ForbiddenCaptureError);
    }
  });

  it("accepts a local allowlisted source", () => {
    const registry = harness();
    expect(() => registry.register(wiredSource())).not.toThrow();
    expect(registry.status("media")).toBeDefined();
  });
});

describe("registry — all-local processing (Req 25.5)", () => {
  it("the default §25.1 catalog uses only local allowlisted integrations", () => {
    expect(() =>
      assertAllLocalIntegrations(
        DEFAULT_AWARENESS_SOURCES.map((s) => ({ id: s.id, integration: s.integration })),
      ),
    ).not.toThrow();
  });
});

describe("registry — ephemeral unless opted into memory (Req 25.4)", () => {
  it("defaults every source to not-remembered, even after opt-in", () => {
    const registry = harness();
    registry.register(wiredSource());
    registry.optIn("media");
    expect(registry.isRemembered("media")).toBe(false);
    expect(registry.status("media")?.remembered).toBe(false);
    // Live signals exist, but nothing is rememberable yet (ephemeral).
    expect(registry.bridge.signals()).toHaveLength(1);
    expect(registry.rememberableSignals()).toEqual([]);
  });

  it("cannot remember a source that is not opted in", () => {
    const registry = harness();
    registry.register(wiredSource());
    registry.optInToMemory("media"); // no-op: not enabled
    expect(registry.isRemembered("media")).toBe(false);
  });

  it("remembers signals only after an explicit memory opt-in", () => {
    const registry = harness();
    registry.register(wiredSource());
    registry.optIn("media");
    registry.optInToMemory("media");
    expect(registry.isRemembered("media")).toBe(true);
    const remembered = registry.rememberableSignals();
    expect(remembered).toHaveLength(1);
    expect(remembered[0].id).toBe("media:1");
  });

  it("returns to ephemeral when memory is opted out", () => {
    const registry = harness();
    registry.register(wiredSource());
    registry.optIn("media");
    registry.optInToMemory("media");
    registry.optOutOfMemory("media");
    expect(registry.isRemembered("media")).toBe(false);
    expect(registry.rememberableSignals()).toEqual([]);
  });

  it("opting a source out also stops remembering it", () => {
    const registry = harness();
    registry.register(wiredSource());
    registry.optIn("media");
    registry.optInToMemory("media");
    registry.optOut("media");
    expect(registry.isRemembered("media")).toBe(false);
    // Re-enabling does NOT silently restore the old memory consent.
    registry.optIn("media");
    expect(registry.isRemembered("media")).toBe(false);
  });

  it("only remembers the sources explicitly opted into memory", () => {
    const registry = harness();
    // Two fully-wired sources so both can contribute live signals.
    registry.register(wiredSource({ id: "media", integration: "mpris" }));
    registry.register(
      wiredSource({
        id: "battery",
        integration: "system",
        read: () => [
          { id: "battery:1", capability: "desktop", priority: 10, recency: 5, voiceText: "Battery low." },
        ],
      }),
    );
    registry.optIn("media");
    registry.optIn("battery");
    registry.optInToMemory("battery");
    const remembered = registry.rememberableSignals();
    expect(remembered.map((s) => s.id)).toEqual(["battery:1"]);
  });
});
