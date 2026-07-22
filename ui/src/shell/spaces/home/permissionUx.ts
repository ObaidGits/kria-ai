/**
 * Permission UX — risk-tier → presence presentation mapping (design.md §10.4 /
 * §19 "Permission", Requirement 10).
 *
 * KRIA never shows dialog fatigue. Approval is expressed *through presence*, and
 * exactly how depends on the safety layer's risk tier (the GREEN/YELLOW/RED
 * classification carried on every {@link ApprovalRequest.risk}). This module is
 * the PURE, deterministic core of that mapping — no SolidJS, no DOM, no store
 * writes — so it can be exhaustively property-tested:
 *
 *   • GREEN  (safe / reversible)      → **report**   — KRIA already acted; it
 *     reports the fact via the Voice Line and offers an **undo** where the
 *     action is reversible. NO blocking prompt (Req 10.1).
 *   • YELLOW                          → **intent**    — KRIA narrates what it is
 *     about to do and opens a brief **halt window**; if the user does not stop
 *     it, it proceeds (Req 10.2). Not a hard modal block.
 *   • RED / BLACK (irreversible /     → **decision**  — KRIA steps the Core
 *     sensitive)                        forward, calms it, and presents a single-
 *     line **allow / deny** through the Focus, routing detail to the Approval
 *     Center on request. It blocks GENTLY — never a red-panic, never a
 *     modal-on-modal (Req 10.3).
 *
 * ── Reuse, not reinvention (Req 10.4) ────────────────────────────────────────
 * The `approvalStore` + Approval Center remain the OWNER of decision detail. A
 * {@link PermissionSubject} is a thin projection of an existing
 * {@link ApprovalRequest}; the presence layer stages decisions back through
 * `approvalStore` (approve / deny / keep-paused) and routes "show me more" to
 * the Approval Center overlay. This module NEVER executes an action, mutates a
 * store, or invents a new backend contract — it only classifies + describes.
 *
 * ── No modal-on-modal (Req 10.3, guardrails.md "Never block with a modal over a
 *    modal") ───────────────────────────────────────────────────────────────────
 * The inline presence permission surface must never stack over an already-open
 * blocking surface (the Approval Center overlay or any ModalHost modal). When
 * one is active, {@link resolvePermissionView} yields a `deferred` view: the
 * homepage shows nothing and lets the existing blocking surface own the
 * decision. This is the testable guarantee behind the no-stacking invariant.
 *
 * Requirements: 10.1, 10.2, 10.3, 10.4.
 */
import type { ApprovalRequest, RiskLevel } from "../../../stores/approvalStore";

// ─── Tier → mode mapping (total & correct) ───────────────────────────────────

/**
 * The three presence presentation modes for a permission subject (design §10.4):
 * `report` (GREEN — act + report + undo), `intent` (YELLOW — narrate + halt
 * window), `decision` (RED/BLACK — step-forward single-line allow/deny).
 */
export type PermissionMode = "report" | "intent" | "decision";

/**
 * Map a safety-layer {@link RiskLevel} to its presence {@link PermissionMode}.
 * TOTAL over every risk tier (green/yellow/red/black) and never throws — the
 * `switch` is exhaustive, so a new tier would be a compile error, not a runtime
 * gap (Req 10.1/10.2/10.3).
 *
 *   green            → report
 *   yellow           → intent
 *   red | black      → decision
 *
 * BLACK (the most severe tier) shares RED's decision presentation: it is
 * irreversible/sensitive and must earn an explicit allow/deny — never a silent
 * report or a mere halt window.
 */
export function resolvePermissionMode(risk: RiskLevel): PermissionMode {
  switch (risk) {
    case "green":
      return "report";
    case "yellow":
      return "intent";
    case "red":
    case "black":
      return "decision";
  }
}

// ─── Permission subject (projection of an approval) ──────────────────────────

/**
 * A homepage-facing projection of an {@link ApprovalRequest}: just what the
 * presence layer needs to CLASSIFY and DESCRIBE the ask. The `approvalStore`
 * request stays the source of truth (owner of routing + detail).
 */
export interface PermissionSubject {
  /** The owning `approvalStore` request id (routes decisions back to it). */
  requestId: string;
  /** The safety-layer risk tier. */
  risk: RiskLevel;
  /** The presentation mode derived from {@link risk}. */
  mode: PermissionMode;
  /** WHAT will happen / happened — the single headline line (Req 10.4). */
  what: string;
  /** WHY — plain-language rationale, always kept visible (Req 10.4). */
  why: string;
  /**
   * Whether the action is reversible. GREEN is reversible by definition, so an
   * **undo** affordance is offered UNLESS the request is explicitly marked
   * irreversible (Req 10.1 "with an undo affordance where applicable").
   */
  reversible: boolean;
  /** Creation time (used only for deterministic selection tie-breaks). */
  createdAt: number;
}

/** Project an {@link ApprovalRequest} into a {@link PermissionSubject}. Pure. */
export function toPermissionSubject(request: ApprovalRequest): PermissionSubject {
  return {
    requestId: request.id,
    risk: request.risk,
    mode: resolvePermissionMode(request.risk),
    what: request.title,
    why: request.description,
    // Reversible unless the request explicitly declares itself irreversible.
    reversible: request.irreversible !== true,
    createdAt: request.createdAt,
  };
}

// ─── Deterministic single-subject selection (Req 10, one Focus subject) ──────

/**
 * Risk precedence for picking the ONE permission subject the homepage presents
 * (the homepage shows a single Focus subject, never two competing asks). Higher
 * wins: a RED decision always outranks a YELLOW halt, which outranks a GREEN
 * report. Ties break on recency (newest first) for determinism.
 */
