/**
 * Progress — on Kobalte Progress (correct role="progressbar" + aria-value*).
 * Determinate or indeterminate; optional label and value text.
 */
import { Progress as KProgress } from "@kobalte/core/progress";
import { splitProps, Show } from "solid-js";
import "./kit.base.css";
import "./Progress.css";

export interface ProgressProps {
  label?: string;
  value?: number;
  minValue?: number;
  maxValue?: number;
  indeterminate?: boolean;
  tone?: "accent" | "success" | "warning" | "danger";
  /** Show the numeric value label (defaults to true when determinate). */
  showValue?: boolean;
  class?: string;
}

export function Progress(props: ProgressProps) {
  const [local] = splitProps(props, [
    "label",
    "value",
    "minValue",
    "maxValue",
    "indeterminate",
    "tone",
    "showValue",
    "class",
  ]);
  const tone = () => local.tone ?? "accent";
  const showValue = () => (local.showValue ?? !local.indeterminate) && !local.indeterminate;

  return (
    <KProgress
      class={`kit-progress ${local.class ?? ""}`}
      value={local.value}
      minValue={local.minValue}
      maxValue={local.maxValue}
      indeterminate={local.indeterminate}
    >
      <Show when={local.label || showValue()}>
        <div class="kit-progress__header">
          <Show when={local.label}>
            <KProgress.Label class="kit-progress__label">{local.label}</KProgress.Label>
          </Show>
          <Show when={showValue()}>
            <KProgress.ValueLabel class="kit-progress__value" />
          </Show>
        </div>
      </Show>
      <KProgress.Track class="kit-progress__track">
        <KProgress.Fill class={`kit-progress__fill kit-progress__fill--tone-${tone()}`} />
      </KProgress.Track>
    </KProgress>
  );
}

export default Progress;
