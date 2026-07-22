/**
 * CurrentWorkSummary — cross-Space presentation contract (task 5.3, UIE-H-010,
 * Req 8.1–8.3, 8.6, 9.5).
 *
 * Pins that the indicator:
 *   • renders a concise, source-truthful work fact from the read-only projection;
 *   • deep-links to the REAL owner (the Converse Work lane) via existing
 *     navigation, and only routes — it never mutates runtime/approval state;
 *   • shows a purposeful idle state with no fabricated narration when idle.
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@solidjs/testing-library";
import CurrentWorkSummary from "./CurrentWorkSummary";
import { currentRoute, setCurrentRoute } from "./router";
import { coreStore } from "../stores/coreStore";
import { converseStore, type WorkBlock } from "../stores/converseStore";
import { approvalStore } from "../stores/approvalStore";
import { capabilityStore } from "../stores/capabilityStore";
import { clearGuiCognitionSession } from "../stores/guiCognitionSession";

function block(overrides: Partial<WorkBlock> = {}): WorkBlock {
  return {
    id: overrides.id ?? "wb-1",
    type: overrides.type ?? "tool-call",
    status: overrides.status ?? "running",
    summary: overrides.summary ?? "Running search",
    startedAt: overrides.startedAt ?? 1000,
    ...overrides,
  };
}

function resetAll(): void {
  cleanup();
  coreStore.reset();
  converseStore.clearWorkBlocks();
  converseStore.setContextRailItems([]);
  approvalStore.setQueue([]);
  capabilityStore.setActiveLlmRuntime(null);
  clearGuiCognitionSession();
  // Start away from Converse so a deep-link to the owner is observable.
  setCurrentRoute({ space: "memory" });
}

beforeEach(resetAll);
afterEach(resetAll);

describe("CurrentWorkSummary — concise active work (Req 8.1/8.3)", () => {
  it("renders the source-owned label of active work", () => {
    converseStore.addWorkBlock(block({ id: "a", status: "running", summary: "Indexing files" }));
    render(() => <CurrentWorkSummary />);

    const link = screen.getByRole("button", { name: /Current work: Indexing files/i });
    expect(link).toBeInTheDocument();
    expect(screen.getByText("Indexing files")).toBeInTheDocument();
  });

  it("falls back to a source-owned kind noun when the block has no label", () => {
    converseStore.addWorkBlock(block({ id: "a", status: "running", type: "reasoning", summary: "   " }));
    render(() => <CurrentWorkSummary />);

    expect(screen.getByText("Reasoning")).toBeInTheDocument();
  });

  it("summarizes multiple active items concisely with a +N count", () => {
    converseStore.addWorkBlock(block({ id: "a", status: "running", summary: "First" }));
    converseStore.addWorkBlock(block({ id: "b", status: "pending", summary: "Second" }));
    render(() => <CurrentWorkSummary />);

    expect(screen.getByText("First +1")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /2 active work items/i }),
    ).toBeInTheDocument();
  });

  it("presents a failed block as resumable work", () => {
    converseStore.addWorkBlock(block({ id: "a", status: "failed", summary: "Broken step" }));
    render(() => <CurrentWorkSummary />);

    const link = screen.getByRole("button", { name: /resumable work/i });
    expect(link).toBeInTheDocument();
    expect(link).toHaveAttribute("data-work-state", "resumable");
  });
});

describe("CurrentWorkSummary — deep-links to the real owner (read-only)", () => {
  it("routes to the Converse Work lane when activated", () => {
    converseStore.addWorkBlock(block({ id: "a", status: "running", summary: "Live work" }));
    render(() => <CurrentWorkSummary />);

    expect(currentRoute().space).toBe("memory");
    fireEvent.click(screen.getByRole("button", { name: /Current work/i }));
    expect(currentRoute().space).toBe("converse");
  });

  it("does NOT mutate runtime or approval state when activated (design §20.1)", () => {
    approvalStore.setQueue([]);
    converseStore.addWorkBlock(block({ id: "a", status: "running", summary: "Live work" }));

    const workBefore = converseStore.workBlocks();
    const statusBefore = workBefore[0].status;
    const pendingBefore = approvalStore.pendingCount();
    const coreBefore = coreStore.state();

    render(() => <CurrentWorkSummary />);
    fireEvent.click(screen.getByRole("button", { name: /Current work/i }));

    // Routing only — the work block, its status, approvals, and Core state are
    // all untouched (no send/approve/cancel/stop).
    expect(converseStore.workBlocks()).toHaveLength(workBefore.length);
    expect(converseStore.workBlocks()[0].status).toBe(statusBefore);
    expect(approvalStore.pendingCount()).toBe(pendingBefore);
    expect(coreStore.state()).toBe(coreBefore);
  });
});

describe("CurrentWorkSummary — purposeful idle state (Req 8.2/9.5)", () => {
  it("shows a truthful idle cue with no fabricated work content", () => {
    render(() => <CurrentWorkSummary />);

    const idle = screen.getByLabelText("No active work");
    expect(idle).toBeInTheDocument();
    expect(idle).toHaveAttribute("data-work-state", "idle");
    expect(screen.getByText("Idle")).toBeInTheDocument();
  });

  it("offers no owner link (nothing to route to) when idle", () => {
    render(() => <CurrentWorkSummary />);

    expect(screen.queryByRole("button", { name: /Current work/i })).toBeNull();
  });

  it("does not treat ambient approvals/error alone as active work", () => {
    // Pending approvals exist but there is no active/resumable WORK: this
    // indicator stays silent (approvals own their PresenceBar/StatusLine home),
    // and it must not fabricate a work item.
    coreStore.setState("thinking");
    render(() => <CurrentWorkSummary />);

    expect(screen.queryByRole("button", { name: /Current work/i })).toBeNull();
    expect(screen.queryByText("Idle")).toBeNull();
  });
});

describe("CurrentWorkSummary — bounded long labels (task 10.7, UIE-H-002)", () => {
  const LONG =
    "Indexing an extraordinarily long source-owned work summary that would otherwise expand the PresenceBar and force horizontal overflow";

  it("bounds a long work label with the shared bounded-text class and offers the full value via title", () => {
    converseStore.addWorkBlock(block({ id: "a", status: "running", summary: LONG }));
    render(() => <CurrentWorkSummary />);

    const label = document.querySelector<HTMLElement>(".kria-work-summary__label")!;
    // Shared bounded-text utility applied (task 10.7 consolidation).
    expect(label).toHaveClass("kria-bounded");
    // Full value stays in the DOM (AT) and is recoverable on hover (title).
    expect(label.textContent).toBe(LONG);
    expect(label).toHaveAttribute("title", LONG);
    // The accessible name also carries the full label for AT.
    expect(
      screen.getByRole("button", { name: new RegExp(`Current work: ${LONG.slice(0, 24)}`, "i") }),
    ).toBeInTheDocument();
  });
});
