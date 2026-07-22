/**
 * Awareness privacy model — enforced invariants (task 3.8, design §25.2/§25.3,
 * Req 25.4/25.5).
 *
 * This module encodes the *non-negotiable* privacy guarantees of the desktop-
 * awareness subsystem as executable invariants, so they hold structurally in
 * code rather than living only in prose:
 *
 *   1. **No forbidden capture** (Req 25.4, design §25.2) — KRIA must NEVER
 *      perform global keylogging, unconsented clipboard/screen-content/file-
 *      history/browsing-history capture, or persistent app-usage recording, and
 *      must never obtain awareness by raw scanning. The registry can therefore
 *      only register sources whose integration is on the **local allowlist**
 *      ({@link ALLOWED_INTEGRATION_KINDS}); any other/forbidden mechanism is
 *      rejected at registration time via {@link assertRegisterableIntegration}.
 *
 *   2. **All-local processing** (Req 25.5, design §25.3) — every allowlisted
 *      integration is a *local* portal/integration/system API. No awareness
 *      integration performs network egress; {@link assertAllLocalIntegrations}
 *      asserts a catalog contains only local kinds so awareness data never
 *      leaves the device.
 *
 *   3. **Ephemeral unless opted into memory** (Req 25.4, design §25.3) —
 *      awareness signals drive the *current* FocusFrame and are never persisted
 *      as history unless the user explicitly opts a source into memory. This
 *      module owns the gate: {@link selectRememberableSignals} yields ONLY the
 *      signals from sources the user has opted into memory (default: none), so
 *      any future memory writer must route through it and can never persist an
 *      un-consented signal.
 *
 * The module is intentionally standalone (no import from the registry) so the
 * registry, the Settings panel, and the guardrail tests can all depend on it
 * without a cycle. Requirements: 25.4, 25.5.
 */

// ─── Integration-kind allowlist (Req 25.4, design §25.2) ─────────────────────

/**
 * The ONLY mechanisms by which KRIA may obtain an awareness signal. Each is a
 * local portal/integration/system API — never raw scanning, never a forbidden
 * capture. This is the structural allowlist enforced at registration time and
 * MUST stay in sync with `SourceIntegrationKind` in `desktopAwarenessBridge.ts`.
 */
export const ALLOWED_INTEGRATION_KINDS = [
  "calendar-integration",
  "editor-integration",
  "mpris",
  "xdg-portal",
  "pipewire-portal",
  "system",
  "file-watch",
] as const;

export type AllowedIntegrationKind = (typeof ALLOWED_INTEGRATION_KINDS)[number];

const ALLOWED_SET: ReadonlySet<string> = new Set(ALLOWED_INTEGRATION_KINDS);

/**
 * Capture mechanisms KRIA must NEVER use (design §25.2 "What KRIA must NEVER
 * know / do"). Listed explicitly so the guardrail lint/tests can assert none of
 * these ever appears as a source integration, and so rejection messages are
 * specific. Anything NOT on {@link ALLOWED_INTEGRATION_KINDS} is rejected too;
 * these are the named, forbidden surveillance kinds.
 */
export const FORBIDDEN_CAPTURE_KINDS = [
  "keylog",
  "keylogger",
  "keylogging",
  "clipboard-capture",
  "clipboard-scan",
  "screen-capture-content",
  "screen-content-capture",
  "screen-scrape",
  "screenshot-capture",
  "file-history",
  "file-history-capture",
  "browsing-history",
  "browser-history",
  "app-usage-history",
  "app-usage-recording",
  "usage-recording",
  "window-scan",
  "process-scan",
  "network-egress",
] as const;

export type ForbiddenCaptureKind = (typeof FORBIDDEN_CAPTURE_KINDS)[number];

const FORBIDDEN_SET: ReadonlySet<string> = new Set(FORBIDDEN_CAPTURE_KINDS);

/** Thrown when a source declares a forbidden / non-allowlisted capture kind. */
export class ForbiddenCaptureError extends Error {
  readonly kind: string;
  readonly sourceId: string;
  constructor(kind: string, sourceId: string, reason: string) {
    super(
      `Awareness source "${sourceId}" is forbidden: integration "${kind}" ${reason} ` +
        `(privacy model — Req 25.4, design §25.2). KRIA never keylogs, captures the ` +
        `clipboard/screen/file-history/browsing-history without consent, records app ` +
        `usage, or scans; sources must use a local allowlisted portal/integration.`,
    );
    this.name = "ForbiddenCaptureError";
    this.kind = kind;
    this.sourceId = sourceId;
  }
}

/** Whether a mechanism is an explicitly-named forbidden capture kind. */
export function isForbiddenCaptureKind(kind: string): boolean {
  return FORBIDDEN_SET.has(kind);
}

/** Whether a mechanism is on the local, non-surveilling allowlist. */
export function isAllowedIntegrationKind(kind: string): kind is AllowedIntegrationKind {
  return ALLOWED_SET.has(kind);
}

/**
 * Assert a source's integration may be registered. Throws
 * {@link ForbiddenCaptureError} when the kind is an explicitly-forbidden capture
 * OR is simply not on the local allowlist (deny-by-default). This is the
 * structural guarantee that the registry can NEVER register a keylogging /
 * unconsented-capture / scanning source (Req 25.4).
 */
export function assertRegisterableIntegration(kind: string, sourceId: string): void {
  if (isForbiddenCaptureKind(kind)) {
    throw new ForbiddenCaptureError(kind, sourceId, "is a prohibited surveillance mechanism");
  }
  if (!isAllowedIntegrationKind(kind)) {
    throw new ForbiddenCaptureError(kind, sourceId, "is not on the local integration allowlist");
  }
}

// ─── All-local processing (Req 25.5, design §25.3) ───────────────────────────

/**
 * Assert every integration kind in a catalog is a local (non-network) mechanism.
 * All allowlisted kinds are local by construction; this asserts the catalog
 * introduced no non-local kind, so awareness is processed on-device and never
 * transmitted off the machine (Req 25.5). Throws {@link ForbiddenCaptureError}
 * on the first offending kind.
 */
export function assertAllLocalIntegrations(
  sources: readonly { id: string; integration: string }[],
): void {
  for (const src of sources) {
    if (!isAllowedIntegrationKind(src.integration)) {
      throw new ForbiddenCaptureError(
        src.integration,
        src.id,
        "is not a local allowlisted integration; awareness must be processed locally",
      );
    }
  }
}

// ─── Ephemeral-unless-opted-into-memory gate (Req 25.4, design §25.3) ────────

/**
 * The privacy default: awareness signals are ephemeral. They drive the current
 * FocusFrame and are NEVER persisted as history unless the user has explicitly
 * opted the producing source into memory.
 */
export const EPHEMERAL_BY_DEFAULT = true as const;

/**
 * The opt-into-memory gate. Given the currently-live awareness signals and a
 * predicate reporting whether a signal's *source* is opted into memory, returns
 * ONLY the signals that may be remembered. With no source opted in (the
 * default) this returns an empty list — nothing is ever persisted without
 * consent (Req 25.4). Any code that would write awareness to memory MUST obtain
 * its input from this function, never from the raw signal stream.
 *
 * @param signals   the live (ephemeral) signals
 * @param isRemembered predicate: is this signal's source opted into memory?
 */
export function selectRememberableSignals<T>(
  signals: readonly T[],
  isRemembered: (signal: T) => boolean,
): T[] {
  const out: T[] = [];
  for (const signal of signals) {
    if (isRemembered(signal)) out.push(signal);
  }
  return out;
}
