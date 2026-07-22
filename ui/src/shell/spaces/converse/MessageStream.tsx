/**
 * MessageStream — the virtualized conversation list (Req 16.2, design.md §1.10).
 *
 *  • Virtualized with `@tanstack/solid-virtual` so long threads only mount the
 *    visible window (+ overscan) → 60fps scroll, near-zero idle cost (Req 16).
 *  • Dynamic row measurement (bubbles vary in height with content + cards).
 *  • Auto-scrolls to the newest message UNLESS the user has scrolled up — then
 *    it holds position (place-preservation, Req 13.4) and offers a jump button.
 *
 * Pure presentation: reads converseStore.messages() and renders MessageBubble.
 * Selection state is local (which message shows its inline actions).
 */
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { createVirtualizer } from "@tanstack/solid-virtual";
import { converseStore } from "../../../stores";
import { IconButton } from "../../../kit";
import { elementRectObserver } from "../../../utils/virtualization";
import { MessageBubble } from "./MessageBubble";
import { InlineWorkTrace } from "./InlineWorkTrace";
import {
  CONVERSATION_SCROLL_OWNER_VALUE,
  computeConversationAnchor,
  registerConversationPlaceOwner,
  resolveConversationRestore,
  type ConversationAnchor,
  type RenderedRow,
} from "./conversationPlace";
import {
  computePageScrollTop,
  isEditableTarget,
  resolveTraversalIntent,
  shouldPerformFocusRestore,
  shouldQueueFocusRestore,
  shouldRevealFocusedRow,
} from "./streamTraversal";
import "./MessageStream.css";

/** Distance (px) from the bottom within which we consider the user "at bottom". */
const BOTTOM_THRESHOLD = 64;
/** Estimated bubble height before measurement (px). */
const ESTIMATED_ROW = 96;

