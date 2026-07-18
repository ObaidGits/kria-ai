/**
 * StatusDot — status indicator = colored dot + label. The dot is decorative
 * (aria-hidden); the label conveys meaning (Req 17.3). Set `hideLabel` to keep
 * the label for assistive tech only while showing just the dot.
 */
import { splitProps, Show } from "solid-js";
import "./kit.base.css";
import "./StatusDot.css";

export type StatusTone = "online" | "busy" | "error" | "info" | "offline";

export interface StatusDotProps {
  tone?: StatusTone;
  label: string;
  hideLabel?: boolean;
  pulse?: boolean;
  class?: string;
}

export function StatusDot(props: StatusDotProps) {
  const [local] = splitProps(props, ["tone", "label", "hideLabel", "pulse", "class"]);
  const tone = () => local.tone ?? "offline";

  return (
    <span
      class={`kit-status kit-status--${tone()} ${local.pulse ? "kit-status--pulse" : ""} ${local.class ?? ""}`}
      role="status"
    >
      <span class="kit-status__dot" aria-hidden="true" />
      <Show when={local.hideLabel} fallback={<span>{local.label}</span>}>
        <span class="kit-visually-hidden">{local.label}</span>
      </Show>
    </span>
  );
}

export default StatusDot;
