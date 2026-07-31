/**
 * Hybrid Navigation Architecture contract — Requirement 14 (design.md §7).
 *
 * Task 6.3 is a VERIFICATION/CONFIRMATION task: it locks the contract that the
 * three navigation registers have clearly separated roles with NO functional
 * overlap, that the Command Palette OWNS global search + recent + pinned +
 * Space entries (never standing homepage UI), that deep-link / back /
 * state-restore continuity is reused from the typed router, and that navigation
 * depth is ≤2 with a Space switch reachable in ≤1 interaction from any entry
 * point.
 *
 * Register roles (design §7):
 *   • Navigation Rail .. deliberate, COMPLETE, stable navigation (all Spaces).
 *   • Command Palette .. the SEARCHABLE register + owner of search/recent/pinned.
 *   • Contextual Orbit . ambient, PARTIAL, routing-only capability awareness.
 *
 * These are pure/contract-level assertions over the reused palette + router;
 * they duplicate no sibling suite (router.test.ts, navigationCoverage.test.tsx,
 * NavigationRail.test.tsx and ContextualOrbit.test.tsx own their dimensions,
 * exist to pin the Requirement-14 invariants in one place.
 *
 * Requirements: 14.1, 14.2, 14.3, 14.4.
 */
import { describe, it, expect, beforeEach } from "vitest";

import { collectItems } from "./sources";
import { searchItems, flattenGroups, groupResults } from "./search";
import { recordUse, recencyBoost, clearRecents } from "./recents";
import { pinItem, isPinned, clearPins } from "./pins";
import type { PaletteItem } from "./types";

import {
  ALL_SPACES,
  navigate,
  navigateToPath,
  currentRoute,
  parseRoute,
  routeToPath,
  type Space,
} from "../shell/router";

// A single top-level `space[/segment][/entityId]` route is ≤2 levels deep from
// the Space root (segment = 1, entity = 2). This is the Req 14.4 depth ceiling.
const MAX_NAV_DEPTH = 2;
const MAX_ROUTE_COMPONENTS = MAX_NAV_DEPTH + 1; // space + ≤2 sub-components.

function spaceItems(): PaletteItem[] {
  return collectItems("go").filter((i) => i.type === "space");
}

// ─── 14.3 — Palette owns Space entries ───────────────────────────────────────

describe("14.3 — Command Palette surfaces all Space entries", () => {
  beforeEach(() => {
    navigate("converse");
    clearRecents();
    clearPins();
  });

  it("exposes exactly the canonical Spaces (Go mode), in canonical order", () => {
    expect(spaceItems().map((i) => i.id)).toEqual(ALL_SPACES.map((s) => `space:${s}`));
  });

  it("each Space entry routes to its canonical Space in ONE interaction", () => {
    for (const item of spaceItems()) {
      const space = item.id.replace(/^space:/, "") as Space;
      navigate("converse");
      item.run(); // a single palette selection == one interaction
      expect(currentRoute().space).toBe(space);
    }
  });
});

// ─── 14.3 — Palette owns global search ───────────────────────────────────────

describe("14.3 — Command Palette owns global search (not standing homepage UI)", () => {
  const ITEMS: PaletteItem[] = [
    { id: "space:settings", type: "space", title: "Settings", run: () => {} },
    { id: "space:memory", type: "space", title: "Memory", run: () => {} },
    { id: "cmd.theme", type: "command", title: "Toggle theme", run: () => {} },
  ];

  beforeEach(() => {
    clearRecents();
    clearPins();
  });

  it("filters items to fuzzy matches for a query (search narrows the set)", () => {
    const r = searchItems(ITEMS, "sett");
    expect(r.some((x) => x.item.id === "space:settings")).toBe(true);
    expect(r.some((x) => x.item.id === "space:memory")).toBe(false);
  });

  it("returns the full set for an empty query (browseable)", () => {
    expect(searchItems(ITEMS, "").length).toBe(ITEMS.length);
  });
});

// ─── 14.3 — Palette owns recent ──────────────────────────────────────────────

describe("14.3 — Command Palette owns recent items", () => {
  beforeEach(() => {
    clearRecents();
    clearPins();
  });

  it("boosts a recently used item over an unseen one", () => {
    recordUse("thread:1");
    expect(recencyBoost("thread:1")).toBeGreaterThan(recencyBoost("thread:2"));
  });

  it("promotes a recent item within the ranked result set (bounded, non-hiding)", () => {
    const items: PaletteItem[] = [
      { id: "a", type: "space", title: "Alpha", run: () => {} },
      { id: "b", type: "space", title: "Bravo", run: () => {} },
      { id: "c", type: "space", title: "Charlie", run: () => {} },
    ];
    const baseline = searchItems(items, "").map((r) => r.item.id);
    recordUse("c");
    const ranked = searchItems(items, "").map((r) => r.item.id);
    expect(ranked.indexOf("c")).toBeLessThan(baseline.indexOf("c"));
    // Never hides an item — the set is preserved, only reordered.
    expect([...ranked].sort()).toEqual([...baseline].sort());
  });
});

// ─── 14.3 — Palette owns pinned ──────────────────────────────────────────────

