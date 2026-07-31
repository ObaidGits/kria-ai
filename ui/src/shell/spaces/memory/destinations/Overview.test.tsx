/**
 * Tests for Overview destination (task 4.2.2).
 *
 * Validates:
 * - isEmpty=true → onboarding section visible; data sections absent
 * - isEmpty=false → recent-changes section visible; onboarding absent
 * - Contradictions section hidden when empty array
 * - Active goals section hidden when empty array
 * - Pending cognition count shown when > 0
 * - Pending cognition section hidden when count is 0
 * - Source consent button rendered in onboarding state
 * - onRequestSourceConsent callback is invoked when consent button clicked
 * - onStartGoal callback is invoked with the entered title
 * - Each change renders kind, label, and timestamp
 * - Each contradiction renders description
 * - Each goal renders title
 *
 * Requirements: F4.2 (task 4.2.2).
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import { render, screen, cleanup, fireEvent } from "@solidjs/testing-library";
import {
  Overview,
  type OverviewProps,
  type OverviewChange,
  type OverviewContradiction,
  type OverviewGoal,
} from "./Overview";

afterEach(() => cleanup());

// ─── Helpers ─────────────────────────────────────────────────────────────────

function makeChange(overrides: Partial<OverviewChange> = {}): OverviewChange {
  return {
    id: "c1",
    kind: "memory",
    label: "Test change",
    timestamp: "2024-01-15T10:00:00Z",
    ...overrides,
  };
}

function makeContradiction(
  overrides: Partial<OverviewContradiction> = {},
): OverviewContradiction {
  return { id: "ct1", description: "Test contradiction", ...overrides };
}

function makeGoal(overrides: Partial<OverviewGoal> = {}): OverviewGoal {
  return { id: "g1", title: "Test goal", status: "active", ...overrides };
}

function renderOverview(props: Partial<OverviewProps> = {}) {
  const defaults: OverviewProps = {
    recentChanges: [],
    contradictions: [],
    activeGoals: [],
    pendingCognitionCount: 0,
    isEmpty: false,
    onboardingState: "none",
    onRequestSourceConsent: vi.fn(),
    onStartGoal: vi.fn(),
  };
  return render(() => <Overview {...defaults} {...props} />);
}

// ─── Empty state / onboarding ─────────────────────────────────────────────────

describe("empty state (isEmpty=true)", () => {
  it("shows the onboarding section", () => {
    renderOverview({ isEmpty: true });
    expect(screen.getByTestId("onboarding")).toBeInTheDocument();
  });

  it("does not show the recent-changes section", () => {
    renderOverview({ isEmpty: true });
    expect(screen.queryByTestId("recent-changes")).not.toBeInTheDocument();
  });

  it("does not show the contradictions section", () => {
    renderOverview({
      isEmpty: true,
      contradictions: [makeContradiction()],
    });
    expect(screen.queryByTestId("contradictions")).not.toBeInTheDocument();
  });

  it("does not show the active-goals section", () => {
    renderOverview({ isEmpty: true, activeGoals: [makeGoal()] });
    expect(screen.queryByTestId("active-goals")).not.toBeInTheDocument();
  });

  it("does not show the pending-cognition section", () => {
    renderOverview({ isEmpty: true, pendingCognitionCount: 5 });
    expect(screen.queryByTestId("pending-cognition")).not.toBeInTheDocument();
  });
});

// ─── Source consent button ────────────────────────────────────────────────────

describe("source consent button", () => {
  it("is rendered in the onboarding section", () => {
    renderOverview({ isEmpty: true });
    expect(screen.getByTestId("source-consent-button")).toBeInTheDocument();
  });

  it("calls onRequestSourceConsent when clicked", () => {
    const onRequestSourceConsent = vi.fn();
    renderOverview({ isEmpty: true, onRequestSourceConsent });
    fireEvent.click(screen.getByTestId("source-consent-button"));
    expect(onRequestSourceConsent).toHaveBeenCalledOnce();
  });

  it("is not rendered outside the onboarding section (isEmpty=false)", () => {
    renderOverview({
      isEmpty: false,
      recentChanges: [makeChange()],
      onRequestSourceConsent: vi.fn(),
    });
    expect(screen.queryByTestId("source-consent-button")).not.toBeInTheDocument();
  });
});

// ─── Onboarding goal form ─────────────────────────────────────────────────────

describe("onboarding goal form", () => {
  it("calls onStartGoal with the entered title", () => {
    const onStartGoal = vi.fn();
    renderOverview({ isEmpty: true, onStartGoal });
    const input = screen.getByRole("textbox", { name: /goal title/i });
    fireEvent.input(input, { target: { value: "Learn SolidJS" } });
    fireEvent.submit(screen.getByTestId("onboarding-goal-form"));
    expect(onStartGoal).toHaveBeenCalledWith("Learn SolidJS");
  });

  it("does not call onStartGoal when the input is empty", () => {
    const onStartGoal = vi.fn();
    renderOverview({ isEmpty: true, onStartGoal });
    fireEvent.submit(screen.getByTestId("onboarding-goal-form"));
    expect(onStartGoal).not.toHaveBeenCalled();
  });
});

// ─── Non-empty state ──────────────────────────────────────────────────────────

describe("non-empty state (isEmpty=false)", () => {
  it("shows the recent-changes section", () => {
    renderOverview({ isEmpty: false, recentChanges: [makeChange()] });
    expect(screen.getByTestId("recent-changes")).toBeInTheDocument();
  });

  it("does not show the onboarding section", () => {
    renderOverview({ isEmpty: false, recentChanges: [makeChange()] });
    expect(screen.queryByTestId("onboarding")).not.toBeInTheDocument();
  });
});

// ─── Recent changes ───────────────────────────────────────────────────────────

describe("recent changes section", () => {
  it("renders each change with kind, label, and timestamp", () => {
    const changes: OverviewChange[] = [
      {
        id: "c1",
        kind: "memory",
        label: "Added fact",
        timestamp: "2024-01-15T10:00:00Z",
      },
      {
        id: "c2",
        kind: "entity",
        label: "Merged entity",
        timestamp: "2024-01-16T12:00:00Z",
      },
    ];
    renderOverview({ isEmpty: false, recentChanges: changes });
    const section = screen.getByTestId("recent-changes");

    // First change
    expect(section).toHaveTextContent("memory");
    expect(section).toHaveTextContent("Added fact");
    expect(section).toHaveTextContent("2024-01-15T10:00:00Z");

    // Second change
    expect(section).toHaveTextContent("entity");
    expect(section).toHaveTextContent("Merged entity");
    expect(section).toHaveTextContent("2024-01-16T12:00:00Z");
  });

  it("shows a fallback message when recentChanges is empty but isEmpty=false", () => {
    renderOverview({ isEmpty: false, recentChanges: [] });
    expect(screen.getByTestId("recent-changes")).toHaveTextContent("No recent changes.");
  });
});

// ─── Contradictions section ───────────────────────────────────────────────────

describe("contradictions section", () => {
  it("shows contradictions section when array is non-empty", () => {
    renderOverview({
      isEmpty: false,
      recentChanges: [makeChange()],
      contradictions: [makeContradiction({ description: "Conflict A vs B" })],
    });
    const section = screen.getByTestId("contradictions");
    expect(section).toBeInTheDocument();
    expect(section).toHaveTextContent("Conflict A vs B");
  });

  it("hides contradictions section when array is empty", () => {
    renderOverview({
      isEmpty: false,
      recentChanges: [makeChange()],
      contradictions: [],
    });
    expect(screen.queryByTestId("contradictions")).not.toBeInTheDocument();
  });
});

// ─── Active goals section ─────────────────────────────────────────────────────

describe("active goals section", () => {
  it("shows active-goals section when array is non-empty", () => {
    renderOverview({
      isEmpty: false,
      recentChanges: [makeChange()],
      activeGoals: [makeGoal({ title: "Ship v2" })],
    });
    const section = screen.getByTestId("active-goals");
    expect(section).toBeInTheDocument();
    expect(section).toHaveTextContent("Ship v2");
  });

  it("hides active-goals section when array is empty", () => {
    renderOverview({
      isEmpty: false,
      recentChanges: [makeChange()],
      activeGoals: [],
    });
    expect(screen.queryByTestId("active-goals")).not.toBeInTheDocument();
  });

  it("renders each goal with its status attribute", () => {
    const goals: OverviewGoal[] = [
      { id: "g1", title: "Goal A", status: "active" },
      { id: "g2", title: "Goal B", status: "paused" },
    ];
    renderOverview({ isEmpty: false, recentChanges: [makeChange()], activeGoals: goals });
    const section = screen.getByTestId("active-goals");
    const items = section.querySelectorAll("li");
    expect(items[0]).toHaveAttribute("data-status", "active");
    expect(items[1]).toHaveAttribute("data-status", "paused");
  });
});

// ─── Pending cognition section ─────────────────────────────────────────────────

describe("pending cognition section", () => {
  it("shows pending-cognition section when count > 0", () => {
    renderOverview({
      isEmpty: false,
      recentChanges: [makeChange()],
      pendingCognitionCount: 3,
    });
    const section = screen.getByTestId("pending-cognition");
    expect(section).toBeInTheDocument();
    expect(section).toHaveTextContent("3 tasks pending");
  });

  it("hides pending-cognition section when count is 0", () => {
    renderOverview({
      isEmpty: false,
      recentChanges: [makeChange()],
      pendingCognitionCount: 0,
    });
    expect(screen.queryByTestId("pending-cognition")).not.toBeInTheDocument();
  });

  it("shows count of 1 correctly", () => {
    renderOverview({
      isEmpty: false,
      recentChanges: [makeChange()],
      pendingCognitionCount: 1,
    });
    expect(screen.getByTestId("pending-cognition")).toHaveTextContent("1 tasks pending");
  });
});

// ─── No health inference from missing data ────────────────────────────────────

describe("no health inference from missing data", () => {
  it("does not render contradictions section with zero-items label", () => {
    const { container } = renderOverview({
      isEmpty: false,
      recentChanges: [makeChange()],
      contradictions: [],
    });
    // Section must not exist at all — not shown as "0 contradictions" or similar
    expect(container.querySelector("[data-testid='contradictions']")).toBeNull();
  });

  it("does not render active-goals section with zero-items label", () => {
    const { container } = renderOverview({
      isEmpty: false,
      recentChanges: [makeChange()],
      activeGoals: [],
    });
    expect(container.querySelector("[data-testid='active-goals']")).toBeNull();
  });

  it("does not render pending-cognition section when count is 0", () => {
    const { container } = renderOverview({
      isEmpty: false,
      recentChanges: [makeChange()],
      pendingCognitionCount: 0,
    });
    expect(container.querySelector("[data-testid='pending-cognition']")).toBeNull();
  });
});
