import { describe, it, expect, afterEach } from "vitest";
import { render, screen, cleanup } from "@solidjs/testing-library";
import HomeSpace from "./HomeSpace";
import { checkRestingCalm } from "./guardrails";

afterEach(cleanup);

describe("HomeSpace (scaffold, Req 22.1/22.2)", () => {
  it("renders a labelled Home region", () => {
    render(() => <HomeSpace />);
    expect(screen.getByRole("region", { name: "Home" })).toBeInTheDocument();
  });

  it("is never blank — always renders the Core presence and a heading", () => {
    const { container } = render(() => <HomeSpace />);
    // Core-forward: the homepage Core is INTERACTIVE (Req 2.3), so it is a
    // labelled role=button (activate to talk / press-hold push-to-talk).
    const core = container.querySelector(".kria-home__core [role='button']");
    expect(core).toBeInTheDocument();
    expect(core?.getAttribute("data-interactive")).toBe("true");
    // A real heading anchors the surface (never an empty node).
    expect(
      screen.getByRole("heading", { name: "What can I help with?" }),
    ).toBeInTheDocument();
  });

  it("forwards an optional class hook for the surrounding layout", () => {
    const { container } = render(() => <HomeSpace class="probe" />);
    const section = container.querySelector(".kria-home");
    expect(section).toBeInTheDocument();
    expect(section?.classList.contains("probe")).toBe(true);
  });

  describe("resting calm — no filler at rest (Req 1.5)", () => {
    it("renders no placeholder widgets, empty cards, stat tiles, or filler", () => {
      const { container } = render(() => <HomeSpace />);
      // The runtime resting-calm guardrail sees a calm homepage.
      expect(checkRestingCalm(container)).toEqual([]);
    });

    it("shows only the Core and the optional greeting slot beside it", () => {
      const { container } = render(() => <HomeSpace />);
      // Optional greeting slot present and marked (not filler).
      const greeting = container.querySelector('[data-slot="greeting"]');
      expect(greeting).toBeInTheDocument();
      // No dashboard-style surfaces at rest.
      for (const sel of ["[data-widget]", "[data-stat-tile]", "[data-chart]", "[data-activity-feed]"]) {
        expect(container.querySelector(sel)).not.toBeInTheDocument();
      }
    });
  });

  describe("reduced-motion static Room end-to-end (Req 1.6/17.4)", () => {
    it("freezes the whole Room to a static frame under the global kill-switch", () => {
      document.documentElement.setAttribute("data-reduced-motion", "on");
      try {
        const { container } = render(() => <HomeSpace />);
        // The composed Room (particles/floor/undertone) degrades to static.
        expect(container.querySelector(".kria-room")?.getAttribute("data-motion")).toBe("static");
        // Still Core-forward and never blank (interactive Core → role=button).
        expect(container.querySelector(".kria-home__core [role='button']")).toBeInTheDocument();
      } finally {
        document.documentElement.removeAttribute("data-reduced-motion");
      }
    });
  });
});
