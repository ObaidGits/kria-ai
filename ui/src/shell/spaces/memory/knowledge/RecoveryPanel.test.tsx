/**
 * Tests for RecoveryPanel (task 4.5.6).
 *
 * Validates:
 * - Root not rendered when isRecoveryMode=false
 * - Root rendered when isRecoveryMode=true
 * - Recovery mode banner shown with role=alert
 * - Diagnostics section renders
 * - Each diagnostic shows name, data-status attribute
 * - Diagnostic detail shown when non-null; hidden when null
 * - Diagnostic correctable indicator shown when correctable=true; hidden when false
 * - Run diagnostics button calls onRunDiagnostics
 * - Available actions section shown when non-empty; hidden when empty
 * - Each action button calls onRunAction with correct action name
 * - Restore idle phase: select button shown
 * - Selecting/verifying phases show status indicators
 * - Verified phase: shows checksum, itemCount, confirm and cancel buttons
 * - Confirm calls onConfirmRestore; cancel calls onCancelRestore
 * - Restoring phase shows status
 * - Complete phase shows revision and message
 * - Failed verification: role=alert, reason shown, recovery mode banner STILL shown
 * - Failed restore: role=alert, reason shown, recovery mode banner STILL shown
 * - Writes disabled invariant: recovery-mode-active banner always present in recovery mode
 *
 * Requirements: MGR-017, MGR-031, MGR-038, MGR-045; F4.5.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { RecoveryPanel } from "./RecoveryPanel";
import type {
  DiagnosticItem,
  RecoveryPanelProps,
  RecoveryPanelState,
  RestorePhase,
} from "./RecoveryPanel";

afterEach(() => cleanup());

// ─── Fixtures ─────────────────────────────────────────────────────────────────

function makeDiagnostic(overrides: Partial<DiagnosticItem> = {}): DiagnosticItem {
  return {
    id: "db-integrity",
    name: "Database integrity",
    status: "pass",
    detail: null,
    correctable: false,
    ...overrides,
  };
}

function makeState(overrides: Partial<RecoveryPanelState> = {}): RecoveryPanelState {
  return {
    isRecoveryMode: true,
    diagnostics: [],
    restorePhase: { phase: "idle" },
    availableActions: [],
    ...overrides,
  };
}

function renderPanel(stateOverrides: Partial<RecoveryPanelState> = {}, propOverrides: Partial<RecoveryPanelProps> = {}) {
  const defaults: RecoveryPanelProps = {
    state: makeState(stateOverrides),
    onRunDiagnostics: vi.fn(),
    onSelectRestoreFile: vi.fn(),
    onConfirmRestore: vi.fn(),
    onCancelRestore: vi.fn(),
    onRunAction: vi.fn(),
  };
  return render(() => <RecoveryPanel {...defaults} {...propOverrides} />);
}

// ─── Visibility gating ────────────────────────────────────────────────────────

describe("visibility gating", () => {
  it("does not render root when isRecoveryMode=false", () => {
    renderPanel({ isRecoveryMode: false });
    expect(screen.queryByTestId("recovery-panel")).not.toBeInTheDocument();
  });

  it("renders root when isRecoveryMode=true", () => {
    renderPanel({ isRecoveryMode: true });
    expect(screen.getByTestId("recovery-panel")).toBeInTheDocument();
  });
});

// ─── Recovery mode banner ─────────────────────────────────────────────────────

describe("recovery mode banner", () => {
  it("shows recovery-mode-active banner with role=alert", () => {
    renderPanel();
    const banner = screen.getByTestId("recovery-mode-active");
    expect(banner).toBeInTheDocument();
    expect(banner).toHaveAttribute("role", "alert");
  });

  it("banner text states writes are disabled", () => {
    renderPanel();
    expect(screen.getByTestId("recovery-mode-active")).toHaveTextContent(
      "Recovery Mode active. All writes are disabled.",
    );
  });
});

// ─── Diagnostics section ──────────────────────────────────────────────────────

describe("diagnostics section", () => {
  it("renders diagnostics section", () => {
    renderPanel();
    expect(screen.getByTestId("diagnostics-section")).toBeInTheDocument();
  });

  it("renders each diagnostic with correct data-testid and data-status", () => {
    renderPanel({
      diagnostics: [
        makeDiagnostic({ id: "db-check", name: "DB check", status: "pass" }),
      ],
    });
    const el = screen.getByTestId("diagnostic-db-check");
    expect(el).toBeInTheDocument();
    expect(el).toHaveAttribute("data-status", "pass");
  });

  it("renders diagnostic name", () => {
    renderPanel({
      diagnostics: [makeDiagnostic({ id: "fts", name: "FTS index", status: "pending" })],
    });
    expect(screen.getByTestId("diagnostic-name-fts")).toHaveTextContent("FTS index");
  });

  it("shows data-status for all status values", () => {
    const statuses: DiagnosticItem["status"][] = ["pass", "fail", "pending", "skipped"];
    for (const status of statuses) {
      renderPanel({
        diagnostics: [makeDiagnostic({ id: `item-${status}`, status })],
      });
      expect(screen.getByTestId(`diagnostic-item-${status}`)).toHaveAttribute("data-status", status);
      cleanup();
    }
  });

  it("shows diagnostic detail when non-null", () => {
    renderPanel({
      diagnostics: [makeDiagnostic({ id: "schema", detail: "Schema v7 OK" })],
    });
    expect(screen.getByTestId("diagnostic-detail-schema")).toHaveTextContent("Schema v7 OK");
  });

  it("hides diagnostic detail when null", () => {
    renderPanel({
      diagnostics: [makeDiagnostic({ id: "schema", detail: null })],
    });
    expect(screen.queryByTestId("diagnostic-detail-schema")).not.toBeInTheDocument();
  });

  it("shows correctable indicator when correctable=true", () => {
    renderPanel({
      diagnostics: [makeDiagnostic({ id: "idx", correctable: true })],
    });
    expect(screen.getByTestId("diagnostic-correctable-idx")).toBeInTheDocument();
  });

  it("hides correctable indicator when correctable=false", () => {
    renderPanel({
      diagnostics: [makeDiagnostic({ id: "idx", correctable: false })],
    });
    expect(screen.queryByTestId("diagnostic-correctable-idx")).not.toBeInTheDocument();
  });

  it("run diagnostics button is present", () => {
    renderPanel();
    expect(screen.getByTestId("run-diagnostics-btn")).toBeInTheDocument();
  });

  it("run diagnostics button calls onRunDiagnostics", () => {
    const onRunDiagnostics = vi.fn();
    renderPanel({}, { state: makeState(), onRunDiagnostics, onSelectRestoreFile: vi.fn(), onConfirmRestore: vi.fn(), onCancelRestore: vi.fn(), onRunAction: vi.fn() });
    fireEvent.click(screen.getByTestId("run-diagnostics-btn"));
    expect(onRunDiagnostics).toHaveBeenCalledOnce();
  });

  it("renders multiple diagnostics", () => {
    renderPanel({
      diagnostics: [
        makeDiagnostic({ id: "a", name: "Check A", status: "pass" }),
        makeDiagnostic({ id: "b", name: "Check B", status: "fail" }),
        makeDiagnostic({ id: "c", name: "Check C", status: "skipped" }),
      ],
    });
    expect(screen.getByTestId("diagnostic-a")).toBeInTheDocument();
    expect(screen.getByTestId("diagnostic-b")).toBeInTheDocument();
    expect(screen.getByTestId("diagnostic-c")).toBeInTheDocument();
  });
});

// ─── Available actions ────────────────────────────────────────────────────────

describe("available actions", () => {
  it("does not render actions section when availableActions is empty", () => {
    renderPanel({ availableActions: [] });
    expect(screen.queryByTestId("recovery-actions-section")).not.toBeInTheDocument();
  });

  it("renders actions section when availableActions is non-empty", () => {
    renderPanel({ availableActions: ["rebuild-index"] });
    expect(screen.getByTestId("recovery-actions-section")).toBeInTheDocument();
  });

  it("renders each action button with correct data-testid", () => {
    renderPanel({ availableActions: ["rebuild-index", "clear-outbox"] });
    expect(screen.getByTestId("recovery-action-rebuild-index")).toBeInTheDocument();
    expect(screen.getByTestId("recovery-action-clear-outbox")).toBeInTheDocument();
  });

  it("action button calls onRunAction with correct action name", () => {
    const onRunAction = vi.fn();
    renderPanel(
      { availableActions: ["rebuild-index"] },
      { state: makeState({ availableActions: ["rebuild-index"] }), onRunDiagnostics: vi.fn(), onSelectRestoreFile: vi.fn(), onConfirmRestore: vi.fn(), onCancelRestore: vi.fn(), onRunAction },
    );
    fireEvent.click(screen.getByTestId("recovery-action-rebuild-index"));
    expect(onRunAction).toHaveBeenCalledWith("rebuild-index");
  });

  it("each action button passes its own name to onRunAction", () => {
    const onRunAction = vi.fn();
    renderPanel(
      { availableActions: ["rebuild-index", "clear-outbox"] },
      { state: makeState({ availableActions: ["rebuild-index", "clear-outbox"] }), onRunDiagnostics: vi.fn(), onSelectRestoreFile: vi.fn(), onConfirmRestore: vi.fn(), onCancelRestore: vi.fn(), onRunAction },
    );
    fireEvent.click(screen.getByTestId("recovery-action-clear-outbox"));
    expect(onRunAction).toHaveBeenCalledWith("clear-outbox");
    fireEvent.click(screen.getByTestId("recovery-action-rebuild-index"));
    expect(onRunAction).toHaveBeenCalledWith("rebuild-index");
  });
});

// ─── Restore flow ─────────────────────────────────────────────────────────────

describe("restore flow — idle", () => {
  it("renders restore-section", () => {
    renderPanel({ restorePhase: { phase: "idle" } });
    expect(screen.getByTestId("restore-section")).toBeInTheDocument();
  });

  it("shows restore-select-btn in idle phase", () => {
    renderPanel({ restorePhase: { phase: "idle" } });
    expect(screen.getByTestId("restore-select-btn")).toBeInTheDocument();
  });

  it("restore-select-btn calls onSelectRestoreFile", () => {
    const onSelectRestoreFile = vi.fn();
    renderPanel(
      { restorePhase: { phase: "idle" } },
      { state: makeState({ restorePhase: { phase: "idle" } }), onRunDiagnostics: vi.fn(), onSelectRestoreFile, onConfirmRestore: vi.fn(), onCancelRestore: vi.fn(), onRunAction: vi.fn() },
    );
    fireEvent.click(screen.getByTestId("restore-select-btn"));
    expect(onSelectRestoreFile).toHaveBeenCalledOnce();
  });
});

describe("restore flow — selecting", () => {
  it("shows restore-phase-selecting with role=status", () => {
    renderPanel({ restorePhase: { phase: "selecting" } });
    const el = screen.getByTestId("restore-phase-selecting");
    expect(el).toBeInTheDocument();
    expect(el).toHaveAttribute("role", "status");
  });
});

describe("restore flow — verifying", () => {
  it("shows restore-phase-verifying with role=status", () => {
    renderPanel({ restorePhase: { phase: "verifying" } });
    const el = screen.getByTestId("restore-phase-verifying");
    expect(el).toBeInTheDocument();
    expect(el).toHaveAttribute("role", "status");
  });
});

describe("restore flow — verified", () => {
  const verifiedPhase: RestorePhase = {
    phase: "verified",
    checksumLabel: "sha256:abc123",
    itemCount: 42,
  };

  it("shows restore-phase-verified", () => {
    renderPanel({ restorePhase: verifiedPhase });
    expect(screen.getByTestId("restore-phase-verified")).toBeInTheDocument();
  });

  it("shows checksumLabel", () => {
    renderPanel({ restorePhase: verifiedPhase });
    expect(screen.getByTestId("restore-checksum-label")).toHaveTextContent("sha256:abc123");
  });

  it("shows itemCount", () => {
    renderPanel({ restorePhase: verifiedPhase });
    expect(screen.getByTestId("restore-item-count")).toHaveTextContent("42");
  });

  it("shows restore-confirm-btn and restore-cancel-btn", () => {
    renderPanel({ restorePhase: verifiedPhase });
    expect(screen.getByTestId("restore-confirm-btn")).toBeInTheDocument();
    expect(screen.getByTestId("restore-cancel-btn")).toBeInTheDocument();
  });

  it("confirm button calls onConfirmRestore", () => {
    const onConfirmRestore = vi.fn();
    renderPanel(
      { restorePhase: verifiedPhase },
      { state: makeState({ restorePhase: verifiedPhase }), onRunDiagnostics: vi.fn(), onSelectRestoreFile: vi.fn(), onConfirmRestore, onCancelRestore: vi.fn(), onRunAction: vi.fn() },
    );
    fireEvent.click(screen.getByTestId("restore-confirm-btn"));
    expect(onConfirmRestore).toHaveBeenCalledOnce();
  });

  it("cancel button calls onCancelRestore", () => {
    const onCancelRestore = vi.fn();
    renderPanel(
      { restorePhase: verifiedPhase },
      { state: makeState({ restorePhase: verifiedPhase }), onRunDiagnostics: vi.fn(), onSelectRestoreFile: vi.fn(), onConfirmRestore: vi.fn(), onCancelRestore, onRunAction: vi.fn() },
    );
    fireEvent.click(screen.getByTestId("restore-cancel-btn"));
    expect(onCancelRestore).toHaveBeenCalledOnce();
  });
});

describe("restore flow — restoring", () => {
  it("shows restore-phase-restoring with role=status", () => {
    renderPanel({ restorePhase: { phase: "restoring" } });
    const el = screen.getByTestId("restore-phase-restoring");
    expect(el).toBeInTheDocument();
    expect(el).toHaveAttribute("role", "status");
  });
});

describe("restore flow — complete", () => {
  const completePhase: RestorePhase = {
    phase: "complete",
    newRevision: 99,
    message: "Restore completed successfully.",
  };

  it("shows restore-phase-complete", () => {
    renderPanel({ restorePhase: completePhase });
    expect(screen.getByTestId("restore-phase-complete")).toBeInTheDocument();
  });

  it("shows newRevision", () => {
    renderPanel({ restorePhase: completePhase });
    expect(screen.getByTestId("restore-new-revision")).toHaveTextContent("99");
  });

  it("shows message", () => {
    renderPanel({ restorePhase: completePhase });
    expect(screen.getByTestId("restore-message")).toHaveTextContent(
      "Restore completed successfully.",
    );
  });
});

// ─── Failed phases — critical invariant: stays in Recovery_Mode ──────────────

describe("restore flow — failed-verification", () => {
  const failedVerPhase: RestorePhase = {
    phase: "failed-verification",
    reason: "Checksum mismatch: expected abc, got xyz.",
  };

  it("shows restore-phase-failed-verification with role=alert", () => {
    renderPanel({ restorePhase: failedVerPhase });
    const el = screen.getByTestId("restore-phase-failed-verification");
    expect(el).toBeInTheDocument();
    expect(el).toHaveAttribute("role", "alert");
  });

  it("shows the failure reason", () => {
    renderPanel({ restorePhase: failedVerPhase });
    expect(screen.getByTestId("restore-failed-verification-reason")).toHaveTextContent(
      "Checksum mismatch: expected abc, got xyz.",
    );
  });

  it("recovery-mode-active banner is STILL shown after failed-verification", () => {
    renderPanel({ restorePhase: failedVerPhase });
    expect(screen.getByTestId("recovery-mode-active")).toBeInTheDocument();
    expect(screen.getByTestId("recovery-mode-active")).toHaveAttribute("role", "alert");
  });

  it("recovery-panel root is still rendered after failed-verification", () => {
    renderPanel({ restorePhase: failedVerPhase });
    expect(screen.getByTestId("recovery-panel")).toBeInTheDocument();
  });
});

describe("restore flow — failed-restore", () => {
  const failedRestorePhase: RestorePhase = {
    phase: "failed-restore",
    reason: "Restore aborted: schema incompatibility detected.",
  };

  it("shows restore-phase-failed-restore with role=alert", () => {
    renderPanel({ restorePhase: failedRestorePhase });
    const el = screen.getByTestId("restore-phase-failed-restore");
    expect(el).toBeInTheDocument();
    expect(el).toHaveAttribute("role", "alert");
  });

  it("shows the failure reason", () => {
    renderPanel({ restorePhase: failedRestorePhase });
    expect(screen.getByTestId("restore-failed-restore-reason")).toHaveTextContent(
      "Restore aborted: schema incompatibility detected.",
    );
  });

  it("recovery-mode-active banner is STILL shown after failed-restore", () => {
    renderPanel({ restorePhase: failedRestorePhase });
    expect(screen.getByTestId("recovery-mode-active")).toBeInTheDocument();
    expect(screen.getByTestId("recovery-mode-active")).toHaveAttribute("role", "alert");
  });

  it("recovery-panel root is still rendered after failed-restore", () => {
    renderPanel({ restorePhase: failedRestorePhase });
    expect(screen.getByTestId("recovery-panel")).toBeInTheDocument();
  });
});

// ─── Writes disabled invariant ────────────────────────────────────────────────

describe("writes disabled invariant", () => {
  const allPhases: RestorePhase[] = [
    { phase: "idle" },
    { phase: "selecting" },
    { phase: "verifying" },
    { phase: "verified", checksumLabel: "sha256:aaa", itemCount: 1 },
    { phase: "restoring" },
    { phase: "complete", newRevision: 1, message: "Done." },
    { phase: "failed-verification", reason: "bad checksum" },
    { phase: "failed-restore", reason: "schema error" },
  ];

  for (const phase of allPhases) {
    it(`recovery-mode-active banner present during phase="${phase.phase}"`, () => {
      renderPanel({ restorePhase: phase });
      expect(screen.getByTestId("recovery-mode-active")).toBeInTheDocument();
      cleanup();
    });
  }
});
