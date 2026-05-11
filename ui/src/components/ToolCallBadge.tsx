import { Component, Show } from "solid-js";

interface ToolCallBadgeProps {
  source: "native" | "mcp" | "openclaw" | "cloud";
  toolName: string;
  duration?: number;
}

const SOURCE_COLORS: Record<string, string> = {
  native: "#3b82f6",
  mcp: "#22c55e",
  openclaw: "#f59e0b",
  cloud: "#8b5cf6",
};

const ToolCallBadge: Component<ToolCallBadgeProps> = (props) => {
  const color = () => SOURCE_COLORS[props.source] || "#6b7280";

  return (
    <span
      class="tool-call-badge"
      style={{
        background: color(),
        padding: "0.2em 0.5em",
        "border-radius": "0.25em",
        color: "white",
        "font-size": "0.75em",
        "font-weight": "500",
      }}
    >
      {props.source.toUpperCase()} · {props.toolName}
      <Show when={props.duration != null}>
        {" "}({props.duration}ms)
      </Show>
    </span>
  );
};

export default ToolCallBadge;
