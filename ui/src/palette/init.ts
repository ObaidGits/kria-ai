/**
 * Palette bootstrap — registers the shell's default commands + shortcuts.
 *
 * Called once from AppShell.onMount. Returns a disposer that unregisters
 * everything (used on unmount / in tests). Feature Spaces register their own
 * commands/sources/shortcuts on their mounts; this only covers shell-level ones.
 */
import { settingsStore, shellStore } from "../stores";
import { registerCommands, type PaletteCommand } from "./commands";
import { registerShortcuts, DEFAULT_SHORTCUTS } from "./shortcuts";
import { requestWindowMode } from "../windowing/modeTransitionCoordinator";
import { setSurface } from "../app/surface";

function defaultCommands(): PaletteCommand[] {
  return [
    {
      id: "cmd.home",
      title: "Home — Command Center",
      subtitle: "Return to the KRIA homepage",
      icon: "sparkles",
      keywords: "home homepage command center core start hud dashboard",
      run: () => setSurface("home"),
    },
    {
      id: "cmd.theme.toggle",
      title: "Toggle theme (dark / light)",
      icon: "eye",
      keywords: "dark light appearance",
      run: () => void settingsStore.updateSettingByKey(
        "ui.theme",
        shellStore.theme() === "dark" ? "light" : "dark",
      ),
    },
    {
      id: "cmd.density.calm",
      title: "Set density: Calm",
      subtitle: "More breathing room",
      icon: "layers",
      keywords: "spacing comfortable",
      run: () => shellStore.setDensity("calm"),
    },
    {
      id: "cmd.density.focused",
      title: "Set density: Focused",
      icon: "layers",
      keywords: "spacing default",
      run: () => shellStore.setDensity("focused"),
    },
    {
      id: "cmd.density.dense",
      title: "Set density: Dense",
      subtitle: "More information per screen",
      icon: "layers",
      keywords: "spacing compact",
      run: () => shellStore.setDensity("dense"),
    },
    {
      id: "cmd.mode.standard",
      title: "Window mode: Standard",
      icon: "monitor",
      keywords: "window size",
      // Route through the coordinator so the switch is continuous (Core as
      // continuity anchor) and preserves shared state (Req 13.2/13.3/13.5).
      run: () => void requestWindowMode("standard"),
    },
    {
      id: "cmd.mode.mini",
      title: "Window mode: Mini",
      icon: "minimize-2",
      keywords: "window size compact",
      run: () => void requestWindowMode("mini"),
    },
    {
      id: "cmd.mode.immersive",
      title: "Window mode: Immersive",
      icon: "maximize-2",
      keywords: "window size fullscreen",
      run: () => void requestWindowMode("immersive"),
    },
    {
      id: "cmd.mode.companion",
      title: "Window mode: Companion",
      icon: "message-circle",
      keywords: "window ember floating cross-application",
      run: () => void requestWindowMode("companion"),
    },
  ];
}

/** Register shell default commands + shortcuts. Returns a disposer. */
export function initPaletteDefaults(): () => void {
  const undoCommands = registerCommands(defaultCommands());
  const undoShortcuts = registerShortcuts(DEFAULT_SHORTCUTS);
  return () => {
    undoCommands();
    undoShortcuts();
  };
}
