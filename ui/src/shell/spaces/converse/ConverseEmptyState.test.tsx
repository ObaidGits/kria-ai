import { describe, it, expect, beforeEach, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup, within } from "@solidjs/testing-library";
import ConverseEmptyState, { COLD_EXAMPLE_INTENTS } from "./ConverseEmptyState";
import {
  getAdaptiveUsage,
  recordAdaptiveUse,
  resetAdaptiveSuggestions,
} from "../../../adaptive";
import { converseStore } from "../../../stores";
import type { Thread } from "../../../stores/converseStore";

function makeThread(id: string, title: string, updatedAt: number): Thread {
  return {
    id,
    title,
    createdAt: updatedAt,
    updatedAt,
    pinned: false,
    archived: false,
    temporary: false,
  };
}

describe("ConverseEmptyState — Core-forward cold/warm empty state (task 3.6, Req 4.6)", () => {
  beforeEach(() => {
    resetAdaptiveSuggestions();
    converseStore.setThreads([]);
    converseStore.clearMessages();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("is Core-forward: always renders the KRIA Core presence", () => {
    render(() => <ConverseEmptyState />);
    // CorePresence renders role="img" with a per-state accessible label.
    expect(screen.getByRole("img", { name: /KRIA/i })).toBeInTheDocument();
  });

  it("COLD: with no prior threads, shows ≤3 example intents", () => {
    render(() => <ConverseEmptyState />);
    const region = screen.getByRole("region", { name: "Start a conversation" });
    expect(region).toHaveAttribute("data-empty-mode", "cold");

    const intents = screen.getByRole("list", { name: "Example intents" });
    const items = intents.querySelectorAll("li");
    expect(items.length).toBeGreaterThan(0);
    expect(items.length).toBeLessThanOrEqual(3);
  });

  it("WARM: with prior threads, shows quiet continue-suggestions (≤3)", () => {
    converseStore.setThreads([
      makeThread("t1", "Daily notes", 3),
      makeThread("t2", "Trip plan", 2),
      makeThread("t3", "Budget", 1),
      makeThread("t4", "Old thread", 0),
    ]);
    render(() => <ConverseEmptyState />);
    const region = screen.getByRole("region", { name: "Start a conversation" });
    expect(region).toHaveAttribute("data-empty-mode", "warm");

    const list = screen.getByRole("list", { name: "Continue suggestions" });
    const items = list.querySelectorAll("li");
    expect(items.length).toBe(3); // capped at 3 even though 4 threads exist
    // Recent-first ordering: most recently updated thread appears first.
    expect(items[0].textContent).toContain("Daily notes");
  });

  it("COLD: clicking an example intent STAGES the composer draft (no send/tool)", () => {
    const updateDraft = vi.spyOn(converseStore, "updateDraft");
    const sendMessage = vi.spyOn(converseStore, "sendMessage");
    const setActiveThread = vi.spyOn(converseStore, "setActiveThread");

    render(() => <ConverseEmptyState />);
    const first = COLD_EXAMPLE_INTENTS[0];
    fireEvent.click(screen.getByRole("button", { name: first.label }));

    // Stages the draft text for review — never auto-sends, never runs a tool.
    expect(updateDraft).toHaveBeenCalledWith({ text: first.draft });
    expect(sendMessage).not.toHaveBeenCalled();
    expect(setActiveThread).not.toHaveBeenCalled();
  });

  it("WARM: clicking a continue-suggestion OPENS the thread (no send/tool)", () => {
    converseStore.setThreads([makeThread("t1", "Daily notes", 3)]);
    const setActiveThread = vi.spyOn(converseStore, "setActiveThread");
    const sendMessage = vi.spyOn(converseStore, "sendMessage");

    render(() => <ConverseEmptyState />);
    fireEvent.click(screen.getByRole("button", { name: "Continue: Daily notes" }));

    expect(setActiveThread).toHaveBeenCalledWith("t1");
    expect(sendMessage).not.toHaveBeenCalled();
  });

  it("never renders a blank page: content is always present (Req 4.6)", () => {
    // Cold
    const { unmount } = render(() => <ConverseEmptyState />);
    expect(screen.getByRole("heading").textContent).toBeTruthy();
    expect(screen.getByRole("img", { name: /KRIA/i })).toBeInTheDocument();
    unmount();

    // Warm
    converseStore.setThreads([makeThread("t1", "Daily notes", 1)]);
    render(() => <ConverseEmptyState />);
    expect(screen.getByRole("heading").textContent).toBeTruthy();
    expect(screen.getByRole("img", { name: /KRIA/i })).toBeInTheDocument();
  });

  it("respects injected adaptive lists (task 13.x hooks)", () => {
    const onSelectIntent = vi.fn();
    const onContinue = vi.fn();

    // Explicit suggestions force WARM regardless of thread state.
    render(() => (
      <ConverseEmptyState
        suggestions={[{ id: "x1", label: "Resume research" }]}
        onContinue={onContinue}
      />
    ));
    fireEvent.click(screen.getByRole("button", { name: "Continue: Resume research" }));
    expect(onContinue).toHaveBeenCalledWith({ id: "x1", label: "Resume research" });
    cleanup();

    // Explicit intents (no threads) stay COLD and use the injected handler.
    render(() => (
      <ConverseEmptyState
        intents={[{ id: "i1", icon: "zap", label: "Custom intent", draft: "custom" }]}
        onSelectIntent={onSelectIntent}
      />
    ));
    fireEvent.click(screen.getByRole("button", { name: "Custom intent" }));
    expect(onSelectIntent).toHaveBeenCalledWith({
      id: "i1",
      icon: "zap",
      label: "Custom intent",
      draft: "custom",
    });
  });
});


describe("ConverseEmptyState — adaptive presentation (Req 19.1/19.2)", () => {
  beforeEach(() => {
    resetAdaptiveSuggestions();
    converseStore.setThreads([]);
    converseStore.clearMessages();
  });

  afterEach(cleanup);

  it("records explicit intent selection and promotes it only on the next presentation", () => {
    const { unmount } = render(() => <ConverseEmptyState />);
    fireEvent.click(screen.getByRole("button", { name: "Remember something" }));
    expect(getAdaptiveUsage("empty-state", "intent:remember")?.count).toBe(1);
    unmount();

    render(() => <ConverseEmptyState />);
    const labels = Array.from(
      screen.getByRole("list", { name: "Example intents" }).querySelectorAll("li"),
      (item) => item.textContent,
    );
    expect(labels[0]).toContain("Remember something");
  });

  it("can promote an older thread into the visible suggestions without deleting peers", () => {
    converseStore.setThreads([
      makeThread("t1", "Newest", 4),
      makeThread("t2", "Second", 3),
      makeThread("t3", "Third", 2),
      makeThread("t4", "Older frequent", 1),
    ]);
    for (let use = 0; use < 5; use += 1) recordAdaptiveUse("empty-state", "thread:t4");

    render(() => <ConverseEmptyState />);
    expect(screen.getByRole("button", { name: "Continue: Older frequent" })).toBeInTheDocument();
    expect(screen.getAllByRole("listitem")).toHaveLength(3);
  });
});


describe("ConverseEmptyState — explainable controls (Req 19.3/19.4)", () => {
  beforeEach(() => {
    resetAdaptiveSuggestions();
    converseStore.setThreads([]);
    converseStore.clearMessages();
  });

  afterEach(cleanup);

  it("explains, pins, dismisses, and resets an adaptive suggestion", () => {
    render(() => <ConverseEmptyState />);
    const controls = screen.getByRole("group", {
      name: "Suggestion controls for Remember something",
    });
    expect(within(controls).getByRole("note")).toHaveTextContent("Default suggestion");

    fireEvent.click(within(controls).getByRole("button", { name: "Pin suggestion: Remember something" }));
    expect(within(controls).getByRole("note")).toHaveTextContent("Pinned by you");

    fireEvent.click(within(controls).getByRole("button", { name: "Dismiss suggestion: Remember something" }));
    expect(screen.queryByRole("button", { name: "Remember something" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Reset suggestions to defaults" }));
    expect(screen.getByRole("button", { name: "Remember something" })).toBeInTheDocument();
  });

  it("keeps returning-user mode after every thread suggestion is dismissed", () => {
    converseStore.setThreads([makeThread("t1", "Daily notes", 1)]);
    render(() => <ConverseEmptyState />);
    fireEvent.click(screen.getByRole("button", { name: "Dismiss suggestion: Daily notes" }));
    expect(screen.getByRole("region", { name: "Start a conversation" }))
      .toHaveAttribute("data-empty-mode", "warm");
    expect(screen.getByRole("heading", { name: "Continue where you left off" })).toBeInTheDocument();
  });
});