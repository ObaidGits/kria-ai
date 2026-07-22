import { describe, it, expect, beforeEach, vi } from "vitest";
import { modalHost, openModal, closeModal, isModalOpen } from "./modalHost";

function makeModal(id: string) {
  return { id, title: id, render: () => null };
}

/** Flush the microtask that returnFocus schedules on close. */
const flush = () => Promise.resolve();

describe("modalHost — one-modal-at-a-time (Req 1.6)", () => {
  beforeEach(() => {
    closeModal();
  });

  it("opens a modal when none is active", () => {
    expect(isModalOpen()).toBe(false);
    expect(openModal(makeModal("a"))).toBe(true);
    expect(isModalOpen()).toBe(true);
    expect(modalHost.activeModal()?.id).toBe("a");
  });

  it("refuses to open a second modal while one is active", () => {
    expect(openModal(makeModal("a"))).toBe(true);
    // A modal spawning another modal is refused (Req 1.6).
    expect(openModal(makeModal("b"))).toBe(false);
    expect(modalHost.activeModal()?.id).toBe("a");
  });

  it("allows opening again after the active modal is closed", () => {
    openModal(makeModal("a"));
    closeModal();
    expect(isModalOpen()).toBe(false);
    expect(openModal(makeModal("b"))).toBe(true);
    expect(modalHost.activeModal()?.id).toBe("b");
  });

  it("invokes onClose when closed", () => {
    const onClose = vi.fn();
    openModal({ id: "a", title: "a", render: () => null, onClose });
    closeModal();
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("does not close a newer modal when targeting a stale id", () => {
    openModal(makeModal("a"));
    // stale caller targets "b" but "a" is active — must not close
    closeModal("b");
    expect(modalHost.activeModal()?.id).toBe("a");
  });
});

describe("modalHost — controlled opener focus return (§20.3/§20.4, gap G4)", () => {
  beforeEach(() => {
    closeModal();
    document.body.innerHTML = "";
  });

  it("returns focus to the opener (focused element at open time) on close", async () => {
    const opener = document.createElement("button");
    document.body.appendChild(opener);
    opener.focus();
    expect(document.activeElement).toBe(opener);

    openModal(makeModal("a")); // captures document.activeElement as the owner
    closeModal("a");
    await flush();
    expect(document.activeElement).toBe(opener);
  });

  it("honors an explicit descriptor.opener over document.activeElement", async () => {
    const explicit = document.createElement("button");
    const other = document.createElement("button");
    document.body.append(explicit, other);
    other.focus();

    openModal({ id: "a", title: "a", render: () => null, opener: explicit });
    closeModal("a");
    await flush();
    expect(document.activeElement).toBe(explicit);
  });

  it("falls back to #space-root when the opener is removed before close", async () => {
    const spaceRoot = document.createElement("div");
    spaceRoot.id = "space-root";
    spaceRoot.tabIndex = -1;
    const region = document.createElement("section");
    const opener = document.createElement("button");
    region.appendChild(opener);
    document.body.append(spaceRoot, region);
    opener.focus();

    openModal(makeModal("a"));
    region.remove(); // opener + its region gone while modal was up
    closeModal("a");
    await flush();
    expect(document.activeElement).toBe(spaceRoot);
  });

  it("skips a destructive opener for a generic modal (§20.4 never-destructive)", async () => {
    const region = document.createElement("section");
    const heading = document.createElement("h2");
    const approve = document.createElement("button");
    approve.className = "kria-approval-card__approve";
    region.append(heading, approve);
    document.body.appendChild(region);
    approve.focus();

    openModal(makeModal("a")); // layer defaults to generic "modal"
    closeModal("a");
    await flush();
    // Never lands back on the destructive Approve control; owning region heading.
    expect(document.activeElement).toBe(heading);
  });

  it("returns to the originating decision control for an approval-confirm modal", async () => {
    const region = document.createElement("section");
    const approve = document.createElement("button");
    approve.className = "kria-approval-card__approve";
    approve.textContent = "Approve";
    region.appendChild(approve);
    document.body.appendChild(region);
    approve.focus();

    // §20.3: approval-confirm Focus_Return_Owner = originating decision control.
    openModal({ id: "c", title: "Confirm", render: () => null, layer: "approval-confirm" });
    closeModal("c");
    await flush();
    expect(document.activeElement).toBe(approve);
  });
});
