/**
 * EvidenceItemDetail — Renders detailed semantics for a single EvidenceItem.
 *
 * Renders:
 *   - source         (data-field="source")
 *   - locator        (data-field="locator")  — only when non-null
 *   - method         (data-field="method")
 *   - version        (data-field="version")
 *   - polarity       (data-field="polarity", data-polarity) — "support" → "Supports", "contradict" → "Contradicts"
 *   - score          (data-field="score")    — only when non-null; formatted as "X/1.0"
 *   - semanticsLabel (data-field="semantics-label")
 *   - policyLabel    (data-field="policy-label") — only when non-null
 *
 * All labels come from the backend; UI never invents text.
 *
 * Requirements: F4.4 (task 4.4.2)
 */
import { Show } from "solid-js";
import type { EvidenceItem } from "./Inspector";

export interface EvidenceItemDetailProps {
  item: EvidenceItem;
}

export function EvidenceItemDetail(props: EvidenceItemDetailProps) {
  const item = () => props.item;

  const polarityText = () =>
    item().polarity === "support" ? "Supports" : "Contradicts";

  const formattedScore = () => {
    const s = item().score;
    if (s === null) return null;
    return `${s}/1.0`;
  };

  return (
    <div data-testid={`evidence-detail-${item().id}`}>
      <span data-field="source">{item().source}</span>

      <Show when={item().locator !== null}>
        <span data-field="locator">{item().locator}</span>
      </Show>

      <span data-field="method">{item().method}</span>

      <span data-field="version">{item().version}</span>

      <span
        data-field="polarity"
        data-polarity={item().polarity}
      >
        {polarityText()}
      </span>

      <Show when={formattedScore() !== null}>
        <span data-field="score">{formattedScore()}</span>
      </Show>

      <span data-field="semantics-label">{item().semanticsLabel}</span>

      <Show when={item().policyLabel !== null}>
        <span data-field="policy-label">{item().policyLabel}</span>
      </Show>
    </div>
  );
}

export default EvidenceItemDetail;
