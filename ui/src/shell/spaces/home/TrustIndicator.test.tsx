import { describe, it, expect, afterEach, vi } from "vitest";
import { render, cleanup } from "@solidjs/testing-library";
import { createSignal } from "solid-js";

import TrustIndicator from "./TrustIndicator";
import { TRUST_SETTINGS_ROUTE } from "./trustIndicator";
import type { CoreState } from "../../../stores/coreStore";
import type { Route } from "../../router";

afterEach(cleanup);

const flush = () => Promise.resolve();

describe("TrustIndicator — on-device trust affordance (Req 9)", () => {
  it("renders a muted, always-lit on-device confirmation (Req 9.1/9.2)", () => {
    const { container, getByText } = render(() => (
      <TrustIndicator online={() => true} coreState={() => "idle"} />
    ));
    const region = container.querySelector('[data-region="trust-indicator"]');
    expect(region).toBeInTheDocument();
    // Muted tone, never emerald (Req 9.2).
    expect(region?.getAttribute("data-tone")).toBe("muted");
    // Present/lit (Req 9.1).
    expect(region?.getAttribute("data-lit")).toBe("true");
    // Plain, non-marketing label carries the meaning as text (Req 9.2 / 21.2).
    expect(getByText("On-device")).toBeInTheDocument();
  });

  it("STAYS LIT when offline and shows a calm offline hint, never an error (Req 9.1)", () => {
    const { container, getByText } = render(() => (
      <TrustIndicator online={() => false} coreState={() => "idle"} />
    ));
    const region = container.querySelector('[data-region="trust-indicator"]');
    // Offline is healthy for a local-first app: still lit, still muted.
    expect(region?.getAttribute("data-lit")).toBe("true");
    expect(region?.getAttribute("data-connectivity")).toBe("offline");
    expect(region?.getAttribute("data-tone")).toBe("muted");
    // A calm secondary word — not an error tone/state.
    expect(getByText("Offline")).toBeInTheDocument();
    // The dot has no error/off variant.
    expect(container.querySelector(".kria-trust__dot")).toBeInTheDocument();
  });

  it("lights the Core→edge reach cue while acting on the device (Req 9.1)", () => {
    const [state, setState] = createSignal<CoreState>("idle");
    const { container } = render(() => (
      <TrustIndicator online={() => true} coreState={state} />
    ));
    const region = container.querySelector('[data-region="trust-indicator"]')!;
    // No reach at rest.
    expect(region.getAttribute("data-reach")).toBe("false");

    // Acting on the computer → reach appears.
    setState("acting");
    expect(region.getAttribute("data-reach")).toBe("true");

    // Running an automation on the device → reach appears.
    setState("running-automation");
    expect(region.getAttribute("data-reach")).toBe("true");

    // Observation/thinking is NOT acting on the device → no reach.
    setState("thinking");
    expect(region.getAttribute("data-reach")).toBe("false");
  });

  it("routes to the Memory & Privacy Settings group on activation — routing only (Req 9.3)", () => {
    const onNavigate = vi.fn();
    const { container } = render(() => (
      <TrustIndicator online={() => true} coreState={() => "idle"} onNavigate={onNavigate} />
    ));
    const button = container.querySelector<HTMLButtonElement>(".kria-trust__button");
    expect(button).toBeInTheDocument();
    // Keyboard-operable native button (Enter/Space activate natively) (Req 21.1).
    expect(button?.tagName).toBe("BUTTON");

    button!.click();
    expect(onNavigate).toHaveBeenCalledTimes(1);
    expect(onNavigate).toHaveBeenCalledWith(TRUST_SETTINGS_ROUTE);
    const expected: Route = { space: "settings", segment: "memory-privacy" };
    expect(onNavigate).toHaveBeenCalledWith(expected);
  });

  it("exposes an accessible label + a polite once-announcing live region (Req 21.1/21.2)", () => {
    const { container } = render(() => (
      <TrustIndicator online={() => false} coreState={() => "acting"} />
    ));
    const button = container.querySelector(".kria-trust__button")!;
    // Label carries the state as text (meaning never by color alone).
    expect(button.getAttribute("aria-label")).toContain("On-device");
    const live = container.querySelector(".kria-trust__sr")!;
    expect(live.getAttribute("role")).toBe("status");
    expect(live.getAttribute("aria-live")).toBe("polite");
    expect(live.getAttribute("aria-atomic")).toBe("true");
    expect(live.textContent).toContain("offline");
  });

  it("degrades the reach to a static cue under reduced motion (Req 17.4/21.4)", () => {
    const { container } = render(() => (
      <TrustIndicator online={() => true} coreState={() => "acting"} reducedMotion />
    ));
    const region = container.querySelector('[data-region="trust-indicator"]')!;
    expect(region.getAttribute("data-motion")).toBe("static");
    // Meaning is still present as data + reach state, just without motion.
    expect(region.getAttribute("data-reach")).toBe("true");
  });

  it("falls back to a lit, muted resting confirmation if the source throws (design §14)", () => {
    const { container, getByText } = render(() => (
      <TrustIndicator
        online={() => {
          throw new Error("connectivity error");
        }}
        coreState={() => "idle"}
      />
    ));
    const region = container.querySelector('[data-region="trust-indicator"]')!;
    // Never crash the homepage; never an unlit/error trust state.
    expect(region.getAttribute("data-lit")).toBe("true");
    expect(region.getAttribute("data-tone")).toBe("muted");
    expect(getByText("On-device")).toBeInTheDocument();
  });

  it("announces connectivity change once without stealing focus (Req 21.2)", async () => {
    const [online, setOnline] = createSignal(true);
    const { container } = render(() => (
      <TrustIndicator online={online} coreState={() => "idle"} />
    ));
    const live = container.querySelector(".kria-trust__sr")!;
    expect(live.textContent).toContain("locally");

    setOnline(false);
    await flush();
    expect(live.textContent).toContain("offline");
    // The live region never takes focus.
    expect(live.hasAttribute("tabindex")).toBe(false);
  });
});
