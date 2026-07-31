/**
 * RecoveryPanel — Recovery Mode diagnostics and restore/import flow.
 *
 * Renders the full Recovery_Mode UX:
 * - Recovery mode active banner (writes disabled)
 * - Diagnostics section with per-item status, detail, and correctable indicators
 * - Available recovery actions from backend
 * - Local verified restore/import flow (idle → selecting → verifying → verified
 *   → restoring → complete | failed-verification | failed-restore)
 *
 * Invariants (F4.5 / task 4.5.6):
 * - Renders nothing when isRecoveryMode=false.
 * - Recovery_Mode permits only diagnostics and verified recovery; all writes disabled.
 * - failed-verification and failed-restore phases keep the panel in Recovery_Mode —
 *   the recovery-mode-active banner remains visible after failures.
 * - All copy (diagnostic names, details, action names) passed from backend verbatim.
 * - UI never invents copy or infers state.
 *
 * Requirements: MGR-017, MGR-031, MGR-038, MGR-045; F4.5.
 */
import { For, Show } from "solid-js";

// ─── Types ────────────────────────────────────────────────────────────────────

export type DiagnosticStatus = "pass" | "fail" | "pending" | "skipped";

export interface DiagnosticItem {
  id: string;
  name: string;
  status: DiagnosticStatus;
  /** Exact detail string from backend — null when none. */
  detail: string | null;
  /** True when an automatic correction is available. */
  correctable: boolean;
}

export type RestorePhase =
  | { phase: "idle" }
  | { phase: "selecting" }
  | { phase: "verifying" }
  | { phase: "verified"; checksumLabel: string; itemCount: number }
  | { phase: "restoring" }
  | { phase: "complete"; newRevision: number; message: string }
  | { phase: "failed-verification"; reason: string }
  | { phase: "failed-restore"; reason: string };

export interface RecoveryPanelState {
  isRecoveryMode: boolean;
  diagnostics: DiagnosticItem[];
  restorePhase: RestorePhase;
  /** Action names provided by the backend. */
  availableActions: string[];
}

export interface RecoveryPanelProps {
  state: RecoveryPanelState;
  onRunDiagnostics: () => void;
  onSelectRestoreFile: () => void;
  onConfirmRestore: () => void;
  onCancelRestore: () => void;
  onRunAction: (actionName: string) => void;
}

// ─── Component ───────────────────────────────────────────────────────────────

export function RecoveryPanel(props: RecoveryPanelProps) {
  const s = () => props.state;

  return (
    <Show when={s().isRecoveryMode}>
      <div data-testid="recovery-panel">
        {/* ── Recovery Mode active banner — always visible in Recovery_Mode ── */}
        <div data-testid="recovery-mode-active" role="alert" aria-live="assertive">
          Recovery Mode active. All writes are disabled.
        </div>

        {/* ── Diagnostics ───────────────────────────────────────────────── */}
        <section data-testid="diagnostics-section" aria-label="Diagnostics">
          <For each={s().diagnostics}>
            {(item) => (
              <div
                data-testid={`diagnostic-${item.id}`}
                data-status={item.status}
              >
                <span data-testid={`diagnostic-name-${item.id}`}>{item.name}</span>

                {/* Detail — only when non-null */}
                <Show when={item.detail !== null}>
                  <span data-testid={`diagnostic-detail-${item.id}`}>{item.detail}</span>
                </Show>

                {/* Correctable indicator — only when correctable=true */}
                <Show when={item.correctable}>
                  <span data-testid={`diagnostic-correctable-${item.id}`}>
                    Correctable
                  </span>
                </Show>
              </div>
            )}
          </For>

          <button
            type="button"
            data-testid="run-diagnostics-btn"
            onClick={() => props.onRunDiagnostics()}
          >
            Run Diagnostics
          </button>
        </section>

        {/* ── Available actions — only when non-empty ───────────────────── */}
        <Show when={s().availableActions.length > 0}>
          <section data-testid="recovery-actions-section" aria-label="Recovery actions">
            <For each={s().availableActions}>
              {(actionName) => (
                <button
                  type="button"
                  data-testid={`recovery-action-${actionName}`}
                  onClick={() => props.onRunAction(actionName)}
                >
                  {actionName}
                </button>
              )}
            </For>
          </section>
        </Show>

        {/* ── Restore flow ──────────────────────────────────────────────── */}
        <section data-testid="restore-section" aria-label="Restore">
          {/* idle */}
          <Show when={s().restorePhase.phase === "idle"}>
            <button
              type="button"
              data-testid="restore-select-btn"
              onClick={() => props.onSelectRestoreFile()}
            >
              Select restore file…
            </button>
          </Show>

          {/* selecting */}
          <Show when={s().restorePhase.phase === "selecting"}>
            <span data-testid="restore-phase-selecting" role="status" aria-live="polite">
              Browsing for restore file…
            </span>
          </Show>

          {/* verifying */}
          <Show when={s().restorePhase.phase === "verifying"}>
            <span data-testid="restore-phase-verifying" role="status" aria-live="polite">
              Verifying restore file…
            </span>
          </Show>

          {/* verified */}
          <Show when={s().restorePhase.phase === "verified"}>
            {(() => {
              const p = s().restorePhase as Extract<RestorePhase, { phase: "verified" }>;
              return (
                <div data-testid="restore-phase-verified">
                  <span data-testid="restore-checksum-label">{p.checksumLabel}</span>
                  <span data-testid="restore-item-count">{p.itemCount}</span>
                  <button
                    type="button"
                    data-testid="restore-confirm-btn"
                    onClick={() => props.onConfirmRestore()}
                  >
                    Confirm Restore
                  </button>
                  <button
                    type="button"
                    data-testid="restore-cancel-btn"
                    onClick={() => props.onCancelRestore()}
                  >
                    Cancel
                  </button>
                </div>
              );
            })()}
          </Show>

          {/* restoring */}
          <Show when={s().restorePhase.phase === "restoring"}>
            <span data-testid="restore-phase-restoring" role="status" aria-live="polite">
              Restoring…
            </span>
          </Show>

          {/* complete */}
          <Show when={s().restorePhase.phase === "complete"}>
            {(() => {
              const p = s().restorePhase as Extract<RestorePhase, { phase: "complete" }>;
              return (
                <div data-testid="restore-phase-complete">
                  <span data-testid="restore-new-revision">{p.newRevision}</span>
                  <span data-testid="restore-message">{p.message}</span>
                </div>
              );
            })()}
          </Show>

          {/* failed-verification — stays in Recovery_Mode */}
          <Show when={s().restorePhase.phase === "failed-verification"}>
            {(() => {
              const p = s().restorePhase as Extract<RestorePhase, { phase: "failed-verification" }>;
              return (
                <div data-testid="restore-phase-failed-verification" role="alert" aria-live="assertive">
                  <span data-testid="restore-failed-verification-reason">{p.reason}</span>
                </div>
              );
            })()}
          </Show>

          {/* failed-restore — stays in Recovery_Mode */}
          <Show when={s().restorePhase.phase === "failed-restore"}>
            {(() => {
              const p = s().restorePhase as Extract<RestorePhase, { phase: "failed-restore" }>;
              return (
                <div data-testid="restore-phase-failed-restore" role="alert" aria-live="assertive">
                  <span data-testid="restore-failed-restore-reason">{p.reason}</span>
                </div>
              );
            })()}
          </Show>
        </section>
      </div>
    </Show>
  );
}

export default RecoveryPanel;
