/** Four persistent center cards for KRIA's living companion homepage. */
import { For, Show, type JSX } from "solid-js";
import { CcIcon } from "./CcIcon";
import {
  coreState,
  currentCognition,
  currentContext,
  currentOperations,
  cycleContext,
  setActiveContext,
  setCoreState,
  type CoreState,
  type OpStatus,
} from "./context";
import { openCapability } from "./homeNav";

interface CardProps {
  onIntent: (value: string) => void;
}

const STATE_PROGRESS: Record<CoreState, number> = {
  idle: 0,
  listening: 24,
  thinking: 64,
  retrieving: 78,
  executing: 88,
};

const STATE_PHASE: Record<CoreState, string> = {
  idle: "Ready for direction",
  listening: "Capturing intent",
  thinking: "Step 4 of 6",
  retrieving: "Connecting evidence",
  executing: "Verifying output",
};

const STATE_ETA: Record<CoreState, string> = {
  idle: "Ready now",
  listening: "Listening",
  thinking: "~18 sec",
  retrieving: "~8 sec",
  executing: "~12 sec",
};

const STATUS_LABEL: Record<OpStatus, string> = {
  running: "Running",
  waiting: "Queued",
  done: "Complete",
  attention: "Review",
};

function CardHeader(props: { icon: string; eyebrow: string; title: string; trailing?: JSX.Element }) {
  return (
    <header class="cc-home-card__head">
      <span class="cc-home-card__icon"><CcIcon name={props.icon} size={15} /></span>
      <span class="cc-home-card__heading"><small>{props.eyebrow}</small><strong>{props.title}</strong></span>
      {props.trailing}
    </header>
  );
}

export function NowCard(props: CardProps) {
  let traceDetails: HTMLDetailsElement | undefined;
  const active = () => coreState() !== "idle";
  const progress = () => STATE_PROGRESS[coreState()];
  const contextCoverage = () => currentCognition().confidence >= 88 ? "High context" : "Context ready";
  const stop = () => setCoreState("idle");
  const inspect = () => { if (traceDetails) traceDetails.open = !traceDetails.open; };

  return (
    <article class="cc-home-card cc-now-card" data-domain={coreState()} data-active={active() ? "true" : "false"}>
      <CardHeader
        icon="activity"
        eyebrow="Now"
        title={active() ? currentCognition().stateLabel : "Context ready"}
        trailing={<span class="cc-now-card__eta"><i />{active() ? STATE_ETA[coreState()] : "Awaiting direction"}</span>}
      />
      <div class="cc-now-card__main" aria-live="polite">
        <div>
          <strong>{currentCognition().activity}</strong>
          <p>{currentCognition().detail}</p>
        </div>
        <span class="cc-now-card__confidence"><b>{contextCoverage()}</b><small>{currentContext().label}</small></span>
      </div>
      <Show when={active()}>
        <div class="cc-now-card__progress">
          <span><b>{STATE_PHASE[coreState()]}</b><small>{progress()}%</small></span>
          <div
            class="cc-progress-track"
            role="progressbar"
            aria-label="Current operation progress"
            aria-valuemin="0"
            aria-valuemax="100"
            aria-valuenow={progress()}
          ><i style={{ width: `${progress()}%` }} /></div>
        </div>
      </Show>
      <details ref={traceDetails} class="cc-now-card__trace">
        <summary>{active() ? "Why KRIA is doing this" : "Why KRIA is waiting"}</summary>
        <p><b>Objective:</b> {currentCognition().goal}</p>
        <p><b>Evidence:</b> {currentCognition().evidence}</p>
      </details>
      <div class="cc-card-actions">
        <Show when={active()} fallback={
          <>
            <button type="button" onClick={inspect}>Inspect context</button>
            <button type="button" class="is-primary" onClick={() => props.onIntent(currentCognition().nextAction)}>Start next</button>
          </>
        }>
          <button type="button" onClick={stop}>Pause</button>
          <button type="button" onClick={stop}>Stop</button>
          <button type="button" class="is-primary" onClick={inspect}>Inspect</button>
        </Show>
      </div>
    </article>
  );
}

