import { describe, it, expect, afterEach } from "vitest";
import { render, cleanup } from "@solidjs/testing-library";
import Room, { MAX_PARTICLES } from "./Room";

afterEach(cleanup);

/** Read the numeric percentage from an inline `left`/`top` style value. */
function pct(value: string | null): number {
  return Number.parseFloat((value ?? "").replace("%", ""));
}

describe("Room — homepage environment (Req 1.1, 1.2, 1.3, 1.6)", () => {
  it("renders the full-bleed Room base layer (Req 1.1)", () => {
    const { container } = render(() => <Room />);
    const room = container.querySelector(".kria-room");
    expect(room).toBeInTheDocument();
    expect(room?.querySelector(".kria-room__base")).toBeInTheDocument();
  });

  it("renders the floor sheen and peripheral-darkness depth cues (Req 1.1/1.3)", () => {
    const { container } = render(() => <Room />);
    expect(container.querySelector(".kria-room__floor")).toBeInTheDocument();
    expect(container.querySelector(".kria-room__vignette")).toBeInTheDocument();
  });

  it("renders the full particle field by default, capped at MAX_PARTICLES (Req 1.3)", () => {
    const { container } = render(() => <Room />);
    const motes = container.querySelectorAll(".kria-room__particle");
    expect(MAX_PARTICLES).toBe(30);
    expect(motes.length).toBe(MAX_PARTICLES);
  });

  it("never renders more than MAX_PARTICLES even when asked for more (Req 1.3)", () => {
    const { container } = render(() => <Room particleCount={999} />);
    expect(container.querySelectorAll(".kria-room__particle").length).toBe(MAX_PARTICLES);
  });

  it("clamps the particle count into [0, MAX_PARTICLES] across inputs (Req 1.3)", () => {
    for (const [requested, expected] of [
      [-5, 0],
      [0, 0],
      [7, 7],
      [30, 30],
      [31, 30],
      [1000, 30],
    ] as const) {
      cleanup();
      const { container } = render(() => <Room particleCount={requested} />);
      expect(container.querySelectorAll(".kria-room__particle").length).toBe(expected);
    }
  });

  it("keeps every mote inside the central band — none at the frame edges (Req 1.3)", () => {
    const { container } = render(() => <Room />);
    const motes = Array.from(container.querySelectorAll<HTMLElement>(".kria-room__particle"));
    expect(motes.length).toBeGreaterThan(0);
    for (const mote of motes) {
      const x = pct(mote.style.left);
      const y = pct(mote.style.top);
      // Confined to the central band (design §3.1: central third, no edges).
      expect(x).toBeGreaterThanOrEqual(28);
      expect(x).toBeLessThanOrEqual(72);
      expect(y).toBeGreaterThanOrEqual(28);
      expect(y).toBeLessThanOrEqual(72);
    }
  });

  it("produces a stable, deterministic particle layout across renders (no thrash)", () => {
    const first = render(() => <Room />);
    const posA = Array.from(first.container.querySelectorAll<HTMLElement>(".kria-room__particle")).map(
      (m) => `${m.style.left}|${m.style.top}`,
    );
    cleanup();
    const second = render(() => <Room />);
    const posB = Array.from(
      second.container.querySelectorAll<HTMLElement>(".kria-room__particle"),
    ).map((m) => `${m.style.left}|${m.style.top}`);
    expect(posA).toEqual(posB);
  });

  it("freezes to a static frame under reduced-motion (Req 1.6)", () => {
    const { container } = render(() => <Room reducedMotion />);
    const room = container.querySelector(".kria-room");
    expect(room?.getAttribute("data-motion")).toBe("static");
    // Layers still present — only motion stops (same layout/colors/meaning).
    expect(container.querySelector(".kria-room__particle")).toBeInTheDocument();
  });

  it("marks the animated frame when motion is allowed", () => {
    const { container } = render(() => <Room reducedMotion={false} />);
    expect(container.querySelector(".kria-room")?.getAttribute("data-motion")).toBe("animated");
  });

  it("goes static from the global kill-switch alone, with no prop (Req 1.6/17.4)", () => {
    // The kill-switch is the app-wide `data-reduced-motion="on"` root attribute
    // (same signal CorePresence honors) — no OS media query needed.
    document.documentElement.setAttribute("data-reduced-motion", "on");
    try {
      const { container } = render(() => <Room />);
      expect(container.querySelector(".kria-room")?.getAttribute("data-motion")).toBe("static");
    } finally {
      document.documentElement.removeAttribute("data-reduced-motion");
    }
  });

  it("reacts live when the kill-switch toggles on after mount (Req 1.6)", async () => {
    const { container } = render(() => <Room />);
    const room = container.querySelector(".kria-room");
    expect(room?.getAttribute("data-motion")).toBe("animated");

    // Flip the global kill-switch; the MutationObserver freezes the field.
    document.documentElement.setAttribute("data-reduced-motion", "on");
    try {
      // Allow the MutationObserver microtask to run.
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 0));
      expect(room?.getAttribute("data-motion")).toBe("static");
    } finally {
      document.documentElement.removeAttribute("data-reduced-motion");
    }
  });

  it("keeps the same layout under static — only motion stops (fade-only, Req 17.4)", () => {
    const { container } = render(() => <Room reducedMotion />);
    // All four environment layers remain (identical layout/colors/meaning).
    for (const sel of [
      ".kria-room__base",
      ".kria-room__particles",
      ".kria-room__floor",
      ".kria-room__vignette",
    ]) {
      expect(container.querySelector(sel)).toBeInTheDocument();
    }
    // Motion is flagged static (CSS then freezes the drift keyframes).
    expect(container.querySelector(".kria-room")?.getAttribute("data-motion")).toBe("static");
  });

  it("degrades to a flat neutral base only — no atmosphere layers (design §14 / Req 1.6)", () => {
    const { container } = render(() => <Room degraded />);
    expect(container.querySelector(".kria-room__base")).toBeInTheDocument();
    expect(container.querySelector(".kria-room__particle")).not.toBeInTheDocument();
    expect(container.querySelector(".kria-room__floor")).not.toBeInTheDocument();
    expect(container.querySelector(".kria-room__vignette")).not.toBeInTheDocument();
    expect(container.querySelector(".kria-room")?.getAttribute("data-degraded")).toBe("true");
  });

  it("is decorative: environment layers are aria-hidden (pure presentation)", () => {
    const { container } = render(() => <Room />);
    for (const sel of [".kria-room__base", ".kria-room__particles", ".kria-room__floor", ".kria-room__vignette"]) {
      expect(container.querySelector(sel)?.getAttribute("aria-hidden")).toBe("true");
    }
  });

  it("renders foreground children above the environment layers, not hidden", () => {
    const { container, getByText } = render(() => (
      <Room>
        <button type="button">Talk</button>
      </Room>
    ));
    const content = container.querySelector(".kria-room__content");
    expect(content).toBeInTheDocument();
    expect(content?.getAttribute("aria-hidden")).toBeNull();
    expect(getByText("Talk")).toBeInTheDocument();
  });

  it("forwards an optional class hook for the surrounding layout", () => {
    const { container } = render(() => <Room class="probe" />);
    expect(container.querySelector(".kria-room")?.classList.contains("probe")).toBe(true);
  });
});
