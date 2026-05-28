/**
 * WorkflowProgress — Canonical Workflow Progress Renderer.
 *
 * Renders workflow state from typed WorkflowSession data.
 * NEVER parses strings to determine workflow state.
 *
 * Features:
 * - Live step progress with execution mode indicators
 * - Verdict badge with honest reporting
 * - HITL modal integration
 * - Continuation action buttons
 * - Lifecycle-aware rendering
 */

import { Component, Show, For, createMemo } from "solid-js";
import type {
  WorkflowSession,
  WorkflowStepState,
  WorkflowVerdict,
  ActiveHitl,
  ContinuationAction,
  StepStatus,
} from "../types/workflowRuntime";
import {
  verdictIcon,
  verdictColorClass,
  verdictLabel,
  stepModeIcon,
  stepTypeIcon,
} from "../types/workflowRuntime";

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — Main Component
// ═══════════════════════════════════════════════════════════════════════════════

interface WorkflowProgressProps {
  session: WorkflowSession;
  onHitlRespond?: (optionId: string) => void;
  onCancel?: () => void;
  onContinuation?: (action: ContinuationAction) => void;
  onDismiss?: () => void;
  compact?: boolean;
}

export const WorkflowProgress: Component<WorkflowProgressProps> = (props) => {
  const completedCount = createMemo(() =>
    props.session.steps.filter((s) => s.status === 'completed').length
  );
  const totalCount = createMemo(() => props.session.steps.length);
  const progressPercent = createMemo(() =>
    totalCount() > 0 ? Math.round((completedCount() / totalCount()) * 100) : 0
  );
  const isActive = createMemo(() =>
    props.session.lifecycle === 'executing' || props.session.lifecycle === 'hitl_pending'
  );

  return (
    <div class="workflow-progress rounded-lg overflow-hidden border border-slate-700 bg-slate-800/50">
      {/* Header */}
      <div class="px-4 py-3 border-b border-slate-700 flex items-center justify-between">
        <div class="flex items-center gap-2">
          <LifecycleIndicator lifecycle={props.session.lifecycle} />
          <span class="text-sm text-slate-300 font-medium">
            {props.session.lifecycle === 'executing' ? 'Running workflow...' :
             props.session.lifecycle === 'hitl_pending' ? 'Action needed' :
             props.session.lifecycle === 'finalized' ? 'Workflow complete' :
             props.session.lifecycle === 'cancelled' ? 'Cancelled' : 'Workflow'}
          </span>
        </div>
        <Show when={isActive()}>
          <button
            onClick={() => props.onCancel?.()}
            class="text-xs text-slate-500 hover:text-red-400 transition-colors"
          >
            Cancel
          </button>
        </Show>
      </div>

      {/* Progress bar */}
      <Show when={isActive()}>
        <div class="h-1 bg-slate-700">
          <div
            class="h-full bg-blue-500 transition-all duration-500"
            style={{ width: `${progressPercent()}%` }}
          />
        </div>
      </Show>

      {/* Steps */}
      <Show when={!props.compact}>
        <div class="px-4 py-3 space-y-1.5">
          <For each={props.session.steps}>
            {(step) => <StepRow step={step} />}
          </For>
        </div>
      </Show>

      {/* Compact mode: just show count */}
      <Show when={props.compact && isActive()}>
        <div class="px-4 py-2 text-xs text-slate-400">
          Step {completedCount() + 1}/{totalCount()}
        </div>
      </Show>

      {/* HITL Modal */}
      <Show when={props.session.hitlState}>
        <HitlPanel
          hitl={props.session.hitlState!}
          onRespond={(id) => props.onHitlRespond?.(id)}
        />
      </Show>

      {/* Verdict */}
      <Show when={props.session.verdict}>
        <VerdictPanel
          verdict={props.session.verdict!}
          continuations={props.session.continuationActions}
          onContinuation={(a) => props.onContinuation?.(a)}
          onDismiss={() => props.onDismiss?.()}
        />
      </Show>
    </div>
  );
};

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Sub-Components
// ═══════════════════════════════════════════════════════════════════════════════

