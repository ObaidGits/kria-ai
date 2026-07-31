/**
 * DegradationBanner — System-wide degradation messaging for Memory Control Center.
 *
 * Displays offline/embedder-loss/LLM-loss/battery/memory-pressure/thermal/
 * model-pressure conditions across all destinations with preserved capabilities,
 * queued work counts, and recovery actions.
 *
 * Invariants (F4.5 / task 4.5.5):
 * - Root rendered only when isVisible=true AND conditions.length > 0.
 * - All description and recoveryAction text is passed through verbatim from the
 *   backend — the UI never invents copy.
 * - preservedCapabilities are passed through from the backend; the UI never
 *   fabricates local-capability claims.
 * - offline does not disable local FTS/lifecycle/correction — that determination
 *   belongs to the backend via preservedCapabilities.
 * - queuedWorkCount shown to reassure user that queued writes will drain when
 *   pressure lifts; hidden when null.
 * - recoveryAction and recovery button shown only when the backend provides them.
 * - Critical severity: role="alert", aria-live="assertive".
 * - Warning/info severity: role="status", aria-live="polite".
 * - Dismiss button calls onDismiss with the condition kind.
 * - Recovery button calls onRecovery with recoveryTarget.
 *
 * Requirements: MGR-017, MGR-031, MGR-045;
 *   F4.5 — degradation messaging with exact preserved capabilities.
 */
import { For, Show } from "solid-js";

// ─── Types ────────────────────────────────────────────────────────────────────

export type DegradationKind =
  | "offline"
  | "embedder-loss"
  | "llm-loss"
  | "battery"
  | "memory-pressure"
  | "thermal"
  | "model-pressure";

export interface PreservedCapability {
  /** Human-readable capability name, e.g. "local FTS search", "lifecycle", "correction". */
  name: string;
  /** True when this capability is still available despite the degradation condition. */
  isAvailable: boolean;
}

export interface DegradationCondition {
  kind: DegradationKind;
  severity: "info" | "warning" | "critical";
  /** Exact description from backend — never invented by the UI. */
  description: string;
  /** What still works (and what does not) — passed through from backend. */
  preservedCapabilities: PreservedCapability[];
  /** Number of queued write operations awaiting drain, or null when not applicable. */
  queuedWorkCount: number | null;
  /** Exact recovery instruction from backend — null when not applicable. */
  recoveryAction: string | null;
  /** Navigation target for the recovery button — null when there is no navigation. */
  recoveryTarget: string | null;
}

export interface DegradationBannerProps {
  conditions: DegradationCondition[];
  isVisible: boolean;
  onRecovery: (target: string) => void;
  onDismiss: (kind: DegradationKind) => void;
}

// ─── Component ───────────────────────────────────────────────────────────────

export function DegradationBanner(props: DegradationBannerProps) {
  const shouldRender = () => props.isVisible && props.conditions.length > 0;

  return (
    <Show when={shouldRender()}>
      <div data-testid="degradation-banner">
        <For each={props.conditions}>
          {(condition) => {
            const isCritical = () => condition.severity === "critical";
            const hasPreserved = () => condition.preservedCapabilities.length > 0;

            return (
              <div
                data-testid={`degradation-condition-${condition.kind}`}
                data-severity={condition.severity}
                data-kind={condition.kind}
                role={isCritical() ? "alert" : "status"}
                aria-live={isCritical() ? "assertive" : "polite"}
              >
                {/* ── Exact description from backend — never invented ── */}
                <p data-testid={`degradation-description-${condition.kind}`}>
                  {condition.description}
                </p>

                {/* ── Preserved capabilities — only when non-empty ── */}
                <Show when={hasPreserved()}>
                  <ul data-testid={`degradation-preserved-${condition.kind}`}>
                    <For each={condition.preservedCapabilities}>
                      {(cap) => (
                        <li
                          data-testid={`preserved-cap-${cap.name}`}
                          data-available={cap.isAvailable ? "true" : "false"}
                        >
                          {cap.name}
                        </li>
                      )}
                    </For>
                  </ul>
                </Show>

                {/* ── Queued work count — only when non-null ── */}
                <Show when={condition.queuedWorkCount !== null}>
                  <span data-testid={`degradation-queued-${condition.kind}`}>
                    {condition.queuedWorkCount}
                  </span>
                </Show>

                {/* ── Recovery action — only when non-null ── */}
                <Show when={condition.recoveryAction !== null}>
                  <div data-testid={`degradation-recovery-${condition.kind}`}>
                    <span>{condition.recoveryAction}</span>
                    {/* ── Recovery navigation button — only when recoveryTarget provided ── */}
                    <Show when={condition.recoveryTarget !== null}>
                      <button
                        type="button"
                        data-testid={`recovery-btn-${condition.kind}`}
                        onClick={() =>
                          props.onRecovery(condition.recoveryTarget!)
                        }
                      >
                        Go to recovery
                      </button>
                    </Show>
                  </div>
                </Show>

                {/* ── Dismiss button — always shown ── */}
                <button
                  type="button"
                  data-testid={`degradation-dismiss-${condition.kind}`}
                  onClick={() => props.onDismiss(condition.kind)}
                  aria-label={`Dismiss ${condition.kind} alert`}
                >
                  Dismiss
                </button>
              </div>
            );
          }}
        </For>
      </div>
    </Show>
  );
}

export default DegradationBanner;
