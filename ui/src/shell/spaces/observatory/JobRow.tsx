import { createSignal, Show } from "solid-js";
import { Button, Progress, StatusDot } from "../../../kit";
import type { Job } from "../../../stores";

export function JobRow(props: { job: Job; onCancel: (id: string) => Promise<boolean> }) {
  const [cancelling, setCancelling] = createSignal(false);
  const [failed, setFailed] = createSignal(false);
  const canCancel = () => !!props.job.cancelKind && ["queued", "running", "paused"].includes(props.job.status);
  const tone = () => props.job.status === "failed" || props.job.status === "timed_out" ? "error"
    : props.job.status === "running" ? "busy"
      : props.job.status === "completed" || props.job.status === "recovered" ? "online" : "offline";

  async function cancel() {
    setCancelling(true);
    setFailed(false);
    const ok = await props.onCancel(props.job.id);
    setFailed(!ok);
    setCancelling(false);
  }

  return (
    <li class="kria-observatory__job-row">
      <div class="kria-observatory__job-main">
        <strong>{props.job.name}</strong>
        <span><StatusDot tone={tone()} label={props.job.status} /> {props.job.status}</span>
        <Show when={Number.isFinite(props.job.progress)}>
          <Progress label={`${props.job.name} progress`} value={props.job.progress!} />
        </Show>
        <Show when={props.job.error}><span role="alert">{props.job.error}</span></Show>
        <Show when={failed()}><span role="alert">Cancellation failed; job remains authoritative.</span></Show>
      </div>
      <Button variant="secondary" size="sm" disabled={!canCancel() || cancelling()} onClick={cancel}>
        {cancelling() ? "Cancelling…" : "Cancel"}
      </Button>
    </li>
  );
}
