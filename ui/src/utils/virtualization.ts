import {
  observeElementRect,
  type Rect,
  type Virtualizer,
} from "@tanstack/solid-virtual";

/**
 * Preserve a bounded render window when an element is mounted before layout is
 * available (hidden panes, SSR hydration, jsdom). Real non-zero measurements
 * always win as soon as ResizeObserver reports them.
 */
export function elementRectObserver(fallback: Rect) {
  return <TScrollElement extends Element, TItemElement extends Element>(
    instance: Virtualizer<TScrollElement, TItemElement>,
    callback: (rect: Rect) => void,
  ): void | (() => void) => observeElementRect(instance, (rect) => {
    callback({
      width: rect.width > 0 ? rect.width : fallback.width,
      height: rect.height > 0 ? rect.height : fallback.height,
    });
  });
}
