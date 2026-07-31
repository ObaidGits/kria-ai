/**
 * Goals — Memory Control Center Goals destination (full operational content).
 *
 * Renders goals with lifecycle status, provenance, evidence, linked memories,
 * priority, progress, conflicts, resume context, and per-status action buttons.
 * Also shows the current action phase (confirming / committing / committed / error).
 *
 * Invariants (F4.2 / task 4.5.1):
 * - Root: data-testid="goals-destination"
 * - Loading: data-testid="goals-loading" role="status" — only when isLoading
 * - Error:  data-testid="goals-error"   role="alert"  — only when errorMessage non-null
 * - Goals list: data-testid="goals-list" role="list" — only when goals non-empty and not loading
 * - Empty: data-testid="goals-empty" — only when not loading and goals empty
 * - Per goal: data-testid="goal-{id}" data-status={goal.status}
 *   - data-testid="goal-title-{id}"
 *   - data-testid="goal-status-{id}"
 *   - data-testid="goal-provenance-{id}"
 *   - data-testid="goal-evidence-{id}" — only when evidenceSummary non-null
 *   - data-testid="goal-linked-memories-{id}" — only when linkedMemoryCount non-null
 *   - data-testid="goal-priority-{id}" — only when priority non-null
 *   - data-testid="goal-last-updated-{id}"
 *   - data-testid="goal-progress-{id}" — only when progress non-null
 *     - data-testid="goal-progress-percent-{id}" — only when percent non-null
 *     - data-testid="goal-progress-milestone-{id}" — only when milestoneLabel non-null
 *   - data-testid="goal-conflicts-{id}" — only when conflicts non-empty
 *     - data-testid="goal-conflict-{conflictingGoalId}"
 *   - data-testid="goal-resume-{id}" — only when resumeContext non-null AND status==="paused"
 *   - data-testid="goal-accept-{id}" / data-testid="goal-reject-{id}" — candidate only
 *   - data-testid="goal-pause-{id}" / data-testid="goal-complete-{id}" — active only
 *   - data-testid="goal-activate-{id}" — paused only
 *   - data-testid="goal-priority-selector-{id}" — active or paused; 1-5 priority buttons
 * - data-testid="goal-action-phase" data-phase={actionPhase.phase} — always present
 * - Committed: data-testid="goal-action-revision" and data-testid="goal-action-audit"
 * - Error: data-testid="goal-action-error" role="alert"
 * - UI never invents facts — all labels come from backend data.
 *
 * Requirements: F4.2 (task 4.5.1)
 */
import { For, Show, Switch, Match } from "solid-js";

// ─── Data types ───────────────────────────────────────────────────────────────

export type GoalStatus =
  | 'candidate'
  | 'active'
  | 'paused'
  | 'completed'
  | 'conflict'
  | 'stale'
  | 'deleted';

export interface GoalProgress {
  percent: number | null;
  milestoneLabel: string | null;
  milestoneCount: number | null;
  milestoneCompleted: number | null;
}

export interface GoalConflict {
  conflictingGoalId: string;
  conflictingGoalLabel: string;
  conflictDescription: string;
}

export interface Goal {
  id: string;
  title: string;
  status: GoalStatus;
  priority: number | null;
  provenanceLabel: string;
  evidenceSummary: string | null;
  linkedMemoryCount: number | null;
  progress: GoalProgress | null;
  conflicts: GoalConflict[];
  resumeContext: string | null;
  lastUpdated: string;
}

export type GoalActionPhase =
  | { phase: 'idle' }
  | { phase: 'confirming'; goalId: string; action: 'activate' | 'pause' | 'complete' | 'reject' | 'accept-candidate' | 'update-priority' }
  | { phase: 'committing' }
  | { phase: 'committed'; newRevision: number; auditRecordId: string }
  | { phase: 'error'; message: string };

export interface GoalsState {
  goals: Goal[];
  isLoading: boolean;
  errorMessage: string | null;
  actionPhase: GoalActionPhase;
  selectedPriorityValue: number | null;
}

// ─── Props ────────────────────────────────────────────────────────────────────

