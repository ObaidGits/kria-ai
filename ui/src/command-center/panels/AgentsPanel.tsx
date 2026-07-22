/** Active Agents — relocatable panel (candidate: Command Deck / Developer). */
import { For } from "solid-js";
import { CcIcon } from "../CcIcon";
import { Panel, Waveform } from "../parts";
import { AGENTS } from "../data";

export function AgentsPanel() {
  return (
    <Panel title="Active Agents" action="View All" class="cc-agents">
      <div class="cc-agent-grid">
        <For each={AGENTS}>
          {(a) => (
            <div class="cc-agent" data-tone={a.tone}>
              <span class="cc-agent__icon"><CcIcon name={a.icon} size={16} /></span>
              <span class="cc-agent__name">{a.name}</span>
              <span class="cc-agent__status"><span class={`cc-dot cc-dot--${a.tone}`} />{a.status}</span>
              <Waveform bars={14} class="cc-agent__wave" />
            </div>
          )}
        </For>
      </div>
    </Panel>
  );
}

export default AgentsPanel;
