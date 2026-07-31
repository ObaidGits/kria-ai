/** SurfaceHost renders exactly one shell-bounded top-level surface. */
import { Show } from "solid-js";
import { currentSurface, setSurface } from "./surface";
import CommandCenter from "../command-center/CommandCenter";
import CommandDeck from "../command-deck/CommandDeck";
import DeveloperObservatory from "../developer/DeveloperObservatory";
import { SpaceRouter } from "../shell/SpaceRouter";
import { registerDeckPanels } from "../command-deck/registerDeckPanels";
import { registerDeveloperPanels } from "../developer/registerDevPanels";

// Populate the destination surfaces once. The homepage stays a calm presence;
// the Command Deck and Developer Observatory own all operational/diagnostic panels.
registerDeckPanels();
registerDeveloperPanels();

export function SurfaceHost() {
  return (
    <div class="kria-surface-host" data-surface={currentSurface()}>
      <Show when={currentSurface() === "home"}>
        <CommandCenter />
      </Show>
      <Show when={currentSurface() === "workspace"}>
        <SpaceRouter />
      </Show>
      <Show when={currentSurface() === "command-deck"}>
        <CommandDeck onExit={() => setSurface("home")} />
      </Show>
      <Show when={currentSurface() === "developer"}>
        <DeveloperObservatory onExit={() => setSurface("home")} />
      </Show>
    </div>
  );
}

export default SurfaceHost;
