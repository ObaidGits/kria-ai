/** System Monitor — relocatable panel (candidate: Developer Observatory). */
import { For } from "solid-js";
import { Panel, RadialGauge } from "../parts";
import { GAUGES } from "../data";

export function SystemMonitorPanel() {
  return (
    <Panel title="System Monitor" class="cc-monitor">
      <div class="cc-gauges">
        <For each={GAUGES}>{(g) => <RadialGauge gauge={g} />}</For>
      </div>
    </Panel>
  );
}

export default SystemMonitorPanel;
