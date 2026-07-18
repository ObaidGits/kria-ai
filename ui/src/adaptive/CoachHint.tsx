import { Show, type JSX } from "solid-js";
import { IconButton } from "../kit";
import { retireCoachHint, shouldShowCoachHint } from "./presentationRanking";
import "./adaptive.css";

export interface CoachHintProps {
  featureId: string;
  children: JSX.Element;
}

/** One-time contextual hint; feature owners retire it on first real use. */
export function CoachHint(props: CoachHintProps) {
  return (
    <Show when={shouldShowCoachHint(props.featureId)}>
      <aside class="kria-coach-hint" role="note" aria-label="Getting started hint">
        <span>{props.children}</span>
        <IconButton
          icon="x"
          size="sm"
          label="Dismiss hint"
          onClick={() => retireCoachHint(props.featureId)}
        />
      </aside>
    </Show>
  );
}

export default CoachHint;
