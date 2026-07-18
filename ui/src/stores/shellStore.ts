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

// ─── Types ─────────────────────────────────────────────────────────────────────

export type WindowMode = "compact" | "standard" | "immersive";
export type Theme = "dark" | "light";
export type Density = "calm" | "focused" | "dense";

export interface InspectorTarget {
  type: string;
  id: string;
  data?: unknown;
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
  if (saved === "compact" || saved === "standard" || saved === "immersive") return saved;
  return "standard";
}

// ─── Signals ───────────────────────────────────────────────────────────────────

const [activeSpace, setActiveSpaceSignal] = createSignal<Space>("converse");
const [windowMode, setWindowModeSignal] = createSignal<WindowMode>(resolveWindowMode());
const [paletteOpen, setPaletteOpenSignal] = createSignal(false);
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

function setPaletteOpen(open: boolean): void {
  setPaletteOpenSignal(open);
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
 * Open the single shared Inspector on a typed target (Req 1.6 / 5.2 / 7.2).
 * Convenience over `setInspectorTarget`. Because there is only one inspector
 * signal, opening a new target REPLACES the current one (never stacks).
 */
function openInspector(type: string, id: string, data?: unknown): void {
  setInspectorTargetSignal({ type, id, data });
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
  closeInspector,
  setTheme,
  toggleTheme,
  setDensity,
} as const;
