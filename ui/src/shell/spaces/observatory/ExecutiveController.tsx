import { createSignal, For, Show } from "solid-js";
import { Badge, Button, Card, EmptyState, StatusDot } from "../../../kit";
import type {
  ExecutiveSnapshot,
  ExecutiveTask,
  ExecutiveTaskCompleted,
  TaskPriority,
  TaskSource,
  TaskState,
} from "../../../types/intelligence";
import type { DataAuthority } from "../../../stores";
import { HonestyBadge } from "./HonestyBadge";

const PRIORITY_LABELS: Record<TaskPriority, string> = {
  Voice: "P0",
  Interactive: "P1",
  HitlResponse: "P2",
  Background: "P3",
  Maintenance: "P4",
};

function priorityTone(priority: TaskPriority) {
  if (priority === "Voice" || priority === "Interactive") return "accent" as const;
  if (priority === "HitlResponse") return "warning" as const;
  return "neutral" as const;
}

function stateTone(state: TaskState) {
  if (state === "Running") return "busy" as const;
  if (state === "Completed") return "online" as const;
  if (state === "Failed") return "error" as const;
  return "offline" as const;
}

function sourceLabel(source: TaskSource): string {
  if (typeof source === "string") return source.replace(/([a-z])([A-Z])/g, "$1 $2");
  return `Compiled skill: ${source.CompiledSkill}`;
}

function taskKind(source: TaskSource): string {
  return typeof source === "string" &&
    ["CuriosityLoop", "ProactiveScheduler", "Maintenance"].includes(source)
    ? "Cognition" : "Job";
}
function formatDuration(durationMs: number | null): string {
  if (durationMs == null) return "In progress";
  return durationMs < 1000 ? `${durationMs} ms` : `${(durationMs / 1000).toFixed(1)} s`;
}

function ExecutiveTaskRow(props: {
  task: ExecutiveTask;
  onCancel?: (taskId: string) => Promise<boolean>;
}) {
  const [cancelRequested, setCancelRequested] = createSignal(false);
  const [cancelFailed, setCancelFailed] = createSignal(false);

  async function requestCancel() {
    if (!props.onCancel || cancelRequested()) return;
    setCancelFailed(false);
    const accepted = await props.onCancel(props.task.id);
    setCancelRequested(accepted);
    setCancelFailed(!accepted);
  }

  return <li class="kria-observatory__executive-task">
    <div class="kria-observatory__executive-task-head">
      <div class="kria-observatory__executive-labels">
        <Badge tone={priorityTone(props.task.priority)}>{PRIORITY_LABELS[props.task.priority]}</Badge>
        <Badge>{taskKind(props.task.source)}</Badge>
        <StatusDot tone={stateTone(props.task.state)} label={props.task.state} />
      </div>
      <Show when={props.task.state === "Running" && props.onCancel}>
        <Button variant="secondary" size="sm" disabled={cancelRequested()} onClick={requestCancel}>
          {cancelRequested() ? "Cancel requested" : "Cancel"}
        </Button>
      </Show>
    </div>
    <strong>{props.task.description}</strong>
    <div class="kria-observatory__executive-meta">
      <span>{sourceLabel(props.task.source)}</span>
      <span>{formatDuration(props.task.duration_ms)}</span>
      <Show when={props.task.requires_gpu}><span>GPU required</span></Show>
    </div>
    <Show when={props.task.error}><span role="alert">{props.task.error}</span></Show>
    <Show when={cancelFailed()}>
      <span role="alert">Cancellation was not accepted; controller state is unchanged.</span>
    </Show>
  </li>;
}

function TaskGroup(props: {
  title: string;
  tasks: ExecutiveTask[];
  onCancel?: (taskId: string) => Promise<boolean>;
}) {
  return <Show when={props.tasks.length > 0}>
    <section class="kria-observatory__executive-group">
      <h3>{props.title} ({props.tasks.length})</h3>
      <ul class="kria-observatory__executive-list">
        <For each={props.tasks}>{(task) =>
          <ExecutiveTaskRow task={task} onCancel={props.onCancel} />
        }</For>
      </ul>
    </section>
  </Show>;
}
function ExecutiveEvents(props: { events: ExecutiveTaskCompleted[] }) {
  return <section class="kria-observatory__executive-group" aria-labelledby="executive-events-heading">
    <h3 id="executive-events-heading">Recent controller events ({props.events.length})</h3>
    <Show when={props.events.length > 0} fallback={
      <p>Awaiting controller completion and preemption events.</p>
    }>
      <ol class="kria-observatory__executive-events">
        <For each={props.events}>{(event) => <li>
          <time dateTime={event.ts}>{new Date(event.ts).toLocaleTimeString()}</time>
          <StatusDot tone={event.success ? "online" : "error"}
            label={event.success ? "Succeeded" : "Stopped or failed"} />
          <span>{event.output_summary || event.error || event.task_id}</span>
          <span>{formatDuration(event.duration_ms)}</span>
        </li>}</For>
      </ol>
    </Show>
  </section>;
}

export function ExecutiveController(props: {
  snapshot: ExecutiveSnapshot | null;
  events: ExecutiveTaskCompleted[];
  authority: DataAuthority;
  onCancel: (taskId: string) => Promise<boolean>;
}) {
  const foreground = () => props.snapshot?.active_foreground
    ? [props.snapshot.active_foreground] : [];

  return <div class="kria-observatory__executive">
    <div class="kria-observatory__card-head">
      <div>
        <h2>Executive controller</h2>
        <p>Authoritative, bounded view of scheduled jobs and background cognition.</p>
      </div>
      <HonestyBadge authority={props.authority} />
    </div>

    <Show when={props.snapshot} fallback={
      <EmptyState icon="brain" title="Awaiting executive controller"
        description="Live controller events will appear here. No substrate is treated as authoritative." />
    }>
      {(snapshot) => <>
        <dl class="kria-observatory__executive-stats">
          <div><dt>Active</dt><dd>{snapshot().active_background.length + foreground().length}</dd></div>
          <div><dt>Queued</dt><dd>{snapshot().queued.length}</dd></div>
          <div><dt>Completed</dt><dd>{snapshot().total_completed}</dd></div>
          <div><dt>Failed</dt><dd>{snapshot().total_failed}</dd></div>
        </dl>

        <Card class="kria-observatory__executive-lease">
          <StatusDot tone={snapshot().gpu_lease_holder ? "busy" : "offline"}
            label={snapshot().gpu_lease_holder ? "GPU lease active" : "GPU lease free"} />
          <Show when={snapshot().gpu_lease_holder}>
            <span>Held by {snapshot().gpu_lease_holder}</span>
          </Show>
        </Card>

        <TaskGroup title="Foreground" tasks={foreground()} />
        <TaskGroup title="Background jobs & cognition" tasks={snapshot().active_background}
          onCancel={props.onCancel} />
        <TaskGroup title="Queue" tasks={snapshot().queued} />
      </>}
    </Show>

    <ExecutiveEvents events={props.events} />
  </div>;
}

export default ExecutiveController;
