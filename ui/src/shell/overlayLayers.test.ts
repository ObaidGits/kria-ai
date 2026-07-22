/**
 * Overlay layering + inertness contract tests (task 8.2, design §20.3).
 *
 * Proves the EXPLICIT stacking priority and background inertness — never
 * incidental DOM/portal mount order:
 *   • pending Approval Center outranks command palette / notification / voice /
 *     Inspector and inerts them (Req 11.9 / 11.13);
 *   • the nested approval confirmation outranks (and inerts) the Approval
 *     Center itself (Req 11.9);
 *   • only one blocking modal at a time (Req 1.6);
 *   • lower surfaces are marked inert (not merely painted under).
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import {
  OVERLAY_LAYER_PRIORITY,
  activeBlockingPriority,
  initOverlayInertness,
  registerOverlaySurface,
} from "./overlayLayers";
import { approvalStore } from "../stores";
import { openModal, closeModal, type ModalDescriptor } from "./modalHost";
import type { ApprovalRequest } from "../stores/approvalStore";

const tick = () => new Promise<void>((r) => setTimeout(r, 0));

function makeRequest(overrides: Partial<ApprovalRequest> = {}): ApprovalRequest {
  return {
    id: "req-1",
    type: "tool-hitl",
    title: "Send the drafted email",
    description: "why",
    risk: "yellow",
    effects: ["Sends 1 email"],
    payload: {},
    createdAt: Date.now(),
    status: "pending",
    ...overrides,
  };
}

function makeModal(id: string, layer?: ModalDescriptor["layer"]): ModalDescriptor {
  return { id, title: id, layer, render: () => null };
}

function el(): HTMLElement {
  const node = document.createElement("div");
  document.body.appendChild(node);
  return node;
}

function isInert(node: HTMLElement): boolean {
  return node.hasAttribute("inert") && node.getAttribute("aria-hidden") === "true";
}

describe("overlay layering priority (design §20.3, Req 11.9)", () => {
  beforeEach(() => {
    approvalStore.setQueue([]);
    closeModal();
  });

  it("orders layers lowest→highest by explicit token priority, not DOM order", () => {
    // shell/inspector < floating < palette/modal < approval < approval-confirm.
    expect(OVERLAY_LAYER_PRIORITY.shell).toBeLessThan(OVERLAY_LAYER_PRIORITY.floating);
    expect(OVERLAY_LAYER_PRIORITY.floating).toBeLessThan(OVERLAY_LAYER_PRIORITY.palette);
    expect(OVERLAY_LAYER_PRIORITY.palette).toEqual(OVERLAY_LAYER_PRIORITY.modal);
    expect(OVERLAY_LAYER_PRIORITY.modal).toBeLessThan(OVERLAY_LAYER_PRIORITY.approval);
    expect(OVERLAY_LAYER_PRIORITY.approval).toBeLessThan(
      OVERLAY_LAYER_PRIORITY["approval-confirm"],
    );
  });

  it("pending approval is the top blocking layer above palette/notify/voice", () => {
    expect(activeBlockingPriority()).toBe(0);
    approvalStore.setQueue([makeRequest()]);
    expect(activeBlockingPriority()).toBe(OVERLAY_LAYER_PRIORITY.approval);
  });

  it("nested approval confirmation outranks the pending Approval Center", () => {
    approvalStore.setQueue([makeRequest()]);
    openModal(makeModal("approval-confirm-req-1", "approval-confirm"));
    expect(activeBlockingPriority()).toBe(OVERLAY_LAYER_PRIORITY["approval-confirm"]);
    expect(activeBlockingPriority()).toBeGreaterThan(OVERLAY_LAYER_PRIORITY.approval);
  });

  it("a plain user modal blocks at the modal layer (below approval)", () => {
    openModal(makeModal("user-modal"));
    expect(activeBlockingPriority()).toBe(OVERLAY_LAYER_PRIORITY.modal);
    expect(activeBlockingPriority()).toBeLessThan(OVERLAY_LAYER_PRIORITY.approval);
  });

  it("refuses a second modal — one blocking modal at a time (Req 1.6)", () => {
    expect(openModal(makeModal("a"))).toBe(true);
    expect(openModal(makeModal("b"))).toBe(false);
  });
});

describe("overlay inertness contract (design §20.3, Req 11.13)", () => {
  let dispose: (() => void) | undefined;
  const cleanups: Array<() => void> = [];

  beforeEach(() => {
    approvalStore.setQueue([]);
    closeModal();
    dispose = initOverlayInertness();
  });

  afterEach(() => {
    cleanups.splice(0).forEach((c) => c());
    dispose?.();
    document.body.innerHTML = "";
  });

  function register(layer: Parameters<typeof registerOverlaySurface>[1]): HTMLElement {
    const node = el();
    cleanups.push(registerOverlaySurface(node, layer));
    return node;
  }

  it("does not inert anything when no blocking layer is active", async () => {
    const palette = register("palette");
    const voice = register("floating");
    await tick();
    expect(isInert(palette)).toBe(false);
    expect(isInert(voice)).toBe(false);
  });

  it("pending approval inerts palette/notification/voice/shell but not itself", async () => {
    const shell = register("shell");
    const palette = register("palette");
    const voice = register("floating");
    const approval = register("approval");

    approvalStore.setQueue([makeRequest()]);
    await tick();

    expect(isInert(shell)).toBe(true);
    expect(isInert(palette)).toBe(true);
    expect(isInert(voice)).toBe(true);
    // The blocking surface itself is never inerted.
    expect(isInert(approval)).toBe(false);
  });

  it("nested confirmation inerts the Approval Center; confirm stays interactive", async () => {
    const approval = register("approval");
    const confirm = register("approval-confirm");

    approvalStore.setQueue([makeRequest()]);
    openModal(makeModal("approval-confirm-req-1", "approval-confirm"));
    await tick();

    expect(isInert(approval)).toBe(true);
    expect(isInert(confirm)).toBe(false);
  });

  it("clears inertness when the blocking layer goes away", async () => {
    const palette = register("palette");
    approvalStore.setQueue([makeRequest()]);
    await tick();
    expect(isInert(palette)).toBe(true);

    approvalStore.setQueue([]);
    await tick();
    expect(isInert(palette)).toBe(false);
  });
});
