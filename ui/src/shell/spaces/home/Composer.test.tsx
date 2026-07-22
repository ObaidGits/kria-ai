import { describe, it, expect, afterEach, beforeEach, vi } from "vitest";
import { render, cleanup, fireEvent } from "@solidjs/testing-library";

import HomeComposer, { SUMMON_HINT } from "./Composer";
import { presenceIntent } from "./sharedLight";
import { converseStore } from "../../../stores/converseStore";
import { checkRestingCalm } from "./guardrails";

afterEach(() => {
  cleanup();
  presenceIntent.reset();
});

beforeEach(() => {
  presenceIntent.reset();
  converseStore.updateDraft({ text: "", attachments: [] });
});

describe("Composer (homepage) — unified action target (Req 4.1/4.2/4.3)", () => {
  it("renders exactly ONE unified Composer on the vertical axis (Req 4.1)", () => {
    const { container } = render(() => <HomeComposer />);
    const region = container.querySelector('[data-region="composer"]');
    expect(region).not.toBeNull();
    expect(region?.getAttribute("data-vertical-axis")).toBe("true");
    // The unified input (wrapped Converse Composer) is present exactly once —
    // no second competing ask-field (Req 4.2).
    expect(container.querySelectorAll(".kria-composer").length).toBe(1);
    expect(container.querySelectorAll("textarea").length).toBe(1);
  });

  it("presents the mic as a peer input inside the Composer (Req 4.2)", () => {
    const { getByLabelText } = render(() => <HomeComposer />);
    // Default "full" width profile → the mic peer control is inline.
    expect(getByLabelText("Start voice input")).toBeInTheDocument();
  });

  it("shows a discoverable ⌘K / Ctrl K command hint (Req 4.2)", () => {
    const { container } = render(() => <HomeComposer />);
    const hint = container.querySelector<HTMLButtonElement>('[data-role="palette-hint"]');
    expect(hint).not.toBeNull();
    expect(hint?.tagName).toBe("BUTTON");
    // Advertises the proven summon chord, platform-correct.
    expect(hint?.textContent).toContain(SUMMON_HINT);
    expect(hint?.getAttribute("aria-keyshortcuts")).toBe("Meta+K Control+K");
    expect(hint?.getAttribute("aria-label")).toContain(SUMMON_HINT);
  });

  it("opens the Command Palette from the ⌘K hint — routing only, never sends (Req 4.2)", () => {
    const onOpenPalette = vi.fn();
    const { container } = render(() => <HomeComposer onOpenPalette={onOpenPalette} />);
    const hint = container.querySelector<HTMLButtonElement>('[data-role="palette-hint"]')!;
    hint.click();
    expect(onOpenPalette).toHaveBeenCalledTimes(1);
  });

  it("on focus: strengthens its light AND leans the Core toward it; blur clears both (Req 4.3)", () => {
    const { container } = render(() => <HomeComposer />);
    const region = container.querySelector<HTMLElement>('[data-region="composer"]')!;

    // At rest: no focus, no lean.
    expect(region.getAttribute("data-composer-focused")).toBe("false");
    expect(presenceIntent.lean()).toBe(0);

    // Focus enters the Composer subtree → rim-light strengthens (data flag) and
    // the meaningful-intent lean drives the Core toward the Composer.
    fireEvent.focusIn(region);
    expect(region.getAttribute("data-composer-focused")).toBe("true");
    expect(presenceIntent.lean()).toBe(1);

    // Blur out of the subtree (relatedTarget outside) → both clear.
    fireEvent.focusOut(region, { relatedTarget: document.body });
    expect(region.getAttribute("data-composer-focused")).toBe("false");
    expect(presenceIntent.lean()).toBe(0);
  });

  it("keeps the lean while focus moves BETWEEN controls inside the Composer (Req 4.3)", () => {
    const { container } = render(() => <HomeComposer />);
    const region = container.querySelector<HTMLElement>('[data-region="composer"]')!;
    const textarea = container.querySelector("textarea")!;

    fireEvent.focusIn(region);
    expect(presenceIntent.lean()).toBe(1);

    // focusout whose relatedTarget is still inside the Composer is NOT a blur.
    fireEvent.focusOut(region, { relatedTarget: textarea });
    expect(region.getAttribute("data-composer-focused")).toBe("true");
    expect(presenceIntent.lean()).toBe(1);
  });

  it("reads the SAME per-thread draft, so a staged chip draft appears in the input (Req 4.3)", () => {
    const { container } = render(() => <HomeComposer />);
    const textarea = container.querySelector<HTMLTextAreaElement>("textarea")!;
    expect(textarea.value).toBe("");

    // A Contextual Chip stages a reviewable draft into the shared draft store.
    converseStore.updateDraft({ text: "Draft: weekly report for the team." });

    // The homepage Composer reflects it (same draft — no send).
    expect(textarea.value).toBe("Draft: weekly report for the team.");
  });

  it("is not resting-calm filler (adds no dashboard widgets/cards)", () => {
    const { container } = render(() => <HomeComposer />);
    // The Composer is one of the allowed resting elements, not filler.
    expect(checkRestingCalm(container)).toEqual([]);
  });

  it("forwards an optional class hook", () => {
    const { container } = render(() => <HomeComposer class="probe" />);
    const region = container.querySelector('[data-region="composer"]');
    expect(region?.classList.contains("probe")).toBe(true);
    expect(region?.classList.contains("kria-home-composer")).toBe(true);
  });
});
