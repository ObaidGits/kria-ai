/**
 * HiddenDock — the presence-homepage navigation rail (Req 7, design §7.1).
 *
 * Adapts the standing `Dock` into the "deliberate navigation" register of the
 * hybrid model: the Dock is VISUALLY ABSENT at rest (receded into the left
 * wall, never competing with the Core — Req 7.5) and is revealed ONLY on
 * explicit intent (Req 7.1):
 *
 *   • cursor reaching the left edge of the viewport,
 *   • holding Alt,
 *   • opening the Command Palette (⌘K / Ctrl K),
 *   • a pinned-open preference (`homeStore.dockPinned`),
 *   • keyboard / assistive-technology focus entering the rail.
 *
 * When revealed it renders over a DIMMED Room (the scrim) and is dismissed on
 * blur / Escape, returning focus per the §20.4 focus-return ladder (Req 7.4).
 *
 * ── Keyboard / AT reachable while hidden (Req 7.3 / 14.5) ─────────────────────
 * The rail is NEVER removed from the accessibility tree or the tab order while
 * hidden. It recedes via `transform` + `opacity` only (see `HiddenDock.css`) —
 * NEVER `display:none` / `visibility:hidden`, which would drop it from AT and
 * Tab. So a keyboard/AT user can Tab straight into it: `focusin` flips the
 * reveal, and CSS `:focus-within` guarantees it paints even if the store were
 * unavailable. Mouse-only discoverability is via the Command Palette (which
 * owns Space entries) plus the one-time first-run hint (Onboarding, task 9.2).
 *
 * ── Canonical order + one-click switch preserved (Req 7.2) ───────────────────
 * The seven Spaces, their canonical `ALL_SPACES` order, `aria-current` on the
 * active Space, and the one-interaction Space switch are ALL inherited verbatim
 * by RENDERING the existing `<Dock />` inside the reveal shell — the list is
 * never duplicated or reordered here.
 *
 * ── Failure degrades to the palette (design §14) ─────────────────────────────
 * This component only adds a reveal wrapper; it never intercepts ⌘K or the
 * router. If the reveal machinery fails (compositor/focus), the Command Palette
 * remains the guaranteed navigation path (Req 7.1 lists ⌘K as a first-class
 * reveal + the palette is independent).
 *
 * Pure presentation + local UI state (reads `homeStore` dock state +
 * `shellStore.paletteOpen`); no orchestration, no domain writes. Token-only,
 * zero raw color, reduced-motion safe.
 *
 * Requirements: 7.1, 7.2, 7.3, 7.4, 7.5, 14.4, 14.5
 */
import { createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { Dock, type DockProps } from "./Dock";
import { Icon } from "../components/Icon";
import { homeStore } from "../stores/homeStore";
import { shellStore } from "../stores/shellStore";
import { captureFocusOwner, returnFocus, type FocusReturnOwner } from "./focusReturn";
import "./HiddenDock.css";

/** Left-edge hot-zone (px) that reveals the rail on cursor approach (Req 7.1). */
const EDGE_REVEAL_PX = 6;

/**
 * Resolve the reduced-motion preference: the global kill-switch
 * (`data-reduced-motion="on"` on the root) wins, then the OS media query.
 * Mirrors `Room`/`CorePresence`/`ContextualChips` so the whole homepage freezes
 * together (Req 17.4 / 21.4).
 */
function detectReducedMotion(): boolean {
  if (typeof document !== "undefined") {
    const root = document.documentElement;
    if (root && root.getAttribute("data-reduced-motion") === "on") return true;
  }
  if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
    try {
      return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    } catch {
      return false;
    }
  }
  return false;
}

export interface HiddenDockProps {
  /** Forwarded to the inner `Dock` (post-navigation analytics hook only). */
  onSelect?: DockProps["onSelect"];
  /**
   * Force the static (reduced-motion) rendering. When omitted the component
   * derives it from the global kill-switch + OS `prefers-reduced-motion`.
   */
  reducedMotion?: boolean;
}

