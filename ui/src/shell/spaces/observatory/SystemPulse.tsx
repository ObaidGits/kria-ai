import { createMemo } from "solid-js";
import { Card, StatusDot } from "../../../kit";
import type { DataAuthority, Job } from "../../../stores";
import { HonestyBadge } from "./HonestyBadge";

export function SystemPulse(props: {
  authority: DataAuthority;
  metrics: Record<string, number>;
  jobs: Job[];
}) {
  const running = createMemo(() => props.jobs.filter((job) => job.status === "running").length);
  const tone = () => props.authority === "error" ? "error"
    : running() > 0 ? "busy" : props.authority === "live" ? "online" : "offline";
  const label = () => props.authority === "error" ? "Needs attention"
    : running() > 0 ? `${running()} running job${running() === 1 ? "" : "s"}`
      : props.authority === "live" ? "Systems nominal" : "No authoritative sample";

  return (
    <Card class="kria-observatory__pulse" aria-label="System pulse">
      <div class="kria-observatory__card-head">
        <h2>System pulse</h2>
        <HonestyBadge authority={props.authority} />
      </div>
      <div class="kria-observatory__pulse-state" role="status" aria-live="polite">
        <StatusDot tone={tone()} label={label()} />
        <strong>{label()}</strong>
      </div>
      <p>{Object.keys(props.metrics).length} resource channels · {running()} active jobs</p>
    </Card>
  );
}
