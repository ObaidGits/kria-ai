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

  onMount(() => scrollToBottom());

  // When the message count grows, follow the tail only if the user is pinned.
  createEffect(
    on(count, (n, prev) => {
      if (prev !== undefined && n > prev && stick()) {
        queueMicrotask(scrollToBottom);
      }
    }),
  );

  onCleanup(() => {
    /* virtualizer disposes with the component; nothing else to release */
  });

  return (
    <div class="kria-stream" data-region="message-stream-virtual">
      {/* The `log`/`aria-live` region is owned by the ConverseSpace container
          that wraps this stream, so the viewport is a plain scroller (avoids a
          duplicate live region). */}
      <div ref={scrollEl} class="kria-stream__viewport" onScroll={onScroll}>
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
