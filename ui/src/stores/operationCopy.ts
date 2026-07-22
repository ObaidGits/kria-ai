/**
 * Operation copy — the PURE, READ-ONLY presentation layer for the ONE shared
 * operation-state vocabulary (UIE-M-013; Req 13.1–13.5; design §17).
 *
 * WHY THIS EXISTS: sub-task 12.2 (`operationState.ts`) DEFINES the vocabulary
 * ({@link OperationState} / {@link OperationSnapshot}) but explicitly does NOT
 * wire presentation copy — that is THIS sub-task (12.6). Today generic
 * "Loading…" / "Waiting…" copy names no operation and no next action (gap G5).
 * This module maps an authoritative {@link OperationSnapshot} plus the operation
 * name into concise, localized copy that:
 *
 *   • NAMES the operation and, where relevant, the next action (Req 13.1/13.2).
 *   • Surfaces a determinate percentage ONLY when the snapshot already carries a
 *     measured `progress` (never a fabricated percentage — UIE-M-013).
 *   • OMITS the source-owned message when the source provides none (mirrors the
 *     {@link OperationSnapshot} omission discipline — never invents a cause).
 *   • Returns `null` for `empty` so a recovered/settled surface CLEARS its stale
 *     loading/error copy instead of lingering (Req 13.5 stale-state clear).
 *   • Emits `recovered` copy under a STABLE key so a live region can announce
 *     restoration exactly once (Req 13.5), the same key-identity de-duplication
 *     `coreNarration` already relies on.
 *
 * This module is intentionally NOT a store and owns NO runtime lifecycle: no
 * signals, no setters, no side effects, no timers, no backend calls. It is a
 * pure function of its input, mirroring `coreNarration.ts`. Cancellation SCOPE
 * copy (Stop naming) is a SEPARATE concern owned by UIE-M-015 / sub-task 12.7 —
 * this module never labels a Stop control.
 *
 * Requirements: 13.1, 13.2, 13.5; design §17, §20.1; UIE-M-013.
 */
import type { OperationSnapshot, OperationState } from "./operationState";
import { isAttentionOperation } from "./operationState";
import { t } from "./i18n";

// ─── Types ───────────────────────────────────────────────────────────────────

/** A concise situational copy line derived from an operation snapshot. */
export interface OperationCopy {
  /** The concise, localized text to display / announce. Never empty. */
  readonly text: string;
  /**
   * The i18n key the text came from. Stable per semantic meaning so a consumer
   * can de-duplicate an unchanged announcement by identity (Req 13.5 / 17.5:
   * announce a milestone once, not every reactive tick).
   */
  readonly key: string;
  /**
   * True when the copy names/points at a next action or owner (failed → retry,
   * blocked → approval, unavailable → the offline service, recovered → restored).
   * Reuses {@link isAttentionOperation} plus the transient recovery states.
   */
  readonly actionable: boolean;
}

/** Optional presentation hints a surface may pass alongside the snapshot. */
export interface OperationCopyOptions {
  /**
   * Human-readable operation name (e.g. a Space label, "settings", "tools").
   * When present the copy NAMES the operation; when absent an objectless but
   * still-truthful phrase is used (never a fabricated name).
   */
  readonly operation?: string | null;
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

function nonEmpty(value: string | undefined | null): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : undefined;
}

/** Interpolate `{name}` placeholders (mirrors `coreNarration.fill`). */
function fill(key: string, values: Record<string, string>): string {
  return Object.entries(values).reduce(
    (copy, [name, value]) => copy.replaceAll(`{${name}}`, value),
    t(key),
  );
}

/**
 * Format a measured [0,1] progress value as an integer percent string. The
 * caller guarantees the value is already normalized (present only when the
 * source measured it), so no fabrication happens here.
 */
function percent(progress: number): string {
  return String(Math.round(progress * 100));
}

/** States that carry a next action / owner pointer beyond the attention set. */
const RECOVERY_ACTIONABLE: ReadonlySet<OperationState> = new Set([
  "retrying",
  "recovered",
]);

// ─── Pure mapping ────────────────────────────────────────────────────────────

/**
 * Pure projection: an {@link OperationSnapshot} (+ optional operation name) →
 * {@link OperationCopy} | null.
 *
 * Deterministic and side-effect free. Returns `null` for `empty` (no operation
 * → no copy, so stale loading/error text is cleared). Every produced line names
 * the operation when a name is supplied, attaches a determinate percentage only
 * when the snapshot measured `progress`, and surfaces the source-owned `message`
 * only for the states that own one (never fabricated).
 */
