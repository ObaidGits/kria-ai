import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { For } from "solid-js";
import { WorkBlock } from "./WorkBlock";
import type {
  WorkBlock as WorkBlockData,
  WorkBlockStatus,
} from "../../../stores/converseStore";

/**
 * WorkBlock — the 5 typed variants (reasoning/tool/plan/gui/run) and every
 * status (pending/running/completed/failed/stopped). Wrapped in a narrow muted
 * column that mimics the WorkLane so the secondary type scale reads correctly.
 */
const meta = {
  title: "Converse/WorkBlock",
  component: WorkBlock,
  decorators: [
    (Story: () => unknown) => (
      <div
        class="kria-shell"
        data-window-mode="standard"
        style={{ width: "360px", padding: "16px", display: "flex", "flex-direction": "column", gap: "12px" }}
      >
        {Story() as never}
      </div>
    ),
  ],
  // Default arg satisfies the required `block` prop; each story below supplies
  // its own block via a custom `render`.
  args: {
    block: {
      id: "default",
      type: "reasoning",
      status: "completed",
      summary: "A work block",
      startedAt: Date.now(),
    },
  },
} satisfies Meta<typeof WorkBlock>;

export default meta;
type Story = StoryObj<typeof meta>;

const now = Date.now();

/** reasoning — a running reasoning step with a trace + evidence. */
export const Reasoning: Story = {
  render: () => (
    <WorkBlock
      block={{
        id: "r1",
        type: "reasoning",
        status: "running",
        summary: "Deciding how to summarize the notes",
        reasoning: "First I group the notes by topic, then extract the key points from each group.",
        evidence: [{ id: "e1", label: "note-1.md" }, { id: "e2", label: "note-2.md" }],
        startedAt: now,
      }}
    />
  ),
};

/** tool-call — a completed tool invocation with args + result. */
export const ToolCall: Story = {
  render: () => (
    <WorkBlock
      block={{
        id: "t1",
        type: "tool-call",
        status: "completed",
        summary: "Searched memory for today's notes",
        toolCall: {
          name: "search_memory",
          args: '{\n  "query": "today notes",\n  "limit": 5\n}',
          result: "Found 3 relevant memories.",
        },
        evidence: [{ id: "s1", label: "memory://notes", href: "#" }],
        startedAt: now,
      }}
    />
  ),
};

/** plan-compare — the shell PlanVisualization (task 3.7) slots into. */
export const PlanCompare: Story = {
  render: () => (
    <WorkBlock
      block={{
        id: "p1",
        type: "plan-compare",
        status: "pending",
        summary: "Comparing two ways to complete the task",
        planOptions: [
          { id: "a", label: "Plan A — direct", summary: "Fewer steps, higher risk.", recommended: true },
          { id: "b", label: "Plan B — careful", summary: "More steps, safer." },
        ],
        startedAt: now,
      }}
    />
  ),
};

/** gui-cognition — observed/acted steps with per-step status. */
export const GuiCognition: Story = {
  render: () => (
    <WorkBlock
      block={{
        id: "g1",
        type: "gui-cognition",
        status: "running",
        summary: "Working through the settings screen",
        guiSteps: [
          { id: "s1", label: "Locate the Settings menu", status: "completed" },
          { id: "s2", label: "Open Privacy tab", status: "running" },
          { id: "s3", label: "Toggle telemetry off", status: "pending" },
        ],
        startedAt: now,
      }}
    />
  ),
};

/** workflow-run — progress + run log. */
export const WorkflowRun: Story = {
  render: () => (
    <WorkBlock
      block={{
        id: "w1",
        type: "workflow-run",
        status: "running",
        summary: "Running the daily digest workflow",
        workflowRun: {
          progress: 0.5,
          completed: 2,
          total: 4,
          log: ["Fetched inbox", "Summarized 12 emails"],
        },
        startedAt: now,
      }}
    />
  ),
};

/** All statuses on one variant to show the status vocabulary (icon + text). */
export const AllStatuses: Story = {
  render: () => {
    const statuses: WorkBlockStatus[] = ["pending", "running", "completed", "failed", "stopped"];
    const blocks: WorkBlockData[] = statuses.map((status, i) => ({
      id: `st-${i}`,
      type: "tool-call",
      status,
      summary: `Tool call — ${status}`,
      toolCall: { name: "do_thing", result: "…" },
      startedAt: now,
    }));
    return <For each={blocks}>{(b) => <WorkBlock block={b} />}</For>;
  },
};
