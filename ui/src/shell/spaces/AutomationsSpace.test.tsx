import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@solidjs/testing-library";
import AutomationsSpace from "./AutomationsSpace";
import { automationStore } from "../../stores";
import type { Workflow, TaskItem, Reminder } from "../../stores";
import { navigate, currentRoute } from "../router";

function makeWorkflow(id: string, name: string, over: Partial<Workflow> = {}): Workflow {
  return {
    id,
    name,
    description: "",
    status: "idle",
    lastRunAt: null,
    createdAt: Date.now(),
    ...over,
  };
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

describe("AutomationsSpace — segments + top-level workflows (task 7.1, Req 6.1/6.2)", () => {
  beforeEach(() => {
    // Schedule region loads from the backend on mount; stub it so seeded data
    // isn't overwritten and the honest loading flag stays settled.
    vi.spyOn(automationStore, "loadSchedule").mockResolvedValue(undefined);
    automationStore.setWorkflows([]);
    automationStore.setScheduledTasks([]);
    automationStore.setTasks([]);
    automationStore.setReminders([]);
    automationStore.setSearchQuery("");
    automationStore.setLoading(false);
    navigate("automations");
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("renders a tablist with all four segments Run/Build/Schedule/History (Req 6.1)", () => {
    render(() => <AutomationsSpace />);
    expect(screen.getByRole("tablist")).toBeInTheDocument();
    for (const name of ["Run", "Build", "Schedule", "History"]) {
      expect(screen.getByRole("tab", { name })).toBeInTheDocument();
    }
  });

  it("defaults to the Run segment, surfacing workflows at the top level (Req 6.2)", () => {
    render(() => <AutomationsSpace />);
    expect(screen.getByRole("tab", { name: "Run" })).toHaveAttribute("aria-selected", "true");
    // Workflows are the first, most prominent region — not buried.
    expect(screen.getByRole("heading", { name: "Workflows" })).toBeInTheDocument();
  });

  it("surfaces workflows prominently in Run where data exists (Req 6.2)", () => {
    automationStore.setWorkflows([
      makeWorkflow("w1", "Nightly backup", { description: "Back up the DB", status: "completed", lastRunAt: Date.now() }),
      makeWorkflow("w2", "Digest email", { status: "running" }),
    ]);
    render(() => <AutomationsSpace />);
    expect(screen.getByText("Nightly backup")).toBeInTheDocument();
    expect(screen.getByText("Digest email")).toBeInTheDocument();
    expect(screen.getByText("Showing 2 of 2")).toBeInTheDocument();
  });

  it("filters top-level workflows by the Run search (Req 6.2)", () => {
    automationStore.setWorkflows([
      makeWorkflow("w1", "Nightly backup"),
      makeWorkflow("w2", "Digest email"),
    ]);
    render(() => <AutomationsSpace />);
    expect(screen.getByText("Showing 2 of 2")).toBeInTheDocument();

    automationStore.setSearchQuery("backup");
    expect(screen.getByText("Showing 1 of 2")).toBeInTheDocument();
    expect(screen.getByText("Nightly backup")).toBeInTheDocument();
    expect(screen.queryByText("Digest email")).toBeNull();
  });

  it("routes the segment via the typed router and swaps the region on switch (Req 1.5/6.1)", () => {
    render(() => <AutomationsSpace />);
    expect(screen.getByRole("heading", { name: "Workflows" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Schedule" })).toBeNull();

    fireEvent.click(screen.getByRole("tab", { name: "Schedule" }));

    expect(currentRoute().space).toBe("automations");
    expect(currentRoute().segment).toBe("schedule");
    expect(automationStore.activeSegment()).toBe("schedule");
    expect(screen.getByRole("heading", { name: "Schedule" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Workflows" })).toBeNull();
  });

  it("reacts to external route changes while mounted", async () => {
    render(() => <AutomationsSpace />);
    navigate("automations", "build");

    await waitFor(() => {
      expect(screen.getByRole("tab", { name: "Build" })).toHaveAttribute("aria-selected", "true");
      expect(screen.getByRole("heading", { name: "Build" })).toBeInTheDocument();
    });
  });

  it("reveals and focuses a workflow entity deep link", async () => {
    automationStore.setWorkflows([
      makeWorkflow("w1", "Nightly backup"),
      makeWorkflow("w2", "Digest email"),
    ]);
    automationStore.setSearchQuery("digest");
    navigate("automations", "run", "w1");
    render(() => <AutomationsSpace />);

    await waitFor(() => {
      const target = document.querySelector<HTMLElement>('li[data-workflow-id="w1"]');
      expect(target).not.toBeNull();
      expect(document.activeElement).toBe(target);
      expect(target).toHaveAttribute("aria-current", "true");
    });
    expect(automationStore.searchQuery()).toBe("");
  });

  it("shows an honest empty state in Run when there are no workflows (Req 6.1)", () => {
    render(() => <AutomationsSpace />);
    expect(screen.getByRole("heading", { name: "No workflows yet" })).toBeInTheDocument();
  });

  it("shows an honest loading state instead of an empty state while loading", () => {
    automationStore.setLoading(true);
    render(() => <AutomationsSpace />);
    expect(screen.getByRole("status")).toHaveTextContent("Loading workflows…");
    expect(screen.queryByRole("heading", { name: "No workflows yet" })).toBeNull();
  });

  it("renders the Build segment as an honest placeholder (builder lands in 7.3)", () => {
    render(() => <AutomationsSpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Build" }));
    expect(currentRoute().segment).toBe("build");
    expect(screen.getByRole("heading", { name: "Build" })).toBeInTheDocument();
  });

  it("merges to-do tasks and reminders into the Schedule segment (Req 6.6)", () => {
    automationStore.setTasks([makeTask(1, "Morning digest")]);
    automationStore.setReminders([makeReminder(1, "Call the dentist")]);
    render(() => <AutomationsSpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Schedule" }));
    expect(screen.getByText("Morning digest")).toBeInTheDocument();
    expect(screen.getByText("Call the dentist")).toBeInTheDocument();
  });

  it("shows an honest empty state in Schedule when nothing is scheduled", () => {
    render(() => <AutomationsSpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Schedule" }));
    expect(screen.getByRole("heading", { name: "Nothing scheduled" })).toBeInTheDocument();
  });

  it("lists past runs in History where a workflow has run (Req 6.1)", () => {
    automationStore.setWorkflows([
      makeWorkflow("w1", "Nightly backup", { status: "completed", lastRunAt: Date.now() }),
      makeWorkflow("w2", "Never run", { status: "idle", lastRunAt: null }),
    ]);
    render(() => <AutomationsSpace />);
    fireEvent.click(screen.getByRole("tab", { name: "History" }));
    expect(screen.getByText("Nightly backup")).toBeInTheDocument();
    expect(screen.queryByText("Never run")).toBeNull();
  });

  it("shows an honest empty state in History when nothing has run", () => {
    render(() => <AutomationsSpace />);
    fireEvent.click(screen.getByRole("tab", { name: "History" }));
    expect(screen.getByRole("heading", { name: "No runs yet" })).toBeInTheDocument();
  });
});
