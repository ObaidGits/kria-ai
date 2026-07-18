import type { Meta, StoryObj } from "storybook-solidjs-vite";
import AutomationsSpace from "./AutomationsSpace";
import { automationStore } from "../../stores";
import type { Workflow, ScheduledTask, TaskItem, Reminder } from "../../stores";
import { navigate } from "../router";

/**
 * Automations Space — segments + top-level workflow surfacing (task 7.1,
 * Req 6.1/6.2). The Space reads the global automationStore + router, so each
 * story seeds them before rendering. Wrapped in a fixed-height `.kria-shell` so
 * the tablist + region scroll read correctly in the workbench.
 */
const meta = {
  title: "Spaces/AutomationsSpace",
  component: AutomationsSpace,
  decorators: [
    (Story: () => unknown) => (
      <div
        class="kria-shell"
        data-window-mode="standard"
        style={{ height: "600px", padding: "24px" }}
      >
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof AutomationsSpace>;

export default meta;
type Story = StoryObj<typeof meta>;

function seedWorkflows(): Workflow[] {
  const now = Date.now();
  return [
    {
      id: "w1",
      name: "Nightly database backup",
      description: "Snapshot the SQLite store and copy it off-disk.",
      status: "completed",
      lastRunAt: now - 3600_000,
      createdAt: now - 86400_000,
    },
    {
      id: "w2",
      name: "Morning news digest",
      description: "Summarize overnight headlines into a briefing.",
      status: "running",
      lastRunAt: now - 120_000,
      createdAt: now - 172800_000,
    },
    {
      id: "w3",
      name: "Inbox triage",
      description: "Classify and label incoming messages.",
      status: "failed",
      lastRunAt: now - 7200_000,
      createdAt: now - 259200_000,
    },
    {
      id: "w4",
      name: "Weekly report",
      description: "Compile the weekly activity report.",
      status: "idle",
      lastRunAt: null,
      createdAt: now - 604800_000,
    },
  ];
}

function seedScheduledTasks(): ScheduledTask[] {
  return [
    { id: "s1", name: "Morning briefing", intervalSecs: 86400, prompt: "Summarize my unread email and today's calendar", enabled: true },
    { id: "s2", name: "Hourly inbox scan", intervalSecs: 3600, prompt: "Check for urgent messages", enabled: false },
  ];
}

function seedTasks(): TaskItem[] {
  const now = Date.now();
  return [
    { id: 1, title: "Draft the release notes", notes: "Cover the Schedule merge", status: "open", priorityBucket: "high", priorityScore: 80, dueAt: now + 3600_000, source: "manual", createdAt: now - 86400_000 },
    { id: 2, title: "Review the UI redesign spec", notes: null, status: "done", priorityBucket: "normal", priorityScore: 30, dueAt: null, source: "manual", createdAt: now - 172800_000 },
  ];
}

function seedReminders(): Reminder[] {
  const now = Date.now();
  return [
    { id: 1, message: "Stand up and stretch", fireAt: now + 1800_000, fired: false, recurrence: "daily" },
    { id: 2, message: "Reply to the design thread", fireAt: now + 7200_000, fired: false, recurrence: null },
  ];
}

function reset() {
  automationStore.setWorkflows([]);
  automationStore.setScheduledTasks([]);
  automationStore.setTasks([]);
  automationStore.setReminders([]);
  automationStore.setSearchQuery("");
  automationStore.setLoading(false);
}

/** Cold Run segment — honest empty state, no workflows yet. */
export const EmptyRun: Story = {
  render: () => {
    reset();
    navigate("automations");
    return <AutomationsSpace />;
  },
};

/** Run segment surfacing workflows at the top level (Req 6.2). */
export const RunWithWorkflows: Story = {
  render: () => {
    reset();
    automationStore.setWorkflows(seedWorkflows());
    navigate("automations");
    return <AutomationsSpace />;
  },
};

/** Build segment — the 2D node canvas placeholder (task 7.3). */
export const BuildPlaceholder: Story = {
  render: () => {
    reset();
    navigate("automations", "build");
    return <AutomationsSpace />;
  },
};

/** Schedule segment merging scheduled tasks + routines + reminders + to-dos
 *  (Req 6.6). */
export const Schedule: Story = {
  render: () => {
    reset();
    automationStore.setScheduledTasks(seedScheduledTasks());
    automationStore.setTasks(seedTasks());
    automationStore.setReminders(seedReminders());
    navigate("automations", "schedule");
    return <AutomationsSpace />;
  },
};

/** History segment — past runs derived from workflows with a last-run time. */
export const History: Story = {
  render: () => {
    reset();
    automationStore.setWorkflows(seedWorkflows());
    navigate("automations", "history");
    return <AutomationsSpace />;
  },
};
