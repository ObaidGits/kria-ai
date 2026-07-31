/**
 * Tests for LifecycleActions component.
 *
 * Verifies all rendering requirements for the Forget/Restore/Hard Delete
 * lifecycle workflow, including the critical crypto invariants:
 *   - "Crypto-shredded" text NEVER appears when cryptoProof=null
 *   - "Cryptographically erased" text NEVER appears when cryptoProof=null
 *
 * Requirements: F4.4 (task 4.4.8)
 */
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@solidjs/testing-library";
import { LifecycleActions } from "./LifecycleActions";
import type {
  LifecycleActionPhase,
  LifecyclePreview,
  LifecycleResult,
  DependencyItem,
  ReconciliationStatus,
  CryptoState,
  LifecycleActionKind,
} from "./LifecycleActions";

// ─── Helpers ──────────────────────────────────────────────────────────────────

function makeDep(overrides: Partial<DependencyItem> = {}): DependencyItem {
  return {
    itemId: "dep-001",
    label: "Dependent Item",
    kind: "memory",
    choice: null,
    ...overrides,
  };
}

function makeReconciliation(overrides: Partial<ReconciliationStatus> = {}): ReconciliationStatus {
  return {
    indexName: "fts5",
    status: "pending",
    ...overrides,
  };
}

function makeCryptoState(overrides: Partial<CryptoState> = {}): CryptoState {
  return {
    hasCryptoCapability: true,
    cryptoProof: null,
    pendingErasure: false,
    ...overrides,
  };
}

function makePreview(
  kind: LifecycleActionKind,
  overrides: Partial<LifecyclePreview> = {},
): LifecyclePreview {
  return {
    kind,
    itemId: "item-001",
    itemLabel: "My Memory Item",
    description: "This item will be forgotten.",
    policyLabel: "governed:lifecycle:forget",
    baseRevision: 10,
    isStale: false,
    dependencies: [],
    restoreWindowDays: kind === "forget" ? 30 : null,
    restoreUntil: kind === "forget" ? "2025-09-01T00:00:00Z" : null,
    cryptoState: kind === "hard-delete" ? makeCryptoState() : null,
    reconciliationStatuses: [],
    ...overrides,
  };
}

function makeResult(
  kind: LifecycleActionKind,
  overrides: Partial<LifecycleResult> = {},
): LifecycleResult {
  return {
    kind,
    newRevision: 11,
    auditRecordId: "audit-life-001",
    description: "Action completed.",
    restoreUntil: kind === "forget" ? "2025-09-01T00:00:00Z" : null,
    cryptoState: kind === "hard-delete" ? makeCryptoState() : null,
    reconciliationStatuses: [],
    ...overrides,
  };
}

function previewState(
  kind: LifecycleActionKind,
  overrides: Partial<LifecyclePreview> = {},
): LifecycleActionPhase {
  return { phase: "preview", action: makePreview(kind, overrides) };
}

function committedState(
  kind: LifecycleActionKind,
  overrides: Partial<LifecycleResult> = {},
): LifecycleActionPhase {
  return { phase: "committed", result: makeResult(kind, overrides) };
}

const idleState: LifecycleActionPhase = { phase: "idle" };
const committingState: LifecycleActionPhase = { phase: "committing" };

function errorState(message: string, canRetry: boolean): LifecycleActionPhase {
  return { phase: "error", message, canRetry };
}

const noOp = () => {};
const noOpChoice = (_id: string, _c: "cascade" | "keep-independent-evidence") => {};

// ─── Tests ────────────────────────────────────────────────────────────────────

