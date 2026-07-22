/**
 * PresenceOnboarding component — presentation tests (design.md §17, Req 19).
 *
 * Verifies the thin presentation contract with injected deps (no localStorage,
 * no live stores):
 *   • shows the single next qualifying hint (Core whisper first);
 *   • the dismissal retires the hint and advances to the next one;
 *   • renders nothing once every hint is retired (never repeats);
 *   • exposes the hint as a polite once-announce live region (Req 21.3);
 *   • Orbit reveal appears only when the Orbit is engaged (Req 19.1).
 */
import { describe, it, expect, afterEach } from "vitest";
import { render, cleanup, fireEvent } from "@solidjs/testing-library";
import { createSignal } from "solid-js";

import PresenceOnboarding from "./PresenceOnboarding";
import { ONBOARDING_COACH_IDS } from "./presenceOnboarding";

afterEach(cleanup);

/**
 * A controllable retired-set harness so tests drive the one-time ledger without
 * touching localStorage. `onRetire` records the coach id reactively.
 */
function harness(orbitEngaged = false) {
  const [retired, setRetired] = createSignal<ReadonlySet<string>>(new Set());
  const [engaged] = createSignal(orbitEngaged);
  return {
    isRetired: (coachId: string) => retired().has(coachId),
    onRetire: (coachId: string) =>
      setRetired((prev) => new Set(prev).add(coachId)),
    orbitEngaged: engaged,
  };
}

describe("PresenceOnboarding — first-run whisper (Req 19)", () => {
  it("shows the Core whisper first on first run", () => {
    const h = harness();
    const { container, getByText } = render(() => (
      <PresenceOnboarding isRetired={h.isRetired} onRetire={h.onRetire} orbitEngaged={h.orbitEngaged} />
    ));
    const el = container.querySelector('[data-onboarding-hint="core-whisper"]');
    expect(el).toBeInTheDocument();
    expect(getByText("This is KRIA. Talk, type, or click me.")).toBeInTheDocument();
  });

  it("is a polite once-announce live region that never steals focus (Req 21.3)", () => {
    const h = harness();
    const { container } = render(() => (
      <PresenceOnboarding isRetired={h.isRetired} onRetire={h.onRetire} orbitEngaged={h.orbitEngaged} />
    ));
    const region = container.querySelector(".kria-onboarding")!;
    expect(region.getAttribute("role")).toBe("status");
    expect(region.getAttribute("aria-live")).toBe("polite");
  });

  it("dismissing retires the hint and advances to the next one", () => {
    const h = harness();
    const { container, getByRole, queryByText } = render(() => (
      <PresenceOnboarding isRetired={h.isRetired} onRetire={h.onRetire} orbitEngaged={h.orbitEngaged} />
    ));
    // Dismiss the Core whisper → it retires and the Dock hint takes its place
    // (orbit-reveal is gated out because the Orbit is not engaged here).
    fireEvent.click(getByRole("button", { name: "Got it" }));
    expect(queryByText("This is KRIA. Talk, type, or click me.")).toBeNull();
    expect(container.querySelector('[data-onboarding-hint="dock-hint"]')).toBeInTheDocument();
  });

  it("shows the Orbit reveal only once the Orbit is engaged", () => {
    // Retire the Core whisper so the next candidate is the orbit reveal.
    const [retired, setRetired] = createSignal<ReadonlySet<string>>(
      new Set([ONBOARDING_COACH_IDS["core-whisper"]]),
    );
    const [engaged, setEngaged] = createSignal(false);
    const { container } = render(() => (
      <PresenceOnboarding
        isRetired={(id) => retired().has(id)}
        onRetire={(id) => setRetired((p) => new Set(p).add(id))}
        orbitEngaged={engaged}
      />
    ));
    // At rest the orbit reveal is hidden; the dock hint is shown instead.
    expect(container.querySelector('[data-onboarding-hint="orbit-reveal"]')).toBeNull();
    expect(container.querySelector('[data-onboarding-hint="dock-hint"]')).toBeInTheDocument();
    // Engaging the Orbit reveals the (earlier, canonical) orbit hint.
    setEngaged(true);
    expect(container.querySelector('[data-onboarding-hint="orbit-reveal"]')).toBeInTheDocument();
  });

  it("renders nothing once every hint is retired (never repeats)", () => {
    const allRetired = new Set(Object.values(ONBOARDING_COACH_IDS));
    const { container } = render(() => (
      <PresenceOnboarding isRetired={(id) => allRetired.has(id)} onRetire={() => {}} orbitEngaged={() => true} />
    ));
    expect(container.querySelector(".kria-onboarding")).toBeNull();
  });
});
