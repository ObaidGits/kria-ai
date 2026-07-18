/**
 * Inspector renderer registry — the content-typed body registry for the single
 * shared Inspector (Req 1.6 / 5.2 / 7.2).
 *
 * The InspectorHost is ONE shared surface driven by `shellStore.inspectorTarget`.
 * Its body is content-typed: each Space contributes a renderer for the target
 * `type`s it owns (memory → 6.2, capability → 8.1, automation node → 7.x,
 * device → 9.x, observatory → 10.x) WITHOUT the shell importing every Space.
 * This module is that decoupling seam:
 *
 *   • `registerInspectorRenderer(type, renderer)` — a Space registers a body
 *     renderer for its target type at module-load (or mount) time. Returns a
 *     disposer so a Space can unregister on unmount / hot-reload.
 *   • `getInspectorRenderer(type)` — the host resolves a renderer for a target.
 *   • `registryVersion()` — a reactive signal the host reads so a renderer
 *     registered AFTER the inspector opened (e.g. a lazily-loaded Space) still
 *     takes effect without reopening the target.
 *
 * The host also accepts a `renderers` prop that OVERRIDES the module registry
 * per-type (used by stories/tests and by AppShell if it ever needs to inject a
 * shell-local renderer). Precedence: props.renderers > module registry >
 * titled fallback.
 *
 * ARCHITECTURE: presentation-only. The registry is a rendering-decoupling
 * mechanism — it carries NO orchestration. Renderers are pure
 * `target → { title, body }` functions supplied by Spaces. Any untrusted
 * content a body shows MUST be sanitized via `lib/markdown` (renderMarkdown /
 * sanitizeHtml); the registry never renders raw HTML itself.
 *
 * Requirements: 1.6, 5.2, 7.2
 */
import { createSignal, type JSX } from "solid-js";
import type { InspectorTarget } from "../stores/shellStore";

/** The resolved body for a target: an accessible heading + a body element. */
export interface InspectorContent {
  /** Accessible heading for the inspector panel. */
  title: string;
  /** Body element. Untrusted content inside MUST be sanitized (lib/markdown). */
  body: JSX.Element;
}

/** A renderer maps an inspector target to its titled body. */
export type InspectorRenderer = (target: InspectorTarget) => InspectorContent;

// Module-level registry. Keyed by `target.type`. One renderer per type — the
// last registration for a type wins (a Space owns its own type).
const registry = new Map<string, InspectorRenderer>();

// Reactive version bumped on every mutation so the host recomputes when a
// renderer registers/unregisters after it already mounted.
const [registryVersion, setRegistryVersion] = createSignal(0);

/**
 * Register a body renderer for an inspector target `type`. Returns a disposer
 * that removes exactly this registration (only if it is still the active one),
 * so Spaces can clean up on unmount without clobbering a re-registration.
 */
export function registerInspectorRenderer(
  type: string,
  renderer: InspectorRenderer,
): () => void {
  registry.set(type, renderer);
  setRegistryVersion((v) => v + 1);
  return () => {
    if (registry.get(type) === renderer) {
      registry.delete(type);
      setRegistryVersion((v) => v + 1);
    }
  };
}

/** Remove any renderer registered for `type`. */
export function unregisterInspectorRenderer(type: string): void {
  if (registry.delete(type)) {
    setRegistryVersion((v) => v + 1);
  }
}

/** Resolve the module-registered renderer for `type`, if any. */
export function getInspectorRenderer(type: string): InspectorRenderer | undefined {
  return registry.get(type);
}

/**
 * Reactive registry version. Read this inside a tracking scope (the host does)
 * so that a renderer registered after mount re-resolves the current target.
 */
export { registryVersion };

/** Clear the entire registry. Intended for tests / hot-reload resets. */
export function resetInspectorRegistry(): void {
  if (registry.size === 0) return;
  registry.clear();
  setRegistryVersion((v) => v + 1);
}
