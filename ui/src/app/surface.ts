/**
 * Surface router — the top-level presentation axis.
 *
 * KRIA has THREE top-level surfaces that sit ORTHOGONAL to the 7-Space Dock
 * router (`shell/router.ts`, governed by expansion-governance-lint):
 *
 *   • "home"         — the presence homepage (Core, Orbit, Hidden Dock, Composer).
 *   • "command-deck" — Mission Control: the on-demand operational workspace.
 *   • "developer"    — the Developer Observatory (debug/logs/metrics).
 *
 * This is a distinct axis, deliberately separate from the Dock's canonical Space
 * set — the Command Deck and Developer Observatory are NOT Dock Spaces (that
 * would violate the 7-Space cap). Default is "home"; the homepage top bar opens
 * the Command Deck, which in turn links to the Developer Observatory.
 *
 * State is intentionally LOCAL to this module (a small reactive signal in a
 * detached root) — surface selection is a shell concern, not global app state.
 */
import { createRoot, createSignal } from "solid-js";

export type Surface = "home" | "command-deck" | "developer";

export const SURFACES: readonly Surface[] = ["home", "command-deck", "developer"] as const;

const DEFAULT_SURFACE: Surface = "home";

const { surface, setSurfaceSignal } = createRoot(() => {
  const [value, setValue] = createSignal<Surface>(DEFAULT_SURFACE);
  return { surface: value, setSurfaceSignal: setValue };
});

/** The current top-level surface. Reactive — read inside a tracking scope. */
export function currentSurface() {
  return surface();
}

/** Switch the top-level surface. */
export function setSurface(next: Surface): void {
  setSurfaceSignal(next);
}

/** Convenience: is a given surface active? */
export function isSurface(target: Surface): boolean {
  return surface() === target;
}
