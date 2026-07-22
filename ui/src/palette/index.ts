/**
 * Command Palette public surface (design.md §1.12, Req 2).
 *
 * Mount <CommandPalette /> once in the AppShell overlay layer; it is controlled
 * by `shellStore.paletteOpen`. Register feature commands/shortcuts/sources via
 * the registries below so new features are palette-discoverable (Req 21.4).
 */
export { CommandPalette, default as CommandPaletteDefault } from "./CommandPalette";
export { initPaletteDefaults } from "./init";

export { registerCommand, registerCommands, listCommands } from "./commands";
export type { PaletteCommand } from "./commands";

export {
  registerShortcut,
  registerShortcuts,
  listShortcuts,
  formatKeys,
  DEFAULT_SHORTCUTS,
} from "./shortcuts";
export type { Shortcut } from "./shortcuts";

export { registerSource, collectItems } from "./sources";
export { searchItems, groupResults, flattenGroups } from "./search";
export { fuzzyMatch, fuzzyScore } from "./fuzzy";
export { recordUse, recencyBoost, recencyRank, getRecents, clearRecents } from "./recents";
export { isPinned, pinItem, unpinItem, togglePin, clearPins } from "./pins";
export { setAskHandler, setChangeHandler, dispatchAsk, dispatchChange } from "./dispatch";
export { MODES, parseQuery, modeDef, nextMode } from "./modes";
export type { ModeDef, ParsedQuery } from "./modes";

export type {
  PaletteMode,
  PaletteItem,
  PaletteItemType,
  PaletteSource,
  PaletteResult,
  PaletteGroup,
} from "./types";
