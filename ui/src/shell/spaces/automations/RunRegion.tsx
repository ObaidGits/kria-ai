/**
 * RunRegion — the Automations "Run" segment (task 7.2, Req 6.3 / 6.5).
 *
 * Composes the full Run experience on top of task 7.1's top-level workflow
 * surfacing:
 *   • {@link AskKriaToPick} — describe an intent → KRIA suggests + prepares a
 *     workflow (Req 6.3).
 *   • a searchable list of {@link WorkflowCard}s surfacing every workflow at the
 *     top level (Req 6.2), each runnable/cancellable with live progress +
 *     evidence, and routing any HITL step to the Approval Center (Req 6.5).
 *
 * Reads `automationStore` only; run/pick/cancel dispatch through existing
 * commands via the store (KRIA runtime authority). Honest loading / empty
 * states (Req 6.5).
 *
 * Requirements: 6.2, 6.3, 6.5
 */
import { createEffect, createMemo, For, Show } from "solid-js";
import { automationStore } from "../../../stores";
import { EmptyState, Search } from "../../../kit";
import { currentRoute, routesEqual } from "../../router";
import { AskKriaToPick } from "./AskKriaToPick";
import { WorkflowCard } from "./WorkflowCard";
import { WorkflowRuns } from "./WorkflowRuns";
import "./run.css";

export function RunRegion() {
  const total = createMemo(() => automationStore.workflows().length);
  let focusedRouteKey: string | null = null;

  // Palette/hash deep links reveal and focus the requested workflow after the
  // authoritative workflow list arrives. Keep the entity in the route so the
  // address remains restorable; user tab/navigation changes clear it.
  createEffect(() => {
    const route = currentRoute();
    const all = automationStore.workflows();
    if (route.space !== "automations" || route.segment !== "run" || !route.entityId) return;
    const workflow = all.find((item) => item.id === route.entityId);
    if (!workflow) return;

    const routeKey = `${route.space}/${route.segment}/${route.entityId}`;
    if (focusedRouteKey === routeKey) return;
    if (automationStore.searchQuery()) automationStore.setSearchQuery("");

    queueMicrotask(() => {
      const latest = currentRoute();
      if (!routesEqual(latest, route)) return;
      const row = Array.from(
        document.querySelectorAll<HTMLElement>("li[data-workflow-id]"),
      ).find((element) => element.dataset.workflowId === workflow.id);
      if (!row) return;
      row.scrollIntoView?.({ block: "center" });
      row.focus({ preventScroll: true });
      focusedRouteKey = routeKey;
    });
  });

  const filtered = createMemo(() => {
    const q = automationStore.searchQuery().trim().toLowerCase();
    const all = automationStore.workflows();
    if (!q) return all;
    return all.filter(
      (w) => w.name.toLowerCase().includes(q) || w.description.toLowerCase().includes(q),
    );
  });

  return (
    <div class="kria-run">
      {/* ask-KRIA-to-pick (Req 6.3) */}
      <AskKriaToPick />

      {/* Active canonical workflow runs — cancel/continuation + HITL pointer
          (task 7.5, Req 6.5 / 11.6). Hidden when there are none. */}
      <WorkflowRuns />

      {/* Top-level workflows (Req 6.2) */}
      <section class="kria-run__section" aria-label="Workflows">
        <div class="kria-automations__run-head">
          <h2 class="kria-run__section-title">Workflows</h2>
          <div class="kria-automations__search">
            <Search
              label="Search workflows"
              placeholder="Search workflows…"
              value={automationStore.searchQuery()}
              onChange={automationStore.setSearchQuery}
            />
          </div>
        </div>

        <Show when={automationStore.loading()}>
          <div class="kria-automations__status" role="status" aria-live="polite">
            Loading workflows…
          </div>
        </Show>

        <Show
          when={!automationStore.loading() && total() > 0}
          fallback={
            <Show when={!automationStore.loading()}>
              <EmptyState
                icon="workflow"
                title="No workflows yet"
                description="Ask KRIA above to find one, or build a workflow in the Build segment. Workflows you build or connect appear here, ready to run."
              />
            </Show>
          }
        >
          <p class="kria-run__count">
            Showing {filtered().length} of {total()}
          </p>
          <Show
            when={filtered().length > 0}
            fallback={<p class="kria-run__muted">No workflows match your search.</p>}
          >
            <ul class="kria-run__list">
              <For each={filtered()}>
                {(wf) => (
                  <li
                    data-workflow-id={wf.id}
                    tabIndex={-1}
                    aria-current={
                      currentRoute().space === "automations"
                      && currentRoute().segment === "run"
                      && currentRoute().entityId === wf.id
                        ? "true"
                        : undefined
                    }
                  >
                    <WorkflowCard workflow={wf} />
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </Show>
      </section>
    </div>
  );
}

export default RunRegion;
