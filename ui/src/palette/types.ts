/**
 * Shared Command Palette types (design.md §1.12, Req 2).
 */

/** The four palette modes (Req 2.2). */
export type PaletteMode = "go" | "do" | "ask" | "change";

/**
 * Entity types the palette can search over (Req 2.1). `command` and `shortcut`
 * are runnable (Do mode); the rest are navigation targets (Go mode).
 */
export type PaletteItemType =
  | "space"
  | "command"
  | "shortcut"
  | "setting"
  | "memory"
  | "workflow"
  | "capability"
  | "model"
  | "thread"
  | "device";

/** A single searchable/selectable palette entry. */
export interface PaletteItem {
  /** Stable unique id (also the recents key). */
  id: string;
  type: PaletteItemType;
  /** Primary label shown + fuzzy-matched. */
  title: string;
  /** Optional secondary line (also fuzzy-matched at a lower weight). */
  subtitle?: string;
  /** Lucide icon id for the leading glyph. */
  icon?: string;
  /** Extra hidden text to match against (aliases, tags). */
  keywords?: string;
  /** Display-only keyboard shortcut hint (e.g. "Ctrl K"). */
  shortcutHint?: string;
  /** Invoked when the item is selected. Navigation or a UI action — never a
   *  direct tool/capability execution (that must flow through Ask). */
  run: () => void;
}

/**
 * A registered source contributes items for one or more modes. Sources are
 * pull-based: the palette calls `getItems()` when it opens / on each keystroke,
 * so a source backed by a store always reflects live data. Empty stores simply
 * return `[]` — later Spaces populate their stores and the items appear with no
 * palette change (the registry is the extension point, Req 21.4).
 */
export interface PaletteSource {
  /** Unique source id (e.g. "spaces", "settings"). */
  id: string;
  /** Modes this source contributes to. */
  modes: PaletteMode[];
  /** Return the current items for this source (called live). */
  getItems: () => PaletteItem[];
}

/** A result = an item plus its computed score + matched indices for the query. */
export interface PaletteResult {
  item: PaletteItem;
  score: number;
  indices: number[];
}

/** A group of results of the same type, in display order. */
export interface PaletteGroup {
  type: PaletteItemType;
  label: string;
  results: PaletteResult[];
}
