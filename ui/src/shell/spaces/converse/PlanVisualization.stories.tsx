import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { PlanVisualization } from "./PlanVisualization";
import type { WorkBlock as WorkBlockData } from "../../../stores/converseStore";

/**
 * PlanVisualization — the revived plan-compare view (Req 20.3). Candidate plans
 * side-by-side with risk (icon + text), model score/confidence, tradeoffs,
 * ordered steps, and the recommended option. Wrapped in a muted WorkLane-like
 * column so the secondary type scale reads correctly. `onSelect` is stubbed so
 * the stories never dispatch a real request.
 */
const meta = {
  title: "Converse/PlanVisualization",
  component: PlanVisualization,
  decorators: [
    (Story: () => unknown) => (
      <div
        class="kria-shell"
        data-window-mode="standard"
        style={{ width: "560px", padding: "16px" }}
      >
        {Story() as never}
      </div>
    ),
  ],
  args: {
    onSelect: () => {},
    block: {
      id: "default",
      type: "plan-compare",
      status: "pending",
      summary: "Comparing plans",
      startedAt: Date.now(),
    } satisfies WorkBlockData,
  },
} satisfies Meta<typeof PlanVisualization>;

export default meta;
type Story = StoryObj<typeof meta>;

const now = Date.now();

/** Three structured paths (Diagnose / Minimal-risk / Aggressive), one recommended. */
export const ThreePaths: Story = {
  render: () => (
    <PlanVisualization
      onSelect={() => {}}
      block={{
        id: "p1",
        type: "plan-compare",
        status: "pending",
        summary: "Comparing three ways to resolve the incident",
        startedAt: now,
        planSelectionReason: "Diagnose-first is recommended: read-only, lowest blast radius.",
        planOptions: [
          {
            id: "diagnose",
            label: "Path A — Diagnose first",
            summary: "Gather evidence before changing anything.",
            recommended: true,
            risk: "safe",
            score: 0.86,
            confidence: 0.78,
            tradeoffs: "Slower to resolve, but fully reversible and safe.",
            steps: [
              { label: "Read service logs", detail: "journalctl -u app --since -10m" },
              { label: "Check disk + memory", detail: "df -h && free -m" },
            ],
          },
          {
            id: "minimal",
            label: "Path B — Minimal risk",
            summary: "Restart the service, keep changes reversible.",
            risk: "moderate",
            score: 0.64,
            confidence: 0.71,
            tradeoffs: "Likely fixes transient faults; reversible.",
            steps: [
              { label: "Restart the service", detail: "systemctl restart app" },
              { label: "Verify health", detail: "curl -f localhost/health" },
            ],
          },
          {
            id: "aggressive",
            label: "Path C — Aggressive",
            summary: "Reprovision from a clean image.",
            risk: "aggressive",
            score: 0.41,
            confidence: 0.55,
            tradeoffs: "Fast resolution but potentially irreversible data loss.",
            steps: [{ label: "Reprovision node", detail: "kria provision --wipe" }],
          },
        ],
      }}
    />
  ),
};

/** Live execution — the recommended plan's steps stream status. */
export const Executing: Story = {
  render: () => (
    <PlanVisualization
      onSelect={() => {}}
      block={{
        id: "p2",
        type: "plan-compare",
        status: "running",
        summary: "Executing the chosen plan",
        startedAt: now,
        planOutcome: { outcome: "continue", reason: "Two of three steps complete." },
        planOptions: [
          {
            id: "chosen",
            label: "Diagnose first",
            recommended: true,
            risk: "safe",
            score: 0.86,
            steps: [
              { label: "Read logs", status: "completed", outcome: "exit 0 · 90ms" },
              { label: "Check resources", status: "completed", outcome: "exit 0 · 60ms" },
              { label: "Summarize findings", status: "running" },
            ],
          },
        ],
      }}
    />
  ),
};

/** Empty — no plans generated yet. */
export const Empty: Story = {
  render: () => (
    <PlanVisualization
      onSelect={() => {}}
      block={{
        id: "p3",
        type: "plan-compare",
        status: "pending",
        summary: "Waiting for the planner",
        startedAt: now,
        planOptions: [],
      }}
    />
  ),
};
