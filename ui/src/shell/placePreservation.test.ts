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

describe("single restoration path — conversation viewport delegated (task 9.3, §21 IU-10)", () => {
  it("does NOT capture the virtualized conversation viewport (marked by data-scroll-owner)", () => {
    document.body.innerHTML = `
      <div id="threads" style="overflow:auto"></div>
      <div id="stream" class="kria-stream__viewport" data-scroll-owner="conversation"></div>
    `;
    const threads = document.getElementById("threads") as HTMLDivElement;
    const stream = document.getElementById("stream") as HTMLDivElement;
    threads.scrollTop = 120;
    stream.scrollTop = 500;

    const snap = capturePlace();

    // The non-virtualized lane scroller IS captured; the conversation viewport
    // is delegated to conversationPlace and MUST be absent from the snapshot.
    const captured = snap.scroll.map((s) => s.el);
    expect(captured).toContain(threads);
    expect(captured).not.toContain(stream);
  });

  it("restore does not write the delegated conversation viewport's scrollTop", () => {
    document.body.innerHTML = `
      <div id="stream" class="kria-stream__viewport" data-scroll-owner="conversation"></div>
    `;
    const stream = document.getElementById("stream") as HTMLDivElement;
    stream.scrollTop = 500;
    const snap = capturePlace();

    stream.scrollTop = 0; // P-C (the single owner) would set this; P-A/P-B must not
    restorePlace(snap);
    expect(stream.scrollTop).toBe(0); // untouched by placePreservation
  });
});

describe("isConversationOwnedScroller (task 9.3)", () => {
  it("matches by the explicit scroll-owner marker", async () => {
    const { isConversationOwnedScroller } = await import("./placePreservation");
    const el = document.createElement("div");
    el.setAttribute("data-scroll-owner", "conversation");
    expect(isConversationOwnedScroller(el)).toBe(true);
  });

  it("matches by the viewport class", async () => {
    const { isConversationOwnedScroller } = await import("./placePreservation");
    const el = document.createElement("div");
    el.className = "kria-stream__viewport";
    expect(isConversationOwnedScroller(el)).toBe(true);
  });

  it("does not match an ordinary lane scroller", async () => {
    const { isConversationOwnedScroller } = await import("./placePreservation");
    const el = document.createElement("div");
    expect(isConversationOwnedScroller(el)).toBe(false);
  });
});

describe("sticky Composer is not a restoration scroller (task 9.5, IU-10; Req 15.5–15.7)", () => {
  it("the sticky Composer container and its textarea are NOT conversation-owned scrollers", async () => {
    const { isConversationOwnedScroller } = await import("./placePreservation");
    const composer = document.createElement("div");
    composer.className = "kria-converse__composer";
    const textarea = document.createElement("textarea");
    textarea.className = "kria-composer__textarea";
    // The single conversation owner only ever writes the viewport; it must never
    // treat the sticky Composer (or its grow-then-scroll textarea) as the scroller
    // to restore, so it can never scroll the Composer out of view.
    expect(isConversationOwnedScroller(composer)).toBe(false);
    expect(isConversationOwnedScroller(textarea)).toBe(false);
  });

  it("capturePlace excludes the conversation viewport while the Composer is present (no double-touch)", () => {
    document.body.innerHTML = `
      <div class="kria-converse">
        <div id="stream" class="kria-stream__viewport" data-scroll-owner="conversation"></div>
        <div class="kria-converse__composer" data-region="composer">
          <textarea class="kria-composer__textarea"></textarea>
        </div>
      </div>
    `;
    const stream = document.getElementById("stream") as HTMLDivElement;
    stream.scrollTop = 640;

    const snap = capturePlace();
    const captured = snap.scroll.map((s) => s.el);
    // The conversation viewport is delegated to the single owner (conversationPlace)
    // and stays absent from the shell snapshot → no shell/stream competition.
    expect(captured).not.toContain(stream);
  });
});
