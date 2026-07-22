/**
 * companionWindow — best-effort native presentation for the Companion ember
 * (design §8.1 "Companion is always-on-top + small"; Req 13.4 / 15.5).
 *
 * This owns ONLY the always-on-top flag + ember sizing on the CURRENT Tauri
 * window; the main-window geometry MEMORY/restore stays owned by
 * `windowModeManager` (which explicitly defers Companion window behaviour to
 * this task). So entering Companion: windowModeManager has already captured the
 * outgoing geometry, then this shrinks + pins the window to an ember; returning:
 * this clears always-on-top and windowModeManager restores the prior geometry.
 *
 * NO new backend/Rust command is used — only the existing `@tauri-apps/api`
 * window/monitor FRONTEND APIs. Every call is guarded and best-effort: where
 * the compositor restricts always-on-top/global positioning (some Wayland
 * sessions) or no Tauri host exists, the calls fail/absent and we degrade to
 * the guaranteed in-app ember (Req 15.5) — never break.
 *
 * Pure geometry lives in `companionEmber.ts`; this file is the thin async shell.
 */
import { PhysicalPosition, PhysicalSize } from "@tauri-apps/api/dpi";
import { currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";
import { isTauriAvailable } from "../../../bridge/types";
import type { EdgeAnchor } from "../../../stores/homeStore";
import {
  emberWindowGeometry,
  resolveCompanionPresentation,
  type CompanionPresentation,
} from "./companionEmber";

/**
 * Probe compositor capability and resolve the ember presentation (Req 15.5).
 * Best-effort: any failure resolves to the safe in-app fallback. `alwaysOnTop`
 * support cannot be queried directly, so we optimistically attempt it when a
 * Tauri host exists and treat a later runtime failure as the degrade signal.
 */
export async function probeCompanionPresentation(): Promise<CompanionPresentation> {
  if (!isTauriAvailable()) return resolveCompanionPresentation({ tauri: false, alwaysOnTopSupported: false });
  try {
    // The presence of a resolvable monitor is a good proxy that global window
    // positioning is available on this session; used only to pick the path.
    const monitor = await currentMonitor();
    return resolveCompanionPresentation({ tauri: true, alwaysOnTopSupported: monitor !== null });
  } catch {
    return resolveCompanionPresentation({ tauri: true, alwaysOnTopSupported: false });
  }
}

/**
 * Enter the native ember presentation: pin always-on-top and shrink to the
 * small edge-anchored ember geometry (design §8.1). Returns the presentation
 * actually achieved so the component can reflect `data-presentation` and fall
 * back to the in-app ember when the compositor refused (Req 15.5).
 */
export async function activateCompanionWindow(anchor: EdgeAnchor): Promise<CompanionPresentation> {
  if (!isTauriAvailable()) return "in-app";
  try {
    const appWindow = getCurrentWindow();
    const monitor = await currentMonitor();
    await appWindow.setAlwaysOnTop(true);
    if (monitor) {
      const geometry = emberWindowGeometry(monitor, anchor);
      await appWindow.setSize(new PhysicalSize(geometry.width, geometry.height));
      await appWindow.setPosition(new PhysicalPosition(geometry.x, geometry.y));
    }
    return "floating-window";
  } catch {
    // Compositor restricted always-on-top / positioning → in-app fallback.
    return "in-app";
  }
}

/**
 * Reposition the pinned ember to a new corner (design §9 "optional reposition/
 * nudge"). No-op/degrades silently off-Tauri or when the compositor refuses.
 */
export async function repositionCompanionWindow(anchor: EdgeAnchor): Promise<void> {
  if (!isTauriAvailable()) return;
  try {
    const appWindow = getCurrentWindow();
    const monitor = await currentMonitor();
    if (!monitor) return;
    const geometry = emberWindowGeometry(monitor, anchor);
    await appWindow.setPosition(new PhysicalPosition(geometry.x, geometry.y));
  } catch {
    // Best-effort; the in-app ember still re-anchors via CSS.
  }
}

/**
 * Leave the native ember presentation: clear always-on-top. The prior windowed
 * geometry is restored by `windowModeManager` on the return mode transition, so
 * this only undoes the pin (never fights the geometry owner).
 */
export async function restoreCompanionWindow(): Promise<void> {
  if (!isTauriAvailable()) return;
  try {
    await getCurrentWindow().setAlwaysOnTop(false);
  } catch {
    // Nothing to undo when the pin never took.
  }
}