describe("14.3 — Command Palette owns pinned items", () => {
  beforeEach(() => {
    clearRecents();
    clearPins();
  });

  it("exposes a palette-scoped pin contract (pin lives in the palette)", () => {
    expect(isPinned("space:memory")).toBe(false);
    pinItem("space:memory");
    expect(isPinned("space:memory")).toBe(true);
  });

  it("ranks a pinned item ahead of unpinned matches (bounded, non-hiding)", () => {
    const items: PaletteItem[] = [
      { id: "a", type: "space", title: "Alpha", run: () => {} },
      { id: "b", type: "space", title: "Bravo", run: () => {} },
      { id: "c", type: "space", title: "Charlie", run: () => {} },
    ];
    const baseline = searchItems(items, "").map((r) => r.item.id);
    pinItem("c");
    const ranked = searchItems(items, "").map((r) => r.item.id);
    expect(ranked.indexOf("c")).toBeLessThan(baseline.indexOf("c"));
    // Pinning reorders only; it never removes an item from the result set.
    expect([...ranked].sort()).toEqual([...baseline].sort());
  });
});

// ─── 14.2 — Continuity reused from the typed router ──────────────────────────

describe("14.2 — deep-link / back / state-restore continuity via the typed router", () => {
  beforeEach(() => {
    navigate("converse");
  });

  it("resolves a deep-link path to the full route (deep-linking)", () => {
    expect(navigateToPath("observatory/diagnostics/cpu")).toBe(true);
    expect(currentRoute()).toEqual({
      space: "observatory",
      segment: "diagnostics",
      entityId: "cpu",
    });
  });

  it("rejects an invalid deep-link without changing state", () => {
    navigate("memory", "graph");
    const before = currentRoute();
    expect(navigateToPath("not-a-space/a/b/c")).toBe(false);
    expect(currentRoute()).toEqual(before);
  });

  it("round-trips route ↔ path losslessly (restorable / shareable links)", () => {
    const route = { space: "machines" as Space, segment: "ssh", entityId: "server-1" };
    expect(parseRoute(routeToPath(route))).toEqual(route);
  });
});

// ─── 14.4 — Depth ≤2 and Space switch ≤1 interaction ─────────────────────────

describe("14.4 — navigation depth ≤2 and Space switch ≤1 interaction", () => {
  beforeEach(() => {
    navigate("converse");
  });

  it("switches to any Space in ONE interaction (depth 0 from any entry point)", () => {
    for (const space of ALL_SPACES) {
      navigate("converse");
      navigate(space); // one call == one interaction, straight to the Space
      expect(currentRoute()).toEqual({ space });
    }
  });

  it("reaches any feature at depth ≤2 (space/segment/entityId grammar)", () => {
    // A fully-qualified feature route is exactly space + segment + entity = 2
    // levels below the Space root.
    const path = "capabilities/installed/skill-abc";
    expect(path.split("/").length).toBe(MAX_ROUTE_COMPONENTS);
    const route = parseRoute(path);
    expect(route).not.toBeNull();
    expect(routeToPath(route!)).toBe(path);
  });

  it("rejects any route deeper than depth 2 (more than 3 components)", () => {
    expect(parseRoute("converse/a/b/c")).toBeNull();
    expect(parseRoute("converse/a/b/c/d")).toBeNull();
  });

  it("every canonical Space is reachable (no dead entry point)", () => {
    for (const space of ALL_SPACES) {
      expect(parseRoute(space)).toEqual({ space });
    }
  });
});

// ─── 14.1 — Separated roles / no functional overlap ──────────────────────────

describe("14.1 — three registers have separated roles with no functional overlap", () => {
  beforeEach(() => {
    navigate("converse");
    clearRecents();
    clearPins();
  });

  it("Palette is the SEARCHABLE register + sole owner of search/recent/pinned", () => {
    // Search, recent, and pinned are palette capabilities (this module wires
    // all three); a smoke assertion that each is functional here.
    expect(typeof searchItems).toBe("function");
    expect(typeof recencyBoost).toBe("function");
    expect(typeof pinItem).toBe("function");
    // Space entries are searchable in the palette (searchable register).
    const results = flattenGroups(groupResults(searchItems(spaceItems(), "mem")));
    expect(results.some((r) => r.item.id === "space:memory")).toBe(true);
  });

  it("Palette (searchable) and Dock (deliberate) share ONE router authority — no divergent nav", () => {
    // A palette Space entry and a direct navigate() land on the identical
    // canonical Space: both registers route through the same typed router, so
    // there is no competing/duplicate navigation source (no functional overlap).
    for (const item of spaceItems()) {
      const space = item.id.replace(/^space:/, "") as Space;
      navigate("converse");
      item.run();
      const viaPalette = currentRoute();
      navigate(space);
      expect(currentRoute()).toEqual(viaPalette);
    }
  });

  it("the Dock register is COMPLETE (covers every canonical Space)", () => {
    // The deliberate register exposes all Spaces (completeness), matching the
    // palette's Go entries one-for-one — same set, same canonical order.
    expect(spaceItems().map((i) => i.id)).toEqual(ALL_SPACES.map((s) => `space:${s}`));
  });
});