export function ActiveContextCard(props: CardProps) {
  const clearContext = () => setActiveContext("general");

  return (
    <article class="cc-home-card cc-context-card" data-domain="reasoning">
      <CardHeader
        icon="layers"
        eyebrow="Working memory"
        title="Active Context"
        trailing={<span class="cc-card-status"><i />Updated now</span>}
      />
      <div class="cc-context-card__primary">
        <div><small>Where</small><strong>KRIA · Command Center</strong></div>
        <div><small>Current goal</small><strong>{currentContext().objective}</strong></div>
        <div><small>Sources</small><strong>Project · Conversation · Memory</strong></div>
      </div>
      <div class="cc-context-card__facts">
        <span><CcIcon name="code" size={14} /><b>CommandCenter.tsx</b></span>
        <span><CcIcon name="monitor" size={14} /><b>Kiro · Linux</b></span>
        <span><CcIcon name="brain" size={14} /><b>{currentContext().label} context</b></span>
      </div>
      <div class="cc-card-actions cc-card-actions--compact">
        <button type="button" onClick={cycleContext}>Change</button>
        <button type="button" class="is-primary" onClick={(event) => openCapability("memory", event.currentTarget)}>Inspect</button>
        <details class="cc-context-more">
          <summary aria-label="More context actions">More</summary>
          <div>
            <button type="button" onClick={clearContext}>Clear context</button>
            <button type="button" onClick={() => props.onIntent("Visualize my active working context")}>Visualize context</button>
          </div>
        </details>
      </div>
    </article>
  );
}

export function WorkstreamCard(props: CardProps) {
  const activeCount = () => currentOperations().filter((operation) => operation.status === "running").length;

  return (
    <article class="cc-home-card cc-workstream-card" data-domain="execution">
      <CardHeader
        icon="flow"
        eyebrow="Execution"
        title="Workstream"
        trailing={<span class="cc-card-status"><i />{activeCount()} active</span>}
      />
      <div class="cc-flow-track" aria-label="Current execution flow">
        <For each={currentOperations()}>
          {(operation, index) => (
            <button
              type="button"
              class="cc-flow-node"
              data-status={operation.status}
              aria-label={`${operation.label}: ${STATUS_LABEL[operation.status]}`}
              onClick={() => props.onIntent(operation.label)}
            >
              <span class="cc-flow-node__icon"><CcIcon name={operation.icon} size={14} /></span>
              <span><b>{operation.label}</b><small>{STATUS_LABEL[operation.status]}</small></span>
              {index() < currentOperations().length - 1 && <i class="cc-flow-node__link" aria-hidden="true" />}
            </button>
          )}
        </For>
      </div>
      <div class="cc-workstream-card__result">
        <span class="cc-result-mark"><CcIcon name="check" size={13} /></span>
        <span><b>Latest result</b><small>Homepage architecture review completed 4 min ago</small></span>
        <button type="button" onClick={() => props.onIntent("Review the latest execution result")}>View</button>
      </div>
    </article>
  );
}

const READINESS = [
  { icon: "brain", label: "Local model", value: "Connected" },
  { icon: "memory", label: "Memory", value: "Healthy" },
  { icon: "mic", label: "Voice", value: "Ready" },
] as const;

export function SystemReadinessCard() {
  return (
    <article class="cc-home-card cc-readiness-card" data-domain="health">
      <CardHeader
        icon="shield"
        eyebrow="Local runtime"
        title="System Readiness"
        trailing={<span class="cc-card-status is-ready"><i />Everything ready</span>}
      />
      <div class="cc-readiness-card__summary">
        <span class="cc-readiness-mark"><CcIcon name="check" size={17} /></span>
        <span><strong>Everything is ready</strong><p>Models, memory, voice and local tools are available.</p></span>
        <span class="cc-local-badge"><CcIcon name="shield" size={12} />100% local</span>
      </div>
      <div class="cc-readiness-grid">
        <For each={READINESS}>
          {(item) => <span><CcIcon name={item.icon} size={14} /><b>{item.label}</b><small>{item.value}</small></span>}
        </For>
      </div>
      <details class="cc-technical-details">
        <summary>Technical details</summary>
        <div><span>Model <b>Local</b></span><span>Privacy <b>On-device</b></span><span>Queue <b>Idle</b></span><span>Skills + MCP <b>Available</b></span></div>
      </details>
    </article>
  );
}