/**
 * Navigation test-coverage consolidation (task 7.8; IU-08).
 *
 * Sub-task 7.8 is the consolidation gate for Phase 4 navigation: it ensures a
 * FOCUSED test exists for every named navigation dimension. Most dimensions are
 * already pinned by sibling suites and are referenced (not duplicated) in the
 * evidence note:
 *   - route IDs/order .......... router.test.ts (ALL_SPACES pin) + Dock.test.tsx
 *   - route grammar ............ router.test.ts (parseRoute valid/invalid/hash)
 *   - router-mirror state ...... routerAuthority.test.tsx (convergence)
 *   - shortcuts ................ summon.test.ts (Ctrl/Cmd+K, Ctrl+Shift+P Do)
 *   - active Space ............. Dock.test.tsx (aria-current="page")
 *   - lazy loading ............. spaces/index.test.ts (Converse eager, six lazy)
 *   - long localization ........ Dock.i18n.test.tsx (long label render)
 *
 * This file closes the remaining GAPS with focused tests:
 *   - command references resolve to valid canonical Space IDs (no stale refs),
 *   - a selected SECONDARY route (space/segment[/entityId]) is reflected, the
 *     correct Space renders, and the Dock still marks the PARENT Space active,
 *   - the summon shortcut does not conflict with Space switching,
 *   - the Dock exposes a screen-reader-navigable landmark structure.
 *
 * Requirements: 7.1, 7.8–7.11, 16.1, 17.1, 17.2 / design §12, §20.1, §20.2
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within, cleanup } from "@solidjs/testing-library";
import { NavigationRail as Dock } from "./NavigationRail";
import { SpaceRouter } from "./SpaceRouter";
import {
  navigate,
  currentRoute,
  parseRoute,
  routeToPath,
  isValidSpace,
  ALL_SPACES,
  type Space,
} from "./router";
import { SPACE_META } from "./spaces";
import { collectItems } from "../palette/sources";
import { converseStore, coreStore, shellStore } from "../stores";
import { initSummon, disposeSummon } from "../summon/summon";

// ─── Command references → canonical Space IDs (Req 7.1, 7.8, 7.11) ───────────
//
// The palette's Go-mode "spaces" source is the command reference that navigates
// to Spaces. Every reference MUST resolve to a canonical Space ID (no stale or
// invalid Space refs). navigate()'s first parameter is typed `Space`, so any
// non-canonical literal in a source is additionally rejected at `npm run check`
// time — this test proves the runtime references + resolution.

describe("Command references resolve to canonical Space IDs (task 7.8)", () => {
  beforeEach(() => {
    navigate("converse");
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  function spaceItems() {
    return collectItems("go").filter((item) => item.type === "space");
  }

  it("exposes exactly one Space command per canonical Space, in canonical order", () => {
    const items = spaceItems();
    expect(items.map((i) => i.id)).toEqual(ALL_SPACES.map((s) => `space:${s}`));
    // Titles/keywords come from the canonical registry — no parallel copy.
    items.forEach((item, index) => {
      const space = ALL_SPACES[index];
      expect(item.title).toBe(SPACE_META[space].label);
      expect(item.keywords).toBe(space);
    });
  });

  it("references only valid canonical Space IDs (no stale/invalid Space refs)", () => {
    for (const item of spaceItems()) {
      const space = item.id.replace(/^space:/, "");
      expect(isValidSpace(space)).toBe(true);
      expect(ALL_SPACES).toContain(space as Space);
    }
  });

  it("each Space command resolves via navigate() to its canonical Space", () => {
    for (const item of spaceItems()) {
      const space = item.id.replace(/^space:/, "") as Space;
      navigate("converse"); // reset between references
      item.run();
      // The reference resolved to a real canonical Space through the router.
      expect(isValidSpace(currentRoute().space)).toBe(true);
      expect(currentRoute().space).toBe(space);
    }
  });
});

// ─── Selected secondary route (Req 7.8, 7.9; design §12 route grammar) ───────
//
// A secondary route is `space/segment[/entityId]` WITHIN a Space. The router
// must reflect the full route, the correct Space must render, and the Dock must
// still mark the PARENT Space active via aria-current (a secondary route never
// changes which top-level Space is "current").

describe("Selected secondary route — reflection + parent-Space active (task 7.8)", () => {
  beforeEach(() => {
    navigate("converse");
    shellStore.setActiveSpace("converse");
    shellStore.setWindowMode("standard");
    converseStore.setActiveThread(null);
    coreStore.reset();
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("reflects a space/segment/entityId secondary route on currentRoute()", () => {
    navigate("capabilities", "installed", "skill-xyz");
    expect(currentRoute()).toEqual({
      space: "capabilities",
      segment: "installed",
      entityId: "skill-xyz",
    });
    // The secondary route round-trips through the canonical grammar.
    expect(routeToPath(currentRoute())).toBe("capabilities/installed/skill-xyz");
    expect(parseRoute("capabilities/installed/skill-xyz")).toEqual(currentRoute());
  });

  it("Dock marks the PARENT Space active (aria-current) while on a secondary route", async () => {
    navigate("machines", "device", "server-1");
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });
    const buttons = within(nav).getAllByRole("button");

    const current = buttons.filter((b) => b.getAttribute("aria-current") === "page");
    // Exactly the parent Space (Machines) is current — the segment/entity does
    // not add or move a top-level active marker.
    expect(current).toHaveLength(1);
    expect(current[0].getAttribute("aria-label")).toBe(SPACE_META.machines.label);
  });

  it("renders the correct Space for a secondary route (parent Space content)", () => {
    // Converse is eager, so its region renders synchronously for the assertion.
    navigate("converse", "thread", "thread-1");
    render(() => <SpaceRouter />);
    // The router selects the PARENT Space's component regardless of segment.
    expect(screen.getByRole("region", { name: "Converse" })).toBeInTheDocument();
  });
});

// ─── Shortcut ↔ Space-switching non-conflict (Req 7.8; design §20.2) ─────────
//
// The palette summon chords (Ctrl/Cmd+K, Ctrl+Shift+P) must coexist with Dock
// navigation: summoning the palette must NOT switch the active Space, and the
// Dock registers no keyboard shortcut that could collide with the chords.

describe("Summon shortcut does not conflict with Space switching (task 7.8)", () => {
  let dispose: (() => void) | undefined;
  beforeEach(() => {
    navigate("memory");
    shellStore.setPaletteOpen(false);
    dispose = initSummon();
  });
  afterEach(() => {
    dispose?.();
    disposeSummon();
    cleanup();
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  it("Ctrl+K opens the palette without changing the active Space", async () => {
    render(() => <Dock />);
    await screen.findByRole("navigation", { name: "Spaces" });

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "k", ctrlKey: true, bubbles: true }));

    expect(shellStore.paletteOpen()).toBe(true);
    // The active Space is untouched by the summon chord (no shortcut collision).
    expect(currentRoute().space).toBe("memory");
  });

  it("Ctrl+Shift+P (Do mode) opens the palette without changing the active Space", async () => {
    render(() => <Dock />);
    await screen.findByRole("navigation", { name: "Spaces" });

    document.dispatchEvent(
      new KeyboardEvent("keydown", { key: "p", ctrlKey: true, shiftKey: true, bubbles: true }),
    );

    expect(shellStore.paletteOpen()).toBe(true);
    expect(shellStore.paletteMode()).toBe("do");
    expect(currentRoute().space).toBe("memory");
  });
});

// ─── Screen-reader navigation structure (Req 16.1, 17.1, 17.2) ───────────────
//
// The Dock must present a screen-reader-navigable landmark: a single named
// "navigation" landmark, a list of seven buttons each with an accessible NAME,
// the active Space announced via aria-current="page", and decorative grouping
// separators removed from the accessibility tree.

describe("Dock screen-reader navigation structure (task 7.8)", () => {
  beforeEach(() => {
    navigate("observatory");
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("exposes a single named navigation landmark", async () => {
    render(() => <Dock />);
    const navs = await screen.findAllByRole("navigation", { name: "Spaces" });
    expect(navs).toHaveLength(1);
    expect(navs[0].tagName.toLowerCase()).toBe("nav");
    expect(navs[0].getAttribute("aria-label")).toBe("Spaces");
  });

  it("gives every one of the seven Space buttons an accessible name in reading order", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });
    const buttons = within(nav).getAllByRole("button");

    expect(buttons).toHaveLength(7);
    // Accessible name (aria-label) present and in canonical reading order.
    expect(buttons.map((b) => b.getAttribute("aria-label"))).toEqual(
      ALL_SPACES.map((s) => SPACE_META[s].label),
    );
    for (const button of buttons) {
      expect(button.getAttribute("aria-label")?.trim()).toBeTruthy();
    }
  });

  it("announces the active Space with aria-current=page (exactly one)", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });
    const current = within(nav)
      .getAllByRole("button")
      .filter((b) => b.getAttribute("aria-current") === "page");

    expect(current).toHaveLength(1);
    expect(current[0].getAttribute("aria-label")).toBe(SPACE_META.observatory.label);
  });

  it("removes decorative group separators from the accessibility tree", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });

    // Separators are presentational and aria-hidden — never announced or a stop.
    const separators = nav.querySelectorAll(".kria-navrail__separator");
    expect(separators.length).toBeGreaterThan(0);
    for (const sep of Array.from(separators)) {
      expect(sep.getAttribute("aria-hidden")).toBe("true");
      expect(sep.getAttribute("role")).toBe("presentation");
    }
    // No stray interactive/labelled elements pollute the reading order.
    expect(within(nav).getAllByRole("button")).toHaveLength(7);
  });
});
