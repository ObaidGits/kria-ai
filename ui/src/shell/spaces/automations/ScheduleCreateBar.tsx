/**
 * ScheduleCreateBar — create surface for the Schedule segment (task 7.4,
 * Req 6.6). One compact form that switches between the three creatable kinds:
 *   • Task ............ a to-do (EXISTING `task_add`), optional due date.
 *   • Reminder ........ a durable reminder in N minutes (EXISTING
 *                       `reminder_set`); pick a recurrence to make it a routine.
 *   • Scheduled task .. an interval, prompt-driven scheduler task (EXISTING
 *                       `add_scheduled_task`).
 *
 * Dispatch-only through `automationStore` → the bridge → existing commands
 * (KRIA runtime authority). Honest success/failure surfaced inline
 * (role="status" / role="alert"); the store reloads the affected list on
 * success so the new item appears without a manual refresh. Accessible: labelled
 * controls, a labelled kind Select, keyboard-operable submit.
 *
 * Requirements: 6.6
 */
import { createSignal, Show } from "solid-js";
import { automationStore } from "../../../stores";
import { Button, Input, Select, Textarea } from "../../../kit";
import { Icon } from "../../../components/Icon";

type CreateKind = "task" | "reminder" | "scheduled";

const KIND_OPTIONS = [
  { value: "task", label: "Task" },
  { value: "reminder", label: "Reminder / routine" },
  { value: "scheduled", label: "Scheduled task" },
];

const RECURRENCE_OPTIONS = [
  { value: "", label: "Once (reminder)" },
  { value: "daily", label: "Daily (routine)" },
  { value: "weekly", label: "Weekly (routine)" },
  { value: "weekly:fri", label: "Weekly on Fri (routine)" },
  { value: "monthly:1", label: "Monthly on the 1st (routine)" },
];

const INTERVAL_OPTIONS = [
  { value: "3600", label: "Every hour" },
  { value: "21600", label: "Every 6 hours" },
  { value: "86400", label: "Every day" },
  { value: "604800", label: "Every week" },
];

export function ScheduleCreateBar() {
  const [kind, setKind] = createSignal<CreateKind>("task");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);

  // Task fields
  const [taskTitle, setTaskTitle] = createSignal("");
  const [taskDue, setTaskDue] = createSignal("");

  // Reminder fields
  const [reminderMsg, setReminderMsg] = createSignal("");
  const [reminderMins, setReminderMins] = createSignal("30");
  const [recurrence, setRecurrence] = createSignal("");

  // Scheduled-task fields
  const [schedName, setSchedName] = createSignal("");
  const [schedInterval, setSchedInterval] = createSignal("86400");
  const [schedPrompt, setSchedPrompt] = createSignal("");

  function resetMessages() {
    setError(null);
    setStatus(null);
  }

  async function submit() {
    resetMessages();
    setBusy(true);
    try {
      if (kind() === "task") {
        const dueIso = taskDue() ? new Date(taskDue()).toISOString() : undefined;
        const res = await automationStore.addTask({ title: taskTitle(), dueAt: dueIso });
        if (!res.ok) return setError(res.message);
        setTaskTitle("");
        setTaskDue("");
        setStatus("Task added.");
      } else if (kind() === "reminder") {
        const res = await automationStore.setReminder({
          message: reminderMsg(),
          fireInMinutes: Number(reminderMins()) || 30,
          recurrence: recurrence() || undefined,
        });
        if (!res.ok) return setError(res.message);
        setReminderMsg("");
        setStatus(recurrence() ? "Routine created." : "Reminder set.");
      } else {
        const res = await automationStore.addScheduledTask({
          name: schedName(),
          intervalSecs: Number(schedInterval()) || 0,
          prompt: schedPrompt(),
        });
        if (!res.ok) return setError(res.message);
        setSchedName("");
        setSchedPrompt("");
        setStatus("Scheduled task created.");
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <form
      class="kria-schedule__create"
      aria-label="Add to the schedule"
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <div class="kria-schedule__create-kind">
        <Select
          label="What to add"
          options={KIND_OPTIONS}
          value={kind()}
          onChange={(v) => {
            resetMessages();
            setKind((v as CreateKind) ?? "task");
          }}
        />
      </div>

      <div class="kria-schedule__create-fields">
        <Show when={kind() === "task"}>
          <Input
            label="Task title"
            placeholder="e.g. Draft the release notes"
            value={taskTitle()}
            onChange={setTaskTitle}
          />
          <Input
            label="Due (optional)"
            type="datetime-local"
            value={taskDue()}
            onChange={setTaskDue}
          />
        </Show>

        <Show when={kind() === "reminder"}>
          <Input
            label="Remind me to…"
            placeholder="e.g. Stand up and stretch"
            value={reminderMsg()}
            onChange={setReminderMsg}
          />
          <Input
            label="In minutes"
            type="number"
            value={reminderMins()}
            onChange={setReminderMins}
          />
          <Select
            label="Repeat"
            options={RECURRENCE_OPTIONS}
            value={recurrence()}
            onChange={(v) => setRecurrence(v ?? "")}
          />
        </Show>

        <Show when={kind() === "scheduled"}>
          <Input
            label="Name"
            placeholder="e.g. Morning briefing"
            value={schedName()}
            onChange={setSchedName}
          />
          <Select
            label="Interval"
            options={INTERVAL_OPTIONS}
            value={schedInterval()}
            onChange={(v) => setSchedInterval(v ?? "86400")}
          />
          <Textarea
            label="Prompt KRIA runs each time"
            placeholder="e.g. Summarize my unread email and today's calendar"
            rows={2}
            value={schedPrompt()}
            onChange={setSchedPrompt}
          />
        </Show>
      </div>

      <div class="kria-schedule__create-submit">
        <Button type="submit" variant="primary" size="sm" disabled={busy()}>
          <Icon name={busy() ? "loader" : "plus"} size={14} />
          {busy() ? "Adding…" : "Add"}
        </Button>
      </div>

      <Show when={error()}>
        <p class="kria-schedule__row-error" role="alert">
          <Icon name="alert-triangle" size={13} aria-hidden /> {error()}
        </p>
      </Show>
      <Show when={status()}>
        <p class="kria-schedule__create-status" role="status" aria-live="polite">
          {status()}
        </p>
      </Show>
    </form>
  );
}

export default ScheduleCreateBar;
