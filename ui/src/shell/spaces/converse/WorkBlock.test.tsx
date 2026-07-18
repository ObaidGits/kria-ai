/**
 * WorkBlock component tests (task 3.3, Req 4.2/4.3/17.3).
 *
 * Covers: the 5 typed variants render status + summary; the details disclosure
 * is keyboard-operable (aria-expanded toggles) and reveals variant content;
 * evidence renders; the independent Stop shows ONLY while running and dispatches
 * the typed per-block cancel path with the block id; status is conveyed by
 * icon + text (not color alone).
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup } from "@solidjs/testing-library";
import { WorkBlock } from "./WorkBlock";
import { converseStore } from "../../../stores";
import { eventBus } from "../../../stores/eventBus";
import type {
  WorkBlock as WorkBlockData,
  WorkBlockType,
} from "../../../stores/converseStore";

function block(overrides: Partial<WorkBlockData> = {}): WorkBlockData {
  return {
    id: "wb-1",
    type: "reasoning",
    status: "completed",
    summary: "Reviewed the notes",
    startedAt: Date.now(),
    ...overrides,
  };
}

const VARIANTS: Record<WorkBlockType, Partial<WorkBlockData>> = {
  reasoning: { type: "reasoning", reasoning: "First I considered the notes." },
  "tool-call": {
    type: "tool-call",
    toolCall: { name: "search_memory", args: '{"q":"notes"}', result: "3 hits" },
  },
  "plan-compare": {
    type: "plan-compare",
    planOptions: [{ id: "p1", label: "Plan A", summary: "Fast", recommended: true }],
  },
  "gui-cognition": {
    type: "gui-cognition",
    guiSteps: [{ id: "s1", label: "Locate button", status: "completed" }],
  },
  "workflow-run": {
    type: "workflow-run",
    workflowRun: { progress: 0.5, completed: 1, total: 2, log: ["started"] },
  },
};

describe("WorkBlock — typed variants (Req 4.2)", () => {
  beforeEach(() => {
    cleanup();
    converseStore.clearWorkBlocks();
  });

  for (const [type, overrides] of Object.entries(VARIANTS) as [
    WorkBlockType,
    Partial<WorkBlockData>,
  ][]) {
    it(`renders the ${type} variant with status + summary`, () => {
      render(() => (
        <WorkBlock block={block({ ...overrides, summary: `${type} summary`, status: "running" })} />
      ));
      // Plain-language summary always visible (Req 4.2).
      expect(screen.getByText(`${type} summary`)).toBeInTheDocument();
      // Status text present (icon + text — Req 17.3).
      expect(screen.getByText("Running")).toBeInTheDocument();
      // Typed marker set for the WorkLane/streaming wiring.
      const group = screen.getByRole("group");
      expect(group).toHaveAttribute("data-work-type", type);
      expect(group).toHaveAttribute("data-work-status", "running");
    });
  }

  it("marks every work-block variant as a KRIA-authored action (Req 20.5)", () => {
    for (const [type, overrides] of Object.entries(VARIANTS) as [
      WorkBlockType,
      Partial<WorkBlockData>,
    ][]) {
      const { unmount } = render(() => <WorkBlock block={block({ ...overrides, type })} />);
      expect(screen.getByRole("group")).toHaveAttribute("data-provenance", "kria");
      expect(screen.getByLabelText("AI-authored by KRIA")).toHaveTextContent("KRIA action");
      unmount();
    }
  });

  it("conveys status by icon AND text, not color alone (Req 17.3)", () => {
    const { container } = render(() => <WorkBlock block={block({ status: "failed" })} />);
    // Text label present.
    expect(screen.getByText("Failed")).toBeInTheDocument();
    // Icon (svg) present within the status badge.
    expect(container.querySelector(".kria-work-block__status svg")).not.toBeNull();
  });
});

describe("WorkBlock — details disclosure (Req 17.1)", () => {
  beforeEach(() => cleanup());

  it("toggles aria-expanded and reveals variant content", async () => {
    render(() => (
      <WorkBlock
        block={block({
          type: "tool-call",
          toolCall: { name: "search_memory", args: "{}", result: "ok" },
        })}
      />
    ));

    const disclosure = screen.getByRole("button", { name: /show details/i });
    expect(disclosure).toHaveAttribute("aria-expanded", "false");
    // Collapsed: details region not rendered.
    expect(document.querySelector('[data-region="work-details"]')).toBeNull();

    fireEvent.click(disclosure);

    const expanded = screen.getByRole("button", { name: /hide details/i });
    expect(expanded).toHaveAttribute("aria-expanded", "true");
    expect(document.querySelector('[data-region="work-details"]')).not.toBeNull();
    // Tool-call content revealed.
    expect(screen.getByText("search_memory")).toBeInTheDocument();
  });

  it("does not render a disclosure when there is no detail content", () => {
    render(() => <WorkBlock block={block({ summary: "bare", reasoning: undefined })} />);
    expect(screen.queryByRole("button", { name: /details/i })).toBeNull();
  });
});

describe("WorkBlock — evidence (Req 4.2)", () => {
  beforeEach(() => cleanup());

  it("renders the evidence section with its items", () => {
    render(() => (
      <WorkBlock
        block={block({
          evidence: [
            { id: "e1", label: "note-1.md" },
            { id: "e2", label: "source", href: "https://example.com" },
          ],
        })}
      />
    ));
    expect(screen.getByRole("region", { name: "Evidence" })).toBeInTheDocument();
    expect(screen.getByText("note-1.md")).toBeInTheDocument();
    const link = screen.getByRole("link", { name: "source" });
    expect(link).toHaveAttribute("href", "https://example.com");
  });
});

describe("WorkBlock — independent Stop (Req 4.2)", () => {
  beforeEach(() => cleanup());

  it("shows Stop ONLY while running", () => {
    const { unmount } = render(() => <WorkBlock block={block({ status: "running" })} />);
    expect(screen.getByRole("button", { name: /stop reasoning/i })).toBeInTheDocument();
    unmount();

    render(() => <WorkBlock block={block({ status: "completed" })} />);
    expect(screen.queryByRole("button", { name: /stop/i })).toBeNull();
  });

  it("dispatches the cancel path with the block id", () => {
    const onStop = vi.fn();
    render(() => <WorkBlock block={block({ id: "wb-42", status: "running" })} onStop={onStop} />);
    fireEvent.click(screen.getByRole("button", { name: /stop reasoning/i }));
    expect(onStop).toHaveBeenCalledWith("wb-42");
  });
});

describe("converseStore.cancelWorkBlock — typed per-block cancel (Req 4.2)", () => {
  beforeEach(() => {
    cleanup();
    converseStore.clearWorkBlocks();
  });

  it("emits a typed cancel request keyed by block id + type and flips to stopped", () => {
    converseStore.addWorkBlock(
      block({ id: "wb-run", type: "workflow-run", status: "running" }),
    );

    const seen: Array<{ blockId: string; blockType: string }> = [];
    const off = eventBus.on("converse:work-cancel-requested", (p) => seen.push(p));

    converseStore.cancelWorkBlock("wb-run");
    off();

    // Typed cancel request carries the block id + type (bridge routes it to the
    // matching existing cancellation command — cancellation propagation intact).
    expect(seen).toEqual([{ blockId: "wb-run", blockType: "workflow-run" }]);
    // Optimistic terminal status reflected immediately.
    const stopped = converseStore.workBlocks().find((b) => b.id === "wb-run");
    expect(stopped?.status).toBe("stopped");
  });

  it("is a no-op when the block is not running", () => {
    converseStore.addWorkBlock(block({ id: "wb-done", status: "completed" }));
    let emitted = false;
    const off = eventBus.on("converse:work-cancel-requested", () => (emitted = true));
    converseStore.cancelWorkBlock("wb-done");
    off();
    expect(emitted).toBe(false);
  });
});
