/**
 * Shell Store — owns the top-level application shell state.
 *
 * Signals: active Space, window mode, palette open, inspector target, theme, density.
 * No cross-store reach-in; communicates via the event bus.
 *
 * Requirements: 1.1 (global shell), 13.4 (preserve state), 16.5 (fine-grained reactivity)
 */
import { createSignal, batch } from "solid-js";
import { eventBus } from "./eventBus";
import type { Space } from "../shell/router";
import type { PaletteMode } from "../palette/types";

// ─── Types ─────────────────────────────────────────────────────────────────────

/**
 * Canonical View Mode axis (design.md §8; Requirements 13.1, 13.6).
 * Exactly four modes — the prior Compact/Standard/Immersive naming is
 * reconciled here: **Mini** replaces the former "compact" compact-companion
 * window, and **Companion** (the detached cross-application ember, built in
 * task 8.3) joins the set. `windowModeManager` owns the in-window geometry for
 * Immersive/Standard/Mini; Companion's ember/window behaviour is owned by the
 * companion subsystem (task 8.3) and its per-mode composition by task 8.7.
 */
export type WindowMode = "immersive" | "standard" | "mini" | "companion";
export type Theme = "dark" | "light";
export type Density = "calm" | "focused" | "dense";

export interface InspectorTarget {
  type: string;
  id: string;
  data?: unknown;
}

/**
 * Focus-return ownership for an Inspector open (design §20.3 Focus_Return_Owner
 * = "Invoking control, or nearest stable owning region if removed"; task 9.3).
 * User-click callers pass an explicit `opener` (the invoking control, e.g. the
 * card/row button); programmatic/route/reactive callers (deep-links,
 * revealMemory, NodeBuilder, constellation) pass a stable `region`/
 * `regionSelector` because `document.activeElement` is not the semantic control
 * — so the §20.4 fallback resolves to a stable region rather than a stray
 * element. Held transiently (never persisted, unlike `inspectorTarget`) and
 * consumed by InspectorHost on the INITIAL open.
 */
export interface OpenInspectorOptions {
  /** Explicit invoking control (user-click callers). */
  opener?: HTMLElement | null;
  /** Stable owning region element (programmatic callers). */
  region?: HTMLElement | null;
  /** Or a selector resolved to the owning region at capture time. */
  regionSelector?: string;
}

// ─── Persistence Keys ──────────────────────────────────────────────────────────

const STORAGE_KEYS = {
  theme: "kria_shell_theme",
  density: "kria_shell_density",
  windowMode: "kria_shell_window_mode",
} as const;

function readStorage(key: string): string | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeStorage(key: string, value: string): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // localStorage full — silently degrade
  }
}

// ─── Resolve Initial Values ────────────────────────────────────────────────────

function resolveTheme(): Theme {
  const saved = readStorage(STORAGE_KEYS.theme);
  return saved === "light" ? "light" : "dark";
}

function resolveDensity(): Density {
  const saved = readStorage(STORAGE_KEYS.density);
  if (saved === "calm" || saved === "focused" || saved === "dense") return saved;
  return "focused";
}

function resolveWindowMode(): WindowMode {
  const saved = readStorage(STORAGE_KEYS.windowMode);
  if (saved === "mini" || saved === "standard" || saved === "immersive" || saved === "companion") return saved;
  return "standard";
}

// ─── Signals ───────────────────────────────────────────────────────────────────

const [activeSpace, setActiveSpaceSignal] = createSignal<Space>("converse");
const [windowMode, setWindowModeSignal] = createSignal<WindowMode>(resolveWindowMode());
const [paletteOpen, setPaletteOpenSignal] = createSignal(false);
// Initial mode the palette should present on its next open. Defaults to "go";
// the proven Ctrl+Shift+P chord opens it directly in "do" (design.md §20.2).
const [paletteMode, setPaletteModeSignal] = createSignal<PaletteMode>("go");
const [approvalsOpen, setApprovalsOpenSignal] = createSignal(false);
const [notificationsOpen, setNotificationsOpenSignal] = createSignal(false);
const [inspectorTarget, setInspectorTargetSignal] = createSignal<InspectorTarget | null>(null);
const [theme, setThemeSignal] = createSignal<Theme>(resolveTheme());
const [density, setDensitySignal] = createSignal<Density>(resolveDensity());

// ─── Actions ───────────────────────────────────────────────────────────────────

function setActiveSpace(space: Space): void {
  const previous = activeSpace();
  if (previous === space) return;
  setActiveSpaceSignal(space);
  eventBus.emit("shell:space-changed", { space, previous });
}

