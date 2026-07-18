/**
 * Cognition controls + result panel (task 6.3, Req 5.6).
 *
 * Fills the Memory Space "Cognition" segment placeholder built in task 6.1.
 * Provides a labelled control per cognition job (reflect / dream / consolidate /
 * active-learning / self-improvement / entity-extraction) and a PERSISTENT
 * result panel that shows WHAT CHANGED when a job completes — never a transient
 * toast (Req 5.6).
 *
 * ── Architecture (KRIA runtime authority) ───────────────────────────────────
 * Cognition is potentially heavy and is run by the RUNTIME. Each control simply
 * triggers the EXISTING `memory_*` command through the store's `runCognition`
 * action (which routes via the bridge) and reflects the outcome. There is no
 * orchestration, no prompt→tool shortcut, and no self-spawning loop here — the
 * UI triggers a bounded job and renders its result. The running state is
 * reflected through the Core (reflecting / remembering / learning) because
 * `runCognition` stages `memory:cognition-*` events that coreStore ingests
 * (task 2.1). The narrative shown is UI-generated from numeric counts and any
 * backend failure text is rendered as TEXT (Solid auto-escapes) — no untrusted
 * HTML path exists (sanitization invariant).
 *
 * Accessibility: the controls are labelled buttons; the result panel is a
 * region with headings; a polite live region announces run/complete; every
 * status uses icon + text (never color alone, Req 17.3).
 *
 * Requirements: 5.6
 */
import { createMemo, createSignal, For, Show } from "solid-js";
import {
  converseStore,
  memoryStore,
  COGNITION_LABEL,
  type CognitionJob,
  type CognitionResult,
} from "../../../stores";
import { Badge, Button, Card, EmptyState, Progress, ProvenanceCue } from "../../../kit";
import { Icon } from "../../../components/Icon";
import "./CognitionPanel.css";

interface JobDef {
  job: CognitionJob;
  icon: string;
  description: string;
}

/** The six cognition jobs (Req 5.6), each mapped to an existing command. */
const JOBS: readonly JobDef[] = [
  { job: "reflect", icon: "sparkles", description: "Review recent memories to form higher-level insights." },
  { job: "dream", icon: "star", description: "Distil procedures, merge goals, and recalibrate worth (idle work)." },
  { job: "consolidate", icon: "layers", description: "Merge the current conversation's memories into long-term memory." },
  { job: "active-learning", icon: "lightbulb", description: "Probe detected knowledge gaps with new questions." },
  { job: "self-improvement", icon: "zap", description: "Propose improvements to how KRIA works." },
  { job: "entity-extraction", icon: "network", description: "Extract and link entities across stored memories." },
] as const;

