/** Memory Insights — relocatable panel (candidate: Developer Observatory). */
import { For } from "solid-js";
import { Panel } from "../parts";
import { MEMORY_STATS } from "../data";

const CONSTELLATION: ReadonlyArray<[number, number]> = [
  [12, 60], [40, 30], [66, 52], [92, 22], [120, 44], [148, 20], [54, 72], [104, 68], [132, 64],
];

export function MemoryInsightsPanel() {
  return (
    <Panel title="Memory Insights" action="View Memory Map" class="cc-memory">
      <div class="cc-memory-grid">
        <svg class="cc-constellation" viewBox="0 0 160 90" aria-hidden="true">
          <polyline points="12,60 40,30 66,52 92,22 120,44 148,20" />
          <For each={CONSTELLATION}>{(p) => <circle cx={p[0]} cy={p[1]} r="2.4" />}</For>
        </svg>
        <ul class="cc-memory-stats">
          <For each={MEMORY_STATS}>
            {(s) => (
              <li><span class="cc-memory-num">{s.value}</span><span class="cc-memory-label">{s.label}</span></li>
            )}
          </For>
        </ul>
      </div>
    </Panel>
  );
}

export default MemoryInsightsPanel;
