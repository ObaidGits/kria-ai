/**
 * Summon — centralizes the "bring KRIA to front + open the Command Palette"
 * action and wires the guaranteed in-app keyboard fallback (Req 2.5, 18.2).
 *
 * Paths (enhancement-with-fallback — architecture invariant):
 *
 *  • Global system hotkey (ENHANCEMENT) — registered in the Rust backend via the
 *    global-shortcut plugin (`crates/kria-desktop/src/summon.rs`). On trigger the
 *    backend focuses the window and emits the [`SUMMON_EVENT`] Tauri event, which
 *    this module listens for. May be unavailable (Wayland restrictions, chord
 *    conflict); backend registration is try/degrade and never crashes.
 *
 *  • In-app webview hotkey (GUARANTEED) — a document `keydown` listener bound here
 *    (Cmd/Ctrl+K). It runs entirely in the webview and never depends on any OS
 *    feature, so it always summons — this is the fallback that cannot break.
 *
 *  • Tray item + KRIA Mini — invoke the same [`summon`] action / `summon`
 *    backend command (the tray also emits [`SUMMON_EVENT`]). KRIA Mini (task
 *    12.4) reuses [`summon`] rather than building its own path.
 *
 * [`summon`] focuses the window via the optional `summon` bridge command (which
 * degrades silently when unavailable) and opens the palette through shellStore —
 * opening the palette is the part that is always guaranteed.
 */
import { shellStore } from "../stores";
import { bridgeInvokeOptional } from "../bridge";
import { isTauriAvailable } from "../bridge/types";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Backend command that focuses/raises the main window. Optional/enhancement. */
export const SUMMON_COMMAND = "summon";

/** Tauri event the backend (global hotkey / tray) emits to request a summon. */
export const SUMMON_EVENT = "app:summon";

/**
 * Bring KRIA to the foreground and open the Command Palette.
 *
 * Window focus is a best-effort enhancement (optional bridge command — swallows
 * unavailability, never throws); opening the palette is the guaranteed in-app
 * behaviour and happens regardless of OS window/focus support (Req 2.5).
 */
export function summon(): void {
  // Best-effort window focus/raise. `bridgeInvokeOptional` never throws — a
  // missing backend/command (plain browser, restricted DE) degrades to null.
  void bridgeInvokeOptional(SUMMON_COMMAND);
  // Guaranteed: open the palette regardless of OS-level support.
  shellStore.setPaletteOpen(true);
}

/**
 * True when the event is the in-app summon chord: Cmd/Ctrl+K (no Alt).
 *
 * Pure matcher — exported so it can be unit-tested without the DOM listener.
 */
export function isSummonHotkey(e: KeyboardEvent): boolean {
  if (e.altKey) return false;
  const mod = e.ctrlKey || e.metaKey;
  return mod && (e.key === "k" || e.key === "K");
}

/**
 * True when the event target is a text-editing surface (input/textarea/select
 * or a contenteditable element) — i.e. the user is typing.
 *
 * The summon hotkey is suppressed on these targets (guard) so a chord never
 * hijacks text entry in the Composer, Settings search, etc. Summon stays
 * reachable via the tray, the palette button, and the global hotkey.
 */
export function isTypingTarget(target: EventTarget | null): boolean {
  const el = target as (HTMLElement & { isContentEditable?: boolean }) | null;
  if (!el || typeof el.tagName !== "string") return false;
  const tag = el.tagName.toLowerCase();
  if (tag === "input" || tag === "textarea" || tag === "select") return true;
  return el.isContentEditable === true;
}

// ─── Wiring (in-app hotkey + backend summon event) ───────────────────────────

let keydownHandler: ((e: KeyboardEvent) => void) | null = null;
let unlistenSummon: UnlistenFn | null = null;

/**
 * Bind the guaranteed in-app hotkey and subscribe to the backend summon event.
 * Idempotent; returns a disposer. Call from AppShell.onMount.
 */
export function initSummon(): () => void {
  // ── Guaranteed path: in-app webview hotkey (Cmd/Ctrl+K) ──
  if (typeof document !== "undefined" && keydownHandler === null) {
    keydownHandler = (e: KeyboardEvent) => {
      if (!isSummonHotkey(e)) return;
      // Guard: don't summon while the user is typing in a field.
      if (isTypingTarget(e.target)) return;
      e.preventDefault();
      summon();
    };
    document.addEventListener("keydown", keydownHandler);
  }

  // ── Enhancement path: backend global hotkey / tray → SUMMON_EVENT ──
  if (isTauriAvailable() && unlistenSummon === null) {
    listen(SUMMON_EVENT, () => summon())
      .then((un) => {
        unlistenSummon = un;
      })
      .catch((err) => {
        // Enhancement absent — the in-app hotkey remains the guaranteed path.
        if (import.meta.env.DEV) {
          console.debug("[summon] backend summon event unavailable:", err);
        }
      });
  }

  return disposeSummon;
}

/** Detach the in-app hotkey and the backend summon-event subscription. */
export function disposeSummon(): void {
  if (keydownHandler !== null && typeof document !== "undefined") {
    document.removeEventListener("keydown", keydownHandler);
    keydownHandler = null;
  }
  if (unlistenSummon !== null) {
    try {
      unlistenSummon();
    } catch {
      // Already detached — ignore.
    }
    unlistenSummon = null;
  }
}
