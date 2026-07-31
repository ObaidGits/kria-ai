/**
 * Tests for Goals destination (task 4.5.1).
 *
 * Validates all rendering requirements:
 * - Root renders with goals-destination testid
 * - Loading state shown/hidden
 * - Error state shown/hidden
 * - Goals list shown when non-empty and not loading
 * - Empty state shown when empty and not loading
 * - Goal fields: title, status, provenance, evidence (conditional),
 *   linkedMemories (conditional), priority (conditional), lastUpdated
 * - Progress: shown when non-null; percent and milestone conditional
 * - Conflicts: shown when non-empty; hidden when empty
 * - Resume context: shown when paused + non-null; hidden otherwise
 * - Candidate actions: accept/reject
 * - Active actions: pause/complete
 * - Paused actions: activate
 * - Priority selector: shown for active/paused; calls onUpdatePriority
 * - Action callbacks fire correctly
 * - Action phase indicator rendered correctly for all phases
 *
 * Requirements: F4.2 (task 4.5.1)
 */
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, cleanup, fireEvent } from "@solidjs/testing-library";
import {
  Goals,
  type GoalsProps,
  type GoalsState,
  type Goal,
  type GoalActionPhase,
} from "./Goals";

afterEach(() => cleanup());

// ─── Helpers ──────────────────────────────────────────────────────────────────

function makeGoal(overrides: Partial<Goal> = {}): Goal {
  return {
    id: "goal-1",
    title: "Test Goal",
    status: "candidate",
    priority: null,
    provenanceLabel: "inferred from conversation",
    evidenceSummary: null,
    linkedMemoryCount: null,
    progress: null,
    conflicts: [],
    resumeContext: null,
    lastUpdated: "2024-01-01T00:00:00Z",
    ...overrides,
  };
}

const idlePhase: GoalActionPhase = { phase: "idle" };

function makeState(overrides: Partial<GoalsState> = {}): GoalsState {
  return {
    goals: [],
    isLoading: false,
    errorMessage: null,
    actionPhase: idlePhase,
    selectedPriorityValue: null,
    ...overrides,
  };
}

function renderGoals(stateOverrides: Partial<GoalsState> = {}, propsOverrides: Partial<GoalsProps> = {}) {
  const defaults: GoalsProps = {
    state: makeState(stateOverrides),
    onActivate: vi.fn(),
    onPause: vi.fn(),
    onComplete: vi.fn(),
    onRejectCandidate: vi.fn(),
    onAcceptCandidate: vi.fn(),
    onUpdatePriority: vi.fn(),
    onActionCommit: vi.fn(),
    onActionCancel: vi.fn(),
    ...propsOverrides,
  };
  return render(() => <Goals {...defaults} />);
}

// ─── Root ─────────────────────────────────────────────────────────────────────

describe("root", () => {
  it("renders goals-destination root", () => {
    renderGoals();
    expect(screen.getByTestId("goals-destination")).toBeInTheDocument();
  });
});

// ─── Loading ──────────────────────────────────────────────────────────────────

describe("loading state", () => {
  it("shows goals-loading with role=status when isLoading=true", () => {
    renderGoals({ isLoading: true });
    const el = screen.getByTestId("goals-loading");
    expect(el).toBeInTheDocument();
    expect(el).toHaveAttribute("role", "status");
  });

  it("hides goals-loading when isLoading=false", () => {
    renderGoals({ isLoading: false });
    expect(screen.queryByTestId("goals-loading")).not.toBeInTheDocument();
  });
});

// ─── Error ────────────────────────────────────────────────────────────────────

describe("error state", () => {
  it("shows goals-error with role=alert when errorMessage is non-null", () => {
    renderGoals({ errorMessage: "Something went wrong" });
    const el = screen.getByTestId("goals-error");
    expect(el).toBeInTheDocument();
    expect(el).toHaveAttribute("role", "alert");
    expect(el).toHaveTextContent("Something went wrong");
  });

  it("hides goals-error when errorMessage is null", () => {
    renderGoals({ errorMessage: null });
    expect(screen.queryByTestId("goals-error")).not.toBeInTheDocument();
  });
});

// ─── Goals list ───────────────────────────────────────────────────────────────

