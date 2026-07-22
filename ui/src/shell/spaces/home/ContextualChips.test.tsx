import { describe, it, expect, afterEach, vi } from "vitest";
import { render, cleanup } from "@solidjs/testing-library";

import ContextualChips from "./ContextualChips";
import { MAX_CHIPS, type Chip } from "../../../stores/homeFocusStore";
import { checkRestingCalm } from "./guardrails";
import type { Route } from "../../router";

afterEach(cleanup);

/** Build a `route` chip with sensible defaults. */
function routeChip(over: Partial<Chip> = {}): Chip {
  return {
    id: over.id ?? "c-route",
    label: over.label ?? "Timeline",
    icon: over.icon ?? "brain",
    kind: "route",
    payload: over.payload ?? ({ space: "memory" } as Route),
  };
}

/** Build a `stage` chip with sensible defaults. */
function stageChip(over: Partial<Chip> = {}): Chip {
  return {
    id: over.id ?? "c-stage",
    label: over.label ?? "Resume draft",
    icon: over.icon ?? "pencil",
    kind: "stage",
    payload: over.payload ?? "Draft: weekly report for the team.",
  };
}

describe("ContextualChips — next actions (Req 5.1–5.4)", () => {
  it("renders at most three chips, ranked from live state (Req 5.1)", () => {
    const chips: Chip[] = [
      routeChip({ id: "a", label: "One" }),
      stageChip({ id: "b", label: "Two" }),
      routeChip({ id: "c", label: "Three" }),
    ];
    const { container } = render(() => <ContextualChips chips={() => chips} />);
    const rendered = container.querySelectorAll('[data-role="chip"]');
    expect(rendered.length).toBe(3);
    // Rendered in the engine-supplied (ranked) order.
    expect([...rendered].map((c) => c.textContent)).toEqual(["One", "Two", "Three"]);
  });

  it("enforces the ≤3 cap defensively even if upstream over-supplies (Req 5.1)", () => {
    const chips: Chip[] = Array.from({ length: 5 }, (_, i) =>
      routeChip({ id: `c${i}`, label: `Chip ${i}` }),
    );
    const { container } = render(() => <ContextualChips chips={() => chips} />);
    expect(container.querySelectorAll('[data-role="chip"]').length).toBe(MAX_CHIPS);
  });

  it("renders each chip as a focus-visible button with BOTH icon and text (Req 5.4)", () => {
    const { container, getByText } = render(() => (
      <ContextualChips chips={() => [routeChip({ label: "Timeline", icon: "brain" })]} />
    ));
    const btn = container.querySelector<HTMLButtonElement>('[data-role="chip"]')!;
    // A real button → keyboard-operable (Enter/Space) with a native focus ring.
    expect(btn.tagName).toBe("BUTTON");
    expect(btn.getAttribute("type")).toBe("button");
    // Text label present (meaning never by icon/color alone).
    expect(getByText("Timeline")).toBeInTheDocument();
    // Icon present, referencing the sprite; decorative (aria-hidden).
    const use = btn.querySelector("use");
    expect(use?.getAttribute("href")).toBe("/icons/lucide-sprite.svg#brain");
    expect(btn.querySelector("svg")?.getAttribute("aria-hidden")).toBe("true");
  });

  it("stages a reviewable draft (never sends) when a `stage` chip is activated (Req 5.3)", () => {
    const onStage = vi.fn();
    const onNavigate = vi.fn();
    const { container } = render(() => (
      <ContextualChips
        chips={() => [stageChip({ payload: "Draft: send the weekly report." })]}
        onStage={onStage}
        onNavigate={onNavigate}
      />
    ));
    const btn = container.querySelector<HTMLButtonElement>('[data-chip-kind="stage"]')!;
    btn.click();
    // Stages the draft text only — no routing, no send/execute side effect.
    expect(onStage).toHaveBeenCalledTimes(1);
    expect(onStage).toHaveBeenCalledWith("Draft: send the weekly report.");
    expect(onNavigate).not.toHaveBeenCalled();
  });

  it("routes to the owning surface only (never sends) when a `route` chip is activated (Req 5.3)", () => {
    const onStage = vi.fn();
    const onNavigate = vi.fn();
    const route: Route = { space: "converse", segment: "approvals", entityId: "ap-1" };
    const { container } = render(() => (
      <ContextualChips
        chips={() => [routeChip({ payload: route })]}
        onStage={onStage}
        onNavigate={onNavigate}
      />
    ));
    const btn = container.querySelector<HTMLButtonElement>('[data-chip-kind="route"]')!;
    btn.click();
    // Routes only — exactly the supplied route, no staging, no send/execute.
    expect(onNavigate).toHaveBeenCalledTimes(1);
    expect(onNavigate).toHaveBeenCalledWith(route);
    expect(onStage).not.toHaveBeenCalled();
  });

  it("omits entirely (renders NOTHING, no filler) when there is no real action (Req 5.2)", () => {
    const { container } = render(() => <ContextualChips chips={() => []} />);
    expect(container.querySelector('[data-region="contextual-chips"]')).not.toBeInTheDocument();
    expect(container.querySelector(".kria-chips")).not.toBeInTheDocument();
    expect(container.querySelector('[data-role="chip"]')).not.toBeInTheDocument();
    // The resting-calm guardrail must find no filler / empty standing surface.
    expect(checkRestingCalm(container)).toEqual([]);
  });

  it("renders NOTHING when reading the frame throws (failure isolation, design §14)", () => {
    const { container } = render(() => (
      <ContextualChips
        chips={() => {
          throw new Error("frame error");
        }}
      />
    ));
    expect(container.querySelector('[data-region="contextual-chips"]')).not.toBeInTheDocument();
    expect(checkRestingCalm(container)).toEqual([]);
  });

  it("exposes the row as an accessible list without stealing focus (Req 5.4)", () => {
    const { container } = render(() => (
      <ContextualChips chips={() => [routeChip(), stageChip()]} />
    ));
    const region = container.querySelector('[data-region="contextual-chips"]')!;
    expect(region.getAttribute("role")).toBe("list");
    expect(region.getAttribute("aria-label")).toBe("Suggested actions");
    expect(region.hasAttribute("tabindex")).toBe(false);
    expect(container.querySelectorAll('[role="listitem"]').length).toBe(2);
  });

  it("marks the row static under reduced motion (Req 17.4/21.4)", () => {
    const { container } = render(() => (
      <ContextualChips chips={() => [routeChip()]} reducedMotion />
    ));
    expect(
      container.querySelector('[data-region="contextual-chips"]')?.getAttribute("data-motion"),
    ).toBe("static");
  });
});
