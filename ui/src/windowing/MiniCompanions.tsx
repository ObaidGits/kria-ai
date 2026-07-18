import {
  createMemo,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
  type JSX,
} from "solid-js";
import { CorePresence } from "../components/CorePresence";
import { Button, EmptyState, IconButton, Input } from "../kit";
import {
  approvalStore,
  converseStore,
  coreStore,
  observatoryStore,
  shellStore,
} from "../stores";
import { JobRow } from "../shell/spaces/observatory/JobRow";
import { SystemPulse } from "../shell/spaces/observatory/SystemPulse";
import {
  closeInlineCompanion,
  inlineCompanion,
  type CompanionSurface as CompanionSurfaceKind,
} from "./detachableSurfaces";
import "../shell/spaces/ObservatorySpace.css";
import "./MiniCompanions.css";

export const MINI_INTENT_MAX_LENGTH = 4096;
export const NOW_MINI_JOB_CAP = 3;

export function normalizeMiniIntent(value: string): string {
  return value.trim().slice(0, MINI_INTENT_MAX_LENGTH);
}

function CompanionActions() {
  return (
    <div class="kria-mini__actions">
      <Show when={approvalStore.hasPending()}>
        <Button size="sm" variant="secondary" onClick={() => shellStore.setApprovalsOpen(true)}>
          Approvals ({approvalStore.pendingCount()})
        </Button>
      </Show>
      <Button
        size="sm"
        variant="danger"
        disabled={!coreStore.isActive()}
        onClick={() => void converseStore.stopTurn()}
      >
        Stop
      </Button>
    </div>
  );
}

export function KriaMiniCompanion() {
  const [intent, setIntent] = createSignal("");
  const [submitting, setSubmitting] = createSignal(false);

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const text = normalizeMiniIntent(intent());
    if (!text || submitting()) return;
    setSubmitting(true);
    const accepted = await converseStore.submitIntent(text);
    if (accepted) setIntent("");
    setSubmitting(false);
  }

  return (
    <section class="kria-mini kria-mini--intent" aria-label="KRIA Mini">
      <header class="kria-mini__header">
        <div class="kria-mini__presence">
          <CorePresence size="md" />
          <div><strong>KRIA</strong><span>Mini</span></div>
        </div>
        <CompanionActions />
      </header>
      <form class="kria-mini__intent" onSubmit={submit as JSX.EventHandlerUnion<HTMLFormElement, SubmitEvent>}>
        <Input
          label="Intent for KRIA"
          hideLabel
          value={intent()}
          onChange={setIntent}
          placeholder="Ask KRIA…"
          inputProps={{ maxlength: MINI_INTENT_MAX_LENGTH, autocomplete: "off" }}
        />
        <Button type="submit" size="sm" disabled={!normalizeMiniIntent(intent()) || submitting()}>
          {submitting() ? "Sending…" : "Send"}
        </Button>
      </form>
    </section>
  );
}

function metric(value: number | undefined): string {
  return Number.isFinite(value) ? `${Math.round(value!)}%` : "Awaiting data";
}

export function NowMiniCompanion() {
  const activeJobs = createMemo(() => observatoryStore.jobs()
    .filter((job) => ["queued", "running", "paused"].includes(job.status))
    .sort((a, b) => a.startedAt - b.startedAt)
    .slice(0, NOW_MINI_JOB_CAP));
  const hiddenJobs = createMemo(() => Math.max(0, observatoryStore.jobs()
    .filter((job) => ["queued", "running", "paused"].includes(job.status)).length - NOW_MINI_JOB_CAP));

  onMount(() => void observatoryStore.loadExecutiveSnapshot());
  onCleanup(observatoryStore.connectExecutiveEvents());
  onCleanup(observatoryStore.connectTelemetry());
  onCleanup(observatoryStore.connectJobs());

  return (
    <section class="kria-mini kria-mini--now kria-observatory" aria-label="Now mini">
      <header class="kria-mini__header">
        <div><strong>Now</strong><span>Runtime read model</span></div>
        <CompanionActions />
      </header>
      <SystemPulse
        authority={observatoryStore.telemetryAuthority()}
        metrics={observatoryStore.resourceMetrics()}
        jobs={observatoryStore.jobs()}
      />
      <dl class="kria-mini__metrics">
        <div><dt>CPU</dt><dd>{metric(observatoryStore.resourceMetrics().cpu_percent)}</dd></div>
        <div><dt>Memory</dt><dd>{metric(observatoryStore.resourceMetrics().memory_percent)}</dd></div>
      </dl>
      <section class="kria-mini__jobs" aria-labelledby="now-mini-jobs">
        <h2 id="now-mini-jobs">Running jobs</h2>
        <Show when={activeJobs().length > 0} fallback={
          <EmptyState icon="clock" title="No running jobs" description="KRIA has no active cancellable work." />
        }>
          <ul><For each={activeJobs()}>{(job) =>
            <JobRow job={job} onCancel={observatoryStore.cancelJob} />
          }</For></ul>
          <Show when={hiddenJobs() > 0}>
            <p class="kria-mini__cap-note">+{hiddenJobs()} more in Observatory</p>
          </Show>
        </Show>
      </section>
    </section>
  );
}

export function MiniCompanionSurface(props: { surface: CompanionSurfaceKind }) {
  return props.surface === "kria-mini" ? <KriaMiniCompanion /> : <NowMiniCompanion />;
}

export function CompanionFallbackHost() {
  return (
    <Show when={inlineCompanion.surface()} keyed>
      {(surface) => (
        <aside class="kria-companion-fallback" aria-label={`${surface} fallback companion`}>
          <IconButton icon="x" label="Close companion" onClick={closeInlineCompanion} />
          <MiniCompanionSurface surface={surface} />
        </aside>
      )}
    </Show>
  );
}