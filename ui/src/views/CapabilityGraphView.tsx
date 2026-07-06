// Task 13.2 — Capability-graph view surface.
//
// Renders the derived `capability_edges` view (dependency / provides-for /
// alternative / supersedes edges) from the CIL CapabilityGraph, with nodes from
// the frozen registry. Gated behind `openclaw_icp_enabled`: honest degraded
// banner + nodes-only when the flag is OFF. Pushes via `openclaw:capability_graph`.

import { Component, createSignal, For, Show, onMount, onCleanup } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  CapabilityGraphPayload,
  CapabilityGraphEdgeView,
  CapabilityGraphNodeView,
} from "./openclawIcpTypes";

const edgeColor: Record<string, string> = {
  depends: "#ef4444",
  provides_for: "#22c55e",
  alternative: "#f59e0b",
  supersedes: "#a855f7",
};

const CapabilityGraphView: Component = () => {
  const [payload, setPayload] = createSignal<CapabilityGraphPayload | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  let unlisten: UnlistenFn | undefined;

  const nameOf = (id: string): string => {
    const n = payload()?.nodes.find((x) => x.skill_id === id);
    return n ? n.name : id;
  };

  const refresh = async () => {
    setError(null);
    try {
      setPayload(await invoke<CapabilityGraphPayload>("openclaw_capability_graph"));
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  onMount(async () => {
    await refresh();
    unlisten = await listen<CapabilityGraphPayload>("openclaw:capability_graph", (ev) => {
      setPayload(ev.payload);
    });
  });
  onCleanup(() => unlisten?.());

  return (
    <div class="openclaw-capability-graph" style={{ padding: "12px" }}>
      <div style={{ display: "flex", "align-items": "center", "justify-content": "space-between" }}>
        <h3>Capability Graph</h3>
        <button onClick={() => void refresh()}>Refresh</button>
      </div>

      <Show when={error()}>
        <p style={{ color: "#ef4444" }}>Error: {error()}</p>
      </Show>

      <Show when={payload()}>
        {(p) => (
          <>
            <p class="settings-hint" style={{ color: p().degraded ? "#f59e0b" : undefined }}>
              {p().status}
            </p>

            <h4>Nodes ({p().nodes.length})</h4>
            <div style={{ display: "flex", "flex-wrap": "wrap", gap: "6px" }}>
              <For each={p().nodes}>
                {(n: CapabilityGraphNodeView) => (
                  <span
                    title={`${n.category} · trust: ${n.trust_tier} · ${n.provenance}`}
                    style={{
                      border: "1px solid rgba(255,255,255,0.15)",
                      "border-radius": "6px",
                      padding: "3px 8px",
                      "font-size": "12px",
                    }}
                  >
                    {n.name}
                  </span>
                )}
              </For>
            </div>

            <h4 style={{ "margin-top": "12px" }}>Edges ({p().edges.length})</h4>
            <Show
              when={p().edges.length > 0}
              fallback={<p class="settings-hint">No capability edges.</p>}
            >
              <For each={p().edges}>
                {(e: CapabilityGraphEdgeView) => (
                  <div style={{ "font-size": "12px", padding: "2px 0" }}>
                    {nameOf(e.from_skill)}{" "}
                    <span style={{ color: edgeColor[e.edge_kind] ?? "#9ca3af" }}>
                      —{e.edge_kind}→
                    </span>{" "}
                    {nameOf(e.to_skill)}
                  </div>
                )}
              </For>
            </Show>
          </>
        )}
      </Show>
    </div>
  );
};

export default CapabilityGraphView;
