/**
 * Composer (homepage) — the unified action target on the true vertical center
 * axis (design.md §2 / §6, Requirement 4.1 / 4.2 / 4.3).
 *
 * This is the homepage's SINGLE primary action target. Per design §2 the
 * Composer sits on the true vertical center axis — the one symmetrical anchor
 * the eye returns to — while the Core is offset in the upper third. There is
 * exactly ONE Composer on the homepage: when the presence homepage owns the
 * surface, the sticky Converse composer is suppressed (see `ConverseSpace`) and
 * this component is the only ask-field (Req 4.2 — "no second competing ask
 * field").
 *
 * ── Reuse, never duplicate (adapts the Converse Composer) ────────────────────
 * The unified text/command/voice input, the mic PEER input, attachments, and
 * the Send⇄Stop primary action are all owned by the existing Converse
 * `Composer` (`../converse/Composer`), which this component WRAPS rather than
 * re-implements. Both instances read the SAME per-thread draft
 * (`converseStore.composerDraft` / `updateDraft`), so a draft staged by a
 * Contextual Chip (task 4.3) or a starter appears here immediately (Req 4.3
 * staging surfaces in the one draft). Wrapping keeps a single source of truth
 * for the input behavior; this file adds ONLY the homepage-presence layer:
 *
 *   • Vertical-axis placement + bounded reading measure (design §2).
 *   • A discoverable `⌘K` / `Ctrl K` command hint (Req 4.2) that opens the
 *     Command Palette via `summon()` — routing/discoverability only, it never
 *     sends or executes (KRIA runtime-authority invariant).
 *   • Focus reaction (Req 4.3): when focus enters the Composer subtree it drives
 *     the meaningful-intent lean (`presenceIntent.setComposerFocused(true)` — so
 *     the Core leans toward the Composer, design §4.2) AND strengthens the
 *     Composer's own rim-light via a `data-composer-focused` flag the CSS reads
 *     against the shared-light `--core-*` variables (design §3.2). On blur out
 *     of the subtree both clear.
 *
 * ── Meaningful-intent only (Req 2.5) ─────────────────────────────────────────
 * The focus reaction fires on real focus in/out of the Composer — NOT on cursor
 * movement. There are no pointer/mousemove listeners here; the lean is a
 * discrete, meaningful-intent reaction exactly as §4.2 requires.
 *
 * ── Token-only, keyboard-operable, reduced-motion safe ───────────────────────
 * Styling is token-only (zero raw color, Req 16.2). The ⌘K hint is a real
 * `<button>` (Enter/Space activate natively, visible focus ring) with an
 * accessible name and `aria-keyshortcuts`. The rim-light and hint transitions
 * are opacity/tint only and freeze under reduced motion (CSS).
 *
 * Requirements: 4.1, 4.2, 4.3 (and 2.5 — meaningful-intent lean).
 */
import { createUniqueId, onCleanup } from "solid-js";

import ConverseComposer from "../converse/Composer";
import type { WidthProfile } from "../converseComposition";
import { presenceIntent } from "./sharedLight";
import { summon } from "../../../summon/summon";
import "./Composer.css";

/**
 * Platform-correct command-palette hint (Req 4.2). macOS shows `⌘K`; every
 * other platform shows `Ctrl K`. Mirrors the PresenceBar palette-trigger hint
 * so the two advertise the same proven summon chord (see `summon.ts`).
 */
export const SUMMON_HINT =
  typeof navigator !== "undefined" &&
  /mac|iphone|ipad|ipod/i.test(navigator.platform || navigator.userAgent || "")
    ? "⌘K"
    : "Ctrl K";

export interface HomeComposerProps {
  /**
   * Active Width Profile, forwarded to the wrapped Converse Composer so its
   * tool cluster adapts (task 8.6). Defaults to "full" so standalone/story/test
   * usage shows every tool inline.
   */
  widthProfile?: WidthProfile;
  /**
   * Command-palette opener for the ⌘K hint. Defaults to `summon()` (focus window
   * + open the palette). Routing/discoverability only — never sends/executes.
   * Overridable for tests/stories.
   */
  onOpenPalette?: () => void;
  class?: string;
}

/**
 * The homepage Composer. Wraps the Converse Composer on the vertical axis and
 * adds the ⌘K hint + focus→lean/rim-light presence reaction.
 */
export function Composer(props: HomeComposerProps) {
  let root: HTMLDivElement | undefined;
  const hintId = createUniqueId();

  const openPalette = (): void => (props.onOpenPalette ?? (() => summon()))();

  /**
   * Focus ENTERED the Composer subtree → meaningful-intent lean (Core leans in)
   * + strengthen the rim-light. `focusin` bubbles, so a single listener on the
   * root covers the textarea and every control.
   */
  const onFocusIn = (): void => {
    presenceIntent.setComposerFocused(true);
    root?.setAttribute("data-composer-focused", "true");
  };

  /**
   * Focus LEFT the Composer subtree → clear the lean + rim-light. `focusout`
   * fires when focus moves BETWEEN controls inside the Composer too, so we only
   * clear when the next focus target is outside the root (true blur).
   */
  const onFocusOut = (event: FocusEvent): void => {
    const next = event.relatedTarget as Node | null;
    if (next && root?.contains(next)) return;
    presenceIntent.setComposerFocused(false);
    root?.setAttribute("data-composer-focused", "false");
  };

  // Teardown: never leave the Core leaning at a stale focus if we unmount while
  // focused (e.g. a send transitions the homepage into Reading Mode).
  onCleanup(() => presenceIntent.setComposerFocused(false));

  return (
    <div
      ref={root}
      class={`kria-home-composer ${props.class ?? ""}`.trim()}
      data-region="composer"
      data-vertical-axis="true"
      data-composer-focused="false"
      onFocusIn={onFocusIn}
      onFocusOut={onFocusOut}
    >
      {/* The unified input, mic peer, attachments, and Send⇄Stop are the
          Converse Composer — reused, not duplicated. It reads the same
          per-thread draft, so staged chips/starters appear here (Req 4.3). */}
      <div class="kria-home-composer__field">
        <ConverseComposer widthProfile={props.widthProfile} />
      </div>

      {/* Discoverable ⌘K / Ctrl K command hint (Req 4.2). A real, labelled,
          keyboard-operable button that opens the Command Palette — routing /
          discoverability only; it never sends or executes. */}
      <button
        type="button"
        id={hintId}
        class="kria-home-composer__palette-hint"
        aria-label={`Open command palette (${SUMMON_HINT})`}
        aria-keyshortcuts="Meta+K Control+K"
        data-role="palette-hint"
        onClick={openPalette}
      >
        <span class="kria-home-composer__hint-text">Search & commands</span>
        <kbd class="kria-home-composer__kbd">{SUMMON_HINT}</kbd>
      </button>
    </div>
  );
}

export default Composer;
