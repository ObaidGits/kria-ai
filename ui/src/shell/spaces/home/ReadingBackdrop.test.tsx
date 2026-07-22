/**
 * ReadingBackdrop — component tests (task 8.4, Req 11.1/11.2/11.4).
 *
 * The backdrop is the receded-Room + ambient-Core depth-recession layer. It is
 * pure decoration behind the dominant message stream, so it must:
 *   • reuse the Room (not rebuild it) and render the ambient Core,
 *   • be fully `aria-hidden` (the conversation is the announced surface, Req 11.4),
 *   • carry the hard-dim scrim element (Req 11.2).
 */
import { afterEach, describe, expect, it } from "vitest";
import { render, cleanup } from "@solidjs/testing-library";

import ReadingBackdrop, { READING_PARTICLE_COUNT } from "./ReadingBackdrop";
import { MAX_PARTICLES } from "./Room";

afterEach(cleanup);

describe("ReadingBackdrop", () => {
  it("renders the receded Room + ambient Core + hard-dim scrim", () => {
    const { container } = render(() => <ReadingBackdrop />);

    const root = container.querySelector('[data-region="reading-backdrop"]');
    expect(root).not.toBeNull();
    // Reuses the Room (not a rebuild).
    expect(root!.querySelector('[data-region="room"]')).not.toBeNull();
    // Ambient Core present (CorePresence renders role=img with a state label).
    expect(root!.querySelector(".kria-reading-backdrop__core")).not.toBeNull();
    // Hard-dim scrim element (Req 11.2).
    expect(root!.querySelector(".kria-reading-backdrop__dim")).not.toBeNull();
  });

  it("is entirely aria-hidden — the message stream is the announced surface (Req 11.4)", () => {
    const { container } = render(() => <ReadingBackdrop />);
    const root = container.querySelector('[data-region="reading-backdrop"]')!;
    expect(root.getAttribute("aria-hidden")).toBe("true");
  });

  it("settles the receded particle field within the tokenized cap (Req 11.2)", () => {
    expect(READING_PARTICLE_COUNT).toBeGreaterThan(0);
    expect(READING_PARTICLE_COUNT).toBeLessThanOrEqual(MAX_PARTICLES);

    const { container } = render(() => <ReadingBackdrop />);
    const room = container.querySelector('[data-region="room"]')!;
    // Frozen (settled) particles: the Room is rendered with reducedMotion, so
    // its motion attribute reports the static frame.
    expect(room.getAttribute("data-motion")).toBe("static");
    expect(room.querySelectorAll(".kria-room__particle").length).toBe(READING_PARTICLE_COUNT);
  });
});