export function MessageStream() {
  let scrollEl: HTMLDivElement | undefined;
  const messages = converseStore.messages;
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  // Whether new messages should pin the view to the bottom (default true).
  const [stick, setStick] = createSignal(true);
  // Focus preservation across virtualization row reuse (task 11.6, gap G5).
  // `lastFocusedMessageId` remembers which message last held focus; when its
  // row unmounts (virtualized away) and focus drops to the body, we queue a
  // restore in `pendingFocusRestoreId` and replay it once that message's row is
  // mounted again — see onFocusIn / onFocusOut and the restore effect below.
  const [lastFocusedMessageId, setLastFocusedMessageId] = createSignal<string | null>(null);
  const [pendingFocusRestoreId, setPendingFocusRestoreId] = createSignal<string | null>(null);

  const count = createMemo(() => messages().length);

  const virtualizer = createVirtualizer({
    get count() {
      return count();
    },
    getScrollElement: () => scrollEl ?? null,
    estimateSize: () => ESTIMATED_ROW,
    overscan: 6,
    getItemKey: (index) => messages()[index]?.id ?? index,
    // Keep a bounded fallback window until real layout becomes available.
    // Non-zero ResizeObserver measurements replace it immediately.
    initialRect: { width: 720, height: 600 },
    observeElementRect: elementRectObserver({ width: 720, height: 600 }),
  });

  function isAtBottom(): boolean {
    if (!scrollEl) return true;
    return scrollEl.scrollHeight - scrollEl.scrollTop - scrollEl.clientHeight <= BOTTOM_THRESHOLD;
  }

  function scrollToBottom(): void {
    const n = count();
    if (n === 0) return;
    virtualizer.scrollToIndex(n - 1, { align: "end" });
    // Belt-and-braces for environments without full layout (and post-measure).
    if (scrollEl) scrollEl.scrollTop = scrollEl.scrollHeight;
  }

  const onScroll = () => setStick(isAtBottom());

  // ── Keyboard traversal (task 9.6, gap G7; design §21 UIE-M-005) ────────────
  // The viewport is keyboard-focusable (tabindex=0) so Page/Home/End scroll it
  // predictably, but it NEVER traps focus: Tab and any key aimed at a child
  // editable control pass through untouched. Only Page/Home/End act, and only
  // when focus is not inside an input/textarea/select/contenteditable.
  function pageScroll(direction: -1 | 1): void {
    if (!scrollEl) return;
    scrollEl.scrollTop = computePageScrollTop(
      scrollEl.scrollTop,
      scrollEl.clientHeight,
      scrollEl.scrollHeight,
      direction,
    );
    setStick(isAtBottom());
  }

  function onKeyDown(event: KeyboardEvent): void {
    const intent = resolveTraversalIntent({
      key: event.key,
      editableFocus: isEditableTarget(event.target),
    });
    // `none` → Tab / arrows / Enter / editable-control keys pass through
    // untouched (no focus trap, no key hijack — a11y, Req 12.11).
    if (intent.kind === "none") return;
    event.preventDefault();
    switch (intent.kind) {
      case "top":
        // Home → first message (top); release follow so we don't snap back.
        setStick(false);
        virtualizer.scrollToIndex(0, { align: "start" });
        if (scrollEl) scrollEl.scrollTop = 0;
        break;
      case "bottom":
        // End → latest message (bottom); re-engage follow tail.
        setStick(true);
        scrollToBottom();
        break;
      case "page":
        pageScroll(intent.direction);
        break;
    }
  }

  // Focus reveal: when a control inside an off-screen message gains focus (Tab
  // into an action outside the visible band, or programmatic focus), reveal it
  // so it is never "focused but invisible". Only reveals when the row is not
  // fully visible → never fights follow/stick for an already-visible message.
  function onFocusIn(): void {
    const id = focusedMessageId();
    if (id == null) return;
    // Remember the message that owns focus so we can restore it if its row is
    // later virtualized away and reused (task 11.6). Focus landed validly, so
    // any queued restore for a prior loss is now moot.
    setLastFocusedMessageId(id);
    setPendingFocusRestoreId(null);
    const index = messages().findIndex((m) => m.id === id);
    if (index < 0) return;
    const rendered = virtualizer.getVirtualItems().find((vi) => vi.index === index);
    const reveal = shouldRevealFocusedRow(
      rendered ? { start: rendered.start, end: rendered.end } : null,
      scrollEl?.scrollTop ?? 0,
      scrollEl?.clientHeight ?? 0,
    );
    if (!reveal) return;
    virtualizer.scrollToIndex(index, { align: "auto" });
    // Revealing a mid-thread message releases stick; revealing the latest
    // message leaves the follow tail intact (End owns re-engaging it).
    if (index < count() - 1) setStick(false);
  }

  /** Whether a message's row is currently mounted in the virtual window. */
  function isMessageRowRendered(id: string | null): boolean {
    if (id == null) return false;
    return virtualizer.getVirtualItems().some((vi) => messages()[vi.index]?.id === id);
  }

  /** Whether focus currently rests on an element inside the stream viewport. */
  function focusIsInsideViewport(): boolean {
    if (!scrollEl || typeof document === "undefined") return false;
    const active = document.activeElement;
    return active instanceof HTMLElement && scrollEl.contains(active);
  }

  // When focus leaves the viewport with no destination (relatedTarget null) AND
  // the message that held focus is no longer rendered, treat it as focus lost
  // to virtualization row-reuse and queue a restore. An intentional Tab-away or
  // click elsewhere (relatedTarget present) is never queued, so we never yank
  // focus back from a deliberate move (task 11.6; Req 16.4, 12.11).
  function onFocusOut(event: FocusEvent): void {
    const last = lastFocusedMessageId();
    const queue = shouldQueueFocusRestore({
      relatedTargetPresent: event.relatedTarget != null,
      lastFocusedId: last,
      lastFocusedRowStillRendered: isMessageRowRendered(last),
    });
    if (queue) setPendingFocusRestoreId(last);
  }

  // ── Single anchor-based restoration owner (design §20/§21 IU-10, task 9.3) ──
  // MessageStream holds the virtualizer, so it is the ONE writer of this
  // viewport's scroll on restore. Mode (P-A) and approval (P-B) transitions
  // delegate here via conversationPlace's coordinator (they no longer touch
  // `.kria-stream__viewport.scrollTop`). Anchor = topmost visible (or focused)
  // message + intra-item offset; atBottom preserves the follow tail.

  /** The message whose row currently contains focus (a message action), if any. */
  function focusedMessageId(): string | null {
    if (!scrollEl || typeof document === "undefined") return null;
    const active = document.activeElement;
    if (!(active instanceof HTMLElement) || !scrollEl.contains(active)) return null;
    const row = active.closest<HTMLElement>(".kria-stream__row");
    const index = row?.dataset.index != null ? Number(row.dataset.index) : NaN;
    return Number.isInteger(index) ? messages()[index]?.id ?? null : null;
  }

  function renderedRows(): RenderedRow[] {
    return virtualizer.getVirtualItems().map((vi) => ({
      index: vi.index,
      id: messages()[vi.index]?.id ?? String(vi.index),
      start: vi.start,
      end: vi.end,
    }));
  }

  const disposeOwner = registerConversationPlaceOwner({
    capture(): ConversationAnchor | null {
      if (!scrollEl) return null;
      return computeConversationAnchor({
        activeThreadId: converseStore.activeThreadId(),
        rows: renderedRows(),
        viewportTop: scrollEl.scrollTop,
        atBottom: isAtBottom(),
        focusedMessageId: focusedMessageId(),
      });
    },
    restore(anchor) {
      const plan = resolveConversationRestore(anchor, (id) =>
        messages().findIndex((m) => m.id === id),
      );
      if (plan.kind === "noop" || !scrollEl) return;
      if (plan.kind === "bottom") {
        setStick(true);
        scrollToBottom();
        return;
      }
      // Anchored mid-thread: land on the anchor index, then apply the intra-item
      // offset so the landed position is within min(item, 24px) of the anchor.
      setStick(false);
      virtualizer.scrollToIndex(plan.index, { align: "start" });
      queueMicrotask(() => {
        if (!scrollEl) return;
        const row = virtualizer.getVirtualItems().find((vi) => vi.index === plan.index);
        const rowStart = row?.start ?? scrollEl.scrollTop;
        scrollEl.scrollTop = rowStart + plan.offsetWithinItem;
      });
    },
  });
  onCleanup(disposeOwner);

  onMount(() => scrollToBottom());

  // When the message count grows, follow the tail only if the user is pinned.
  createEffect(
    on(count, (n, prev) => {
      if (prev !== undefined && n > prev && stick()) {
        queueMicrotask(scrollToBottom);
      }
    }),
  );

  // Focus restoration (task 11.6, gap G5): once the message that lost focus to a
  // virtualization unmount is mounted again, and focus is still dropped, return
  // focus to that message's row so a reused DOM node never leaves focus on the
  // wrong message. Reruns as the virtual window changes (getVirtualItems is
  // reactive); performs at most one restore per loss, then clears the queue.
  createEffect(() => {
    virtualizer.getVirtualItems(); // track window changes
    const pending = pendingFocusRestoreId();
    if (
      !shouldPerformFocusRestore({
        pendingId: pending,
        isRendered: isMessageRowRendered(pending),
        focusInsideViewport: focusIsInsideViewport(),
      })
    ) {
      return;
    }
    const index = messages().findIndex((m) => m.id === pending);
    if (index < 0) return;
    queueMicrotask(() => {
      const row = scrollEl?.querySelector<HTMLElement>(
        `.kria-stream__row[data-index="${index}"] .kria-msg`,
      );
      if (!row) return;
      row.focus();
      setLastFocusedMessageId(pending);
      setPendingFocusRestoreId(null);
    });
  });

  onCleanup(() => {
    /* virtualizer disposes with the component; nothing else to release */
  });

  return (
    <div class="kria-stream" data-region="message-stream-virtual">
      {/* The `log`/`aria-live` region is owned by the ConverseSpace container
          that wraps this stream, so the viewport is a plain scroller (avoids a
          duplicate live region). */}
      <div
        ref={scrollEl}
        class="kria-stream__viewport"
        data-scroll-owner={CONVERSATION_SCROLL_OWNER_VALUE}
        tabindex={0}
        aria-label="Conversation, use Page Up and Page Down, Home and End to scroll"
        onScroll={onScroll}
        onKeyDown={onKeyDown}
        onFocusIn={onFocusIn}
        onFocusOut={onFocusOut}
      >
        <div class="kria-stream__sizer" style={{ height: `${virtualizer.getTotalSize()}px` }}>
          <For each={virtualizer.getVirtualItems()}>
            {(row) => {
              const message = () => messages()[row.index];
              return (
                <Show when={message()}>
                  <div
                    class="kria-stream__row"
                    data-index={row.index}
                    ref={(el) => queueMicrotask(() => virtualizer.measureElement(el))}
                    style={{ transform: `translateY(${row.start}px)` }}
                  >
                    <MessageBubble
                      message={message()!}
                      selected={selectedId() === message()!.id}
                      onSelect={setSelectedId}
                    />
                    {/* Inline per-turn activity trace (replaces the Work lane).
                        Rendered right after the user turn it belongs to; it
                        self-hides when the turn produced no work. Its live
                        growth is re-measured by the virtualizer's ResizeObserver
                        so scroll position stays stable. */}
                    <Show when={message()!.role === "user"}>
                      <InlineWorkTrace turnId={message()!.id} />
                    </Show>
                  </div>
                </Show>
              );
            }}
          </For>
        </div>
      </div>

      {/* Jump-to-latest — appears only when the user has scrolled up. */}
      <Show when={!stick()}>
        <div class="kria-stream__jump">
          <IconButton
            icon="chevron-down"
            label="Jump to latest message"
            onClick={() => {
              setStick(true);
              scrollToBottom();
            }}
          />
        </div>
      </Show>
    </div>
  );
}

export default MessageStream;
