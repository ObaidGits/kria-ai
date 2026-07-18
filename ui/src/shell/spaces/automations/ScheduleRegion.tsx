/**
 * ScheduleRegion — the Automations "Schedule" segment (task 7.4, Req 6.6).
 *
 * Merges every "on a schedule" surface KRIA has into ONE unified, grouped view:
 *   • Scheduled tasks .. the interval scheduler (cron/interval, prompt-driven) —
 *                        EXISTING `list_scheduled_tasks` / `add_scheduled_task`
 *                        / `remove_scheduled_task`.
 *   • Routines ......... recurring reminders (KRIA has no separate routine
 *                        engine — a reminder with a `recurrence` IS the routine
 *                        primitive) — EXISTING `reminder_*`.
 *   • Reminders ........ one-shot durable reminders — EXISTING `reminder_*`.
 *   • Tasks ............ the unified to-do queue (folds the legacy TasksView) —
 *                        EXISTING `task_*`.
 *
 * ── KRIA runtime-authority invariant ────────────────────────────────────────
 * Pure presentation + command dispatch. Every action (create / complete-toggle
 * / edit / snooze / delete) routes through an EXISTING task, reminder, or
 * scheduler command via `automationStore` → the bridge. The runtime owns
 * execution + persistence; the UI reflects the HONEST success/failure result
 * (Req 6.5 / 20.4) and updates optimistically. No orchestration, no
 * prompt→tool shortcut. Titles / messages / prompts are UNTRUSTED and rendered
 * as escaped text (Solid auto-escapes) — never as HTML.
 *
 * Accessibility (Req 17): each group is a labelled region; the completion
 * control is a real checkbox with a text label; status is shown by icon+text
 * badge (never color alone, Req 17.3); destructive delete/dismiss goes through
 * a deliberate {@link Confirm} dialog (Req 6.6 / 11.3). Enablement of a
 * scheduled task is shown as read-only state because the backend exposes no
 * enable/disable command — the UI never renders a toggle that would silently do
 * nothing (Req 10.6).
 *
 * Requirements: 6.6
 */
import { createEffect, createMemo, createSignal, For, onMount, Show } from "solid-js";
import { automationStore, isRoutine } from "../../../stores";
import type {
  BriefingConfig,
  BriefingSection,
  ScheduledTask,
  TaskItem,
  TaskStatus,
  Reminder,
} from "../../../stores";
import { Badge, Button, Card, Confirm, EmptyState, Input, Select } from "../../../kit";
import type { BadgeTone } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { ScheduleCreateBar } from "./ScheduleCreateBar";
import "./schedule.css";

// ─── Formatting helpers ──────────────────────────────────────────────────────

/** Human-readable interval, e.g. 3600 → "every 1h". */
function formatInterval(secs: number): string {
  if (secs <= 0) return "manual";
  if (secs % 86400 === 0) return `every ${secs / 86400}d`;
  if (secs % 3600 === 0) return `every ${secs / 3600}h`;
  if (secs % 60 === 0) return `every ${secs / 60}m`;
  return `every ${secs}s`;
}

function formatWhen(ms: number): string {
  return new Date(ms).toLocaleString();
}

/** Status → icon + text + tone (never color-alone, Req 17.3). */
function taskStatusPresentation(status: TaskStatus): { icon: string; label: string; tone: BadgeTone } {
  switch (status) {
    case "done":
      return { icon: "check-circle", label: "Done", tone: "success" };
    case "in_progress":
      return { icon: "loader", label: "In progress", tone: "info" };
    case "blocked":
      return { icon: "octagon-alert", label: "Blocked", tone: "danger" };
    case "waiting":
      return { icon: "clock", label: "Waiting", tone: "warning" };
    case "cancelled":
      return { icon: "x-circle", label: "Cancelled", tone: "neutral" };
    case "open":
    default:
      return { icon: "circle", label: "Open", tone: "neutral" };
  }
}

const STATUS_OPTIONS: { value: TaskStatus; label: string }[] = [
  { value: "open", label: "Open" },
  { value: "in_progress", label: "In progress" },
  { value: "blocked", label: "Blocked" },
  { value: "waiting", label: "Waiting" },
  { value: "done", label: "Done" },
  { value: "cancelled", label: "Cancelled" },
];

// ─── Region ──────────────────────────────────────────────────────────────────

