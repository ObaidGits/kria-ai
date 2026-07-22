/**
 * MemoryGraphFallback — mandatory, always-available semantic table for the
 * current 2D Knowledge Graph view.
 *
 * It provides sortable entity/component/centrality/connection columns,
 * visible-label filtering, keyboard row navigation, focus expansion, and
 * view-state actions. Predicted-link writes still route through
 * graphData.materializePrediction; this component does not invent graph facts.
 */
import { For, Show, createMemo, createSignal } from "solid-js";
import { Badge, Button, EmptyState, Search } from "../../../../kit";
import { Icon } from "../../../../components/Icon";
import { graphData } from "./graphData";
import { maxCentrality, nodeSizeForCentrality, type GraphEdge } from "./graphModel";

export interface MemoryGraphFallbackProps {
  /** Reduced-motion / static posture (the 2D table is inherently static). */
  static?: boolean;
  /** Human-readable reason the 2D representation is showing (diagnostics/a11y). */
  reason?: string;
}

type SortKey = "label" | "component" | "centrality" | "connections";
type SortDir = "asc" | "desc";

interface FallbackRow {
  id: string;
  label: string;
  component: number;
  centrality: number;
  connections: number;
  size: number;
}

const COLUMNS: ReadonlyArray<{ key: SortKey; header: string; numeric: boolean }> = [
  { key: "label", header: "Entity", numeric: false },
  { key: "component", header: "Component", numeric: true },
  { key: "centrality", header: "Centrality", numeric: true },
  { key: "connections", header: "Connections", numeric: true },
];

