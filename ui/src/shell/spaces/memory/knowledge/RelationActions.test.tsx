/**
 * Tests for RelationActions component.
 *
 * Verifies:
 *   - Root element renders with correct testid
 *   - idle phase: data-phase="idle", no content
 *   - preview phase: label, description, policyLabel, baseRevision shown
 *   - stale warning shown when isStale=true with role=alert; hidden when false
 *   - commit button disabled when isStale=true; enabled when false
 *   - commit button calls onCommit (when not stale)
 *   - cancel button calls onCancel
 *   - pending confirmation banner shown when isPendingConfirmation=true; hidden when false
 *   - predictionInfo: score shown as "Rank: X%"; rationale shown; profile shown
 *   - prediction score NEVER contains "confidence" (invariant)
 *   - prediction section hidden when predictionInfo is null
 *   - type-change: currentType and proposedType shown
 *   - direction-change: currentDirection and proposedDirection shown
 *   - add-evidence: evidenceSummary shown
 *   - undo: undoTargetRevision shown
 *   - committing phase: data-phase="committing", committing indicator with role=status
 *   - committed phase: newRevision, auditRecordId, description shown
 *   - undo button shown when isPendingUndo=true; calls onUndo; hidden when false
 *   - error phase: error message with role=alert
 *   - retry shown when canRetry=true; calls onCommit; hidden when false
 *
 * Requirements: F4.4 (task 4.4.6)
 */
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@solidjs/testing-library";
import { RelationActions } from "./RelationActions";
import type {
  RelationActionPhase,
  RelationActionPreview,
  RelationActionResult,
  PredictionInfo,
} from "./RelationActions";

// ─── Helpers ──────────────────────────────────────────────────────────────────

function makePredictionInfo(overrides: Partial<PredictionInfo> = {}): PredictionInfo {
  return {
    relativeScore: 0.75,
    rationale: "Strong contextual match based on co-occurrence.",
    profileId: "profile-alpha-001",
    ...overrides,
  };
}

function makePreview(overrides: Partial<RelationActionPreview> = {}): RelationActionPreview {
  return {
    kind: "edit",
    relationId: "rel-001",
    label: "Edit relation",
    description: "Will update the relation metadata",
    policyLabel: "governed:correction:relation-edit",
    baseRevision: 5,
    isStale: false,
    isPendingConfirmation: false,
    predictionInfo: null,
    ...overrides,
  };
}

function makeResult(overrides: Partial<RelationActionResult> = {}): RelationActionResult {
  return {
    kind: "edit",
    newRevision: 6,
    auditRecordId: "audit-rel-001",
    description: "Relation metadata updated.",
    isPendingUndo: false,
    ...overrides,
  };
}

function previewState(overrides: Partial<RelationActionPreview> = {}): RelationActionPhase {
  return { phase: "preview", action: makePreview(overrides) };
}

function committedState(overrides: Partial<RelationActionResult> = {}): RelationActionPhase {
  return { phase: "committed", result: makeResult(overrides) };
}

const idleState: RelationActionPhase = { phase: "idle" };
const committingState: RelationActionPhase = { phase: "committing" };

function errorState(message: string, canRetry: boolean): RelationActionPhase {
  return { phase: "error", message, canRetry };
}

const noOp = () => {};

// ─── Tests ────────────────────────────────────────────────────────────────────

