/** Live Intelligence Feed — relocatable panel (candidate: Command Deck). */
import { For, Show } from "solid-js";
import { CcIcon } from "../CcIcon";
import { Panel } from "../parts";
import { FEED } from "../data";

export function IntelligenceFeedPanel() {
  return (
    <Panel title="Live Intelligence Feed" action="View All Intelligence" class="cc-feed">
      <ul class="cc-feed-list">
        <For each={FEED}>
          {(item) => (
            <li class="cc-feed-row" data-tag={item.tag.toLowerCase()}>
              <span class="cc-feed-icon"><CcIcon name={item.icon} size={16} /></span>
              <span class="cc-feed-text">
                <span class="cc-feed-title">{item.title}</span>
                <span class="cc-feed-sub">{item.sub}</span>
              </span>
              <Show
                when={item.action}
                fallback={<span class="cc-tag" data-tag={item.tag.toLowerCase()}>{item.tag}</span>}
              >
                <button type="button" class="cc-mini-btn">{item.action}</button>
              </Show>
            </li>
          )}
        </For>
      </ul>
    </Panel>
  );
}

export default IntelligenceFeedPanel;
