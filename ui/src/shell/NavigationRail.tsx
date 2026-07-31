/**
 * NavigationRail — the single desktop navigation rail for every shell surface.
 * The seven Spaces remain in canonical router order; Home, Command Deck, and
 * Developer Observatory live on the orthogonal top-level surface axis.
 */
import { For, Show, batch, createSignal } from "solid-js";
import { Icon } from "../components/Icon";
import { bridgeInvokeOptional } from "../bridge/invoke";
import { shellStore, voiceStore, type WindowMode } from "../stores";
import { currentSurface, setSurface } from "../app/surface";
import { requestWindowMode } from "../windowing/modeTransitionCoordinator";
import { ALL_SPACES, currentRoute, navigate, type Space } from "./router";
import { SPACE_META } from "./spaces";
import { getTerm, type TermId } from "./terminology";
import "./AppShell.css";

export type NavigationGroup = "primary" | "supporting" | "system" | "utility";

export const SPACE_GROUP: Record<Space, NavigationGroup> = {
  converse: "primary",
  memory: "supporting",
  automations: "supporting",
  capabilities: "supporting",
  machines: "system",
  observatory: "system",
  settings: "utility",
};

const SPACE_TERM: Partial<Record<Space, TermId>> = {
  machines: "machines",
  observatory: "observatory",
  memory: "memory",
};

const MODE_ORDER: readonly WindowMode[] = ["standard", "immersive", "mini"];
const MODE_LABEL: Record<WindowMode, string> = {
  standard: "Normal",
  immersive: "Immersive",
  mini: "Mini",
  companion: "Companion",
};

export function spaceOutcome(space: Space): string | undefined {
  const id = SPACE_TERM[space];
  return id ? getTerm(id).outcome : undefined;
}

