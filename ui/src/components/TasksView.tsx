import { Component, createSignal, For, onMount, Show } from "solid-js";
import { appStore, type Task } from "../stores/app";

const STATUSES = ["open", "in_progress", "blocked", "waiting", "done", "cancelled"];

function bucketColor(bucket: string): string {
  switch (bucket) {
    case "urgent":
      return "#e5484d";
    case "important":
      return "#f5a623";
    case "blocked":
      return "#8e8e8e";
    case "waiting":
      return "#4a9eff";
    default:
      return "#3fb950";
  }
}

/**
 * TasksView — unified task board + durable reminders (Phase 2 frontend).
 */
const TasksView: Component = () => {
  const { tasks, taskStats, reminders } = appStore;
  const [title, setTitle] = createSignal("");
  const [dueAt, setDueAt] = createSignal("");
  const [activeOnly, setActiveOnly] = createSignal(true);
  const [reminderMsg, setReminderMsg] = createSignal("");
  const [reminderMins, setReminderMins] = createSignal(30);
  const [reminderRecur, setReminderRecur] = createSignal("");
  const [completeText, setCompleteText] = createSignal("");
  const [planBlocks, setPlanBlocks] = createSignal<
    { task_id: number; title: string; start: string; end: string; minutes: number }[]
  >([]);
  const [planned, setPlanned] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");

  const refresh = async () => {
    await Promise.all([
      appStore.loadTasks({ activeOnly: activeOnly() }),
      appStore.loadTaskStats(),
      appStore.loadReminders(),
    ]);
  };

  onMount(refresh);

  const addTask = async () => {
    const t = title().trim();
    if (!t) return;
    setBusy(true);
    setError("");
    try {
      const due = dueAt() ? new Date(dueAt()).toISOString() : undefined;
      await appStore.addTask(t, undefined, due);
      setTitle("");
      setDueAt("");
    } catch (e) {
      setError(`Add failed: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  const setReminder = async () => {
    const m = reminderMsg().trim();
    if (!m) return;
    setBusy(true);
    setError("");
    try {
      await appStore.setReminder(m, {
        fireInMinutes: reminderMins(),
        recurrence: reminderRecur() || undefined,
      });
      setReminderMsg("");
      setReminderRecur("");
    } catch (e) {
      setError(`Reminder failed: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  const completeByText = async () => {
    const t = completeText().trim();
    if (!t) return;
    setBusy(true);
    setError("");
    try {
      const done = await appStore.completeTaskByText(t);
      if (!done) setError("No matching active task found.");
      setCompleteText("");
    } catch (e) {
      setError(`Complete failed: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  const planDay = async () => {
    setBusy(true);
    setError("");
    try {
      const res = await appStore.planMyDay();
      setPlanBlocks(res.planned ?? []);
      setPlanned(true);
    } catch (e) {
      setError(`Plan failed: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  const editTaskTitle = async (id: number, current: string) => {
    const next = window.prompt("Edit task title", current);
    if (next && next.trim() && next.trim() !== current) {
      try {
        await appStore.editTask(id, { title: next.trim() });
      } catch (e) {
        setError(`Edit failed: ${e}`);
      }
    }
  };

  const statCard = (label: string, value: number, color?: string) => (
    <div
      class="mcp-server-trust"
      style={`flex:1;min-width:90px;text-align:center;padding:0.6rem;${color ? `border-left:3px solid ${color}` : ""}`}
    >
      <div style="font-size:1.4rem;font-weight:600">{value}</div>
      <div class="field-hint">{label}</div>
    </div>
  );

  return (
    <div class="tasks-view" style="padding:1rem;max-width:920px;margin:0 auto;overflow:auto">
      <h2>Tasks</h2>

      {/* Stats */}
      <Show when={taskStats()}>
        {(s) => (
          <div style="display:flex;flex-wrap:wrap;gap:0.5rem;margin:0.75rem 0">
            {statCard("Open", s().open)}
            {statCard("In progress", s().in_progress, "#4a9eff")}
            {statCard("Urgent", s().urgent, "#e5484d")}
            {statCard("Overdue", s().overdue, "#e5484d")}
            {statCard("Done today", s().done_today, "#3fb950")}
            {statCard("Blocked", s().blocked, "#8e8e8e")}
          </div>
        )}
      </Show>

      {/* Add task */}
      <div style="display:flex;gap:0.5rem;flex-wrap:wrap;align-items:center;margin-bottom:0.75rem">
        <input
          type="text"
          placeholder="New task title…"
          value={title()}
          onInput={(e) => setTitle(e.currentTarget.value)}
          onKeyDown={(e) => e.key === "Enter" && addTask()}
          style="flex:1;min-width:240px"
        />
        <input type="datetime-local" value={dueAt()} onInput={(e) => setDueAt(e.currentTarget.value)} />
        <button class="btn-primary" disabled={busy()} onClick={addTask}>
          Add
        </button>
        <label style="display:flex;align-items:center;gap:0.35rem">
          <input
            type="checkbox"
            checked={activeOnly()}
            onChange={(e) => {
              setActiveOnly(e.currentTarget.checked);
              void refresh();
            }}
          />
          Active only
        </label>
      </div>

      {/* Quick actions: plan day + natural completion */}
      <div style="display:flex;gap:0.5rem;flex-wrap:wrap;align-items:center;margin-bottom:0.75rem">
        <button class="btn-secondary" disabled={busy()} onClick={planDay}>
          📅 Plan my day
        </button>
        <input
          type="text"
          placeholder="I finished… (e.g. 'report ho gaya')"
          value={completeText()}
          onInput={(e) => setCompleteText(e.currentTarget.value)}
          onKeyDown={(e) => e.key === "Enter" && completeByText()}
          style="flex:1;min-width:220px"
        />
        <button class="btn-secondary" disabled={busy()} onClick={completeByText}>
          Mark done
        </button>
      </div>

      <Show when={planned()}>
        <div style="border:1px solid var(--border,#333);border-radius:8px;padding:0.6rem;margin-bottom:0.75rem">
          <strong>Today's plan</strong>
          <Show
            when={planBlocks().length > 0}
            fallback={<p class="field-hint">No free slots or tasks to plan.</p>}
          >
            <For each={planBlocks()}>
              {(b) => (
                <div class="field-hint" style="display:flex;gap:0.5rem;margin-top:0.2rem">
                  <span>
                    {new Date(b.start).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
                    –
                    {new Date(b.end).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
                  </span>
                  <span style="color:var(--text)">{b.title}</span>
                </div>
              )}
            </For>
          </Show>
        </div>
      </Show>

      <Show when={error()}>
        <div class="settings-error" style="margin-bottom:0.6rem">{error()}</div>
      </Show>

      {/* Task list */}
      <Show
        when={tasks().length > 0}
        fallback={<p class="field-hint">No tasks. Add one above, or ask KRIA to add tasks.</p>}
      >
        <For each={tasks()}>
          {(task: Task) => (
            <div
              style="display:flex;align-items:center;gap:0.6rem;padding:0.55rem 0.7rem;border:1px solid var(--border,#333);border-radius:8px;margin-bottom:0.4rem"
            >
              <span
                title={task.priority_bucket}
                style={`width:10px;height:10px;border-radius:50%;background:${bucketColor(task.priority_bucket)};flex:0 0 auto`}
              ></span>
              <div style="flex:1;min-width:0">
                <div style={task.status === "done" ? "text-decoration:line-through;opacity:0.6" : ""}>
                  {task.title}
                </div>
                <Show when={task.due_at}>
                  <div class="field-hint">due {new Date(task.due_at!).toLocaleString()}</div>
                </Show>
              </div>
              <select
                value={task.status}
                onChange={(e) => void appStore.updateTaskStatus(task.id, e.currentTarget.value)}
              >
                <For each={STATUSES}>{(s) => <option value={s}>{s}</option>}</For>
              </select>
              <button class="btn-secondary" onClick={() => void editTaskTitle(task.id, task.title)}>
                ✎
              </button>
              <button class="btn-secondary" onClick={() => void appStore.deleteTask(task.id)}>
                ✕
              </button>
            </div>
          )}
        </For>
      </Show>

      {/* Reminders */}
      <h3 style="margin-top:1.5rem">Reminders</h3>
      <p class="field-hint">Durable — they fire even after a restart.</p>
      <div style="display:flex;gap:0.5rem;flex-wrap:wrap;align-items:center;margin:0.5rem 0">
        <input
          type="text"
          placeholder="Remind me to…"
          value={reminderMsg()}
          onInput={(e) => setReminderMsg(e.currentTarget.value)}
          style="flex:1;min-width:240px"
        />
        <input
          type="number"
          min="1"
          value={reminderMins()}
          onInput={(e) => setReminderMins(Number(e.currentTarget.value) || 30)}
          style="width:90px"
        />
        <span class="field-hint">min</span>
        <select value={reminderRecur()} onChange={(e) => setReminderRecur(e.currentTarget.value)}>
          <option value="">once</option>
          <option value="daily">daily</option>
          <option value="weekly:mon">weekly (Mon)</option>
          <option value="weekly:fri">weekly (Fri)</option>
          <option value="monthly:1">monthly (1st)</option>
        </select>
        <button class="btn-primary" disabled={busy()} onClick={setReminder}>
          Set
        </button>
      </div>
      <Show when={reminders().length > 0} fallback={<p class="field-hint">No pending reminders.</p>}>
        <For each={reminders()}>
          {(r) => (
            <div style="display:flex;gap:0.6rem;align-items:center;padding:0.4rem 0.7rem;border:1px solid var(--border,#333);border-radius:8px;margin-bottom:0.35rem">
              <span>⏰</span>
              <div style="flex:1">
                {r.message}
                <Show when={r.recurrence}>
                  <span class="mcp-server-trust" style="margin-left:0.4rem">{r.recurrence}</span>
                </Show>
              </div>
              <span class="field-hint">{new Date(r.fire_at).toLocaleString()}</span>
              <button class="btn-secondary" onClick={() => void appStore.snoozeReminder(r.id, 10)}>
                Snooze
              </button>
              <button class="btn-secondary" onClick={() => void appStore.cancelReminder(r.id)}>
                ✕
              </button>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
};

export default TasksView;
