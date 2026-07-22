/**
 * ContextPanel — the single contextual surface that emerges from the Core.
 *
 * Enforces the One-Surface Rule: at most one panel exists at a time. Selecting a
 * different Orbit capability replaces its content (keyed `<Show>` re-mounts →
 * emerge animation replays). Dismisses on backdrop click or ESC (global).
 *
 * Phase 6: capability-aware. Content is resolved from the shared `CAPABILITIES`
 * catalog (title, summary, rows, optional actions) — no panel data lives here.
 */
import { For, Show, onMount } from "solid-js";
import { CcIcon } from "./CcIcon";
import { CAPABILITIES } from "./capabilities";
import { activeCapability, closeCapability } from "./homeNav";

export function ContextPanel() {
  const capability = () => {
    const id = activeCapability();
    return id ? CAPABILITIES[id] ?? null : null;
  };

  return (
    <Show when={capability()} keyed>
      {(c) => {
        let panelRef: HTMLElement | undefined;
        onMount(() => panelRef?.focus());
        return (
        <div class="cc-surface" data-open="true">
          <div class="cc-surface__backdrop" aria-hidden="true" onClick={closeCapability} />
          <section ref={panelRef} tabindex="-1" class="cc-surface__panel" role="dialog" aria-modal="true" aria-label={c.label}>
            <header class="cc-surface__head">
              <div class="cc-surface__heading">
                <h2 class="cc-surface__title">{c.label}</h2>
                <span class="cc-surface__summary">{c.panel.summary}</span>
              </div>
              <button type="button" class="cc-surface__close" aria-label="Close" onClick={closeCapability}>
                <CcIcon name="chevron" size={16} />
              </button>
            </header>
            <div class="cc-surface__rows">
              <For each={c.panel.rows}>
                {(row) => (
                  <div class="cc-surface__row">
                    <span class="cc-surface__row-icon"><CcIcon name={row.icon} size={16} /></span>
                    <span class="cc-surface__row-label">{row.label}</span>
                    <span class="cc-surface__row-detail">{row.detail}</span>
                  </div>
                )}
              </For>
            </div>
            <Show when={c.panel.actions}>
              {(actions) => (
                <div class="cc-surface__actions">
                  <For each={actions()}>
                    {(action) => (
                      <button type="button" class="cc-surface__action">
                        <CcIcon name={action.icon} size={14} /> {action.label}
                      </button>
                    )}
                  </For>
                </div>
              )}
            </Show>
          </section>
        </div>
        );
      }}
    </Show>
  );
}

export default ContextPanel;
