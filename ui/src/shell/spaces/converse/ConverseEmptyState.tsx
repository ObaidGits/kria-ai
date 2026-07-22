/**
 * ConverseEmptyState — the Core-forward empty state driven by the single
 * deterministic empty-state classifier (task 6.4; Req 6.1–6.6, UIE-H-004,
 * UIE-H-005, UIE-L-002).
 *
 * When a thread has no messages the ConversationLane shows THIS instead of a
 * blank page. It is Core-forward (a prominent CorePresence) and adapts to the
 * classifier's state (`converseStore.emptyStateClass()`, design.md §11.6):
 *
 *   • cold-start ............. Core forward + slightly larger, one concise
 *                              orientation line ("What can I help with?"), and
 *                              ≤3 grounded existing-capability starters.
 *   • intentional-new-thread . the NEW-TASK state — a new-task prompt + the same
 *                              grounded starters, shown REGARDLESS of unrelated
 *                              history (UIE-H-005). Explicit intent outranks
 *                              unrelated global history.
 *   • continuation ........... Core at rest, one concise line ("Continue where
 *                              you left off"), and ≤3 relevant resumptions drawn
 *                              from the most-recent non-archived threads.
 *   • active ................. never reaches here (gated out by the hasMessages
 *                              MessageStream path); classified as continuation-
 *                              like fallback for safety only.
 *
 * Neither branch is ever blank — CorePresence + a heading always render (Req 4.6).
 *
 * ── Grounded starters (UIE-L-002, Req 6.6) ───────────────────────────────────
 * Cold Start / Intentional New Thread starters come from `groundedStarters()`,
 * which reads `capabilityStore` READ-ONLY to surface only enabled/available
 * capabilities (an image starter only when generation is available, an automate
 * starter only when tools exist, etc.) and falls back to safe generic-but-
 * truthful base starters otherwise. Reading capability state triggers NO loads,
 * tools, or side effects.
 *
 * ── KRIA runtime-authority invariant ─────────────────────────────────────────
 * Starters NEVER shortcut prompt→tool and never run anything. Selecting a
 * starter STAGES its draft (`converseStore.updateDraft`) so the user reviews
 * before sending — it does not send or invoke a tool (Req 6.5; hardened 6.6).
 * Selecting a continuation OPENS the thread (`converseStore.setActiveThread`),
 * pure navigation. No orchestration.
 *
 * ── Adaptive-ranking (task 13.1) ─────────────────────────────────────────────
 * Starters and continuations are ranked only inside this designated empty-state
 * zone. Usage is recorded only after explicit selection. Ranking never sends,
 * executes, or changes runtime authority.
 *
 * ── Secondary disclosure (UIE-H-004, task 6.5, Req 6.4) ──────────────────────
 * Coach text, per-suggestion adaptive controls (pin/dismiss/explain), and the
 * "Reset suggestions to defaults" control are DEFERRED behind a labelled
 * secondary disclosure ("Customize suggestions", a kit Popover). The primary
 * starters/continuations and the Composer stay the focal, uncluttered content
 * and the ONLY primary tab stops until the user reveals the controls. Nothing
 * is deleted — every handler (recordAdaptiveUse, retireCoachHint,
 * resetAdaptiveSuggestions, AdaptiveSuggestionControls) stays wired and
 * reachable through the disclosure, which is keyboard-accessible and focus-
 * managed (Kobalte Popover: focus into panel on open, Escape/outside dismiss,
 * focus restored on close).
 *
 * Motion: the ONLY animating element is the Core (Req 3.5 / 13.5); it honors
 * reduced-motion by rendering static. Nothing else here moves.
 *
 * Accessibility: a labelled region with a real heading, and every control is a
 * labelled, focus-visible control (Req 17.1 / 17.2).
 *
 * Requirements: 6.1, 6.2, 6.4, 6.6, 5.4, 4.6
 */
