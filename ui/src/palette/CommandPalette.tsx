/**
 * CommandPalette — the in-house Solid command palette (design.md §1.12, Req 2).
 *
 * A11y model: an accessible modal dialog (role="dialog" + aria-modal, focus
 * trap, Escape to close — Req 17.6) wrapping an ARIA combobox+listbox for the
 * query/results (correct roles, `aria-activedescendant`, full keyboard nav —
 * Req 2.3 / 17.1). kbar/cmdk are React and are intentionally NOT used.
 *
 * Why not Kobalte's Dialog here: this surface is *externally controlled* by
 * `shellStore.paletteOpen`. Kobalte's controlled Dialog cannot mount under
 * jsdom (its DismissableLayer reads a not-yet-assigned content ref → null; see
 * shell/AppShell.test.tsx), which would make the required component tests
 * impossible. The palette is a global singleton, so a small hand-rolled dialog
 * with a proper focus trap + Escape gives the same a11y guarantees while
 * remaining fully testable. Kobalte still backs the kit primitives elsewhere.
 *
 * Modes (Req 2.2):
 *   • Go     — navigate to a Space/setting/memory/workflow/etc. (default)
 *   • Do     — run a registered UI command or keyboard shortcut
 *   • Ask    — send a message to KRIA (through the normal pipeline; dispatch.ts)
 *   • Change — natural-language settings change (routed to Settings)
 * Switch modes via the header chips (keyboard-reachable) or a leading prefix
 * token in the query (">" Do, "?" Ask, "~" Change).
 *
 * Instant open (Req 2.1): the component is mounted once in the AppShell overlay
 * layer and toggled via `shellStore.paletteOpen` — there is no lazy chunk loaded
 * on open, so opening is a signal flip, not a fetch.
 *
 * Requirements: 2.1, 2.2, 2.3, 2.4, 17.1, 17.2, 17.6
 */
import { createSignal, createMemo, createEffect, For, Show, batch } from "solid-js";
import { Portal } from "solid-js/web";
import { Icon } from "../components/Icon";
import { shellStore } from "../stores";
import { MODES, modeDef, parseQuery, type ParsedQuery } from "./modes";
import { collectItems } from "./sources";
import { searchItems, groupResults, flattenGroups } from "./search";
import { clearRecents, recordUse } from "./recents";
import { dispatchAsk, dispatchChange } from "./dispatch";
import type { PaletteItem, PaletteMode } from "./types";
import "./CommandPalette.css";
import "../adaptive/adaptive.css";

/** A free-text mode shows a submit affordance instead of an item list. */
function isTextMode(mode: PaletteMode): boolean {
  return mode === "ask" || mode === "change";
}

const FOCUSABLE =
  'button:not([disabled]), [href], input, [tabindex]:not([tabindex="-1"])';

