/** Mission Timeline — relocatable panel (candidate: Command Deck). */
import { For, Show } from "solid-js";
import { CcIcon } from "../CcIcon";
import { Panel } from "../parts";
import { TIMELINE } from "../data";

export function TimelinePanel() {
  return (
    <Panel title="Mission Timeline" action="Today" class="cc-timeline">
      <ul class="cc-tl-list">
        <For each={TIMELINE}>
          {(t) => (
            <li class="cc-tl-row" data-done={t.done ? "true" : "false"}>
              <span class="cc-tl-time">{t.time}</span>
              <span class="cc-tl-dot">
                <Show when={t.done} fallback={<i />}><CcIcon name="check" size={10} /></Show>
              </span>
              <span class="cc-tl-body">
                <span class="cc-tl-title">{t.title}</span>
                <span class="cc-tl-meta">{t.meta}</span>
              </span>
            </li>
          )}
        </For>
      </ul>
      <button type="button" class="cc-panel__foot">View Full Schedule <CcIcon name="chevron" size={12} /></button>
    </Panel>
  );
}

export default TimelinePanel;
