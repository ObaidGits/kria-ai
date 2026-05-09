/**
 * QuarantineQueue — Safety UI for approving/rejecting compiled skills.
 *
 * This is the most critical safety interface. It displays:
 * - Risk Level (Green/Yellow/Red/Black) with color coding
 * - Success Count (how many times the skill has been tested)
 * - Source (SkillCompiler, DynamicDiscovery, McpServer)
 * - Description and parameter schema
 * - Approve / Reject buttons
 *
 * Tools in "PendingApproval" state require explicit user action before
 * they can be promoted to the active registry.
 */

import {
  Component,
  Show,
  For,
  createSignal,
  createMemo,
  onMount,
  batch,
} from "solid-js";
import { appStore } from "../stores/app";
import type {
  QuarantinedTool,
  QuarantineApprovalRequest,
  RiskLevel,
  QuarantineStatus,
  ToolSourceKind,
} from "../types/intelligence";

// ─── Risk Level Styling ─────────────────────────────────────────────────────

const RISK_COLORS: Record<RiskLevel, string> = {
  Green: "#3bc975",
  Yellow: "#f3b54a",
  Red: "#f86d6d",
  Black: "#4a0000",
};

const RISK_BG: Record<RiskLevel, string> = {
  Green: "rgba(59, 201, 117, 0.12)",
  Yellow: "rgba(243, 181, 74, 0.12)",
  Red: "rgba(248, 109, 109, 0.12)",
  Black: "rgba(74, 0, 0, 0.2)",
};

function RiskBadge(props: { level: RiskLevel }) {
  return (
    <span
      style={{
        display: "inline-flex",
        "align-items": "center",
        gap: "4px",
        padding: "3px 10px",
        "border-radius": "6px",
        "font-size": "11px",
        "font-weight": "700",
        "letter-spacing": "0.5px",
        color: RISK_COLORS[props.level],
        background: RISK_BG[props.level],
        border: `1px solid ${RISK_COLORS[props.level]}40`,
      }}
    >
      <span
        style={{
          width: "6px",
          height: "6px",
          "border-radius": "50%",
          "background-color": RISK_COLORS[props.level],
        }}
      />
      {props.level.toUpperCase()}
    </span>
  );
}

// ─── Status Badge ───────────────────────────────────────────────────────────

const STATUS_LABELS: Record<QuarantineStatus, string> = {
  Testing: "Testing",
  PendingApproval: "Needs Approval",
  Active: "Active",
  Disabled: "Disabled (Circuit Breaker)",
  Rejected: "Rejected",
};

const STATUS_COLORS: Record<QuarantineStatus, string> = {
  Testing: "#7b919f",
  PendingApproval: "#f3b54a",
  Active: "#3bc975",
  Disabled: "#f86d6d",
  Rejected: "#4a5568",
};

function StatusBadge(props: { status: QuarantineStatus }) {
  return (
    <span
      style={{
        "font-size": "11px",
        "font-weight": "600",
        color: STATUS_COLORS[props.status],
      }}
    >
      {STATUS_LABELS[props.status]}
    </span>
  );
}

// ─── Source Label ───────────────────────────────────────────────────────────

const SOURCE_LABELS: Record<ToolSourceKind, string> = {
  SkillCompiler: "🧠 Skill Compiler",
  DynamicDiscovery: "🔍 Dynamic Discovery",
  McpServer: "🔌 MCP Server",
};

function SourceLabel(props: { source: ToolSourceKind }) {
  return (
    <span style={{ "font-size": "11px", color: "var(--text-muted)" }}>
      {SOURCE_LABELS[props.source] ?? props.source}
    </span>
  );
}

// ─── Success Rate Bar ───────────────────────────────────────────────────────

