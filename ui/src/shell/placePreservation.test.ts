/**
 * Place-preservation tests (task 4.3, Req 13.4).
 *
 * Proves the helper restores the user's transient place after an interruption:
 * the focused control, the caret/selection inside a text field, and scroll
 * offsets — so approving/denying returns the user exactly where they were.
 */
import { describe, it, expect, afterEach } from "vitest";
import { capturePlace, restorePlace } from "./placePreservation";

afterEach(() => {
  document.body.innerHTML = "";
});

describe("place preservation (task 4.3, Req 13.4)", () => {
  it("restores focus to the previously focused control", () => {
    document.body.innerHTML = `
      <textarea id="composer"></textarea>
      <button id="approve">Approve</button>
    `;
    const composer = document.getElementById("composer") as HTMLTextAreaElement;
    const approve = document.getElementById("approve") as HTMLButtonElement;

    composer.focus();
    const snap = capturePlace();

    // Interruption seizes focus.
    approve.focus();
    expect(document.activeElement).toBe(approve);

    restorePlace(snap);
    expect(document.activeElement).toBe(composer);
  });

  it("restores the caret/selection inside a text field", () => {
    document.body.innerHTML = `<textarea id="composer">hello world</textarea>`;
    const composer = document.getElementById("composer") as HTMLTextAreaElement;
    composer.focus();
    composer.setSelectionRange(2, 7);

    const snap = capturePlace();
    composer.setSelectionRange(0, 0);

    restorePlace(snap);
    expect(composer.selectionStart).toBe(2);
    expect(composer.selectionEnd).toBe(7);
  });

  it("restores scroll offset of a scrollable region", () => {
    document.body.innerHTML = `<div id="stream"></div>`;
    const stream = document.getElementById("stream") as HTMLDivElement;
    // jsdom lets us set scrollTop directly.
    stream.scrollTop = 240;

    const snap = capturePlace();
    stream.scrollTop = 0;

    restorePlace(snap);
    expect(stream.scrollTop).toBe(240);
  });

  it("safely skips a target that was removed from the document", () => {
    document.body.innerHTML = `<button id="gone">x</button>`;
    const gone = document.getElementById("gone") as HTMLButtonElement;
    gone.focus();
    const snap = capturePlace();

    gone.remove();
    // Must not throw when the captured element is detached.
    expect(() => restorePlace(snap)).not.toThrow();
  });

  it("no-ops on a null snapshot", () => {
    expect(() => restorePlace(null)).not.toThrow();
  });
});
