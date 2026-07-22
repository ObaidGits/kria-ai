/**
 * ApprovalCenter component tests (task 4.1).
 *
 * Proves the Center renders the pending queue from `approvalStore` (Req 11.1),
 * that it is the one blocking interrupt — it auto-opens on a new decision and
 * seizes focus onto the card (not the Approve button, Req 11.3/11.5), that Esc
 * does not silently dismiss a pending decision (Req 11.3/17.6), and that
 * approve/deny STAGE typed decisions through the store/event bus rather than
 * executing anything (KRIA runtime-authority invariant).
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, within } from "@solidjs/testing-library";
import { ApprovalCenter } from "./ApprovalCenter";
import { approvalStore, shellStore } from "../../stores";
import { eventBus } from "../../stores/eventBus";
import { closeModal } from "../modalHost";
import { isOverlaySurfaceRegistered } from "../overlayLayers";
import type { ApprovalRequest } from "../../stores/approvalStore";
import { setWindowPresentationActive } from "../../windowing/detachableSurfaces";

const tick = () => new Promise<void>((r) => setTimeout(r, 0));

function makeRequest(overrides: Partial<ApprovalRequest> = {}): ApprovalRequest {
  return {
    id: "req-1",
    type: "tool-hitl",
    title: "Send the drafted email",
    description: "You asked KRIA to reply to Sam once the report was ready.",
    risk: "yellow",
    effects: ["Sends 1 email to sam@example.com"],
    payload: {},
    createdAt: Date.now(),
    status: "pending",
    ...overrides,
  };
}

describe("ApprovalCenter (task 4.1)", () => {
  beforeEach(() => {
    approvalStore.setQueue([]);
    shellStore.setApprovalsOpen(false);
    setWindowPresentationActive(true);
    closeModal();
    vi.restoreAllMocks();
  });

  it("renders the pending approvals from the store (Req 11.1)", () => {
    approvalStore.setQueue([makeRequest(), makeRequest({ id: "req-2", title: "Run the backup workflow" })]);
    shellStore.setApprovalsOpen(true);
    render(() => <ApprovalCenter />);

    const dialog = screen.getByRole("dialog", { name: "Approval Center" });
    expect(within(dialog).getByRole("heading", { name: "Send the drafted email" })).toBeInTheDocument();
    expect(within(dialog).getByRole("heading", { name: "Run the backup workflow" })).toBeInTheDocument();
    expect(within(dialog).getByText("2 pending")).toBeInTheDocument();
  });

  it("shows a calm empty state when nothing is pending", () => {
    shellStore.setApprovalsOpen(true);
    render(() => <ApprovalCenter />);
    expect(screen.getByText("Nothing needs your approval")).toBeInTheDocument();
  });

  it("auto-opens and seizes focus onto the card, not the Approve button (Req 11.3/11.5)", async () => {
    render(() => <ApprovalCenter />);
    expect(screen.queryByRole("dialog", { name: "Approval Center" })).toBeNull();

    approvalStore.addRequest(makeRequest());
    await tick();

    expect(shellStore.approvalsOpen()).toBe(true);
    const dialog = screen.getByRole("dialog", { name: "Approval Center" });
    // Focus landed inside the panel on the card — not on Approve (deliberate).
    const approve = within(dialog).getByRole("button", { name: /Approve/ });
    expect(document.activeElement).not.toBe(approve);
    expect(dialog.contains(document.activeElement)).toBe(true);
  });

  it("registers its panel at the approval layer so it is the sole blocking surface (§20.3, task 8.3)", () => {
    approvalStore.setQueue([makeRequest()]);
    shellStore.setApprovalsOpen(true);
    render(() => <ApprovalCenter />);

    const dialog = screen.getByRole("dialog", { name: "Approval Center" });
    // Registered in the overlay stacking/inertness contract as the blocking
    // surface above palette/notification/voice/Inspector (priority enforced by
    // overlayLayers, verified in overlayLayers.test.ts — here we lock that the
    // Center actually participates rather than relying on DOM order).
    expect(isOverlaySurfaceRegistered(dialog)).toBe(true);
  });

  it("mirrors a pending approval only to the active KRIA window (Req 11.4)", async () => {
    setWindowPresentationActive(false);
    render(() => <ApprovalCenter />);
    approvalStore.addRequest(makeRequest());
    await tick();
    expect(shellStore.approvalsOpen()).toBe(false);

    setWindowPresentationActive(true);
    await tick();
    expect(shellStore.approvalsOpen()).toBe(true);
  });

  it("does not silently dismiss a pending decision on Escape (Req 11.3/17.6)", () => {
    approvalStore.setQueue([makeRequest()]);
    shellStore.setApprovalsOpen(true);
    render(() => <ApprovalCenter />);

    const dialog = screen.getByRole("dialog", { name: "Approval Center" });
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(shellStore.approvalsOpen()).toBe(true);
    expect(dialog).toBeInTheDocument();
  });

  it("swallows a pending-decision Escape and marks it handled so no lower layer peels (§20.3)", () => {
    approvalStore.setQueue([makeRequest()]);
    shellStore.setApprovalsOpen(true);
    render(() => <ApprovalCenter />);

    const dialog = screen.getByRole("dialog", { name: "Approval Center" });
    const event = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    dialog.dispatchEvent(event);
    // Pending decision is not dismissed AND the event is consumed, so the same
    // Escape cannot also exit Immersive or close another overlay (one-layer peel).
    expect(shellStore.approvalsOpen()).toBe(true);
    expect(event.defaultPrevented).toBe(true);
  });

  it("does not dismiss a pending decision on backdrop click (Req 11.3)", () => {
    approvalStore.setQueue([makeRequest()]);
    shellStore.setApprovalsOpen(true);
    render(() => <ApprovalCenter />);

    const dialog = screen.getByRole("dialog", { name: "Approval Center" });
    const overlay = document.querySelector(".kria-approvals__overlay") as HTMLElement;
    expect(overlay).not.toBeNull();
    expect(overlay.classList.contains("is-blocking")).toBe(true);

    fireEvent.click(overlay);

    // The blocking backdrop click is inert while work is pending — no silent
    // dismiss; the panel stays open for an explicit Deny/Keep paused.
    expect(shellStore.approvalsOpen()).toBe(true);
    expect(dialog).toBeInTheDocument();
  });

  it("closes on backdrop click once the queue is empty", () => {
    approvalStore.setQueue([]);
    shellStore.setApprovalsOpen(true);
    render(() => <ApprovalCenter />);

    const overlay = document.querySelector(".kria-approvals__overlay") as HTMLElement;
    fireEvent.click(overlay);
    expect(shellStore.approvalsOpen()).toBe(false);
  });

  it("traps Tab focus inside the panel, wrapping last→first and first→last (Req 11.13)", () => {
    approvalStore.setQueue([makeRequest({ risk: "green", scopeOptions: ["once"] })]);
    shellStore.setApprovalsOpen(true);
    render(() => <ApprovalCenter />);

    const dialog = screen.getByRole("dialog", { name: "Approval Center" });
    // jsdom reports offsetParent === null for every HTMLElement and undefined
    // for SVGElement, which inverts the panel's visibility filter (real browsers
    // report a truthy offsetParent for laid-out controls and null for inert SVG
    // <use> nodes caught by the [href] selector). Recreate the browser view:
    // real interactive controls are "visible", decorative SVG nodes are not.
    const candidates = Array.from(
      dialog.querySelectorAll<HTMLElement>(
        'button:not([disabled]), [href], input, [tabindex]:not([tabindex="-1"])'
      )
    );
    for (const el of candidates) {
      const visible = !(el instanceof SVGElement);
      Object.defineProperty(el, "offsetParent", {
        configurable: true,
        get: () => (visible ? document.body : null),
      });
    }
    const focusables = candidates.filter((el) => el.offsetParent !== null);
    expect(focusables.length).toBeGreaterThan(1);
    const first = focusables[0];
    const last = focusables[focusables.length - 1];

    // Forward Tab off the last control wraps to the first.
    last.focus();
    expect(document.activeElement).toBe(last);
    const fwd = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
    dialog.dispatchEvent(fwd);
    expect(fwd.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(first);

    // Shift+Tab off the first control wraps to the last.
    first.focus();
    expect(document.activeElement).toBe(first);
    const back = new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, cancelable: true });
    dialog.dispatchEvent(back);
    expect(back.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(last);
  });

  it("closes on Escape once the queue is empty", () => {
    approvalStore.setQueue([]);
    shellStore.setApprovalsOpen(true);
    render(() => <ApprovalCenter />);
    const dialog = screen.getByRole("dialog", { name: "Approval Center" });
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(shellStore.approvalsOpen()).toBe(false);
  });

  it("deny STAGES a typed decision through the store/bus — no tool execution (Req 11.1)", () => {
    approvalStore.setQueue([makeRequest()]);
    shellStore.setApprovalsOpen(true);
    const emit = vi.spyOn(eventBus, "emit");
    render(() => <ApprovalCenter />);

    fireEvent.click(screen.getByRole("button", { name: /Deny/ }));

    expect(emit).toHaveBeenCalledWith("approval:resolved", { id: "req-1", action: "deny", reason: undefined });
    expect(approvalStore.queue().find((r) => r.id === "req-1")?.status).toBe("denied");
  });

  it("approve (deliberate press, low risk) STAGES an approve decision (Req 11.3)", () => {
    approvalStore.setQueue([makeRequest({ risk: "green", scopeOptions: ["once"] })]);
    shellStore.setApprovalsOpen(true);
    const emit = vi.spyOn(eventBus, "emit");
    render(() => <ApprovalCenter />);

    fireEvent.click(screen.getByRole("button", { name: /Approve/ }));

    expect(emit).toHaveBeenCalledWith("approval:resolved", { id: "req-1", action: "approve", scope: "once" });
    expect(approvalStore.queue().find((r) => r.id === "req-1")?.status).toBe("approved");
  });

  it("keep paused STAGES a keep-paused decision and leaves it un-approved (Req 11.3)", () => {
    approvalStore.setQueue([makeRequest()]);
    shellStore.setApprovalsOpen(true);
    const emit = vi.spyOn(eventBus, "emit");
    render(() => <ApprovalCenter />);

    fireEvent.click(screen.getByRole("button", { name: /Keep paused/ }));

    expect(emit).toHaveBeenCalledWith("approval:resolved", { id: "req-1", action: "keep-paused" });
    expect(approvalStore.queue().find((r) => r.id === "req-1")?.status).toBe("kept-paused");
  });
});