const RISK_PRECEDENCE: Readonly<Record<RiskLevel, number>> = {
  black: 3,
  red: 3,
  yellow: 2,
  green: 1,
};

/**
 * Choose the single most-urgent PENDING permission subject from an approval
 * queue, or `undefined` when nothing is pending. Deterministic: highest risk
 * precedence first, then newest `createdAt`, then id (final stable tie-break).
 * Only `pending` requests are eligible — resolved/expired items never surface.
 */
export function selectPermissionSubject(
  requests: readonly ApprovalRequest[],
): PermissionSubject | undefined {
  let best: ApprovalRequest | undefined;
  for (const r of requests) {
    if (r.status !== "pending") continue;
    if (best === undefined || isMoreUrgent(r, best)) best = r;
  }
  return best ? toPermissionSubject(best) : undefined;
}

/** Strict "a should surface before b" ordering used by {@link selectPermissionSubject}. */
function isMoreUrgent(a: ApprovalRequest, b: ApprovalRequest): boolean {
  const pa = RISK_PRECEDENCE[a.risk];
  const pb = RISK_PRECEDENCE[b.risk];
  if (pa !== pb) return pa > pb;
  if (a.createdAt !== b.createdAt) return a.createdAt > b.createdAt;
  return a.id > b.id;
}

// ─── No-modal-on-modal gate ──────────────────────────────────────────────────

/**
 * The blocking-overlay state the inline permission surface must respect. When
 * EITHER a ModalHost modal or the Approval Center overlay is already open, a
 * blocking surface is active and the inline presence permission surface must
 * NOT stack over it (Req 10.3 / guardrails.md "Never block with a modal over a
 * modal").
 */
export interface OverlayState {
  /** The pending Approval Center overlay is open. */
  approvalCenterOpen: boolean;
  /** A ModalHost modal (or nested approval confirm) is open. */
  modalOpen: boolean;
}

/**
 * Whether the inline permission surface must DEFER to an already-active blocking
 * surface (never stack). This is the pure, testable core of the no-modal-on-
 * modal invariant: `true` ⟺ some blocking overlay is already active.
 */
export function shouldDeferToActiveOverlay(overlay: OverlayState): boolean {
  return overlay.approvalCenterOpen || overlay.modalOpen;
}

// ─── Halt window (YELLOW) ─────────────────────────────────────────────────────

/**
 * The brief window (ms) a YELLOW subject stays haltable before proceeding
 * (Req 10.2 "a brief window to halt, then proceed if not stopped"). Short by
 * design — long enough to catch, not long enough to nag.
 */
export const HALT_WINDOW_MS = 4000;

// ─── Presentation view (what the component renders) ──────────────────────────

/**
 * The resolved presence view for the current permission subject + overlay state.
 * A discriminated union so the component renders exactly one shape and the
 * mapping stays exhaustively checkable.
 *
 *   none      — nothing to present (rest).
 *   deferred  — a blocking overlay is already open; defer to it (NO stacking).
 *   report    — GREEN: report line + optional undo, non-blocking (Req 10.1).
 *   intent    — YELLOW: intent line + halt window (Req 10.2).
 *   decision  — RED/BLACK: single-line allow/deny, calm step-forward (Req 10.3).
 */
export type PermissionView =
  | { kind: "none" }
  | { kind: "deferred"; requestId: string }
  | { kind: "report"; requestId: string; what: string; why: string; undo: boolean }
  | { kind: "intent"; requestId: string; what: string; why: string; haltWindowMs: number }
  | {
      kind: "decision";
      requestId: string;
      what: string;
      why: string;
      /** Calm/interruptibility-blocked posture (via the ember, never audio). */
      blockedContext: boolean;
    };

/** Extra context that shapes a `decision` view's calm posture. */
export interface PermissionViewContext {
  /**
   * True in an interruptibility-blocked context (call/record/present/DND). A RED
   * decision then surfaces CALMLY (via the ember, never audio) — Req 26.3. Only
   * meaningful for `decision`; ignored otherwise. Defaults to `false`.
   */
  blockedContext?: boolean;
}

/**
 * Resolve the single presence view for a permission subject given the current
 * blocking-overlay state. Pure and total:
 *
 *   • no subject                       → `none`
 *   • any blocking overlay already open → `deferred` (no modal-on-modal, Req 10.3)
 *   • GREEN                             → `report`   (+undo iff reversible, Req 10.1)
 *   • YELLOW                            → `intent`   (+halt window, Req 10.2)
 *   • RED / BLACK                       → `decision` (single-line allow/deny, Req 10.3)
 *
 * The deferral check runs BEFORE the mode split, so NO tier ever produces an
 * inline surface while a blocking overlay is active — the no-stacking guarantee
 * holds for every risk level, not just RED.
 */
export function resolvePermissionView(
  subject: PermissionSubject | undefined,
  overlay: OverlayState,
  context: PermissionViewContext = {},
): PermissionView {
  if (!subject) return { kind: "none" };

  // No-modal-on-modal (Req 10.3): if a blocking surface is already open, defer
  // to it — the homepage never stacks its own permission surface on top.
  if (shouldDeferToActiveOverlay(overlay)) {
    return { kind: "deferred", requestId: subject.requestId };
  }

  switch (subject.mode) {
    case "report":
      return {
        kind: "report",
        requestId: subject.requestId,
        what: subject.what,
        why: subject.why,
        undo: subject.reversible,
      };
    case "intent":
      return {
        kind: "intent",
        requestId: subject.requestId,
        what: subject.what,
        why: subject.why,
        haltWindowMs: HALT_WINDOW_MS,
      };
    case "decision":
      return {
        kind: "decision",
        requestId: subject.requestId,
        what: subject.what,
        why: subject.why,
        blockedContext: context.blockedContext === true,
      };
  }
}
