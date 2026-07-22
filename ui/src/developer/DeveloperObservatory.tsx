/**
 * DeveloperObservatory — the developer-tooling surface SHELL (Phase 1).
 *
 * Dedicated destination for debug panels, logs, internal metrics, AI reasoning
 * inspection, memory inspection, provider diagnostics, and performance analysis.
 * Production-ready shell + registry seam only; future phases relocate tools here
 * via `developerRegistry`. Distinct from the canonical "observatory" Dock Space
 * (which stays governed by the 7-Space rule) — this is developer tooling on the
 * separate surface axis.
 *
 * Reached only via the surface router (`app/surface` → "developer"); dormant in
 * Phase 1, so no visual change.
 */
import { For, Show } from "solid-js";
import { developerRegistry } from "./devRegistry";
import "./developer.css";

export function DeveloperObservatory(props: { onExit?: () => void }) {
  const tools = () => developerRegistry.panels();
  return (
    <div class="dev" data-region="developer-observatory">
      <header class="dev__bar">
        <h1 class="dev__title">Developer Observatory</h1>
        <button type="button" class="dev__exit" onClick={() => props.onExit?.()}>Back to Home</button>
      </header>
      <main class="dev__grid" aria-label="Developer Observatory">
        <Show
          when={tools().length > 0}
          fallback={<div class="dev__empty">Developer Observatory ready. Diagnostics &amp; inspection tools will appear here.</div>}
        >
          <For each={tools()}>
            {(tool) => (
              <section class="dev__slot" data-tool={tool.id} data-region-name={tool.region}>
                {tool.render()}
              </section>
            )}
          </For>
        </Show>
      </main>
    </div>
  );
}

export default DeveloperObservatory;
