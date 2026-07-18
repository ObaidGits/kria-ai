import { For, Show } from "solid-js";
import { createVirtualizer } from "@tanstack/solid-virtual";
import { EmptyState } from "../../../kit";
import type { DataAuthority, ForensicRecord } from "../../../stores";
import { elementRectObserver } from "../../../utils/virtualization";
import { HonestyBadge } from "./HonestyBadge";

function severityLabel(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (["critical", "error", "high"].includes(normalized)) return "High severity";
  if (["warning", "warn", "medium"].includes(normalized)) return "Warning";
  return "Information";
}

export function ForensicTimeline(props: { records: ForensicRecord[]; authority: DataAuthority }) {
  let scrollEl: HTMLDivElement | undefined;
  const virtualizer = createVirtualizer({
    get count() { return props.records.length; },
    getScrollElement: () => scrollEl ?? null,
    estimateSize: () => 92,
    overscan: 6,
    getItemKey: (index) => props.records[index]?.id ?? index,
    initialRect: { width: 720, height: 520 },
    observeElementRect: elementRectObserver({ width: 720, height: 520 }),
  });

  return (
    <section aria-labelledby="forensics-heading">
      <div class="kria-observatory__region-head">
        <h2 id="forensics-heading">Forensics &amp; recovery</h2>
        <HonestyBadge authority={props.authority} />
      </div>
      <Show when={props.records.length > 0} fallback={
        <EmptyState icon="shield-check" title="No forensic records"
          description={props.authority === "shadow-mode" ? "Forensic service unavailable; no recovery claim can be made." : "Awaiting authoritative forensic events."} />
      }>
        <div ref={scrollEl} class="kria-observatory__timeline-viewport" data-virtual-list="forensic-timeline">
          <ol class="kria-observatory__timeline kria-observatory__timeline-sizer"
            style={{ height: `${virtualizer.getTotalSize()}px` }}>
            <For each={virtualizer.getVirtualItems()}>{(row) => {
              const record = () => props.records[row.index];
              return (
                <Show when={record()}>
                  <li data-record-id={record()!.id} data-index={row.index}
                    ref={(el) => queueMicrotask(() => virtualizer.measureElement(el))}
                    style={{ transform: `translateY(${row.start}px)` }}>
                    <time dateTime={new Date(record()!.timestamp_unix_ms).toISOString()}>
                      {new Date(record()!.timestamp_unix_ms).toLocaleString()}
                    </time>
                    <div>
                      <strong>{record()!.summary}</strong>
                      <span>{severityLabel(record()!.severity)} · {record()!.category} · {record()!.source}</span>
                      <Show when={record()!.evidence}><p>{record()!.evidence}</p></Show>
                      <Show when={record()!.last_gasp_detected}><b>Last-gasp evidence captured</b></Show>
                    </div>
                  </li>
                </Show>
              );
            }}</For>
          </ol>
        </div>
      </Show>
    </section>
  );
}
