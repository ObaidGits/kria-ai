/** Global reduced-motion authority for CSS, Core, and 3D rendering. */
export const REDUCED_MOTION_QUERY = "(prefers-reduced-motion: reduce)";

export interface ReducedMotionController {
  readonly reducedMotion: () => boolean;
  /** Force motion off. False returns authority to the OS preference. */
  setKillSwitch(enabled: boolean): void;
  dispose(): void;
}

export interface ReducedMotionControllerOptions {
  document?: Document;
  window?: Window;
  initialReducedMotion: boolean;
  onChange: (reducedMotion: boolean) => void;
}

function stampRoot(doc: Document | undefined, reducedMotion: boolean): void {
  const root = doc?.documentElement;
  if (!root) return;
  if (reducedMotion) root.setAttribute("data-reduced-motion", "on");
  else root.removeAttribute("data-reduced-motion");
}

/**
 * One bounded, event-driven preference controller. No polling or retry loop.
 * The kill-switch can only reduce motion; it cannot override an OS reduction.
 */
export function createReducedMotionController(
  options: ReducedMotionControllerOptions,
): ReducedMotionController {
  let systemReduced = options.initialReducedMotion;
  let forcedReduced = false;
  let current = systemReduced;
  let disposed = false;

  const commit = () => {
    if (disposed) return;
    const next = forcedReduced || systemReduced;
    stampRoot(options.document, next);
    if (next === current) return;
    current = next;
    options.onChange(next);
  };

  let media: MediaQueryList | undefined;
  const onMediaChange = (event: MediaQueryListEvent) => {
    systemReduced = event.matches;
    commit();
  };
  try {
    media = options.window?.matchMedia?.(REDUCED_MOTION_QUERY);
    media?.addEventListener?.("change", onMediaChange);
  } catch {
    media = undefined;
  }
  stampRoot(options.document, current);

  return {
    reducedMotion: () => current,
    setKillSwitch(enabled) {
      forcedReduced = enabled;
      commit();
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      media?.removeEventListener?.("change", onMediaChange);
      media = undefined;
    },
  };
}