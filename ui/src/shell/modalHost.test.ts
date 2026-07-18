import { describe, it, expect, beforeEach, vi } from "vitest";
import { modalHost, openModal, closeModal, isModalOpen } from "./modalHost";

function makeModal(id: string) {
  return { id, title: id, render: () => null };
}

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
