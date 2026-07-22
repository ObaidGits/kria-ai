import { createEffect, createMemo, For, onCleanup, onMount, Show } from "solid-js";
import { EmptyState, IconButton, Tabs } from "../../kit";
import { observatoryStore, shellStore, type ObservatorySegment } from "../../stores";
import { currentRoute, navigate } from "../router";
import { openCompanion, openDetachedSurface } from "../../windowing/detachableSurfaces";
import {
  AnalyticsTiles,
  ExecutiveController,
  ForensicTimeline,
  HraDiagnosticsPanel,
  HraForensicsPanel,
  HraNowPanel,
  JobRow,
  ResourceMeter,
  SystemPulse,
  TestConsole,
} from "./observatory";
import "./ObservatorySpace.css";

const SEGMENTS: ReadonlyArray<{ value: ObservatorySegment; label: string }> = [
  { value: "now", label: "Now" },
  { value: "jobs", label: "Jobs & Cognition" },
  { value: "analytics", label: "Analytics" },
  { value: "forensics", label: "Forensics & Recovery" },
  { value: "diagnostics", label: "Diagnostics" },
];

function routedSegment(): ObservatorySegment {
  const segment = currentRoute().segment as ObservatorySegment | undefined;
  return SEGMENTS.some((item) => item.value === segment) ? segment! : "now";
}

export default function ObservatorySpace() {
  const isMini = createMemo(() => shellStore.windowMode() === "mini");
  createEffect(() => observatoryStore.setActiveSegment(routedSegment()));
  onMount(() => {
    void observatoryStore.loadExecutiveSnapshot();
    void observatoryStore.loadAnalytics();
    void observatoryStore.loadForensics();
    void observatoryStore.refreshTestRunState();
  });
  onCleanup(observatoryStore.connectExecutiveEvents());
  onCleanup(observatoryStore.connectTelemetry());
  onCleanup(observatoryStore.connectJobs());

  function selectSegment(value: string) {
    if (value === "now") navigate("observatory");
    else navigate("observatory", value);
  }

  const items = SEGMENTS.map((segment) => ({
    value: segment.value,
    label: segment.label,
    content: () => <SegmentRegion segment={segment.value} />,
  }));

  return (
    <section class="kria-observatory" data-space="observatory" aria-label="Observatory">
      <header>
        <div class="kria-observatory__region-head">
          <h1>Observatory</h1>
          <div class="kria-observatory__region-actions">
            <IconButton icon="minimize-2" label="Open Now mini"
              onClick={() => void openCompanion("now-mini")} />
            <IconButton icon="monitor" label="Detach Observatory Now"
              onClick={() => void openDetachedSurface("observatory-now")} />
          </div>
        </div>
        <p>KRIA system state, history, and bounded diagnostics.</p>
      </header>
      <Show
        when={isMini()}
        fallback={<Tabs items={items} value={routedSegment()} onChange={selectSegment} />}
      >
        <div class="kria-observatory__compact" data-curated-primary="now-mini">
          <NowRegion />
        </div>
      </Show>
    </section>
  );
}
function SegmentRegion(props: { segment: ObservatorySegment }) {
  return <div class="kria-observatory__region" data-segment={props.segment}>
    <Show when={props.segment === "now"}><NowRegion /></Show>
    <Show when={props.segment === "jobs"}><JobsRegion /></Show>
    <Show when={props.segment === "analytics"}>
      <AnalyticsTiles tiles={observatoryStore.analytics()} authority={observatoryStore.analyticsAuthority()} />
    </Show>
    <Show when={props.segment === "forensics"}>
      <ForensicTimeline records={observatoryStore.forensics()} authority={observatoryStore.forensicsAuthority()} />
      <HraForensicsPanel snapshot={observatoryStore.hraDiagnostics()} authority={observatoryStore.hraAuthority()} />
    </Show>
    <Show when={props.segment === "diagnostics"}><DiagnosticsRegion /></Show>
  </div>;
}

export function NowRegion() {
  return <>
    <SystemPulse authority={observatoryStore.telemetryAuthority()}
      metrics={observatoryStore.resourceMetrics()} jobs={observatoryStore.jobs()} />
    <div class="kria-observatory__meters">
      <ResourceMeter title="CPU" metric="cpu_percent" unit="%"
        points={observatoryStore.telemetryBuffer()} authority={observatoryStore.telemetryAuthority()} />
      <ResourceMeter title="Memory" metric="memory_percent" unit="%"
        points={observatoryStore.telemetryBuffer()} authority={observatoryStore.telemetryAuthority()} />
    </div>
    <HraNowPanel snapshot={observatoryStore.hraDiagnostics()} authority={observatoryStore.hraAuthority()} />
    <section aria-labelledby="running-jobs-heading">
      <div class="kria-observatory__region-head"><h2 id="running-jobs-heading">Running jobs</h2></div>
      <Show when={observatoryStore.jobs().some((job) => ["queued", "running", "paused"].includes(job.status))}
        fallback={<EmptyState icon="clock" title="No running jobs" description="KRIA has no active cancellable work." />}>
        <ul class="kria-observatory__jobs"><For each={observatoryStore.jobs().filter((job) => ["queued", "running", "paused"].includes(job.status))}>
          {(job) => <JobRow job={job} onCancel={observatoryStore.cancelJob} />}
        </For></ul>
      </Show>
    </section>
  </>;
}
function JobsRegion() {
  return <>
    <ExecutiveController snapshot={observatoryStore.executiveSnapshot()}
      events={observatoryStore.executiveRecentEvents()}
      authority={observatoryStore.executiveAuthority()}
      onCancel={observatoryStore.cancelExecutiveTask} />
    <section aria-labelledby="other-jobs-heading">
      <div class="kria-observatory__region-head">
        <h2 id="other-jobs-heading">Other runtime jobs</h2>
      </div>
      <Show when={observatoryStore.jobs().length > 0}
        fallback={<EmptyState icon="clock" title="No additional jobs"
          description="Capability and test-runner jobs will appear when their runtimes report work." />}>
        <ul class="kria-observatory__jobs"><For each={observatoryStore.jobs()}>
          {(job) => <JobRow job={job} onCancel={observatoryStore.cancelJob} />}
        </For></ul>
      </Show>
    </section>
  </>;
}

function DiagnosticsRegion() {
  return <>
    <HraDiagnosticsPanel snapshot={observatoryStore.hraDiagnostics()} authority={observatoryStore.hraAuthority()} />
    <Show when={import.meta.env.DEV} fallback={
      <EmptyState icon="lock" title="Test console unavailable" description="Bounded test controls are exposed only in development builds." />
    }>
      <TestConsole authority={observatoryStore.testAuthority()} state={observatoryStore.testRunState()}
        onStart={observatoryStore.startTestRun} onStop={observatoryStore.stopTestRun}
        onRefresh={observatoryStore.refreshTestRunState} />
    </Show>
  </>;
}
