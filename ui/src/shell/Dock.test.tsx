/**
 * Guard tests for the Dock — the 7-Space navigation rail (task 7.1; UIE-H-003).
 *
 * These tests PIN the canonical Space identities, labels, order, and interaction
 * contract from design.md §12 (canonical navigation table) and §20.1 (route/state
 * authority). They lock:
 *   - exactly seven one-click buttons in canonical DOM order with canonical labels,
 *   - DOM order == focus/tab order (no tabindex reordering),
 *   - single-action (one click) Space switching via navigate(space),
 *   - aria-current="page" on the active Space only.
 *
 * Requirements: 1.2, 1.3, 7.1, 7.8, 17.1, 17.2
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within, cleanup } from "@solidjs/testing-library";
import { Dock, SPACE_GROUP, spaceOutcome } from "./Dock";
import { navigate, currentRoute, ALL_SPACES, type Space } from "./router";
import { SPACE_META } from "./spaces";
import { getTerm } from "./terminology";

// Canonical order + labels from design.md §12. Locked here on purpose.
const CANONICAL: ReadonlyArray<{ id: string; label: string }> = [
  { id: "converse", label: "Converse" },
  { id: "memory", label: "Memory" },
  { id: "automations", label: "Automations" },
  { id: "capabilities", label: "Capabilities" },
  { id: "machines", label: "Machines" },
  { id: "observatory", label: "Observatory" },
  { id: "settings", label: "Settings" },
];

describe("Dock — canonical Space rail (task 7.1)", () => {
  beforeEach(() => {
    navigate("converse");
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("renders exactly seven one-click buttons", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });
    expect(within(nav).getAllByRole("button")).toHaveLength(7);
  });

  it("renders the seven buttons in canonical DOM order with canonical labels", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });
    const buttons = within(nav).getAllByRole("button");

    const renderedLabels = buttons.map((b) => b.getAttribute("aria-label"));
    expect(renderedLabels).toEqual(CANONICAL.map((c) => c.label));
  });

  it("DOM order matches ALL_SPACES and SPACE_META labels (no source/label drift)", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });
    const buttons = within(nav).getAllByRole("button");

    ALL_SPACES.forEach((space, index) => {
      expect(buttons[index].getAttribute("aria-label")).toBe(SPACE_META[space].label);
      expect(CANONICAL[index].id).toBe(space);
    });
  });

  it("DOM order == focus/tab order (no positive/reordering tabindex)", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });
    const buttons = within(nav).getAllByRole("button");

    // A native <button> is in the natural tab order. No button may carry a
    // tabindex that removes it from, or reorders, the natural DOM sequence.
    for (const button of buttons) {
      const tabindex = button.getAttribute("tabindex");
      expect(tabindex === null || tabindex === "0").toBe(true);
    }

    // Sanity: buttons appear in document order equal to canonical order.
    const domOrder = buttons.map((b) => b.getAttribute("aria-label"));
    expect(domOrder).toEqual(CANONICAL.map((c) => c.label));
  });

  it("switches Space in a single click via navigate(space)", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });

    fireEvent.click(within(nav).getByRole("button", { name: "Memory" }));
    expect(currentRoute().space).toBe("memory");

    fireEvent.click(within(nav).getByRole("button", { name: "Observatory" }));
    expect(currentRoute().space).toBe("observatory");
  });

  it("invokes onSelect exactly once per single click (one-action switch)", async () => {
    const onSelect = vi.fn();
    render(() => <Dock onSelect={onSelect} />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });

    fireEvent.click(within(nav).getByRole("button", { name: "Automations" }));
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledWith("automations");
  });

  it("marks only the active Space with aria-current=page", async () => {
    navigate("machines");
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });
    const buttons = within(nav).getAllByRole("button");

    const current = buttons.filter((b) => b.getAttribute("aria-current") === "page");
    expect(current).toHaveLength(1);
    expect(current[0].getAttribute("aria-label")).toBe("Machines");
  });

  it("moves aria-current to the newly selected Space after a switch", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });

    fireEvent.click(within(nav).getByRole("button", { name: "Settings" }));
    const buttons = within(nav).getAllByRole("button");
    const current = buttons.filter((b) => b.getAttribute("aria-current") === "page");
    expect(current).toHaveLength(1);
    expect(current[0].getAttribute("aria-label")).toBe("Settings");
  });
});

/**
 * Grouping/emphasis guard tests (task 7.6; UIE-H-003).
 *
 * These PIN that beginner-oriented visual grouping/emphasis (design §12,
 * Req 7.2) is PRESENTATION ONLY: it may emphasize Converse and separate the
 * supporting/system/utility groups, but MUST NOT reorder the DOM/focus order,
 * add a focus stop, add/remove a route, or break one-click switching +
 * aria-current.
 *
 * Requirements: 7.2, 7.7, 1.3, 17.1, 17.2
 */