export function describeOperation(
  snapshot: OperationSnapshot,
  options: OperationCopyOptions = {},
): OperationCopy | null {
  const operation = nonEmpty(options.operation);
  const message = nonEmpty(snapshot.message);
  const named = operation !== undefined;
  const actionable =
    isAttentionOperation(snapshot.state) || RECOVERY_ACTIONABLE.has(snapshot.state);

  switch (snapshot.state) {
    case "empty":
      // No operation → OMIT (clears stale copy; Req 13.5). Never fabricate.
      return null;

    case "loading": {
      // Determinate progress ONLY when the source measured it (UIE-M-013).
      if (snapshot.progress !== undefined) {
        const pct = percent(snapshot.progress);
        return named
          ? { text: fill("operation_copy_loading_progress", { operation: operation!, percent: pct }), key: "operation_copy_loading_progress", actionable }
          : { text: fill("operation_copy_loading_progress_unnamed", { percent: pct }), key: "operation_copy_loading_progress_unnamed", actionable };
      }
      return named
        ? { text: fill("operation_copy_loading_named", { operation: operation! }), key: "operation_copy_loading_named", actionable }
        : { text: t("operation_copy_loading"), key: "operation_copy_loading", actionable };
    }

    case "active":
      return named
        ? { text: fill("operation_copy_active_named", { operation: operation! }), key: "operation_copy_active_named", actionable }
        : { text: t("operation_copy_active"), key: "operation_copy_active", actionable };

    case "waiting":
      return named
        ? { text: fill("operation_copy_waiting_named", { operation: operation! }), key: "operation_copy_waiting_named", actionable }
        : { text: t("operation_copy_waiting"), key: "operation_copy_waiting", actionable };

    case "blocked":
      return named
        ? { text: fill("operation_copy_blocked_named", { operation: operation! }), key: "operation_copy_blocked_named", actionable }
        : { text: t("operation_copy_blocked"), key: "operation_copy_blocked", actionable };

    case "completed":
      return named
        ? { text: fill("operation_copy_completed_named", { operation: operation! }), key: "operation_copy_completed_named", actionable }
        : { text: t("operation_copy_completed"), key: "operation_copy_completed", actionable };

    case "failed": {
      // Cause + affected scope (operation name) + recovery is available
      // (actionable). Prefer the source-owned cause; omit it when absent.
      if (message !== undefined) {
        return named
          ? { text: fill("operation_copy_failed_message", { operation: operation!, message }), key: "operation_copy_failed_message", actionable }
          : { text: fill("operation_copy_failed_message_unnamed", { message }), key: "operation_copy_failed_message_unnamed", actionable };
      }
      return named
        ? { text: fill("operation_copy_failed_named", { operation: operation! }), key: "operation_copy_failed_named", actionable }
        : { text: t("operation_copy_failed"), key: "operation_copy_failed", actionable };
    }

    case "retrying":
      return named
        ? { text: fill("operation_copy_retrying_named", { operation: operation! }), key: "operation_copy_retrying_named", actionable }
        : { text: t("operation_copy_retrying"), key: "operation_copy_retrying", actionable };

    case "recovered":
      // Restoration announced under a STABLE key so it fires exactly once
      // (Req 13.5). Consumers de-duplicate by this key identity.
      return named
        ? { text: fill("operation_copy_recovered_named", { operation: operation! }), key: "operation_copy_recovered_named", actionable }
        : { text: t("operation_copy_recovered"), key: "operation_copy_recovered", actionable };

    case "optional-service-unavailable": {
      if (message !== undefined) {
        return named
          ? { text: fill("operation_copy_unavailable_message", { operation: operation!, message }), key: "operation_copy_unavailable_message", actionable }
          : { text: fill("operation_copy_unavailable_message_unnamed", { message }), key: "operation_copy_unavailable_message_unnamed", actionable };
      }
      return named
        ? { text: fill("operation_copy_unavailable_named", { operation: operation! }), key: "operation_copy_unavailable_named", actionable }
        : { text: t("operation_copy_unavailable"), key: "operation_copy_unavailable", actionable };
    }
  }
}
