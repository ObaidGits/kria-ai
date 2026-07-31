/**
 * Tests for CorrectionPreview component.
 *
 * Verifies:
 *   - Preview phase renders all required fields
 *   - currentValue shown
 *   - proposedValue shown
 *   - evidence shown when non-null; hidden when null
 *   - scope shown
 *   - affectedCount shown
 *   - reversibility with data-reversible attribute (true/false cases)
 *   - reversibilityWindow shown when reversible and non-null; hidden when null/irreversible
 *   - baseRevision shown
 *   - auditConsequence shown
 *   - staleWarning shown when isStale=true with role=alert; hidden when false
 *   - commit button disabled when isStale=true
 *   - commit button enabled when not stale
 *   - cancel button calls onCancel
 *   - commit button calls onCommit
 *   - committing phase shows committing-indicator with role=status
 *   - committed phase shows new-revision, audit-record-id, affected-count
 *   - undo button shown when canUndo=true; hidden when false
 *   - undo button calls onUndo
 *   - undo expiry shown when non-null; hidden when null
 *   - error phase shows error with role=alert
 *   - retry button shown when canRetry=true; calls onCommit
 *   - retry button hidden when canRetry=false
 *
 * Requirements: F4.4 (task 4.4.4)
 */
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@solidjs/testing-library";
import { CorrectionPreview } from "./CorrectionPreview";
import type {
  CorrectionPhase,
  CorrectionPreviewData,
  CorrectionCommitResult,
} from "./CorrectionPreview";

// ─── Helpers ──────────────────────────────────────────────────────────────────

function makePreviewData(overrides: Partial<CorrectionPreviewData> = {}): CorrectionPreviewData {
  return {
    itemId: "item-001",
    fieldName: "title",
    currentValue: "Old Title",
    proposedValue: "New Title",
    evidence: null,
    scope: "this item only",
    affectedCount: 1,
    isReversible: false,
    reversibilityWindow: null,
    baseRevision: 42,
    auditConsequence: "A correction audit record will be created.",
    isStale: false,
    ...overrides,
  };
}

function makeCommitResult(overrides: Partial<CorrectionCommitResult> = {}): CorrectionCommitResult {
  return {
    newRevision: 43,
    auditRecordId: "audit-abc-123",
    affectedCount: 1,
    canUndo: false,
    undoWindowExpiry: null,
    ...overrides,
  };
}

function previewState(data: Partial<CorrectionPreviewData> = {}): CorrectionPhase {
  return { phase: "preview", data: makePreviewData(data) };
}

function committedState(result: Partial<CorrectionCommitResult> = {}): CorrectionPhase {
  return { phase: "committed", result: makeCommitResult(result) };
}

const committingState: CorrectionPhase = { phase: "committing" };

function errorState(message: string, canRetry: boolean): CorrectionPhase {
  return { phase: "error", message, canRetry };
}

const noOp = () => {};

// ─── Tests ────────────────────────────────────────────────────────────────────

