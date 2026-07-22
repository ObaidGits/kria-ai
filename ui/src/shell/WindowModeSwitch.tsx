import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { Button, Popover } from "../kit";
import { shellStore, type WindowMode } from "../stores";
import { Icon } from "../components/Icon";
import { requestWindowMode } from "../windowing/modeTransitionCoordinator";

/**
 * WindowModeSwitch — a SECONDARY shell preference, not a primary task action
 * (UIE-H-009, UIE-L-004, Req 10.6). Mode changes carry low idle visual weight
 * and collapse into a concise labelled disclosure where available width
 * requires it, instead of a row of equally-weighted buttons competing with the
 * Composer and primary work.
 *
 * Contracts preserved here (presentation only — transition semantics belong to
 * the window-mode manager, task 4.5):
 *   • The current mode stays visible at all times (pressed state inline; on the
 *     disclosure trigger label when collapsed).
 *   • The Immersive EXIT stays explicit and always reachable in both layouts
 *     (Req 10.2 / 10.8; Escape still exits via the manager when unconsumed).
 *   • Every option is a real labelled button with aria-pressed state and full
 *     keyboard access.
 *
 * KRIA-owned control; never replaces or imitates host window decorations.
 */

interface ModeSpec {
  mode: WindowMode;
  label: string;
  icon: string;
  /** Concise purpose shown as a tooltip and inside the disclosure (UIE-L-004). */
  purpose: string;
}

// Canonical View Mode axis (design.md §8; Requirements 13.1, 13.6):
// Immersive / Standard / Mini / Companion. "Mini" supersedes the former
// "Compact" naming; "Companion" is the detached cross-application ember whose
// window/ember behaviour is owned by task 8.3.
const MODES: ReadonlyArray<ModeSpec> = [
  { mode: "standard", label: "Standard", icon: "monitor", purpose: "Normal shell chrome and spacing." },
  { mode: "mini", label: "Mini", icon: "minimize-2", purpose: "Compact quick-interaction window for side-by-side use." },
  { mode: "immersive", label: "Immersive", icon: "maximize-2", purpose: "Full-focus workspace; Esc returns to Standard." },
  { mode: "companion", label: "Companion", icon: "message-circle", purpose: "Floating ember that stays present across applications." },
];

/**
 * Collapse the inline options into a disclosure at constrained widths. Reuses
 * the shell's existing narrow breakpoint rather than introducing a new layout
 * store. Defaults to expanded when matchMedia is unavailable (e.g. jsdom), so
 * the control degrades to the fully-visible option group.
 */
const COLLAPSE_QUERY = "(max-width: 780px)";

function useCollapsed() {
  const [collapsed, setCollapsed] = createSignal(false);
  onMount(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    let mql: MediaQueryList | undefined;
    try {
      mql = window.matchMedia(COLLAPSE_QUERY);
    } catch {
      return;
    }
    const onChange = () => setCollapsed(!!mql?.matches);
    onChange();
    mql.addEventListener?.("change", onChange);
    onCleanup(() => mql?.removeEventListener?.("change", onChange));
  });
  return collapsed;
}

/** The mode buttons as an accessible group. `menu` renders the stacked,
 * purpose-annotated variant used inside the disclosure. */
function ModeOptions(props: { menu?: boolean }) {
  return (
    <div
      class="kria-window-modes__options"
      classList={{ "kria-window-modes__options--menu": props.menu }}
      role="group"
      aria-label="Window mode"
    >
      <For each={MODES}>
        {(item) => {
          const active = () => shellStore.windowMode() === item.mode;
          return (
            <Button
              variant={active() ? "secondary" : "ghost"}
              size="sm"
              class="kria-window-modes__button"
              aria-label={`${item.label} window mode`}
              aria-pressed={active()}
              title={item.purpose}
              onClick={() => requestWindowMode(item.mode)}
            >
              <Icon name={item.icon} size={14} aria-hidden={true} />
              <span class="kria-window-modes__label">{item.label}</span>
              <Show when={props.menu}>
                <span class="kria-window-modes__purpose">{item.purpose}</span>
              </Show>
            </Button>
          );
        }}
      </For>
    </div>
  );
}

export function WindowModeSwitch() {
  const collapsed = useCollapsed();
  const currentLabel = () =>
    MODES.find((item) => item.mode === shellStore.windowMode())?.label ?? "Standard";

  return (
    <div class="kria-window-modes" classList={{ "kria-window-modes--collapsed": collapsed() }}>
      <Show when={collapsed()} fallback={<ModeOptions />}>
        {/* Current mode stays visible on the trigger label; the options (with
            purpose descriptions) are disclosed on demand. */}
        <Popover triggerLabel={`Window mode: ${currentLabel()}`} title="Window mode" placement="bottom">
          <ModeOptions menu />
        </Popover>
      </Show>
      <Show when={shellStore.windowMode() === "immersive"}>
        <Button
          variant="secondary"
          size="sm"
          class="kria-window-modes__exit"
          aria-keyshortcuts="Escape"
          onClick={() => requestWindowMode("standard")}
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
