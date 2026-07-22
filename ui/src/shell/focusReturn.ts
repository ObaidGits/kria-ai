/**
 * Focus-return owner + §20.4 fallback ladder for controlled overlays (gap G4).
 *
 * The kit Dialog returns focus to its own `triggerRef` on close — but only when
 * it renders its *own* trigger (`triggerLabel`). When a Dialog is driven by the
 * `open` prop (the controlled ModalHost path), there is no internal trigger, so
 * focus would be lost on close. Design §20.3 names the required owner:
 *
 *   ModalHost / kit Dialog row — "Focus_Return_Owner = Self-trigger when
 *   present; otherwise opener/ModalHost must capture explicit owner before open."
 *   Approval-confirmation-in-ModalHost row — "Focus_Return_Owner = Originating
 *   ApprovalCard decision control."
 *
 * This module owns that *controlled/opener* contract: capture the opener (the
 * element that had focus when the modal opened, e.g. the button whose onClick
 * called `openModal`) and, on close, return focus to it following the §20.4
 * ladder:
 *
 *   1. the opener                — if still connected/visible (and, for a
 *                                  generic modal, not a destructive control)
 *   2. its owning region heading — the region that contained the opener
 *   3. its owning region container
 *   4. `#space-root`             — the primary workspace landmark
 *   5. a stable shell control    — last-resort shell anchor
 *
 * Never-destructive (§20.4): a *fallback* anchor is never a destructive/Approve
 * control. The opener itself is the deliberately chosen owner, so for the
 * "approval-confirm" layer — whose §20.3 owner IS the originating decision
 * control (the Approve button) — returning to that opener is allowed even
 * though it is destructive (`allowDestructiveOpener`). Generic modals skip a
 * destructive opener and fall through the ladder.
 *
 * Distinct owner from the AppShell approval place snapshot (task 8.3,
 * `approvalPlace.ts`): that snapshot owns focus for the asynchronous Approval
 * Center interrupt "until the queue clears"; this owns the synchronous,
 * user/application-initiated ModalHost open/close lifecycle. Reuses only
 * `isDestructiveTarget` from that module. Returning focus never resets draft,
 * route, selection, scroll, or work state (§20.4).
 *
 * Pure DOM, no framework coupling; safe under jsdom (exercised by the tests).
 * Intended to be reused by the CommandPalette (G5) and InspectorHost (G6)
 * close-focus-return work in task 8.9.
 *
 * Requirements: 4.4, 11.5 (design §20.3 Focus_Return_Owner, §20.4 focus fallback)
 */
import { isDestructiveTarget } from "./approvals/approvalPlace";

/** Containers that count as an "owning region" for the §20.4 fallback. */
const REGION_SELECTOR =
  '[data-owning-region], [role="region"], section, main, aside, nav, form, article';
const HEADING_SELECTOR = "h1, h2, h3, h4, h5, h6, [role='heading']";
/** Last-resort stable shell anchors, tried after `#space-root`. */
const STABLE_SHELL_SELECTOR = "[data-shell-root], .kria-skip-link";

/**
 * A captured focus-return owner: the opener plus the region that contained it
 * at capture time (held directly so the §20.4 ladder still resolves the region
 * even after the specific opener control is removed).
 */
export interface FocusReturnOwner {
  opener: HTMLElement | null;
  owningRegion: HTMLElement | null;
}

function owningRegionOf(el: HTMLElement | null): HTMLElement | null {
  return el?.closest<HTMLElement>(REGION_SELECTOR) ?? null;
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
 * Capture the current focus owner. `opener` defaults to `document.activeElement`
 * — the element focused when the modal opens (the control whose handler called
 * `openModal`). Its owning region is resolved now so the fallback survives that
 * control being removed while the modal is up.
 *
 * `regionOverride` names a stable owning region explicitly. This is for
 * PROGRAMMATIC opens (route/deep-link/reactive) where `document.activeElement`
 * is NOT the semantic invoking control (§20.3 InspectorHost Focus_Return_Owner
 * = "Invoking control, or nearest stable owning region if removed"). Pass
 * `opener = null` with a `regionOverride` so the §20.4 ladder resolves to that
 * stable region (its heading/container) instead of a stray active element. When
 * `regionOverride` is itself a region it is used directly; otherwise its nearest
 * owning region is resolved.
 */
export function captureFocusOwner(
  opener: HTMLElement | null =
    typeof document !== "undefined" && document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null,
  regionOverride?: HTMLElement | null,
): FocusReturnOwner {
  const owningRegion = regionOverride
    ? owningRegionOf(regionOverride) ?? regionOverride
    : owningRegionOf(opener);
  return { opener, owningRegion };
}

/**
 * Resolve the §20.4 focus target: opener → owning region heading → owning
 * region container → `#space-root` → stable shell control. Fallback anchors are
 * never destructive. `allowDestructiveOpener` permits returning to a
 * destructive opener when it is the designated §20.3 owner (approval-confirm's
 * originating decision control).
 */
export function resolveFocusReturnTarget(
  owner: FocusReturnOwner,
  { allowDestructiveOpener = false }: { allowDestructiveOpener?: boolean } = {},
): HTMLElement | null {
  const opener = owner.opener;
  if (isFocusable(opener) && (allowDestructiveOpener || !isDestructiveTarget(opener))) {
    return opener;
  }

  const region = owner.owningRegion && owner.owningRegion.isConnected ? owner.owningRegion : null;
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
 * Return focus to the resolved §20.4 target, deferred a microtask so the
 * overlay has torn down (matching the kit Dialog's own close timing) and any
 * background inertness is lifted first. Only touches focus — never draft, route,
 * selection, scroll, or work state.
 */
export function returnFocus(
  owner: FocusReturnOwner | null | undefined,
  options: { allowDestructiveOpener?: boolean } = {},
): void {
  if (!owner) return;
  queueMicrotask(() => {
    const target = resolveFocusReturnTarget(owner, options);
    target?.focus();
  });
}