describe("CorrectionPreview", () => {

  // Root element
  it("renders root element with correct testid", () => {
    render(() => (
      <CorrectionPreview
        state={previewState()}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("correction-preview")).toBeTruthy();
  });

  // ── Preview phase ──────────────────────────────────────────────────────────

  it("preview phase sets data-phase='preview' on correction-phase element", () => {
    render(() => (
      <CorrectionPreview state={previewState()} onCommit={noOp} onCancel={noOp} onUndo={noOp} />
    ));
    const phase = screen.getByTestId("correction-phase");
    expect(phase.getAttribute("data-phase")).toBe("preview");
  });

  it("shows currentValue", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ currentValue: "The old text" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("current-value").textContent).toBe("The old text");
  });

  it("shows proposedValue", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ proposedValue: "The proposed text" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("proposed-value").textContent).toBe("The proposed text");
  });

  it("shows evidence-note when evidence is non-null", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ evidence: "Source: Wikipedia article on cats" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const el = screen.getByTestId("evidence-note");
    expect(el.textContent).toBe("Source: Wikipedia article on cats");
  });

  it("hides evidence-note when evidence is null", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ evidence: null })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("evidence-note")).toBeNull();
  });

  it("shows scope-label with exact scope text", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ scope: "this item and 3 derivations" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("scope-label").textContent).toBe("this item and 3 derivations");
  });

  it("shows affected-count", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ affectedCount: 7 })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("affected-count").textContent).toBe("7");
  });

  it("reversibility has data-reversible='true' when isReversible=true", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ isReversible: true, reversibilityWindow: "30 days" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const el = screen.getByTestId("reversibility");
    expect(el.getAttribute("data-reversible")).toBe("true");
  });

  it("reversibility has data-reversible='false' when isReversible=false", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ isReversible: false })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const el = screen.getByTestId("reversibility");
    expect(el.getAttribute("data-reversible")).toBe("false");
    expect(el.textContent).toBe("Irreversible");
  });

  it("reversibility shows 'Reversible: N' text when isReversible=true", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ isReversible: true, reversibilityWindow: "30 days" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const el = screen.getByTestId("reversibility");
    expect(el.textContent).toContain("Reversible:");
    expect(el.textContent).toContain("30 days");
  });

  it("shows reversibility-window when reversible and window non-null", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ isReversible: true, reversibilityWindow: "30 days" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const el = screen.getByTestId("reversibility-window");
    expect(el.textContent).toBe("30 days");
  });

  it("hides reversibility-window when reversibilityWindow is null", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ isReversible: true, reversibilityWindow: null })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("reversibility-window")).toBeNull();
  });

  it("hides reversibility-window when not reversible", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ isReversible: false, reversibilityWindow: null })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("reversibility-window")).toBeNull();
  });

  it("shows base-revision", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ baseRevision: 99 })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("base-revision").textContent).toBe("99");
  });

  it("shows audit-consequence with exact backend text", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ auditConsequence: "Correction will be logged permanently." })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("audit-consequence").textContent).toBe(
      "Correction will be logged permanently."
    );
  });

  it("shows stale-warning with role=alert when isStale=true", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ isStale: true })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const el = screen.getByTestId("stale-warning");
    expect(el.getAttribute("role")).toBe("alert");
    expect(el.textContent).toContain("Preview is stale");
  });

  it("hides stale-warning when isStale=false", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ isStale: false })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("stale-warning")).toBeNull();
  });

  it("commit button is disabled when isStale=true", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ isStale: true })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const btn = screen.getByTestId("commit-button") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("commit button is enabled when isStale=false", () => {
    render(() => (
      <CorrectionPreview
        state={previewState({ isStale: false })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const btn = screen.getByTestId("commit-button") as HTMLButtonElement;
    expect(btn.disabled).toBe(false);
  });

  it("commit button calls onCommit when clicked and not stale", () => {
    const onCommit = vi.fn();
    render(() => (
      <CorrectionPreview
        state={previewState({ isStale: false })}
        onCommit={onCommit}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    screen.getByTestId("commit-button").click();
    expect(onCommit).toHaveBeenCalledOnce();
  });

  it("cancel button calls onCancel", () => {
    const onCancel = vi.fn();
    render(() => (
      <CorrectionPreview
        state={previewState()}
        onCommit={noOp}
        onCancel={onCancel}
        onUndo={noOp}
      />
    ));
    screen.getByTestId("cancel-button").click();
    expect(onCancel).toHaveBeenCalledOnce();
  });

  // ── Committing phase ───────────────────────────────────────────────────────

  it("committing phase sets data-phase='committing'", () => {
    render(() => (
      <CorrectionPreview
        state={committingState}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const phase = screen.getByTestId("correction-phase");
    expect(phase.getAttribute("data-phase")).toBe("committing");
  });

  it("committing phase shows committing-indicator with role=status", () => {
    render(() => (
      <CorrectionPreview
        state={committingState}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const el = screen.getByTestId("committing-indicator");
    expect(el.getAttribute("role")).toBe("status");
  });

  // ── Committed phase ────────────────────────────────────────────────────────

  it("committed phase sets data-phase='committed'", () => {
    render(() => (
      <CorrectionPreview
        state={committedState()}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const phase = screen.getByTestId("correction-phase");
    expect(phase.getAttribute("data-phase")).toBe("committed");
  });

  it("committed phase shows new-revision", () => {
    render(() => (
      <CorrectionPreview
        state={committedState({ newRevision: 55 })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("new-revision").textContent).toBe("55");
  });

  it("committed phase shows audit-record-id", () => {
    render(() => (
      <CorrectionPreview
        state={committedState({ auditRecordId: "audit-xyz-789" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("audit-record-id").textContent).toBe("audit-xyz-789");
  });

  it("committed phase shows committed-affected-count", () => {
    render(() => (
      <CorrectionPreview
        state={committedState({ affectedCount: 3 })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("committed-affected-count").textContent).toBe("3");
  });

  it("undo button shown when canUndo=true", () => {
    render(() => (
      <CorrectionPreview
        state={committedState({ canUndo: true })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("undo-button")).toBeTruthy();
  });

  it("undo button hidden when canUndo=false", () => {
    render(() => (
      <CorrectionPreview
        state={committedState({ canUndo: false })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("undo-button")).toBeNull();
  });

  it("undo button calls onUndo when clicked", () => {
    const onUndo = vi.fn();
    render(() => (
      <CorrectionPreview
        state={committedState({ canUndo: true })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={onUndo}
      />
    ));
    screen.getByTestId("undo-button").click();
    expect(onUndo).toHaveBeenCalledOnce();
  });

  it("undo-expiry shown when undoWindowExpiry is non-null", () => {
    render(() => (
      <CorrectionPreview
        state={committedState({ canUndo: true, undoWindowExpiry: "2025-01-01T00:00:00Z" })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const el = screen.getByTestId("undo-expiry");
    expect(el.textContent).toBe("2025-01-01T00:00:00Z");
  });

  it("undo-expiry hidden when undoWindowExpiry is null", () => {
    render(() => (
      <CorrectionPreview
        state={committedState({ undoWindowExpiry: null })}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("undo-expiry")).toBeNull();
  });

  // ── Error phase ────────────────────────────────────────────────────────────

  it("error phase sets data-phase='error'", () => {
    render(() => (
      <CorrectionPreview
        state={errorState("Something went wrong", false)}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const phase = screen.getByTestId("correction-phase");
    expect(phase.getAttribute("data-phase")).toBe("error");
  });

  it("error phase shows correction-error with role=alert", () => {
    render(() => (
      <CorrectionPreview
        state={errorState("Commit failed due to conflict", false)}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    const el = screen.getByTestId("correction-error");
    expect(el.getAttribute("role")).toBe("alert");
    expect(el.textContent).toBe("Commit failed due to conflict");
  });

  it("retry button shown when canRetry=true", () => {
    render(() => (
      <CorrectionPreview
        state={errorState("Timeout", true)}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.getByTestId("correction-retry")).toBeTruthy();
  });

  it("retry button calls onCommit when clicked", () => {
    const onCommit = vi.fn();
    render(() => (
      <CorrectionPreview
        state={errorState("Timeout", true)}
        onCommit={onCommit}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    screen.getByTestId("correction-retry").click();
    expect(onCommit).toHaveBeenCalledOnce();
  });

  it("retry button hidden when canRetry=false", () => {
    render(() => (
      <CorrectionPreview
        state={errorState("Permanent failure", false)}
        onCommit={noOp}
        onCancel={noOp}
        onUndo={noOp}
      />
    ));
    expect(screen.queryByTestId("correction-retry")).toBeNull();
  });

});