export function CommandPalette() {
  const [baseMode, setBaseMode] = createSignal<PaletteMode>("go");
  const [rawInput, setRawInput] = createSignal("");
  const [activeIndex, setActiveIndex] = createSignal(0);
  let inputRef: HTMLInputElement | undefined;
  let listRef: HTMLDivElement | undefined;
  let panelRef: HTMLDivElement | undefined;

  const open = () => shellStore.paletteOpen();

  const parsed = createMemo<ParsedQuery>(() => parseQuery(rawInput(), baseMode()));
  const mode = () => parsed().mode;
  const searchText = () => parsed().text;

  // Live items for the effective mode (reads store signals → reactive).
  const items = createMemo<PaletteItem[]>(() =>
    isTextMode(mode()) ? [] : collectItems(mode())
  );
  const groups = createMemo(() => groupResults(searchItems(items(), searchText())));
  const flat = createMemo(() => flattenGroups(groups()));

  // Keep the active index in range whenever the result set changes.
  createEffect(() => {
    const len = flat().length;
    if (activeIndex() >= len) setActiveIndex(len > 0 ? len - 1 : 0);
  });

  // Reset transient state each time the palette opens, and focus the input so
  // the user can type immediately (Req 2.1 keyboard-first).
  createEffect(() => {
    if (open()) {
      batch(() => {
        setRawInput("");
        setBaseMode("go");
        setActiveIndex(0);
      });
      queueMicrotask(() => inputRef?.focus());
    }
  });

  // Scroll the active option into view on keyboard navigation.
  createEffect(() => {
    const idx = activeIndex();
    if (!open() || !listRef) return;
    const el = listRef.querySelector<HTMLElement>(`[data-index="${idx}"]`);
    el?.scrollIntoView?.({ block: "nearest" });
  });

  function close(): void {
    shellStore.setPaletteOpen(false);
  }

  function selectItem(item: PaletteItem): void {
    recordUse(item.id);
    close();
    item.run();
  }

  function submitText(): void {
    const text = searchText().trim();
    if (!text) return;
    const m = mode();
    close();
    if (m === "ask") dispatchAsk(text);
    else if (m === "change") dispatchChange(text);
  }

  function selectMode(next: PaletteMode): void {
    batch(() => {
      setBaseMode(next);
      // Strip any conflicting prefix so the chip choice wins.
      setRawInput(searchText());
      setActiveIndex(0);
    });
    inputRef?.focus();
  }

  function onInputKeyDown(e: KeyboardEvent): void {
    if (isTextMode(mode())) {
      if (e.key === "Enter") {
        e.preventDefault();
        submitText();
      }
      return;
    }
    const len = flat().length;
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        setActiveIndex((i) => (len === 0 ? 0 : Math.min(len - 1, i + 1)));
        break;
      case "ArrowUp":
        e.preventDefault();
        setActiveIndex((i) => (len === 0 ? 0 : Math.max(0, i - 1)));
        break;
      case "Home":
        e.preventDefault();
        setActiveIndex(0);
        break;
      case "End":
        e.preventDefault();
        setActiveIndex(len > 0 ? len - 1 : 0);
        break;
      case "Enter": {
        e.preventDefault();
        const r = flat()[activeIndex()];
        if (r) selectItem(r.item);
        break;
      }
    }
  }

  // Dialog-level keys: Escape closes; Tab is trapped within the panel (Req 17.6).
  function onPanelKeyDown(e: KeyboardEvent): void {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
      return;
    }
    if (e.key === "Tab" && panelRef) {
      const focusables = Array.from(
        panelRef.querySelectorAll<HTMLElement>(FOCUSABLE)
      ).filter((el) => el.offsetParent !== null || el === document.activeElement);
      if (focusables.length === 0) return;
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      const activeEl = document.activeElement as HTMLElement | null;
      if (e.shiftKey && activeEl === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && activeEl === last) {
        e.preventDefault();
        first.focus();
      }
    }
  }

  const optionId = (index: number): string => `kria-palette-option-${index}`;
  const indexOfResult = (itemId: string): number =>
    flat().findIndex((r) => r.item.id === itemId);

  return (
    <Show when={open()}>
      <Portal>
        <div class="kria-palette__overlay" aria-hidden={true} onClick={close} />
        <div class="kria-palette__positioner">
          <div
            ref={panelRef}
            class="kria-palette"
            role="dialog"
            aria-modal="true"
            aria-label="Command palette"
            onKeyDown={onPanelKeyDown}
          >
            {/* Mode chips — keyboard-reachable mode switch (Req 2.2/2.3). */}
            <div class="kria-palette__modes" role="tablist" aria-label="Palette mode">
              <For each={MODES}>
                {(m) => (
                  <button
                    type="button"
                    role="tab"
                    aria-selected={mode() === m.mode}
                    class="kria-palette__mode kit-focusable"
                    classList={{ "is-active": mode() === m.mode }}
                    onClick={() => selectMode(m.mode)}
                  >
                    <Icon name={m.icon} size={14} aria-hidden={true} />
                    <span>{m.label}</span>
                  </button>
                )}
              </For>
            </div>

            {/* Query input — ARIA combobox driving the listbox below. */}
            <div class="kria-palette__search">
              <Icon name="search" size={18} aria-hidden={true} />
              <input
                ref={inputRef}
                class="kria-palette__input kit-focusable"
                type="text"
                role="combobox"
                aria-expanded={true}
                aria-controls="kria-palette-listbox"
                aria-activedescendant={
                  !isTextMode(mode()) && flat().length > 0
                    ? optionId(activeIndex())
                    : undefined
                }
                aria-label={modeDef(mode()).hint}
                autocomplete="off"
                spellcheck={false}
                placeholder={modeDef(mode()).placeholder}
                value={rawInput()}
                onInput={(e) => {
                  setRawInput(e.currentTarget.value);
                  setActiveIndex(0);
                }}
                onKeyDown={onInputKeyDown}
              />
            </div>

            {/* Results / free-text affordance. */}
            <div
              ref={listRef}
              id="kria-palette-listbox"
              role="listbox"
              aria-label="Results"
              class="kria-palette__results"
            >
              <Show when={isTextMode(mode())}>
                <div class="kria-palette__texthint">
                  <Show
                    when={searchText().trim().length > 0}
                    fallback={
                      <p class="kria-palette__empty">
                        {mode() === "ask"
                          ? "Type a message and press Enter to send it to KRIA."
                          : "Describe the change and press Enter (opens Settings)."}
                      </p>
                    }
                  >
                    <button
                      type="button"
                      class="kria-palette__submit kit-focusable"
                      onClick={submitText}
                    >
                      <Icon name={modeDef(mode()).icon} size={16} aria-hidden={true} />
                      <span>
                        {mode() === "ask" ? "Ask KRIA: " : "Change: "}
                        <strong>{searchText().trim()}</strong>
                      </span>
                    </button>
                  </Show>
                </div>
              </Show>

              <Show when={!isTextMode(mode())}>
                <Show
                  when={flat().length > 0}
                  fallback={<p class="kria-palette__empty">No matches. Try another mode or query.</p>}
                >
                  <For each={groups()}>
                    {(group) => (
                      <div class="kria-palette__group" role="group" aria-label={group.label}>
                        <div class="kria-palette__grouplabel" aria-hidden={true}>
                          {group.label}
                        </div>
                        <For each={group.results}>
                          {(result) => {
                            const idx = () => indexOfResult(result.item.id);
                            return (
                              <div
                                id={optionId(idx())}
                                data-index={idx()}
                                role="option"
                                aria-selected={idx() === activeIndex()}
                                class="kria-palette__option"
                                classList={{ "is-active": idx() === activeIndex() }}
                                onPointerMove={() => setActiveIndex(idx())}
                                onClick={() => selectItem(result.item)}
                              >
                                <Show when={result.item.icon}>
                                  <Icon
                                    class="kria-palette__optionicon"
                                    name={result.item.icon!}
                                    size={16}
                                    aria-hidden={true}
                                  />
                                </Show>
                                <span class="kria-palette__optiontext">
                                  <span class="kria-palette__optiontitle">{result.item.title}</span>
                                  <Show when={result.item.subtitle}>
                                    <span class="kria-palette__optionsub">{result.item.subtitle}</span>
                                  </Show>
                                </span>
                                <Show when={result.item.shortcutHint}>
                                  <kbd class="kria-palette__kbd">{result.item.shortcutHint}</kbd>
                                </Show>
                              </div>
                            );
                          }}
                        </For>
                      </div>
                    )}
                  </For>
                </Show>
              </Show>
            </div>

            {/* Keyboard help + explicit explanation/reset for adaptive ordering. */}
            <div class="kria-palette__footer">
              <span><kbd class="kria-palette__kbd">↑</kbd><kbd class="kria-palette__kbd">↓</kbd> navigate</span>
              <span><kbd class="kria-palette__kbd">↵</kbd> select</span>
              <span><kbd class="kria-palette__kbd">Esc</kbd> close</span>
              <span class="kria-palette__footerhint">Match order adapts from items you use.</span>
              <button
                type="button"
                class="kria-adaptive-reset kit-focusable"
                onClick={() => clearRecents()}
              >
                Reset ranking
              </button>
            </div>
          </div>
        </div>
      </Portal>
    </Show>
  );
}

export default CommandPalette;