export interface NavigationRailProps {
  onSelect?: (space: Space) => void;
}
export function NavigationRail(props: NavigationRailProps) {
  const [expandedPreference, setExpandedPreference] = createSignal(false);
  const isMini = () => shellStore.windowMode() === "mini";
  const expanded = () => !isMini() && expandedPreference();
  const spaceIsCurrent = (space: Space) =>
    currentSurface() === "workspace" && currentRoute().space === space;

  const selectSpace = (space: Space) => {
    batch(() => {
      navigate(space);
      setSurface("workspace");
    });
    props.onSelect?.(space);
  };

  const toggleVoice = () => {
    if (voiceStore.active()) {
      voiceStore.deactivate();
      void bridgeInvokeOptional("stop_voice");
    } else {
      voiceStore.activate();
      void bridgeInvokeOptional("start_voice");
    }
  };

  const cycleMode = () => {
    const index = MODE_ORDER.indexOf(shellStore.windowMode());
    const next = index < 0 ? MODE_ORDER[0] : MODE_ORDER[(index + 1) % MODE_ORDER.length];
    void requestWindowMode(next, { durationMs: 200 });
  };

  return (
    <aside
      id="kria-navigation-rail"
      class="kria-navrail"
      data-expanded={expanded() ? "true" : "false"}
      data-window-mode={shellStore.windowMode()}
      aria-label="Primary navigation rail"
    >
      <div class="kria-navrail__head">
        <span class="kria-navrail__eyebrow" aria-hidden="true">Command rail</span>
        <button
          type="button"
          class="kria-navrail__expand kit-focusable"
          aria-controls="kria-navigation-rail"
          aria-expanded={expanded()}
          aria-label={expanded() ? "Collapse navigation rail" : "Expand navigation rail"}
          title={expanded() ? "Collapse navigation rail" : "Expand navigation rail"}
          disabled={isMini()}
          onClick={() => setExpandedPreference((value) => !value)}
        >
          <Icon name="chevron-right" size={16} />
        </button>
      </div>

      <button
        type="button"
        class="kria-navrail__button kria-navrail__home kit-focusable"
        classList={{ "is-active": currentSurface() === "home" }}
        aria-current={currentSurface() === "home" ? "page" : undefined}
        aria-label="Home — Command Center"
        title="Home — Command Center"
        onClick={() => setSurface("home")}
      >
        <span class="kria-navrail__icon"><Icon name="sparkles" size={19} /></span>
        <span class="kria-navrail__label">Home</span>
      </button>

      <nav class="kria-navrail__spaces" aria-label="Spaces">
        <ul class="kria-navrail__list">
          <For each={ALL_SPACES}>
            {(space, index) => {
              const meta = SPACE_META[space];
              const group = SPACE_GROUP[space];
              const outcome = spaceOutcome(space);
              const descriptionId = outcome ? `kria-navrail-desc-${space}` : undefined;
              const startsNewGroup =
                index() > 0 && SPACE_GROUP[ALL_SPACES[index() - 1]] !== group;
              return (
                <>
                  <Show when={startsNewGroup}>
                    <li class="kria-navrail__separator" role="presentation" aria-hidden="true" />
                  </Show>
                  <li class="kria-navrail__item" data-navigation-group={group}>
                    <button
                      type="button"
                      class="kria-navrail__button kit-focusable"
                      classList={{
                        "is-active": spaceIsCurrent(space),
                        "kria-navrail__button--primary": group === "primary",
                      }}
                      aria-current={spaceIsCurrent(space) ? "page" : undefined}
                      aria-label={meta.label}
                      aria-describedby={descriptionId}
                      title={outcome ? `${meta.label}: ${outcome}` : meta.label}
                      onClick={() => selectSpace(space)}
                    >
                      <span class="kria-navrail__icon"><Icon name={meta.icon} size={19} /></span>
                      <span class="kria-navrail__label">{meta.label}</span>
                      <Show when={outcome}>
                        <span id={descriptionId} class="kit-visually-hidden">{outcome}</span>
                      </Show>
                    </button>
                  </li>
                </>
              );
            }}
          </For>
        </ul>
      </nav>
      <div class="kria-navrail__tools" aria-label="Surface and utility controls">
        <button
          type="button"
          class="kria-navrail__button kit-focusable"
          classList={{ "is-active": currentSurface() === "command-deck" }}
          aria-current={currentSurface() === "command-deck" ? "page" : undefined}
          aria-label="Command Deck"
          title="Command Deck"
          onClick={() => setSurface("command-deck")}
        >
          <span class="kria-navrail__icon"><Icon name="layout-dashboard" size={18} /></span>
          <span class="kria-navrail__label">Command Deck</span>
        </button>
        <button
          type="button"
          class="kria-navrail__button kit-focusable"
          classList={{ "is-active": currentSurface() === "developer" }}
          aria-current={currentSurface() === "developer" ? "page" : undefined}
          aria-label="Developer Observatory"
          title="Developer Observatory"
          onClick={() => setSurface("developer")}
        >
          <span class="kria-navrail__icon"><Icon name="code-2" size={18} /></span>
          <span class="kria-navrail__label">Developer</span>
        </button>
        <button
          type="button"
          class="kria-navrail__button kit-focusable"
          aria-pressed={voiceStore.active()}
          aria-label={voiceStore.active() ? "Stop voice input" : "Speak to KRIA"}
          title={voiceStore.active() ? "Stop voice input" : "Speak to KRIA"}
          onClick={toggleVoice}
        >
          <span class="kria-navrail__icon"><Icon name="mic" size={18} /></span>
          <span class="kria-navrail__label">{voiceStore.active() ? "Stop voice" : "Voice"}</span>
        </button>
        <button
          type="button"
          class="kria-navrail__button kit-focusable"
          aria-label={`Window mode: ${MODE_LABEL[shellStore.windowMode()]}`}
          title="Cycle window mode"
          onClick={cycleMode}
        >
          <span class="kria-navrail__icon"><Icon name="monitor" size={18} /></span>
          <span class="kria-navrail__label">{MODE_LABEL[shellStore.windowMode()]}</span>
        </button>
      </div>
      <span class="kria-navrail__edge" aria-hidden="true" />
    </aside>
  );
}

export default NavigationRail;
