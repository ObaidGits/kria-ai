/**
 * PlanVisualization — Renders the 7B Planner's 3 Structured Paths side-by-side.
 *
 * When the StructuredBranchingPlanner generates its 3 paths (Diagnose-First,
 * Minimal-Risk, Aggressive), this component renders them in a comparative layout.
 * The winner chosen by the SelfModel's Beta posterior score is highlighted.
 *
 * Shows:
 * - 3 path cards (Diagnose, Minimal-Risk, Aggressive)
 * - SelfModel Beta posterior score for each path
 * - Winner highlight with selection reason
 * - Step-by-step breakdown for each path
 * - Live execution progress (step results streaming in)
 * - Goal verification outcome
 */

import {
  Component,
  Show,
  For,
  createSignal,
  createMemo,
  Match,
  Switch,
} from "solid-js";
import { appStore } from "../stores/app";
import type {
  StructuredPath,
  PlannedStep,
  PlanGenerated,
  PlanStepResult,
  GoalVerification,
  PathRisk,
} from "../types/intelligence";

// ─── Path Risk Styling ──────────────────────────────────────────────────────

const RISK_CONFIG: Record<
  PathRisk,
  { color: string; bg: string; label: string; icon: string }
> = {
  DiagnoseFirst: {
    color: "#3bc975",
    bg: "rgba(59, 201, 117, 0.08)",
    label: "Diagnose First",
    icon: "🔍",
  },
  MinimalRisk: {
    color: "#f3b54a",
    bg: "rgba(243, 181, 74, 0.08)",
    label: "Minimal Risk",
    icon: "🔧",
  },
  Aggressive: {
    color: "#f86d6d",
    bg: "rgba(248, 109, 109, 0.08)",
    label: "Aggressive Fix",
    icon: "⚡",
  },
};

// ─── Score Bar ──────────────────────────────────────────────────────────────

function ScoreBar(props: { score: number; isWinner: boolean }) {
  const pct = createMemo(() => Math.round(props.score * 100));
  const color = createMemo(() => {
    if (props.isWinner) return "var(--accent)";
    if (props.score >= 0.8) return "#3bc975";
    if (props.score >= 0.5) return "#f3b54a";
    return "#f86d6d";
  });

  return (
    <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
      <div
        style={{
          flex: "1",
          height: "6px",
          background: "var(--surface-2)",
          "border-radius": "3px",
          overflow: "hidden",
        }}
      >
        <div
          style={{
            width: `${pct()}%`,
            height: "100%",
            background: color(),
            "border-radius": "3px",
            transition: "width 0.5s ease",
          }}
        />
      </div>
      <span
        style={{
          "font-size": "13px",
          "font-weight": "700",
          color: color(),
          "min-width": "40px",
          "text-align": "right",
        }}
      >
        {pct()}%
      </span>
    </div>
  );
}

// ─── Step Row ───────────────────────────────────────────────────────────────

function StepRow(props: { step: PlannedStep; result?: PlanStepResult }) {
  const errorHandlingLabel = createMemo(() => {
    switch (props.step.error_handling) {
      case "continue":
        return "Skip on error";
      case "abort":
        return "Abort on error";
      case "retry":
        return "Retry on error";
      default:
        return props.step.error_handling;
    }
  });

  return (
    <div
      style={{
        display: "flex",
        "align-items": "flex-start",
        gap: "10px",
        padding: "8px 0",
        "border-bottom": "1px solid var(--border)",
      }}
    >
      {/* Step number */}
      <div
        style={{
          "min-width": "24px",
          height: "24px",
          "border-radius": "50%",
          display: "flex",
          "align-items": "center",
          "justify-content": "center",
          "font-size": "11px",
          "font-weight": "700",
          background: props.result
            ? props.result.success
              ? "var(--success)"
              : "var(--danger)"
            : "var(--surface-2)",
          color: props.result ? "#fff" : "var(--text-muted)",
        }}
      >
        <Show when={props.result} fallback={props.step.step_number}>
          {props.result!.success ? "✓" : "✗"}
        </Show>
      </div>

      <div style={{ flex: "1", "min-width": "0" }}>
        <div style={{ "font-size": "13px", "font-weight": "500", color: "var(--text-primary)" }}>
          {props.step.description}
        </div>
        <div
          style={{
            "font-size": "11px",
            color: "var(--text-muted)",
            "margin-top": "2px",
            "font-family": "var(--font-mono)",
          }}
        >
          {props.step.command.binary} {props.step.command.args.join(" ")}
          <span style={{ opacity: "0.6" }}> → {props.step.command.target}</span>
        </div>
        <div style={{ "font-size": "10px", color: "var(--text-muted)", "margin-top": "2px" }}>
          {errorHandlingLabel()}
        </div>
      </div>

      <Show when={props.result}>
        <div style={{ "text-align": "right", "min-width": "60px" }}>
          <div
            style={{
              "font-size": "12px",
              "font-weight": "600",
              color: props.result!.success ? "var(--success)" : "var(--danger)",
            }}
          >
            {props.result!.exit_code !== null ? `exit ${props.result!.exit_code}` : props.result!.success ? "OK" : "FAIL"}
          </div>
          <div style={{ "font-size": "10px", color: "var(--text-muted)" }}>
            {props.result!.duration_ms < 1000
              ? `${props.result!.duration_ms}ms`
              : `${(props.result!.duration_ms / 1000).toFixed(1)}s`}
          </div>
        </div>
      </Show>
    </div>
  );
}