export interface GoalsProps {
  state: GoalsState;
  onActivate: (goalId: string) => void;
  onPause: (goalId: string) => void;
  onComplete: (goalId: string) => void;
  onRejectCandidate: (goalId: string) => void;
  onAcceptCandidate: (goalId: string) => void;
  onUpdatePriority: (goalId: string, priority: number) => void;
  onActionCommit: () => void;
  onActionCancel: () => void;
}

// ─── Priority selector ────────────────────────────────────────────────────────

function PrioritySelector(props: {
  goalId: string;
  onUpdatePriority: (goalId: string, priority: number) => void;
}) {
  const priorities = [1, 2, 3, 4, 5] as const;
  return (
    <div data-testid={`goal-priority-selector-${props.goalId}`}>
      <For each={priorities}>
        {(p) => (
          <button
            type="button"
            data-testid={`goal-priority-selector-${props.goalId}-${p}`}
            onClick={() => props.onUpdatePriority(props.goalId, p)}
          >
            {p}
          </button>
        )}
      </For>
    </div>
  );
}

// ─── Single goal item ─────────────────────────────────────────────────────────

function GoalItem(props: { goal: Goal; goalProps: GoalsProps }) {
  const goal = () => props.goal;
  const id = () => props.goal.id;

  return (
    <li
      data-testid={`goal-${id()}`}
      data-status={goal().status}
      role="listitem"
    >
      {/* Title */}
      <span data-testid={`goal-title-${id()}`}>{goal().title}</span>

      {/* Status label */}
      <span data-testid={`goal-status-${id()}`}>{goal().status}</span>

      {/* Provenance */}
      <span data-testid={`goal-provenance-${id()}`}>{goal().provenanceLabel}</span>

      {/* Evidence summary — only when non-null */}
      <Show when={goal().evidenceSummary !== null}>
        <span data-testid={`goal-evidence-${id()}`}>{goal().evidenceSummary}</span>
      </Show>

      {/* Linked memory count — only when non-null */}
      <Show when={goal().linkedMemoryCount !== null}>
        <span data-testid={`goal-linked-memories-${id()}`}>{goal().linkedMemoryCount}</span>
      </Show>

      {/* Priority — only when non-null */}
      <Show when={goal().priority !== null}>
        <span data-testid={`goal-priority-${id()}`}>{goal().priority}</span>
      </Show>

      {/* Last updated */}
      <span data-testid={`goal-last-updated-${id()}`}>{goal().lastUpdated}</span>

      {/* Progress — only when non-null */}
      <Show when={goal().progress !== null}>
        <div data-testid={`goal-progress-${id()}`}>
          <Show when={goal().progress!.percent !== null}>
            <span data-testid={`goal-progress-percent-${id()}`}>
              {goal().progress!.percent}
            </span>
          </Show>
          <Show when={goal().progress!.milestoneLabel !== null}>
            <span data-testid={`goal-progress-milestone-${id()}`}>
              {goal().progress!.milestoneLabel}
            </span>
          </Show>
        </div>
      </Show>

      {/* Conflicts — only when non-empty */}
      <Show when={goal().conflicts.length > 0}>
        <div data-testid={`goal-conflicts-${id()}`}>
          <For each={goal().conflicts}>
            {(conflict) => (
              <div data-testid={`goal-conflict-${conflict.conflictingGoalId}`}>
                <span>{conflict.conflictingGoalLabel}</span>
                <span>{conflict.conflictDescription}</span>
              </div>
            )}
          </For>
        </div>
      </Show>

      {/* Resume context — only when paused AND resumeContext non-null */}
      <Show when={goal().status === 'paused' && goal().resumeContext !== null}>
        <div data-testid={`goal-resume-${id()}`}>{goal().resumeContext}</div>
      </Show>

      {/* ── Status-specific action buttons ───────────────────────────── */}

      {/* Candidate: accept + reject */}
      <Show when={goal().status === 'candidate'}>
        <button
          type="button"
          data-testid={`goal-accept-${id()}`}
          onClick={() => props.goalProps.onAcceptCandidate(id())}
        >
          Accept
        </button>
        <button
          type="button"
          data-testid={`goal-reject-${id()}`}
          onClick={() => props.goalProps.onRejectCandidate(id())}
        >
          Reject
        </button>
      </Show>

      {/* Active: pause + complete */}
      <Show when={goal().status === 'active'}>
        <button
          type="button"
          data-testid={`goal-pause-${id()}`}
          onClick={() => props.goalProps.onPause(id())}
        >
          Pause
        </button>
        <button
          type="button"
          data-testid={`goal-complete-${id()}`}
          onClick={() => props.goalProps.onComplete(id())}
        >
          Complete
        </button>
      </Show>

      {/* Paused: activate */}
      <Show when={goal().status === 'paused'}>
        <button
          type="button"
          data-testid={`goal-activate-${id()}`}
          onClick={() => props.goalProps.onActivate(id())}
        >
          Activate
        </button>
      </Show>

      {/* Priority selector — active or paused */}
      <Show when={goal().status === 'active' || goal().status === 'paused'}>
        <PrioritySelector
          goalId={id()}
          onUpdatePriority={props.goalProps.onUpdatePriority}
        />
      </Show>
    </li>
  );
}

