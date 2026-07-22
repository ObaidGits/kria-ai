/**
 * PermissionSurface — component tests (design.md §10.4, Requirement 10).
 *
 * Exercises the presence Permission UX: GREEN report+undo, YELLOW intent+halt
 * window, RED single-line allow/deny, the no-modal-on-modal deferral, and the
 * accessibility contract. Uses injected subject/overlay/handlers so the tests
 * are deterministic and decoupled from the live approval queue.
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import { render, cleanup, fireEvent } from "@solidjs/testing-library";

import PermissionSurface from "./PermissionSurface";
import { resolvePermissionMode, type OverlayState, type PermissionSubject } from "./permissionUx";
import type { RiskLevel } from "../../../stores/approvalStore";

afterEach(cleanup);

const CLOSED: OverlayState = { approvalCenterOpen: false, modalOpen: false };

function subject(over: Partial<PermissionSubject> = {}): PermissionSubject {
  const risk = over.risk ?? "red";
  return {
    requestId: over.requestId ?? "req-1",
    risk,
    mode: over.mode ?? resolvePermissionMode(risk),
    what: over.what ?? "Delete build output",
    why: over.why ?? "you asked me to clean up",
    reversible: over.reversible ?? true,
    createdAt: over.createdAt ?? 1_000,
  };
}

describe("PermissionSurface — GREEN report (Req 10.1)", () => {
  it("reports via a polite live region and offers Undo when reversible", () => {
    const onUndo = vi.fn();
    const { container, getByText } = render(() => (
      <PermissionSurface
        subject={() => subject({ risk: "green", reversible: true, what: "Archived 3 notes" })}
        overlay={() => CLOSED}
        blockedContext={() => false}
        onUndo={onUndo}
      />
    ));
    const region = container.querySelector('[data-region="permission-surface"]')!;
    expect(region.getAttribute("data-mode")).toBe("report");
    expect(getByText("Archived 3 notes")).toBeInTheDocument();

    const report = container.querySelector(".kria-permission__report")!;
    expect(report.getAttribute("role")).toBe("status");
    expect(report.getAttribute("aria-live")).toBe("polite");

    const undo = container.querySelector('[data-role="undo"]') as HTMLButtonElement;
    expect(undo).toBeInTheDocument();
    fireEvent.click(undo);
    expect(onUndo).toHaveBeenCalledWith("req-1");
  });

  it("omits Undo for an irreversible GREEN report (no blocking prompt either)", () => {
    const { container } = render(() => (
      <PermissionSurface
        subject={() => subject({ risk: "green", reversible: false })}
        overlay={() => CLOSED}
        blockedContext={() => false}
      />
    ));
    expect(container.querySelector('[data-role="undo"]')).not.toBeInTheDocument();
    // A report is never a blocking allow/deny.
    expect(container.querySelector('[data-role="allow"]')).not.toBeInTheDocument();
    expect(container.querySelector('[data-role="deny"]')).not.toBeInTheDocument();
  });
});

describe("PermissionSurface — YELLOW intent + halt window (Req 10.2)", () => {
  it("narrates intent with a Stop control and the halt window", () => {
    const onHalt = vi.fn();
    const { container, getByText } = render(() => (
      <PermissionSurface
        subject={() => subject({ risk: "yellow", what: "Sending the email" })}
        overlay={() => CLOSED}
        blockedContext={() => false}
        onHalt={onHalt}
        onProceed={vi.fn()}
      />
    ));
    const intent = container.querySelector(".kria-permission__intent")!;
    expect(intent.getAttribute("data-halt-window-ms")).toBe("4000");
    expect(getByText("Sending the email")).toBeInTheDocument();

    const stop = container.querySelector('[data-role="halt"]') as HTMLButtonElement;
    fireEvent.click(stop);
    expect(onHalt).toHaveBeenCalledWith("req-1");
  });

  it("proceeds after the halt window when NOT stopped", () => {
    vi.useFakeTimers();
    const onProceed = vi.fn();
    const onHalt = vi.fn();
    render(() => (
      <PermissionSurface
        subject={() => subject({ risk: "yellow", requestId: "y1" })}
        overlay={() => CLOSED}
        blockedContext={() => false}
        onProceed={onProceed}
        onHalt={onHalt}
      />
    ));
    expect(onProceed).not.toHaveBeenCalled();
    vi.advanceTimersByTime(4000);
    expect(onProceed).toHaveBeenCalledWith("y1");
    expect(onHalt).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it("does NOT proceed after the window when the user pressed Stop", () => {
    vi.useFakeTimers();
    const onProceed = vi.fn();
    const onHalt = vi.fn();
    const { container } = render(() => (
      <PermissionSurface
        subject={() => subject({ risk: "yellow", requestId: "y2" })}
        overlay={() => CLOSED}
        blockedContext={() => false}
        onProceed={onProceed}
        onHalt={onHalt}
      />
    ));
    fireEvent.click(container.querySelector('[data-role="halt"]') as HTMLButtonElement);
    vi.advanceTimersByTime(4000);
    expect(onHalt).toHaveBeenCalledWith("y2");
    expect(onProceed).not.toHaveBeenCalled();
    vi.useRealTimers();
  });
});

describe("PermissionSurface — RED decision (Req 10.3/10.4)", () => {
  function renderRed(over: Partial<PermissionSubject> = {}, handlers: Record<string, unknown> = {}) {
    return render(() => (
      <PermissionSurface
        subject={() => subject({ risk: "red", ...over })}
        overlay={() => CLOSED}
        blockedContext={() => false}
        {...handlers}
      />
    ));
  }

  it("presents a single-line Allow/Deny with what/why visible", () => {
    const { container, getByText } = renderRed({ what: "Wipe the disk", why: "requested by you" });
    const region = container.querySelector('[data-region="permission-surface"]')!;
    expect(region.getAttribute("data-mode")).toBe("decision");
    expect(getByText("Wipe the disk")).toBeInTheDocument();
    expect(getByText("requested by you")).toBeInTheDocument();
    expect(container.querySelector('[data-role="allow"]')).toBeInTheDocument();
    expect(container.querySelector('[data-role="deny"]')).toBeInTheDocument();
  });

  it("routes Allow/Deny back through the provided handlers (reuse approvalStore)", () => {
    const onAllow = vi.fn();
    const onDeny = vi.fn();
    const { container } = renderRed({ requestId: "r7" }, { onAllow, onDeny });
    fireEvent.click(container.querySelector('[data-role="allow"]') as HTMLButtonElement);
    expect(onAllow).toHaveBeenCalledWith("r7");
    fireEvent.click(container.querySelector('[data-role="deny"]') as HTMLButtonElement);
    expect(onDeny).toHaveBeenCalledWith("r7");
  });

  it("routes detail to the Approval Center (owner of decision detail, Req 10.4)", () => {
    const onReviewDetail = vi.fn();
    const { container } = renderRed({ requestId: "r8" }, { onReviewDetail });
    fireEvent.click(container.querySelector('[data-role="review"]') as HTMLButtonElement);
    expect(onReviewDetail).toHaveBeenCalledWith("r8");
  });

  it("is a labelled group whose body is a polite live region (Req 21)", () => {
    const { container } = renderRed();
    const group = container.querySelector(".kria-permission__decision")!;
    expect(group.getAttribute("role")).toBe("group");
    expect(group.getAttribute("aria-label")).toBe("Permission required");
    const body = container.querySelector(".kria-permission__body")!;
    expect(body.getAttribute("aria-live")).toBe("polite");
  });

  it("marks the calm posture in an interruptibility-blocked context (Req 26.3)", () => {
    const { container } = render(() => (
      <PermissionSurface
        subject={() => subject({ risk: "red" })}
        overlay={() => CLOSED}
        blockedContext={() => true}
      />
    ));
    const region = container.querySelector('[data-region="permission-surface"]')!;
    expect(region.getAttribute("data-blocked-context")).toBe("true");
  });
});

describe("PermissionSurface — no modal-on-modal + resting calm (Req 10.3)", () => {
  it("renders NOTHING at rest (no subject)", () => {
    const { container } = render(() => (
      <PermissionSurface subject={() => undefined} overlay={() => CLOSED} blockedContext={() => false} />
    ));
    expect(container.querySelector('[data-region="permission-surface"]')).not.toBeInTheDocument();
  });

  it("DEFERS (renders nothing) while the Approval Center overlay is open", () => {
    const { container } = render(() => (
      <PermissionSurface
        subject={() => subject({ risk: "red" })}
        overlay={() => ({ approvalCenterOpen: true, modalOpen: false })}
        blockedContext={() => false}
      />
    ));
    expect(container.querySelector('[data-region="permission-surface"]')).not.toBeInTheDocument();
  });

  it("DEFERS (renders nothing) while a ModalHost modal is open", () => {
    const { container } = render(() => (
      <PermissionSurface
        subject={() => subject({ risk: "red" })}
        overlay={() => ({ approvalCenterOpen: false, modalOpen: true })}
        blockedContext={() => false}
      />
    ));
    expect(container.querySelector('[data-region="permission-surface"]')).not.toBeInTheDocument();
  });

  it("renders nothing when reading the view throws (failure isolation, design §14)", () => {
    const { container } = render(() => (
      <PermissionSurface
        subject={() => {
          throw new Error("boom");
        }}
        overlay={() => CLOSED}
        blockedContext={() => false}
      />
    ));
    expect(container.querySelector('[data-region="permission-surface"]')).not.toBeInTheDocument();
  });
});

describe("PermissionSurface — reduced motion (Req 17.4/21.4)", () => {
  it("marks static rendering when reducedMotion is forced", () => {
    const { container } = render(() => (
      <PermissionSurface
        subject={() => subject({ risk: "green" })}
        overlay={() => CLOSED}
        blockedContext={() => false}
        reducedMotion
      />
    ));
    const region = container.querySelector('[data-region="permission-surface"]')!;
    expect(region.getAttribute("data-motion")).toBe("static");
  });
});