describe("RelationActions", () => {

  // ── Root element ──────────────────────────────────────────────────────────

  it("renders root element with correct testid", () => {
    render(() => (
      <RelationActions state={idleState} onCommit={noOp} onCancel={noOp} onUndo={noOp} />
    ));
    expect(screen.getByTestId("relation-actions-root")).toBeTruthy();
  });

  // ── Idle phase ────────────────────────────────────────────────────────────

  it("idle phase sets data-phase='idle'", () => {
    render(() => (
      <RelationActions state={idleState} onCommit={noOp} onCancel={noOp} onUndo={noOp} />
    ));
    const phase = screen.getByTestId("relation-action-phase");
    expect(phase.getAttribute("data-phase")).toBe("idle");
  });

  it("idle phase renders no preview content", () => {
    render(() => (
      <RelationActions state={idleState} onCommit={noOp} onCancel={noOp} onUndo={noOp} />
    ));
    expect(screen.queryByTestId("relation-action-label")).toBeNull();
    expect(screen.queryByTestId("relation-action-description")).toBeNull();
    expect(screen.queryByTestId("relation-commit-button")).toBeNull();
    expect(screen.queryByTestId("relation-cancel-button")).toBeNull();
  });

  // ── Preview phase — common fields ─────────────────────────────────────────

  it("preview phase sets data-phase='preview'", () => {
    render(() => (
      <RelationActions state={previewState()} onCommit={noOp} onCancel={noOp} onUndo={noOp} />
    ));
    const phase = screen.getByTestId("relation-action-phase");
    expect(phase.getAttribute("data-phase")).toBe("preview");
  });

  it("preview phase shows relation-action-label", () => {
    render(() => (
      <RelationActions
        state={previewState({ label: "Edit relation" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("relation-action-label").textContent).toBe("Edit relation");
  });

  it("preview phase shows relation-action-description", () => {
    render(() => (
      <RelationActions
        state={previewState({ description: "Will update the relation metadata" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("relation-action-description").textContent).toBe(
      "Will update the relation metadata"
    );
  });

  it("preview phase shows relation-action-policy-label", () => {
    render(() => (
      <RelationActions
        state={previewState({ policyLabel: "governed:correction:relation-edit" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("relation-action-policy-label").textContent).toBe(
      "governed:correction:relation-edit"
    );
  });

  it("preview phase shows relation-action-base-revision", () => {
    render(() => (
      <RelationActions
        state={previewState({ baseRevision: 42 })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("relation-action-base-revision").textContent).toBe("42");
  });

  // ── Stale handling ────────────────────────────────────────────────────────

  it("shows relation-stale-warning with role=alert when isStale=true", () => {
    render(() => (
      <RelationActions
        state={previewState({ isStale: true })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const el = screen.getByTestId("relation-stale-warning");
    expect(el.getAttribute("role")).toBe("alert");
  });

  it("hides relation-stale-warning when isStale=false", () => {
    render(() => (
      <RelationActions
        state={previewState({ isStale: false })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("relation-stale-warning")).toBeNull();
  });

  it("commit button is disabled when isStale=true", () => {
    render(() => (
      <RelationActions
        state={previewState({ isStale: true })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const btn = screen.getByTestId("relation-commit-button") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("commit button is enabled when isStale=false", () => {
    render(() => (
      <RelationActions
        state={previewState({ isStale: false })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const btn = screen.getByTestId("relation-commit-button") as HTMLButtonElement;
    expect(btn.disabled).toBe(false);
  });

  // ── Commit / Cancel ───────────────────────────────────────────────────────

  it("commit button calls onCommit when clicked and not stale", () => {
    const onCommit = vi.fn();
    render(() => (
      <RelationActions
        state={previewState({ isStale: false })}
        onCommit={onCommit}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    screen.getByTestId("relation-commit-button").click();
    expect(onCommit).toHaveBeenCalledOnce();
  });

  it("cancel button calls onCancel", () => {
    const onCancel = vi.fn();
    render(() => (
      <RelationActions
        state={previewState()}
        onCommit={noOp}
        onCancel={onCancel}
        onUndo={noOp}
      />
    ));
    screen.getByTestId("relation-cancel-button").click();
    expect(onCancel).toHaveBeenCalledOnce();
  });

  // ── Pending confirmation ──────────────────────────────────────────────────

  it("shows pending-confirmation-banner when isPendingConfirmation=true", () => {
    render(() => (
      <RelationActions
        state={previewState({ isPendingConfirmation: true })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("pending-confirmation-banner")).toBeTruthy();
  });

  it("hides pending-confirmation-banner when isPendingConfirmation=false", () => {
    render(() => (
      <RelationActions
        state={previewState({ isPendingConfirmation: false })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("pending-confirmation-banner")).toBeNull();
  });

  // ── Prediction info ───────────────────────────────────────────────────────

  it("shows prediction-score as 'Rank: X%' when predictionInfo is present", () => {
    render(() => (
      <RelationActions
        state={previewState({ predictionInfo: makePredictionInfo({ relativeScore: 0.83 }) })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("prediction-score").textContent).toBe("Rank: 83%");
  });

  it("rounds prediction score to nearest integer", () => {
    render(() => (
      <RelationActions
        state={previewState({ predictionInfo: makePredictionInfo({ relativeScore: 0.756 }) })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("prediction-score").textContent).toBe("Rank: 76%");
  });

  it("prediction score NEVER contains the word 'confidence'", () => {
    render(() => (
      <RelationActions
        state={previewState({ predictionInfo: makePredictionInfo({ relativeScore: 0.9 }) })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const scoreText = screen.getByTestId("prediction-score").textContent ?? "";
    expect(scoreText.toLowerCase()).not.toContain("confidence");
    expect(scoreText.toLowerCase()).not.toContain("certainty");
    expect(scoreText.toLowerCase()).not.toContain("probability");
  });

  it("shows prediction-rationale when predictionInfo is present", () => {
    render(() => (
      <RelationActions
        state={previewState({
          predictionInfo: makePredictionInfo({
            rationale: "Strong contextual match based on co-occurrence.",
          }),
        })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("prediction-rationale").textContent).toBe(
      "Strong contextual match based on co-occurrence."
    );
  });

  it("shows prediction-profile when predictionInfo is present", () => {
    render(() => (
      <RelationActions
        state={previewState({
          predictionInfo: makePredictionInfo({ profileId: "profile-beta-007" }),
        })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("prediction-profile").textContent).toBe("profile-beta-007");
  });

  it("hides prediction section when predictionInfo is null", () => {
    render(() => (
      <RelationActions
        state={previewState({ predictionInfo: null })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("prediction-score")).toBeNull();
    expect(screen.queryByTestId("prediction-rationale")).toBeNull();
    expect(screen.queryByTestId("prediction-profile")).toBeNull();
  });

  // ── Kind-specific: type-change ────────────────────────────────────────────

  it("type-change shows relation-type-current when currentType is provided", () => {
    render(() => (
      <RelationActions
        state={previewState({ kind: "type-change", currentType: "KNOWS" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("relation-type-current").textContent).toBe("KNOWS");
  });

  it("type-change shows relation-type-proposed when proposedType is provided", () => {
    render(() => (
      <RelationActions
        state={previewState({ kind: "type-change", proposedType: "WORKS_WITH" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("relation-type-proposed").textContent).toBe("WORKS_WITH");
  });

  it("relation-type-current is hidden when currentType is not provided", () => {
    render(() => (
      <RelationActions
        state={previewState({ kind: "type-change" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("relation-type-current")).toBeNull();
  });

  it("relation-type-proposed is hidden when proposedType is not provided", () => {
    render(() => (
      <RelationActions
        state={previewState({ kind: "type-change" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("relation-type-proposed")).toBeNull();
  });

  // ── Kind-specific: direction-change ───────────────────────────────────────

  it("direction-change shows relation-dir-current when currentDirection is provided", () => {
    render(() => (
      <RelationActions
        state={previewState({ kind: "direction-change", currentDirection: "outgoing" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("relation-dir-current").textContent).toBe("outgoing");
  });

  it("direction-change shows relation-dir-proposed when proposedDirection is provided", () => {
    render(() => (
      <RelationActions
        state={previewState({ kind: "direction-change", proposedDirection: "symmetric" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("relation-dir-proposed").textContent).toBe("symmetric");
  });

  it("relation-dir-current is hidden when currentDirection is not provided", () => {
    render(() => (
      <RelationActions
        state={previewState({ kind: "direction-change" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("relation-dir-current")).toBeNull();
  });

  it("relation-dir-proposed is hidden when proposedDirection is not provided", () => {
    render(() => (
      <RelationActions
        state={previewState({ kind: "direction-change" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("relation-dir-proposed")).toBeNull();
  });

  // ── Kind-specific: add-evidence ───────────────────────────────────────────

  it("add-evidence shows relation-evidence-summary when evidenceSummary is provided", () => {
    render(() => (
      <RelationActions
        state={previewState({ kind: "add-evidence", evidenceSummary: "Source: Wikipedia article on X" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("relation-evidence-summary").textContent).toBe(
      "Source: Wikipedia article on X"
    );
  });

  it("relation-evidence-summary is hidden when evidenceSummary is not provided", () => {
    render(() => (
      <RelationActions
        state={previewState({ kind: "add-evidence" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("relation-evidence-summary")).toBeNull();
  });

  // ── Kind-specific: undo ───────────────────────────────────────────────────

  it("undo shows undo-target-revision when undoTargetRevision is provided", () => {
    render(() => (
      <RelationActions
        state={previewState({ kind: "undo", undoTargetRevision: 3 })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("undo-target-revision").textContent).toBe("3");
  });

  it("undo-target-revision is hidden when undoTargetRevision is not provided", () => {
    render(() => (
      <RelationActions
        state={previewState({ kind: "undo" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("undo-target-revision")).toBeNull();
  });

  // ── Committing phase ───────────────────────────────────────────────────────

  it("committing phase sets data-phase='committing'", () => {
    render(() => (
      <RelationActions state={committingState} onCommit={noOp} onCancel={noOp} onUndo={noOp} />
    ));
    const phase = screen.getByTestId("relation-action-phase");
    expect(phase.getAttribute("data-phase")).toBe("committing");
  });

  it("committing phase shows relation-committing indicator with role=status", () => {
    render(() => (
      <RelationActions state={committingState} onCommit={noOp} onCancel={noOp} onUndo={noOp} />
    ));
    const el = screen.getByTestId("relation-committing");
    expect(el.getAttribute("role")).toBe("status");
  });

  // ── Committed phase ────────────────────────────────────────────────────────

  it("committed phase sets data-phase='committed'", () => {
    render(() => (
      <RelationActions state={committedState()} onCommit={noOp} onCancel={noOp} onUndo={noOp} />
    ));
    const phase = screen.getByTestId("relation-action-phase");
    expect(phase.getAttribute("data-phase")).toBe("committed");
  });

  it("committed phase shows relation-result-revision", () => {
    render(() => (
      <RelationActions
        state={committedState({ newRevision: 99 })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("relation-result-revision").textContent).toBe("99");
  });

  it("committed phase shows relation-result-audit-id", () => {
    render(() => (
      <RelationActions
        state={committedState({ auditRecordId: "audit-rel-xyz-789" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("relation-result-audit-id").textContent).toBe("audit-rel-xyz-789");
  });

  it("committed phase shows relation-result-description", () => {
    render(() => (
      <RelationActions
        state={committedState({ description: "Relation direction updated successfully." })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("relation-result-description").textContent).toBe(
      "Relation direction updated successfully."
    );
  });

  // ── Undo button ────────────────────────────────────────────────────────────

  it("undo button shown when isPendingUndo=true", () => {
    render(() => (
      <RelationActions
        state={committedState({ isPendingUndo: true })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("relation-undo-button")).toBeTruthy();
  });

  it("undo button hidden when isPendingUndo=false", () => {
    render(() => (
      <RelationActions
        state={committedState({ isPendingUndo: false })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("relation-undo-button")).toBeNull();
  });

  it("undo button calls onUndo when clicked", () => {
    const onUndo = vi.fn();
    render(() => (
      <RelationActions
        state={committedState({ isPendingUndo: true })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={onUndo}
      />
    ));
    screen.getByTestId("relation-undo-button").click();
    expect(onUndo).toHaveBeenCalledOnce();
  });

  // ── Error phase ────────────────────────────────────────────────────────────

  it("error phase sets data-phase='error'", () => {
    render(() => (
      <RelationActions
        state={errorState("Something failed", false)}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const phase = screen.getByTestId("relation-action-phase");
    expect(phase.getAttribute("data-phase")).toBe("error");
  });

  it("error phase shows relation-error with role=alert", () => {
    render(() => (
      <RelationActions
        state={errorState("Commit conflict detected", false)}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const el = screen.getByTestId("relation-error");
    expect(el.getAttribute("role")).toBe("alert");
    expect(el.textContent).toBe("Commit conflict detected");
  });

  it("retry button shown when canRetry=true", () => {
    render(() => (
      <RelationActions
        state={errorState("Timeout", true)}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("relation-retry")).toBeTruthy();
  });

  it("retry button hidden when canRetry=false", () => {
    render(() => (
      <RelationActions
        state={errorState("Permanent failure", false)}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("relation-retry")).toBeNull();
  });

  it("retry button calls onCommit when clicked", () => {
    const onCommit = vi.fn();
    render(() => (
      <RelationActions
        state={errorState("Timeout", true)}
        onCommit={onCommit}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    screen.getByTestId("relation-retry").click();
    expect(onCommit).toHaveBeenCalledOnce();
  });

});
