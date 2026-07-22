/** LLM Status — relocatable panel (candidate: Developer Observatory). */
import { For } from "solid-js";
import { CcIcon } from "../CcIcon";
import { Panel } from "../parts";
import { PROVIDERS, PROVIDER_STATE_LABEL } from "../data";

export function LlmStatusPanel() {
  return (
    <Panel title="LLM Status" action="4 Connected" class="cc-llm">
      <div class="cc-llm-grid">
        <For each={PROVIDERS}>
          {(p) => (
            <div class="cc-provider" data-state={p.state}>
              <span class="cc-provider__icon"><CcIcon name={p.icon} size={14} /></span>
              <span class="cc-provider__name">{p.name}</span>
              <span class="cc-provider__state"><span class="cc-dot" />{PROVIDER_STATE_LABEL[p.state]}</span>
            </div>
          )}
        </For>
      </div>
      <button type="button" class="cc-panel__foot cc-panel__foot--center">Manage Providers <CcIcon name="chevron" size={12} /></button>
    </Panel>
  );
}

export default LlmStatusPanel;
