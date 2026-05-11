import { Component, createSignal, onCleanup, onMount, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import type { SubstrateStatus as SubstrateStatusType } from "../types/openclaw";

// ── Colour map ────────────────────────────────────────────────────────────────
const STATUS_STYLES: Record<string, { dot: string; label: string; bg: string; text: string }> = {
  running:     { dot: "#22c55e", label: "Healthy",   bg: "#f0fdf4", text: "#166534" },
  busy:        { dot: "#f59e0b", label: "Active",    bg: "#fffbeb", text: "#92400e" },
  unavailable: { dot: "#ef4444", label: "Offline",   bg: "#fef2f2", text: "#991b1b" },
  restarting:  { dot: "#6366f1", label: "Restarting",bg: "#eef2ff", text: "#3730a3" },
};

function statusStyle(s: string) {
  return STATUS_STYLES[s] ?? STATUS_STYLES.unavailable;
}

const POOL_TARGET = 3; // expected warm pool size

const SubstrateStatus: Component = () => {
  const [status, setStatus] = createSignal<SubstrateStatusType>({
    status: "unavailable",
    details: "Connecting…",
    active_invocations: 0,
    warm_pool_count: 0,
  });
  const [restarting, setRestarting] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [lastUpdated, setLastUpdated] = createSignal<Date | null>(null);

  const fetchStatus = async () => {
    try {
      const result = await invoke<SubstrateStatusType>("openclaw_substrate_status");
      setStatus(result);
      setLastUpdated(new Date());
      setError(null);
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  // Poll every 3 seconds for live counts
  onMount(() => {
    void fetchStatus();
    const interval = setInterval(() => void fetchStatus(), 3000);
    onCleanup(() => clearInterval(interval));
  });

  const restartSubstrate = async () => {
    setRestarting(true);
    setError(null);
    try {
      await invoke("openclaw_substrate_restart");
      await fetchStatus();
    } catch (e: unknown) {
      setError(String(e));
    } finally {
      setRestarting(false);
    }
  };

  const s = () => restarting() ? "restarting" : status().status;
  const style = () => statusStyle(s());

  return (
    <div style={{
      background: style().bg,
      border: `1px solid ${style().dot}33`,
      "border-radius": "12px",
      padding: "14px 16px",
      "font-family": "system-ui, sans-serif",
    }}>
      {/* Header */}
      <div style={{ display: "flex", "align-items": "center", "justify-content": "space-between", "margin-bottom": "10px" }}>
        <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
          {/* Pulsing status dot */}
          <span style={{
            width: "10px", height: "10px", "border-radius": "50%",
            background: style().dot, display: "inline-block",
            "box-shadow": `0 0 0 3px ${style().dot}33`,
          }} />
          <span style={{ "font-weight": "600", "font-size": "14px" }}>Container Substrate</span>
        </div>
        <span style={{
          "font-size": "11px", "font-weight": "600", padding: "2px 8px",
          "border-radius": "999px", background: style().dot + "22", color: style().text,
        }}>{style().label}</span>
      </div>

      {/* Details */}
      <p style={{ margin: "0 0 10px", "font-size": "12px", color: "#4b5563" }}>
        {status().details}
      </p>

      {/* Metrics row */}
      <div style={{ display: "grid", "grid-template-columns": "1fr 1fr", gap: "8px", "margin-bottom": "10px" }}>
        <MetricBar
          label="Active Invocations"
          value={status().active_invocations}
          max={4}
          color="#6366f1"
        />
        <MetricBar
          label="Warm Pool"
          value={status().warm_pool_count}
          max={POOL_TARGET}
          color="#22c55e"
        />
      </div>

      {/* Error */}
      <Show when={error()}>
        <p style={{ margin: "0 0 8px", "font-size": "11px", color: "#991b1b" }}>
          {error()}
        </p>
      </Show>

      {/* Footer row */}
      <div style={{ display: "flex", "align-items": "center", "justify-content": "space-between" }}>
        <span style={{ "font-size": "10px", color: "#9ca3af" }}>
          {lastUpdated() ? `Updated ${lastUpdated()!.toLocaleTimeString()}` : ""}
        </span>
        <button
          style={{
            padding: "5px 12px", border: "none", "border-radius": "8px",
            "font-size": "12px", "font-weight": "500", cursor: "pointer",
            background: restarting() ? "#e5e7eb" : "#6366f1",
            color: restarting() ? "#9ca3af" : "#ffffff",
          }}
          disabled={restarting()}
          onClick={() => void restartSubstrate()}
        >
          {restarting() ? "Restarting…" : "Restart"}
        </button>
      </div>
    </div>
  );
};

// ── MetricBar sub-component ───────────────────────────────────────────────────
interface MetricBarProps { label: string; value: number; max: number; color: string }
const MetricBar: Component<MetricBarProps> = (props) => {
  const pct = () => Math.min(100, Math.round((props.value / Math.max(props.max, 1)) * 100));
  return (
    <div>
      <div style={{ display: "flex", "justify-content": "space-between", "margin-bottom": "3px" }}>
        <span style={{ "font-size": "10px", color: "#6b7280" }}>{props.label}</span>
        <span style={{ "font-size": "10px", "font-weight": "600", color: "#374151" }}>
          {props.value}/{props.max}
        </span>
      </div>
      <div style={{
        height: "6px", "border-radius": "999px", background: "#e5e7eb", overflow: "hidden",
      }}>
        <div style={{
          height: "100%", width: `${pct()}%`, background: props.color,
          "border-radius": "999px", transition: "width 0.4s ease",
        }} />
      </div>
    </div>
  );
};

export default SubstrateStatus;