describe("goals list", () => {
  it("shows goals-list with role=list when goals non-empty and not loading", () => {
    renderGoals({ goals: [makeGoal()], isLoading: false });
    const list = screen.getByTestId("goals-list");
    expect(list).toBeInTheDocument();
    expect(list).toHaveAttribute("role", "list");
  });

  it("hides goals-list when goals array is empty", () => {
    renderGoals({ goals: [] });
    expect(screen.queryByTestId("goals-list")).not.toBeInTheDocument();
  });

  it("hides goals-list while loading even if goals non-empty", () => {
    renderGoals({ goals: [makeGoal()], isLoading: true });
    expect(screen.queryByTestId("goals-list")).not.toBeInTheDocument();
  });
});

// ─── Empty state ──────────────────────────────────────────────────────────────

describe("empty state", () => {
  it("shows goals-empty when goals empty and not loading", () => {
    renderGoals({ goals: [], isLoading: false });
    expect(screen.getByTestId("goals-empty")).toBeInTheDocument();
  });

  it("hides goals-empty when goals non-empty", () => {
    renderGoals({ goals: [makeGoal()], isLoading: false });
    expect(screen.queryByTestId("goals-empty")).not.toBeInTheDocument();
  });

  it("hides goals-empty while loading", () => {
    renderGoals({ goals: [], isLoading: true });
    expect(screen.queryByTestId("goals-empty")).not.toBeInTheDocument();
  });
});

// ─── Goal fields ──────────────────────────────────────────────────────────────

describe("goal fields", () => {
  it("renders goal-title-{id}", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", title: "My Goal" })] });
    expect(screen.getByTestId("goal-title-g1")).toHaveTextContent("My Goal");
  });

  it("renders goal-status-{id} with status text", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "active" })] });
    expect(screen.getByTestId("goal-status-g1")).toHaveTextContent("active");
  });

  it("renders goal-provenance-{id}", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", provenanceLabel: "memory:session:42" })] });
    expect(screen.getByTestId("goal-provenance-g1")).toHaveTextContent("memory:session:42");
  });

  it("renders goal-evidence-{id} when evidenceSummary non-null", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", evidenceSummary: "User mentioned this twice" })] });
    expect(screen.getByTestId("goal-evidence-g1")).toHaveTextContent("User mentioned this twice");
  });

  it("hides goal-evidence-{id} when evidenceSummary is null", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", evidenceSummary: null })] });
    expect(screen.queryByTestId("goal-evidence-g1")).not.toBeInTheDocument();
  });

  it("renders goal-linked-memories-{id} when linkedMemoryCount non-null", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", linkedMemoryCount: 7 })] });
    expect(screen.getByTestId("goal-linked-memories-g1")).toHaveTextContent("7");
  });

  it("hides goal-linked-memories-{id} when linkedMemoryCount is null", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", linkedMemoryCount: null })] });
    expect(screen.queryByTestId("goal-linked-memories-g1")).not.toBeInTheDocument();
  });

  it("renders goal-priority-{id} when priority non-null", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", priority: 3 })] });
    expect(screen.getByTestId("goal-priority-g1")).toHaveTextContent("3");
  });

  it("hides goal-priority-{id} when priority is null", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", priority: null })] });
    expect(screen.queryByTestId("goal-priority-g1")).not.toBeInTheDocument();
  });

  it("renders goal-last-updated-{id}", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", lastUpdated: "2024-06-01T12:00:00Z" })] });
    expect(screen.getByTestId("goal-last-updated-g1")).toHaveTextContent("2024-06-01T12:00:00Z");
  });

  it("goal item has data-status attribute", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "paused" })] });
    expect(screen.getByTestId("goal-g1")).toHaveAttribute("data-status", "paused");
  });
});

// ─── Progress ─────────────────────────────────────────────────────────────────