// ─── Path Card ──────────────────────────────────────────────────────────────

function PathCard(props: {
  path: StructuredPath;
  index: number;
  stepResults: PlanStepResult[];
}) {
  const [expanded, setExpanded] = createSignal(props.path.is_winner);
  const config = createMemo(() => RISK_CONFIG[props.path.risk_level]);

  const matchingResults = createMemo(() => {
    const results = props.stepResults;
    return (step: PlannedStep) =>
      results.find(
        (r) => r.step_number === step.step_number && r.tool_name === step.tool_name
      );
  });

  return (
    <div
      style={{
        flex: "1",
        "min-width": "280px",
        background: props.path.is_winner ? config().bg : "var(--bg-secondary)",
        border: props.path.is_winner
          ? `2px solid ${config().color}`
          : "1px solid var(--border)",
        "border-radius": "var(--radius)",
        overflow: "hidden",
        transition: "border-color 0.3s ease, background 0.3s ease",
        position: "relative",
      }}
    >
      {/* Winner badge */}
      <Show when={props.path.is_winner}>
        <div
          style={{
            position: "absolute",
            top: "0",
            right: "0",
            background: config().color,
            color: "#fff",
            "font-size": "10px",
            "font-weight": "700",
            padding: "4px 12px",
            "border-radius": "0 0 0 8px",
            "letter-spacing": "0.5px",
          }}
        >
          ★ WINNER
        </div>
      </Show>

      {/* Header */}
      <div
        style={{
          padding: "16px",
          cursor: "pointer",
        }}
        onClick={() => setExpanded(!expanded())}
      >
        <div style={{ display: "flex", "align-items": "center", gap: "8px", "margin-bottom": "8px" }}>
          <span style={{ "font-size": "18px" }}>{config().icon}</span>
          <span
            style={{
              "font-size": "14px",
              "font-weight": "700",
              color: config().color,
            }}
          >
            Path {String.fromCharCode(65 + props.index)}: {config().label}
          </span>
        </div>

        <div style={{ "font-size": "12px", color: "var(--text-secondary)", "margin-bottom": "12px" }}>
          {props.path.steps.length} steps · {props.path.risk_level === "DiagnoseFirst" ? "Read-only" : props.path.risk_level === "MinimalRisk" ? "Reversible" : "Potentially irreversible"}
        </div>

        <div style={{ "margin-bottom": "8px" }}>
          <div style={{ "font-size": "11px", color: "var(--text-muted)", "margin-bottom": "4px" }}>
            SELF-MODEL SCORE (Beta Posterior)
          </div>
          <ScoreBar score={props.path.self_model_score} isWinner={props.path.is_winner} />
        </div>

        <div>
          <div style={{ "font-size": "11px", color: "var(--text-muted)", "margin-bottom": "4px" }}>
            CONFIDENCE
          </div>
          <ScoreBar score={props.path.confidence} isWinner={false} />
        </div>

        <div
          style={{
            "font-size": "11px",
            color: "var(--text-muted)",
            "margin-top": "8px",
            transform: expanded() ? "rotate(180deg)" : "rotate(0deg)",
            transition: "transform 0.2s ease",
            "text-align": "center",
          }}
        >
          ▼
        </div>
      </div>

      {/* Steps */}
      <Show when={expanded()}>
        <div style={{ padding: "0 16px 16px 16px", "border-top": "1px solid var(--border)" }}>
          <div style={{ "font-size": "11px", color: "var(--text-muted)", "margin-top": "12px", "margin-bottom": "4px" }}>
            EXECUTION STEPS
          </div>
          <For each={props.path.steps}>
            {(step) => <StepRow step={step} result={matchingResults()(step)} />}
          </For>
        </div>
      </Show>
    </div>
  );
}