describe("LifecycleActions", () => {

  // ── Root element ──────────────────────────────────────────────────────────

  it("renders root element with correct testid", () => {
    render(() => (
      <LifecycleActions
        state={idleState}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("lifecycle-actions-root")).toBeTruthy();
  });

  // ── Idle phase ────────────────────────────────────────────────────────────

  it("idle phase sets data-phase='idle'", () => {
    render(() => (
      <LifecycleActions
        state={idleState}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const phase = screen.getByTestId("lifecycle-action-phase");
    expect(phase.getAttribute("data-phase")).toBe("idle");
  });

  it("idle phase renders no preview content", () => {
    render(() => (
      <LifecycleActions
        state={idleState}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("lifecycle-action-kind")).toBeNull();
    expect(screen.queryByTestId("lifecycle-item-label")).toBeNull();
    expect(screen.queryByTestId("lifecycle-commit-button")).toBeNull();
  });

  // ── Preview phase — common fields ─────────────────────────────────────────

  it("preview phase sets data-phase='preview'", () => {
    render(() => (
      <LifecycleActions
        state={previewState("forget")}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("lifecycle-action-phase").getAttribute("data-phase")).toBe("preview");
  });

  it("shows lifecycle-action-kind for forget", () => {
    render(() => (
      <LifecycleActions
        state={previewState("forget")}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("lifecycle-action-kind").textContent).toBe("forget");
  });

  it("shows lifecycle-action-kind for restore", () => {
    render(() => (
      <LifecycleActions
        state={previewState("restore")}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("lifecycle-action-kind").textContent).toBe("restore");
  });

  it("shows lifecycle-action-kind for hard-delete", () => {
    render(() => (
      <LifecycleActions
        state={previewState("hard-delete")}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("lifecycle-action-kind").textContent).toBe("hard-delete");
  });

  it("shows lifecycle-item-label", () => {
    render(() => (
      <LifecycleActions
        state={previewState("forget", { itemLabel: "The Label" })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("lifecycle-item-label").textContent).toBe("The Label");
  });

  it("shows lifecycle-description from backend", () => {
    render(() => (
      <LifecycleActions
        state={previewState("forget", { description: "Backend-provided description." })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("lifecycle-description").textContent).toBe(
      "Backend-provided description."
    );
  });

  it("shows lifecycle-policy-label", () => {
    render(() => (
      <LifecycleActions
        state={previewState("forget", { policyLabel: "governed:lifecycle:forget" })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("lifecycle-policy-label").textContent).toBe(
      "governed:lifecycle:forget"
    );
  });

  it("shows lifecycle-base-revision", () => {
    render(() => (
      <LifecycleActions
        state={previewState("forget", { baseRevision: 42 })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("lifecycle-base-revision").textContent).toBe("42");
  });

  // ── Stale warning ─────────────────────────────────────────────────────────

  it("shows lifecycle-stale-warning with role=alert when isStale=true", () => {
    render(() => (
      <LifecycleActions
        state={previewState("forget", { isStale: true })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const el = screen.getByTestId("lifecycle-stale-warning");
    expect(el.getAttribute("role")).toBe("alert");
  });

  it("hides lifecycle-stale-warning when isStale=false", () => {
    render(() => (
      <LifecycleActions
        state={previewState("forget", { isStale: false })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("lifecycle-stale-warning")).toBeNull();
  });

  // ── Dependencies list ─────────────────────────────────────────────────────

  it("shows dependencies-list when dependencies non-empty", () => {
    render(() => (
      <LifecycleActions
        state={previewState("forget", { dependencies: [makeDep()] })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("dependencies-list")).toBeTruthy();
  });

  it("hides dependencies-list when dependencies empty", () => {
    render(() => (
      <LifecycleActions
        state={previewState("forget", { dependencies: [] })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("dependencies-list")).toBeNull();
  });

  it("each dependency renders label, kind, cascade and keep buttons", () => {
    const dep = makeDep({ itemId: "dep-abc", label: "Linked Memory", kind: "summary" });
    render(() => (
      <LifecycleActions
        state={previewState("forget", { dependencies: [dep] })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const depEl = screen.getByTestId("dependency-dep-abc");
    expect(depEl.querySelector('[data-field="dep-label"]')?.textContent).toBe("Linked Memory");
    expect(depEl.querySelector('[data-field="dep-kind"]')?.textContent).toBe("summary");
    expect(screen.getByTestId("dep-cascade-dep-abc")).toBeTruthy();
    expect(screen.getByTestId("dep-keep-dep-abc")).toBeTruthy();
  });

  it("dependency data-selected-choice is 'none' when choice is null", () => {
    const dep = makeDep({ itemId: "dep-001", choice: null });
    render(() => (
      <LifecycleActions
        state={previewState("forget", { dependencies: [dep] })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const depEl = screen.getByTestId("dependency-dep-001");
    expect(depEl.getAttribute("data-selected-choice")).toBe("none");
  });

  it("dependency data-selected-choice reflects 'cascade'", () => {
    const dep = makeDep({ itemId: "dep-001", choice: "cascade" });
    render(() => (
      <LifecycleActions
        state={previewState("forget", { dependencies: [dep] })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("dependency-dep-001").getAttribute("data-selected-choice")).toBe("cascade");
  });

  it("dependency data-selected-choice reflects 'keep-independent-evidence'", () => {
    const dep = makeDep({ itemId: "dep-001", choice: "keep-independent-evidence" });
    render(() => (
      <LifecycleActions
        state={previewState("forget", { dependencies: [dep] })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("dependency-dep-001").getAttribute("data-selected-choice")).toBe(
      "keep-independent-evidence"
    );
  });

  it("clicking cascade button calls onDependencyChoice with correct args", () => {
    const onChoice = vi.fn();
    const dep = makeDep({ itemId: "dep-xyz", choice: null });
    render(() => (
      <LifecycleActions
        state={previewState("forget", { dependencies: [dep] })}
        onDependencyChoice={onChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    screen.getByTestId("dep-cascade-dep-xyz").click();
    expect(onChoice).toHaveBeenCalledWith("dep-xyz", "cascade");
  });

  it("clicking keep button calls onDependencyChoice with correct args", () => {
    const onChoice = vi.fn();
    const dep = makeDep({ itemId: "dep-xyz", choice: null });
    render(() => (
      <LifecycleActions
        state={previewState("forget", { dependencies: [dep] })}
        onDependencyChoice={onChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    screen.getByTestId("dep-keep-dep-xyz").click();
    expect(onChoice).toHaveBeenCalledWith("dep-xyz", "keep-independent-evidence");
  });

  // ── Commit button gating ──────────────────────────────────────────────────

  it("commit button disabled when a dependency has null choice", () => {
    const dep = makeDep({ itemId: "dep-001", choice: null });
    render(() => (
      <LifecycleActions
        state={previewState("forget", { dependencies: [dep], isStale: false })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const btn = screen.getByTestId("lifecycle-commit-button") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("commit button enabled when all deps have choices and not stale", () => {
    const dep = makeDep({ itemId: "dep-001", choice: "cascade" });
    render(() => (
      <LifecycleActions
        state={previewState("forget", { dependencies: [dep], isStale: false })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const btn = screen.getByTestId("lifecycle-commit-button") as HTMLButtonElement;
    expect(btn.disabled).toBe(false);
  });

  it("commit button disabled when isStale=true", () => {
    render(() => (
      <LifecycleActions
        state={previewState("forget", { isStale: true })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const btn = screen.getByTestId("lifecycle-commit-button") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("commit button enabled when no deps and not stale", () => {
    render(() => (
      <LifecycleActions
        state={previewState("forget", { dependencies: [], isStale: false })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const btn = screen.getByTestId("lifecycle-commit-button") as HTMLButtonElement;
    expect(btn.disabled).toBe(false);
  });

  it("restore: commit enabled even when deps present (restore never requires choices)", () => {
    const dep = makeDep({ itemId: "dep-001", choice: null });
    render(() => (
      <LifecycleActions
        state={previewState("restore", { dependencies: [dep], isStale: false })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const btn = screen.getByTestId("lifecycle-commit-button") as HTMLButtonElement;
    expect(btn.disabled).toBe(false);
  });

  it("commit button calls onCommit when enabled and clicked", () => {
    const onCommit = vi.fn();
    render(() => (
      <LifecycleActions
        state={previewState("forget", { dependencies: [], isStale: false })}
        onDependencyChoice={noOpChoice}
        onCommit={onCommit}
        onCancel={noOp}
      />
    ));
    screen.getByTestId("lifecycle-commit-button").click();
    expect(onCommit).toHaveBeenCalledOnce();
  });

  it("cancel button calls onCancel", () => {
    const onCancel = vi.fn();
    render(() => (
      <LifecycleActions
        state={previewState("forget")}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={onCancel}
      />
    ));
    screen.getByTestId("lifecycle-cancel-button").click();
    expect(onCancel).toHaveBeenCalledOnce();
  });

  // ── Forget-specific: restore window ───────────────────────────────────────

  it("forget: shows restore-window with restoreWindowDays", () => {
    render(() => (
      <LifecycleActions
        state={previewState("forget", { restoreWindowDays: 30 })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("restore-window").textContent).toBe("30");
  });

  it("forget: shows restore-until with ISO timestamp", () => {
    render(() => (
      <LifecycleActions
        state={previewState("forget", { restoreUntil: "2025-09-01T00:00:00Z" })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("restore-until").textContent).toBe("2025-09-01T00:00:00Z");
  });

  it("forget: hides restore-window when restoreWindowDays=null", () => {
    render(() => (
      <LifecycleActions
        state={previewState("forget", { restoreWindowDays: null })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("restore-window")).toBeNull();
  });

  it("forget: hides restore-until when restoreUntil=null", () => {
    render(() => (
      <LifecycleActions
        state={previewState("forget", { restoreUntil: null })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("restore-until")).toBeNull();
  });

  // ── Hard-delete crypto state ──────────────────────────────────────────────

  it("hard-delete: renders crypto-capability with data-has-capability=true", () => {
    render(() => (
      <LifecycleActions
        state={previewState("hard-delete", {
          cryptoState: makeCryptoState({ hasCryptoCapability: true }),
        })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const el = screen.getByTestId("crypto-capability");
    expect(el.getAttribute("data-has-capability")).toBe("true");
  });

  it("hard-delete: renders crypto-capability with data-has-capability=false", () => {
    render(() => (
      <LifecycleActions
        state={previewState("hard-delete", {
          cryptoState: makeCryptoState({ hasCryptoCapability: false, cryptoProof: null }),
        })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const el = screen.getByTestId("crypto-capability");
    expect(el.getAttribute("data-has-capability")).toBe("false");
  });

  it("hard-delete: shows crypto-proof-confirmed when cryptoProof non-null", () => {
    render(() => (
      <LifecycleActions
        state={previewState("hard-delete", {
          cryptoState: makeCryptoState({ cryptoProof: "sha256:abcdef1234567890" }),
        })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("crypto-proof-confirmed")).toBeTruthy();
  });

  it("hard-delete: hides crypto-proof-confirmed when cryptoProof=null", () => {
    render(() => (
      <LifecycleActions
        state={previewState("hard-delete", {
          cryptoState: makeCryptoState({ cryptoProof: null }),
        })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("crypto-proof-confirmed")).toBeNull();
  });

  it("hard-delete: shows crypto-pending when pendingErasure=true and cryptoProof=null", () => {
    render(() => (
      <LifecycleActions
        state={previewState("hard-delete", {
          cryptoState: makeCryptoState({ pendingErasure: true, cryptoProof: null }),
        })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("crypto-pending")).toBeTruthy();
    expect(screen.getByTestId("crypto-pending").textContent).toContain("Content deletion in progress");
  });

  it("hard-delete: shows crypto-unavailable when !hasCryptoCapability and cryptoProof=null", () => {
    render(() => (
      <LifecycleActions
        state={previewState("hard-delete", {
          cryptoState: makeCryptoState({ hasCryptoCapability: false, cryptoProof: null }),
        })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const el = screen.getByTestId("crypto-unavailable");
    expect(el.textContent).toContain("Hard Delete (content marked for removal)");
  });

  // ── CRITICAL crypto invariants ────────────────────────────────────────────

  it("CRITICAL: text 'Crypto-shredded' never appears when cryptoProof=null", () => {
    render(() => (
      <LifecycleActions
        state={previewState("hard-delete", {
          cryptoState: makeCryptoState({
            hasCryptoCapability: true,
            cryptoProof: null,
            pendingErasure: false,
          }),
        })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const root = screen.getByTestId("lifecycle-actions-root");
    expect(root.textContent ?? "").not.toContain("Crypto-shredded");
  });

  it("CRITICAL: text 'Cryptographically erased' never appears when cryptoProof=null", () => {
    render(() => (
      <LifecycleActions
        state={previewState("hard-delete", {
          cryptoState: makeCryptoState({
            hasCryptoCapability: true,
            cryptoProof: null,
            pendingErasure: false,
          }),
        })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const root = screen.getByTestId("lifecycle-actions-root");
    expect(root.textContent ?? "").not.toContain("Cryptographically erased");
  });

  it("CRITICAL: crypto-unavailable text never says 'Crypto-shredded'", () => {
    render(() => (
      <LifecycleActions
        state={previewState("hard-delete", {
          cryptoState: makeCryptoState({ hasCryptoCapability: false, cryptoProof: null }),
        })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const root = screen.getByTestId("lifecycle-actions-root");
    expect(root.textContent ?? "").not.toContain("Crypto-shredded");
    expect(root.textContent ?? "").not.toContain("Cryptographically erased");
  });

  // ── Reconciliation list ───────────────────────────────────────────────────

  it("shows reconciliation-list when non-empty", () => {
    const statuses = [
      makeReconciliation({ indexName: "fts5", status: "pending" }),
      makeReconciliation({ indexName: "vector", status: "complete" }),
    ];
    render(() => (
      <LifecycleActions
        state={previewState("hard-delete", { reconciliationStatuses: statuses })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("reconciliation-list")).toBeTruthy();
  });

  it("hides reconciliation-list when empty", () => {
    render(() => (
      <LifecycleActions
        state={previewState("hard-delete", { reconciliationStatuses: [] })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("reconciliation-list")).toBeNull();
  });

  it("each reconciliation item has correct data-status", () => {
    const statuses = [
      makeReconciliation({ indexName: "fts5", status: "in-progress" }),
      makeReconciliation({ indexName: "graph", status: "failed" }),
    ];
    render(() => (
      <LifecycleActions
        state={previewState("hard-delete", { reconciliationStatuses: statuses })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const fts5 = screen.getByTestId("reconciliation-fts5");
    expect(fts5.getAttribute("data-status")).toBe("in-progress");
    const graph = screen.getByTestId("reconciliation-graph");
    expect(graph.getAttribute("data-status")).toBe("failed");
  });

  // ── Committed phase ───────────────────────────────────────────────────────

  it("committed phase sets data-phase='committed'", () => {
    render(() => (
      <LifecycleActions
        state={committedState("forget")}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("lifecycle-action-phase").getAttribute("data-phase")).toBe("committed");
  });

  it("committed phase shows lifecycle-result-revision", () => {
    render(() => (
      <LifecycleActions
        state={committedState("forget", { newRevision: 99 })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("lifecycle-result-revision").textContent).toBe("99");
  });

  it("committed phase shows lifecycle-result-audit-id", () => {
    render(() => (
      <LifecycleActions
        state={committedState("forget", { auditRecordId: "audit-life-xyz" })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("lifecycle-result-audit-id").textContent).toBe("audit-life-xyz");
  });

  it("committed phase shows lifecycle-result-description", () => {
    render(() => (
      <LifecycleActions
        state={committedState("forget", { description: "Item successfully forgotten." })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("lifecycle-result-description").textContent).toBe(
      "Item successfully forgotten."
    );
  });

  it("committed phase shows lifecycle-result-restore-until when non-null", () => {
    render(() => (
      <LifecycleActions
        state={committedState("forget", { restoreUntil: "2025-09-01T00:00:00Z" })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("lifecycle-result-restore-until").textContent).toBe(
      "2025-09-01T00:00:00Z"
    );
  });

  it("committed phase hides lifecycle-result-restore-until when null", () => {
    render(() => (
      <LifecycleActions
        state={committedState("hard-delete", { restoreUntil: null })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("lifecycle-result-restore-until")).toBeNull();
  });

  it("committed phase reconciliation list shows items", () => {
    const statuses = [makeReconciliation({ indexName: "cache", status: "complete" })];
    render(() => (
      <LifecycleActions
        state={committedState("hard-delete", { reconciliationStatuses: statuses })}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("lifecycle-result-reconciliation-list")).toBeTruthy();
    expect(screen.getByTestId("reconciliation-cache").getAttribute("data-status")).toBe("complete");
  });

  // ── Error phase ───────────────────────────────────────────────────────────

  it("error phase sets data-phase='error'", () => {
    render(() => (
      <LifecycleActions
        state={errorState("Something failed", false)}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("lifecycle-action-phase").getAttribute("data-phase")).toBe("error");
  });

  it("error phase shows lifecycle-error with role=alert", () => {
    render(() => (
      <LifecycleActions
        state={errorState("Commit conflict", false)}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    const el = screen.getByTestId("lifecycle-error");
    expect(el.getAttribute("role")).toBe("alert");
    expect(el.textContent).toBe("Commit conflict");
  });

  it("error phase shows lifecycle-retry when canRetry=true", () => {
    render(() => (
      <LifecycleActions
        state={errorState("Timeout", true)}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.getByTestId("lifecycle-retry")).toBeTruthy();
  });

  it("error phase hides lifecycle-retry when canRetry=false", () => {
    render(() => (
      <LifecycleActions
        state={errorState("Permanent failure", false)}
        onDependencyChoice={noOpChoice}
        onCommit={noOp}
        onCancel={noOp}
      />
    ));
    expect(screen.queryByTestId("lifecycle-retry")).toBeNull();
  });

  it("retry button calls onCommit when clicked", () => {
    const onCommit = vi.fn();
    render(() => (
      <LifecycleActions
        state={errorState("Timeout", true)}
        onDependencyChoice={noOpChoice}
        onCommit={onCommit}
        onCancel={noOp}
      />
    ));
    screen.getByTestId("lifecycle-retry").click();
    expect(onCommit).toHaveBeenCalledOnce();
  });

});
