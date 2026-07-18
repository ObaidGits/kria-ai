import { For, Show } from "solid-js";
import { Card, EmptyState } from "../../../kit";
import type { AnalyticsTile, DataAuthority } from "../../../stores";
import { HonestyBadge } from "./HonestyBadge";

export function AnalyticsTiles(props: { tiles: AnalyticsTile[]; authority: DataAuthority }) {
  return (
    <section aria-labelledby="analytics-heading">
      <div class="kria-observatory__region-head">
        <h2 id="analytics-heading">Analytics</h2>
        <HonestyBadge authority={props.authority} />
      </div>
      <Show when={props.tiles.length > 0} fallback={
        <EmptyState icon="chart-no-axes-column" title="No analytics yet"
          description={props.authority === "shadow-mode" ? "Analytics substrate unavailable; advisory view has no data." : "Awaiting an authoritative analytics snapshot."} />
      }>
        <div class="kria-observatory__tiles">
          <For each={props.tiles}>{(tile) => (
            <Card class="kria-observatory__tile">
              <span>{tile.label}</span>
              <strong>{tile.value.toLocaleString()}</strong>
              <small>{tile.unit}{tile.trend ? ` · ${tile.trend}` : ""}</small>
            </Card>
          )}</For>
        </div>
      </Show>
    </section>
  );
}
