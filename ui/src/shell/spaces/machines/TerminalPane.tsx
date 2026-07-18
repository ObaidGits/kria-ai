/**
 * TerminalPane — the focused device's live terminal stream (task 9.1, Req 8.1).
 * Read-only in this task (streaming remote INPUT is the remote-desktop canvas,
 * task 9.2) so no dangerous input control is wired inert (architecture
 * invariant).
 *
 * KEYBOARD ACCESSIBLE (Req 17.1): the scroll body is a focusable
 * `role="log"` live region (`tabindex=0`) so keyboard users can focus it and
 * scroll with arrows/Page keys, with a visible focus ring. A labelled Detach
 * button closes the stream.
 *
 * SECURITY: terminal text is UNTRUSTED substrate output — rendered as escaped
 * text (Solid), never HTML.
 *
 * Requirements: 8.1, 17.1
 */
import { For, Show, createMemo } from "solid-js";
import { createVirtualizer } from "@tanstack/solid-virtual";
import { IconButton } from "../../../kit";
import { Icon } from "../../../components/Icon";
import type {
  DeviceTargetView,
  DeviceTerminalLine,
} from "../../../hooks/useDeviceStatus";
import { elementRectObserver } from "../../../utils/virtualization";
import "./machines.css";

export interface TerminalPaneProps {
  device: DeviceTargetView | null;
  lines: DeviceTerminalLine[];
  /** Cap rendered lines (defaults to the last 500). */
  maxLines?: number;
  onDetach?: () => void;
}

export function TerminalPane(props: TerminalPaneProps) {
  let scrollEl: HTMLDivElement | undefined;
  const visibleLines = createMemo(() => {
    const cap = props.maxLines ?? 500;
    const lines = props.lines;
    return lines.length <= cap ? lines : lines.slice(lines.length - cap);
  });
  const virtualizer = createVirtualizer({
    get count() { return visibleLines().length; },
    getScrollElement: () => scrollEl ?? null,
    estimateSize: () => 24,
    overscan: 10,
    getItemKey: (index) => visibleLines()[index]?.offset ?? index,
    initialRect: { width: 720, height: 320 },
    observeElementRect: elementRectObserver({ width: 720, height: 320 }),
  });

  return (
    <Show
      when={props.device}
      fallback={
        <div class="kria-terminal">
          <div class="kria-terminal__empty">
            Select a device to attach its terminal stream.
          </div>
        </div>
      }
    >
      {(device) => (
        <section class="kria-terminal" aria-label={`Terminal — ${device().displayName}`}>
          <header class="kria-terminal__head">
            <div class="kria-terminal__title">
              <span class="kria-terminal__name">{device().displayName}</span>
              <span class="kria-fleet__id">{device().targetId}</span>
            </div>
            <Show when={props.onDetach}>
              <IconButton
                icon="x"
                label="Detach terminal"
                size="sm"
                onClick={() => props.onDetach!()}
              />
            </Show>
          </header>

          <div
            ref={scrollEl}
            class="kria-terminal__body"
            role="log"
            aria-label={`Terminal output for ${device().displayName}`}
            aria-live="polite"
            tabindex={0}
            data-virtual-list="terminal-log"
          >
            <Show
              when={visibleLines().length > 0}
              fallback={
                <div class="kria-terminal__empty">
                  <Icon name="terminal" size={13} aria-hidden /> No terminal output yet.
                </div>
              }
            >
              <div class="kria-terminal__sizer" style={{ height: `${virtualizer.getTotalSize()}px` }}>
                <For each={virtualizer.getVirtualItems()}>
                  {(row) => {
                    const line = () => visibleLines()[row.index];
                    return (
                      <Show when={line()}>
                        <div
                          class={`kria-terminal__line kria-terminal__line--${line()!.stream}`}
                          data-index={row.index}
                          data-offset={line()!.offset}
                          ref={(el) => queueMicrotask(() => virtualizer.measureElement(el))}
                          style={{ transform: `translateY(${row.start}px)` }}
                        >
                          <span class="kria-terminal__offset">#{line()!.offset}</span>
                          <span class="kria-terminal__text">{line()!.text}</span>
                        </div>
                      </Show>
                    );
                  }}
                </For>
              </div>
            </Show>
          </div>
        </section>
      )}
    </Show>
  );
}

export default TerminalPane;
