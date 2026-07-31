/**
 * Tests for EntityActions component.
 *
 * Verifies:
 *   - Root element renders with correct testid
 *   - idle phase: data-phase="idle", no preview content
 *   - preview phase: label, description, policyLabel, baseRevision shown
 *   - stale warning shown when isStale=true with role=alert; hidden when false
 *   - commit button disabled when isStale=true; enabled when false
 *   - commit button calls onCommit (when not stale)
 *   - cancel button calls onCancel
 *   - rename: currentName and proposedName shown when provided
 *   - type-change: currentType and proposedType shown
 *   - add-alias/remove-alias: aliasValue shown
 *   - accept/reject-proposal: proposalId shown
 *   - merge: mergeTargetId and mergeTargetLabel shown when non-null
 *   - split: splitField shown
 *   - committing phase: data-phase="committing", committing indicator with role=status
 *   - committed phase: newRevision, auditRecordId, description shown
 *   - error phase: error message with role=alert
 *   - retry shown when canRetry=true; hidden when false; calls onCommit
 *
 * Requirements: F4.4 (task 4.4.5)
 */
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@solidjs/testing-library";
import { EntityActions } from "./EntityActions";
import type {
  EntityActionPhase,
  EntityActionPreview,
  EntityActionResult,
} from "./EntityActions";

// ─── Helpers ──────────────────────────────────────────────────────────────────

function makePreview(overrides: Partial<EntityActionPreview> = {}): EntityActionPreview {
  return {
    kind: "rename",
    itemId: "entity-001",
    label: "Rename entity",
    description: "Will change display name from Old to New",
    policyLabel: "governed:correction:rename",
    baseRevision: 10,
    isStale: false,
    ...overrides,
  };
}

function makeResult(overrides: Partial<EntityActionResult> = {}): EntityActionResult {
  return {
    kind: "rename",
    newRevision: 11,
    auditRecordId: "audit-001",
    description: "Entity display name changed.",
    ...overrides,
  };
}

function previewState(overrides: Partial<EntityActionPreview> = {}): EntityActionPhase {
  return { phase: "preview", action: makePreview(overrides) };
}

function committedState(overrides: Partial<EntityActionResult> = {}): EntityActionPhase {
  return { phase: "committed", result: makeResult(overrides) };
}

const idleState: EntityActionPhase = { phase: "idle" };
const committingState: EntityActionPhase = { phase: "committing" };

function errorState(message: string, canRetry: boolean): EntityActionPhase {
  return { phase: "error", message, canRetry };
}

const noOp = () => {};

// ─── Tests ────────────────────────────────────────────────────────────────────