describe("progress", () => {
  it("renders goal-progress-{id} when progress non-null", () => {
    renderGoals({
      goals: [makeGoal({ id: "g1", progress: { percent: 50, milestoneLabel: "Phase 2", milestoneCount: 4, milestoneCompleted: 2 } })],
    });
    expect(screen.getByTestId("goal-progress-g1")).toBeInTheDocument();
  });

  it("hides goal-progress-{id} when progress is null", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", progress: null })] });
    expect(screen.queryByTestId("goal-progress-g1")).not.toBeInTheDocument();
  });

  it("renders goal-progress-percent-{id} when percent non-null", () => {
    renderGoals({
      goals: [makeGoal({ id: "g1", progress: { percent: 75, milestoneLabel: null, milestoneCount: null, milestoneCompleted: null } })],
    });
    expect(screen.getByTestId("goal-progress-percent-g1")).toHaveTextContent("75");
  });

  it("hides goal-progress-percent-{id} when percent is null", () => {
    renderGoals({
      goals: [makeGoal({ id: "g1", progress: { percent: null, milestoneLabel: "Milestone A", milestoneCount: null, milestoneCompleted: null } })],
    });
    expect(screen.queryByTestId("goal-progress-percent-g1")).not.toBeInTheDocument();
  });

  it("renders goal-progress-milestone-{id} when milestoneLabel non-null", () => {
    renderGoals({
      goals: [makeGoal({ id: "g1", progress: { percent: null, milestoneLabel: "Kickoff", milestoneCount: 3, milestoneCompleted: 1 } })],
    });
    expect(screen.getByTestId("goal-progress-milestone-g1")).toHaveTextContent("Kickoff");
  });

  it("hides goal-progress-milestone-{id} when milestoneLabel is null", () => {
    renderGoals({
      goals: [makeGoal({ id: "g1", progress: { percent: 40, milestoneLabel: null, milestoneCount: null, milestoneCompleted: null } })],
    });
    expect(screen.queryByTestId("goal-progress-milestone-g1")).not.toBeInTheDocument();
  });
});

// ─── Conflicts ────────────────────────────────────────────────────────────────

describe("conflicts", () => {
  it("renders goal-conflicts-{id} when conflicts non-empty", () => {
    const conflicts = [{ conflictingGoalId: "cg1", conflictingGoalLabel: "Other Goal", conflictDescription: "Same resource" }];
    renderGoals({ goals: [makeGoal({ id: "g1", conflicts })] });
    expect(screen.getByTestId("goal-conflicts-g1")).toBeInTheDocument();
  });

  it("hides goal-conflicts-{id} when conflicts empty", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", conflicts: [] })] });
    expect(screen.queryByTestId("goal-conflicts-g1")).not.toBeInTheDocument();
  });

  it("renders each conflict with conflictingGoalId as testid", () => {
    const conflicts = [
      { conflictingGoalId: "cg1", conflictingGoalLabel: "Goal A", conflictDescription: "Desc A" },
      { conflictingGoalId: "cg2", conflictingGoalLabel: "Goal B", conflictDescription: "Desc B" },
    ];
    renderGoals({ goals: [makeGoal({ id: "g1", conflicts })] });
    expect(screen.getByTestId("goal-conflict-cg1")).toBeInTheDocument();
    expect(screen.getByTestId("goal-conflict-cg2")).toBeInTheDocument();
  });

  it("renders conflict label and description", () => {
    const conflicts = [{ conflictingGoalId: "cg1", conflictingGoalLabel: "Conflicting", conflictDescription: "Why conflict" }];
    renderGoals({ goals: [makeGoal({ id: "g1", conflicts })] });
    const el = screen.getByTestId("goal-conflict-cg1");
    expect(el).toHaveTextContent("Conflicting");
    expect(el).toHaveTextContent("Why conflict");
  });
});

// ─── Resume context ───────────────────────────────────────────────────────────

describe("resume context", () => {
  it("shows goal-resume-{id} when status=paused and resumeContext non-null", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "paused", resumeContext: "Pick up from step 3" })] });
    expect(screen.getByTestId("goal-resume-g1")).toHaveTextContent("Pick up from step 3");
  });

  it("hides goal-resume-{id} when status=paused but resumeContext is null", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "paused", resumeContext: null })] });
    expect(screen.queryByTestId("goal-resume-g1")).not.toBeInTheDocument();
  });

  it("hides goal-resume-{id} when resumeContext non-null but status is not paused", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "active", resumeContext: "some context" })] });
    expect(screen.queryByTestId("goal-resume-g1")).not.toBeInTheDocument();
  });
});

// ─── Candidate actions ────────────────────────────────────────────────────────

