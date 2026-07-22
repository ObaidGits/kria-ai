/**
 * SpaceRouter — renders the active Space from the typed router. Each Space is a
 * lazily-loaded chunk (design.md §2.3) except Converse, so only shell +
 * Converse are in the initial bundle. Wrapped in <Suspense> so a Space's chunk
 * loads without blocking the shell; the region is the document's <main>
 * landmark (Req 17.2).
 *
 * The Suspense fallback names the Space being opened rather than showing a
 * generic "Loading…" (UIE-M-013 / Req 13.1): it maps a `loading` operation
 * snapshot through the shared operation vocabulary (`operationState`) and copy
 * layer (`operationCopy`), using the canonical Space label. The fallback is
 * scoped INSIDE `<main id="space-root">`, so a lazy chunk load never removes the
 * Dock, PresenceBar, StatusLine, Composer, or any safety control from the shell
 * (Req 13.3 — unrelated navigation/conversation/draft/context/safety controls
 * stay available). This is a read-only projection: it owns no runtime lifecycle.
 *
 * Requirements: 1.2, 13.1, 13.3, 16 (lazy loading), 17.2
 */
import { Suspense, createMemo } from "solid-js";
import { Dynamic } from "solid-js/web";
import { currentRoute } from "./router";
import { SPACE_COMPONENTS, SPACE_META } from "./spaces";
import { deriveOperationSnapshot } from "../stores/operationState";
import { describeOperation } from "../stores/operationCopy";
import "./AppShell.css";

export function SpaceRouter() {
  const activeComponent = createMemo(() => SPACE_COMPONENTS[currentRoute().space]);

  // Operation-specific loading copy that NAMES the Space being opened, derived
  // through the shared operation vocabulary (no fabricated progress: the lazy
  // import exposes none, so this is an indeterminate, named "loading" state).
  const loadingCopy = createMemo(() =>
    describeOperation(
      deriveOperationSnapshot({ source: "spaceRouter", loading: true }),
      { operation: SPACE_META[currentRoute().space].label },
    ),
  );

  return (
    <main
      class="kria-space-router"
      id="space-root"
      tabindex={-1}
      aria-label="Primary workspace"
      /* Scroll-ownership marker (task 9.2, UIE-M-005): exposes the active Space
         id so the router's redundant vertical overflow can be scoped off for
         Converse (which owns per-lane vertical scroll) without affecting any
         other Space, which still scrolls via this router. See AppShell.css. */
      data-active-space={currentRoute().space}
    >
      <Suspense
        fallback={
          <div
            class="kria-space-router__loading"
            role="status"
            aria-live="polite"
            data-operation-state="loading"
          >
            {loadingCopy()?.text ?? SPACE_META[currentRoute().space].label}
          </div>
        }
      >
        <Dynamic component={activeComponent()} />
      </Suspense>
    </main>
  );
}

export default SpaceRouter;