import { createMemo, For, Show } from "solid-js";
import { converseStore } from "../../../stores";
import { CorePresence } from "../../../components/CorePresence";
import { Card, Popover } from "../../../kit";
import { Icon } from "../../../components/Icon";
import {
  AdaptiveSuggestionControls,
  CoachHint,
  rankEmptyStateCandidates,
  recordAdaptiveUse,
  resetAdaptiveSuggestions,
  retireCoachHint,
} from "../../../adaptive";
import type { EmptyStateClass } from "./emptyStateClassifier";
import { groundedStarters, MAX_STARTERS, type ExampleIntent } from "./groundedStarters";
import { capabilityDisclosures, openCapabilityDisclosure } from "./capabilityDisclosure";
import { BOUNDED, boundedTitle } from "../../boundedText";
import "./ConverseEmptyState.css";

export type { ExampleIntent } from "./groundedStarters";

/** A continuation choice — a pointer to an existing thread to reopen. */
export interface ContinueSuggestion {
  /** The thread id to reopen. */
  id: string;
  /** Human label (thread title). */
  label: string;
}

/**
 * The safe generic-but-truthful base starters, re-exported for stories/tests
 * and backward-compatible references. Cold/New states use `groundedStarters()`
 * by default; this is the fallback subset shown when no capability is available.
 */
export { BASE_STARTERS as COLD_EXAMPLE_INTENTS } from "./groundedStarters";

const MAX_SUGGESTIONS = MAX_STARTERS;
const EMPTY_SUGGESTIONS_COACH = "converse-empty-suggestions";

export interface ConverseEmptyStateProps {
  /**
   * Override the cold/new starters (task 13.x adaptive ranking, stories/tests).
   * Defaults to `groundedStarters()`. Capped at 3.
   */
  intents?: readonly ExampleIntent[];
  /**
   * Override the continuation choices (task 13.x adaptive ranking, stories/tests).
   * Defaults to the most-recent non-archived threads. Capped at 3.
   */
  suggestions?: readonly ContinueSuggestion[];
  /**
   * Handler when a starter is selected. Defaults to staging the draft
   * (`converseStore.updateDraft`). Tests/stories only.
   */
  onSelectIntent?: (intent: ExampleIntent) => void;
  /**
   * Handler when a continuation is selected. Defaults to opening the thread
   * (`converseStore.setActiveThread`). Tests/stories only.
   */
  onContinue?: (suggestion: ContinueSuggestion) => void;
}

/**
 * Which presentation branch to render. Derived from the single deterministic
 * classifier (task 6.2) so an Intentional New Thread renders the new-task state
 * even when unrelated history exists (UIE-H-005). Explicit prop overrides
 * (tests/stories) win over the live classifier.
 */
function resolveStateClass(props: ConverseEmptyStateProps): EmptyStateClass {
  if (props.suggestions) return props.suggestions.length > 0 ? "continuation" : "cold-start";
  if (props.intents) return "cold-start";
  return converseStore.emptyStateClass();
}

