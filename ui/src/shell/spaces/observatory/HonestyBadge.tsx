import { Badge } from "../../../kit";
import type { DataAuthority } from "../../../stores";

const COPY: Record<DataAuthority, string> = {
  "awaiting-data": "Awaiting data",
  live: "Live telemetry",
  "shadow-mode": "Shadow mode · advisory",
  error: "Telemetry error",
};

export function HonestyBadge(props: { authority: DataAuthority }) {
  const tone = () => props.authority === "live" ? "success"
    : props.authority === "error" ? "danger"
      : props.authority === "shadow-mode" ? "warning" : "neutral";
  return <Badge tone={tone()}>{COPY[props.authority]}</Badge>;
}
