/**
 * Surface panel registry — the composition seam future phases use to POPULATE
 * the Command Deck and Developer Observatory shells.
 *
 * A surface (Command Deck / Developer Observatory) owns one registry. Later
 * phases relocate a homepage panel simply by registering it here — the shell
 * lays it out automatically. Phase 1 builds the seam; it starts empty (no
 * migration yet), so the shells render their empty state.
 *
 * Reactive: registrations are a signal, so a shell re-renders when panels are
 * added/removed. State is local per registry (no global app state).
 */
import { createRoot, createSignal, type JSX } from "solid-js";

export interface SurfacePanelSpec {
  /** Stable id (kebab-case). */
  id: string;
  /** Human title (for slot labelling / ordering). */
  title: string;
  /** Optional named layout region within the shell grid. */
  region?: string;
  /** Presentation render fn (the relocated panel). */
  render: () => JSX.Element;
}

export interface PanelRegistry {
  /** Register (or replace by id) a panel; returns an unregister fn. */
  register(spec: SurfacePanelSpec): () => void;
  /** Current registered panels (reactive). */
  panels(): SurfacePanelSpec[];
}

export function createPanelRegistry(): PanelRegistry {
  return createRoot(() => {
    const [items, setItems] = createSignal<SurfacePanelSpec[]>([]);
    return {
      register(spec) {
        setItems((prev) => [...prev.filter((p) => p.id !== spec.id), spec]);
        return () => setItems((prev) => prev.filter((p) => p.id !== spec.id));
      },
      panels: () => items(),
    };
  });
}
