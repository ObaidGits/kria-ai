// Task 13.2 — Execution logs surface.
//
// Displays recent OpenClaw execution + bundle-lifecycle events sourced from the
// live `openclaw::event` + `bundle::events` streams (buffered in the backend by
// `spawn_openclaw_log_buffer`). Live-updates by listening to the existing
// `openclaw:execution_event` / `openclaw:bundle_event` streams and reconciles
// via polling `openclaw_execution_logs` (eventual consistency, R10.2).

import { Component, createSignal, For, Show, onMount, onCleanup } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ExecutionLogEntry, ExecutionLogsPayload } from "./openclawIcpTypes";

const ExecutionLogsView: Component = () => {
  const [entries, setEntries] = createSignal<ExecutionLogEntry[]>([]);
  const [note, setNote] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const unlisteners: UnlistenFn[] = [];

  const refresh = async () => {
    setError(null);
    try {
      const p = await invoke<ExecutionLogsPayload>("openclaw_execution_logs", { limit: 200 });
      setEntries(p.entries);
      setNote(p.note);
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  onMount(async () => {
    await refresh();
    // Push-sync on the existing (preserved) event names; refresh to reconcile.
    unlisteners.push(await listen("openclaw:execution_event", () => void refresh()));
    unlisteners.push(await listen("openclaw:bundle_event", () => void refresh()));
  });
  onCleanup(() => unlisteners.forEach((u) => u()));

  return (
    <div class="openclaw-execution-logs" style={{ padding: "12px" }}>
      <div style={{ display: "flex", "align-items": "center", "justify-content": "space-between" }}>
        <h3>Execution Logs</h3>
        <button onClick={() => void refresh()}>Refresh</button>
      </div>

      <Show when={note()}>
        <p class="settings-hint">{note()}</p>
      </Show>
      <Show when={error()}>
        <p style={{ color: "#ef4444" }}>Error: {error()}</p>
      </Show>
      <Show when={entries().length === 0}>
        <p class="settings-hint">No execution events observed yet.</p>
      </Show>

      <div style={{ "font-family": "monospace", "font-size": "12px" }}>
        <For each={entries().slice().reverse()}>
          {(entry: ExecutionLogEntry) => (
            <div
              style={{
                "border-bottom": "1px solid rgba(255,255,255,0.08)",
                padding: "4px 0",
              }}
            >
              <span style={{ color: entry.kind === "bundle" ? "#0ea5e9" : "#f59e0b" }}>
                [{entry.kind}]
              </span>{" "}
              <span style={{ color: "#9ca3af" }}>{entry.received_at}</span>
              <div style={{ color: "#d1d5db", "white-space": "pre-wrap", "word-break": "break-word" }}>
                {JSON.stringify(entry.event)}
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
};

export default ExecutionLogsView;