export function HiddenDock(props: HiddenDockProps) {
  let railRef: HTMLDivElement | undefined;
  // The element that had focus BEFORE focus entered the rail, so Escape can
  // return focus to it via the §20.4 ladder (Req 7.4). Captured on focus-in.
  let focusOwner: FocusReturnOwner | null = null;

  // Independent reveal-intent reasons (Req 7.1). Reveal = the union of these;
  // holding this locally (not one boolean) prevents flip-flop when two reasons
  // overlap (e.g. Alt held while focus is inside).
  const [altHeld, setAltHeld] = createSignal(false);
  const [edgeHover, setEdgeHover] = createSignal(false);
  const [focusWithin, setFocusWithin] = createSignal(false);
  const [reducedMotionDetected, setReducedMotionDetected] = createSignal(detectReducedMotion());

  const isStatic = () => props.reducedMotion ?? reducedMotionDetected();
  const pinned = () => homeStore.dockPinned();
  const paletteOpen = () => shellStore.paletteOpen();
  const revealed = () => homeStore.dockRevealed();

  // Drive the single authoritative store flag from the union of intents. The
  // store is the source of truth for `dockRevealed` (task 0.5); a pinned rail
  // stays revealed (the store ignores a hide while pinned).
  createEffect(() => {
    const anyIntent = pinned() || altHeld() || edgeHover() || focusWithin() || paletteOpen();
    homeStore.setDockRevealed(anyIntent);
  });

  // ── Focus (keyboard/AT) reveal + Escape focus-return ──────────────────────
  const onFocusIn = (event: FocusEvent) => {
    if (!focusWithin()) {
      // Capture the element losing focus (the true invoker) so Escape returns
      // to it — not to a control inside the rail.
      const prev = event.relatedTarget instanceof HTMLElement ? event.relatedTarget : null;
      focusOwner = captureFocusOwner(prev);
    }
    setFocusWithin(true);
  };

  const onFocusOut = (event: FocusEvent) => {
    const next = event.relatedTarget as Node | null;
    // Still moving between controls inside the rail → keep it revealed.
    if (railRef && next && railRef.contains(next)) return;
    // Focus left the rail entirely (Tab-out / click elsewhere): dismiss on blur
    // (Req 7.4). Focus already moved on its own, so no forced return here.
    setFocusWithin(false);
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key !== "Escape") return;
    if (!revealed() || pinned()) return; // a pinned rail is not Escape-dismissable
    // Escape while focus is in the rail: clear the transient reasons and return
    // focus to the pre-reveal owner via the §20.4 ladder (Req 7.4). `returnFocus`
    // moves focus out, which fires `focusout` → `focusWithin=false`.
    event.stopPropagation();
    setAltHeld(false);
    setEdgeHover(false);
    const owner = focusOwner;
    focusOwner = null;
    returnFocus(owner);
  };

  // ── Global intent listeners (Alt, left-edge cursor) ───────────────────────
  onMount(() => {
    const onDocKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Alt") setAltHeld(true);
    };
    const onDocKeyUp = (e: KeyboardEvent) => {
      if (e.key === "Alt") setAltHeld(false);
    };
    const onMouseMove = (e: MouseEvent) => {
      // While hidden the hot-zone is a thin left strip; while revealed the whole
      // rail width keeps it open, so the pointer can travel onto the buttons
      // without it collapsing. Leaving the zone hides it (mouse-reveal only —
      // never overrides a pin/Alt/focus reason).
      const width = revealed() ? (railRef?.getBoundingClientRect().width ?? 0) : EDGE_REVEAL_PX;
      setEdgeHover(e.clientX <= Math.max(width, EDGE_REVEAL_PX));
    };
    const onWindowBlur = () => {
      // Window lost focus (e.g. Alt+Tab): drop the transient hover/Alt reasons.
      setAltHeld(false);
      setEdgeHover(false);
    };

    document.addEventListener("keydown", onDocKeyDown);
    document.addEventListener("keyup", onDocKeyUp);
    document.addEventListener("mousemove", onMouseMove);
    window.addEventListener("blur", onWindowBlur);

    // Keep reduced-motion live (OS media query + the global kill-switch attr),
    // mirroring Room so the whole homepage freezes/thaws together.
    const onReducedMotionChange = () => setReducedMotionDetected(detectReducedMotion());
    let mql: MediaQueryList | undefined;
    if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
      try {
        mql = window.matchMedia("(prefers-reduced-motion: reduce)");
        mql.addEventListener("change", onReducedMotionChange);
      } catch {
        mql = undefined;
      }
    }
    let observer: MutationObserver | undefined;
    if (typeof MutationObserver !== "undefined" && typeof document !== "undefined") {
      observer = new MutationObserver(onReducedMotionChange);
      observer.observe(document.documentElement, {
        attributes: true,
        attributeFilter: ["data-reduced-motion"],
      });
    }

    onCleanup(() => {
      document.removeEventListener("keydown", onDocKeyDown);
      document.removeEventListener("keyup", onDocKeyUp);
      document.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("blur", onWindowBlur);
      mql?.removeEventListener("change", onReducedMotionChange);
      observer?.disconnect();
    });
  });

  const togglePinned = () => homeStore.setDockPinned(!pinned());

  return (
    <>
      {/* Dim-over-Room scrim (Req 7.4). Presentational + non-interactive so it
          never traps the pointer or competes with the Core; the room reads
          through it. Dismissal is by blur/Escape, not a scrim click. */}
      <div
        class="kria-hidden-dock__scrim"
        data-revealed={revealed() ? "true" : "false"}
        data-motion={isStatic() ? "static" : undefined}
        aria-hidden="true"
      />
      {/* The reveal shell. Receded (transform/opacity) at rest but ALWAYS in the
          a11y tree + tab order (Req 7.3): no display:none / visibility:hidden.
          `:focus-within` in CSS guarantees paint on keyboard entry even without
          the store. */}
      <div
        ref={railRef}
        class="kria-hidden-dock"
        classList={{ "is-pinned": pinned() }}
        data-region="hidden-dock"
        data-revealed={revealed() ? "true" : "false"}
        data-motion={isStatic() ? "static" : undefined}
        onFocusIn={onFocusIn}
        onFocusOut={onFocusOut}
        onKeyDown={onKeyDown}
        onMouseLeave={() => setEdgeHover(false)}
      >
        {/* Canonical 7-Space list reused verbatim — order + aria-current +
            one-click switch inherited from Dock (Req 7.2). */}
        <Dock onSelect={props.onSelect} />

        {/* Pinned-open preference (Req 7.1): keeps the rail revealed until an
            explicit unpin. Real <button>, keyboard-operable, labelled, with
            aria-pressed reflecting the current pin state. */}
        <button
          type="button"
          class="kria-hidden-dock__pin kit-focusable kit-transition"
          aria-pressed={pinned() ? "true" : "false"}
          aria-label={pinned() ? "Unpin navigation" : "Pin navigation open"}
          title={pinned() ? "Unpin navigation" : "Pin navigation open"}
          onClick={togglePinned}
        >
          <Icon name="pin" size={16} />
          <span class="kria-hidden-dock__pin-label">{pinned() ? "Unpin" : "Pin"}</span>
        </button>
      </div>
    </>
  );
}

export default HiddenDock;