describe("Dock — presentation-only grouping/emphasis (task 7.6)", () => {
  beforeEach(() => {
    navigate("converse");
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("classifies the canonical Spaces into design §12 hierarchy roles", () => {
    // Presentation metadata only — Converse primary; memory/automations/
    // capabilities supporting; machines/observatory system; settings utility.
    expect(ALL_SPACES.map((s) => SPACE_GROUP[s])).toEqual([
      "primary",
      "supporting",
      "supporting",
      "supporting",
      "system",
      "system",
      "utility",
    ]);
  });

  it("keeps DOM order the canonical seven despite grouping (no reorder)", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });
    const buttons = within(nav).getAllByRole("button");

    expect(buttons).toHaveLength(7);
    expect(buttons.map((b) => b.getAttribute("aria-label"))).toEqual(
      CANONICAL.map((c) => c.label),
    );
  });

  it("adds no extra focus stops — only the seven buttons are focusable", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });

    // Decorative separators must not be tabbable/interactive. The only
    // focusable/interactive elements remain the seven Space buttons.
    const focusable = within(nav).queryAllByRole("button");
    expect(focusable).toHaveLength(7);
    expect(nav.querySelectorAll("a, input, select, textarea, [tabindex]")).toHaveLength(0);
  });

  it("renders decorative group separators that are aria-hidden and non-focusable", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });

    // Three group boundaries: primary→supporting, supporting→system,
    // system→utility. Each separator is aria-hidden and carries no button role.
    const separators = nav.querySelectorAll(".kria-dock__separator");
    expect(separators).toHaveLength(3);
    for (const sep of Array.from(separators)) {
      expect(sep.getAttribute("aria-hidden")).toBe("true");
      expect(sep.getAttribute("role")).toBe("presentation");
      // Not a tab stop and holds no interactive content.
      expect(sep.getAttribute("tabindex")).toBeNull();
      expect(sep.querySelector("button, a, input, [tabindex]")).toBeNull();
    }
  });

  it("emphasizes only the primary destination (Converse)", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });
    const buttons = within(nav).getAllByRole("button");

    const primary = buttons.filter((b) =>
      b.classList.contains("kria-dock__button--primary"),
    );
    expect(primary).toHaveLength(1);
    expect(primary[0].getAttribute("aria-label")).toBe("Converse");
  });

  it("marks each item with its group without changing button semantics", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });

    const items = Array.from(nav.querySelectorAll(".kria-dock__item"));
    expect(items).toHaveLength(7);
    items.forEach((item, index) => {
      expect(item.getAttribute("data-dock-group")).toBe(SPACE_GROUP[ALL_SPACES[index]]);
      // Every grouped item still contains exactly one one-click Space button.
      expect(item.querySelectorAll("button")).toHaveLength(1);
    });
  });

  it("preserves one-click switching + aria-current across group boundaries", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });

    // Switch into a system-group Space in a single click.
    fireEvent.click(within(nav).getByRole("button", { name: "Observatory" }));
    expect(currentRoute().space).toBe("observatory");

    const buttons = within(nav).getAllByRole("button");
    const current = buttons.filter((b) => b.getAttribute("aria-current") === "page");
    expect(current).toHaveLength(1);
    expect(current[0].getAttribute("aria-label")).toBe("Observatory");
  });
});

/**
 * Concise outcome descriptions + Mini accessible-name/reachability guard
 * tests (task 7.7; IU-08; Req 7.3–7.7, 16, 17.1, 17.2).
 *
 * These PIN that the Dock surfaces concise outcome distinctions READ FROM the
 * terminology matrix (single source of truth, task 7.5) — never re-authored —
 * and that Mini icon-only presentation keeps the full accessible NAME
 * (aria-label), keeps the outcome DESCRIPTION, keeps focus order stable, and
 * keeps every route reachable in exactly one click.
 *
 * Requirements: 7.3, 7.4, 7.7, 16, 17.1, 17.2
 */
// Spaces that ARE top-level Space_Routes in the matrix carry a concise outcome.
const MATRIX_SPACES: ReadonlyArray<{ space: Space; termId: Parameters<typeof getTerm>[0] }> = [
  { space: "memory", termId: "memory" },
  { space: "machines", termId: "machines" },
  { space: "observatory", termId: "observatory" },
];
// Spaces with NO space-route matrix entry get no fabricated outcome copy.
const NON_MATRIX_SPACES: ReadonlyArray<Space> = [
  "converse",
  "automations",
  "capabilities",
  "settings",
];

