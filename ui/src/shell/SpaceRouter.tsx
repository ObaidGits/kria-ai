/**
 * SpaceRouter — renders the active Space from the typed router. Each Space is a
 * lazily-loaded chunk (design.md §2.3) except Converse, so only shell +
 * Converse are in the initial bundle. Wrapped in <Suspense> so a Space's chunk
 * loads without blocking the shell; the region is the document's <main>
 * landmark (Req 17.2).
 *
 * Requirements: 1.2, 16 (lazy loading), 17.2
 */
import { Suspense, createMemo } from "solid-js";
import { Dynamic } from "solid-js/web";
import { currentRoute } from "./router";
import { SPACE_COMPONENTS } from "./spaces";
import "./AppShell.css";

export function SpaceRouter() {
  const activeComponent = createMemo(() => SPACE_COMPONENTS[currentRoute().space]);

  return (
    <main
      class="kria-space-router"
      id="space-root"
      tabindex={-1}
      aria-label="Primary workspace"
    >
      <Suspense
        fallback={
          <div class="kria-space-router__loading" role="status" aria-live="polite">
            Loading…
          </div>
        }
      >
        <Dynamic component={activeComponent()} />
      </Suspense>
    </main>
  );
}

export default SpaceRouter;
