/**
 * HiddenDock — presence-homepage navigation rail (Req 7, design §7.1).
 *
 * Proves the reveal model + a11y contract of task 6.1:
 *   • invisible at rest, yet fully tab-/AT-reachable (no display:none/hidden);
 *   • reveals on EACH intent — Alt, left-edge cursor, keyboard/AT focus, pin,
 *     and ⌘K/Command-Palette open (Req 7.1);
 *   • dims the Room when revealed (Req 7.4);
 *   • dismisses on blur/Escape and returns focus per the §20.4 ladder (Req 7.4);
 *   • preserves the canonical Space order + `aria-current` + one-click switch
 *     inherited from Dock (Req 7.2);
 *   • degrades to a static reveal under reduced motion (Req 17.4).
 */
import { describe, it, expect, afterEach, beforeEach } from "vitest";
import { render, cleanup, fireEvent } from "@solidjs/testing-library";

import { HiddenDock } from "./HiddenDock";
import hiddenDockCss from "./HiddenDock.css?raw";
import { ALL_SPACES, navigate } from "./router";
import { SPACE_META } from "./spaces";
import { homeStore } from "../stores/homeStore";
import { shellStore } from "../stores/shellStore";

/** Flush Solid's reactive queue + any queued microtask (returnFocus). */
const tick = () => new Promise<void>((r) => setTimeout(r, 0));

beforeEach(() => {
  homeStore.reset();
  shellStore.setPaletteOpen(false);
  navigate("converse");
  document.body.innerHTML = "";
});

afterEach(() => {
  cleanup();
  homeStore.reset();
});

function railOf(container: HTMLElement): HTMLElement {
  return container.querySelector<HTMLElement>('[data-region="hidden-dock"]')!;
}

