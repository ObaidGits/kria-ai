/**
 * CompanionEmber — component/interaction tests (task 8.3, Req 15).
 *
 * Drives the real component with the `active`/`enabled`/`onReturn` overrides so
 * the tests stay deterministic without steering the whole View-Mode machine or
 * the native window. Verifies:
 *   • the ember MIRRORS coreStore state, read-only (Req 15.1),
 *   • it brightens ONLY for meaningful needs (Req 15.2),
 *   • the opt-out / not-active states render nothing (Req 15.4),
 *   • click-to-talk + continuous return + reposition are keyboard-operable
 *     real controls, and the AT live region announces (Req 15.3/15.4).
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import { render, cleanup, fireEvent } from "@solidjs/testing-library";

import { CompanionEmber } from "./CompanionEmber";
import { coreStore } from "../../../stores/coreStore";
import { homeStore } from "../../../stores/homeStore";
import { DEFAULT_EMBER_ANCHOR, nextEmberAnchor } from "./companionEmber";

afterEach(() => {
  cleanup();
  coreStore.reset();
  homeStore.reset();
});

const on = () => true;

describe("CompanionEmber — visibility gating (Req 15.4)", () => {
  it("renders nothing when not active", () => {
    const { container } = render(() => <CompanionEmber active={() => false} enabled={on} />);
    expect(container.querySelector('[data-region="companion-ember"]')).not.toBeInTheDocument();
  });

  it("renders nothing when opted out (one-setting opt-out)", () => {
    const { container } = render(() => <CompanionEmber active={on} enabled={() => false} />);
    expect(container.querySelector('[data-region="companion-ember"]')).not.toBeInTheDocument();
  });

  it("renders the ember when active and enabled", () => {
    const { container } = render(() => <CompanionEmber active={on} enabled={on} />);
    expect(container.querySelector('[data-region="companion-ember"]')).toBeInTheDocument();
  });
});

describe("CompanionEmber — mirrors Core state read-only (Req 15.1)", () => {
  it("renders the CorePresence glyph at the live coreStore state and follows changes", () => {
    coreStore.setState("thinking");
    const { container } = render(() => <CompanionEmber active={on} enabled={on} />);
    const glyph = () => container.querySelector(".kria-companion-ember__glyph [data-core-state]");
    expect(glyph()?.getAttribute("data-core-state")).toBe("thinking");

    coreStore.setState("speaking");
    expect(glyph()?.getAttribute("data-core-state")).toBe("speaking");

    // The ember never wrote coreStore — it only mirrored it.
    expect(coreStore.state()).toBe("speaking");
  });
});

describe("CompanionEmber — brightens only for meaningful needs (Req 15.2)", () => {
  function brightened(container: HTMLElement): string | null {
    return container.querySelector('[data-region="companion-ember"]')?.getAttribute("data-brightened") ?? null;
  }

  it("stays dim for idle and ordinary work states", () => {
    coreStore.setState("idle");
    const { container } = render(() => <CompanionEmber active={on} enabled={on} />);
    expect(brightened(container)).toBe("false");

    coreStore.setState("thinking");
    expect(brightened(container)).toBe("false");

    coreStore.setState("acting");
    expect(brightened(container)).toBe("false");
  });

  it("brightens for attention (meaningful-need) states", () => {
    coreStore.setState("idle");
    const { container } = render(() => <CompanionEmber active={on} enabled={on} />);
    expect(brightened(container)).toBe("false");

    coreStore.setState("blocked");
    expect(brightened(container)).toBe("true");

    coreStore.setState("waiting");
    expect(brightened(container)).toBe("true");

    coreStore.setState("idle");
    expect(brightened(container)).toBe("false");
  });
});

describe("CompanionEmber — controls + AT (Req 15.3/15.4)", () => {
  it("exposes keyboard-operable Return and reposition buttons", () => {
    const { getByLabelText } = render(() => <CompanionEmber active={on} enabled={on} />);
    expect(getByLabelText("Return to KRIA")).toBeInTheDocument();
    expect(getByLabelText("Move companion to next corner")).toBeInTheDocument();
  });

  it("continuous return invokes the sanctioned return path", () => {
    const onReturn = vi.fn();
    const { getByLabelText } = render(() => (
      <CompanionEmber active={on} enabled={on} onReturn={onReturn} />
    ));
    fireEvent.click(getByLabelText("Return to KRIA"));
    expect(onReturn).toHaveBeenCalledTimes(1);
  });

  it("reposition cycles the ember anchor corner", () => {
    const { container, getByLabelText } = render(() => <CompanionEmber active={on} enabled={on} />);
    const ember = () => container.querySelector('[data-region="companion-ember"]');
    expect(ember()?.getAttribute("data-anchor")).toBe(DEFAULT_EMBER_ANCHOR);

    fireEvent.click(getByLabelText("Move companion to next corner"));
    expect(ember()?.getAttribute("data-anchor")).toBe(nextEmberAnchor(DEFAULT_EMBER_ANCHOR));
  });

  it("announces mood/need changes via a polite live region", () => {
    coreStore.setState("idle");
    const { container } = render(() => <CompanionEmber active={on} enabled={on} />);
    const live = container.querySelector('.kria-companion-ember__live');
    expect(live?.getAttribute("aria-live")).toBe("polite");
    expect(live?.getAttribute("role")).toBe("status");
    expect(live?.textContent).toBe("KRIA is idle");

    coreStore.setState("blocked");
    expect(live?.textContent).toContain("needs your attention");
  });
});
