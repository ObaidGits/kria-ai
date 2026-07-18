import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { ScheduleRegion } from "./ScheduleRegion";
import { automationStore } from "../../../stores";
import type { ScheduledTask, TaskItem, Reminder } from "../../../stores";

/**
 * Automations · Schedule segment (task 7.4, Req 6.6). Merges scheduled tasks +
 * routines (recurring reminders) + one-shot reminders + to-do tasks into one
 * grouped view. Stories seed the shared automationStore before rendering.
 */
const meta = {
  title: "Spaces/Automations/ScheduleRegion",
  component: ScheduleRegion,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="standard" style={{ height: "640px", padding: "24px", overflow: "auto" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof ScheduleRegion>;

export default meta;
type Story = StoryObj<typeof meta>;

function seedScheduled(): ScheduledTask[] {
  return [
    { id: "s1", name: "Morning briefing", intervalSecs: 86400, prompt: "Summarize my unread email and today's calendar", enabled: true },
    { id: "s2", name: "Hourly inbox scan", intervalSecs: 3600, prompt: "Check for urgent messages and flag them", enabled: false },
  ];
}
function seedTasks(): TaskItem[] {
  const now = Date.now();
  return [
    { id: 1, title: "Draft the release notes", notes: "Cover the Schedule merge", status: "open", priorityBucket: "high", priorityScore: 80, dueAt: now + 3600_000, source: "manual", createdAt: now - 86400_000 },
    { id: 2, title: "Review the UI redesign spec", notes: null, status: "in_progress", priorityBucket: "normal", priorityScore: 40, dueAt: null, source: "manual", createdAt: now - 172800_000 },
    { id: 3, title: "Archive old logs", notes: null, status: "done", priorityBucket: "low", priorityScore: 10, dueAt: null, source: "manual", createdAt: now - 259200_000 },
  ];
}
function seedReminders(): Reminder[] {
  const now = Date.now();
  return [
    { id: 1, message: "Stand up and stretch", fireAt: now + 1800_000, fired: false, recurrence: "daily" },
    { id: 2, message: "Weekly review", fireAt: now + 604800_000, fired: false, recurrence: "weekly" },
    { id: 3, message: "Reply to the design thread", fireAt: now + 7200_000, fired: false, recurrence: null },
  ];
}

function reset() {
  automationStore.setScheduledTasks([]);
  automationStore.setTasks([]);
  automationStore.setReminders([]);
  automationStore.setLoading(false);
}

/** Fully populated — all four groups (scheduled tasks, routines, reminders, to-dos). */
export const Populated: Story = {
  render: () => {
    reset();
    automationStore.setScheduledTasks(seedScheduled());
    automationStore.setTasks(seedTasks());
    automationStore.setReminders(seedReminders());
    return <ScheduleRegion />;
  },
};

/** Cold — honest empty state with the create bar available. */
export const Empty: Story = {
  render: () => {
    reset();
    return <ScheduleRegion />;
  },
};

/** Only recurring reminders — the Routines group. */
export const RoutinesOnly: Story = {
  render: () => {
    reset();
    automationStore.setReminders(seedReminders().filter((r) => r.recurrence));
    return <ScheduleRegion />;
  },
};
