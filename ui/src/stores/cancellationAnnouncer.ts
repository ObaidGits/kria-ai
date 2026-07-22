/**
 * Cancellation-milestone announcer (Req 12.12; UIE-M-015 / §17.5).
 *
 * When a scoped Stop is activated (turn/response, per-item work block, or the
 * GUI-cognition turn), the surface must announce the SEMANTIC MILESTONE — that
 * the named scope stopped — exactly ONCE, not a raw stream of reactive ticks
 * (design §17.5: "announce a milestone once, not every reactive tick"). This
 * shared signal lets the existing cancellation handlers publish one concise,
 * scope-named milestone to a POLITE live region (rendered by ConverseSpace as a
 * `role="status"` element — implicit `aria-live="polite"`) WITHOUT moving focus
 * off the originating Stop control.
 *
 * It lives in the store layer (alongside the cancellation handlers that fire
 * it) so `converseStore` never has to import from the shell layer. Every Stop
 * control funnels through those existing handlers, so announcing here covers
 * the Composer primary Stop, the immersive PresenceBar Stop, the per-item
 * WorkBlock Stop, and the GUI-cognition Stop from a single, deduplicated point.
 *
 * Single-live-region invariant: the sole conversation `[aria-live]` region is
 * `.kria-converse__stream` (role=log). Like `copyAnnouncer`, this announcer's
 * status element relies on `role="status"`'s IMPLICIT polite semantics and does
 * NOT set an explicit `aria-live` attribute, so it never becomes a second
 * `[aria-live]` region.
 *
 * Deduplicate / throttle: an identical milestone fired again within the window
 * (e.g. Composer Stop and the immersive Global Stop both invoking `stopTurn`,
 * or a double-click) is collapsed to a single announcement — the milestone is
 * spoken once, not per tick. A distinct milestone, or the same one after the
 * window elapses, is re-announced by clearing then re-keying the text so the
 * polite region re-reads it even when the string is unchanged.
 */
import { createSignal } from "solid-js";

/** Window within which an identical repeated milestone is suppressed (ms). */
export const CANCELLATION_DEDUP_WINDOW_MS = 1000;
/** How long the announced milestone lingers before it is cleared (ms). */
const CANCELLATION_CLEAR_AFTER_MS = 4000;

const [announcement, setAnnouncement] = createSignal("");

let lastText = "";
let lastAt = 0;
let clearTimer: ReturnType<typeof setTimeout> | undefined;

/** Reactive accessor for the current cancellation milestone (empty when idle). */
export const cancellationAnnouncement = announcement;

/**
 * Publish a scope-named cancellation milestone to the polite status region.
 *
 * @param milestone concise, scope-named milestone text (e.g. "Response
 *   stopped", "GUI cognition stopped", "Tool call stopped"). Announced once.
 * @param now injectable clock for deterministic tests.
 */
export function announceCancellation(milestone: string, now: number = Date.now()): void {
  const text = milestone.trim();
  if (text.length === 0) return;

  // Deduplicate identical rapid milestones (throttle repeated unchanged text)
  // so a milestone is announced ONCE, not on every reactive tick or repeated
  // Stop activation of the same scope.
  if (text === lastText && now - lastAt < CANCELLATION_DEDUP_WINDOW_MS) return;
  lastText = text;
  lastAt = now;

  // Clear then re-key so an intended re-announce of the SAME milestone (a
  // repeat outside the dedup window) is still spoken by the polite region,
  // which only reads on a content change.
  setAnnouncement("");
  queueMicrotask(() => setAnnouncement(text));

  if (clearTimer) clearTimeout(clearTimer);
  clearTimer = setTimeout(() => setAnnouncement(""), CANCELLATION_CLEAR_AFTER_MS);
}

/** Test-only reset of the dedup/throttle state and pending clear timer. */
export function resetCancellationAnnouncerForTest(): void {
  if (clearTimer) clearTimeout(clearTimer);
  clearTimer = undefined;
  lastText = "";
  lastAt = 0;
  setAnnouncement("");
}
