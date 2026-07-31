/**
 * CommandDeck — KRIA's operational Mission Control (Phase 7).
 *
 * Homepage = presence; Command Deck = operations; Developer Observatory =
 * engineering. This surface answers "what is KRIA doing?" — never "who is KRIA?".
 *
 * It composes a context-aware Mission Header (one-glance operational state) over
 * a designed operational flow (Current Activity → Running Operations → Mission
 * Status → Upcoming), laid out via CSS grid areas rather than an undifferentiated
 * card grid. Panels are still the relocated command-center panels, placed by
 * their registry `region`; the deck owns layout + hierarchy, not panel internals.
 * It is context-aware via the shared Context Engine (no duplicated context logic).
 */
import { For, Show } from "solid-js";
import { commandDeckRegistry } from "./deckRegistry";
import { MissionHeader } from "./MissionHeader";
import { setSurface } from "../app/surface";
import { activeContext, currentContext } from "../command-center/context";
import "./command-deck.css";

/** Operational-flow caption for each layout zone (connects the panels). */
function zoneCaption(region: string | undefined): string {
  switch (region) {
    case "activity":
      return `Current Activity · ${currentContext().deckFocus}`;
    case "operations":
      return "Running Operations";
    case "status":
      return "Mission Status";
    case "upcoming":
      return "Upcoming";
    default:
      return "";
  }
}

export function CommandDeck(props: { onExit?: () => void }) {
  const panels = () => commandDeckRegistry.panels();
  return (
    <div class="cd" data-region="command-deck" data-context={activeContext()}>
      <header class="cd__bar">
        <div class="cd__bar-lead">
          <h1 class="cd__title">Mission Control</h1>
          <span class="cd__context" aria-label={`Operating context: ${currentContext().label}`}>
            {currentContext().label}
          </span>
        </div>
        <div class="cd__bar-actions">
          <button type="button" class="cd__exit" onClick={() => setSurface("developer")}>Developer</button>
          <button type="button" class="cd__exit" onClick={() => props.onExit?.()}>Back to Home</button>
        </div>
      </header>

      <MissionHeader />

      <main id="space-root" class="cd__zones" tabindex={-1} aria-label="Operations">
        <Show
          when={panels().length > 0}
          fallback={<div class="cd__empty">The Command Deck is ready. Operational panels will appear here.</div>}
        >
          <For each={panels()}>
            {(panel) => (
              <section
                class="cd__slot"
                data-panel={panel.id}
                data-region-name={panel.region}
                style={panel.region ? { "grid-area": panel.region } : undefined}
              >
                <Show when={zoneCaption(panel.region)}>
                  <span class="cd__zone-cap">{zoneCaption(panel.region)}</span>
                </Show>
                {panel.render()}
              </section>
            )}
          </For>
        </Show>
      </main>
    </div>
  );
}

export default CommandDeck;