describe("EntityActions", () => {

  // ── Root element ──────────────────────────────────────────────────────────

  it("renders root element with correct testid", () => {
    render(() => (
      <EntityActions state={idleState} onCommit={noOp} onCancel={noOp} />
    ));
    expect(screen.getByTestId("entity-actions-root")).toBeTruthy();
  });

  // ── Idle phase ────────────────────────────────────────────────────────────

  it("idle phase sets data-phase='idle'", () => {
    render(() => (
      <EntityActions state={idleState} onCommit={noOp} onCancel={noOp} />
    ));
    const phase = screen.getByTestId("entity-action-phase");
    expect(phase.getAttribute("data-phase")).toBe("idle");
  });

  it("idle phase renders no preview content", () => {
    render(() => (
      <EntityActions state={idleState} onCommit={noOp} onCancel={noOp} />
    ));
    expect(screen.queryByTestId("action-label")).toBeNull();
    expect(screen.queryByTestId("action-description")).toBeNull();
    expect(screen.queryByTestId("action-commit-button")).toBeNull();
    expect(screen.queryByTestId("action-cancel-button")).toBeNull();
  });

  // ── Preview phase — common fields ─────────────────────────────────────────

  it("preview phase sets data-phase='preview'", () => {
    render(() => (
      <EntityActions state={previewState()} onCommit={noOp} onCancel={noOp} />
    ));
    const phase = screen.getByTestId("entity-action-phase");
    expect(phase.getAttribute("data-phase")).toBe("preview");
  });

  it("preview phase shows action-label", () => {
    render(() => (
      <EntityActions
        state={previewState({ label: "Rename entity" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("action-label").textContent).toBe("Rename entity");
  });

  it("preview phase shows action-description", () => {
    render(() => (
      <EntityActions
        state={previewState({ description: "Will change display name from Old to New" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("action-description").textContent).toBe(
      "Will change display name from Old to New"
    );
  });

  it("preview phase shows action-policy-label", () => {
    render(() => (
      <EntityActions
        state={previewState({ policyLabel: "governed:correction:rename" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("action-policy-label").textContent).toBe(
      "governed:correction:rename"
    );
  });

  it("preview phase shows action-base-revision", () => {
    render(() => (
      <EntityActions
        state={previewState({ baseRevision: 42 })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("action-base-revision").textContent).toBe("42");
  });

  // ── Stale handling ────────────────────────────────────────────────────────

  it("shows action-stale-warning with role=alert when isStale=true", () => {
    render(() => (
      <EntityActions
        state={previewState({ isStale: true })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const el = screen.getByTestId("action-stale-warning");
    expect(el.getAttribute("role")).toBe("alert");
  });

  it("hides action-stale-warning when isStale=false", () => {
    render(() => (
      <EntityActions
        state={previewState({ isStale: false })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("action-stale-warning")).toBeNull();
  });

  it("commit button is disabled when isStale=true", () => {
    render(() => (
      <EntityActions
        state={previewState({ isStale: true })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const btn = screen.getByTestId("action-commit-button") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("commit button is enabled when isStale=false", () => {
    render(() => (
      <EntityActions
        state={previewState({ isStale: false })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const btn = screen.getByTestId("action-commit-button") as HTMLButtonElement;
    expect(btn.disabled).toBe(false);
  });

  // ── Commit / Cancel ───────────────────────────────────────────────────────

  it("commit button calls onCommit when clicked and not stale", () => {
    const onCommit = vi.fn();
    render(() => (
      <EntityActions
        state={previewState({ isStale: false })}
        onCommit={onCommit}
        onCancel={noOp}
      />
    ));
    screen.getByTestId("action-commit-button").click();
    expect(onCommit).toHaveBeenCalledOnce();
  });

  it("cancel button calls onCancel", () => {
    const onCancel = vi.fn();
    render(() => (
      <EntityActions
        state={previewState()}
        onCommit={noOp}
        onCancel={onCancel}
      />
    ));
    screen.getByTestId("action-cancel-button").click();
    expect(onCancel).toHaveBeenCalledOnce();
  });

  // ── Kind-specific: rename ─────────────────────────────────────────────────

  it("rename shows rename-current when currentName is provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "rename", currentName: "Old Name" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("rename-current").textContent).toBe("Old Name");
  });

  it("rename shows rename-proposed when proposedName is provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "rename", proposedName: "New Name" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("rename-proposed").textContent).toBe("New Name");
  });

  it("rename-current is hidden when currentName is not provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "rename" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("rename-current")).toBeNull();
  });

  it("rename-proposed is hidden when proposedName is not provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "rename" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("rename-proposed")).toBeNull();
  });

  // ── Kind-specific: type-change ────────────────────────────────────────────

  it("type-change shows type-current when currentType is provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "type-change", currentType: "Person" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("type-current").textContent).toBe("Person");
  });

  it("type-change shows type-proposed when proposedType is provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "type-change", proposedType: "Organization" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("type-proposed").textContent).toBe("Organization");
  });

  it("type-current is hidden when currentType is not provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "type-change" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("type-current")).toBeNull();
  });

  it("type-proposed is hidden when proposedType is not provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "type-change" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("type-proposed")).toBeNull();
  });

  // ── Kind-specific: alias ──────────────────────────────────────────────────

  it("add-alias shows alias-value when aliasValue is provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "add-alias", aliasValue: "J. Doe" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("alias-value").textContent).toBe("J. Doe");
  });

  it("remove-alias shows alias-value when aliasValue is provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "remove-alias", aliasValue: "Old Alias" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("alias-value").textContent).toBe("Old Alias");
  });

  it("alias-value is hidden when aliasValue is not provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "add-alias" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("alias-value")).toBeNull();
  });

  // ── Kind-specific: proposal accept / reject ───────────────────────────────

  it("accept-proposal shows proposal-id when proposalId is provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "accept-proposal", proposalId: "prop-abc-123" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("proposal-id").textContent).toBe("prop-abc-123");
  });

  it("reject-proposal shows proposal-id when proposalId is provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "reject-proposal", proposalId: "prop-xyz-999" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("proposal-id").textContent).toBe("prop-xyz-999");
  });

  it("proposal-id is hidden when proposalId is not provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "accept-proposal" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("proposal-id")).toBeNull();
  });

  // ── Kind-specific: merge ──────────────────────────────────────────────────

  it("merge shows merge-target-id when mergeTargetId is provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "merge", mergeTargetId: "entity-002" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("merge-target-id").textContent).toBe("entity-002");
  });

  it("merge shows merge-target-label when mergeTargetLabel is provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "merge", mergeTargetLabel: "John Smith" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("merge-target-label").textContent).toBe("John Smith");
  });

  it("merge-target-id is hidden when mergeTargetId is not provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "merge" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("merge-target-id")).toBeNull();
  });

  it("merge-target-label is hidden when mergeTargetLabel is not provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "merge" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("merge-target-label")).toBeNull();
  });

  // ── Kind-specific: split ──────────────────────────────────────────────────

  it("split shows split-field when splitField is provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "split", splitField: "birth_name" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("split-field").textContent).toBe("birth_name");
  });

  it("split-field is hidden when splitField is not provided", () => {
    render(() => (
      <EntityActions
        state={previewState({ kind: "split" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("split-field")).toBeNull();
  });

  // ── Committing phase ───────────────────────────────────────────────────────

  it("committing phase sets data-phase='committing'", () => {
    render(() => (
      <EntityActions state={committingState} onCommit={noOp} onCancel={noOp} />
    ));
    const phase = screen.getByTestId("entity-action-phase");
    expect(phase.getAttribute("data-phase")).toBe("committing");
  });

  it("committing phase shows action-committing indicator with role=status", () => {
    render(() => (
      <EntityActions state={committingState} onCommit={noOp} onCancel={noOp} />
    ));
    const el = screen.getByTestId("action-committing");
    expect(el.getAttribute("role")).toBe("status");
  });

  // ── Committed phase ────────────────────────────────────────────────────────

  it("committed phase sets data-phase='committed'", () => {
    render(() => (
      <EntityActions state={committedState()} onCommit={noOp} onCancel={noOp} />
    ));
    const phase = screen.getByTestId("entity-action-phase");
    expect(phase.getAttribute("data-phase")).toBe("committed");
  });

  it("committed phase shows action-result-revision", () => {
    render(() => (
      <EntityActions
        state={committedState({ newRevision: 77 })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("action-result-revision").textContent).toBe("77");
  });

  it("committed phase shows action-result-audit-id", () => {
    render(() => (
      <EntityActions
        state={committedState({ auditRecordId: "audit-zzz-456" })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("action-result-audit-id").textContent).toBe("audit-zzz-456");
  });

  it("committed phase shows action-result-description", () => {
    render(() => (
      <EntityActions
        state={committedState({ description: "Entity renamed successfully." })}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("action-result-description").textContent).toBe(
      "Entity renamed successfully."
    );
  });

  // ── Error phase ────────────────────────────────────────────────────────────

  it("error phase sets data-phase='error'", () => {
    render(() => (
      <EntityActions
        state={errorState("Something failed", false)}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const phase = screen.getByTestId("entity-action-phase");
    expect(phase.getAttribute("data-phase")).toBe("error");
  });

  it("error phase shows action-error with role=alert", () => {
    render(() => (
      <EntityActions
        state={errorState("Commit conflict detected", false)}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const el = screen.getByTestId("action-error");
    expect(el.getAttribute("role")).toBe("alert");
    expect(el.textContent).toBe("Commit conflict detected");
  });

  it("retry button shown when canRetry=true", () => {
    render(() => (
      <EntityActions
        state={errorState("Timeout", true)}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("action-retry")).toBeTruthy();
  });

  it("retry button hidden when canRetry=false", () => {
    render(() => (
      <EntityActions
        state={errorState("Permanent failure", false)}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("action-retry")).toBeNull();
  });

  it("retry button calls onCommit when clicked", () => {
    const onCommit = vi.fn();
    render(() => (
      <EntityActions
        state={errorState("Timeout", true)}
        onCommit={onCommit}
        onCancel={noOp}
      />
    ));
    screen.getByTestId("action-retry").click();
    expect(onCommit).toHaveBeenCalledOnce();
  });

});
