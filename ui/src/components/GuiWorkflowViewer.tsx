/**
 * RFC 007 Phase 4 - GUI Workflow Viewer Component
 * 
 * Renders HTN workflow execution state with:
 * - Immutable sub-goals with execution status
 * - Bounded micro-retry status for verification steps
 * - Kill switch indicator and emergency stop button
 * - Safe abort steps display when abort is triggered
 */

import { Component, createSignal, createEffect, onCleanup, Show, For } from "solid-js";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type {
  GuiWorkflow,
  GuiExecutionProgress,
  SubGoal,
  KillSwitchState,
  GuiWorkflowStartedEvent,
  GuiWorkflowStepEvent,
  GuiWorkflowCompletedEvent,
  KillSwitchTriggeredEvent,
} from "../types/guiAutomation";

// Status badge colors
const STATUS_COLORS: Record<string, string> = {
  pending: "bg-gray-200 text-gray-700",
  running: "bg-blue-500 text-white animate-pulse",
  completed: "bg-blue-600 text-white", // generic success fallback
  verified: "bg-green-600 text-white shadow-[0_0_8px_rgba(22,163,74,0.5)]", // FullSemantic
  partially_verified: "bg-yellow-500 text-white shadow-[0_0_8px_rgba(234,179,8,0.5)]", // PartialObservable / StructuralOnly
  unverified: "bg-orange-400 text-white", // Executed but Unobservable
  failed: "bg-red-500 text-white shadow-[0_0_8px_rgba(239,68,68,0.5)]",
  aborted: "bg-orange-500 text-white",
};

const STATUS_ICONS: Record<string, string> = {
  pending: "⏳",
  running: "▶",
  completed: "✓",
  verified: "✨", // high confidence
  partially_verified: "✓", // lower confidence
  unverified: "👁‍🗨", // unobservable
  failed: "✗",
  aborted: "⚠",
};

interface GuiWorkflowViewerProps {
  initialWorkflow?: GuiWorkflow;
  taskId?: string;
  onCancel?: () => void;
  showKillSwitch?: boolean;
  class?: string;
}