const LifecycleIndicator: Component<{ lifecycle: string }> = (props) => {
  const indicator = createMemo(() => {
    switch (props.lifecycle) {
      case 'executing': return { icon: '●', class: 'text-blue-400 animate-pulse' };
      case 'hitl_pending': return { icon: '⏸', class: 'text-yellow-400' };
      case 'finalized': return { icon: '●', class: 'text-green-400' };
      case 'cancelled': return { icon: '●', class: 'text-slate-500' };
      default: return { icon: '○', class: 'text-slate-500' };
    }
  });

  return <span class={`text-xs ${indicator().class}`}>{indicator().icon}</span>;
};

const StepRow: Component<{ step: WorkflowStepState }> = (props) => {
  const statusIcon = createMemo(() => {
    switch (props.step.status) {
      case 'completed': return '✓';
      case 'running': return '●';
      case 'failed': return '✗';
      case 'skipped': return '○';
      default: return '○';
    }
  });

  const statusClass = createMemo(() => {
    switch (props.step.status) {
      case 'completed': return 'text-green-400';
      case 'running': return 'text-blue-400 animate-pulse';
      case 'failed': return 'text-red-400';
      case 'skipped': return 'text-slate-600';
      default: return 'text-slate-600';
    }
  });

  return (
    <div class="flex items-center gap-2 text-sm">
      <span class={`w-4 text-center ${statusClass()}`}>{statusIcon()}</span>
      <span class="text-slate-500 text-xs">{stepModeIcon(props.step.executionMode)}</span>
      <span class={props.step.status === 'pending' ? 'text-slate-500' : 'text-slate-300'}>
        {props.step.description}
      </span>
    </div>
  );
};

const HitlPanel: Component<{ hitl: ActiveHitl; onRespond: (id: string) => void }> = (props) => {
  return (
    <div class="px-4 py-3 bg-yellow-900/20 border-t border-yellow-700/50">
      <p class="text-sm text-yellow-200 mb-2">{props.hitl.context}</p>
      <div class="flex flex-wrap gap-2">
        <For each={props.hitl.options}>
          {(option) => (
            <button
              onClick={() => props.onRespond(option.id)}
              class={`text-xs px-3 py-1.5 rounded font-medium transition-colors ${
                option.action_type.type === 'cancel'
                  ? 'bg-slate-700 text-slate-300 hover:bg-slate-600'
                  : 'bg-blue-600 text-white hover:bg-blue-500'
              }`}
            >
              {option.label}
            </button>
          )}
        </For>
      </div>
    </div>
  );
};

const VerdictPanel: Component<{
  verdict: WorkflowVerdict;
  continuations: ContinuationAction[];
  onContinuation: (a: ContinuationAction) => void;
  onDismiss: () => void;
}> = (props) => {
  return (
    <div class="px-4 py-3 border-t border-slate-700">
      <div class="flex items-center gap-2 mb-2">
        <span class={`text-lg ${verdictColorClass(props.verdict)}`}>
          {verdictIcon(props.verdict)}
        </span>
        <span class={`text-sm font-medium ${verdictColorClass(props.verdict)}`}>
          {verdictLabel(props.verdict)}
        </span>
      </div>

      {/* Unverified outcomes notice */}
      <Show when={props.verdict.type === 'structurally_complete'}>
        <p class="text-xs text-slate-400 mb-2">
          {(props.verdict as any).unverified_outcomes?.join(', ')}
        </p>
      </Show>

      {/* Continuation actions */}
      <Show when={props.continuations.length > 0}>
        <div class="flex flex-wrap gap-2 mt-2">
          <For each={props.continuations}>
            {(action) => (
              <button
                onClick={() => props.onContinuation(action)}
                class="text-xs px-3 py-1.5 rounded bg-slate-700 text-slate-300 hover:bg-slate-600 transition-colors"
              >
                {action.label}
              </button>
            )}
          </For>
        </div>
      </Show>

      <button
        onClick={() => props.onDismiss()}
        class="mt-2 text-xs text-slate-500 hover:text-slate-400"
      >
        Dismiss
      </button>
    </div>
  );
};

export default WorkflowProgress;
