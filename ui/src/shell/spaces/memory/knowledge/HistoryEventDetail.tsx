/**
 * HistoryEventDetail — Renders detailed semantics for a single HistoryEvent.
 *
 * Renders:
 *   - eventType    (data-field="event-type", data-event-type, data-event-class)
 *                    "correction"   → label "Correction applied", class "mutation"
 *                    "supersession" → label "Superseded",          class "lifecycle"
 *                    "contradiction"→ label "Contradiction recorded", class "truth"
 *                    "creation"     → label "Created",              class "lifecycle"
 *                    "deletion"     → label "Deleted",              class "lifecycle"
 *                    anything else  → raw value from backend,       class omitted (no data-event-class)
 *   - timestamp    (data-field="timestamp")
 *   - actor        (data-field="actor") — only when non-null
 *   - description  (data-field="description")
 *
 * All labels come from the backend; UI never invents text beyond the fixed
 * event-type label map defined in the spec.
 *
 * Requirements: F4.4 (task 4.4.2)
 */
import { Show } from "solid-js";
import type { HistoryEvent } from "./Inspector";

export interface HistoryEventDetailProps {
  event: HistoryEvent;
}

interface EventTypeMeta {
  label: string;
  eventClass: string;
}

const EVENT_TYPE_MAP: Record<string, EventTypeMeta> = {
  correction:    { label: "Correction applied",     eventClass: "mutation"   },
  supersession:  { label: "Superseded",             eventClass: "lifecycle"  },
  contradiction: { label: "Contradiction recorded", eventClass: "truth"      },
  creation:      { label: "Created",                eventClass: "lifecycle"  },
  deletion:      { label: "Deleted",                eventClass: "lifecycle"  },
};

export function HistoryEventDetail(props: HistoryEventDetailProps) {
  const ev = () => props.event;

  const meta = () => EVENT_TYPE_MAP[ev().eventType] ?? null;

  const eventLabel = () => meta()?.label ?? ev().eventType;

  return (
    <div data-testid={`history-detail-${ev().id}`}>
      <span
        data-field="event-type"
        data-event-type={ev().eventType}
        data-event-class={meta()?.eventClass}
      >
        {eventLabel()}
      </span>

      <span data-field="timestamp">{ev().timestamp}</span>

      <Show when={ev().actor !== null}>
        <span data-field="actor">{ev().actor}</span>
      </Show>

      <span data-field="description">{ev().description}</span>
    </div>
  );
}

export default HistoryEventDetail;
