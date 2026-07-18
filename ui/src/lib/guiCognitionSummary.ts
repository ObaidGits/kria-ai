import type {
  GuiCognitionLifecycle,
  GuiCognitionSessionState,
} from "../types/guiCognition";

export type GuiCognitionStatusTone = "active" | "success" | "warning" | "danger" | "neutral";

export interface GuiCognitionFact {
  label: string;
  value: string;
}

export interface GuiCognitionSummary {
  /** One-word badge text, e.g. Completed / Paused / Blocked. */
  statusLabel: string;
  statusTone: GuiCognitionStatusTone;
  /** Plain-language one-liner for a layman. No hashes/IDs. */
  headline: string;
  /** 3-5 short key facts. */
  facts: GuiCognitionFact[];
  /** Top safety/perception warnings (already plain language). */
  warnings: string[];
  /** Optional suggested next step. */
  nextStep?: string;
}

/**
 * Task 10.4 / Requirement 16.5 — privacy guarantee for the layman layer.
 *
 * The layman layer (status badge + headline + facts + warnings + next step)
 * must NEVER leak low-level technical identifiers. Even though the backend
 * already emits "safe" explanations, this is a hard, defense-in-depth scrub so
 * that no hash, internal ID, coordinate, or secret can reach the plain-language
 * layer regardless of upstream regressions. The full technical detail (hashes,
 * IDs, coordinates, envelopes) stays in the panel's collapsible developer layer.
 *
 * Redacted classes:
 *  - secret markers (token/password/api_key/bearer/... = value)
 *  - UUIDs
 *  - hash mentions (`*-hash-*`) and hex digests (>= 12 hex chars)
 *  - internal ID tokens carrying a known KRIA prefix (session-/turn-/ctx-/...)
 *  - coordinate / pixel-size pairs (12,24 or 180x32)
 */
const REDACTED = "[redacted]";

const SECRET_RE =
  /\b(token|password|passwd|secret|api[_-]?key|apikey|access[_-]?key|bearer|authorization|auth[_-]?token)\b\s*[:=]\s*\S+/gi;
const UUID_RE =
  /\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b/g;
const HASH_TOKEN_RE = /\b[A-Za-z0-9]+[-_]hash[-_][A-Za-z0-9]+\b/gi;
const HEX_DIGEST_RE = /\b[0-9a-f]{12,}\b/gi;
const ID_TOKEN_RE =
  /\b(session|turn|workflow|context|ctx|control|proposal|execution|resolution|plan|goal|checkpoint|request|verification|receipt|observation|obs|candidate|validation|target|prompt|query)[-_](?:hash[-_])?[A-Za-z0-9]+(?:[-_][A-Za-z0-9]+)*\b/gi;
const PIXEL_SIZE_RE = /\b\d{2,5}x\d{2,5}\b/g;
const COORD_PAIR_RE = /\b\d{2,4}\s*,\s*\d{2,4}\b/g;

/**
 * Scrubs a single layman-facing string. Idempotent and safe to apply to any
 * plain-language fragment (headline, fact value, warning, next step).
 */
export function sanitizeLaymanText(input: string | undefined): string {
  if (!input) return "";
  return input
    .replace(SECRET_RE, (_match, key: string) => `${key}=${REDACTED}`)
    .replace(UUID_RE, REDACTED)
    .replace(HASH_TOKEN_RE, REDACTED)
    .replace(HEX_DIGEST_RE, REDACTED)
    .replace(ID_TOKEN_RE, REDACTED)
    .replace(PIXEL_SIZE_RE, REDACTED)
    .replace(COORD_PAIR_RE, REDACTED)
    .replace(/\s{2,}/g, " ")
    .trim();
}

const STATUS_LABEL: Record<GuiCognitionLifecycle, string> = {
  idle: "Idle",
  observing: "Working",
  planning: "Working",
  resolving: "Working",
  safety: "Working",
  awaiting_approval: "Needs approval",
  executing: "Working",
  verifying: "Working",
  blocked: "Blocked",
  completed: "Completed",
  failed: "Failed",
  cancelled: "Cancelled",
};

export function guiCognitionStatusTone(lifecycle: GuiCognitionLifecycle): GuiCognitionStatusTone {
  if (lifecycle === "completed") return "success";
  if (lifecycle === "blocked" || lifecycle === "failed") return "danger";
  if (lifecycle === "awaiting_approval" || lifecycle === "cancelled") return "warning";
  if (
    lifecycle === "executing" ||
    lifecycle === "verifying" ||
    lifecycle === "observing" ||
    lifecycle === "planning" ||
    lifecycle === "resolving" ||
    lifecycle === "safety"
  ) {
    return "active";
  }
  return "neutral";
}

function actionLabel(session: GuiCognitionSessionState): string | undefined {
  const action = session.currentAction ?? session.executionReceipt;
  const kind = action?.actionKind;
  if (!kind) return undefined;
  const pretty: Record<string, string> = {
    OpenApp: "opened the app",
    SwitchWindow: "switched window",
    FocusField: "focused the field",
    TypeText: "typed the text",
    ClickControl: "clicked the control",
    PressKey: "pressed the key",
    Submit: "submitted",
    Send: "sent",
    Copy: "copied",
    Paste: "pasted",
    Scroll: "scrolled",
  };
  return pretty[kind] ?? `ran ${kind}`;
}

function targetLabel(session: GuiCognitionSessionState): string | undefined {
  const action = session.currentAction ?? session.executionReceipt;
  return action?.target || session.target?.label || undefined;
}

