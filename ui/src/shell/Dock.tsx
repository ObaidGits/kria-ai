/**
 * Dock — the 7-Space navigation rail (Req 1.2). Selecting an item switches
 * Space in exactly ONE interaction (Req 1.3). Every item is a real <button>,
 * keyboard-operable, focus-visible (kit-focusable), labelled, and marks the
 * active Space with aria-current (Req 17.1/17.2).
 *
 * Requirements: 1.2, 1.3, 17.1, 17.2
 */
import { For } from "solid-js";
import { Icon } from "../components/Icon";
import { navigate, currentRoute, ALL_SPACES, type Space } from "./router";
import { SPACE_META } from "./spaces";
import "./AppShell.css";

export interface DockProps {
  /** Called after navigation (e.g. to mirror into shellStore). */
  onSelect?: (space: Space) => void;
}

export function Dock(props: DockProps) {
  const activeSpace = () => currentRoute().space;

  const select = (space: Space) => {
    navigate(space);
    props.onSelect?.(space);
  };

  return (
    <nav class="kria-dock" aria-label="Spaces">
      <ul class="kria-dock__list">
        <For each={ALL_SPACES}>
          {(space) => {
            const meta = SPACE_META[space];
            const isActive = () => activeSpace() === space;
            return (
              <li class="kria-dock__item">
                <button
                  type="button"
                  class="kria-dock__button kit-focusable kit-transition"
                  classList={{ "is-active": isActive() }}
                  aria-current={isActive() ? "page" : undefined}
                  aria-label={meta.label}
                  title={meta.label}
                  onClick={() => select(space)}
                >
                  <Icon name={meta.icon} size={20} />
                  <span class="kria-dock__label">{meta.label}</span>
                </button>
              </li>
            );
          }}
        </For>
      </ul>
    </nav>
  );
}

export default Dock;
