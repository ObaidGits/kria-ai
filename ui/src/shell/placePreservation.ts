/**
 * Place preservation (design.md §11.11.4, Req 13.4).
 *
 * "Approving/denying returns focus exactly where it was." When a blocking
 * interruption seizes focus (the Approval Center), the user's PLACE must survive
 * it: the focused control, the caret/selection inside a text field, and the
 * scroll offsets of scrollable regions. Drafts (task 3.4) and session state
 * (task 1.5) already persist content; this helper covers the transient,
 * in-flight place that those don't — so no interruption ever loses it.
 *
 * Usage (the shell wires this around the blocking interrupt):
 *   const snap = capturePlace();  // just before focus is seized
 *   …interruption resolves…
 *   restorePlace(snap);           // focus + caret + scroll restored
 *
 * Pure DOM, no framework coupling; safe under jsdom (used in tests).
 *
 * SINGLE restoration path (design §21 IU-10 / UIE-M-005, task 9.3): the
 * virtualized conversation viewport is NOT captured here. Raw `scrollTop` is the
 * wrong unit for a dynamic-measure virtual list — it does not map back to the
 * same message after a reversible transition. That viewport is owned by the
 * anchor-based conversation owner (`conversationPlace.ts`); this helper still
 * captures/restores focus + caret + every OTHER (non-virtualized) lane scroller
 * (threads/Work/Context/Inspector), so mode (P-A) and approval (P-B) transitions
 * keep those while deferring the stream to its single owner.
 */
import {
  CONVERSATION_SCROLL_OWNER_ATTR,
  CONVERSATION_SCROLL_OWNER_VALUE,
  CONVERSATION_VIEWPORT_CLASS,
} from "./spaces/converse/conversationPlace";

/** A restorable snapshot of the user's transient place. */
export interface PlaceSnapshot {
  /** The element that had focus (held directly so restore is exact). */
  activeElement: HTMLElement | null;
  /** Caret/selection range for a text input/textarea, if that was focused. */
  selectionStart: number | null;
  selectionEnd: number | null;
  selectionDirection: "forward" | "backward" | "none" | null;
  /** Scroll offsets for every scrollable element captured. */
  scroll: Array<{ el: Element; top: number; left: number }>;
}

function isTextEntry(el: Element | null): el is HTMLInputElement | HTMLTextAreaElement {
  return (
    el instanceof HTMLTextAreaElement ||
    (el instanceof HTMLInputElement && typeof el.selectionStart === "number")
  );
}

/**
 * Collect scrollable elements under `root` that currently have a non-zero
 * scroll offset (only those need restoring). Always includes the document
 * scrolling element.
 */
function collectScrollables(root: ParentNode): Element[] {
  const out: Element[] = [];
  const candidates = root.querySelectorAll<HTMLElement>("*");
  candidates.forEach((el) => {
    if (isConversationOwnedScroller(el)) return; // delegated to conversationPlace
    if (el.scrollTop > 0 || el.scrollLeft > 0) out.push(el);
  });
  if (typeof document !== "undefined" && document.scrollingElement) {
    out.push(document.scrollingElement);
  }
  return out;
}

/**
 * Whether an element is the virtualized conversation viewport, whose scroll
 * restoration is owned by `conversationPlace.ts` (anchor + offset, not raw px).
 * Matched by the explicit scroll-owner marker or the viewport class.
 */
export function isConversationOwnedScroller(el: Element): boolean {
  if (typeof (el as HTMLElement).getAttribute === "function") {
    if (el.getAttribute(CONVERSATION_SCROLL_OWNER_ATTR) === CONVERSATION_SCROLL_OWNER_VALUE) {
      return true;
    }
  }
  return el.classList?.contains(CONVERSATION_VIEWPORT_CLASS) ?? false;
}

/**
 * Capture the current place: focused element, its selection (if a text field),
 * and scroll offsets of scrollable regions under `root` (defaults to the
 * document body).
 */
export function capturePlace(root: ParentNode | null = typeof document !== "undefined" ? document.body : null): PlaceSnapshot {
  const activeElement =
    typeof document !== "undefined" && document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null;

  let selectionStart: number | null = null;
  let selectionEnd: number | null = null;
  let selectionDirection: PlaceSnapshot["selectionDirection"] = null;
  if (isTextEntry(activeElement)) {
    selectionStart = activeElement.selectionStart;
    selectionEnd = activeElement.selectionEnd;
    selectionDirection = activeElement.selectionDirection ?? null;
  }

  const scroll = root
    ? collectScrollables(root).map((el) => ({ el, top: el.scrollTop, left: el.scrollLeft }))
    : [];

  return { activeElement, selectionStart, selectionEnd, selectionDirection, scroll };
}

/**
 * Restore a previously captured place. Reinstates scroll offsets, returns focus
 * to the captured element (if it is still in the document), and restores the
 * caret/selection for text fields. Missing/detached targets are skipped safely.
 */
export function restorePlace(snapshot: PlaceSnapshot | null | undefined): void {
  if (!snapshot) return;

  for (const { el, top, left } of snapshot.scroll) {
    if (el.isConnected) {
      el.scrollTop = top;
      el.scrollLeft = left;
    }
  }

  const el = snapshot.activeElement;
  if (el && el.isConnected && typeof el.focus === "function") {
    el.focus();
    if (
      isTextEntry(el) &&
      snapshot.selectionStart !== null &&
      snapshot.selectionEnd !== null &&
      typeof el.setSelectionRange === "function"
    ) {
      try {
        el.setSelectionRange(
          snapshot.selectionStart,
          snapshot.selectionEnd,
          snapshot.selectionDirection ?? undefined
        );
      } catch {
        // Some input types disallow setSelectionRange — focus alone is enough.
      }
    }
  }
}
