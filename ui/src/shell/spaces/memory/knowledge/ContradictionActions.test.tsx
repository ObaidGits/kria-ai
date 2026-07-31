/**
 * Tests for ContradictionActions component.
 *
 * Verifies:
 *   - Root element renders with correct testid
 *   - idle phase: data-phase="idle", no preview content
 *   - preview phase: itemA/itemB fields rendered
 *   - description, policyLabel, baseRevision shown
 *   - stale warning shown when isStale=true with role=alert; hidden when false
 *   - Evidence path shown when non-null; hidden when null
 *   - canConfirm=true shows action-confirm-button; false hides it completely
 *   - canSupersede=true shows button; false hides completely
 *   - canKeepBoth=true shows button; false hides completely
 *   - Selecting action calls onSelectAction with correct kind
 *   - Selected action has data-selected=true; others have data-selected=false
 *   - commit disabled when selectedAction=null
 *   - commit disabled when isStale=true (even with selectedAction set)
 *   - commit enabled when selectedAction set and not stale
 *   - commit calls onCommit; cancel calls onCancel
 *   - supersededBy shown when present
 *   - committing phase
 *   - committed phase: revision, audit ID, description, kind
 *   - error phase with retry
 *   - No "hidden" indicators — absent actions leave no trace
 *
 * Requirements: F4.4 (task 4.4.7)
 */
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@solidjs/testing-library";
import { ContradictionActions } from "./ContradictionActions";
import type {
  ContradictionActionPhase,
  ContradictionPreview,
  ContradictionItem,
  ContradictionActionResult,
  EvidencePath,
  ContradictionActionKind,
} from "./ContradictionActions";

// ─── Helpers ──────────────────────────────────────────────────────────────────

function makeItem(overrides: Partial<ContradictionItem> = {}): ContradictionItem {
  return {
    itemId: "item-001",
    label: "Item Alpha",
    value: "value-alpha",
    truthState: "asserted",
    evidenceSummary: null,
    ...overrides,
  };
}

function makeEvidencePath(overrides: Partial<EvidencePath> = {}): EvidencePath {
  return {
    pathId: "path-001",
    steps: ["node-A", "edge-X", "node-B"],
    evidenceSummary: "Evidence from three linked sources.",
    ...overrides,
  };
}

function makePreview(overrides: Partial<ContradictionPreview> = {}): ContradictionPreview {
  return {
    itemA: makeItem({ itemId: "item-001", label: "Alpha", value: "val-a", truthState: "asserted" }),
    itemB: makeItem({ itemId: "item-002", label: "Beta", value: "val-b", truthState: "refuted" }),
    description: "Values A and B contradict each other on field X.",
    policyLabel: "governed:contradiction:review",
    baseRevision: 7,
    isStale: false,
    canConfirm: true,
    canSupersede: true,
    canKeepBoth: true,
    evidencePath: null,
    ...overrides,
  };
}

function makeResult(overrides: Partial<ContradictionActionResult> = {}): ContradictionActionResult {
  return {
    kind: "confirm",
    newRevision: 8,
    auditRecordId: "audit-con-001",
    description: "Contradiction confirmed as unresolved.",
    ...overrides,
  };
}

function previewState(
  overrides: Partial<ContradictionPreview> = {},
): ContradictionActionPhase {
  return { phase: "preview", action: makePreview(overrides) };
}

function committedState(
  overrides: Partial<ContradictionActionResult> = {},
): ContradictionActionPhase {
  return { phase: "committed", result: makeResult(overrides) };
}

const idleState: ContradictionActionPhase = { phase: "idle" };

function committingState(kind: ContradictionActionKind = "confirm"): ContradictionActionPhase {
  return { phase: "committing", kind };
}

function errorState(message: string, canRetry: boolean): ContradictionActionPhase {
  return { phase: "error", message, canRetry };
}

const noOp = () => {};
const noOpSelect = (_kind: ContradictionActionKind) => {};