// ─── Action phase indicator ───────────────────────────────────────────────────

function ActionPhaseIndicator(props: {
  actionPhase: GoalActionPhase;
  onActionCommit: () => void;
  onActionCancel: () => void;
}) {
  return (
    <Switch>
      <Match when={props.actionPhase.phase === 'idle'}>
        <div data-testid="goal-action-phase" data-phase="idle" />
      </Match>

      <Match when={props.actionPhase.phase === 'confirming'}>
        <div data-testid="goal-action-phase" data-phase="confirming">
          <button type="button" data-testid="goal-action-commit" onClick={props.onActionCommit}>
            Confirm
          </button>
          <button type="button" data-testid="goal-action-cancel" onClick={props.onActionCancel}>
            Cancel
          </button>
        </div>
      </Match>

      <Match when={props.actionPhase.phase === 'committing'}>
        <div data-testid="goal-action-phase" data-phase="committing">
          <span role="status" aria-live="polite">Committing…</span>
        </div>
      </Match>

      <Match when={props.actionPhase.phase === 'committed' && props.actionPhase}>
        {(s) => {
          const committed = () => s() as Extract<GoalActionPhase, { phase: 'committed' }>;
          return (
            <div data-testid="goal-action-phase" data-phase="committed">
              <span data-testid="goal-action-revision">{committed().newRevision}</span>
              <span data-testid="goal-action-audit">{committed().auditRecordId}</span>
            </div>
          );
        }}
      </Match>

      <Match when={props.actionPhase.phase === 'error' && props.actionPhase}>
        {(s) => {
          const err = () => s() as Extract<GoalActionPhase, { phase: 'error' }>;
          return (
            <div data-testid="goal-action-phase" data-phase="error">
              <div data-testid="goal-action-error" role="alert">
                {err().message}
              </div>
            </div>
          );
        }}
      </Match>
    </Switch>
  );
}

// ─── Root component ───────────────────────────────────────────────────────────

export function Goals(props: GoalsProps) {
  const state = () => props.state;
  const hasGoals = () => state().goals.length > 0;
  const showEmpty = () => !state().isLoading && !hasGoals();

  return (
    <section data-testid="goals-destination" aria-label="Goals">

      {/* ── Loading indicator ──────────────────────────────────────────── */}
      <Show when={state().isLoading}>
        <span data-testid="goals-loading" role="status" aria-live="polite">
          Loading goals…
        </span>
      </Show>

      {/* ── Error ──────────────────────────────────────────────────────── */}
      <Show when={state().errorMessage !== null}>
        <div data-testid="goals-error" role="alert">
          {state().errorMessage}
        </div>
      </Show>

      {/* ── Goals list ────────────────────────────────────────────────── */}
      <Show when={!state().isLoading && hasGoals()}>
        <ul data-testid="goals-list" role="list" aria-label="Goals list">
          <For each={state().goals}>
            {(goal) => <GoalItem goal={goal} goalProps={props} />}
          </For>
        </ul>
      </Show>

      {/* ── Empty state ───────────────────────────────────────────────── */}
      <Show when={showEmpty()}>
        <span data-testid="goals-empty">No goals</span>
      </Show>

      {/* ── Action phase ──────────────────────────────────────────────── */}
      <ActionPhaseIndicator
        actionPhase={state().actionPhase}
        onActionCommit={props.onActionCommit}
        onActionCancel={props.onActionCancel}
      />

    </section>
  );
}

export default Goals;
