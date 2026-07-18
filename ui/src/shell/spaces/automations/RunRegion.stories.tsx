import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { RunRegion } from "./RunRegion";
import { WorkflowCard } from "./WorkflowCard";
import { SuggestionCard } from "./SuggestionCard";
import { PreparedInputPreview } from "./PreparedInputPreview";
import { RunProgress } from "./RunProgress";
import { EvidenceViewer } from "./EvidenceViewer";
import { automationStore, approvalStore } from "../../../stores";
import type { Workflow } from "../../../stores";

/**
 * Automations · Run segment (task 7.2, Req 6.3/6.5). Components read the global
 * automationStore / approvalStore, so each story seeds them before rendering.
 * Wrapped in a fixed-height shell surface for the workbench.
 */
const meta = {
  title: "Spaces/Automations/Run",
  component: RunRegion,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="standard" style={{ height: "700px", padding: "24px", "overflow-y": "auto" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof RunRegion>;

export default meta;
type Story = StoryObj<typeof meta>;

function seed(): Workflow[] {
  const now = Date.now();
  return [
    { id: "w1", name: "Nightly database backup", description: "Snapshot the SQLite store and copy it off-disk.", status: "idle", lastRunAt: now - 3600_000, createdAt: now - 86400_000, version: "1" },
    { id: "w2", name: "Morning news digest", description: "Summarize overnight headlines into a briefing.", status: "running", lastRunAt: now - 120_000, createdAt: now - 172800_000, version: "3" },
  ];
}

function reset() {
  automationStore.setWorkflows([]);
  automationStore.setSearchQuery("");
  automationStore.setLoading(false);
  automationStore.clearRunState();
  approvalStore.setQueue([]);
  for (const id of automationStore.runningWorkflowIds()) automationStore.markWorkflowCompleted(id, true);
}

/** Cold Run — the ask bar + an honest empty workflow list. */
export const Empty: Story = {
  render: () => {
    reset();
    return <RunRegion />;
  },
};

/** Run with workflows surfaced at the top level, one live with progress. */
export const WithWorkflows: Story = {
  render: () => {
    reset();
    automationStore.setWorkflows(seed());
    automationStore.markWorkflowStarted("w2");
    automationStore.updateRunProgress({ workflowId: "w2", phase: "running", completedSteps: 1, totalSteps: 3, message: "Fetching headlines…", updatedAt: Date.now() });
    automationStore.setRunEvidenceFor("w2", [{ label: "Headlines fetched", detail: "Collected <b>42</b> items from 6 sources." }]);
    return <RunRegion />;
  },
};

/** A workflow run awaiting a HITL decision — pointer to the Approval Center. */
export const AwaitingApproval: Story = {
  render: () => {
    reset();
    automationStore.setWorkflows(seed());
    automationStore.markWorkflowStarted("w2");
    automationStore.updateRunProgress({ workflowId: "w2", phase: "waiting", completedSteps: 2, totalSteps: 3, message: "Waiting for your approval.", updatedAt: Date.now() });
    approvalStore.setQueue([
      { id: "ap1", type: "workflow-resume", title: "Approve send", description: "Send the morning digest email", risk: "yellow", routing: { workflowId: "w2" }, payload: null, createdAt: Date.now(), status: "pending" },
    ]);
    return <RunRegion />;
  },
};

// ── Individual components ──────────────────────────────────────────────────────

export const Suggestion: StoryObj = {
  render: () => (
    <SuggestionCard
      suggestion={{
        workflowId: "wf-digest",
        workflowVersion: "2",
        displayName: "Email digest",
        reason: "Matches summarizing your unread email into a briefing.",
        confidence: 0.91,
        confidenceLabel: "High confidence",
        riskTier: "green",
        requiresConfirmation: false,
        missingInputs: [],
      }}
      onPrepare={() => {}}
      onRun={() => {}}
    />
  ),
};

export const Prepared: StoryObj = {
  render: () => (
    <PreparedInputPreview
      prepared={{
        workflowId: "wf-digest",
        workflowVersion: "2",
        displayName: "Email digest",
        prompt: "summarize my unread email",
        payload: { hours: 24, label: "unread", maxItems: 20 },
        fields: [
          { name: "hours", type: "number", required: true, description: "Lookback window in hours" },
          { name: "label", type: "string", required: false, description: "Gmail label to filter on" },
        ],
        missingInputs: [],
        validationIssues: [],
        explanation: "KRIA derived a 24-hour window and the 'unread' label from your request.",
        inputMapped: true,
      }}
      onConfirm={() => {}}
      onCancel={() => {}}
    />
  ),
};

export const Progress: StoryObj = {
  render: () => (
    <RunProgress
      progress={{ workflowId: "w1", phase: "running", completedSteps: 2, totalSteps: 4, message: "Step 2 of 4 running…", updatedAt: Date.now() }}
    />
  ),
};

export const Evidence: StoryObj = {
  render: () => (
    <EvidenceViewer
      evidence={[
        { label: "Run output", detail: "Backup completed: <b>1.2 GB</b> written." },
        { label: "n8n execution", href: "https://example.test/exec/1" },
      ]}
    />
  ),
};

export const WorkflowCardIdle: StoryObj = {
  render: () => {
    reset();
    return <WorkflowCard workflow={{ id: "w1", name: "Nightly backup", description: "Back up the DB", status: "idle", lastRunAt: null, createdAt: Date.now(), version: "1" }} onRun={() => {}} />;
  },
};
