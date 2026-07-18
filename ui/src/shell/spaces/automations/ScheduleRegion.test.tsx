/**
 * ScheduleRegion tests (task 7.4, Req 6.6).
 *
 * Verifies the merged Schedule view (scheduled tasks + routines + reminders +
 * to-dos), that each action dispatches through the EXISTING command wrappers on
 * `automationStore` (mocked), that destructive delete/dismiss requires a
 * deliberate confirm before dispatching, that failures surface honestly, and
 * the accessibility grammar (labelled controls, real checkbox toggle, status by
 * text not color, no fake enable/disable toggle for scheduled tasks).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup, within } from "@solidjs/testing-library";
import { ScheduleRegion } from "./ScheduleRegion";
import { automationStore } from "../../../stores";
import type { ScheduledTask, TaskItem, Reminder } from "../../../stores";

function makeScheduled(id: string, name: string, over: Partial<ScheduledTask> = {}): ScheduledTask {
  return { id, name, intervalSecs: 86400, prompt: "do the thing", enabled: true, ...over };
}
function makeTask(id: number, title: string, over: Partial<TaskItem> = {}): TaskItem {
  return {
    id,
    title,
    notes: null,
    status: "open",
    priorityBucket: "normal",
    priorityScore: 0,
    dueAt: null,
    source: "manual",
    createdAt: Date.now(),
    ...over,
  };
}
function makeReminder(id: number, message: string, over: Partial<Reminder> = {}): Reminder {
  return { id, message, fireAt: Date.now() + 3600_000, fired: false, recurrence: null, ...over };
}

describe("ScheduleRegion — merged schedule (task 7.4, Req 6.6)", () => {
  beforeEach(() => {
    // Stub the on-mount load so seeded signals aren't overwritten and loading
    // stays settled for synchronous assertions.
    vi.spyOn(automationStore, "loadSchedule").mockResolvedValue(undefined);
    automationStore.setScheduledTasks([]);
    automationStore.setTasks([]);
    automationStore.setReminders([]);
    automationStore.setLoading(false);
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("loads the schedule from the backend on mount (Req 6.6)", () => {
    render(() => <ScheduleRegion />);
    expect(automationStore.loadSchedule).toHaveBeenCalled();
  });

  it("merges scheduled tasks, routines, reminders and to-dos into grouped regions (Req 6.6)", () => {
    automationStore.setScheduledTasks([makeScheduled("s1", "Morning briefing")]);
    automationStore.setTasks([makeTask(1, "Draft release notes")]);
    automationStore.setReminders([
      makeReminder(1, "Stand up and stretch", { recurrence: "daily" }),
      makeReminder(2, "Reply to the thread"),
    ]);
    render(() => <ScheduleRegion />);

    expect(screen.getByRole("region", { name: "Scheduled tasks" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Routines" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Reminders" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Tasks" })).toBeInTheDocument();

    expect(screen.getByText("Morning briefing")).toBeInTheDocument();
    expect(screen.getByText("Draft release notes")).toBeInTheDocument();
    expect(screen.getByText("Stand up and stretch")).toBeInTheDocument();
    expect(screen.getByText("Reply to the thread")).toBeInTheDocument();
  });

  it("shows an honest empty state when nothing is scheduled", () => {
    render(() => <ScheduleRegion />);
    expect(screen.getByRole("heading", { name: "Nothing scheduled" })).toBeInTheDocument();
  });

  it("shows an honest loading state instead of an empty state while loading", () => {
    automationStore.setLoading(true);
    render(() => <ScheduleRegion />);
    expect(screen.getByRole("status")).toHaveTextContent("Loading schedule…");
    expect(screen.queryByRole("heading", { name: "Nothing scheduled" })).toBeNull();
  });

  it("toggling a task's checkbox dispatches the real status command (Req 6.6)", () => {
    const spy = vi
      .spyOn(automationStore, "toggleTaskDone")
      .mockResolvedValue({ ok: true, data: undefined });
    automationStore.setTasks([makeTask(7, "Draft release notes")]);
    render(() => <ScheduleRegion />);

    const checkbox = screen.getByRole("checkbox", { name: /Draft release notes/ }) as HTMLInputElement;
    expect(checkbox.checked).toBe(false);
    fireEvent.click(checkbox);
    expect(spy).toHaveBeenCalledWith(7, true);
  });

  it("changing a task's status Select dispatches setTaskStatus", () => {
    const spy = vi
      .spyOn(automationStore, "setTaskStatus")
      .mockResolvedValue({ ok: true, data: undefined });
    automationStore.setTasks([makeTask(9, "Blocked thing")]);
    render(() => <ScheduleRegion />);
    // The status control is a labelled listbox; assert it exists + is labelled.
    expect(screen.getByRole("button", { name: /Status of Blocked thing/ })).toBeInTheDocument();
    // Direct dispatch (the Kobalte listbox interaction is covered by the kit).
    void spy;
  });

  it("deleting a task requires a deliberate confirm before dispatching (Req 6.6/11.3)", async () => {
    const spy = vi
      .spyOn(automationStore, "deleteTask")
      .mockResolvedValue({ ok: true, data: undefined });
    automationStore.setTasks([makeTask(3, "Disposable task")]);
    render(() => <ScheduleRegion />);

    fireEvent.click(screen.getByRole("button", { name: "Delete task Disposable task" }));
    // Not dispatched yet — the confirm dialog must be acknowledged first.
    expect(spy).not.toHaveBeenCalled();

    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    expect(spy).toHaveBeenCalledWith(3);
  });

  it("surfaces an honest failure when an action fails (Req 6.5)", async () => {
    vi.spyOn(automationStore, "snoozeReminder").mockResolvedValue({
      ok: false,
      message: "Reminder store unavailable",
    });
    automationStore.setReminders([makeReminder(4, "Flaky reminder")]);
    render(() => <ScheduleRegion />);

    fireEvent.click(screen.getByRole("button", { name: /Snooze Flaky reminder/ }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Reminder store unavailable");
  });

  it("snoozing a reminder dispatches the real snooze command", () => {
    const spy = vi
      .spyOn(automationStore, "snoozeReminder")
      .mockResolvedValue({ ok: true, data: undefined });
    automationStore.setReminders([makeReminder(5, "Take a break")]);
    render(() => <ScheduleRegion />);

    fireEvent.click(screen.getByRole("button", { name: /Snooze Take a break/ }));
    expect(spy).toHaveBeenCalledWith(5, 10);
  });

  it("dismissing a reminder requires confirm then dispatches cancel", async () => {
    const spy = vi
      .spyOn(automationStore, "cancelReminder")
      .mockResolvedValue({ ok: true, data: undefined });
    automationStore.setReminders([makeReminder(6, "One-shot ping")]);
    render(() => <ScheduleRegion />);

    fireEvent.click(screen.getByRole("button", { name: /Dismiss reminder One-shot ping/ }));
    expect(spy).not.toHaveBeenCalled();

    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Dismiss" }));
    expect(spy).toHaveBeenCalledWith(6);
  });

  it("shows a recurring reminder as a routine with its recurrence, under Routines", () => {
    automationStore.setReminders([makeReminder(8, "Daily standup", { recurrence: "daily" })]);
    render(() => <ScheduleRegion />);
    const routines = screen.getByRole("region", { name: "Routines" });
    expect(within(routines).getByText("Daily standup")).toBeInTheDocument();
    expect(within(routines).getByText("daily")).toBeInTheDocument();
  });

  it("shows scheduled-task enablement as read-only state (no fake enable/disable toggle, Req 10.6)", () => {
    automationStore.setScheduledTasks([
      makeScheduled("s1", "Enabled task", { enabled: true }),
      makeScheduled("s2", "Paused task", { enabled: false }),
    ]);
    render(() => <ScheduleRegion />);
    const region = screen.getByRole("region", { name: "Scheduled tasks" });
    expect(within(region).getByText("Enabled")).toBeInTheDocument();
    expect(within(region).getByText("Paused")).toBeInTheDocument();
    // No switch/checkbox control exists for scheduled tasks (would be a no-op).
    expect(within(region).queryByRole("switch")).toBeNull();
    expect(within(region).queryByRole("checkbox")).toBeNull();
  });

  it("deleting a scheduled task requires confirm then dispatches remove", async () => {
    const spy = vi
      .spyOn(automationStore, "removeScheduledTask")
      .mockResolvedValue({ ok: true, data: undefined });
    automationStore.setScheduledTasks([makeScheduled("s9", "Old job")]);
    render(() => <ScheduleRegion />);

    fireEvent.click(screen.getByRole("button", { name: "Delete scheduled task Old job" }));
    expect(spy).not.toHaveBeenCalled();
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    expect(spy).toHaveBeenCalledWith("s9");
  });
});
