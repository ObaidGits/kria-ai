import { Component, createSignal, Show } from "solid-js";
import type { QualityPreset } from "../remoteDesktopApi";
import type { TouchMode } from "../rdpInput";

/**
 * Clean, collapsible remote-desktop toolbar.
 *
 * Primary actions are always visible (keyboard, fit/zoom reset, disconnect);
 * secondary actions (fullscreen, touch-mode, quality, reconnect, stats) live in
 * a "More" popover so the bar stays uncluttered on narrow screens. The whole
 * bar can auto-collapse in landscape/fullscreen and be re-summoned by an edge
 * handle (the `collapsed`/`onExpand` props are driven by the view).
 */
export interface RdToolbarProps {
  collapsed: () => boolean;
  onExpand: () => void;
  onToggleKeyboard: () => void;
  onFitReset: () => void;
  onDisconnect: () => void;
  onFullscreen: () => void;
  onReconnect: () => void;
  touchMode: () => TouchMode;
  onToggleTouchMode: () => void;
  quality: () => QualityPreset;
  onCycleQuality: () => void;
  showStats: () => boolean;
  onToggleStats: () => void;
}

const RdToolbar: Component<RdToolbarProps> = (props) => {
  const [moreOpen, setMoreOpen] = createSignal(false);

  return (
    <Show
      when={!props.collapsed()}
      fallback={
        <button
          class="mobile-desktop-toolbar-handle"
          aria-label="Show toolbar"
          onClick={props.onExpand}
        >
          ⋯
        </button>
      }
    >
      <div class="mobile-desktop-toolbar" role="toolbar" aria-label="Remote desktop controls">
        <button aria-label="Toggle keyboard" onClick={props.onToggleKeyboard}>
          ⌨︎
        </button>
        <button aria-label="Fit to screen / reset zoom" onClick={props.onFitReset}>
          ⤢
        </button>

        <div class="mobile-desktop-toolbar-more">
          <button
            aria-label="More controls"
            aria-expanded={moreOpen()}
            classList={{ active: moreOpen() }}
            onClick={() => setMoreOpen(!moreOpen())}
          >
            ⋯
          </button>
          <Show when={moreOpen()}>
            <div class="mobile-desktop-toolbar-menu" role="menu">
              <button role="menuitem" onClick={() => { props.onFullscreen(); setMoreOpen(false); }}>
                ⛶ Fullscreen
              </button>
              <button
                role="menuitem"
                aria-label={`Touch mode: ${props.touchMode()}`}
                onClick={() => props.onToggleTouchMode()}
              >
                {props.touchMode() === "direct" ? "👆 Direct" : "🖱 Trackpad"}
              </button>
              <button
                role="menuitem"
                aria-label={`Quality: ${props.quality()}`}
                onClick={() => props.onCycleQuality()}
              >
                ⚙ Quality: {props.quality()}
              </button>
              <button
                role="menuitem"
                classList={{ active: props.showStats() }}
                onClick={() => props.onToggleStats()}
              >
                📊 Stats
              </button>
              <button role="menuitem" onClick={() => { props.onReconnect(); setMoreOpen(false); }}>
                ⟳ Reconnect
              </button>
            </div>
          </Show>
        </div>

        <button
          class="danger mobile-desktop-toolbar-disconnect"
          aria-label="Disconnect / kill switch"
          onClick={props.onDisconnect}
        >
          ⏻
        </button>
      </div>
    </Show>
  );
};

export default RdToolbar;
