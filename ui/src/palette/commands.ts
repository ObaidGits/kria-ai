/**
 * UI command registry for the palette's "Do" mode (Req 2.2).
 *
 * A command is a registered UI action — toggling the theme, changing density,
 * opening a surface, etc. Commands are pure UI/navigation operations. They must
 * NOT wrap a KRIA capability/tool execution; anything that asks KRIA to *act*
 * goes through Ask mode and the normal Intent→Capability→Policy pipeline. This
 * keeps the palette an overlay over the runtime, never a shortcut around policy.
 *
 * Spaces/features register their own commands on mount via `registerCommand`,
 * so "Do" grows without the palette knowing about every feature (Req 21.4).
 */

export interface PaletteCommand {
  /** Stable id (recents key). */
  id: string;
  /** Human label, fuzzy-matched. */
  title: string;
  /** Optional secondary line. */
  subtitle?: string;
  /** Lucide icon. */
  icon?: string;
  /** Search aliases. */
  keywords?: string;
  /** Display-only shortcut hint. */
  shortcutHint?: string;
  /** The action to run. */
  run: () => void;
}

const registry = new Map<string, PaletteCommand>();

/** Register (or replace) a command. Returns an unregister function. */
export function registerCommand(command: PaletteCommand): () => void {
  registry.set(command.id, command);
  return () => {
    if (registry.get(command.id) === command) registry.delete(command.id);
  };
}

/** Register many commands at once. */
export function registerCommands(commands: PaletteCommand[]): () => void {
  const undos = commands.map(registerCommand);
  return () => undos.forEach((u) => u());
}

/** All registered commands, in registration order. */
export function listCommands(): PaletteCommand[] {
  return [...registry.values()];
}

/** Clear the registry (tests). */
export function clearCommands(): void {
  registry.clear();
}