function availabilityChip(session: GuiCognitionSessionState): string {
  const o = session.observation;
  const mark = (v: boolean | undefined) => (v ? "ok" : "off");
  return `Screen ${mark(o.screenshotAvailable)} · OCR ${mark(o.ocrAvailable)} · A11y ${mark(
    o.accessibilityAvailable,
  )}`;
}

function controlsChip(session: GuiCognitionSessionState): string {
  const o = session.observation;
  const total = o.visibleControlCount ?? 0;
  const disabled = o.disabledControlCount ?? 0;
  return disabled > 0 ? `${total} (${disabled} disabled)` : `${total}`;
}

function plainWarnings(session: GuiCognitionSessionState): string[] {
  const out: string[] = [];
  const o = session.observation;
  if ((o.accessibilityOverallStatus && o.accessibilityOverallStatus !== "healthy") ||
      (o.accessibilityTimeoutCount ?? 0) > 0) {
    out.push("Accessibility data is degraded on this screen.");
  }
  if ((o.observationTotalMs ?? 0) > 2000) {
    out.push("Observation was slow (OCR is the bottleneck).");
  }
  if ((o.disabledControlCount ?? 0) > 0 && (o.visibleControlCount ?? 0) > 0 &&
      (o.disabledControlCount ?? 0) === (o.visibleControlCount ?? 0)) {
    out.push("All visible controls are disabled/hidden — nothing is actionable here.");
  }
  if (session.observation.terminalLike) {
    out.push("Terminal focus is active; blind typing stays blocked.");
  }
  return out.slice(0, 2);
}

/**
 * Derives a layered, plain-language summary from the structured GUI Cognition
 * session. The raw/technical detail stays in the panel's developer accordion.
 */
export function deriveGuiCognitionSummary(
  session: GuiCognitionSessionState,
): GuiCognitionSummary {
  const lifecycle = session.lifecycle;
  const statusLabel = STATUS_LABEL[lifecycle] ?? "Working";
  const statusTone = guiCognitionStatusTone(lifecycle);
  const activeWindow = session.observation.activeWindow || "unknown";
  const action = actionLabel(session);
  const target = targetLabel(session);
  const verified = session.verification?.status === "verified";

  let headline: string;
  let nextStep: string | undefined;

  const workflow = session.workflow;
  const recovery = session.recovery;

  if (lifecycle === "awaiting_approval") {
    headline = action
      ? `Paused — ${action} needs your approval before I continue.`
      : "Paused — this action needs your approval before I continue.";
    nextStep = "Approve or deny in the prompt above.";
  } else if (lifecycle === "cancelled") {
    const reason = session.blocker?.reason || "Turn cancelled by you.";
    headline = `Cancelled — ${reason}`;
    nextStep = "Send a new prompt when you're ready.";
  } else if (lifecycle === "blocked" || lifecycle === "failed") {
    const reason =
      session.blocker?.reason ||
      session.checkpoint?.resumeExplanation ||
      recovery?.safeExplanation ||
      workflow?.blockedReason ||
      "I could not proceed safely.";
    headline = `Stopped safely — ${reason}`;
  } else if (recovery && recovery.status && recovery.status !== "recovered") {
    headline = `${action ? `Tried to ${action}` : "Tried the action"} but could not confirm the result.`;
    nextStep = "Re-observe before retrying; no blind retry was performed.";
  } else if (lifecycle === "completed") {
    if (workflow && (workflow.completedStepCount ?? 0) > 0) {
      headline = `Completed ${workflow.completedStepCount} step${
        (workflow.completedStepCount ?? 0) === 1 ? "" : "s"
      }, each verified one at a time.`;
    } else if (recovery?.status === "recovered") {
      headline = `Recovered safely via ${recovery.recoveryActionKind || "a safe action"} and restored the expected state.`;
    } else if (action) {
      headline = verified
        ? `Done — ${action}${target ? ` (${target})` : ""} and verified the result.`
        : `Done — ${action}${target ? ` (${target})` : ""}.`;
    } else {
      headline = `Observed your screen (${activeWindow}). No GUI action was taken.`;
    }
  } else {
    // Running / in-progress states.
    headline = action ? `Working — ${action}…` : `Observing ${activeWindow}…`;
  }

  const facts: GuiCognitionFact[] = [
    { label: "Active window", value: activeWindow },
    { label: "Controls", value: controlsChip(session) },
    { label: "Sources", value: availabilityChip(session) },
  ];
  if (action) {
    facts.push({
      label: "Action",
      value: `${action}${session.verification?.status ? ` · ${session.verification.status}` : ""}`,
    });
  } else if (lifecycle === "completed") {
    facts.push({ label: "Action", value: "observe only" });
  }
  if (workflow?.stepCount) {
    facts.push({
      label: "Workflow",
      value: `${workflow.completedStepCount ?? 0}/${workflow.stepCount} steps`,
    });
  }

  // Task 10.4 / Req 16.5: hard privacy scrub of every layman-facing string
  // before it leaves the builder. The developer layer keeps the raw detail.
  return {
    statusLabel,
    statusTone,
    headline: sanitizeLaymanText(headline),
    facts: facts.map((fact) => ({
      label: fact.label,
      value: sanitizeLaymanText(fact.value),
    })),
    warnings: plainWarnings(session).map((warning) => sanitizeLaymanText(warning)),
    nextStep: nextStep ? sanitizeLaymanText(nextStep) : undefined,
  };
}
