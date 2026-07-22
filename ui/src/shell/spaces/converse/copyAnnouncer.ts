/**
 * Copy-outcome announcer (Req 12.3, 12.5; UIE-M-009).
 *
 * The per-message Copy action and the markdown code-block copy button are
 * purely-local clipboard operations whose success/failure was previously
 * dropped silently. This shared signal lets those paths publish a concise
 * outcome to a POLITE live region (rendered by ConverseSpace as a
 * `role="status"` element — implicit `aria-live="polite"`) WITHOUT moving
 * focus off the originating control.
 *
 * Single-live-region invariant: the sole conversation `[aria-live]` region is
 * `.kria-converse__stream` (role=log). This announcer's status element
 * deliberately relies on `role="status"`'s IMPLICIT polite semantics and does
 * NOT set an explicit `aria-live` attribute, so it never becomes a second
 * `[aria-live]` region (the invariant asserted by converseA11yScroll.test.tsx).
 *
 * Deduplicate / throttle (record risk "repeated announcements"): identical
 * outcomes fired in rapid succession are collapsed to a single announcement.
 * A distinct outcome, or the same outcome after the window elapses, is spoken
 * again by clearing then re-keying the text so the polite region re-reads it
 * even when the string is unchanged.
 */
import { createSignal } from "solid-js";

export type CopyOutcome = "success" | "failure";

/** Window within which an identical repeated outcome is suppressed (ms). */
export const COPY_DEDUP_WINDOW_MS = 1000;
/** How long the announced text lingers before it is cleared (ms). */
const COPY_CLEAR_AFTER_MS = 4000;

const OUTCOME_TEXT: Readonly<Record<CopyOutcome, string>> = {
  success: "Copied to clipboard",
  failure: "Copy failed",
};

const [announcement, setAnnouncement] = createSignal("");

let lastText = "";
let lastAt = 0;
let clearTimer: ReturnType<typeof setTimeout> | undefined;

/** Reactive accessor for the current copy announcement (empty when idle). */
export const copyAnnouncement = announcement;

/**
 * Publish a copy outcome to the polite status region.
 *
 * @param outcome success/failure of the clipboard write.
 * @param now injectable clock for deterministic tests.
 */
export function announceCopyOutcome(outcome: CopyOutcome, now: number = Date.now()): void {
  const text = OUTCOME_TEXT[outcome];

  // Deduplicate identical rapid outcomes (throttle repeated unchanged text).
  if (text === lastText && now - lastAt < COPY_DEDUP_WINDOW_MS) return;
  lastText = text;
  lastAt = now;

  // Clear then re-key so an intended re-announce of the SAME text (a repeat
  // outside the dedup window) is still spoken by the polite region, which
  // only reads on a content change.
  setAnnouncement("");
  queueMicrotask(() => setAnnouncement(text));

  if (clearTimer) clearTimeout(clearTimer);
  clearTimer = setTimeout(() => setAnnouncement(""), COPY_CLEAR_AFTER_MS);
}

/** Test-only reset of the dedup/throttle state and pending clear timer. */
export function resetCopyAnnouncerForTest(): void {
  if (clearTimer) clearTimeout(clearTimer);
  clearTimer = undefined;
  lastText = "";
  lastAt = 0;
  setAnnouncement("");
}