export function CognitionPanel() {
  // A polite status message for screen readers — reflects the latest trigger /
  // completion. Never a substitute for the persistent result panel below.
  const [status, setStatus] = createSignal("");

  // consolidate needs a session id (the active conversation). Without one the
  // control is honestly disabled rather than silently doing nothing.
  const activeSession = createMemo(() => converseStore.activeThreadId());

  async function trigger(def: JobDef) {
    const label = COGNITION_LABEL[def.job];
    setStatus(`${label} running…`);
    const args =
      def.job === "consolidate" ? { sessionId: activeSession() ?? "" } : undefined;
    const res = await memoryStore.runCognition(def.job, args);
    setStatus(
      res.ok
        ? `${label} finished — see results below.`
        : `${label} failed: ${res.message}`,
    );
  }

  return (
    <div class="kria-cognition">
      <h2 class="kria-memory__region-title">Cognition</h2>

      {/* Live status for assistive tech (Req 17.2). The durable record is the
          result panel — this is only an announcement. */}
      <div class="kria-cognition__status" role="status" aria-live="polite">
        {status()}
      </div>

      <section class="kria-cognition__controls" aria-label="Cognition controls">
        <ul class="kria-cognition__jobs">
          <For each={JOBS}>
            {(def) => {
              const label = COGNITION_LABEL[def.job];
              const running = createMemo(() => memoryStore.cognitionRunning().includes(def.job));
              const needsSession = createMemo(() => def.job === "consolidate" && !activeSession());
              return (
                <li class="kria-cognition__job">
                  <Card class="kria-cognition__job-card">
                    <div class="kria-cognition__job-head">
                      <Icon name={def.icon} size={16} aria-hidden />
                      <h3 class="kria-cognition__job-title">{label}</h3>
                    </div>
                    <p class="kria-cognition__job-desc">{def.description}</p>

                    <Show when={running()}>
                      <Progress
                        indeterminate
                        label={`${label} running`}
                        tone="accent"
                        class="kria-cognition__job-progress"
                      />
                    </Show>

                    <Show when={needsSession()}>
                      <p class="kria-cognition__job-note">
                        <Icon name="info" size={13} aria-hidden /> Open a
                        conversation to consolidate its session.
                      </p>
                    </Show>

                    <Button
                      variant="secondary"
                      size="sm"
                      class="kria-cognition__job-run"
                      disabled={running() || needsSession()}
                      aria-label={`Run ${label}`}
                      onClick={() => void trigger(def)}
                    >
                      <Icon name={running() ? "loader" : "play"} size={14} />
                      {running() ? "Running…" : "Run"}
                    </Button>
                  </Card>
                </li>
              );
            }}
          </For>
        </ul>
      </section>

      <section class="kria-cognition__results" aria-label="Cognition results">
        <div class="kria-cognition__results-head">
          <h3 class="kria-cognition__results-title">Results</h3>
          <Show when={memoryStore.cognitionResults().length > 0}>
            <Button
              variant="ghost"
              size="sm"
              aria-label="Clear cognition results"
              onClick={() => memoryStore.clearCognitionResults()}
            >
              <Icon name="x" size={14} /> Clear
            </Button>
          </Show>
        </div>

        <Show
          when={memoryStore.cognitionResults().length > 0}
          fallback={
            <EmptyState
              icon="sparkles"
              title="No cognition results yet"
              description="Trigger a job above. When it completes, what changed will appear here — and stay here."
            />
          }
        >
          <ul class="kria-cognition__result-list">
            <For each={memoryStore.cognitionResults()}>
              {(result) => <ResultCard result={result} />}
            </For>
          </ul>
        </Show>
      </section>
    </div>
  );
}

/** One persistent cognition result — the "what changed" record (Req 5.6). */
function ResultCard(props: { result: CognitionResult }) {
  const r = () => props.result;
  const label = () => COGNITION_LABEL[r().job];
  const when = () => new Date(r().at).toLocaleTimeString();
  return (
    <li
      class="kria-cognition__result"
      data-job={r().job}
      data-ok={r().ok}
      data-provenance="kria"
    >
      <Card class="kria-cognition__result-card">
        <ProvenanceCue source="kria" label="Generated by KRIA" />
        <div class="kria-cognition__result-head">
          <Icon name={r().ok ? "check-circle" : "alert-circle"} size={16} aria-hidden />
          <span class="kria-cognition__result-name">{label()}</span>
          <Badge tone={r().ok ? "success" : "danger"}>{r().ok ? "Updated" : "Failed"}</Badge>
          <time class="kria-cognition__result-when">{when()}</time>
        </div>

        <Show
          when={r().ok}
          fallback={
            <p class="kria-cognition__result-error">
              <Icon name="alert-triangle" size={13} aria-hidden /> {r().message}
            </p>
          }
        >
          {/* Plain-language narrative — UI-generated from counts, rendered as
              text (auto-escaped). */}
          <p class="kria-cognition__result-summary">{r().summary}</p>
          <Show
            when={r().changes.length > 0}
            fallback={<p class="kria-cognition__result-none">No changes were needed.</p>}
          >
            <ul class="kria-cognition__changes" aria-label="What changed">
              <For each={r().changes}>
                {(change) => (
                  <li class="kria-cognition__change">
                    <span class="kria-cognition__change-label">{change.label}</span>
                    <Badge tone={change.value > 0 ? "accent" : "neutral"}>{change.value}</Badge>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </Show>
      </Card>
    </li>
  );
}

export default CognitionPanel;
