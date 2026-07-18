import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@solidjs/testing-library";
import { QuarantineReview } from "./QuarantineReview";
import { modalHost, closeModal } from "../../modalHost";
import type { QuarantineToolView } from "../../../stores";

function tool(over: Partial<QuarantineToolView> = {}): QuarantineToolView {
  return {
    id: "t1",
    name: "PDF summarizer",
    description: "Summarize a PDF",
    riskLevel: "yellow",
    status: "PendingApproval",
    source: "SkillCompiler",
    successCount: 3,
    consecutiveFailures: 1,
    totalExecutions: 4,
    createdAt: "2024-01-01",
    lastTested: "2024-01-02",
    reviewNotes: null,
    parametersSchema: null,
    ...over,
  };
}

const noop = () => {};

describe("QuarantineReview (task 8.4, Req 20.3/17.3/11.3/1.6)", () => {
  beforeEach(() => closeModal());
  afterEach(() => {
    closeModal();
    cleanup();
  });

  it("renders a pending tool with its name, source and risk as text (Req 17.3)", () => {
    render(() => (
      <QuarantineReview tools={[tool()]} loading={false} onApprove={noop} onReject={noop} onReload={noop} />
    ));
    expect(screen.getByText("PDF summarizer")).toBeInTheDocument();
    // Risk conveyed by text, never color alone (Req 17.3).
    expect(screen.getByText("Elevated")).toBeInTheDocument();
    expect(screen.getByText("Needs approval")).toBeInTheDocument();
  });

  it("exposes an accessible, labelled filter group (Req 17.2)", () => {
    render(() => (
      <QuarantineReview tools={[tool()]} loading={false} onApprove={noop} onReject={noop} onReload={noop} />
    ));
    expect(screen.getByRole("group", { name: "Filter quarantined tools" })).toBeInTheDocument();
  });

  it("shows an honest empty state when nothing awaits approval", () => {
    render(() => (
      <QuarantineReview tools={[]} loading={false} onApprove={noop} onReject={noop} onReload={noop} />
    ));
    expect(screen.getByRole("heading", { name: "Nothing awaiting approval" })).toBeInTheDocument();
  });

  it("approve opens a deliberate confirm and does NOT relay immediately (Req 11.3/1.6)", () => {
    let approved: string | null = null;
    render(() => (
      <QuarantineReview
        tools={[tool({ id: "abc" })]}
        loading={false}
        onApprove={(id) => {
          approved = id;
        }}
        onReject={noop}
        onReload={noop}
      />
    ));
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    // Nothing relayed — a deliberate confirm modal is staged instead.
    expect(approved).toBeNull();
    const modal = modalHost.activeModal();
    expect(modal).not.toBeNull();
    expect(modal!.id).toBe("quarantine-approve-abc");
  });

  it("reject opens a deliberate danger confirm and does NOT relay immediately (Req 11.3/1.6)", () => {
    let rejected: string | null = null;
    render(() => (
      <QuarantineReview
        tools={[tool({ id: "xyz" })]}
        loading={false}
        onApprove={noop}
        onReject={(id) => {
          rejected = id;
        }}
        onReload={noop}
      />
    ));
    fireEvent.click(screen.getByRole("button", { name: "Reject" }));
    expect(rejected).toBeNull();
    const modal = modalHost.activeModal();
    expect(modal).not.toBeNull();
    expect(modal!.id).toBe("quarantine-reject-xyz");
  });

  it("calls onReload when the reload control is pressed", () => {
    let reloaded = false;
    render(() => (
      <QuarantineReview
        tools={[tool()]}
        loading={false}
        onApprove={noop}
        onReject={noop}
        onReload={() => {
          reloaded = true;
        }}
      />
    ));
    fireEvent.click(screen.getByRole("button", { name: /Reload/ }));
    expect(reloaded).toBe(true);
  });
});
