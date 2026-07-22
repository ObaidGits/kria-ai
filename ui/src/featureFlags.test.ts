import { describe, it, expect, afterEach, beforeEach } from "vitest";
import { createMemo, createRoot } from "solid-js";
import {
  FEATURE_FLAGS,
  isFeatureEnabled,
  setFeatureFlag,
  resetFeatureFlag,
} from "./featureFlags";

const FLAG = "home.presence.v2" as const;

beforeEach(() => {
  try {
    localStorage.clear();
  } catch {
    // ignore
  }
});

afterEach(() => {
  // Restore the module-level singleton to its resolved default so tests stay
  // independent of order.
  try {
    localStorage.clear();
  } catch {
    // ignore
  }
  resetFeatureFlag(FLAG);
});

describe("featureFlags", () => {
  it("defaults home.presence.v2 to ON (Phase-2 exit rollout — task 2.4)", () => {
    // Phase-2 exit rolls out the 2D presence homepage as the default surface.
    // The legacy Converse empty state stays reachable via the override rollback
    // path (Req 22.1), verified below.
    expect(FEATURE_FLAGS[FLAG]).toBe(true);
    expect(isFeatureEnabled(FLAG)).toBe(true);
  });

  it("setFeatureFlag(OFF) rolls back to the legacy empty state and persists the override (Req 22.1/22.2)", () => {
    setFeatureFlag(FLAG, false);
    expect(isFeatureEnabled(FLAG)).toBe(false);
    // The rollback path is a persisted localStorage override — no rebuild.
    expect(localStorage.getItem("kria.flag.home.presence.v2")).toBe("false");
  });

  it("setFeatureFlag(ON) persists the enabled override", () => {
    setFeatureFlag(FLAG, false);
    setFeatureFlag(FLAG, true);
    expect(isFeatureEnabled(FLAG)).toBe(true);
    expect(localStorage.getItem("kria.flag.home.presence.v2")).toBe("true");
  });

  it("resetFeatureFlag clears the override and returns to the default (now ON)", () => {
    setFeatureFlag(FLAG, false);
    expect(isFeatureEnabled(FLAG)).toBe(false);
    resetFeatureFlag(FLAG);
    expect(isFeatureEnabled(FLAG)).toBe(true);
    expect(localStorage.getItem("kria.flag.home.presence.v2")).toBeNull();
  });

  it("is reactive: a tracked read reflects flag flips (live surface swap / rollback)", () => {
    createRoot((dispose) => {
      // A memo depends on the flag; it recomputes on read when the source
      // changed. This mirrors how the home surface's `<Show when={...}>` gate
      // reacts to a rollout/rollback flip.
      const gate = createMemo(() => isFeatureEnabled(FLAG));
      expect(gate()).toBe(true);

      setFeatureFlag(FLAG, false);
      expect(gate()).toBe(false);

      setFeatureFlag(FLAG, true);
      expect(gate()).toBe(true);

      dispose();
    });
  });
});
