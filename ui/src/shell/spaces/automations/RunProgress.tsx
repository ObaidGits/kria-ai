/**
 * RunProgress — live progress for a single workflow run (task 7.2, Req 6.5).
 *
 * Consumes a {@link RunProgress} entry from `automationStore.runProgress`
 * (fed by the Tauri bridge from existing n8n/workflow run events) and renders
 * the kit `Progress` (correct `role="progressbar"` + aria-value*). Phase is
 * conveyed by ICON + TEXT (never color alone — Req 17.3). While the total step
 * count is unknown and the run is active, the bar is indeterminate; once steps
 * are known it becomes determinate; terminal phases render a static frame (no
 * ambient motion, Req 16).
 *
 * Presentation only — no dispatch.
 *
 * Requirements: 6.5, 17.3
 */
import { Show } from "solid-js";
import { Progress } from "../../../kit";
import { Icon } from "../../../components/Icon";
import type { RunPhase, RunProgress as RunProgressData } from "../../../stores";
import "./run.css";

const PHASE_META: Record<RunPhase, { label: string; icon: string; active: boolean }> = {
  triggering: { label: "Starting", icon: "loader", active: true },
  running: { label: "Running", icon: "loader", active: true },
  waiting: { label: "Waiting for approval", icon: "pause", active: true },
  completed: { label: "Completed", icon: "check", active: false },
  failed: { label: "Failed", icon: "x", active: false },
  cancelled: { label: "Cancelled", icon: "square", active: false },
};

function toneFor(phase: RunPhase): "accent" | "success" | "warning" | "danger" {
  if (phase === "completed") return "success";
  if (phase === "failed") return "danger";
  if (phase === "waiting" || phase === "cancelled") return "warning";
  return "accent";
}

export interface RunProgressProps {
  progress: RunProgressData;
}

export function RunProgress(props: RunProgressProps) {
  const p = () => props.progress;
  const meta = () => PHASE_META[p().phase];
  const total = () => p().totalSteps;
  const determinate = () => typeof total() === "number" && (total() as number) > 0;
  // Indeterminate only while the run is active AND the step count is unknown.
  const indeterminate = () => meta().active && !determinate();
  // Always resolve a numeric value for the determinate track: known steps →
  // proportion; a completed run with unknown total → full; otherwise empty.
  const percent = () => {
    if (determinate()) return Math.round((p().completedSteps / (total() as number)) * 100);
    if (p().phase === "completed") return 100;
    return 0;
  };

  const label = () =>
    determinate() ? `Step ${p().completedSteps} of ${total()}` : meta().label;

  return (
    <div class="kria-runprogress" data-phase={p().phase}>
      <div class="kria-runprogress__head">
        <span class="kria-runprogress__phase">
          <Icon name={meta().icon} size={13} aria-hidden />
          <span>{meta().label}</span>
        </span>
      </div>
      <Progress
        label={label()}
        value={percent()}
        minValue={0}
        maxValue={100}
        indeterminate={indeterminate()}
        showValue={false}
        tone={toneFor(p().phase)}
      />
      <Show when={p().message}>
        <p class="kria-runprogress__message">{p().message}</p>
      </Show>
    </div>
  );
}

export default RunProgress;
