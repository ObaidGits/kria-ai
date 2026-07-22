/**
 * Approval place snapshot + focus-return (design.md §20.3 / §20.4, gap G7).
 *
 * The Approval Center is KRIA's sole asynchronous Blocking_Interrupt: when a
 * decision becomes pending it may auto-open and seize focus. Design §20.3 names
 * the **AppShell approval place snapshot** as the `Focus_Return_Owner` "until the
 * queue clears". This module owns that snapshot/restore contract; AppShell wires
 * it around `approvalStore.hasPending()` (see AppShell boot).
 *
 * `placePreservation` already captures/restores the *transient* place (focused
 * control, caret/selection, scroll) for the happy path where the original
 * control still exists. The delta this module adds is the §20.4 focus-fallback
 * ladder for when the invoking element was removed by a route/profile/lane/state
 * change while the interrupt was up, plus the "never land on a destructive
 * action" guarantee:
 *
 *   1. original invoking element  — if still connected/visible and NOT destructive
 *   2. its owning region heading   — the region that contained the invoker
 *   3. its owning region container
 *   4. `#space-root`               — the primary workspace landmark
 *   5. a stable shell control      — last-resort shell anchor
 *
 * Focus never moves to an Approve/destructive control by default (§20.4).
 * Restoring place only touches focus + caret + scroll — it never resets draft,
 * route, selection, or work state.
 *
 * Pure DOM, no framework coupling; safe under jsdom (exercised by the tests).
 *
 * Requirements: 11.5, 13.4 (design §20.3 Focus_Return_Owner, §20.4 focus fallback)
 */
import { capturePlace, restorePlace, type PlaceSnapshot } from "../placePreservation";

/** A restorable approval place: the transient place plus its owning region. */
export interface ApprovalPlaceSnapshot {
  /** Transient place (focused control, caret, scroll) captured pre-interrupt. */
  place: PlaceSnapshot;
  /**
   * The region that owned the invoking element at capture time. Held directly
   * so the §20.4 fallback can still resolve the region even after the specific
   * invoking control is gone (as long as the region itself survives).
   */
  owningRegion: HTMLElement | null;
}

/** Containers that count as an "owning region" for the §20.4 fallback. */
const REGION_SELECTOR =
  '[data-owning-region], [role="region"], section, main, aside, nav, form, article';
const HEADING_SELECTOR = "h1, h2, h3, h4, h5, h6, [role='heading']";
/** Last-resort stable shell anchors, tried after `#space-root`. */
const STABLE_SHELL_SELECTOR = "[data-shell-root], .kria-skip-link";
/**
 * Destructive/approve controls that focus must never land on by default
 * (§20.4). Matched by explicit markers or accessible name.
 */
const DESTRUCTIVE_SELECTOR =
  '.kria-approval-card__approve, .kria-approval-card__deny, [data-destructive="true"], [data-approval-action]';
const DESTRUCTIVE_NAME = /\b(approve|approving|deny|delete|remove|discard|destroy)\b/i;

function owningRegionOf(el: HTMLElement | null): HTMLElement | null {
  return el?.closest<HTMLElement>(REGION_SELECTOR) ?? null;
}

function accessibleName(el: HTMLElement): string {
  return (el.getAttribute("aria-label") ?? el.textContent ?? "").trim();
}

/** Whether focus must avoid this element by default (§20.4 never-destructive). */
export function isDestructiveTarget(el: HTMLElement | null): boolean {
  if (!el) return false;
  if (typeof el.matches === "function" && el.matches(DESTRUCTIVE_SELECTOR)) return true;
  return DESTRUCTIVE_NAME.test(accessibleName(el));
}

/**
 * Whether an element can currently receive focus. Deliberately layout-free so
 * it is meaningful under jsdom: connected, not hidden/inert/aria-hidden.
 */
function isFocusable(el: HTMLElement | null): el is HTMLElement {
  if (!el || !el.isConnected) return false;
  if (el.hasAttribute("hidden")) return false;
  if (el.getAttribute("aria-hidden") === "true") return false;
  if (typeof el.closest === "function" && el.closest("[inert]")) return false;
  return typeof el.focus === "function";
}

/** Make a non-interactive container programmatically focusable (tabindex=-1). */
function ensureFocusable(el: HTMLElement): HTMLElement {
  if (!el.hasAttribute("tabindex")) el.setAttribute("tabindex", "-1");
  return el;
}

/**
 * Capture the approval place just before the interrupt seizes focus. Records
 * the transient place and resolves the invoking element's owning region so the
 * fallback ladder survives that control being removed.
 */
export function captureApprovalPlace(
  root: ParentNode | null = typeof document !== "undefined" ? document.body : null
): ApprovalPlaceSnapshot {
  const place = capturePlace(root);
  return { place, owningRegion: owningRegionOf(place.activeElement) };
}

/**
 * Resolve the §20.4 focus target: original invoker → owning region heading →
 * owning region container → `#space-root` → stable shell control. Destructive
 * candidates are skipped at every step.
 */
function resolveFocusTarget(snap: ApprovalPlaceSnapshot): HTMLElement | null {
  const original = snap.place.activeElement;
  if (isFocusable(original) && !isDestructiveTarget(original)) return original;

  const region =
    snap.owningRegion && snap.owningRegion.isConnected ? snap.owningRegion : null;
  if (region) {
    const heading = region.querySelector<HTMLElement>(HEADING_SELECTOR);
    if (isFocusable(heading) && !isDestructiveTarget(heading)) return ensureFocusable(heading);
    if (!isDestructiveTarget(region)) return ensureFocusable(region);
  }

  if (typeof document !== "undefined") {
    const spaceRoot = document.querySelector<HTMLElement>("#space-root");
    if (isFocusable(spaceRoot) && !isDestructiveTarget(spaceRoot)) return spaceRoot;

    const shell = document.querySelector<HTMLElement>(STABLE_SHELL_SELECTOR);
    if (isFocusable(shell) && !isDestructiveTarget(shell)) return ensureFocusable(shell);
  }

  return null;
}

/**
 * Restore the approval place once the pending queue clears. Reinstates scroll
 * offsets (never a state reset), then returns focus following the §20.4 ladder.
 * When the original invoker is the target, its caret/selection is restored too.
 */
export function restoreApprovalPlace(snap: ApprovalPlaceSnapshot | null | undefined): void {
  if (!snap) return;

  const target = resolveFocusTarget(snap);

  // Original invoker survived — exact restore (scroll + focus + caret).
  if (target && target === snap.place.activeElement) {
    restorePlace(snap.place);
    return;
  }

  // Fallback path: still preserve scroll place, then focus the fallback anchor.
  for (const { el, top, left } of snap.place.scroll) {
    if (el.isConnected) {
      el.scrollTop = top;
      el.scrollLeft = left;
    }
  }
  if (target) target.focus();
}
