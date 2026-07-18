/**
 * ConverseEmptyState — the Core-forward cold/warm empty state (task 3.6, Req 4.6).
 *
 * When a thread has no messages the ConversationLane shows THIS instead of a
 * blank page. It is Core-forward (a prominent CorePresence) and adapts to
 * whether the user is new or returning (design.md §11.11.3):
 *
 *   • COLD (first ever — no prior conversation history): the Core forward and
 *     slightly larger, one quiet line ("What can I help with?"), and ≤3 example
 *     intents (ask / automate / remember) as clickable suggestions.
 *   • WARM (returning — prior threads exist): the Core at rest, one quiet line
 *     ("Continue where you left off"), and ≤3 quiet continue-suggestions drawn
 *     from the most-recent threads.
 *
 * Neither branch is ever blank — CorePresence + a heading always render.
 *
 * ── KRIA runtime-authority invariant ────────────────────────────────────────
 * Suggestions NEVER shortcut prompt→tool and never run anything. Selecting a
 * cold example intent STAGES it into the composer draft
 * (`converseStore.updateDraft`) so the user reviews before sending — it does not
 * call send or a tool. Selecting a warm continue-suggestion OPENS the thread
 * (`converseStore.setActiveThread`), pure navigation. No orchestration.
 *
 * ── Adaptive-ranking (task 13.1) ────────────────────────────────────────────
 * Cold intents and warm continue-suggestions are ranked only inside this
 * clearly designated empty-state zone. Usage is recorded only after explicit
 * selection. Ranking never sends, executes, or changes runtime authority.
 *
 * Motion: the ONLY animating element is the Core (Req 3.5 / 13.5); it honors
 * reduced-motion by rendering static. Nothing else here moves.
 *
 * Accessibility: a labelled region with a real heading, and every suggestion is
 * a labelled, focus-visible control (Req 17.1 / 17.2).
 *
 * Requirements: 4.6
 */
import { createMemo, For, Show } from "solid-js";
import { converseStore } from "../../../stores";
import { CorePresence } from "../../../components/CorePresence";
import { Card } from "../../../kit";
import { Icon } from "../../../components/Icon";
import {
  AdaptiveSuggestionControls,
  CoachHint,
  rankEmptyStateCandidates,
  recordAdaptiveUse,
  resetAdaptiveSuggestions,
  retireCoachHint,
} from "../../../adaptive";
import "./ConverseEmptyState.css";

/** A cold-start example intent — a starter prompt staged into the composer. */
export interface ExampleIntent {
  id: string;
  /** Lucide icon id shown on the suggestion. */
  icon: string;
  /** Short human label shown on the suggestion (the accessible name). */
  label: string;
  /** Draft text staged into the composer when selected (user reviews first). */
  draft: string;
}

/** A warm continue-suggestion — a pointer to an existing thread to reopen. */
export interface ContinueSuggestion {
  /** The thread id to reopen. */
  id: string;
  /** Human label (thread title). */
  label: string;
}

/**
 * The default curated cold example intents (≤3): ask / automate / remember
 * (design.md §11.11.3). Each stages a starter draft the user can edit before
 * sending — never an auto-run.
 */
export const COLD_EXAMPLE_INTENTS: readonly ExampleIntent[] = [
  {
    id: "ask",
    icon: "message-circle",
    label: "Ask a question",
    draft: "What can you help me with?",
  },
  {
    id: "automate",
    icon: "workflow",
    label: "Automate a task",
    draft: "Set up an automation to ",
  },
  {
    id: "remember",
    icon: "brain",
    label: "Remember something",
    draft: "Remember that ",
  },
];

const MAX_SUGGESTIONS = 3;
const EMPTY_SUGGESTIONS_COACH = "converse-empty-suggestions";

export interface ConverseEmptyStateProps {
  /**
   * Override the cold example intents (task 13.x adaptive ranking). Defaults to
   * the curated `COLD_EXAMPLE_INTENTS`. Capped at 3.
   */
  intents?: readonly ExampleIntent[];
  /**
   * Override the warm continue-suggestions (task 13.x adaptive ranking).
   * Defaults to the most-recent non-archived threads. Capped at 3.
   */
  suggestions?: readonly ContinueSuggestion[];
  /**
   * Handler when a cold example intent is selected. Defaults to staging the
   * draft (`converseStore.updateDraft`). Tests/stories only.
   */
  onSelectIntent?: (intent: ExampleIntent) => void;
  /**
   * Handler when a warm continue-suggestion is selected. Defaults to opening the
   * thread (`converseStore.setActiveThread`). Tests/stories only.
   */
  onContinue?: (suggestion: ContinueSuggestion) => void;
}

export function ConverseEmptyState(props: ConverseEmptyStateProps) {
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

  // Cold examples use the same designated zone with namespaced ids.
  const intents = createMemo<ExampleIntent[]>(() =>
    rankEmptyStateCandidates(
      (props.intents ?? COLD_EXAMPLE_INTENTS).map((intent) => ({
        id: `intent:${intent.id}`,
        intent,
      })),
    )
      .slice(0, MAX_SUGGESTIONS)
      .map(({ intent }) => intent),
  );

  // WARM reflects existing history, not merely currently-visible suggestions.
  // Dismissing every suggestion must not pretend the user is first-run again.
  const isWarm = createMemo(() =>
    props.suggestions
      ? props.suggestions.length > 0
      : converseStore.threads().some((thread) => !thread.archived),
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
      data-empty-mode={isWarm() ? "warm" : "cold"}
      aria-label="Start a conversation"
    >
      {/* Core-forward: prominent in cold, at rest in warm. CorePresence
          carries its own accessible label (Req 17.2) — not hidden. */}
      <div class="kria-converse-empty__core">
        <CorePresence size={isWarm() ? "lg" : 72} />
      </div>

      <CoachHint featureId={EMPTY_SUGGESTIONS_COACH}>
        Suggestions adapt from explicit use. Each one explains why; pin, dismiss, or reset anytime.
      </CoachHint>

      <Show
        when={isWarm()}
        fallback={
          <>
            <h2 class="kria-converse-empty__title">What can I help with?</h2>
            <ul class="kria-converse-empty__intents" aria-label="Example intents">
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
                      <AdaptiveSuggestionControls
                        zone="empty-state"
                        id={`intent:${intent.id}`}
                        label={intent.label}
                        onPreferenceChange={retireSuggestionCoach}
                      />
                    </Card>
                  </li>
                )}
              </For>
            </ul>
          </>
        }
      >
        <h2 class="kria-converse-empty__title">Continue where you left off</h2>
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
                  <AdaptiveSuggestionControls
                    zone="empty-state"
                    id={`thread:${suggestion.id}`}
                    label={suggestion.label}
                    onPreferenceChange={retireSuggestionCoach}
                  />
                </Card>
              </li>
            )}
          </For>
        </ul>
      </Show>

      <button
        type="button"
        class="kria-adaptive-reset kit-focusable"
        onClick={() => resetAdaptiveSuggestions("empty-state")}
      >
        Reset suggestions to defaults
      </button>
    </section>
  );
}

export default ConverseEmptyState;