export const GuiWorkflowViewer: Component<GuiWorkflowViewerProps> = (props) => {
  // Reactive state for workflow execution
  const [workflow, setWorkflow] = createSignal<GuiWorkflow | null>(props.initialWorkflow || null);
  const [progress, setProgress] = createSignal<GuiExecutionProgress | null>(null);
  const [killSwitchActive, setKillSwitchActive] = createSignal(false);
  const [abortTriggered, setAbortTriggered] = createSignal(false);
  const [completedSteps, setCompletedSteps] = createSignal<Set<number>>(new Set<number>());
  const [failedSteps, setFailedSteps] = createSignal<Set<number>>(new Set<number>());
  const [stepStates, setStepStates] = createSignal<Map<number, string>>(new Map<number, string>());
  const [retryCount, setRetryCount] = createSignal<Record<number, number>>({});
  
  // Event listeners cleanup
  let unlisteners: UnlistenFn[] = [];

  // Subscribe to Tauri events
  createEffect(() => {
    const setupListeners = async () => {
      // Workflow started event
      const unlistenStart = await listen<GuiWorkflowStartedEvent>(
        "gui-workflow-started",
        (event) => {
          console.log("[GuiWorkflowViewer] Workflow started:", event.payload);
          setWorkflow(event.payload.workflow);
          setProgress({
            task_id: event.payload.task_id,
            status: "running",
            current_step: 0,
            total_steps: event.payload.workflow.sub_goals.length,
            current_action: "initializing",
            sub_goals: event.payload.workflow.sub_goals,
            safe_abort_steps: event.payload.workflow.safe_abort_steps,
            kill_switch: { is_active: false },
            started_at: event.payload.timestamp,
          });
          setCompletedSteps(new Set<number>());
          setFailedSteps(new Set<number>());
          setStepStates(new Map<number, string>());
          setAbortTriggered(false);
        }
      );

      // Step progress event
      const unlistenStep = await listen<GuiWorkflowStepEvent>(
        "gui-workflow-step",
        (event) => {
          console.log("[GuiWorkflowViewer] Step update:", event.payload);
          const { step, status, action } = event.payload;
          
          setProgress((prev) =>
            prev
              ? {
                  ...prev,
                  current_step: step,
                  current_action: action,
                  status: status === "failed" ? "failed" : "running",
                }
              : null
          );

          if (status === "failed") {
            setFailedSteps((prev) => new Set([...prev, step]));
            setRetryCount((prev) => ({
              ...prev,
              [step]: (prev[step] || 0) + 1,
            }));
          } else if (status === "completed" || status === "verified" || status === "partially_verified" || status === "unverified") {
            setCompletedSteps((prev) => new Set([...prev, step]));
            setStepStates((prev) => {
              const next = new Map(prev);
              next.set(step, status);
              return next;
            });
          }
        }
      );

      // Workflow completed event
      const unlistenComplete = await listen<GuiWorkflowCompletedEvent>(
        "gui-workflow-completed",
        (event) => {
          console.log("[GuiWorkflowViewer] Workflow completed:", event.payload);
          const result = event.payload.result;
          
          setProgress((prev) =>
            prev
              ? {
                  ...prev,
                  status: result.success ? "completed" : result.aborted ? "aborted" : "failed",
                  result,
                  completed_at: event.payload.timestamp,
                }
              : null
          );
        }
      );

      // Kill switch triggered event
      const unlistenKillSwitch = await listen<KillSwitchTriggeredEvent>(
        "kill-switch-triggered",
        (event) => {
          console.log("[GuiWorkflowViewer] Kill switch triggered:", event.payload);
          setKillSwitchActive(true);
          setAbortTriggered(true);
          
          setProgress((prev) =>
            prev
              ? {
                  ...prev,
                  status: "aborted",
                  kill_switch: {
                    is_active: true,
                    triggered_at: event.payload.timestamp,
                    reason: event.payload.reason,
                  },
                }
              : null
          );
        }
      );

      unlisteners = [unlistenStart, unlistenStep, unlistenComplete, unlistenKillSwitch];
    };

    setupListeners();

    onCleanup(() => {
      unlisteners.forEach((unlisten) => unlisten());
    });
  });

  // Calculate progress percentage
  const progressPercent = () => {
    const wf = workflow();
    const completed = completedSteps();
    if (!wf) return 0;
    return Math.round((completed.size / wf.sub_goals.length) * 100);
  };

  // Handle kill switch button click
  const handleKillSwitch = async () => {
    if (props.onCancel) {
      props.onCancel();
    } else {
      // Emit kill switch event via Tauri command
      try {
        await invoke("trigger_kill_switch", { taskId: workflow()?.task_id });
      } catch (err) {
        console.error("[GuiWorkflowViewer] Failed to trigger kill switch:", err);
      }
    }
  };

  return (
    <div class={`gui-workflow-viewer ${props.class || ""}`}>
      {/* Header with workflow ID and status */}
      <Show when={workflow()}>
        <div class="workflow-header bg-slate-800 rounded-t-lg p-4 border-b border-slate-700">
          <div class="flex items-center justify-between">
            <div class="flex items-center gap-3">
              <span class="text-2xl">🤖</span>
              <div>
                <h3 class="text-white font-semibold">GUI Workflow</h3>
                <p class="text-slate-400 text-sm font-mono">
                  Task: {workflow()?.task_id}
                </p>
              </div>
            </div>
            
            {/* Kill Switch Button */}
            <Show when={props.showKillSwitch !== false && progress()?.status === "running"}>
              <button
                onClick={handleKillSwitch}
                class="kill-switch-btn bg-red-600 hover:bg-red-700 text-white px-4 py-2 rounded-lg font-bold flex items-center gap-2 transition-colors"
              >
                <span class="text-xl">🛑</span>
                <span>KILL SWITCH</span>
              </button>
            </Show>
          </div>

          {/* Progress bar */}
          <div class="mt-4">
            <div class="flex justify-between text-sm text-slate-400 mb-1">
              <span>Progress</span>
              <span>{progressPercent()}%</span>
            </div>
            <div class="h-2 bg-slate-700 rounded-full overflow-hidden">
              <div
                class="h-full bg-gradient-to-r from-blue-500 to-green-500 transition-all duration-300"
                style={{ width: `${progressPercent()}%` }}
              />
            </div>
          </div>
        </div>
      </Show>

      {/* Kill Switch Active Warning */}
      <Show when={killSwitchActive()}>
        <div class="kill-switch-warning bg-red-900 border-l-4 border-red-500 p-4">
          <div class="flex items-center gap-3">
            <span class="text-3xl animate-pulse">🛑</span>
            <div>
              <h4 class="text-red-200 font-bold text-lg">KILL SWITCH ACTIVATED</h4>
              <p class="text-red-300 text-sm">
                Workflow aborted. Executing safe abort sequence...
              </p>
            </div>
          </div>
        </div>
      </Show>

      {/* Sub-goals list */}
      <Show when={workflow()}>
        <div class="sub-goals-container bg-slate-900 p-4">
          <h4 class="text-slate-300 font-semibold mb-3 flex items-center gap-2">
            <span>📋</span>
            <span>Sub-Goals (Immutable)</span>
          </h4>
          
          <div class="space-y-2">
            <For each={workflow()?.sub_goals}>
              {(goal: SubGoal) => {
                const isCompleted = () => completedSteps().has(goal.step);
                const isFailed = () => failedSteps().has(goal.step);
                const isRunning = () => progress()?.current_step === goal.step && !isCompleted() && !isFailed();
                const stepState = () => stepStates().get(goal.step);
                
                const statusClass = () => {
                    if (isFailed()) return "failed";
                    if (isRunning()) return "running";
                    if (isCompleted()) return stepState() || "completed";
                    return "pending";
                };

                const retryNum = () => retryCount()[goal.step] || 0;
                
                return (
                  <div
                    class={`sub-goal-item p-3 rounded-lg border transition-all ${
                      STATUS_COLORS[statusClass()] || "bg-slate-800 border-slate-700"
                    }`}
                  >
                    <div class="flex items-start gap-3">
                      {/* Step number */}
                      <div
                        class={`flex-shrink-0 w-8 h-8 rounded-full flex items-center justify-center font-bold ${
                          isCompleted()
                            ? "bg-green-600 text-white"
                            : isFailed()
                            ? "bg-red-600 text-white"
                            : isRunning()
                            ? "bg-blue-600 text-white"
                            : "bg-slate-700 text-slate-400"
                        }`}
                      >
                        {goal.step}
                      </div>

                      {/* Action details */}
                      <div class="flex-1 min-w-0">
                        <div class="flex items-center gap-2">
                          <span class="text-white font-mono text-sm">
                            {goal.action}
                          </span>
                          <Show when={isRunning()}>
                            <span class="animate-spin">⏳</span>
                          </Show>
                          <Show when={isCompleted()}>
                            {statusClass() === 'verified' && <span class="ml-2 text-xs opacity-75">Verified</span>}
                            {statusClass() === 'partially_verified' && <span class="ml-2 text-xs opacity-75">Partially Verified</span>}
                            {statusClass() === 'unverified' && <span class="ml-2 text-xs opacity-75">Executed (Unverified)</span>}
                            <span class="text-white ml-2">{STATUS_ICONS[statusClass()] || STATUS_ICONS.completed}</span>
                          </Show>
                          <Show when={isFailed()}>
                            <span class="text-white">{STATUS_ICONS.failed}</span>
                          </Show>
                        </div>
                        
                        {/* Parameters */}
                        <Show when={Object.keys(goal.params).length > 0}>
                          <div class="mt-1 text-slate-400 text-xs font-mono">
                            {JSON.stringify(goal.params)}
                          </div>
                        </Show>

                        {/* Verification strategy */}
                        <Show when={goal.verify.type !== "none"}>
                          <div class="mt-2 text-xs text-slate-500">
                            <span class="font-semibold">Verify:</span>{" "}
                            {goal.verify.type}
                            <Show when={goal.verify.type === "text_present"}>
                              <span class="ml-1 text-slate-400">
                                "{(goal.verify as { type: "text_present"; text: string }).text}"
                              </span>
                            </Show>
                          </div>
                        </Show>

                        {/* Retry indicator */}
                        <Show when={retryNum() > 0}>
                          <div class="mt-2 text-xs text-orange-400 flex items-center gap-1">
                            <span>🔄</span>
                            <span>Retry {retryNum()}/3 (bounded micro-retry)</span>
                          </div>
                        </Show>
                      </div>
                    </div>
                  </div>
                );
              }}
            </For>
          </div>
        </div>
      </Show>

      {/* Safe Abort Steps */}
      <Show when={workflow()?.safe_abort_steps && workflow()!.safe_abort_steps.length > 0}>
        <div class={`safe-abort-container p-4 ${abortTriggered() ? "bg-orange-900/30" : "bg-slate-800"}`}>
          <details class="group">
            <summary class="flex items-center gap-2 cursor-pointer text-slate-300 font-semibold">
              <span>🛡️</span>
              <span>Safe Abort Steps</span>
              <Show when={abortTriggered()}>
                <span class="text-orange-400 text-sm animate-pulse">
                  (EXECUTING)
                </span>
              </Show>
              <span class="ml-auto text-slate-500 group-open:rotate-180 transition-transform">
                ▼
              </span>
            </summary>
            
            <div class="mt-3 space-y-2">
              <For each={workflow()?.safe_abort_steps}>
                {(step, idx) => (
                  <div
                    class={`p-2 rounded border ${
                      abortTriggered()
                        ? "bg-orange-900/50 border-orange-700"
                        : "bg-slate-900 border-slate-700"
                    }`}
                  >
                    <div class="flex items-center gap-2">
                      <span class="text-slate-400 text-sm">{idx() + 1}.</span>
                      <span class="text-white font-mono text-sm">{step.action}</span>
                      <Show when={abortTriggered()}>
                        <span class="text-orange-400 animate-pulse">▶</span>
                      </Show>
                    </div>
                    <Show when={Object.keys(step.params).length > 0}>
                      <div class="mt-1 text-slate-500 text-xs font-mono ml-5">
                        {JSON.stringify(step.params)}
                      </div>
                    </Show>
                  </div>
                )}
              </For>
            </div>
          </details>
        </div>
      </Show>

      {/* Result summary */}
      <Show when={progress()?.result}>
        <div
          class={`result-summary p-4 rounded-b-lg ${
            progress()?.result?.success
              ? "bg-green-900/50 border-t border-green-700"
              : "bg-red-900/50 border-t border-red-700"
          }`}
        >
          <div class="flex items-center gap-3">
            <Show when={progress()?.result?.success}>
              <>
                <span class="text-3xl">✅</span>
                <div>
                  <h4 class="text-green-200 font-bold">Workflow Completed</h4>
                  <p class="text-green-300 text-sm">
                    {progress()?.result?.completed_steps}/{progress()?.result?.total_steps} steps
                    in {progress()?.result?.duration_ms}ms
                  </p>
                </div>
              </>
            </Show>
            <Show when={!progress()?.result?.success}>
              <>
                <span class="text-3xl">❌</span>
                <div>
                  <h4 class="text-red-200 font-bold">
                    {progress()?.result?.aborted ? "Workflow Aborted" : "Workflow Failed"}
                  </h4>
                  <p class="text-red-300 text-sm">
                    {progress()?.result?.error || "Unknown error"}
                  </p>
                </div>
              </>
            </Show>
          </div>
        </div>
      </Show>

      {/* Empty state */}
      <Show when={!workflow()}>
        <div class="empty-state bg-slate-800 rounded-lg p-8 text-center">
          <span class="text-4xl">🤖</span>
          <p class="text-slate-400 mt-2">No GUI workflow active</p>
        </div>
      </Show>
    </div>
  );
};

export default GuiWorkflowViewer;
