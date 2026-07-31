/**
 * Top-level presentation axis, orthogonal to the canonical seven-Space router.
 * `workspace` hosts SpaceRouter; switching away from it never resets the last
 * Space, segment, selection, or draft.
 */
import { createRoot, createSignal } from "solid-js";

export type Surface = "home" | "workspace" | "command-deck" | "developer";

export const SURFACES: readonly Surface[] = [
  "home",
  "workspace",
  "command-deck",
  "developer",
] as const;

function initialSurface(): Surface {
  if (typeof window === "undefined") return "home";
  const path = window.location.hash.replace(/^#\/?/, "").replace(/^\/+|\/+$/g, "");
  if (path === "home" || path === "command-deck" || path === "developer") return path;
  return path ? "workspace" : "home";
}

const { surface, setSurfaceSignal } = createRoot(() => {
  const [value, setValue] = createSignal<Surface>(initialSurface());
  return { surface: value, setSurfaceSignal: setValue };
});

/** The current top-level surface. Reactive — read inside a tracking scope. */
export function currentSurface() {
  return surface();
}

/** Switch the top-level surface and keep non-workspace deep links canonical. */
export function setSurface(next: Surface): void {
  setSurfaceSignal(next);
  if (typeof window !== "undefined" && next !== "workspace") {
    const nextHash = `#/${next}`;
    if (window.location.hash !== nextHash) {
      window.history.replaceState(window.history.state, "", nextHash);
    }
  }
}

/** Convenience: is a given surface active? */
export function isSurface(target: Surface): boolean {
  return surface() === target;
}
