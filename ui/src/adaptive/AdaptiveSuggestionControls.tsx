import { createMemo } from "solid-js";
import { IconButton } from "../kit";
import {
  dismissAdaptiveSuggestion,
  explainAdaptiveSuggestion,
  isAdaptivePinned,
  setAdaptivePinned,
  type AdaptiveZone,
} from "./presentationRanking";
import "./adaptive.css";

export interface AdaptiveSuggestionControlsProps {
  zone: AdaptiveZone;
  id: string;
  label: string;
  onPreferenceChange?: () => void;
}

/** Presentation-only controls. Never dispatches capabilities or runtime work. */
export function AdaptiveSuggestionControls(props: AdaptiveSuggestionControlsProps) {
  const pinned = createMemo(() => isAdaptivePinned(props.zone, props.id));
  const explanation = createMemo(() => explainAdaptiveSuggestion(props.zone, props.id));

  return (
    <div class="kria-adaptive-controls" role="group" aria-label={`Suggestion controls for ${props.label}`}>
      <span class="kria-adaptive-controls__why" role="note">{explanation()}</span>
      <IconButton
        icon="pin"
        size="sm"
        label={pinned() ? `Unpin suggestion: ${props.label}` : `Pin suggestion: ${props.label}`}
        aria-pressed={pinned()}
        onClick={() => {
          setAdaptivePinned(props.zone, props.id, !pinned());
          props.onPreferenceChange?.();
        }}
      />
      <IconButton
        icon="x"
        size="sm"
        label={`Dismiss suggestion: ${props.label}`}
        onClick={() => {
          dismissAdaptiveSuggestion(props.zone, props.id);
          props.onPreferenceChange?.();
        }}
      />
    </div>
  );
}

export default AdaptiveSuggestionControls;