describe("HiddenDock — invisible at rest, but keyboard/AT reachable (Req 7.1/7.3/14.5)", () => {
  it("renders hidden at rest (data-revealed=false) but keeps the full Space list in the DOM", () => {
    const { container } = render(() => <HiddenDock />);
    const rail = railOf(container);
    expect(rail).toBeInTheDocument();
    expect(rail.getAttribute("data-revealed")).toBe("false");
    expect(homeStore.dockRevealed()).toBe(false);

    // All seven Spaces are present as real, tab-reachable <button>s even while
    // hidden — never dropped from the DOM / tab order.
    const buttons = rail.querySelectorAll<HTMLButtonElement>(".kria-dock__button");
    expect(buttons.length).toBe(ALL_SPACES.length);
    buttons.forEach((b) => expect(b.tagName).toBe("BUTTON"));
  });

  it("recedes via transform/opacity ONLY — never display:none or visibility:hidden (Req 7.3)", () => {
    // Assert against the actual CSS RULES, not the doc comments (which mention
    // these techniques as forbidden). Strip /* … */ comments first.
    const rules = hiddenDockCss.replace(/\/\*[\s\S]*?\*\//g, "");
    // The hidden state must not remove the rail from the a11y tree / tab order.
    expect(rules).not.toMatch(/visibility:\s*hidden/);
    expect(rules).not.toMatch(/display:\s*none/);
    // It uses the AT-safe recede technique instead.
    expect(hiddenDockCss).toMatch(/transform:\s*translateX\(-100%\)/);
    expect(hiddenDockCss).toMatch(/opacity:\s*0/);
    // …and paints on keyboard entry via :focus-within.
    expect(hiddenDockCss).toMatch(/:focus-within/);
  });
});

describe("HiddenDock — reveals on explicit intent (Req 7.1)", () => {
  it("reveals while Alt is held and hides on release", async () => {
    const { container } = render(() => <HiddenDock />);
    const rail = railOf(container);

    fireEvent.keyDown(document, { key: "Alt" });
    await tick();
    expect(rail.getAttribute("data-revealed")).toBe("true");

    fireEvent.keyUp(document, { key: "Alt" });
    await tick();
    expect(rail.getAttribute("data-revealed")).toBe("false");
  });

  it("reveals when the cursor reaches the left edge and hides when it leaves", async () => {
    const { container } = render(() => <HiddenDock />);
    const rail = railOf(container);

    fireEvent.mouseMove(document, { clientX: 0 });
    await tick();
    expect(rail.getAttribute("data-revealed")).toBe("true");

    fireEvent.mouseMove(document, { clientX: 800 });
    await tick();
    expect(rail.getAttribute("data-revealed")).toBe("false");
  });

  it("reveals when keyboard/AT focus enters the rail (Req 7.3/14.5)", async () => {
    const { container } = render(() => <HiddenDock />);
    const rail = railOf(container);

    fireEvent.focusIn(rail, { relatedTarget: document.body });
    await tick();
    expect(rail.getAttribute("data-revealed")).toBe("true");
  });

  it("reveals while the Command Palette (⌘K) is open (Req 7.1)", async () => {
    const { container } = render(() => <HiddenDock />);
    const rail = railOf(container);

    shellStore.setPaletteOpen(true);
    await tick();
    expect(rail.getAttribute("data-revealed")).toBe("true");

    shellStore.setPaletteOpen(false);
    await tick();
    expect(rail.getAttribute("data-revealed")).toBe("false");
  });

  it("stays revealed while pinned and cannot be dismissed by losing other intents (Req 7.1)", async () => {
    const { container } = render(() => <HiddenDock />);
    const rail = railOf(container);

    const pin = rail.querySelector<HTMLButtonElement>(".kria-hidden-dock__pin")!;
    expect(pin.getAttribute("aria-pressed")).toBe("false");
    pin.click();
    await tick();
    expect(homeStore.dockPinned()).toBe(true);
    expect(rail.getAttribute("data-revealed")).toBe("true");
    expect(pin.getAttribute("aria-pressed")).toBe("true");

    // A blur / cursor-leave does not hide a pinned rail.
    fireEvent.mouseMove(document, { clientX: 800 });
    fireEvent.focusOut(rail, { relatedTarget: document.body });
    await tick();
    expect(rail.getAttribute("data-revealed")).toBe("true");

    // Explicit unpin releases it.
    pin.click();
    await tick();
    expect(homeStore.dockPinned()).toBe(false);
    expect(rail.getAttribute("data-revealed")).toBe("false");
  });
});

describe("HiddenDock — dim-over-Room + dismissal (Req 7.4/7.5)", () => {
  it("dims the Room (scrim) in lock-step with the reveal", async () => {
    const { container } = render(() => <HiddenDock />);
    const rail = railOf(container);
    const scrim = container.querySelector<HTMLElement>(".kria-hidden-dock__scrim")!;

    expect(scrim.getAttribute("data-revealed")).toBe("false");
    fireEvent.keyDown(document, { key: "Alt" });
    await tick();
    expect(rail.getAttribute("data-revealed")).toBe("true");
    expect(scrim.getAttribute("data-revealed")).toBe("true");
    // The scrim is presentational + non-interactive (never competes / traps).
    expect(scrim.getAttribute("aria-hidden")).toBe("true");
  });

  it("dismisses on blur (focus leaving the rail) when not pinned", async () => {
    const { container } = render(() => <HiddenDock />);
    const rail = railOf(container);

    fireEvent.focusIn(rail, { relatedTarget: document.body });
    await tick();
    expect(rail.getAttribute("data-revealed")).toBe("true");

    fireEvent.focusOut(rail, { relatedTarget: document.body });
    await tick();
    expect(rail.getAttribute("data-revealed")).toBe("false");
  });

  it("dismisses on Escape and returns focus to the pre-reveal owner (§20.4 ladder)", async () => {
    const owner = document.createElement("button");
    owner.type = "button";
    owner.textContent = "opener";
    document.body.appendChild(owner);
    owner.focus();

    const { container } = render(() => <HiddenDock />);
    const rail = railOf(container);

    // Focus enters the rail; the owner (the element losing focus) is captured.
    fireEvent.focusIn(rail, { relatedTarget: owner });
    await tick();
    expect(rail.getAttribute("data-revealed")).toBe("true");

    // Escape → focus returns to the owner via the §20.4 ladder.
    fireEvent.keyDown(rail, { key: "Escape" });
    await tick();
    expect(document.activeElement).toBe(owner);
  });
});

describe("HiddenDock — canonical order + one-click switch preserved (Req 7.2)", () => {
  it("renders the seven Spaces in canonical ALL_SPACES order", () => {
    const { container } = render(() => <HiddenDock />);
    const rail = railOf(container);
    const labels = [...rail.querySelectorAll<HTMLButtonElement>(".kria-dock__button")].map((b) =>
      b.getAttribute("aria-label"),
    );
    expect(labels).toEqual(ALL_SPACES.map((s) => SPACE_META[s].label));
  });

  it("switches Space in one click and marks the active Space with aria-current", async () => {
    const { container } = render(() => <HiddenDock />);
    const rail = railOf(container);
    const memoryBtn = rail.querySelector<HTMLButtonElement>(
      `.kria-dock__button[aria-label="${SPACE_META.memory.label}"]`,
    )!;

    memoryBtn.click();
    await tick();
    expect(memoryBtn.getAttribute("aria-current")).toBe("page");
    // Exactly one active Space at a time.
    expect(rail.querySelectorAll('.kria-dock__button[aria-current="page"]').length).toBe(1);
  });
});

describe("HiddenDock — reduced motion (Req 17.4/21.4)", () => {
  it("marks the rail + scrim static when reduced motion is forced", () => {
    const { container } = render(() => <HiddenDock reducedMotion />);
    expect(railOf(container).getAttribute("data-motion")).toBe("static");
    expect(
      container.querySelector<HTMLElement>(".kria-hidden-dock__scrim")!.getAttribute("data-motion"),
    ).toBe("static");
  });
});
