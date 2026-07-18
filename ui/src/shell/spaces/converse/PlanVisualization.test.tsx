/**
 * PlanVisualization tests (task 3.7, Req 20.3 revive; Req 4.2/17.3).
 *
 * Covers: the revived view renders candidate plans with steps / tradeoffs /
 * recommended; it mounts inside the WorkBlock's plan-visualization-slot; risk
 * and step status are conveyed by icon + text (not color alone); plan-select
 * routes through the typed converse/approval REQUEST path (NOT a tool call);
 * and untrusted model text is sanitized before it reaches the DOM.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup } from "@solidjs/testing-library";
import { PlanVisualization } from "./PlanVisualization";
import { WorkBlock } from "./WorkBlock";
import { converseStore } from "../../../stores";
import { eventBus } from "../../../stores/eventBus";
import type { WorkBlock as WorkBlockData } from "../../../stores/converseStore";

function planBlock(overrides: Partial<WorkBlockData> = {}): WorkBlockData {
  return {
    id: "wb-plan",
    type: "plan-compare",
    status: "pending",
    summary: "Comparing two ways to complete the task",
    startedAt: Date.now(),
    planSelectionReason: "Plan A balances speed and safety.",
    planOptions: [
      {
        id: "a",
        label: "Plan A — direct",
        summary: "Fewer steps, higher risk.",
        recommended: true,
        risk: "aggressive",
        score: 0.82,
        confidence: 0.7,
        tradeoffs: "Fast but potentially irreversible.",
        steps: [
          { label: "Stop the service", status: "completed", detail: "systemctl stop app", outcome: "exit 0 · 120ms" },
          { label: "Apply the fix", status: "running" },
        ],
      },
      {
        id: "b",
        label: "Plan B — careful",
        summary: "More steps, safer.",
        risk: "safe",
        score: 0.6,
        confidence: 0.9,
        steps: [{ label: "Diagnose logs", status: "pending" }],
      },
    ],
    ...overrides,
  };
}

describe("PlanVisualization — revived plan comparison (Req 20.3)", () => {
  beforeEach(() => {
    cleanup();
    converseStore.clearWorkBlocks();
  });

  it("renders candidate plans with steps, tradeoffs, and the recommended one", () => {
    render(() => <PlanVisualization block={planBlock()} onSelect={() => {}} />);

    // Both candidate plans present.
    expect(screen.getByText("Plan A — direct")).toBeInTheDocument();
    expect(screen.getByText("Plan B — careful")).toBeInTheDocument();
    // Recommended marker on the recommended plan.
    expect(screen.getByText("Recommended")).toBeInTheDocument();
    // Tradeoffs surfaced.
    expect(screen.getByText(/potentially irreversible/i)).toBeInTheDocument();
    // Steps surfaced.
    expect(screen.getByText("Stop the service")).toBeInTheDocument();
    expect(screen.getByText("Apply the fix")).toBeInTheDocument();
    // Selection reason surfaced.
    expect(screen.getByText(/balances speed and safety/i)).toBeInTheDocument();
  });

  it("conveys risk and step status by icon AND text, not color alone (Req 17.3)", () => {
    const { container } = render(() => (
      <PlanVisualization block={planBlock()} onSelect={() => {}} />
    ));
    // Risk label text present (icon + text).
    expect(screen.getByText("Aggressive")).toBeInTheDocument();
    expect(screen.getByText("Diagnose first")).toBeInTheDocument();
    // Step status text present.
    expect(screen.getAllByText("Done").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Running").length).toBeGreaterThan(0);
    // Icons (svg) rendered alongside.
    expect(container.querySelector(".kria-plan-viz__risk svg")).not.toBeNull();
  });

  it("shows an honest empty state when there are no plan options", () => {
    render(() => <PlanVisualization block={planBlock({ planOptions: [] })} onSelect={() => {}} />);
    expect(screen.getByText("No plan options yet.")).toBeInTheDocument();
  });

  it("renders a goal-verification outcome when present", () => {
    render(() => (
      <PlanVisualization
        block={planBlock({ planOutcome: { outcome: "achieved", reason: "All checks passed." } })}
        onSelect={() => {}}
      />
    ));
    expect(screen.getByText("Goal achieved")).toBeInTheDocument();
    expect(screen.getByText(/all checks passed/i)).toBeInTheDocument();
  });
});

describe("PlanVisualization — plan-select routing (KRIA runtime-authority invariant)", () => {
  beforeEach(() => {
    cleanup();
    converseStore.clearWorkBlocks();
  });

  it("select calls the provided handler with the block + option id (never a tool call)", () => {
    const onSelect = vi.fn();
    render(() => <PlanVisualization block={planBlock()} onSelect={onSelect} />);
    fireEvent.click(screen.getByRole("button", { name: /use plan: plan a — direct/i }));
    expect(onSelect).toHaveBeenCalledWith("wb-plan", "a");
  });

  it("default select stages a typed converse:plan-selected REQUEST on the bus", () => {
    const block = planBlock();
    converseStore.addWorkBlock(block);

    const seen: Array<{ blockId: string; optionId: string }> = [];
    const off = eventBus.on("converse:plan-selected", (p) => seen.push(p));

    // No onSelect → uses converseStore.selectPlanOption (the request path).
    render(() => <PlanVisualization block={block} />);
    fireEvent.click(screen.getByRole("button", { name: /use plan: plan b — careful/i }));
    off();

    // A staged request keyed by block + option — routed by the bridge to the
    // existing approve/converse path, NOT a direct tool invocation.
    expect(seen).toEqual([{ blockId: "wb-plan", optionId: "b" }]);
  });

  it("selectPlanOption is a no-op for a missing option or non-plan block", () => {
    converseStore.addWorkBlock(planBlock());
    let emitted = 0;
    const off = eventBus.on("converse:plan-selected", () => (emitted += 1));
    converseStore.selectPlanOption("wb-plan", "does-not-exist");
    converseStore.selectPlanOption("nope", "a");
    off();
    expect(emitted).toBe(0);
  });
});

describe("PlanVisualization — mounts in the WorkBlock slot + sanitization (Req 20.3, security)", () => {
  beforeEach(() => cleanup());

  it("renders inside the plan-visualization-slot when details are opened", () => {
    render(() => <WorkBlock block={planBlock()} />);
    fireEvent.click(screen.getByRole("button", { name: /show details/i }));

    const slot = document.querySelector('[data-region="plan-visualization-slot"]');
    expect(slot).not.toBeNull();
    // The revived view is mounted within the slot.
    expect(slot!.querySelector('[data-region="plan-visualization"]')).not.toBeNull();
    expect(screen.getByText("Plan A — direct")).toBeInTheDocument();
  });

  it("sanitizes untrusted model text — no script survives (design.md §1.17)", () => {
    render(() => (
      <PlanVisualization
        block={planBlock({
          planSelectionReason: "safe<script>window.__pwned=1</script>",
          planOptions: [
            {
              id: "x",
              label: "Plan X",
              summary: "<img src=x onerror=\"window.__pwned=1\">shown",
              tradeoffs: "<script>window.__pwned=1</script>fine",
            },
          ],
        })}
        onSelect={() => {}}
      />
    ));
    // No script element reaches the DOM; the onerror attribute is stripped.
    expect(document.querySelector("script")).toBeNull();
    expect(document.querySelector("img[onerror]")).toBeNull();
    // Benign text still renders.
    expect(screen.getByText(/safe/)).toBeInTheDocument();
  });
});