function setWindowMode(mode: WindowMode): void {
  const previous = windowMode();
  if (previous === mode) return;
  // Capture hooks run before reactive composition changes. Mode changes remain
  // presentation-only: no Space/domain store is reset or reinitialized.
  eventBus.emit("shell:mode-changing", { mode, previous });
  batch(() => {
    setWindowModeSignal(mode);
    writeStorage(STORAGE_KEYS.windowMode, mode);
  });
  eventBus.emit("shell:mode-changed", { mode, previous });
}

/**
 * Open/close the Command Palette. When opening, `mode` selects the initial
 * palette mode (default "go"); this is how the proven Ctrl+Shift+P chord opens
 * the palette directly in "do" mode via the existing Do-mode path (§20.2).
 * Closing leaves the mode untouched (the palette resets it on next open).
 */
function setPaletteOpen(open: boolean, mode: PaletteMode = "go"): void {
  batch(() => {
    if (open) setPaletteModeSignal(mode);
    setPaletteOpenSignal(open);
  });
  eventBus.emit("shell:palette-toggled", { open });
}

function togglePalette(): void {
  setPaletteOpen(!paletteOpen());
}

/**
 * Open/close the Approval Center overlay (Req 11.1/11.5). The Center is the one
 * true blocking interrupt — it may seize focus when a decision is pending. It
 * is NOT a modal (it does not go through the one-at-a-time modalHost); the
 * high-risk confirm inside it does.
 */
function setApprovalsOpen(open: boolean): void {
  setApprovalsOpenSignal(open);
}

/**
 * Open/close the Notification Center panel (Req 13.3). Unlike the Approval
 * Center it is NON-blocking: it never auto-opens, never seizes focus, and never
 * traps focus — it is a quiet panel the user pulls open from the PresenceBar
 * bell (Req 13.2).
 */
function setNotificationsOpen(open: boolean): void {
  setNotificationsOpenSignal(open);
}

function toggleNotifications(): void {
  setNotificationsOpenSignal(!notificationsOpen());
}

function setInspectorTarget(target: InspectorTarget | null): void {
  setInspectorTargetSignal(target);
}

/**
 * Transient Focus_Return_Owner descriptor for the NEXT Inspector open. Set by
 * `openInspector(…, opts)` and consumed once by InspectorHost when it captures
 * the owner on the initial open. Not a signal / not persisted — it only bridges
 * the caller's opts to the host's capture within the same tick (§20.3).
 */
let pendingInspectorOpener: OpenInspectorOptions | null = null;

/**
 * Open the single shared Inspector on a typed target (Req 1.6 / 5.2 / 7.2).
 * Convenience over `setInspectorTarget`. Because there is only one inspector
 * signal, opening a new target REPLACES the current one (never stacks).
 *
 * `opts` supplies the Focus_Return_Owner (§20.3): an explicit `opener` for
 * user-click callers, or a stable `region`/`regionSelector` for programmatic
 * opens where `document.activeElement` is not the semantic invoking control.
 */
function openInspector(
  type: string,
  id: string,
  data?: unknown,
  opts?: OpenInspectorOptions,
): void {
  pendingInspectorOpener = opts ?? null;
  setInspectorTargetSignal({ type, id, data });
}

/**
 * Consume (and clear) the pending Focus_Return_Owner descriptor. InspectorHost
 * calls this on every target change; it USES the value only on an initial open.
 */
function consumeInspectorOpener(): OpenInspectorOptions | null {
  const desc = pendingInspectorOpener;
  pendingInspectorOpener = null;
  return desc;
}

/** Close the shared Inspector (clears the target). */
function closeInspector(): void {
  setInspectorTargetSignal(null);
}

function setTheme(t: Theme): void {
  batch(() => {
    setThemeSignal(t);
    writeStorage(STORAGE_KEYS.theme, t);
  });
  eventBus.emit("shell:theme-changed", { theme: t });
}

function toggleTheme(): void {
  setTheme(theme() === "dark" ? "light" : "dark");
}

function setDensity(d: Density): void {
  batch(() => {
    setDensitySignal(d);
    writeStorage(STORAGE_KEYS.density, d);
  });
}

// ─── Export ────────────────────────────────────────────────────────────────────

export const shellStore = {
  // Read-only signals
  activeSpace,
  windowMode,
  paletteOpen,
  paletteMode,
  approvalsOpen,
  notificationsOpen,
  inspectorTarget,
  theme,
  density,

  // Actions
  setActiveSpace,
  setWindowMode,
  setPaletteOpen,
  togglePalette,
  setApprovalsOpen,
  setNotificationsOpen,
  toggleNotifications,
  setInspectorTarget,
  openInspector,
  consumeInspectorOpener,
  closeInspector,
  setTheme,
  toggleTheme,
  setDensity,
} as const;