describe("candidate actions", () => {
  it("shows goal-accept-{id} for candidate status", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "candidate" })] });
    expect(screen.getByTestId("goal-accept-g1")).toBeInTheDocument();
  });

  it("shows goal-reject-{id} for candidate status", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "candidate" })] });
    expect(screen.getByTestId("goal-reject-g1")).toBeInTheDocument();
  });

  it("hides goal-accept-{id} for non-candidate status", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "active" })] });
    expect(screen.queryByTestId("goal-accept-g1")).not.toBeInTheDocument();
  });

  it("hides goal-reject-{id} for non-candidate status", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "paused" })] });
    expect(screen.queryByTestId("goal-reject-g1")).not.toBeInTheDocument();
  });

  it("calls onAcceptCandidate with goalId when accept clicked", () => {
    const onAcceptCandidate = vi.fn();
    renderGoals({ goals: [makeGoal({ id: "g1", status: "candidate" })] }, { onAcceptCandidate });
    fireEvent.click(screen.getByTestId("goal-accept-g1"));
    expect(onAcceptCandidate).toHaveBeenCalledOnce();
    expect(onAcceptCandidate).toHaveBeenCalledWith("g1");
  });

  it("calls onRejectCandidate with goalId when reject clicked", () => {
    const onRejectCandidate = vi.fn();
    renderGoals({ goals: [makeGoal({ id: "g1", status: "candidate" })] }, { onRejectCandidate });
    fireEvent.click(screen.getByTestId("goal-reject-g1"));
    expect(onRejectCandidate).toHaveBeenCalledOnce();
    expect(onRejectCandidate).toHaveBeenCalledWith("g1");
  });
});

// ─── Active actions ───────────────────────────────────────────────────────────

describe("active actions", () => {
  it("shows goal-pause-{id} for active status", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "active" })] });
    expect(screen.getByTestId("goal-pause-g1")).toBeInTheDocument();
  });

  it("shows goal-complete-{id} for active status", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "active" })] });
    expect(screen.getByTestId("goal-complete-g1")).toBeInTheDocument();
  });

  it("hides goal-pause-{id} for non-active status", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "paused" })] });
    expect(screen.queryByTestId("goal-pause-g1")).not.toBeInTheDocument();
  });

  it("hides goal-complete-{id} for non-active status", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "candidate" })] });
    expect(screen.queryByTestId("goal-complete-g1")).not.toBeInTheDocument();
  });

  it("calls onPause with goalId when pause clicked", () => {
    const onPause = vi.fn();
    renderGoals({ goals: [makeGoal({ id: "g1", status: "active" })] }, { onPause });
    fireEvent.click(screen.getByTestId("goal-pause-g1"));
    expect(onPause).toHaveBeenCalledOnce();
    expect(onPause).toHaveBeenCalledWith("g1");
  });

  it("calls onComplete with goalId when complete clicked", () => {
    const onComplete = vi.fn();
    renderGoals({ goals: [makeGoal({ id: "g1", status: "active" })] }, { onComplete });
    fireEvent.click(screen.getByTestId("goal-complete-g1"));
    expect(onComplete).toHaveBeenCalledOnce();
    expect(onComplete).toHaveBeenCalledWith("g1");
  });
});

// ─── Paused actions ───────────────────────────────────────────────────────────

describe("paused actions", () => {
  it("shows goal-activate-{id} for paused status", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "paused" })] });
    expect(screen.getByTestId("goal-activate-g1")).toBeInTheDocument();
  });

  it("hides goal-activate-{id} for non-paused status", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "active" })] });
    expect(screen.queryByTestId("goal-activate-g1")).not.toBeInTheDocument();
  });

  it("hides goal-activate-{id} for candidate status", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "candidate" })] });
    expect(screen.queryByTestId("goal-activate-g1")).not.toBeInTheDocument();
  });

  it("calls onActivate with goalId when activate clicked", () => {
    const onActivate = vi.fn();
    renderGoals({ goals: [makeGoal({ id: "g1", status: "paused" })] }, { onActivate });
    fireEvent.click(screen.getByTestId("goal-activate-g1"));
    expect(onActivate).toHaveBeenCalledOnce();
    expect(onActivate).toHaveBeenCalledWith("g1");
  });
});

// ─── Priority selector ────────────────────────────────────────────────────────

