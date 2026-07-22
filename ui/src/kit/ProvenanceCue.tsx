import { Icon } from "../components/Icon";
import "./ProvenanceCue.css";

export type ProvenanceSource = "kria" | "user";

export interface ProvenanceCueProps {
  source: ProvenanceSource;
  label?: string;
  class?: string;
}

/**
 * Canonical authorship cue for user and KRIA-generated content/actions.
 * Icon + text make provenance visible without relying on color (Req 20.5).
 */
export function ProvenanceCue(props: ProvenanceCueProps) {
  const isKria = () => props.source === "kria";
  const label = () => props.label ?? (isKria() ? "KRIA" : "You");

  return (
    <span
      class={`kit-provenance${props.class ? ` ${props.class}` : ""}`}
      data-provenance-cue={props.source}
      aria-label={isKria() ? "AI-authored by KRIA" : "User-authored"}
    >
      <Icon name={isKria() ? "sparkles" : "user"} size="caption" aria-hidden />
      <span>{label()}</span>
    </span>
  );
}

export default ProvenanceCue;