export function ScheduleRegion() {
  // Load the merged schedule from all three backends on mount (honest loading
  // state; each source degrades independently if its service is absent).
  onMount(() => void automationStore.loadSchedule());

  const scheduledTasks = createMemo(() => automationStore.scheduledTasks());
  const tasks = createMemo(() => automationStore.tasks());
  const reminders = createMemo(() => automationStore.reminders());
  const routines = createMemo(() => reminders().filter(isRoutine));
  const oneShotReminders = createMemo(() => reminders().filter((r) => !isRoutine(r)));

  const isEmpty = createMemo(
    () =>
      scheduledTasks().length === 0 &&
      tasks().length === 0 &&
      reminders().length === 0,
  );

  return (
    <div class="kria-schedule">
      <h2 class="kria-automations__region-title">Schedule</h2>
      <p class="kria-schedule__lead">
        Everything KRIA runs on a schedule — scheduled tasks, recurring routines,
        reminders, and your to-dos — in one place.
      </p>

      <ScheduleCreateBar />

      <BriefingEditor />

      <Show when={automationStore.loading()}>
        <div class="kria-automations__status" role="status" aria-live="polite">
          Loading schedule…
        </div>
      </Show>

      <Show
        when={!automationStore.loading() && !isEmpty()}
        fallback={
          <Show when={!automationStore.loading()}>
            <EmptyState
              icon="clock"
              title="Nothing scheduled"
              description="Scheduled tasks, routines, reminders, and to-dos will appear here. Create one above."
            />
          </Show>
        }
      >
        <Show when={scheduledTasks().length > 0}>
          <section class="kria-schedule__group" aria-label="Scheduled tasks">
            <h3 class="kria-schedule__group-title">
              <Icon name="repeat" size={15} aria-hidden /> Scheduled tasks
            </h3>
            <ul class="kria-schedule__rows">
              <For each={scheduledTasks()}>
                {(task) => <ScheduledTaskRow task={task} />}
              </For>
            </ul>
          </section>
        </Show>

        <Show when={routines().length > 0}>
          <section class="kria-schedule__group" aria-label="Routines">
            <h3 class="kria-schedule__group-title">
              <Icon name="rotate-cw" size={15} aria-hidden /> Routines
            </h3>
            <ul class="kria-schedule__rows">
              <For each={routines()}>
                {(reminder) => <ReminderRow reminder={reminder} routine />}
              </For>
            </ul>
          </section>
        </Show>

        <Show when={oneShotReminders().length > 0}>
          <section class="kria-schedule__group" aria-label="Reminders">
            <h3 class="kria-schedule__group-title">
              <Icon name="bell" size={15} aria-hidden /> Reminders
            </h3>
            <ul class="kria-schedule__rows">
              <For each={oneShotReminders()}>
                {(reminder) => <ReminderRow reminder={reminder} />}
              </For>
            </ul>
          </section>
        </Show>

        <Show when={tasks().length > 0}>
          <section class="kria-schedule__group" aria-label="Tasks">
            <h3 class="kria-schedule__group-title">
              <Icon name="list-checks" size={15} aria-hidden /> Tasks
            </h3>
            <ul class="kria-schedule__rows">
              <For each={tasks()}>{(task) => <TaskRow task={task} />}</For>
            </ul>
          </section>
        </Show>
      </Show>
    </div>
  );
}

function cloneBriefing(config: BriefingConfig): BriefingConfig {
  return {
    sections: config.sections.map((section) => ({ ...section })),
    schedule: { ...config.schedule, delivery: [...config.schedule.delivery] },
  };
}

