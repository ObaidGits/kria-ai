import { For, Show } from "solid-js";
import { Button } from "../kit";
import { shellStore, type WindowMode } from "../stores";
import { Icon } from "../components/Icon";

const MODES: ReadonlyArray<{ mode: WindowMode; label: string; icon: string }> = [
  { mode: "compact", label: "Compact", icon: "minimize-2" },
  { mode: "standard", label: "Standard", icon: "monitor" },
  { mode: "immersive", label: "Immersive", icon: "maximize-2" },
];

/** KRIA-owned mode control; never replaces or imitates host window decorations. */
export function WindowModeSwitch() {
  return (
    <div class="kria-window-modes" role="group" aria-label="Window mode">
      <For each={MODES}>
        {(item) => (
          <Button
            variant={shellStore.windowMode() === item.mode ? "secondary" : "ghost"}
            size="sm"
            class="kria-window-modes__button"
            aria-label={`${item.label} window mode`}
            aria-pressed={shellStore.windowMode() === item.mode}
            title={`${item.label} window mode`}
            onClick={() => shellStore.setWindowMode(item.mode)}
          >
            <Icon name={item.icon} size={14} aria-hidden={true} />
            <span class="kria-window-modes__label">{item.label}</span>
          </Button>
        )}
      </For>
      <Show when={shellStore.windowMode() === "immersive"}>
        <Button
          variant="secondary"
          size="sm"
          class="kria-window-modes__exit"
          aria-keyshortcuts="Escape"
          onClick={() => shellStore.setWindowMode("standard")}
          title="Exit Immersive (Esc)"
        >
          <Icon name="x" size={14} aria-hidden={true} />
          Exit Immersive
        </Button>
      </Show>
    </div>
  );
}

export default WindowModeSwitch;
