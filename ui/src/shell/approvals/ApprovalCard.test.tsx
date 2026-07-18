/**
 * ApprovalCard component tests (task 4.1).
 *
 * Proves the card anatomy (what/why/risk/effects/evidence — Req 11.2), that
 * deny/keep-paused are single actions and approve is deliberate (Req 11.3), that
 * high-risk/irreversible requires an explicit confirm before staging approval
 * (Req 11.3), that risk is conveyed by icon + text not color alone (Req 17.3),
 * and that evidence is sanitized before it reaches the DOM (design.md §1.17).
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@solidjs/testing-library";
import { ApprovalCard } from "./ApprovalCard";
import { modalHost, closeModal } from "../modalHost";
import type { ApprovalRequest, RiskLevel } from "../../stores/approvalStore";

function makeRequest(overrides: Partial<ApprovalRequest> = {}): ApprovalRequest {
  return {
    id: "req-1",
    type: "tool-hitl",
    title: "Delete 42 cached files",
    description: "Frees ~1.2 GB so the next build has room.",
    risk: "green",
    effects: ["Removes ~1.2 GB from the build cache", "Next build will be slower once"],
    evidence: "Cache dir: /tmp/kria-build-cache",
    payload: { cmd: "clear-cache" },
    createdAt: Date.now(),
    status: "pending",
    ...overrides,
  };
}

describe("ApprovalCard (task 4.1)", () => {
  beforeEach(() => {
    closeModal();
  });

  it("states what/why/effects/evidence (Req 11.2)", () => {
    render(() => (
      <ApprovalCard request={makeRequest()} onApprove={vi.fn()} onDeny={vi.fn()} onKeepPaused={vi.fn()} />
    ));
    // What
    expect(screen.getByRole("heading", { name: "Delete 42 cached files" })).toBeInTheDocument();
    // Why
    expect(screen.getByText(/Frees ~1.2 GB/)).toBeInTheDocument();
    // Effects
    expect(screen.getByText("Removes ~1.2 GB from the build cache")).toBeInTheDocument();
    // Evidence
    expect(screen.getByText(/kria-build-cache/)).toBeInTheDocument();
    // Requested action is visibly KRIA-authored, not user content (Req 20.5).
    expect(screen.getByLabelText("AI-authored by KRIA")).toHaveTextContent("Requested by KRIA");
  });

  it("conveys risk by icon + text, not color alone (Req 17.3)", () => {
    const { unmount } = render(() => (
      <ApprovalCard request={makeRequest({ risk: "green" })} onApprove={vi.fn()} onDeny={vi.fn()} onKeepPaused={vi.fn()} />
    ));
    expect(screen.getByText("Low risk")).toBeInTheDocument();
    unmount();

    const levels: Array<[RiskLevel, string]> = [
      ["yellow", "Medium risk"],
      ["red", "High risk"],
      ["black", "Critical / irreversible"],
    ];
    for (const [risk, label] of levels) {
      const view = render(() => (
        <ApprovalCard request={makeRequest({ risk })} onApprove={vi.fn()} onDeny={vi.fn()} onKeepPaused={vi.fn()} />
      ));
      expect(screen.getByText(label)).toBeInTheDocument();
      view.unmount();
    }
  });

  it("deny is a single action (Req 11.3)", () => {
    const onDeny = vi.fn();
    render(() => (
      <ApprovalCard request={makeRequest()} onApprove={vi.fn()} onDeny={onDeny} onKeepPaused={vi.fn()} />
    ));
    fireEvent.click(screen.getByRole("button", { name: /Deny/ }));
    expect(onDeny).toHaveBeenCalledTimes(1);
  });

  it("keep paused is a single action (Req 11.3)", () => {
    const onKeepPaused = vi.fn();
    render(() => (
      <ApprovalCard request={makeRequest()} onApprove={vi.fn()} onDeny={vi.fn()} onKeepPaused={onKeepPaused} />
    ));
    fireEvent.click(screen.getByRole("button", { name: /Keep paused/ }));
    expect(onKeepPaused).toHaveBeenCalledTimes(1);
  });

  it("low/medium-risk approve stages the decision on a deliberate press (Req 11.3)", () => {
    const onApprove = vi.fn();
    render(() => (
      <ApprovalCard
        request={makeRequest({ risk: "green", scopeOptions: ["session"] })}
        onApprove={onApprove}
        onDeny={vi.fn()}
        onKeepPaused={vi.fn()}
      />
    ));
    // Approve is NOT the initially focused control — deliberate action required.
    const approve = screen.getByRole("button", { name: /Approve/ });
    expect(document.activeElement).not.toBe(approve);

    fireEvent.click(approve);
    expect(onApprove).toHaveBeenCalledTimes(1);
    expect(onApprove).toHaveBeenCalledWith("session");
    // No modal — low risk needs no extra confirm.
    expect(modalHost.activeModal()).toBeNull();
  });

  it("high-risk approve requires an explicit confirm before staging (Req 11.3)", () => {
    const onApprove = vi.fn();
    render(() => (
      <ApprovalCard
        request={makeRequest({ risk: "red" })}
        onApprove={onApprove}
        onDeny={vi.fn()}
        onKeepPaused={vi.fn()}
      />
    ));
    // Pressing Approve does NOT stage the decision — it opens a confirm modal
    // via the one-at-a-time modal host.
    fireEvent.click(screen.getByRole("button", { name: /Approve/ }));
    expect(onApprove).not.toHaveBeenCalled();
    const modal = modalHost.activeModal();
    expect(modal).not.toBeNull();
    expect(modal!.id).toBe("approval-confirm-req-1");

    // Rendering + confirming the modal footer stages the approval.
    render(() => <>{modal!.footer}</>);
    fireEvent.click(screen.getByRole("button", { name: "Yes, approve" }));
    expect(onApprove).toHaveBeenCalledTimes(1);
  });

  it("irreversible (green risk) still requires an explicit confirm (Req 11.3)", () => {
    const onApprove = vi.fn();
    render(() => (
      <ApprovalCard
        request={makeRequest({ risk: "green", irreversible: true })}
        onApprove={onApprove}
        onDeny={vi.fn()}
        onKeepPaused={vi.fn()}
      />
    ));
    fireEvent.click(screen.getByRole("button", { name: /Approve/ }));
    expect(onApprove).not.toHaveBeenCalled();
    expect(modalHost.activeModal()).not.toBeNull();
  });

  it("shows the scope ladder and approves at the selected scope (Req 7.3)", () => {
    const onApprove = vi.fn();
    render(() => (
      <ApprovalCard
        request={makeRequest({
          type: "capability-run",
          risk: "green",
          scopeOptions: ["once", "session", "workspace", "always"],
        })}
        onApprove={onApprove}
        onDeny={vi.fn()}
        onKeepPaused={vi.fn()}
      />
    ));
    // The scope radiogroup is present with all four options (Req 7.3).
    const group = screen.getByRole("radiogroup", { name: "Grant scope" });
    expect(group).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /Just once/ })).toHaveAttribute("aria-checked", "true");

    // Selecting "This workspace" then approving stages that scope.
    fireEvent.click(screen.getByRole("radio", { name: /This workspace/ }));
    fireEvent.click(screen.getByRole("button", { name: /Approve/ }));
    expect(onApprove).toHaveBeenCalledWith("workspace");
  });

  it("hides the scope ladder when only one scope is offered", () => {
    render(() => (
      <ApprovalCard
        request={makeRequest({ risk: "green", scopeOptions: ["once"] })}
        onApprove={vi.fn()}
        onDeny={vi.fn()}
        onKeepPaused={vi.fn()}
      />
    ));
    expect(screen.queryByRole("radiogroup", { name: "Grant scope" })).toBeNull();
  });

  it("sanitizes evidence — no script survives (design.md §1.17)", () => {
    const { container } = render(() => (
      <ApprovalCard
        request={makeRequest({ evidence: '<img src=x onerror="alert(1)"><script>alert(2)</script>ok' })}
        onApprove={vi.fn()}
        onDeny={vi.fn()}
        onKeepPaused={vi.fn()}
      />
    ));
    expect(container.querySelector("script")).toBeNull();
    const body = container.querySelector(".kria-approval-card__evidencebody");
    expect(body?.innerHTML ?? "").not.toContain("onerror");
  });
});