// ─── Goal Verification Banner ───────────────────────────────────────────────

function GoalVerificationBanner(props: { verification: GoalVerification }) {
  const config = createMemo(() => {
    switch (props.verification.outcome) {
      case "Achieved":
        return { color: "var(--success)", bg: "rgba(59, 201, 117, 0.1)", icon: "✅", label: "Goal Achieved" };
      case "Failed":
        return { color: "var(--danger)", bg: "rgba(248, 109, 109, 0.1)", icon: "❌", label: "Goal Failed" };
      case "Continue":
        return { color: "var(--text-muted)", bg: "var(--surface-1)", icon: "⏳", label: "Continuing..." };
    }
  });

  return (
    <div
      style={{
        display: "flex",
        "align-items": "center",
        gap: "12px",
        padding: "12px 16px",
        background: config().bg,
        border: `1px solid ${config().color}40`,
        "border-radius": "var(--radius-sm)",
      }}
    >
      <span style={{ "font-size": "20px" }}>{config().icon}</span>
      <div style={{ flex: "1" }}>
        <div style={{ "font-size": "14px", "font-weight": "600", color: config().color }}>
          {config().label}
        </div>
        <Show when={props.verification.reason}>
          <div style={{ "font-size": "12px", color: "var(--text-secondary)", "margin-top": "2px" }}>
            {props.verification.reason}
          </div>
        </Show>
      </div>
      <div style={{ "font-size": "11px", color: "var(--text-muted)" }}>
        {new Date(props.verification.ts).toLocaleTimeString()}
      </div>
    </div>
  );
}

// ─── Main Component ─────────────────────────────────────────────────────────

const PlanVisualization: Component = () => {
  const plan = createMemo(() => appStore.latestPlan());
  const stepResults = createMemo(() => appStore.planStepResults());
  const verification = createMemo(() => appStore.latestGoalVerification());

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
          Plan Visualization
        </h2>
        <Show when={plan()}>
          <span style={{ "font-size": "12px", color: "var(--text-muted)" }}>
            Goal: {plan()!.goal}
          </span>
        </Show>
      </div>

      {/* Goal Verification */}
      <Show when={verification()}>
        <GoalVerificationBanner verification={verification()!} />
      </Show>

      {/* No Plan State */}
      <Show
        when={plan()}
        fallback={
          <div
            style={{
              flex: "1",
              display: "flex",
              "align-items": "center",
              "justify-content": "center",
              color: "var(--text-muted)",
              "font-size": "14px",
            }}
          >
            <div style={{ "text-align": "center" }}>
              <div style={{ "font-size": "48px", "margin-bottom": "16px", opacity: "0.3" }}>🧠</div>
              <div>No plan generated yet.</div>
              <div style={{ "font-size": "12px", "margin-top": "4px", opacity: "0.7" }}>
                The 7B planner will generate 3 structured paths when a complex task arrives.
              </div>
            </div>
          </div>
        }
      >
        {/* Selection Reason */}
        <div
          style={{
            background: "var(--surface-1)",
            border: "1px solid var(--border)",
            "border-radius": "var(--radius-sm)",
            padding: "10px 14px",
            "font-size": "12px",
            color: "var(--text-secondary)",
          }}
        >
          <strong style={{ color: "var(--accent)" }}>
            Path {String.fromCharCode(65 + plan()!.winner_index)}
          </strong>{" "}
          selected: {plan()!.selection_reason}
        </div>

        {/* 3 Path Cards */}
        <div
          style={{
            display: "flex",
            gap: "12px",
            flex: "1",
            "min-height": "0",
            overflow: "auto",
          }}
        >
          <For each={plan()!.paths}>
            {(path, i) => (
              <PathCard
                path={path}
                index={i()}
                stepResults={
                  path.is_winner ? stepResults() : []
                }
              />
            )}
          </For>
        </div>
      </Show>
    </div>
  );
};

export default PlanVisualization;