function BriefingEditor() {
  const [draft, setDraft] = createSignal<BriefingConfig | null>(null);

  createEffect(() => {
    const config = automationStore.briefingConfig();
    if (config) setDraft(cloneBriefing(config));
  });

  function updateSection(index: number, patch: Partial<BriefingSection>) {
    const config = draft();
    if (!config) return;
    const next = cloneBriefing(config);
    next.sections[index] = { ...next.sections[index], ...patch };
    setDraft(next);
  }

  function updateSchedule(patch: Partial<BriefingConfig["schedule"]>) {
    const config = draft();
    if (!config) return;
    const next = cloneBriefing(config);
    next.schedule = { ...next.schedule, ...patch };
    setDraft(next);
  }

  function toggleDelivery(channel: string) {
    const config = draft();
    if (!config) return;
    const delivery = new Set(config.schedule.delivery);
    delivery.has(channel) ? delivery.delete(channel) : delivery.add(channel);
    updateSchedule({ delivery: [...delivery] });
  }

  async function save() {
    const config = draft();
    if (!config) return;
    const result = await automationStore.saveBriefingConfig(config);
    if (result.ok) setDraft(cloneBriefing(result.data));
  }

  return (
    <section class="kria-schedule__group" aria-labelledby="briefing-builder-title">
      <div class="kria-schedule__briefing-heading">
        <div>
          <h3 id="briefing-builder-title" class="kria-schedule__group-title">
            <Icon name="newspaper" size={15} aria-hidden /> Briefing
          </h3>
          <p class="kria-schedule__row-sub">
            Choose content and delivery for <code>gw_morning_briefing</code>.
          </p>
        </div>
        <Show when={automationStore.briefingError()}>
          <Button variant="secondary" size="sm" onClick={() => void automationStore.loadBriefingConfig()}>
            Retry loading
          </Button>
        </Show>
      </div>

      <Show when={automationStore.briefingLoading() && !draft()}>
        <p class="kria-automations__status" role="status">Loading briefing configuration…</p>
      </Show>
      <Show when={automationStore.briefingError()}>
        {(message) => <p class="kria-schedule__row-error" role="alert"><Icon name="alert-triangle" size={13} aria-hidden /> {message()}</p>}
      </Show>

      <Show when={draft()}>
        {(config) => (
          <div class="kria-schedule__briefing">
            <div class="kria-schedule__briefing-sections">
              <For each={config().sections}>
                {(section, index) => (
                  <Card class="kria-schedule__briefing-card">
                    <label class="kria-schedule__check-label">
                      <input
                        type="checkbox"
                        class="kria-schedule__checkbox kit-focusable"
                        checked={section.enabled}
                        onChange={(event) => updateSection(index(), { enabled: event.currentTarget.checked })}
                      />
                      <strong class="kria-schedule__source">{section.source}</strong>
                    </label>

                    <Show when={section.source === "gmail"}>
                      <div class="kria-schedule__briefing-fields">
                        <Input
                          class="kria-schedule__briefing-grow"
                          label="Gmail query"
                          placeholder="is:unread, subject:urgent"
                          value={section.query ?? ""}
                          onChange={(value) => updateSection(index(), { query: value })}
                        />
                        <Input
                          class="kria-schedule__briefing-max"
                          label="Maximum messages"
                          type="number"
                          value={String(section.max ?? 10)}
                          inputProps={{ min: 1 }}
                          onChange={(value) => updateSection(index(), { max: Number(value) || 10 })}
                        />
                      </div>
                    </Show>

                    <Show when={section.source === "calendar"}>
                      <div class="kria-schedule__briefing-fields">
                        <Select
                          label="Calendar window"
                          options={[{ value: "today", label: "Today" }, { value: "next24h", label: "Next 24 hours" }]}
                          value={section.window ?? "today"}
                          onChange={(value) => value && updateSection(index(), { window: value })}
                        />
                        <label class="kria-schedule__check-label">
                          <input
                            type="checkbox"
                            class="kria-schedule__checkbox kit-focusable"
                            checked={section.include_conflicts ?? true}
                            onChange={(event) => updateSection(index(), { include_conflicts: event.currentTarget.checked })}
                          />
                          Detect conflicts
                        </label>
                      </div>
                    </Show>

                    <Show when={section.source === "github"}>
                      <Input
                        label="GitHub MCP tool"
                        value={section.tool ?? "list_notifications"}
                        onChange={(value) => updateSection(index(), { tool: value })}
                      />
                    </Show>

                    <Show when={section.source === "tasks"}>
                      <Select
                        label="Task filter"
                        options={[
                          { value: "urgent_and_overdue", label: "Urgent and overdue" },
                          { value: "active", label: "All active" },
                          { value: "all", label: "Everything" },
                        ]}
                        value={section.filter ?? "urgent_and_overdue"}
                        onChange={(value) => value && updateSection(index(), { filter: value })}
                      />
                    </Show>
                  </Card>
                )}
              </For>
            </div>

            <Card class="kria-schedule__briefing-card kria-schedule__briefing-delivery">
              <label class="kria-schedule__check-label">
                <input
                  type="checkbox"
                  class="kria-schedule__checkbox kit-focusable"
                  checked={config().schedule.auto}
                  onChange={(event) => updateSchedule({ auto: event.currentTarget.checked })}
                />
                <strong>Auto-deliver daily</strong>
              </label>
              <div class="kria-schedule__briefing-fields">
                <Input
                  label="Delivery time"
                  type="time"
                  value={config().schedule.time}
                  onChange={(value) => updateSchedule({ time: value })}
                />
                <For each={["notification", "chat", "tts"]}>
                  {(channel) => (
                    <label class="kria-schedule__check-label">
                      <input
                        type="checkbox"
                        class="kria-schedule__checkbox kit-focusable"
                        checked={config().schedule.delivery.includes(channel)}
                        onChange={() => toggleDelivery(channel)}
                      />
                      {channel}
                    </label>
                  )}
                </For>
              </div>
            </Card>

            <div class="kria-schedule__briefing-actions">
              <Button variant="primary" disabled={automationStore.briefingSaving()} onClick={() => void save()}>
                {automationStore.briefingSaving() ? "Saving…" : "Save briefing"}
              </Button>
              <Show when={automationStore.briefingStatus()}>
                {(status) => <span class="kria-schedule__create-status" role="status">{status()}</span>}
              </Show>
            </div>
          </div>
        )}
      </Show>
    </section>
  );
}

