/**
 * ConstellationFallback — the mandatory, always-available 2D/keyboard catalog
 * representation of the Capabilities Constellation (Req 7.5 / 16.3 / 17.5).
 *
 * The Capabilities analogue of MemoryGraphFallback (task 6.5): what the lens
 * yields to whenever 3D is NOT enabled (no WebGL, reduced-motion, failing/absent
 * G2 probe, auto-degrade under load, manual 2D), AND the DEFAULT on WebKitGTK
 * (task 0.6 — 3D is opt-in). It is a REAL accessible table exposing everything
 * the 3D lens shows:
 *   • sortable columns (node / kind / connections),
 *   • search / filter over capabilities/tools/skills/models/integrations,
 *   • arrow-key row navigation with a visible focus state (roving tabindex),
 *   • select → focus + reveal connected nodes as rows,
 *   • per-node actions: focus, pin, hide, and "Inspect" for tool nodes,
 *   • an honest "showing N of M" cap indicator,
 *   • a live region announcing load/focus/select for assistive tech,
 *   • kind conveyed by icon + text (never color alone — Req 17.3).
 *
 * ── ARCHITECTURE INVARIANT ──────────────────────────────────────────────────
 * Pure read-model + shared-Inspector open. This lens is READ/VISUALIZE ONLY: it
 * runs no capability and writes nothing. Selecting a tool node opens its
 * descriptor in the ONE shared Inspector (Req 7.2) — the SAME descriptor the
 * Tools segment uses. KRIA remains orchestration authority.
 */
import { For, Show, createMemo, createSignal } from "solid-js";
import { Badge, Button, EmptyState, Search } from "../../../../kit";
import { Icon } from "../../../../components/Icon";
import { shellStore } from "../../../../stores";
import type { GraphEdge } from "../../memory/graph/graphModel";
import { constellationData } from "./constellationData";
import {
  iconForKind,
  labelForKind,
  type ConstellationNodeKind,
} from "./constellationModel";
import "./constellation.css";

export interface ConstellationFallbackProps {
  /** Reduced-motion / static posture (the 2D table is inherently static). */
  static?: boolean;
  /** Human-readable reason the 2D representation is showing (diagnostics/a11y). */
  reason?: string;
}

type SortKey = "label" | "kind" | "connections";
type SortDir = "asc" | "desc";

interface FallbackRow {
  id: string;
  label: string;
  kind: ConstellationNodeKind;
  connections: number;
  detail: string;
}

const COLUMNS: ReadonlyArray<{ key: SortKey; header: string; numeric: boolean }> = [
  { key: "label", header: "Node", numeric: false },
  { key: "kind", header: "Kind", numeric: false },
  { key: "connections", header: "Connections", numeric: true },
];