// ─── Tests ────────────────────────────────────────────────────────────────────

describe("ContradictionActions", () => {

  // ── Root element ──────────────────────────────────────────────────────────

  it("renders root element with correct testid", () => {
    render(() => (
      <ContradictionActions
        state={idleState}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("contradiction-actions-root")).toBeTruthy();
  });

  // ── Idle phase ────────────────────────────────────────────────────────────

  it("idle phase sets data-phase='idle'", () => {
    render(() => (
      <ContradictionActions
        state={idleState}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const phase = screen.getByTestId("contradiction-action-phase");
    expect(phase.getAttribute("data-phase")).toBe("idle");
  });

  it("idle phase renders no preview content", () => {
    render(() => (
      <ContradictionActions
        state={idleState}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("contradiction-item-a")).toBeNull();
    expect(screen.queryByTestId("contradiction-item-b")).toBeNull();
    expect(screen.queryByTestId("contradiction-description")).toBeNull();
    expect(screen.queryByTestId("contradiction-commit-button")).toBeNull();
  });

  // ── Preview phase — item fields ───────────────────────────────────────────

  it("preview phase sets data-phase='preview'", () => {
    render(() => (
      <ContradictionActions
        state={previewState()}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const phase = screen.getByTestId("contradiction-action-phase");
    expect(phase.getAttribute("data-phase")).toBe("preview");
  });

  it("itemA is rendered with correct data-item-id", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ itemA: makeItem({ itemId: "item-aaa" }) })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const el = screen.getByTestId("contradiction-item-a");
    expect(el.getAttribute("data-item-id")).toBe("item-aaa");
  });

  it("itemB is rendered with correct data-item-id", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ itemB: makeItem({ itemId: "item-bbb" }) })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const el = screen.getByTestId("contradiction-item-b");
    expect(el.getAttribute("data-item-id")).toBe("item-bbb");
  });

  it("itemA shows item-label, item-value, truth-state fields", () => {
    render(() => (
      <ContradictionActions
        state={previewState({
          itemA: makeItem({ label: "Label A", value: "Value A", truthState: "asserted" }),
        })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const item = screen.getByTestId("contradiction-item-a");
    const fields = item.querySelectorAll("[data-field]");
    const fieldMap: Record<string, string> = {};
    fields.forEach((f) => {
      fieldMap[f.getAttribute("data-field")!] = f.textContent ?? "";
    });
    expect(fieldMap["item-label"]).toBe("Label A");
    expect(fieldMap["item-value"]).toBe("Value A");
    expect(fieldMap["truth-state"]).toBe("asserted");
  });

  it("itemA evidence-summary field shown when non-null", () => {
    render(() => (
      <ContradictionActions
        state={previewState({
          itemA: makeItem({ evidenceSummary: "Some evidence for A" }),
        })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const item = screen.getByTestId("contradiction-item-a");
    const evField = item.querySelector('[data-field="evidence-summary"]');
    expect(evField).not.toBeNull();
    expect(evField!.textContent).toBe("Some evidence for A");
  });

  it("itemA evidence-summary field hidden when null", () => {
    render(() => (
      <ContradictionActions
        state={previewState({
          itemA: makeItem({ evidenceSummary: null }),
        })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const item = screen.getByTestId("contradiction-item-a");
    const evField = item.querySelector('[data-field="evidence-summary"]');
    expect(evField).toBeNull();
  });

  // ── Preview phase — common fields ─────────────────────────────────────────

  it("shows contradiction-description from backend", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ description: "Field X has contradictory values." })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("contradiction-description").textContent).toBe(
      "Field X has contradictory values."
    );
  });

  it("shows contradiction-policy-label", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ policyLabel: "governed:contradiction:strict" })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("contradiction-policy-label").textContent).toBe(
      "governed:contradiction:strict"
    );
  });

  it("shows contradiction-base-revision", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ baseRevision: 42 })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("contradiction-base-revision").textContent).toBe("42");
  });

  // ── Stale handling ────────────────────────────────────────────────────────

  it("shows contradiction-stale-warning with role=alert when isStale=true", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ isStale: true })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const el = screen.getByTestId("contradiction-stale-warning");
    expect(el.getAttribute("role")).toBe("alert");
  });

  it("hides contradiction-stale-warning when isStale=false", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ isStale: false })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("contradiction-stale-warning")).toBeNull();
  });

  // ── Evidence path ─────────────────────────────────────────────────────────

  it("shows evidence-path section when evidencePath is non-null", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ evidencePath: makeEvidencePath() })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("evidence-path")).toBeTruthy();
  });

  it("shows evidence-path-summary text", () => {
    render(() => (
      <ContradictionActions
        state={previewState({
          evidencePath: makeEvidencePath({ evidenceSummary: "Derived from 3 linked sources." }),
        })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("evidence-path-summary").textContent).toBe(
      "Derived from 3 linked sources."
    );
  });

  it("shows evidence-path-steps list", () => {
    render(() => (
      <ContradictionActions
        state={previewState({
          evidencePath: makeEvidencePath({ steps: ["A", "B", "C"] }),
        })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const stepsEl = screen.getByTestId("evidence-path-steps");
    expect(stepsEl.textContent).toContain("A");
    expect(stepsEl.textContent).toContain("B");
    expect(stepsEl.textContent).toContain("C");
  });

  it("hides evidence-path section when evidencePath is null", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ evidencePath: null })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("evidence-path")).toBeNull();
    expect(screen.queryByTestId("evidence-path-summary")).toBeNull();
    expect(screen.queryByTestId("evidence-path-steps")).toBeNull();
  });

  // ── Capability gating — confirm ───────────────────────────────────────────

  it("shows action-confirm-button when canConfirm=true", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ canConfirm: true })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("action-confirm-button")).toBeTruthy();
  });

  it("hides action-confirm-button completely when canConfirm=false", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ canConfirm: false })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("action-confirm-button")).toBeNull();
  });

  // ── Capability gating — supersede ─────────────────────────────────────────

  it("shows action-supersede-button when canSupersede=true", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ canSupersede: true })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("action-supersede-button")).toBeTruthy();
  });

  it("hides action-supersede-button completely when canSupersede=false", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ canSupersede: false })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("action-supersede-button")).toBeNull();
  });

  // ── Capability gating — keep-both ─────────────────────────────────────────

  it("shows action-keep-both-button when canKeepBoth=true", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ canKeepBoth: true })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("action-keep-both-button")).toBeTruthy();
  });

  it("hides action-keep-both-button completely when canKeepBoth=false", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ canKeepBoth: false })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("action-keep-both-button")).toBeNull();
  });

  // ── No-capability invariant — no trace when hidden ────────────────────────

  it("when all caps false — no action buttons, no locked icons, no hint elements", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ canConfirm: false, canSupersede: false, canKeepBoth: false })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    // No action buttons at all
    expect(screen.queryByTestId("action-confirm-button")).toBeNull();
    expect(screen.queryByTestId("action-supersede-button")).toBeNull();
    expect(screen.queryByTestId("action-keep-both-button")).toBeNull();
    // No elements containing "locked", "permission", "unauthorized" text
    const root = screen.getByTestId("contradiction-actions-root");
    const text = root.textContent ?? "";
    expect(text.toLowerCase()).not.toContain("locked");
    expect(text.toLowerCase()).not.toContain("permission");
    expect(text.toLowerCase()).not.toContain("unauthorized");
    expect(text.toLowerCase()).not.toContain("you need");
  });

  it("when evidencePath=null — no evidence-path element, no 'hidden' hint", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ evidencePath: null })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("evidence-path")).toBeNull();
    const root = screen.getByTestId("contradiction-actions-root");
    expect((root.textContent ?? "").toLowerCase()).not.toContain("evidence path");
  });

  // ── Action selection ──────────────────────────────────────────────────────

  it("clicking action-confirm-button calls onSelectAction with 'confirm'", () => {
    const onSelect = vi.fn();
    render(() => (
      <ContradictionActions
        state={previewState({ canConfirm: true })}
        selectedAction={null}
        onSelectAction={onSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    screen.getByTestId("action-confirm-button").click();
    expect(onSelect).toHaveBeenCalledWith("confirm");
  });

  it("clicking action-supersede-button calls onSelectAction with 'supersede'", () => {
    const onSelect = vi.fn();
    render(() => (
      <ContradictionActions
        state={previewState({ canSupersede: true })}
        selectedAction={null}
        onSelectAction={onSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    screen.getByTestId("action-supersede-button").click();
    expect(onSelect).toHaveBeenCalledWith("supersede");
  });

  it("clicking action-keep-both-button calls onSelectAction with 'keep-both'", () => {
    const onSelect = vi.fn();
    render(() => (
      <ContradictionActions
        state={previewState({ canKeepBoth: true })}
        selectedAction={null}
        onSelectAction={onSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    screen.getByTestId("action-keep-both-button").click();
    expect(onSelect).toHaveBeenCalledWith("keep-both");
  });

  // ── data-selected attribute ───────────────────────────────────────────────

  it("selected action button has data-selected=true", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ canConfirm: true, canSupersede: true, canKeepBoth: true })}
        selectedAction="confirm"
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(
      screen.getByTestId("action-confirm-button").getAttribute("data-selected")
    ).toBe("true");
  });

  it("non-selected action buttons have data-selected=false", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ canConfirm: true, canSupersede: true, canKeepBoth: true })}
        selectedAction="confirm"
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(
      screen.getByTestId("action-supersede-button").getAttribute("data-selected")
    ).toBe("false");
    expect(
      screen.getByTestId("action-keep-both-button").getAttribute("data-selected")
    ).toBe("false");
  });

  it("all buttons have data-selected=false when selectedAction=null", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ canConfirm: true, canSupersede: true, canKeepBoth: true })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(
      screen.getByTestId("action-confirm-button").getAttribute("data-selected")
    ).toBe("false");
    expect(
      screen.getByTestId("action-supersede-button").getAttribute("data-selected")
    ).toBe("false");
    expect(
      screen.getByTestId("action-keep-both-button").getAttribute("data-selected")
    ).toBe("false");
  });

  // ── Commit / Cancel gating ────────────────────────────────────────────────

  it("commit button disabled when selectedAction=null", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ isStale: false })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const btn = screen.getByTestId("contradiction-commit-button") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("commit button disabled when isStale=true even with selectedAction set", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ isStale: true })}
        selectedAction="confirm"
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const btn = screen.getByTestId("contradiction-commit-button") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("commit button enabled when selectedAction set and not stale", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ isStale: false })}
        selectedAction="supersede"
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const btn = screen.getByTestId("contradiction-commit-button") as HTMLButtonElement;
    expect(btn.disabled).toBe(false);
  });

  it("commit button calls onCommit when clicked and enabled", () => {
    const onCommit = vi.fn();
    render(() => (
      <ContradictionActions
        state={previewState({ isStale: false })}
        selectedAction="keep-both"
        onSelectAction={noOpSelect}
        onCommit={onCommit}
        onCancel={noOp}
      />
    ));
    screen.getByTestId("contradiction-commit-button").click();
    expect(onCommit).toHaveBeenCalledOnce();
  });

  it("cancel button calls onCancel", () => {
    const onCancel = vi.fn();
    render(() => (
      <ContradictionActions
        state={previewState()}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={onCancel}
      />
    ));
    screen.getByTestId("contradiction-cancel-button").click();
    expect(onCancel).toHaveBeenCalledOnce();
  });

  // ── supersededBy ──────────────────────────────────────────────────────────

  it("shows superseded-by when supersededBy='a'", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ supersededBy: "a" })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("superseded-by").textContent).toBe("a");
  });

  it("shows superseded-by when supersededBy='b'", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ supersededBy: "b" })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("superseded-by").textContent).toBe("b");
  });

  it("hides superseded-by when supersededBy is absent", () => {
    render(() => (
      <ContradictionActions
        state={previewState({ supersededBy: undefined })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("superseded-by")).toBeNull();
  });

  // ── Committing phase ──────────────────────────────────────────────────────

  it("committing phase sets data-phase='committing'", () => {
    render(() => (
      <ContradictionActions
        state={committingState("confirm")}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const phase = screen.getByTestId("contradiction-action-phase");
    expect(phase.getAttribute("data-phase")).toBe("committing");
  });

  it("committing phase shows contradiction-committing indicator with role=status", () => {
    render(() => (
      <ContradictionActions
        state={committingState("supersede")}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const el = screen.getByTestId("contradiction-committing");
    expect(el.getAttribute("role")).toBe("status");
  });

  // ── Committed phase ───────────────────────────────────────────────────────

  it("committed phase sets data-phase='committed'", () => {
    render(() => (
      <ContradictionActions
        state={committedState()}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const phase = screen.getByTestId("contradiction-action-phase");
    expect(phase.getAttribute("data-phase")).toBe("committed");
  });

  it("committed phase shows contradiction-result-revision", () => {
    render(() => (
      <ContradictionActions
        state={committedState({ newRevision: 55 })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("contradiction-result-revision").textContent).toBe("55");
  });

  it("committed phase shows contradiction-result-audit-id", () => {
    render(() => (
      <ContradictionActions
        state={committedState({ auditRecordId: "audit-con-xyz-789" })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("contradiction-result-audit-id").textContent).toBe(
      "audit-con-xyz-789"
    );
  });

  it("committed phase shows contradiction-result-description", () => {
    render(() => (
      <ContradictionActions
        state={committedState({ description: "Contradiction marked as resolved via supersede." })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("contradiction-result-description").textContent).toBe(
      "Contradiction marked as resolved via supersede."
    );
  });

  it("committed phase shows contradiction-result-kind", () => {
    render(() => (
      <ContradictionActions
        state={committedState({ kind: "keep-both" })}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("contradiction-result-kind").textContent).toBe("keep-both");
  });

  // ── Error phase ───────────────────────────────────────────────────────────

  it("error phase sets data-phase='error'", () => {
    render(() => (
      <ContradictionActions
        state={errorState("Something failed", false)}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const phase = screen.getByTestId("contradiction-action-phase");
    expect(phase.getAttribute("data-phase")).toBe("error");
  });

  it("error phase shows contradiction-error with role=alert", () => {
    render(() => (
      <ContradictionActions
        state={errorState("Commit conflict detected", false)}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const el = screen.getByTestId("contradiction-error");
    expect(el.getAttribute("role")).toBe("alert");
    expect(el.textContent).toBe("Commit conflict detected");
  });

  it("retry button shown when canRetry=true", () => {
    render(() => (
      <ContradictionActions
        state={errorState("Timeout", true)}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("contradiction-retry")).toBeTruthy();
  });

  it("retry button hidden when canRetry=false", () => {
    render(() => (
      <ContradictionActions
        state={errorState("Permanent failure", false)}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("contradiction-retry")).toBeNull();
  });

  it("retry button calls onCommit when clicked", () => {
    const onCommit = vi.fn();
    render(() => (
      <ContradictionActions
        state={errorState("Timeout", true)}
        selectedAction={null}
        onSelectAction={noOpSelect}
        onCommit={onCommit}
        onCancel={noOp}
      />
    ));
    screen.getByTestId("contradiction-retry").click();
    expect(onCommit).toHaveBeenCalledOnce();
  });

});
