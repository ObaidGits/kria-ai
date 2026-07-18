/**
 * AskKriaToPick — describe an intent in natural language → KRIA suggests a
 * workflow to run (task 7.2, Req 6.3 "ask-KRIA-to-pick").
 *
 * The user types what they want; submitting dispatches the EXISTING
 * `suggest_n8n_workflows` command through `automationStore.pickWorkflow`. KRIA's
 * ranked candidates render as {@link SuggestionCard}s. From a suggestion the
 * user can either Prepare inputs — showing {@link PreparedInputPreview} for
 * review BEFORE running (Req 6.3) — or Run directly. Confirming a prepared run
 * dispatches the EXISTING run command. Honest loading / empty / failure states
 * throughout (Req 6.5); nothing runs without a deliberate action.
 *
 * ── KRIA runtime-authority invariant ────────────────────────────────────────
 * Pick / prepare / run are DISPATCH-ONLY via existing commands; there is no
 * prompt→tool shortcut and no orchestration in the UI — KRIA's runtime picks,
 * prepares, and runs.
 *
 * Requirements: 6.3, 6.5
 */
import { createSignal, For, Show } from "solid-js";
import { Button, Textarea } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { automationStore } from "../../../stores";
import type { SuggestedWorkflow } from "../../../stores";
import { SuggestionCard } from "./SuggestionCard";
import { PreparedInputPreview } from "./PreparedInputPreview";
import "./run.css";

export function AskKriaToPick() {
  const [prompt, setPrompt] = createSignal("");
  const [confirming, setConfirming] = createSignal(false);
  const [runError, setRunError] = createSignal<string | null>(null);
  // Which suggestion a prepared-input preview belongs to (for confirm-run).
  const [preparingFor, setPreparingFor] = createSignal<SuggestedWorkflow | null>(null);

  async function ask(e?: Event) {
    e?.preventDefault();
    setRunError(null);
    automationStore.clearPreparedInput();
    setPreparingFor(null);
    await automationStore.pickWorkflow(prompt());
  }

  async function prepare(s: SuggestedWorkflow) {
    setRunError(null);
    setPreparingFor(s);
    const res = await automationStore.prepareRun({
      workflowId: s.workflowId,
      workflowVersion: s.workflowVersion,
      prompt: automationStore.lastPickPrompt() || prompt(),
    });
    if (!res.ok) {
      setRunError(res.message);
      setPreparingFor(null);
    }
  }

  async function runSuggestion(s: SuggestedWorkflow) {
    setRunError(null);
    setConfirming(true);
    try {
      const res = await automationStore.startRun({
        workflowId: s.workflowId,
        workflowVersion: s.workflowVersion,
        inputPayload: s.suggestedInputPayload ?? {},
      });
      if (!res.ok) setRunError(res.message);
    } finally {
      setConfirming(false);
    }
  }

  async function confirmPrepared() {
    const prepared = automationStore.preparedInput();
    if (!prepared) return;
    setRunError(null);
    setConfirming(true);
    try {
      const res = await automationStore.startRun({
        workflowId: prepared.workflowId,
        workflowVersion: prepared.workflowVersion,
        inputPayload: prepared.payload,
        inputMapped: prepared.inputMapped,
      });
      if (!res.ok) {
        setRunError(res.message);
        return;
      }
      automationStore.clearPreparedInput();
      setPreparingFor(null);
    } finally {
      setConfirming(false);
    }
  }

  function cancelPrepared() {
    automationStore.clearPreparedInput();
    setPreparingFor(null);
  }

  const canAsk = () => prompt().trim().length > 0 && !automationStore.suggesting();

  return (
    <section class="kria-run__section" aria-label="Ask KRIA to pick a workflow">
      <h2 class="kria-run__section-title">Ask KRIA</h2>
      <div class="kria-ask">
        <form class="kria-ask__form" onSubmit={ask}>
          <Textarea
            label="Describe what you want to automate"
            placeholder="e.g. Summarize today's unread emails and save a briefing"
            rows={2}
            autoResize
            value={prompt()}
            onChange={setPrompt}
          />
          <div class="kria-ask__actions">
            <Button type="submit" variant="primary" size="sm" disabled={!canAsk()}>
              <Icon name={automationStore.suggesting() ? "loader" : "sparkles"} size={14} />
              {automationStore.suggesting() ? "Asking KRIA…" : "Ask KRIA"}
            </Button>
            <Show when={automationStore.suggestedWorkflows().length > 0 || automationStore.suggestError()}>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => {
                  automationStore.clearSuggestion();
                  cancelPrepared();
                  setRunError(null);
                }}
              >
                Clear
              </Button>
            </Show>
          </div>
        </form>

        {/* Honest loading (Req 6.5). */}
        <Show when={automationStore.suggesting()}>
          <p class="kria-ask__status" role="status" aria-live="polite">
            KRIA is finding a workflow…
          </p>
        </Show>

        {/* Honest failure. */}
        <Show when={automationStore.suggestError()}>
          <p class="kria-ask__error" role="alert">
            <Icon name="alert-triangle" size={13} aria-hidden /> {automationStore.suggestError()}
          </p>
        </Show>

        <Show when={runError()}>
          <p class="kria-ask__error" role="alert">
            <Icon name="alert-triangle" size={13} aria-hidden /> {runError()}
          </p>
        </Show>

        {/* KRIA's message about the suggestion (e.g. ambiguity note). */}
        <Show when={!automationStore.suggesting() && automationStore.suggestionMessage()}>
          <p class="kria-ask__message">{automationStore.suggestionMessage()}</p>
        </Show>

        {/* Prepared-input review — the confirm gate BEFORE running (Req 6.3). */}
        <Show when={automationStore.preparedInput()}>
          <PreparedInputPreview
            prepared={automationStore.preparedInput()!}
            confirming={confirming()}
            onConfirm={() => void confirmPrepared()}
            onCancel={cancelPrepared}
          />
        </Show>

        {/* Suggestions. Empty only shown after an ask that returned nothing. */}
        <Show
          when={automationStore.suggestedWorkflows().length > 0}
          fallback={
            <Show
              when={
                !automationStore.suggesting() &&
                !automationStore.suggestError() &&
                automationStore.lastPickPrompt() &&
                !automationStore.suggestionMessage()
              }
            >
              <p class="kria-run__muted">
                KRIA didn't find a matching workflow. Try rephrasing, or build one in the Build
                segment.
              </p>
            </Show>
          }
        >
          <ul class="kria-run__list" aria-label="Suggested workflows">
            <For each={automationStore.suggestedWorkflows()}>
              {(s) => (
                <li>
                  <SuggestionCard
                    suggestion={s}
                    busy={confirming() || preparingFor()?.workflowId === s.workflowId}
                    onPrepare={(sug) => void prepare(sug)}
                    onRun={(sug) => void runSuggestion(sug)}
                  />
                </li>
              )}
            </For>
          </ul>
        </Show>
      </div>
    </section>
  );
}

export default AskKriaToPick;
