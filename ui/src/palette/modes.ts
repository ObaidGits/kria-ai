/**
 * Palette mode grammar (Req 2.2): Go / Do / Ask / Change.
 *
 * A mode can be switched two ways (both keyboard-reachable, Req 2.3):
 *   1. Clicking / activating a mode chip in the palette header.
 *   2. Typing a leading prefix token in the query:
 *        (none) → Go    ">" → Do    "?" → Ask    "~" → Change
 *
 * `parseQuery` is pure so it can be unit-tested and reused by the component.
 */
import type { PaletteMode } from "./types";

export interface ModeDef {
  mode: PaletteMode;
  label: string;
  icon: string;
  /** Single-character leading prefix that selects this mode (Go has none). */
  prefix: string;
  /** Placeholder shown in the input for this mode. */
  placeholder: string;
  /** Short hint describing what the mode does. */
  hint: string;
}

export const MODES: readonly ModeDef[] = [
  {
    mode: "go",
    label: "Go",
    icon: "grid-3x3",
    prefix: "",
    placeholder: "Go to a Space, memory, workflow, device…",
    hint: "Navigate anywhere",
  },
  {
    mode: "do",
    label: "Do",
    icon: "zap",
    prefix: ">",
    placeholder: "Run a command or shortcut…",
    hint: "Run a command",
  },
  {
    mode: "ask",
    label: "Ask",
    icon: "message-circle",
    prefix: "?",
    placeholder: "Ask KRIA anything…",
    hint: "Send a message to KRIA",
  },
  {
    mode: "change",
    label: "Change",
    icon: "sliders-horizontal",
    prefix: "~",
    placeholder: "Change a setting in plain language…",
    hint: "Change a setting",
  },
] as const;

export const MODE_ORDER: readonly PaletteMode[] = MODES.map((m) => m.mode);

export function modeDef(mode: PaletteMode): ModeDef {
  return MODES.find((m) => m.mode === mode) ?? MODES[0];
}

/** Map a leading prefix char to its mode, or null if it is not a mode prefix. */
export function modeForPrefix(char: string): PaletteMode | null {
  const def = MODES.find((m) => m.prefix !== "" && m.prefix === char);
  return def ? def.mode : null;
}

export interface ParsedQuery {
  /** The active mode after applying any prefix. */
  mode: PaletteMode;
  /** The query with the mode prefix stripped. */
  text: string;
  /** True if the mode came from a typed prefix (vs. the base mode). */
  fromPrefix: boolean;
}

/**
 * Resolve the effective mode + search text from raw input and the currently
 * selected base mode. A leading prefix char overrides the base mode; otherwise
 * the base mode is used and the whole string is the search text.
 */
export function parseQuery(raw: string, baseMode: PaletteMode): ParsedQuery {
  if (raw.length > 0) {
    const prefixMode = modeForPrefix(raw[0]);
    if (prefixMode) {
      return { mode: prefixMode, text: raw.slice(1).trimStart(), fromPrefix: true };
    }
  }
  return { mode: baseMode, text: raw, fromPrefix: false };
}

/** Cycle to the next mode (used by the keyboard mode-cycle shortcut). */
export function nextMode(mode: PaletteMode): PaletteMode {
  const i = MODE_ORDER.indexOf(mode);
  return MODE_ORDER[(i + 1) % MODE_ORDER.length];
}