describe("priority selector", () => {
  it("shows goal-priority-selector-{id} for active status", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "active" })] });
    expect(screen.getByTestId("goal-priority-selector-g1")).toBeInTheDocument();
  });

  it("shows goal-priority-selector-{id} for paused status", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "paused" })] });
    expect(screen.getByTestId("goal-priority-selector-g1")).toBeInTheDocument();
  });

  it("hides goal-priority-selector-{id} for candidate status", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "candidate" })] });
    expect(screen.queryByTestId("goal-priority-selector-g1")).not.toBeInTheDocument();
  });

  it("hides goal-priority-selector-{id} for completed status", () => {
    renderGoals({ goals: [makeGoal({ id: "g1", status: "completed" })] });
    expect(screen.queryByTestId("goal-priority-selector-g1")).not.toBeInTheDocument();
  });

  it("calls onUpdatePriority with goalId and priority value when a priority button clicked", () => {
    const onUpdatePriority = vi.fn();
    renderGoals({ goals: [makeGoal({ id: "g1", status: "active" })] }, { onUpdatePriority });
    fireEvent.click(screen.getByTestId("goal-priority-selector-g1-3"));
    expect(onUpdatePriority).toHaveBeenCalledOnce();
    expect(onUpdatePriority).toHaveBeenCalledWith("g1", 3);
  });

  it("calls onUpdatePriority with priority 1 when 1-button clicked", () => {
    const onUpdatePriority = vi.fn();
    renderGoals({ goals: [makeGoal({ id: "g1", status: "paused" })] }, { onUpdatePriority });
    fireEvent.click(screen.getByTestId("goal-priority-selector-g1-1"));
    expect(onUpdatePriority).toHaveBeenCalledWith("g1", 1);
  });

  it("calls onUpdatePriority with priority 5 when 5-button clicked", () => {
    const onUpdatePriority = vi.fn();
    renderGoals({ goals: [makeGoal({ id: "g1", status: "active" })] }, { onUpdatePriority });
    fireEvent.click(screen.getByTestId("goal-priority-selector-g1-5"));
    expect(onUpdatePriority).toHaveBeenCalledWith("g1", 5);
  });
});

// ─── Action phase ─────────────────────────────────────────────────────────────

describe("action phase", () => {
  it("shows goal-action-phase with data-phase=idle when idle", () => {
    renderGoals({ actionPhase: { phase: "idle" } });
    const el = screen.getByTestId("goal-action-phase");
    expect(el).toHaveAttribute("data-phase", "idle");
  });

  it("shows goal-action-phase with data-phase=confirming when confirming", () => {
    renderGoals({ actionPhase: { phase: "confirming", goalId: "g1", action: "activate" } });
    const el = screen.getByTestId("goal-action-phase");
    expect(el).toHaveAttribute("data-phase", "confirming");
  });

  it("shows goal-action-phase with data-phase=committing when committing", () => {
    renderGoals({ actionPhase: { phase: "committing" } });
    const el = screen.getByTestId("goal-action-phase");
    expect(el).toHaveAttribute("data-phase", "committing");
  });

  it("shows goal-action-phase with data-phase=committed when committed", () => {
    renderGoals({ actionPhase: { phase: "committed", newRevision: 5, auditRecordId: "audit-001" } });
    const el = screen.getByTestId("goal-action-phase");
    expect(el).toHaveAttribute("data-phase", "committed");
  });

  it("shows goal-action-revision and goal-action-audit when committed", () => {
    renderGoals({ actionPhase: { phase: "committed", newRevision: 12, auditRecordId: "audit-xyz" } });
    expect(screen.getByTestId("goal-action-revision")).toHaveTextContent("12");
    expect(screen.getByTestId("goal-action-audit")).toHaveTextContent("audit-xyz");
  });

  it("shows goal-action-phase with data-phase=error when error", () => {
    renderGoals({ actionPhase: { phase: "error", message: "Failed to commit" } });
    const el = screen.getByTestId("goal-action-phase");
    expect(el).toHaveAttribute("data-phase", "error");
  });

  it("shows goal-action-error with role=alert when error phase", () => {
    renderGoals({ actionPhase: { phase: "error", message: "Network error" } });
    const el = screen.getByTestId("goal-action-error");
    expect(el).toHaveAttribute("role", "alert");
    expect(el).toHaveTextContent("Network error");
  });

  it("confirming phase shows commit and cancel buttons", () => {
    renderGoals({ actionPhase: { phase: "confirming", goalId: "g1", action: "pause" } });
    expect(screen.getByTestId("goal-action-commit")).toBeInTheDocument();
    expect(screen.getByTestId("goal-action-cancel")).toBeInTheDocument();
  });

  it("confirming phase commit button calls onActionCommit", () => {
    const onActionCommit = vi.fn();
    renderGoals(
      { actionPhase: { phase: "confirming", goalId: "g1", action: "pause" } },
      { onActionCommit },
    );
    fireEvent.click(screen.getByTestId("goal-action-commit"));
    expect(onActionCommit).toHaveBeenCalledOnce();
  });

  it("confirming phase cancel button calls onActionCancel", () => {
    const onActionCancel = vi.fn();
    renderGoals(
      { actionPhase: { phase: "confirming", goalId: "g1", action: "pause" } },
      { onActionCancel },
    );
    fireEvent.click(screen.getByTestId("goal-action-cancel"));
    expect(onActionCancel).toHaveBeenCalledOnce();
  });
});
