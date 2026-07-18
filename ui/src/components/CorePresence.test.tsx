import { describe, it, expect, afterEach } from "vitest";
import { render, cleanup } from "@solidjs/testing-library";
import { CorePresence, CORE_STATE_LABELS } from "./CorePresence";
import { coreStore } from "../stores";
import type { CoreState } from "../stores/coreStore";

const ALL_STATES: CoreState[] = [
  "idle",
  "listening",
  "thinking",
  "planning",
  "speaking",
  "acting",
  "running-automation",
  "watching",
  "remembering",
  "reflecting",
  "learning",
  "waiting",
  "blocked",
  "error",
  "recovering",
];

afterEach(() => {
  cleanup();
  coreStore.reset();
  document.documentElement.removeAttribute("data-reduced-motion");
});

describe("CorePresence", () => {
  it("renders every Core state with the correct accessible label", () => {
    for (const state of ALL_STATES) {
      const { getByRole, unmount } = render(() => <CorePresence state={state} />);
      const el = getByRole("img");
      expect(el.getAttribute("aria-label")).toBe(CORE_STATE_LABELS[state]);
      unmount();
    }
  });

  it("exposes the state via a data-core-state attribute", () => {
    for (const state of ALL_STATES) {
      const { getByRole, unmount } = render(() => <CorePresence state={state} />);
      expect(getByRole("img").getAttribute("data-core-state")).toBe(state);
      unmount();
    }
  });

  it("is a role=img with an accessible name (not an unlabeled decoration)", () => {
    const { getByRole } = render(() => <CorePresence state="thinking" />);
    const el = getByRole("img");
    expect(el.getAttribute("aria-label")).toBe("KRIA is thinking");
  });

  it("renders NO spinner element (Req 3.2 — state via the Core, never a spinner)", () => {
    const { container } = render(() => <CorePresence state="thinking" />);
    expect(container.querySelector('[role="progressbar"]')).toBeNull();
    expect(container.querySelector(".spinner, [class*='spin'], [class*='loader']")).toBeNull();
    // No <svg> spinner either — the Core is CSS-driven layers.
    expect(container.querySelector("svg")).toBeNull();
  });

  it("reflects the live coreStore state when no explicit state prop is given", () => {
    coreStore.setState("acting");
    const { getByRole } = render(() => <CorePresence />);
    expect(getByRole("img").getAttribute("data-core-state")).toBe("acting");
  });

  it("applies the requested named size as a CSS custom property", () => {
    const { getByRole } = render(() => <CorePresence state="idle" size="lg" />);
    expect(getByRole("img").getAttribute("style")).toContain("--core-size: 48px");
  });

  it("accepts a numeric size", () => {
    const { getByRole } = render(() => <CorePresence state="idle" size={64} />);
    expect(getByRole("img").getAttribute("style")).toContain("--core-size: 64px");
  });

  it("renders animated by default", () => {
    const { getByRole } = render(() => <CorePresence state="thinking" />);
    expect(getByRole("img").getAttribute("data-motion")).toBe("animated");
  });

  it("renders a STATIC frame when reducedMotion is forced (Req 3.5)", () => {
    const { getByRole } = render(() => <CorePresence state="thinking" reducedMotion />);
    expect(getByRole("img").getAttribute("data-motion")).toBe("static");
  });

  it("honors the global reduced-motion kill-switch on the document root", () => {
    document.documentElement.setAttribute("data-reduced-motion", "on");
    const { getByRole } = render(() => <CorePresence state="thinking" />);
    expect(getByRole("img").getAttribute("data-motion")).toBe("static");
  });
});
