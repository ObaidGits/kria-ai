/**
 * Keyboard-shortcut registry (Req 2.4: expose all keyboard shortcuts
 * discoverably via the palette).
 *
 * Every global/app shortcut is registered here once. The palette surfaces them
 * as searchable, runnable items in Do mode, so the shortcut list is always
 * discoverable and self-documenting (there is no separate cheat-sheet to drift
 * out of sync). A shortcut optionally carries a `run` action so selecting it
 * from the palette performs the same thing the key chord would.
 */

export interface Shortcut {
  /** Stable id (recents key). */
  id: string;
  /** Human label describing what the shortcut does. */
  label: string;
  /** Display keys, in press order, e.g. ["Ctrl", "K"]. */
  keys: string[];
  /** Optional Lucide icon. */
  icon?: string;
  /** Optional action performed when chosen from the palette. */
  run?: () => void;
  /** Extra search aliases. */
  keywords?: string;
}

const registry = new Map<string, Shortcut>();

/** Register (or replace) a shortcut. Returns an unregister function. */
export function registerShortcut(shortcut: Shortcut): () => void {
  registry.set(shortcut.id, shortcut);
  return () => {
    if (registry.get(shortcut.id) === shortcut) registry.delete(shortcut.id);
  };
}

/** Register many shortcuts at once. */
export function registerShortcuts(shortcuts: Shortcut[]): () => void {
  const undos = shortcuts.map(registerShortcut);
  return () => undos.forEach((u) => u());
}

/** All registered shortcuts, in registration order. */
export function listShortcuts(): Shortcut[] {
  return [...registry.values()];
}

/** Format a shortcut's keys for display, e.g. "Ctrl K". */
export function formatKeys(keys: string[]): string {
  return keys.join(" ");
}

/** Clear the registry (tests). */
export function clearShortcuts(): void {
  registry.clear();
}

/**
 * The built-in shortcuts that ship with the shell. Kept small and honest — only
 * chords the app actually binds. Spaces/features register their own on mount.
 */
export const DEFAULT_SHORTCUTS: Shortcut[] = [
  { id: "shortcut.palette", label: "Open Command Palette", keys: ["Ctrl", "K"], icon: "command", keywords: "palette search intent bar" },
  { id: "shortcut.palette.do", label: "Command Palette — Do mode", keys: ["Ctrl", "Shift", "P"], icon: "zap", keywords: "run command" },
  { id: "shortcut.close", label: "Close overlay / palette", keys: ["Esc"], icon: "x", keywords: "escape dismiss" },
];
