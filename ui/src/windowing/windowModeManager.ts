import { PhysicalPosition, PhysicalSize } from "@tauri-apps/api/dpi";
import { availableMonitors, currentMonitor, getCurrentWindow, type Window as TauriWindow } from "@tauri-apps/api/window";
import { isTauriAvailable } from "../bridge/types";
import { eventBus } from "../stores/eventBus";
import { shellStore, type WindowMode } from "../stores/shellStore";
import { isWindowGeometry, normalizeGeometry, type GeometryMonitor, type WindowGeometry } from "./windowGeometry";
import { requestWindowMode, syncViewModeFromShell } from "./modeTransitionCoordinator";

const STORAGE_KEY = "kria_window_geometry_v2";
/**
 * Geometry-bearing in-window modes. Immersive is fullscreen (no windowed
 * geometry) and Companion is the detached ember (its window behaviour is owned
 * by task 8.3), so neither carries a saved main-window geometry here.
 */
type WindowedMode = "standard" | "mini";
type GeometryMemory = Partial<Record<WindowedMode, WindowGeometry>>;

function isGeometryMode(mode: WindowMode): mode is WindowedMode {
  return mode === "standard" || mode === "mini";
}

let disposeManager: (() => void) | null = null;

function readMemory(): GeometryMemory {
  if (typeof window === "undefined") return {};
  try {
    const raw = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "{}");
    if (!raw || typeof raw !== "object") return {};
    const value = raw as Record<string, unknown>;
    return {
      mini: isWindowGeometry(value.mini) ? value.mini : undefined,
      standard: isWindowGeometry(value.standard) ? value.standard : undefined,
    };
  } catch {
    return {};
  }
}

function writeMemory(mode: WindowedMode, geometry: WindowGeometry): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...readMemory(), [mode]: geometry }));
  } catch {
    // Geometry persistence is an enhancement; window remains usable without storage.
  }
}

async function captureGeometry(appWindow: TauriWindow, mode: WindowedMode): Promise<void> {
  if (await appWindow.isFullscreen()) return;
  const [position, size, scaleFactor] = await Promise.all([
    appWindow.outerPosition(), appWindow.outerSize(), appWindow.scaleFactor(),
  ]);
  writeMemory(mode, { x: position.x, y: position.y, width: size.width, height: size.height, scaleFactor });
}

/**
 * Mini fallback geometry used only when no Mini geometry has been persisted:
 * one quarter of the monitor work area (50% width × 50% height), with practical
 * 400×320 CSS-pixel minimums, a 24px right margin, and vertical centering.
 * Exported for deterministic unit coverage of the fallback math.
 */
export function miniDefault(monitor: GeometryMonitor): WindowGeometry {
  const work = monitor.workArea;
  const scale = monitor.scaleFactor;
  const width = Math.min(work.size.width, Math.max(400 * scale, Math.round(work.size.width * 0.5)));
  const height = Math.min(work.size.height, Math.max(320 * scale, Math.round(work.size.height * 0.5)));
  const margin = Math.min(24 * scale, Math.max(0, work.size.width - width));
  return {
    x: work.position.x + work.size.width - width - margin,
    y: work.position.y + Math.max(0, Math.round((work.size.height - height) / 2)),
    width,
    height,
    scaleFactor: scale,
  };
}

/**
 * Deterministic native-presentation plan for one Window Mode transition
 * (design §10 transition table), decoupled from the async Tauri window so the
 * exact geometry/fullscreen semantics are unit-testable without a live desktop
 * target. `undefined` fields mean "leave that native aspect untouched".
 *
 * - Standard/Mini → Immersive: request fullscreen (only when not already).
 * - Immersive → Standard/Mini: exit fullscreen, then restore/derive the
 *   target mode's windowed geometry.
 * - Standard ↔ Mini: no fullscreen change; restore saved geometry, or derive
 *   the Mini fallback when Mini has no saved geometry. Standard with no
 *   saved geometry leaves the current windowed geometry untouched.
 * - → Companion: the detached ember (task 8.3) owns its own window; this plan
 *   only exits fullscreen if needed and requests no main-window geometry.
 * Native presentation is an enhancement: when no monitor work area is available
 * the plan still exits fullscreen but requests no geometry.
 */
export interface NativeTransitionPlan {
  fullscreen?: boolean;
  geometry?: WindowGeometry;
}

export function planNativeTransition(
  target: WindowMode,
  isFullscreen: boolean,
  savedTarget: WindowGeometry | undefined,
  monitors: readonly GeometryMonitor[],
  fallbackMonitor: GeometryMonitor | null,
): NativeTransitionPlan {
  if (target === "immersive") {
    return isFullscreen ? {} : { fullscreen: true };
  }
  const plan: NativeTransitionPlan = isFullscreen ? { fullscreen: false } : {};
  if (monitors.length === 0) return plan;
  const desired =
    savedTarget ?? (target === "mini" && fallbackMonitor ? miniDefault(fallbackMonitor) : null);
  if (!desired) return plan;
  const geometry = normalizeGeometry(desired, monitors);
  if (geometry) plan.geometry = geometry;
  return plan;
}

