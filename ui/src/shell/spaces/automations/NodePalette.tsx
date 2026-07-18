/**
 * NodePalette — the list of node types you can add to the 2D builder canvas
 * (task 7.3, Req 6.4). Curated in-house (no backend node catalog command
 * exists); each item maps to a real `n8n-nodes-base.*` type so a saved draft is
 * a valid n8n workflow.
 *
 * Two ways to add a node (Req 17.1 keyboard-first):
 *   • Click / Enter / Space on a palette button → click-to-add (adds the node
 *     to the canvas + selects it, opening its Inspector).
 *   • Drag a palette item onto the canvas → drop-to-add at the drop point
 *     (pointer enhancement; the click path is the accessible fallback).
 *
 * Presentation-only: adding a node mutates LOCAL draft state via
 * `automationStore.addNode`; nothing is dispatched to the backend until the
 * author saves/tests/approves (KRIA runtime-authority invariant).
 *
 * Requirements: 6.4, 17.1
 */
import { For } from "solid-js";
import { Icon } from "../../../components/Icon";
import { NODE_PALETTE, automationStore } from "../../../stores";
import type { NodePaletteItem } from "../../../stores";
import "./builder.css";

export interface NodePaletteProps {
  /** Override the add handler (tests/stories). Defaults to the store. */
  onAdd?: (kind: string) => void;
}

export function NodePalette(props: NodePaletteProps) {
  const add = (kind: string) =>
    props.onAdd ? props.onAdd(kind) : automationStore.addNode(kind);

  return (
    <div class="kria-nb-palette" role="group" aria-label="Node palette">
      <h3 class="kria-nb-palette__title">Nodes</h3>
      <p class="kria-nb-palette__hint">Click a node to add it, or drag it onto the canvas.</p>
      <ul class="kria-nb-palette__list">
        <For each={NODE_PALETTE}>
          {(item: NodePaletteItem) => (
            <li>
              <button
                type="button"
                class="kria-nb-palette__item"
                draggable={true}
                aria-label={`Add ${item.label} node`}
                title={item.description}
                onClick={() => add(item.kind)}
                onDragStart={(e) => {
                  e.dataTransfer?.setData("application/x-kria-node", item.kind);
                  if (e.dataTransfer) e.dataTransfer.effectAllowed = "copy";
                }}
              >
                <span class="kria-nb-palette__icon" aria-hidden="true">
                  <Icon name={item.icon} size={16} />
                </span>
                <span class="kria-nb-palette__label">{item.label}</span>
              </button>
            </li>
          )}
        </For>
      </ul>
    </div>
  );
}

export default NodePalette;