describe("Dock — concise outcome descriptions from the matrix (task 7.7)", () => {
  beforeEach(() => {
    navigate("converse");
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("surfaces the matrix outcome as an accessible description for Machines/Observatory/Memory", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });

    for (const { space, termId } of MATRIX_SPACES) {
      const outcome = getTerm(termId).outcome;
      // spaceOutcome reads from the matrix — not parallel copy.
      expect(spaceOutcome(space)).toBe(outcome);

      const button = within(nav).getByRole("button", { name: SPACE_META[space].label });
      const descId = button.getAttribute("aria-describedby");
      expect(descId).toBeTruthy();
      const desc = nav.querySelector(`#${descId}`);
      expect(desc?.textContent).toBe(outcome);
      // Tooltip conveys the same distinction (name + outcome) on hover/focus.
      expect(button.getAttribute("title")).toBe(`${SPACE_META[space].label}: ${outcome}`);
    }
  });

  it("does NOT fabricate outcome copy for Spaces without a matrix space-route entry", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });

    for (const space of NON_MATRIX_SPACES) {
      expect(spaceOutcome(space)).toBeUndefined();
      const button = within(nav).getByRole("button", { name: SPACE_META[space].label });
      expect(button.getAttribute("aria-describedby")).toBeNull();
      // Title falls back to the bare label — no invented copy.
      expect(button.getAttribute("title")).toBe(SPACE_META[space].label);
    }
  });

  it("keeps the accessible NAME the full Space label (outcome is a description, not the name)", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });

    for (const { space, termId } of MATRIX_SPACES) {
      const button = within(nav).getByRole("button", { name: SPACE_META[space].label });
      // Name stays the label; the outcome never replaces the accessible name.
      expect(button.getAttribute("aria-label")).toBe(SPACE_META[space].label);
      expect(button.getAttribute("aria-label")).not.toBe(getTerm(termId).outcome);
    }
  });
});

describe("Dock — Mini icon-only accessible name + reachability (task 7.7)", () => {
  beforeEach(() => {
    navigate("converse");
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  // Render the Dock inside a Mini shell wrapper. In Mini the visible
  // `.kria-dock__label` is CSS-hidden (AppShell.css), but the accessible name
  // (aria-label) and outcome description are DOM attributes, so they persist
  // independently of the visual label — which is exactly what this proves.
  function renderMini() {
    return render(() => (
      <div class="kria-shell" data-window-mode="mini">
        <Dock />
      </div>
    ));
  }

  it("retains the full accessible name for all seven buttons when labels are hidden", async () => {
    renderMini();
    const nav = await screen.findByRole("navigation", { name: "Spaces" });
    const buttons = within(nav).getAllByRole("button");

    expect(buttons).toHaveLength(7);
    // Every button is still findable BY ITS FULL NAME even icon-only.
    ALL_SPACES.forEach((space) => {
      expect(within(nav).getByRole("button", { name: SPACE_META[space].label })).toBeInTheDocument();
    });
  });

  it("retains the outcome description for matrix Spaces in Mini", async () => {
    renderMini();
    const nav = await screen.findByRole("navigation", { name: "Spaces" });

    for (const { space, termId } of MATRIX_SPACES) {
      const button = within(nav).getByRole("button", { name: SPACE_META[space].label });
      const descId = button.getAttribute("aria-describedby");
      expect(descId).toBeTruthy();
      expect(nav.querySelector(`#${descId}`)?.textContent).toBe(getTerm(termId).outcome);
    }
  });

  it("keeps focus order stable (DOM order == canonical) with no extra focus stops in Mini", async () => {
    renderMini();
    const nav = await screen.findByRole("navigation", { name: "Spaces" });
    const buttons = within(nav).getAllByRole("button");

    expect(buttons.map((b) => b.getAttribute("aria-label"))).toEqual(
      CANONICAL.map((c) => c.label),
    );
    // Only the seven Space buttons are focusable; nothing else was introduced.
    expect(nav.querySelectorAll("a, input, select, textarea, [tabindex]")).toHaveLength(0);
  });

  it("keeps every route reachable in exactly one click in Mini", async () => {
    renderMini();
    const nav = await screen.findByRole("navigation", { name: "Spaces" });

    for (const space of ALL_SPACES) {
      fireEvent.click(within(nav).getByRole("button", { name: SPACE_META[space].label }));
      expect(currentRoute().space).toBe(space);
    }
  });
});