// ─── Rows ──────────────────────────────────────────────────────────────────────

/** A row-level honest error line (Req 6.5). */
function RowError(props: { message: string | null }) {
  return (
    <Show when={props.message}>
      <p class="kria-schedule__row-error" role="alert">
        <Icon name="alert-triangle" size={13} aria-hidden /> {props.message}
      </p>
    </Show>
  );
}

/**
 * ScheduledTaskRow — an interval scheduler task. Enablement is read-only (no
 * backend enable/disable command → no fake toggle, Req 10.6). Delete is a
 * deliberate confirm (Req 6.6).
 */
function ScheduledTaskRow(props: { task: ScheduledTask }) {
  const [error, setError] = createSignal<string | null>(null);

  async function remove() {
    setError(null);
    const res = await automationStore.removeScheduledTask(props.task.id);
    if (!res.ok) setError(res.message);
  }

  return (
    <li class="kria-schedule__row" data-scheduled-task-id={props.task.id}>
      <Card class="kria-schedule__card">
        <div class="kria-schedule__row-main">
          <span class="kria-schedule__row-title">{props.task.name}</span>
          <Show when={props.task.prompt}>
            <span class="kria-schedule__row-sub">{props.task.prompt}</span>
          </Show>
        </div>
        <div class="kria-schedule__row-meta">
          <Badge tone="neutral">{formatInterval(props.task.intervalSecs)}</Badge>
          <Badge tone={props.task.enabled ? "success" : "neutral"}>
            <Icon name={props.task.enabled ? "check-circle" : "pause-circle"} size={12} aria-hidden />{" "}
            {props.task.enabled ? "Enabled" : "Paused"}
          </Badge>
        </div>
        <div class="kria-schedule__row-actions">
          <Confirm
            triggerIcon="trash-2"
            triggerLabel={`Delete scheduled task ${props.task.name}`}
            title="Delete scheduled task?"
            message={`"${props.task.name}" will stop running on its schedule. This can't be undone.`}
            risk="danger"
            confirmLabel="Delete"
            onConfirm={() => void remove()}
          />
        </div>
      </Card>
      <RowError message={error()} />
    </li>
  );
}

/**
 * TaskRow — a to-do task. The completion toggle is a real checkbox wired to
 * `task_update_status` (done ↔ open); status is fully editable via a Select;
 * delete is a deliberate confirm.
 */
