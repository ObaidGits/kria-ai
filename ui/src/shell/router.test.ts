/**
 * Unit tests for the KRIA internal typed router.
 * Covers: route parsing, serialization, navigation, state persistence/restore, deep-link.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";

// We need to test the module in isolation — re-import per test group where needed.
// For pure functions, direct import is fine.
import {
  parseRoute,
  routeToPath,
  isValidSpace,
  routesEqual,
  ALL_SPACES,
  type Route,
  type Space,
} from "./router";

// ─── Route Parsing ─────────────────────────────────────────────────────────────

describe("parseRoute", () => {
  it("parses a bare space", () => {
    expect(parseRoute("converse")).toEqual({ space: "converse" });
    expect(parseRoute("memory")).toEqual({ space: "memory" });
    expect(parseRoute("settings")).toEqual({ space: "settings" });
  });

  it("parses space/segment", () => {
    expect(parseRoute("converse/thread")).toEqual({
      space: "converse",
      segment: "thread",
    });
    expect(parseRoute("memory/graph")).toEqual({
      space: "memory",
      segment: "graph",
    });
  });

  it("parses space/segment/entityId", () => {
    expect(parseRoute("converse/thread/abc-123")).toEqual({
      space: "converse",
      segment: "thread",
      entityId: "abc-123",
    });
  });

  it("strips leading/trailing slashes", () => {
    expect(parseRoute("/converse/")).toEqual({ space: "converse" });
    expect(parseRoute("///memory/graph///")).toEqual({
      space: "memory",
      segment: "graph",
    });
  });

  it("returns null for invalid space", () => {
    expect(parseRoute("unknown")).toBeNull();
    expect(parseRoute("chat")).toBeNull();
    expect(parseRoute("")).toBeNull();
  });

  it("returns null for too many segments", () => {
    expect(parseRoute("converse/a/b/c")).toBeNull();
  });

  it("returns null for empty string", () => {
    expect(parseRoute("")).toBeNull();
  });
});

// ─── Route Serialization ───────────────────────────────────────────────────────

describe("routeToPath", () => {
  it("serializes a bare space", () => {
    expect(routeToPath({ space: "converse" })).toBe("converse");
  });

  it("serializes space/segment", () => {
    expect(routeToPath({ space: "memory", segment: "graph" })).toBe("memory/graph");
  });

  it("serializes space/segment/entityId", () => {
    expect(
      routeToPath({ space: "converse", segment: "thread", entityId: "xyz" })
    ).toBe("converse/thread/xyz");
  });

  it("ignores entityId when no segment", () => {
    // entityId without segment should not appear in path
    expect(routeToPath({ space: "settings", entityId: "foo" })).toBe("settings");
  });
});

// ─── Route Equality ────────────────────────────────────────────────────────────

describe("routesEqual", () => {
  it("returns true for identical routes", () => {
    const a: Route = { space: "converse", segment: "thread", entityId: "1" };
    expect(routesEqual(a, { ...a })).toBe(true);
  });

  it("returns false for different spaces", () => {
    expect(
      routesEqual({ space: "converse" }, { space: "memory" })
    ).toBe(false);
  });

  it("returns false when one has segment and other does not", () => {
    expect(
      routesEqual({ space: "converse" }, { space: "converse", segment: "thread" })
    ).toBe(false);
  });

  it("treats undefined fields consistently", () => {
    const a: Route = { space: "converse" };
    const b: Route = { space: "converse", segment: undefined, entityId: undefined };
    expect(routesEqual(a, b)).toBe(true);
  });
});

// ─── Space Validation ──────────────────────────────────────────────────────────

describe("isValidSpace", () => {
  it("validates all defined spaces", () => {
    for (const s of ALL_SPACES) {
      expect(isValidSpace(s)).toBe(true);
    }
  });

  it("rejects invalid strings", () => {
    expect(isValidSpace("chat")).toBe(false);
    expect(isValidSpace("")).toBe(false);
    expect(isValidSpace("CONVERSE")).toBe(false);
  });
});

// ─── Navigation & State Persistence ────────────────────────────────────────────

describe("navigation and persistence", () => {
  const STORAGE_KEY = "kria_router_session";

  beforeEach(() => {
    // Clear storage before each test
    window.localStorage.removeItem(STORAGE_KEY);
  });

  it("navigate sets the current route", async () => {
    // Dynamic import to get fresh module state isn't trivial with Solid signals.
    // Instead test via the exported navigate + currentRoute.
    const { navigate, currentRoute } = await import("./router");
    navigate("memory", "graph");
    expect(currentRoute()).toEqual({ space: "memory", segment: "graph" });
  });

  it("navigate with entityId", async () => {
    const { navigate, currentRoute } = await import("./router");
    navigate("converse", "thread", "session-42");
    expect(currentRoute()).toEqual({
      space: "converse",
      segment: "thread",
      entityId: "session-42",
    });
  });

  it("navigateToPath resolves a deep-link", async () => {
    const { navigateToPath, currentRoute } = await import("./router");
    const result = navigateToPath("observatory/diagnostics/cpu");
    expect(result).toBe(true);
    expect(currentRoute()).toEqual({
      space: "observatory",
      segment: "diagnostics",
      entityId: "cpu",
    });
  });

  it("navigateToPath returns false for invalid path", async () => {
    const { navigateToPath } = await import("./router");
    expect(navigateToPath("invalid/a/b/c")).toBe(false);
    expect(navigateToPath("")).toBe(false);
  });

  it("setSpaceState / getSpaceState persists per-Space scroll + selection", async () => {
    const { setSpaceState, getSpaceState } = await import("./router");
    setSpaceState("converse", { scrollTop: 250, selection: "msg-7" });
    const state = getSpaceState("converse");
    expect(state.scrollTop).toBe(250);
    expect(state.selection).toBe("msg-7");
  });

  it("getSpaceState returns defaults for unset space", async () => {
    const { getSpaceState } = await import("./router");
    const state = getSpaceState("automations");
    expect(state.scrollTop).toBe(0);
    expect(state.selection).toBeNull();
  });
});

// ─── State Restoration (localStorage round-trip) ───────────────────────────────

describe("state restoration from localStorage", () => {
  const STORAGE_KEY = "kria_router_session";

  afterEach(() => {
    window.localStorage.removeItem(STORAGE_KEY);
  });

  it("parseRoute + routeToPath round-trip is lossless for all valid forms", () => {
    const cases: Route[] = [
      { space: "converse" },
      { space: "memory", segment: "graph" },
      { space: "machines", segment: "ssh", entityId: "server-1" },
    ];
    for (const route of cases) {
      const path = routeToPath(route);
      const parsed = parseRoute(path);
      expect(parsed).toEqual(route);
    }
  });

  it("persisted session structure is valid JSON", () => {
    const session = {
      route: { space: "converse" as Space, segment: "thread", entityId: "x" },
      spaceStates: {
        converse: { scrollTop: 100, selection: "msg-1" },
        memory: { scrollTop: 0, selection: null },
      },
    };
    const json = JSON.stringify(session);
    const restored = JSON.parse(json);
    expect(restored.route.space).toBe("converse");
    expect(restored.spaceStates.converse.scrollTop).toBe(100);
  });
});

// ─── Hash synchronization lifecycle ──────────────────────────────────────────

describe("hash synchronization", () => {
  it("hydrates from hash, mirrors route changes without churn, and fully disposes", async () => {
    const router = await import("./router");
    window.history.replaceState(null, "", "#/memory/graph/fact-7");

    const replaceSpy = vi.spyOn(window.history, "replaceState");
    const dispose = router.initHashSync();
    expect(router.currentRoute()).toEqual({
      space: "memory",
      segment: "graph",
      entityId: "fact-7",
    });

    replaceSpy.mockClear();
    router.navigate("automations", "run", "workflow-9");
    await Promise.resolve();
    expect(window.location.hash).toBe("#/automations/run/workflow-9");
    expect(replaceSpy).toHaveBeenCalledTimes(1);

    dispose();
    router.navigate("settings");
    await Promise.resolve();
    expect(window.location.hash).toBe("#/automations/run/workflow-9");

    window.history.replaceState(null, "", "#/machines/device/device-1");
    window.dispatchEvent(new HashChangeEvent("hashchange"));
    expect(router.currentRoute()).toEqual({ space: "settings" });
    replaceSpy.mockRestore();
  });

  it("reuses one active hash synchronizer", async () => {
    const router = await import("./router");
    const first = router.initHashSync();
    const second = router.initHashSync();
    expect(second).toBe(first);
    first();
  });
});

// ─── Deep-Link Resolution ──────────────────────────────────────────────────────

describe("deep-link resolution", () => {
  it("all 7 spaces are addressable", () => {
    for (const space of ALL_SPACES) {
      const route = parseRoute(space);
      expect(route).not.toBeNull();
      expect(route!.space).toBe(space);
    }
  });

  it("any space/segment/entity path is addressable", () => {
    const path = "capabilities/installed/skill-abc";
    const route = parseRoute(path);
    expect(route).toEqual({
      space: "capabilities",
      segment: "installed",
      entityId: "skill-abc",
    });
  });
});