async function monitorsForRestore(): Promise<GeometryMonitor[]> {
  const monitors = await availableMonitors();
  if (monitors.length > 0) return monitors;
  const monitor = await currentMonitor();
  return monitor ? [monitor] : [];
}
async function applyMode(
  appWindow: TauriWindow,
  mode: WindowMode,
  previous: WindowMode,
  capturePrevious: boolean,
): Promise<void> {
  // Capture the geometry we are leaving so each windowed mode keeps an
  // independent memory (design §10). Immersive (fullscreen) and Companion
  // (detached ember, task 8.3) have no main-window geometry to remember.
  if (capturePrevious && isGeometryMode(previous)) await captureGeometry(appWindow, previous);

  const isFullscreen = await appWindow.isFullscreen();
  const monitors = mode === "immersive" ? [] : await monitorsForRestore();
  const fallbackMonitor = mode === "immersive" ? null : ((await currentMonitor()) ?? monitors[0] ?? null);
  const savedTarget = isGeometryMode(mode) ? readMemory()[mode] : undefined;
  const plan = planNativeTransition(mode, isFullscreen, savedTarget, monitors, fallbackMonitor);

  if (plan.fullscreen !== undefined) await appWindow.setFullscreen(plan.fullscreen);
  if (plan.geometry) {
    await appWindow.setSize(new PhysicalSize(plan.geometry.width, plan.geometry.height));
    await appWindow.setPosition(new PhysicalPosition(plan.geometry.x, plan.geometry.y));
  }
}

/**
 * Owns native presentation only: geometry/fullscreen/window events. Domain work
 * remains in KRIA stores and runtime pipelines; this adapter never dispatches tools.
 */
export function initWindowModeManager(): void {
  if (disposeManager || typeof window === "undefined") return;
  // Align homeStore.viewMode with the shell's restored window mode at boot so
  // continuous transitions never start from a mismatched view mode (task 8.2).
  syncViewModeFromShell();
  let disposed = false;
  let persistTimer: number | undefined;
  const nativeUnlisteners: Array<() => void> = [];
  let operation = Promise.resolve();

  const onKeyDown = (event: KeyboardEvent) => {
    // Respect an overlay/dialog that already consumed Escape: keyboard handling
    // peels one layer at a time. Otherwise this in-webview path is independent
    // of GNOME/KDE and Wayland/X11 compositor shortcuts.
    if (!event.defaultPrevented && event.key === "Escape" && shellStore.windowMode() === "immersive") {
      event.preventDefault();
      // Route the keyboard exit through the coordinator (Req 13.5) so it is a
      // continuous, shared-state-preserving transition like every other trigger.
      requestWindowMode("standard");
    }
  };
  window.addEventListener("keydown", onKeyDown);

  let stopModeEvents = () => {};
  if (isTauriAvailable()) {
    const appWindow = getCurrentWindow();
    const enqueue = (mode: WindowMode, previous: WindowMode, capturePrevious: boolean) => {
      operation = operation
        .then(() => applyMode(appWindow, mode, previous, capturePrevious))
        .catch((error) => console.warn("Window mode update unavailable; using in-app composition only.", error));
    };
    stopModeEvents = eventBus.on("shell:mode-changed", ({ mode, previous }) => {
      enqueue(mode as WindowMode, previous as WindowMode, true);
    }, "none");

    const scheduleGeometrySave = () => {
      if (persistTimer !== undefined) window.clearTimeout(persistTimer);
      persistTimer = window.setTimeout(() => {
        const mode = shellStore.windowMode();
        if (isGeometryMode(mode)) {
          void captureGeometry(appWindow, mode).catch(() => {
            // Host WM may transiently reject reads during monitor/workspace changes.
          });
        }
      }, 200);
    };

    enqueue(shellStore.windowMode(), shellStore.windowMode(), false);
    void Promise.all([appWindow.onMoved(scheduleGeometrySave), appWindow.onResized(scheduleGeometrySave)])
      .then((unlisteners) => {
        if (disposed) unlisteners.forEach((unlisten) => unlisten());
        else nativeUnlisteners.push(...unlisteners);
      })
      .catch(() => {
        // Native listeners are an enhancement; explicit mode controls still work.
      });
  }

  disposeManager = () => {
    disposed = true;
    window.removeEventListener("keydown", onKeyDown);
    stopModeEvents();
    nativeUnlisteners.splice(0).forEach((unlisten) => unlisten());
    if (persistTimer !== undefined) window.clearTimeout(persistTimer);
    disposeManager = null;
  };
}

export function disposeWindowModeManager(): void {
  disposeManager?.();
}
