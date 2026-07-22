/**
 * SurfaceHost — renders the active top-level surface.
 *
 * The single switch point for the three top-level surfaces (home / command-deck
 * / developer), orthogonal to the 7-Space Dock router. "home" renders the
 * presence homepage (Command Center); "command-deck" renders Mission Control;
 * "developer" renders the Developer Observatory. The destination surfaces are
 * populated once, below, via their panel registries.
 */
import { Show } from "solid-js";
import { currentSurface, setSurface } from "./surface";
import CommandCenter from "../command-center/CommandCenter";
import CommandDeck from "../command-deck/CommandDeck";
import DeveloperObservatory from "../developer/DeveloperObservatory";
import { registerDeckPanels } from "../command-deck/registerDeckPanels";
import { registerDeveloperPanels } from "../developer/registerDevPanels";

// Populate the destination surfaces once. The homepage stays a calm presence;
// the Command Deck and Developer Observatory own all operational/diagnostic panels.
registerDeckPanels();
registerDeveloperPanels();

export function SurfaceHost() {
  return (
    <>
      <Show when={currentSurface() === "home"}>
        <CommandCenter />
      </Show>
      <Show when={currentSurface() === "command-deck"}>
        <CommandDeck onExit={() => setSurface("home")} />
      </Show>
      <Show when={currentSurface() === "developer"}>
        <DeveloperObservatory onExit={() => setSurface("home")} />
      </Show>
    </>
  );
}

export default SurfaceHost;