export function MemoryGraphFallback(props: MemoryGraphFallbackProps) {
  let tableRef: HTMLTableElement | undefined;

  const [query, setQuery] = createSignal("");
  const [sortKey, setSortKey] = createSignal<SortKey>("centrality");
  const [sortDir, setSortDir] = createSignal<SortDir>("desc");
  const [activeRowId, setActiveRowId] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal("");

  const nodes = createMemo(() => graphData.visibleNodes());
  const cap = createMemo(() => graphData.capped());
  const maxCent = createMemo(() => Math.max(1, maxCentrality(nodes())));

  /** Degree per visible node, from the pruned visible edge set. */
  const connectionCounts = createMemo(() => {
    const counts = new Map<string, number>();
    for (const edge of graphData.visibleEdges()) {
      counts.set(edge.source, (counts.get(edge.source) ?? 0) + 1);
      counts.set(edge.target, (counts.get(edge.target) ?? 0) + 1);
    }
    return counts;
  });

  /** Filtered + sorted rows for the table body. */
  const rows = createMemo<FallbackRow[]>(() => {
    const q = query().trim().toLowerCase();
    const counts = connectionCounts();
    const max = maxCent();
    let list: FallbackRow[] = nodes().map((n) => ({
      id: n.id,
      label: n.label,
      component: n.community,
      centrality: n.centrality,
      connections: counts.get(n.id) ?? 0,
      size: nodeSizeForCentrality(n.centrality, max),
    }));

    if (q) {
      list = list.filter(
        (r) => r.label.toLowerCase().includes(q) || r.id.toLowerCase().includes(q),
      );
    }

    const key = sortKey();
    const dir = sortDir() === "asc" ? 1 : -1;
    return list.sort((a, b) => {
      let cmp: number;
      if (key === "label") cmp = a.label.localeCompare(b.label);
      else cmp = (a[key] as number) - (b[key] as number);
      if (cmp === 0) cmp = a.id.localeCompare(b.id); // stable, deterministic
      return cmp * dir;
    });
  });

  /** Relationship + predicted-link rows for the focused node (expand view). */
  const focusEdges = createMemo<GraphEdge[]>(() => {
    const focused = graphData.focusedId();
    if (!focused) return [];
    return graphData
      .visibleEdges()
      .filter((e) => e.source === focused || e.target === focused);
  });

  const labelFor = (id: string): string =>
    nodes().find((n) => n.id === id)?.label ?? id.slice(0, 8);

  function ariaSortFor(key: SortKey): "ascending" | "descending" | "none" {
    if (sortKey() !== key) return "none";
    return sortDir() === "asc" ? "ascending" : "descending";
  }

  function onSort(key: SortKey) {
    if (sortKey() === key) {
      setSortDir((d) => (d === "asc" ? "desc" : "asc"));
    } else {
      setSortKey(key);
      // Text defaults to A→Z; numeric defaults to high→low (most relevant first).
      setSortDir(key === "label" ? "asc" : "desc");
    }
  }

  async function selectRow(id: string) {
    setActiveRowId(id);
    await graphData.expand(id);
    const rels = focusEdges().length;
    setStatus(`Focused ${labelFor(id)}. ${rels} relationship${rels === 1 ? "" : "s"} shown.`);
  }

  function hideNode(id: string) {
    const label = labelFor(id);
    graphData.hide(id);
    setStatus(`${label} hidden. Use "Show hidden" to restore.`);
  }

  async function materialize(targetId: string) {
    const label = labelFor(targetId);
    const res = await graphData.materializePrediction(targetId);
    setStatus(
      res.ok
        ? `Predicted link to ${label} materialized as a real relationship.`
        : `Couldn't materialize link to ${label}: ${res.message}`,
    );
  }

  /** Roving-tabindex arrow-key navigation across the row select buttons. */
  function onTableKeyDown(event: KeyboardEvent) {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    const buttons = Array.from(
      tableRef?.querySelectorAll<HTMLButtonElement>("[data-graph-row]") ?? [],
    );
    if (buttons.length === 0) return;
    const current = buttons.findIndex((b) => b === document.activeElement);
    let next = current;
    if (event.key === "ArrowDown") next = current < 0 ? 0 : Math.min(current + 1, buttons.length - 1);
    else if (event.key === "ArrowUp") next = current < 0 ? 0 : Math.max(current - 1, 0);
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = buttons.length - 1;
    event.preventDefault();
    const target = buttons[next];
    if (target) {
      setActiveRowId(target.dataset.graphRow ?? null);
      target.focus();
    }
  }

  /** The roving tabindex: exactly one row button is tab-focusable at a time. */
  function rowTabIndex(id: string, index: number): number {
    const active = activeRowId();
    if (active == null) return index === 0 ? 0 : -1;
    return id === active ? 0 : -1;
  }

  return (
    <div class="kria-graph__fallback" data-static={props.static ? "true" : "false"}>
      <Show when={props.reason}>
        <p class="kria-graph__reason" role="note">
          {props.reason}
        </p>
      </Show>

      {/* Live region — announces load / focus / expand / action outcomes. */}
      <p class="kit-visually-hidden" role="status" aria-live="polite">
        {status()}
      </p>

      <Show when={graphData.loading()}>
        <p class="kria-graph__count" role="status" aria-live="polite">
          Loading knowledge graph…
        </p>
      </Show>

      <Show when={graphData.error()}>
        <EmptyState
          icon="alert-triangle"
          title="Couldn't load the knowledge graph"
          description={graphData.error() ?? "The graph service is unavailable."}
        />
      </Show>

      <Show
        when={!graphData.loading() && !graphData.error() && nodes().length > 0}
        fallback={
          <Show when={!graphData.loading() && !graphData.error()}>
            <EmptyState
              icon="network"
              title="No graph yet"
              description="Entities and relationships will appear here as KRIA links what it knows. Run entity extraction from the Cognition lens, then reload."
            />
          </Show>
        }
      >
        <div class="kria-graph__fallback-toolbar">
          <Search
            label="Filter entities"
            placeholder="Filter entities…"
            value={query()}
            onChange={setQuery}
            class="kria-graph__fallback-search"
          />
          <Show when={graphData.hidden().size > 0}>
            <Button variant="ghost" size="sm" onClick={() => graphData.unhideAll()}>
              <Icon name="eye" size={14} /> Show hidden ({graphData.hidden().size})
            </Button>
          </Show>
        </div>

        <Show when={cap()}>
          {(c) => (
            <p class="kria-graph__count" aria-live="polite">
              {c().label}
              <Show when={query().trim().length > 0}>
                {" "}· {rows().length} match{rows().length === 1 ? "" : "es"}
              </Show>
            </p>
          )}
        </Show>

        <div class="kria-graph__table-wrap">
          <table
            ref={tableRef}
            class="kria-graph__table"
            onKeyDown={onTableKeyDown}
          >
            <caption class="kit-visually-hidden">
              Knowledge graph entities. Use arrow keys to move between rows, Enter to focus an
              entity and reveal its relationships.
            </caption>
            <thead>
              <tr>
                <For each={COLUMNS}>
                  {(col) => (
                    <th scope="col" aria-sort={ariaSortFor(col.key)} data-numeric={col.numeric}>
                      <button
                        type="button"
                        class="kria-graph__sort kit-focusable"
                        onClick={() => onSort(col.key)}
                      >
                        <span>{col.header}</span>
                        <Show when={sortKey() === col.key}>
                          <Icon
                            name={sortDir() === "asc" ? "chevron-up" : "chevron-down"}
                            size={14}
                            title={sortDir() === "asc" ? "sorted ascending" : "sorted descending"}
                          />
                        </Show>
                      </button>
                    </th>
                  )}
                </For>
                <th scope="col">Actions</th>
              </tr>
            </thead>
            <tbody>
              <For each={rows()}>
                {(row, index) => {
                  const isFocused = () => graphData.focusedId() === row.id;
                  return (
                    <tr data-focused={isFocused() ? "true" : "false"}>
                      <th scope="row" class="kria-graph__cell-entity">
                        <button
                          type="button"
                          data-graph-row={row.id}
                          tabindex={rowTabIndex(row.id, index())}
                          class="kria-graph__row-select kit-focusable"
                          aria-pressed={isFocused()}
                          onClick={() => void selectRow(row.id)}
                          onFocus={() => setActiveRowId(row.id)}
                        >
                          <Show when={isFocused()}>
                            <Icon name="check-circle" size={14} title="focused" />
                          </Show>
                          <span>{row.label}</span>
                          <Show when={isPinned()}>
                            <Badge tone="accent">
                              <Icon name="pin" size={12} /> pinned
                            </Badge>
                          </Show>
                        </button>
                      </th>
                      <td data-numeric="true">
                        <Show when={row.component >= 0} fallback={<span class="kria-graph__muted">—</span>}>
                          <Badge tone="info">component {row.component}</Badge>
                        </Show>
                      </td>
                      <td data-numeric="true">{row.centrality}</td>
                      <td data-numeric="true">{row.connections}</td>
                      <td class="kria-graph__cell-actions">
                        <Button
                          variant="ghost"
                          size="sm"
                          aria-pressed={isFocused()}
                          onClick={() => void selectRow(row.id)}
                        >
                          <Icon name="git-branch" size={14} /> {isFocused() ? "Expanded" : "Expand"}
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          aria-pressed={isPinned()}
                          onClick={() => togglePin(row.id)}
                        >
                          <Icon name="pin" size={14} /> {isPinned() ? "Unpin" : "Pin"}
                        </Button>
                        <Button variant="ghost" size="sm" onClick={() => hideNode(row.id)}>
                          <Icon name="eye-off" size={14} /> Hide
                        </Button>
                      </td>
                    </tr>
                  );
                }}
              </For>
            </tbody>
          </table>
        </div>

        {/* Expanded focus view: relationships + predicted links as rows. */}
        <Show when={graphData.focusedId()}>
          {(focused) => (
            <section class="kria-graph__focus" aria-label={`Relationships for ${labelFor(focused())}`}>
              <h3 class="kria-graph__focus-title">
                Relationships for {labelFor(focused())}
              </h3>
              <Show
                when={focusEdges().length > 0}
                fallback={<p class="kria-graph__muted">No relationships or predicted links yet.</p>}
              >
                <table class="kria-graph__table kria-graph__table--rel">
                  <thead>
                    <tr>
                      <th scope="col">Connected entity</th>
                      <th scope="col">Relationship</th>
                      <th scope="col">Status</th>
                      <th scope="col">Actions</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={focusEdges()}>
                      {(edge) => {
                        const other = () => (edge.source === focused() ? edge.target : edge.source);
                        return (
                          <tr>
                            <th scope="row">{labelFor(other())}</th>
                            <td>{edge.relType ?? "related"}</td>
                            <td>
                              <Show
                                when={edge.predicted}
                                fallback={
                                  <span class="kria-graph__status-real">
                                    <Icon name="check" size={14} title="confirmed" /> Confirmed
                                  </span>
                                }
                              >
                                <Badge tone="warning">
                                  <Icon name="sparkles" size={12} /> Predicted
                                </Badge>
                              </Show>
                            </td>
                            <td>
                              <Show when={edge.predicted}>
                                <Button
                                  variant="secondary"
                                  size="sm"
                                  onClick={() => void materialize(other())}
                                >
                                  <Icon name="zap" size={14} /> Materialize
                                </Button>
                              </Show>
                            </td>
                          </tr>
                        );
                      }}
                    </For>
                  </tbody>
                </table>
              </Show>
            </section>
          )}
        </Show>
      </Show>
    </div>
  );
}

export default MemoryGraphFallback;
