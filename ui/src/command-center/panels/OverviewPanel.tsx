/** AI Core Overview — relocatable panel (candidate: Command Deck). */
import { For } from "solid-js";
import { CcIcon } from "../CcIcon";
import { Panel } from "../parts";
import { OVERVIEW } from "../data";

export function OverviewPanel() {
  return (
    <Panel title="AI Core Overview" class="cc-overview">
      <ul class="cc-ov-list">
        <For each={OVERVIEW}>
          {(row) => (
            <li class="cc-ov-row">
              <span class="cc-ov-icon"><CcIcon name={row.icon} size={16} /></span>
              <span class="cc-ov-label">{row.label}</span>
              <span class="cc-ov-detail" data-tone={row.tone}>
                <span class={`cc-dot cc-dot--${row.tone}`} />{row.detail}
              </span>
            </li>
          )}
        </For>
      </ul>
    </Panel>
  );
}

export default OverviewPanel;