export function ConverseEmptyState(props: ConverseEmptyStateProps) {
  const stateClass = createMemo<EmptyStateClass>(() => resolveStateClass(props));
  // "active" never renders here (hasMessages gate), but treat it as continuation
  // for a safe non-blank fallback rather than a starter state.
  const isContinuation = createMemo(
    () => stateClass() === "continuation" || stateClass() === "active",
  );
  const isNewThread = createMemo(() => stateClass() === "intentional-new-thread");

  // Rank every eligible thread before capping the visible zone. The adaptive
  // module permits only bounded presentation movement and never removes data.
  const suggestions = createMemo<ContinueSuggestion[]>(() => {
    const source = props.suggestions
      ? props.suggestions.slice()
      : converseStore
          .threads()
          .filter((t) => !t.archived)
          .slice()
          .sort((a, b) => b.updatedAt - a.updatedAt)
          .map((t) => ({ id: t.id, label: t.title }));

    return rankEmptyStateCandidates(
      source.map((suggestion) => ({
        id: `thread:${suggestion.id}`,
        suggestion,
      })),
    )
      .slice(0, MAX_SUGGESTIONS)
      .map(({ suggestion }) => suggestion);
  });

  // Cold / New-thread starters — grounded in enabled capabilities by default,
  // ranked inside the same designated zone with namespaced ids, capped at 3.
  const intents = createMemo<ExampleIntent[]>(() =>
    rankEmptyStateCandidates(
      (props.intents ?? groundedStarters()).map((intent) => ({
        id: `intent:${intent.id}`,
        intent,
      })),
    )
      .slice(0, MAX_SUGGESTIONS)
      .map(({ intent }) => intent),
  );

  const dataMode = createMemo(() =>
    isContinuation() ? "continuation" : isNewThread() ? "new" : "cold",
  );

  // Read-only capability disclosure (task 10.6, UIE-M-019): a concise,
  // informational "what KRIA can do" cue for the F6 (tools/MCP) and F7 (OpenClaw
  // skills) facts, GROUNDED in global enabled/available state via the shared
  // capabilityFieldMap omission rules (offline OpenClaw → truthful "unavailable";
  // not-loaded registry → omitted; M5: global only, no per-turn set). Shown only
  // in the starter (cold / new-thread) states, never in continuation. Purely
  // informational — activating a cue only deep-links (navigate/openInspector) to
  // the capability's existing home; it never invokes, launches, approves, sends,
  // or bypasses staged review.
  const disclosures = createMemo(() => (isContinuation() ? [] : capabilityDisclosures()));

  // The visible zone's items, namespaced for the adaptive module. Per-suggestion
  // pin/dismiss/explain controls live behind the secondary "Customize
  // suggestions" disclosure (UIE-H-004, Req 6.4) so the starters/continuations
  // and Composer stay the focal, uncluttered content until intent forms. The
  // controls stay fully wired — nothing here is deleted, only deferred.
  const controlItems = createMemo<{ id: string; label: string }[]>(() =>
    isContinuation()
      ? suggestions().map((s) => ({ id: `thread:${s.id}`, label: s.label }))
      : intents().map((i) => ({ id: `intent:${i.id}`, label: i.label })),
  );

  const heading = createMemo(() =>
    isContinuation()
      ? "Continue where you left off"
      : isNewThread()
        ? "Start a new task"
        : "What can I help with?",
  );

  const retireSuggestionCoach = (): void => retireCoachHint(EMPTY_SUGGESTIONS_COACH);

  const selectIntent = (intent: ExampleIntent): void => {
    // Usage changes later presentation only. Action remains user-triggered and
    // stages a reviewable draft — never prompt→tool or auto-send.
    recordAdaptiveUse("empty-state", `intent:${intent.id}`);
    retireSuggestionCoach();
    (props.onSelectIntent ?? ((i: ExampleIntent) => converseStore.updateDraft({ text: i.draft })))(
      intent,
    );
  };

  const continueThread = (suggestion: ContinueSuggestion): void => {
    // Record only after explicit selection, then perform pure navigation.
    recordAdaptiveUse("empty-state", `thread:${suggestion.id}`);
    retireSuggestionCoach();
    (props.onContinue ?? ((s: ContinueSuggestion) => converseStore.setActiveThread(s.id)))(
      suggestion,
    );
  };

  return (
    <section
      class="kria-converse-empty"
      data-empty-mode={dataMode()}
      aria-label="Start a conversation"
    >
      {/* Core-forward: prominent in cold/new, at rest in continuation.
          CorePresence carries its own accessible label (Req 17.2). */}
      <div class="kria-converse-empty__core">
        <CorePresence size={isContinuation() ? "lg" : 72} />
      </div>

      <Show
        when={isContinuation()}
        fallback={
          <>
            <h2 class="kria-converse-empty__title">{heading()}</h2>
            <ul class="kria-converse-empty__intents" aria-label="Starter prompts">
              <For each={intents()}>
                {(intent) => (
                  <li>
                    <Card class="kria-converse-empty__intent">
                      <button
                        type="button"
                        class="kria-converse-empty__target kit-focusable"
                        aria-label={intent.label}
                        onClick={() => selectIntent(intent)}
                      >
                        <Icon name={intent.icon} size={16} aria-hidden />
                        <span>{intent.label}</span>
                      </button>
                    </Card>
                  </li>
                )}
              </For>
            </ul>

            {/* Read-only capability disclosure (task 10.6, UIE-M-019): grounded
                in global enabled/available state. A "show" cue is a read-only
                deep-link to the capability's existing home; an "unavailable" cue
                (offline OpenClaw runtime) is a static, non-actionable label shown
                truthfully — never presented as ready. Omitted facts render
                nothing. No cue invokes/launches/approves/sends/bypasses. */}
            <Show when={disclosures().length > 0}>
              <ul
                class="kria-converse-empty__capabilities"
                aria-label="Available capabilities"
              >
                <For each={disclosures()}>
                  {(disclosure) => (
                    <li>
                      <Show
                        when={disclosure.outcome === "show"}
                        fallback={
                          <span
                            class={`kria-converse-empty__capability kria-converse-empty__capability--unavailable ${BOUNDED}`}
                            data-fact={disclosure.factId}
                            data-outcome={disclosure.outcome}
                            title={boundedTitle(`${disclosure.label} unavailable`)}
                          >
                            {disclosure.label} unavailable
                          </span>
                        }
                      >
                        <button
                          type="button"
                          class={`kria-converse-empty__capability kit-focusable ${BOUNDED}`}
                          data-fact={disclosure.factId}
                          data-outcome={disclosure.outcome}
                          title={boundedTitle(disclosure.label)}
                          aria-label={
                            disclosure.link
                              ? `${disclosure.label}: ${disclosure.link.destinationLabel}`
                              : disclosure.label
                          }
                          onClick={() => openCapabilityDisclosure(disclosure.factId)}
                        >
                          {disclosure.label}
                        </button>
                      </Show>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </>
        }
      >
        <h2 class="kria-converse-empty__title">{heading()}</h2>
        <ul class="kria-converse-empty__suggestions" aria-label="Continue suggestions">
          <For each={suggestions()}>
            {(suggestion) => (
              <li>
                <Card class="kria-converse-empty__suggestion">
                  <button
                    type="button"
                    class="kria-converse-empty__target kit-focusable"
                    aria-label={`Continue: ${suggestion.label}`}
                    onClick={() => continueThread(suggestion)}
                  >
                    <Icon name="message-circle" size={16} aria-hidden />
                    <span class="kria-converse-empty__suggestion-label">{suggestion.label}</span>
                    <Icon
                      name="arrow-right"
                      size={16}
                      class="kria-converse-empty__suggestion-go"
                      aria-hidden
                    />
                  </button>
                </Card>
              </li>
            )}
          </For>
        </ul>
      </Show>

      {/* Secondary disclosure (UIE-H-004): coach text, per-suggestion adaptive
          controls, and reset are deferred here so they don't compete with the
          primary starters/Composer or add tab stops before intent. Kobalte-
          backed Popover → labelled trigger, focus moved into panel on open,
          Escape/outside-click dismiss, focus restored on close. All existing
          behavior stays reachable and wired. */}
      <div class="kria-converse-empty__customize">
        <Popover triggerLabel="Customize suggestions" title="Suggestion settings">
          <div class="kria-converse-empty__customize-panel">
            <CoachHint featureId={EMPTY_SUGGESTIONS_COACH}>
              Suggestions adapt from explicit use. Each one explains why; pin,
              dismiss, or reset anytime.
            </CoachHint>

            <ul class="kria-converse-empty__customize-list" aria-label="Adjust suggestions">
              <For each={controlItems()}>
                {(item) => (
                  <li class="kria-converse-empty__customize-row">
                    <span class="kria-converse-empty__customize-label">{item.label}</span>
                    <AdaptiveSuggestionControls
                      zone="empty-state"
                      id={item.id}
                      label={item.label}
                      onPreferenceChange={retireSuggestionCoach}
                    />
                  </li>
                )}
              </For>
            </ul>

            <button
              type="button"
              class="kria-adaptive-reset kit-focusable"
              onClick={() => resetAdaptiveSuggestions("empty-state")}
            >
              Reset suggestions to defaults
            </button>
          </div>
        </Popover>
      </div>
    </section>
  );
}

export default ConverseEmptyState;