export function ConstellationFallback(props: ConstellationFallbackProps) {
  let tableRef: HTMLTableElement | undefined;

  const [query, setQuery] = createSignal("");
  const [sortKey, setSortKey] = createSignal<SortKey>("connections");
  const [sortDir, setSortDir] = createSignal<SortDir>("desc");
  const [activeRowId, setActiveRowId] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal("");

  const nodes = createMemo(() => constellationData.visibleNodes());
  const cap = createMemo(() => constellationData.capped());

  /** Degree per visible node, from the pruned visible edge set. */
  const connectionCounts = createMemo(() => {
    const counts = new Map<string, number>();
    for (const edge of constellationData.visibleEdges()) {
      counts.set(edge.source, (counts.get(edge.source) ?? 0) + 1);
      counts.set(edge.target, (counts.get(edge.target) ?? 0) + 1);
    }
    return counts;
  });

  /** Filtered + sorted rows for the table body. */
  const rows = createMemo<FallbackRow[]>(() => {
    const q = query().trim().toLowerCase();
    const counts = connectionCounts();
    let list: FallbackRow[] = nodes().map((n) => {
      const m = constellationData.metaFor(n.id);
      return {
        id: n.id,
        label: n.label,
        kind: m?.kind ?? "tool",
        connections: counts.get(n.id) ?? 0,
        detail: m?.detail ?? "",
      };
    });

    if (q) {
      list = list.filter(
        (r) =>
          r.label.toLowerCase().includes(q) ||
          r.detail.toLowerCase().includes(q) ||
          labelForKind(r.kind).toLowerCase().includes(q),
      );
    }

    const key = sortKey();
    const dir = sortDir() === "asc" ? 1 : -1;
    return list.sort((a, b) => {
      let cmp: number;
      if (key === "connections") cmp = a.connections - b.connections;
      else if (key === "kind") cmp = labelForKind(a.kind).localeCompare(labelForKind(b.kind));
      else cmp = a.label.localeCompare(b.label);
      if (cmp === 0) cmp = a.id.localeCompare(b.id); // stable, deterministic
      return cmp * dir;
    });
  });

  /** Connected-node rows for the focused node (expand view). */
  const focusEdges = createMemo<GraphEdge[]>(() => {
    const focused = constellationData.focusedId();
    if (!focused) return [];
    return constellationData
      .visibleEdges()
      .filter((e) => e.source === focused || e.target === focused);
  });

  const labelFor = (id: string): string =>
    nodes().find((n) => n.id === id)?.label ?? constellationData.metaFor(id)?.name ?? id;

  function ariaSortFor(key: SortKey): "ascending" | "descending" | "none" {
    if (sortKey() !== key) return "none";
    return sortDir() === "asc" ? "ascending" : "descending";
  }

  function onSort(key: SortKey) {
    if (sortKey() === key) {
      setSortDir((d) => (d === "asc" ? "desc" : "asc"));
    } else {
      setSortKey(key);
      // Text defaults to A→Z; numeric defaults to high→low (most connected first).
      setSortDir(key === "connections" ? "desc" : "asc");
    }
  }

  /**
   * Select a node → focus it + reveal its connections. For a TOOL node this
   * also opens the shared descriptor Inspector (Req 7.2). Never executes.
   */
  function selectRow(id: string) {
    setActiveRowId(id);
    constellationData.focus(id);
    const m = constellationData.metaFor(id);
    if (m?.hasDescriptor && m.providerId && m.capabilityId) {
      // Rows re-sort/filter, so hand the stable Capabilities region as the
      // Focus_Return_Owner rather than a transient row (§20.3/§20.4).
      shellStore.openInspector(
        "capability",
        id,
        {
          providerId: m.providerId,
          capabilityId: m.capabilityId,
          name: m.name,
        },
        { regionSelector: '[data-space="capabilities"]' },
      );
    }
    const rels = focusEdges().length;
    setStatus(
      `Focused ${labelFor(id)}. ${rels} connection${rels === 1 ? "" : "s"} shown.` +
        (m?.hasDescriptor ? " Descriptor opened in the Inspector." : ""),
    );
  }

  function togglePin(id: string) {
    constellationData.togglePin(id);
    const pinned = constellationData.pinned().has(id);
    setStatus(`${labelFor(id)} ${pinned ? "pinned" : "unpinned"}.`);
  }

  function hideNode(id: string) {
    const label = labelFor(id);
    constellationData.hide(id);
    setStatus(`${label} hidden. Use "Show hidden" to restore.`);
  }

  /** Roving-tabindex arrow-key navigation across the row select buttons. */
  function onTableKeyDown(event: KeyboardEvent) {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    const buttons = Array.from(
      tableRef?.querySelectorAll<HTMLButtonElement>("[data-node-row]") ?? [],
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
      setActiveRowId(target.dataset.nodeRow ?? null);
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

      {/* Live region — announces load / focus / select / action outcomes. */}
      <p class="kit-visually-hidden" role="status" aria-live="polite">
        {status()}
      </p>

      <Show when={constellationData.loading()}>
        <p class="kria-graph__count" role="status" aria-live="polite">
          Loading capability constellation…
        </p>
      </Show>

      <Show when={constellationData.error()}>
        <EmptyState
          icon="alert-triangle"
          title="Couldn't load the constellation"
          description={constellationData.error() ?? "The capability catalog is unavailable."}
        />
      </Show>

      <Show
        when={!constellationData.loading() && !constellationData.error() && nodes().length > 0}
        fallback={
          <Show when={!constellationData.loading() && !constellationData.error()}>
            <EmptyState
              icon="network"
              title="No capabilities yet"
              description="Tools, skills, models, and integrations will appear here as KRIA discovers them. Open the Tools or Models segments, then reload."
            />
          </Show>
        }
      >
        <div class="kria-graph__fallback-toolbar">
          <Search
            label="Filter capabilities"
            placeholder="Filter capabilities…"
            value={query()}
            onChange={setQuery}
            class="kria-graph__fallback-search"
          />
          <Show when={constellationData.hidden().size > 0}>
            <Button variant="ghost" size="sm" onClick={() => constellationData.unhideAll()}>
              <Icon name="eye" size={14} /> Show hidden ({constellationData.hidden().size})
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
          <table ref={tableRef} class="kria-graph__table" onKeyDown={onTableKeyDown}>
            <caption class="kit-visually-hidden">
              Capability constellation nodes. Use arrow keys to move between rows, Enter to focus a
              node and reveal its connections; tool nodes open their descriptor in the Inspector.
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
                  const isFocused = () => constellationData.focusedId() === row.id;
                  const isPinned = () => constellationData.pinned().has(row.id);
                  const hasDescriptor = () => constellationData.metaFor(row.id)?.hasDescriptor;
                  return (
                    <tr data-focused={isFocused() ? "true" : "false"}>
                      <th scope="row" class="kria-graph__cell-entity">
                        <button
                          type="button"
                          data-node-row={row.id}
                          tabindex={rowTabIndex(row.id, index())}
                          class="kria-graph__row-select kit-focusable"
                          aria-pressed={isFocused()}
                          onClick={() => selectRow(row.id)}
                          onFocus={() => setActiveRowId(row.id)}
                        >
                          <Show when={isFocused()}>
                            <Icon name="check-circle" size={14} title="focused" />
                          </Show>
                          <Icon name={iconForKind(row.kind)} size={14} aria-hidden />
                          <span>{row.label}</span>
                          <Show when={isPinned()}>
                            <Badge tone="accent">
                              <Icon name="pin" size={12} /> pinned
                            </Badge>
                          </Show>
                        </button>
                      </th>
                      <td>
                        <Badge tone="info">
                          <Icon name={iconForKind(row.kind)} size={12} aria-hidden />{" "}
                          {labelForKind(row.kind)}
                        </Badge>
                      </td>
                      <td data-numeric="true">{row.connections}</td>
                      <td class="kria-graph__cell-actions">
                        <Button
                          variant="ghost"
                          size="sm"
                          aria-pressed={isFocused()}
                          onClick={() => selectRow(row.id)}
                        >
                          <Icon name="git-branch" size={14} /> {isFocused() ? "Focused" : "Focus"}
                        </Button>
                        <Show when={hasDescriptor()}>
                          <Button variant="ghost" size="sm" onClick={() => selectRow(row.id)}>
                            <Icon name="info" size={14} /> Inspect
                          </Button>
                        </Show>
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

        {/* Expanded focus view: connected nodes as rows. */}
        <Show when={constellationData.focusedId()}>
          {(focused) => (
            <section class="kria-graph__focus" aria-label={`Connections for ${labelFor(focused())}`}>
              <h3 class="kria-graph__focus-title">Connections for {labelFor(focused())}</h3>
              <Show
                when={focusEdges().length > 0}
                fallback={<p class="kria-graph__muted">No connections yet.</p>}
              >
                <table class="kria-graph__table kria-graph__table--rel">
                  <thead>
                    <tr>
                      <th scope="col">Connected node</th>
                      <th scope="col">Kind</th>
                      <th scope="col">Relationship</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={focusEdges()}>
                      {(edge) => {
                        const other = () => (edge.source === focused() ? edge.target : edge.source);
                        const kind = () => constellationData.metaFor(other())?.kind ?? "tool";
                        return (
                          <tr>
                            <th scope="row">{labelFor(other())}</th>
                            <td>
                              <Badge tone="info">
                                <Icon name={iconForKind(kind())} size={12} aria-hidden />{" "}
                                {labelForKind(kind())}
                              </Badge>
                            </td>
                            <td>{edge.relType ?? "related"}</td>
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

export default ConstellationFallback;
