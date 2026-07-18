/**
 * MemoryCard — the compact memory tile in the Explorer/landing lists (task 6.2,
 * Req 5.2). Shows a content preview plus accessible cues (confidence / worth /
 * staleness via kit `Badge`, each icon+text so meaning is never color-only —
 * Req 17.3) and the source. The whole card is a single semantic button: opening
 * it sets the shared Inspector target (`shellStore.openInspector("memory", …)`)
 * so the one shared Inspector shows the full detail + actions (Req 1.6 / 5.2).
 *
 * SECURITY: fact content is UNTRUSTED. It is rendered as text (Solid
 * auto-escapes) — never as HTML — so memory content cannot inject markup.
 *
 * Presentation only: a click routes a selection into `shellStore`; it performs
 * no memory mutation and no orchestration (KRIA runtime-authority invariant).
 *
 * Requirements: 5.2, 17.3
 */
import { For, Show } from "solid-js";
import type { MemoryFact } from "../../../stores";
import { shellStore } from "../../../stores";
import { Badge } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { confidenceCue, worthCue, stalenessCue, type MemoryCue } from "./memoryCues";
import "./MemoryCard.css";

export interface MemoryCardProps {
  fact: MemoryFact;
  /** Whether this card is the current Inspector target (visual selection). */
  selected?: boolean;
  /**
   * Open handler. Defaults to setting the shared Inspector target. Overridable
   * for stories/tests.
   */
  onOpen?: (fact: MemoryFact) => void;
}

function CueBadge(props: { cue: MemoryCue }) {
  return (
    <Badge tone={props.cue.tone} class="kria-memory-card__cue">
      <Icon name={props.cue.icon} size={13} />
      <span>{props.cue.label}</span>
    </Badge>
  );
}

export function MemoryCard(props: MemoryCardProps) {
  const open = () =>
    (props.onOpen ?? ((f: MemoryFact) => shellStore.openInspector("memory", f.id, f)))(props.fact);

  const cues = (): MemoryCue[] => [
    confidenceCue(props.fact.confidence),
    worthCue(props.fact.worth),
    stalenessCue(props.fact.staleness),
  ];

  return (
    <button
      type="button"
      class="kit-focusable kria-memory-card"
      data-fact-id={props.fact.id}
      aria-selected={props.selected ? "true" : "false"}
      aria-label={`Memory: ${props.fact.content}`}
      onClick={() => open()}
    >
      <p class="kria-memory-card__content">{props.fact.content}</p>
      <div class="kria-memory-card__meta">
        <For each={cues()}>{(cue) => <CueBadge cue={cue} />}</For>
        <Show when={props.fact.source}>
          <span class="kria-memory-card__source">
            <Icon name="database" size={13} aria-hidden />
            {props.fact.source}
          </span>
        </Show>
      </div>
    </button>
  );
}

export default MemoryCard;