function TaskRow(props: { task: TaskItem }) {
  const [error, setError] = createSignal<string | null>(null);
  const pres = createMemo(() => taskStatusPresentation(props.task.status));
  const done = createMemo(() => props.task.status === "done");
  const checkboxId = `task-done-${props.task.id}`;

  async function toggle(next: boolean) {
    setError(null);
    const res = await automationStore.toggleTaskDone(props.task.id, next);
    if (!res.ok) setError(res.message);
  }

  async function changeStatus(value: string | undefined) {
    if (!value) return;
    setError(null);
    const res = await automationStore.setTaskStatus(props.task.id, value as TaskStatus);
    if (!res.ok) setError(res.message);
  }

  async function remove() {
    setError(null);
    const res = await automationStore.deleteTask(props.task.id);
    if (!res.ok) setError(res.message);
  }

  return (
    <li class="kria-schedule__row" data-task-id={props.task.id}>
      <Card class="kria-schedule__card">
        <div class="kria-schedule__row-check">
          <input
            id={checkboxId}
            type="checkbox"
            class="kria-schedule__checkbox kit-focusable"
            checked={done()}
            onChange={(e) => void toggle(e.currentTarget.checked)}
          />
          <label for={checkboxId} class="kria-schedule__row-main">
            <span
              class="kria-schedule__row-title"
              classList={{ "kria-schedule__row-title--done": done() }}
            >
              {props.task.title}
            </span>
            <Show when={props.task.notes}>
              <span class="kria-schedule__row-sub">{props.task.notes}</span>
            </Show>
          </label>
        </div>
        <div class="kria-schedule__row-meta">
          <Show when={props.task.dueAt !== null}>
            <Badge tone="info">
              <Icon name="calendar" size={12} aria-hidden /> Due {formatWhen(props.task.dueAt!)}
            </Badge>
          </Show>
          <Badge tone={pres().tone}>
            <Icon name={pres().icon} size={12} aria-hidden /> {pres().label}
          </Badge>
        </div>
        <div class="kria-schedule__row-actions">
          <Select
            label={`Status of ${props.task.title}`}
            hideLabel
            options={STATUS_OPTIONS}
            value={props.task.status}
            onChange={(v) => void changeStatus(v)}
          />
          <Confirm
            triggerIcon="trash-2"
            triggerLabel={`Delete task ${props.task.title}`}
            title="Delete task?"
            message={`"${props.task.title}" will be removed. This can't be undone.`}
            risk="danger"
            confirmLabel="Delete"
            onConfirm={() => void remove()}
          />
        </div>
      </Card>
      <RowError message={error()} />
    </li>
  );
}

/**
 * ReminderRow — a one-shot reminder or (when `routine`) a recurring routine.
 * Snooze + dismiss are real reminder commands; dismiss is a deliberate confirm.
 */
function ReminderRow(props: { reminder: Reminder; routine?: boolean }) {
  const [error, setError] = createSignal<string | null>(null);

  async function snooze() {
    setError(null);
    const res = await automationStore.snoozeReminder(props.reminder.id, 10);
    if (!res.ok) setError(res.message);
  }

  async function dismiss() {
    setError(null);
    const res = await automationStore.cancelReminder(props.reminder.id);
    if (!res.ok) setError(res.message);
  }

  return (
    <li class="kria-schedule__row" data-reminder-id={props.reminder.id}>
      <Card class="kria-schedule__card">
        <div class="kria-schedule__row-main">
          <span class="kria-schedule__row-title">{props.reminder.message}</span>
          <span class="kria-schedule__row-sub">
            {props.routine ? "Repeats " : "Fires "}
            {formatWhen(props.reminder.fireAt)}
          </span>
        </div>
        <div class="kria-schedule__row-meta">
          <Show when={props.routine}>
            <Badge tone="accent">
              <Icon name="rotate-cw" size={12} aria-hidden /> {props.reminder.recurrence}
            </Badge>
          </Show>
          <Badge tone={props.reminder.fired ? "neutral" : "info"}>
            <Icon name={props.reminder.fired ? "check" : "bell"} size={12} aria-hidden />{" "}
            {props.reminder.fired ? "Fired" : "Pending"}
          </Badge>
        </div>
        <div class="kria-schedule__row-actions">
          <Button
            variant="secondary"
            size="sm"
            aria-label={`Snooze ${props.reminder.message} 10 minutes`}
            onClick={() => void snooze()}
          >
            <Icon name="alarm-clock" size={14} /> Snooze
          </Button>
          <Confirm
            triggerIcon={props.routine ? "trash-2" : "x"}
            triggerLabel={`${props.routine ? "Delete routine" : "Dismiss reminder"} ${props.reminder.message}`}
            title={props.routine ? "Delete routine?" : "Dismiss reminder?"}
            message={
              props.routine
                ? `"${props.reminder.message}" will stop repeating. This can't be undone.`
                : `"${props.reminder.message}" will be cancelled.`
            }
            risk={props.routine ? "danger" : "warning"}
            confirmLabel={props.routine ? "Delete" : "Dismiss"}
            onConfirm={() => void dismiss()}
          />
        </div>
      </Card>
      <RowError message={error()} />
    </li>
  );
}

export default ScheduleRegion;
