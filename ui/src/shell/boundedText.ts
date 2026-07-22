/**
 * Bounded-text presentation helper (task 10.7, IU-07; UIE-H-002, UIE-M-011,
 * UIE-M-018).
 *
 * The single, shared seam every Task-10 surface uses to present a possibly-long
 * model / provider / source / detail / label value TRUTHFULLY and BOUNDED:
 *   • the CSS class constants apply the shared `.kria-bounded*` truncation (see
 *     `styles/bounded-text.css`) so a long value clamps/ellipsizes rather than
 *     forcing horizontal overflow (Task-8 no-horizontal-overflow invariant), and
 *   • `boundedTitle(...)` yields the FULL value for a `title` (and/or accessible
 *     name) so the complete string stays recoverable on hover while the DOM
 *     keeps it for assistive tech.
 *
 * It NEVER fabricates: an absent / blank value yields `undefined` (no empty
 * `title`, no placeholder) — reusing the same `nonEmpty` omission discipline as
 * every other Task-10 surface (imported, not forked). Purely presentational —
 * no truncation of the underlying fact, no side effects.
 */
import { nonEmpty } from "../stores/currentWorkSummary";

/** Single-line ellipsis (labels, source values, chips). */
export const BOUNDED = "kria-bounded";
/** Clamp to 2 lines (concise multi-word labels). */
export const BOUNDED_CLAMP_2 = "kria-bounded--2";
/** Clamp to 3 lines (concise detail text). */
export const BOUNDED_CLAMP_3 = "kria-bounded--3";

/**
 * The FULL value for a `title` attribute on a visually-truncated element, so a
 * sighted user can recover the complete text on hover. Returns `undefined` for
 * an absent / blank value (never an empty title, never a fabricated placeholder)
 * via the shared `nonEmpty` discipline.
 */
export function boundedTitle(value: string | null | undefined): string | undefined {
  return nonEmpty(value);
}