function SuccessRateBar(props: { successes: number; failures: number }) {
  const total = createMemo(() => props.successes + props.failures);
  const rate = createMemo(() => (total() > 0 ? props.successes / total() : 0));
  const color = createMemo(() => {
    const r = rate();
    if (r >= 0.8) return "#3bc975";
    if (r >= 0.5) return "#f3b54a";
    return "#f86d6d";
  });

  return (
    <div style={{ display: "flex", "align-items": "center", gap: "8px", "min-width": "120px" }}>
      <div
        style={{
          flex: "1",
          height: "4px",
          background: "var(--surface-2)",
          "border-radius": "2px",
          overflow: "hidden",
        }}
      >
        <div
          style={{
            width: `${rate() * 100}%`,
            height: "100%",
            background: color(),
            "border-radius": "2px",
            transition: "width 0.3s ease",
          }}
        />
      </div>
      <span style={{ "font-size": "11px", color: "var(--text-muted)", "min-width": "45px", "text-align": "right" }}>
        {props.successes}/{total()}
      </span>
    </div>
  );
}

// ─── Tool Card ──────────────────────────────────────────────────────────────

function ToolCard(props: {
  tool: QuarantinedTool;
  onApprove: (id: string) => void;
  onReject: (id: string) => void;
  approving: boolean;
}) {
  const [expanded, setExpanded] = createSignal(false);

  const needsApproval = createMemo(() => props.tool.status === "PendingApproval");

  return (
    <div
      style={{
        background: "var(--bg-secondary)",
        border: needsApproval()
          ? `1px solid ${RISK_COLORS[props.tool.risk_level]}60`
          : "1px solid var(--border)",
        "border-radius": "var(--radius)",
        overflow: "hidden",
        transition: "border-color 0.2s ease",
      }}
    >
      {/* Header */}
      <div
        style={{
          display: "flex",
          "align-items": "center",
          gap: "12px",
          padding: "12px 16px",
          cursor: "pointer",
        }}
        onClick={() => setExpanded(!expanded())}
      >
        <RiskBadge level={props.tool.risk_level} />

        <div style={{ flex: "1", "min-width": "0" }}>
          <div
            style={{
              "font-size": "14px",
              "font-weight": "600",
              color: "var(--text-primary)",
              overflow: "hidden",
              "text-overflow": "ellipsis",
              "white-space": "nowrap",
            }}
          >
            {props.tool.name}
          </div>
          <div style={{ "font-size": "12px", color: "var(--text-muted)", "margin-top": "2px" }}>
            <SourceLabel source={props.tool.source} />
          </div>
        </div>

        <SuccessRateBar
          successes={props.tool.success_count}
          failures={props.tool.consecutive_failures}
        />

        <StatusBadge status={props.tool.status} />

        <span
          style={{
            color: "var(--text-muted)",
            transform: expanded() ? "rotate(180deg)" : "rotate(0deg)",
            transition: "transform 0.2s ease",
            "font-size": "12px",
          }}
        >
          ▼
        </span>
      </div>

      {/* Expanded Details */}
      <Show when={expanded()}>
        <div
          style={{
            padding: "0 16px 12px 16px",
            "border-top": "1px solid var(--border)",
          }}
        >
          <div style={{ "margin-top": "12px" }}>
            <div style={{ "font-size": "12px", color: "var(--text-muted)", "margin-bottom": "4px" }}>
              DESCRIPTION
            </div>
            <div style={{ "font-size": "13px", color: "var(--text-secondary)" }}>
              {props.tool.description || "No description provided."}
            </div>
          </div>

          <Show when={props.tool.parameters_schema}>
            <div style={{ "margin-top": "12px" }}>
              <div style={{ "font-size": "12px", color: "var(--text-muted)", "margin-bottom": "4px" }}>
                PARAMETER SCHEMA
              </div>
              <pre
                style={{
                  background: "var(--bg-primary)",
                  padding: "8px 12px",
                  "border-radius": "var(--radius-sm)",
                  "font-size": "11px",
                  color: "var(--text-secondary)",
                  overflow: "auto",
                  "max-height": "120px",
                  margin: "0",
                }}
              >
                {JSON.stringify(props.tool.parameters_schema, null, 2)}
              </pre>
            </div>
          </Show>

          <div style={{ "margin-top": "12px", display: "flex", gap: "16px", "font-size": "12px", color: "var(--text-muted)" }}>
            <span>Created: {new Date(props.tool.created_at).toLocaleDateString()}</span>
            <span>Last tested: {new Date(props.tool.last_tested).toLocaleDateString()}</span>
            <span>Total executions: {props.tool.total_executions}</span>
          </div>

          <Show when={props.tool.review_notes}>
            <div style={{ "margin-top": "8px", "font-size": "12px", color: "var(--text-secondary)", "font-style": "italic" }}>
              📝 {props.tool.review_notes}
            </div>
          </Show>
        </div>
      </Show>

      {/* Approval Actions */}
      <Show when={needsApproval()}>
        <div
          style={{
            display: "flex",
            gap: "8px",
            padding: "12px 16px",
            background: "var(--surface-1)",
            "border-top": "1px solid var(--border)",
          }}
        >
          <button
            disabled={props.approving}
            onClick={(e) => {
              e.stopPropagation();
              props.onApprove(props.tool.id);
            }}
            style={{
              flex: "1",
              padding: "8px 16px",
              "border-radius": "var(--radius-sm)",
              border: "1px solid var(--accent)",
              background: "var(--accent-soft)",
              color: "var(--accent)",
              "font-weight": "600",
              cursor: props.approving ? "not-allowed" : "pointer",
              opacity: props.approving ? "0.5" : "1",
              "font-size": "13px",
            }}
          >
            ✓ Approve & Promote
          </button>
          <button
            disabled={props.approving}
            onClick={(e) => {
              e.stopPropagation();
              props.onReject(props.tool.id);
            }}
            style={{
              flex: "1",
              padding: "8px 16px",
              "border-radius": "var(--radius-sm)",
              border: "1px solid var(--danger)",
              background: "var(--danger-soft)",
              color: "var(--danger)",
              "font-weight": "600",
              cursor: props.approving ? "not-allowed" : "pointer",
              opacity: props.approving ? "0.5" : "1",
              "font-size": "13px",
            }}
          >
            ✗ Reject
          </button>
        </div>
      </Show>
    </div>
  );
}

