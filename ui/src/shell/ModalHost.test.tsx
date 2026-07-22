/**
 * ModalHost render integration — controlled focus return (§20.3/§20.4, gap G4).
 *
 * The kit Dialog returns focus to its own trigger only on the self-trigger
 * (`triggerLabel`) path. ModalHost drives it via `open`, so the store captures
 * the opener and returns focus on close. This verifies the full round trip:
 * focus moves INTO the modal on open, and back to the opener on close.
 */
import { describe, it, expect, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@solidjs/testing-library";
import { ModalHost } from "./ModalHost";
import { openModal, closeModal } from "./modalHost";

const flush = () => Promise.resolve();

describe("ModalHost — controlled focus round trip (gap G4)", () => {
  afterEach(() => {
    closeModal();
    cleanup();
    document.body.innerHTML = "";
  });

  it("moves focus into the modal on open and back to the opener on close", async () => {
    render(() => <ModalHost />);

    const opener = document.createElement("button");
    opener.textContent = "Open thing";
    document.body.appendChild(opener);
    opener.focus();
    expect(document.activeElement).toBe(opener);

    openModal({
      id: "m",
      title: "A modal",
      render: () => <button type="button">Inner action</button>,
    });
    // Dialog focuses its first focusable on open (queued microtask).
    await flush();
    const dialog = await screen.findByRole("dialog", { name: "A modal" });
    expect(dialog.contains(document.activeElement)).toBe(true);
    expect(document.activeElement).not.toBe(opener);

    // Escape closes → ModalHost.onOpenChange → closeModal → focus returns.
    fireEvent.keyDown(dialog, { key: "Escape", code: "Escape" });
    await flush();
    await flush();
    expect(document.activeElement).toBe(opener);
  });
});
