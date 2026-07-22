/**
 * Core narration — a PURE, READ-ONLY mapping from the authoritative Core state
 * to concise situational text (UIE-H-013, Req 8.5; design §15 UIE-H-013,
 * §8 principle 2 "Truth before theater").
 *
 * WHY THIS EXISTS: `CorePresence` communicates broad Core state through motion,
 * but motion alone cannot convey the current object, wait reason, or next
 * action — and it is inaccessible to assistive tech. This module pairs the Core
 * state with a short textual projection so the situational meaning is visible
 * and announced WITHOUT changing CorePresence visuals or motion (Req 8.6: Core
 * presence remains the sole ambient presence — this is additive text on the
 * status surface, not a Core change).
 *
 * ── Truth before theater (design §8 principle 2) ────────────────────────────
 *   • Text is derived ONLY from authoritative signals (coreStore state +
 *     error/block metadata, approval count, and the source-owned active-work
 *     label). Nothing is inferred or invented.
 *   • Text is produced ONLY for the states where it improves user action or
 *     understanding. Every OTHER state — including `idle` and any unmapped /
 *     future / unknown Core state — is OMITTED (returns `null`). Idle fabricates
 *     nothing.
 *   • Where a state has a concrete authoritative object (blocked → the pending
 *     approval, error → the error message, acting → the active work label) the
 *     text names it and is marked actionable so it can point at the owner.
 *     Where the object is unknown it is omitted, never invented — the objectless
 *     concise phrase is used instead.
 *
 * This module is intentionally NOT a store: no signals, no setters, no side
 * effects, no timers. The live reactive accessor {@link coreNarration} wires the
 * authoritative signals for consumers (StatusLine); the pure
 * {@link narrateCoreState} is unit-tested with seeded inputs.
 *
 * Requirements: 8.5, 8.6; design §8 (Truth before theater), §11.8, §15 (UIE-H-013).
 */
import type { CoreState } from "./coreStore";
import { coreStore } from "./coreStore";
import { approvalStore } from "./approvalStore";
import { currentWorkSummary } from "./currentWorkSummary";
import { t } from "./i18n";

// ─── Types ───────────────────────────────────────────────────────────────────

/** Authoritative snapshot the narration is derived from (test-seedable). */
export interface CoreNarrationInput {
  readonly state: CoreState;
  /** coreStore.errorMessage — the source-owned error text, if any. */
  readonly errorMessage?: string | null;
  /** coreStore.blockReason — the source-owned block reason, if any. */
  readonly blockReason?: string | null;
  /** approvalStore.pendingCount — used only to point `blocked` at its owner. */
  readonly pendingApprovals?: number;
  /** currentWorkSummary work[0].label — the source-owned active-work object. */
  readonly activeWorkLabel?: string | null;
}

/** A concise situational narration paired with a Core state. */
export interface CoreNarration {
  /** The concise, localized text to display/announce. Never empty. */
  readonly text: string;
  /**
   * The i18n key (or key family) the text came from. Stable per semantic
   * meaning so consumers can deduplicate unchanged announcements by identity.
   */
  readonly key: string;
  /**
   * True when the narration names/points at a concrete owner or next action
   * (blocked → approval, error → recovery, acting → a named work object).
   */
  readonly actionable: boolean;
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

function nonEmpty(value: string | undefined | null): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : undefined;
}

/** Interpolate `{name}` placeholders in a localized template (mirrors the
 * existing Settings `featureCopy` convention). */
function fill(key: string, values: Record<string, string>): string {
  return Object.entries(values).reduce(
    (copy, [name, value]) => copy.replaceAll(`{${name}}`, value),
    t(key),
  );
}

// ─── Pure mapping ────────────────────────────────────────────────────────────

/**
 * Pure projection: authoritative snapshot → {@link CoreNarration} | null.
 *
 * Deterministic and side-effect free. Returns `null` (OMIT — no text) for
 * `idle` and for every state outside the mapped set
 * (listening / thinking / planning / acting / waiting / blocked / error /
 * recovering); those states either add no situational value here or are owned
 * by other surfaces, and must never be given fabricated narration.
 */
export function narrateCoreState(input: CoreNarrationInput): CoreNarration | null {
  switch (input.state) {
    case "listening":
      return { text: t("core_narration_listening"), key: "core_narration_listening", actionable: false };

    case "thinking":
      return { text: t("core_narration_thinking"), key: "core_narration_thinking", actionable: false };

    case "planning":
      return { text: t("core_narration_planning"), key: "core_narration_planning", actionable: false };

    case "acting": {
      // Name the concrete work object when the source provides one; otherwise
      // use the objectless phrase (never invent an object).
      const object = nonEmpty(input.activeWorkLabel);
      if (object) {
        return {
          text: fill("core_narration_acting_object", { object }),
          key: "core_narration_acting_object",
          actionable: true,
        };
      }
      return { text: t("core_narration_acting"), key: "core_narration_acting", actionable: false };
    }

    case "waiting":
      // The wait reason is not surfaced as an authoritative signal, so no object
      // is named — a truthful, concise, objectless phrase is used.
      return { text: t("core_narration_waiting"), key: "core_narration_waiting", actionable: false };

    case "blocked": {
      // Blocked is produced only by an approval gate: point at the owner
      // (Approval Center). Prefer the source-owned reason when present.
      const reason = nonEmpty(input.blockReason);
      if (reason) {
        return {
          text: fill("core_narration_blocked_reason", { reason }),
          key: "core_narration_blocked_reason",
          actionable: true,
        };
      }
      return { text: t("core_narration_blocked"), key: "core_narration_blocked", actionable: true };
    }

    case "error": {
      // Prefer the source-owned error message; otherwise a truthful generic
      // that still signals recovery is available.
      const message = nonEmpty(input.errorMessage);
      if (message) {
        return {
          text: fill("core_narration_error_message", { message }),
          key: "core_narration_error_message",
          actionable: true,
        };
      }
      return { text: t("core_narration_error"), key: "core_narration_error", actionable: true };
    }

    case "recovering":
      return { text: t("core_narration_recovering"), key: "core_narration_recovering", actionable: true };

    // idle + every other/unknown/unmapped Core state → OMITTED (no fabrication).
    default:
      return null;
  }
}

// ─── Live reactive accessor ──────────────────────────────────────────────────

/**
 * Live read-only narration wired to the authoritative signals. Safe to call
 * inside a Solid memo/JSX: it reads the source signals (establishing reactive
 * dependencies) and returns a fresh {@link CoreNarration} or `null`. It performs
 * no writes and owns no lifecycle.
 */
export function coreNarration(): CoreNarration | null {
  const summary = currentWorkSummary();
  return narrateCoreState({
    state: coreStore.state(),
    errorMessage: coreStore.errorMessage(),
    blockReason: coreStore.blockReason(),
    pendingApprovals: approvalStore.pendingCount(),
    activeWorkLabel: summary.work[0]?.label ?? null,
  });
}