// ─── Filter Tabs ────────────────────────────────────────────────────────────

type FilterTab = "pending" | "all" | "testing" | "disabled";

const FILTER_TABS: { key: FilterTab; label: string }[] = [
  { key: "pending", label: "Needs Approval" },
  { key: "testing", label: "Testing" },
  { key: "all", label: "All Tools" },
  { key: "disabled", label: "Disabled" },
];

// ─── Main Component ─────────────────────────────────────────────────────────

const QuarantineQueue: Component = () => {
  const [activeTab, setActiveTab] = createSignal<FilterTab>("pending");
  const [approvingId, setApprovingId] = createSignal<string | null>(null);

  const allTools = createMemo(() => appStore.quarantinedTools());
  const pendingApproval = createMemo(() => appStore.quarantinePendingApproval());

  const filteredTools = createMemo(() => {
    const tools = allTools();
    switch (activeTab()) {
      case "pending":
        return tools.filter((t) => t.status === "PendingApproval");
      case "testing":
        return tools.filter((t) => t.status === "Testing");
      case "disabled":
        return tools.filter((t) => t.status === "Disabled" || t.status === "Rejected");
      case "all":
      default:
        return tools;
    }
  });

  async function handleApprove(toolId: string) {
    setApprovingId(toolId);
    try {
      await appStore.approveQuarantinedTool(toolId);
    } catch (e) {
      console.error("Approval failed:", e);
    } finally {
      setApprovingId(null);
    }
  }

  async function handleReject(toolId: string) {
    setApprovingId(toolId);
    try {
      await appStore.rejectQuarantinedTool(toolId);
    } catch (e) {
      console.error("Rejection failed:", e);
    } finally {
      setApprovingId(null);
    }
  }

  onMount(() => {
    void appStore.loadQuarantinedTools();
  });

  return (
    <div
      style={{
        display: "flex",
        "flex-direction": "column",
        height: "100%",
        gap: "16px",
        padding: "16px",
        "box-sizing": "border-box",
      }}
    >
      {/* Header */}
      <div style={{ display: "flex", "justify-content": "space-between", "align-items": "center" }}>
        <h2 style={{ margin: "0", "font-size": "18px", color: "var(--text-primary)" }}>
          Quarantine Queue
        </h2>
        <Show when={pendingApproval().length > 0}>
          <span
            style={{
              background: "var(--warning)",
              color: "#000",
              "font-size": "11px",
              "font-weight": "700",
              padding: "3px 10px",
              "border-radius": "12px",
            }}
          >
            {pendingApproval().length} pending
          </span>
        </Show>
      </div>

      {/* Safety notice */}
      <div
        style={{
          background: "var(--surface-1)",
          border: "1px solid var(--border)",
          "border-radius": "var(--radius-sm)",
          padding: "10px 14px",
          "font-size": "12px",
          color: "var(--text-secondary)",
          "line-height": "1.5",
        }}
      >
        ⚠️ <strong style={{ color: "var(--text-primary)" }}>Safety Gate</strong> — Compiled skills
        are tested in quarantine before promotion. <strong>Green</strong> (read-only) tools auto-promote
        after 3 successes. <strong>Yellow/Red</strong> tools require your explicit approval.
      </div>

      {/* Filter Tabs */}
      <div style={{ display: "flex", gap: "4px" }}>
        <For each={FILTER_TABS}>
          {(tab) => {
            const isActive = createMemo(() => activeTab() === tab.key);
            const count = createMemo(() => {
              const tools = allTools();
              switch (tab.key) {
                case "pending":
                  return tools.filter((t) => t.status === "PendingApproval").length;
                case "testing":
                  return tools.filter((t) => t.status === "Testing").length;
                case "disabled":
                  return tools.filter((t) => t.status === "Disabled" || t.status === "Rejected")
                    .length;
                case "all":
                default:
                  return tools.length;
              }
            });

            return (
              <button
                onClick={() => setActiveTab(tab.key)}
                style={{
                  padding: "6px 14px",
                  "border-radius": "6px",
                  border: isActive()
                    ? "1px solid var(--accent-border)"
                    : "1px solid var(--border)",
                  background: isActive() ? "var(--accent-soft)" : "transparent",
                  color: isActive() ? "var(--accent)" : "var(--text-muted)",
                  cursor: "pointer",
                  "font-size": "12px",
                  "font-weight": isActive() ? "600" : "400",
                }}
              >
                {tab.label}
                <Show when={count() > 0}>
                  <span
                    style={{
                      "margin-left": "6px",
                      "font-size": "10px",
                      opacity: "0.7",
                    }}
                  >
                    ({count()})
                  </span>
                </Show>
              </button>
            );
          }}
        </For>
      </div>

      {/* Tool Cards */}
      <div style={{ flex: "1", overflow: "auto", display: "flex", "flex-direction": "column", gap: "8px" }}>
        <Show
          when={filteredTools().length > 0}
          fallback={
            <div
              style={{
                "text-align": "center",
                padding: "40px 20px",
                color: "var(--text-muted)",
                "font-size": "14px",
              }}
            >
              <Show
                when={activeTab() === "pending"}
                fallback={<span>No tools in this category.</span>}
              >
                <span>✅ No tools awaiting approval.</span>
              </Show>
            </div>
          }
        >
          <For each={filteredTools()}>
            {(tool) => (
              <ToolCard
                tool={tool}
                onApprove={handleApprove}
                onReject={handleReject}
                approving={approvingId() === tool.id}
              />
            )}
          </For>
        </Show>
      </div>
    </div>
  );
};

export default QuarantineQueue;
