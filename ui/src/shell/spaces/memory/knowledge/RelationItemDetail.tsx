/**
 * RelationItemDetail — Renders detailed semantics for a single RelationItem.
 *
 * Renders:
 *   - direction      (data-field="direction", data-direction)
 *                      "outgoing"  → "→"
 *                      "incoming"  → "←"
 *                      "symmetric" → "↔"
 *   - registryLabel  (data-field="registry-label")
 *   - sourceLabel    (data-field="source-label")
 *   - targetLabel    (data-field="target-label")
 *   - evidenceCount  (data-field="evidence-count")
 *   - validity       (data-field="validity")
 *
 * All labels come from the backend; UI never invents text.
 *
 * Requirements: F4.4 (task 4.4.2)
 */
import type { RelationItem } from "./Inspector";

export interface RelationItemDetailProps {
  item: RelationItem;
}

const DIRECTION_ARROWS: Record<RelationItem["direction"], string> = {
  outgoing: "→",
  incoming: "←",
  symmetric: "↔",
};

export function RelationItemDetail(props: RelationItemDetailProps) {
  const item = () => props.item;

  const arrow = () => DIRECTION_ARROWS[item().direction];

  return (
    <div data-testid={`relation-detail-${item().id}`}>
      <span
        data-field="direction"
        data-direction={item().direction}
      >
        {arrow()}
      </span>

      <span data-field="registry-label">{item().registryLabel}</span>

      <span data-field="source-label">{item().sourceLabel}</span>

      <span data-field="target-label">{item().targetLabel}</span>

      <span data-field="evidence-count">{item().evidenceCount}</span>

      <span data-field="validity">{item().validity}</span>
    </div>
  );
}

export default RelationItemDetail;
