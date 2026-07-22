import { describe, it, expect, afterEach, vi } from "vitest";
import { render, cleanup } from "@solidjs/testing-library";
import { createSignal } from "solid-js";

import ContextualOrbit, { ORBIT_FADE_MS } from "./ContextualOrbit";
import type { OrbitPoint } from "../../../stores/homeFocusStore";
import { checkRestingCalm, checkSingleCapabilityAwareness } from "./guardrails";
import type { Route } from "../../router";

afterEach(cleanup);

/** Build an actionable (routing) Orbit point with sensible defaults. */
function routePoint(over: Partial<OrbitPoint> = {}): OrbitPoint {
  return {
    id: over.id ?? "orbit:memory",
    capability: over.capability ?? "memory",
    lit: over.lit ?? true,
    label: over.label ?? "Just learned",
    route: over.route ?? ({ space: "memory" } as Route),
  };
}

/** Build a non-actionable (no route) awareness light. */
function lightPoint(over: Partial<OrbitPoint> = {}): OrbitPoint {
  return {
    id: over.id ?? "orbit:local",
    capability: over.capability ?? "local",
    lit: over.lit ?? true,
    label: over.label ?? "Working locally",
  };
}

describe("ContextualOrbit — capability awareness (Req 6.1–6.6)", () => {
  it("renders ONLY lit points and hides unlit ones (Req 6.2)", () => {
    const points: OrbitPoint[] = [
      routePoint({ id: "a", label: "Lit A" }),
      { ...lightPoint({ id: "b", label: "Unlit B" }), lit: false },
      lightPoint({ id: "c", label: "Lit C" }),
    ];
    const { container } = render(() => (
      <ContextualOrbit orbit={() => points} engaged={() => true} />
    ));
    const rendered = container.querySelectorAll('[data-role="orbit-point"]');
    expect(rendered.length).toBe(2);
    const labels = [...rendered].map((p) => p.textContent);
    expect(labels).toContain("Lit A");
    expect(labels).toContain("Lit C");
    expect(labels).not.toContain("Unlit B");
  });

  it("appears on engagement and hides on disengage (Req 6.1)", () => {
    const [engaged, setEngaged] = createSignal(false);
    const { container } = render(() => (
      // reducedMotion → hide is instant (no fade timer), deterministic here.
      <ContextualOrbit orbit={() => [routePoint()]} engaged={engaged} reducedMotion />
    ));
    // Absent at rest (not engaged) — no Orbit DOM at all.
    expect(container.querySelector('[data-region="contextual-orbit"]')).not.toBeInTheDocument();

    setEngaged(true);
    expect(container.querySelector('[data-region="contextual-orbit"]')).toBeInTheDocument();

    setEngaged(false);
    expect(container.querySelector('[data-region="contextual-orbit"]')).not.toBeInTheDocument();
  });

  it("fades out (temporary) before unmounting on disengage under motion (Req 6.1)", () => {
    vi.useFakeTimers();
    try {
      const [engaged, setEngaged] = createSignal(true);
      const { container } = render(() => (
        <ContextualOrbit orbit={() => [routePoint()]} engaged={engaged} reducedMotion={false} />
      ));
      const region = () => container.querySelector('[data-region="contextual-orbit"]');
      expect(region()?.getAttribute("data-engaged")).toBe("true");

      setEngaged(false);
      // Still mounted but marked leaving (fading) — temporary, not permanent.
      expect(region()).toBeInTheDocument();
      expect(region()?.getAttribute("data-engaged")).toBe("false");

      vi.advanceTimersByTime(ORBIT_FADE_MS + 10);
      expect(region()).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("exposes an actionable point as a focusable button that ROUTES ONLY (Req 6.4)", () => {
    const onNavigate = vi.fn();
    const route: Route = { space: "automations", segment: "runs", entityId: "run-1" };
    const { container, getByText } = render(() => (
      <ContextualOrbit
        orbit={() => [routePoint({ label: "Automation running", route })]}
        engaged={() => true}
        onNavigate={onNavigate}
      />
    ));
    const btn = container.querySelector<HTMLButtonElement>('[data-actionable="true"]')!;
    // A real button → keyboard-operable (Enter/Space) with a native focus ring.
    expect(btn.tagName).toBe("BUTTON");
    expect(btn.getAttribute("type")).toBe("button");
    // Labelled: both an accessible name and visible text (never color/icon alone).
    expect(btn.getAttribute("aria-label")).toBe("Automation running");
    expect(getByText("Automation running")).toBeInTheDocument();
    btn.click();
    // Routes only — exactly the supplied route, no send/tool/approval side effect.
    expect(onNavigate).toHaveBeenCalledTimes(1);
    expect(onNavigate).toHaveBeenCalledWith(route);
  });

  it("renders a non-actionable point as a labelled, non-interactive light (Req 6.4)", () => {
    const { container } = render(() => (
      <ContextualOrbit orbit={() => [lightPoint({ label: "Working locally" })]} engaged={() => true} />
    ));
    const point = container.querySelector('[data-actionable="false"]')!;
    expect(point.tagName).not.toBe("BUTTON");
    expect(point.getAttribute("role")).toBe("img");
    expect(point.getAttribute("aria-label")).toBe("Working locally");
  });

  it("degrades to static labelled dots under reduced motion, same labels + routing (Req 6.6)", () => {
    const onNavigate = vi.fn();
    const { container, getByText } = render(() => (
      <ContextualOrbit
        orbit={() => [routePoint({ label: "Just learned" })]}
        engaged={() => true}
        reducedMotion
        onNavigate={onNavigate}
      />
    ));
    const region = container.querySelector('[data-region="contextual-orbit"]')!;
    expect(region.getAttribute("data-motion")).toBe("static");
    // Same label preserved…
    expect(getByText("Just learned")).toBeInTheDocument();
    // …and routing still works in the static path.
    container.querySelector<HTMLButtonElement>('[data-actionable="true"]')!.click();
    expect(onNavigate).toHaveBeenCalledTimes(1);
  });

  it("is body language, not a menu / navigation region (Req 6.3)", () => {
    const { container } = render(() => (
      <ContextualOrbit orbit={() => [routePoint()]} engaged={() => true} />
    ));
    const region = container.querySelector('[data-region="contextual-orbit"]')!;
    expect(region.getAttribute("role")).toBe("group");
    expect(region.getAttribute("aria-label")).toBe("What KRIA can help with right now");
    // Never a menu/navigation register.
    expect(container.querySelector('[role="menu"]')).not.toBeInTheDocument();
    expect(container.querySelector('[role="menubar"]')).not.toBeInTheDocument();
    expect(container.querySelector('[role="navigation"]')).not.toBeInTheDocument();
  });

  it("renders NOTHING when there are no lit points (Req 6.1, resting calm)", () => {
    const { container } = render(() => <ContextualOrbit orbit={() => []} engaged={() => true} />);
    expect(container.querySelector('[data-region="contextual-orbit"]')).not.toBeInTheDocument();
    expect(checkRestingCalm(container)).toEqual([]);
  });

  it("renders NOTHING when reading the frame throws (failure isolation, design §14)", () => {
    const { container } = render(() => (
      <ContextualOrbit
        orbit={() => {
          throw new Error("frame error");
        }}
        engaged={() => true}
      />
    ));
    expect(container.querySelector('[data-region="contextual-orbit"]')).not.toBeInTheDocument();
  });

  it("is the SINGLE capability-awareness system, no duplicate sparks UI (Req 6.5)", () => {
    const { container } = render(() => (
      <ContextualOrbit orbit={() => [routePoint(), lightPoint()]} engaged={() => true} />
    ));
    // Marks itself as the one capability-awareness system…
    const systems = container.querySelectorAll("[data-capability-awareness]");
    expect(systems.length).toBe(1);
    expect(systems[0].getAttribute("data-capability-awareness")).toBe("orbit");
    // …with no legacy sparks UI and no second system → guardrail clean.
    expect(checkSingleCapabilityAwareness(container)).toEqual([]);
  });
});
