/**
 * Overview — Memory Control Center Overview destination.
 *
 * Renders a truthful summary built from authority-backed data:
 * recent changes, contradictions, active goals, pending cognition count,
 * and actions. When no data exists yet (isEmpty === true) a goal-led
 * manual onboarding section is shown instead, along with a source-consent
 * request button.
 *
 * Invariants (F4.2):
 * - Health is never inferred from missing data.
 * - Each section is hidden when its array is empty (not shown as "0 items").
 * - pendingCognitionCount IS shown as a count ("N tasks pending") when > 0
 *   because it represents a non-empty quantity, not an inferred health state.
 * - No editorial copy (brain/mind/sentience/emotion) is ever rendered.
 * - This is a pure display component — no mutations, no policy enforcement.
 *
 * Requirements: F4.2 (task 4.2.2) — Overview destination.
 */
import { For, Show } from "solid-js";

// ─── Supporting interfaces ────────────────────────────────────────────────────

export interface OverviewChange {
  id: string;
  kind: string;
  label: string;
  timestamp: string;
}

export interface OverviewContradiction {
  id: string;
  description: string;
}

export interface OverviewGoal {
  id: string;
  title: string;
  status: "active" | "paused";
}

// ─── Props ───────────────────────────────────────────────────────────────────

export interface OverviewProps {
  /** Authority-backed recent changes. Empty array = no data (section hidden). */
  recentChanges: OverviewChange[];
  /** Known contradictions in memory. Empty array hides the section. */
  contradictions: OverviewContradiction[];
  /** Active or paused goals. Empty array hides the section. */
  activeGoals: OverviewGoal[];
  /**
   * Number of pending cognition tasks. 0 = nothing pending (section hidden).
   * Shown as "N tasks pending" when > 0 — this is a real count, not an
   * inferred health state, so it is the one exception to the hide-when-zero
   * rule that applies to arrays.
   */
  pendingCognitionCount: number;
  /**
   * True when no authority data has been recorded yet. When true the
   * onboarding section replaces the data sections.
   */
  isEmpty: boolean;
  /** Onboarding flow state. */
  onboardingState: "none" | "prompted" | "in-progress";
  /** Called when the user consents to allowing a source scan. */
  onRequestSourceConsent: () => void;
  /** Called when the user submits a new goal title during onboarding. */
  onStartGoal: (title: string) => void;
}

// ─── Component ───────────────────────────────────────────────────────────────

export function Overview(props: OverviewProps) {
  // Local signal for the onboarding goal input value
  let goalInputRef: HTMLInputElement | undefined;

  function handleGoalSubmit(e: Event) {
    e.preventDefault();
    const value = goalInputRef?.value.trim() ?? "";
    if (value) {
      props.onStartGoal(value);
      if (goalInputRef) goalInputRef.value = "";
    }
  }

  return (
    <section aria-label="Overview">
      {/* ── Onboarding (empty state) ─────────────────────────────────────── */}
      <Show when={props.isEmpty}>
        <section data-testid="onboarding" aria-label="Get started">
          <h2>Get started</h2>
          <p>No data has been recorded yet. Add a goal to begin.</p>

          {/* Goal-led onboarding form */}
          <form
            data-testid="onboarding-goal-form"
            onSubmit={handleGoalSubmit}
            aria-label="Start a goal"
          >
            <label for="onboarding-goal-input">Goal title</label>
            <input
              id="onboarding-goal-input"
              ref={goalInputRef}
              type="text"
              placeholder="What do you want to accomplish?"
              autocomplete="off"
            />
            <button type="submit">Start goal</button>
          </form>

          {/* Source-consent request — always shown in the onboarding section */}
          <button
            data-testid="source-consent-button"
            type="button"
            onClick={() => props.onRequestSourceConsent()}
          >
            Allow source scan
          </button>
        </section>
      </Show>

      {/* ── Data sections (non-empty state) ─────────────────────────────── */}
      <Show when={!props.isEmpty}>
        {/* Recent changes — always shown in the non-empty view */}
        <section data-testid="recent-changes" aria-label="Recent changes">
          <h2>Recent changes</h2>
          <Show
            when={props.recentChanges.length > 0}
            fallback={<p>No recent changes.</p>}
          >
            <ul>
              <For each={props.recentChanges}>
                {(change) => (
                  <li data-change-id={change.id}>
                    <span data-field="kind">{change.kind}</span>
                    {" — "}
                    <span data-field="label">{change.label}</span>
                    {" "}
                    <time data-field="timestamp">{change.timestamp}</time>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </section>

        {/* Contradictions — hidden when empty (no "0 items" shown) */}
        <Show when={props.contradictions.length > 0}>
          <section data-testid="contradictions" aria-label="Contradictions">
            <h2>Contradictions</h2>
            <ul>
              <For each={props.contradictions}>
                {(c) => (
                  <li data-contradiction-id={c.id}>
                    {c.description}
                  </li>
                )}
              </For>
            </ul>
          </section>
        </Show>

        {/* Active goals — hidden when empty (no "0 items" shown) */}
        <Show when={props.activeGoals.length > 0}>
          <section data-testid="active-goals" aria-label="Active goals">
            <h2>Active goals</h2>
            <ul>
              <For each={props.activeGoals}>
                {(goal) => (
                  <li data-goal-id={goal.id} data-status={goal.status}>
                    {goal.title}
                  </li>
                )}
              </For>
            </ul>
          </section>
        </Show>

        {/* Pending cognition — shown only when count > 0 */}
        <Show when={props.pendingCognitionCount > 0}>
          <section data-testid="pending-cognition" aria-label="Pending cognition">
            <span>{props.pendingCognitionCount} tasks pending</span>
          </section>
        </Show>
      </Show>
    </section>
  );
}

export default Overview;
