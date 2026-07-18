import { PhysicalPosition, PhysicalSize } from "@tauri-apps/api/dpi";
import { availableMonitors, currentMonitor, getCurrentWindow, type Window as TauriWindow } from "@tauri-apps/api/window";
import { isTauriAvailable } from "../bridge/types";
import { eventBus } from "../stores/eventBus";
import { shellStore, type WindowMode } from "../stores/shellStore";
import { isWindowGeometry, normalizeGeometry, type GeometryMonitor, type WindowGeometry } from "./windowGeometry";

const STORAGE_KEY = "kria_window_geometry_v1";
type WindowedMode = Exclude<WindowMode, "immersive">;
type GeometryMemory = Partial<Record<WindowedMode, WindowGeometry>>;

let disposeManager: (() => void) | null = null;

function readMemory(): GeometryMemory {
  if (typeof window === "undefined") return {};
  try {
    const raw = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "{}");
    if (!raw || typeof raw !== "object") return {};
    const value = raw as Record<string, unknown>;
    return {
      compact: isWindowGeometry(value.compact) ? value.compact : undefined,
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

function compactDefault(monitor: GeometryMonitor): WindowGeometry {
  const work = monitor.workArea;
  const scale = monitor.scaleFactor;
  const width = Math.min(work.size.width, Math.max(400 * scale, Math.round(work.size.width * 0.3)));
  const height = Math.min(work.size.height, Math.max(500 * scale, Math.round(work.size.height * 0.7)));
  const margin = Math.min(24 * scale, Math.max(0, work.size.width - width));
  return {
    x: work.position.x + work.size.width - width - margin,
    y: work.position.y + Math.max(0, Math.round((work.size.height - height) / 2)),
    width,
    height,
    scaleFactor: scale,
  };
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
  if (capturePrevious && previous !== "immersive") await captureGeometry(appWindow, previous);

  if (mode === "immersive") {
    if (!(await appWindow.isFullscreen())) await appWindow.setFullscreen(true);
    return;
  }

  if (await appWindow.isFullscreen()) await appWindow.setFullscreen(false);
  const monitors = await monitorsForRestore();
  if (monitors.length === 0) return;
  const saved = readMemory()[mode];
  const fallbackMonitor = (await currentMonitor()) ?? monitors[0];
  const desired = saved ?? (mode === "compact" ? compactDefault(fallbackMonitor) : null);
  if (!desired) return;
  const geometry = normalizeGeometry(desired, monitors);
  if (!geometry) return;
  await appWindow.setSize(new PhysicalSize(geometry.width, geometry.height));
  await appWindow.setPosition(new PhysicalPosition(geometry.x, geometry.y));
}

/**
 * Owns native presentation only: geometry/fullscreen/window events. Domain work
 * remains in KRIA stores and runtime pipelines; this adapter never dispatches tools.
 */
export function initWindowModeManager(): void {
  if (disposeManager || typeof window === "undefined") return;
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
      shellStore.setWindowMode("standard");
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
        if (mode !== "immersive") {
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
