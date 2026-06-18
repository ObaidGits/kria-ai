import { Component, For, Show, createMemo, createSignal } from "solid-js";
import type { GuiCognitionLifecycle, GuiCognitionSessionState } from "../types/guiCognition";
import { deriveGuiCognitionSummary } from "../lib/guiCognitionSummary";

interface GuiCognitionPanelProps {
  session: GuiCognitionSessionState;
  onDismiss?: () => void;
  /**
   * Task 10.3: invoked by the visible Stop/Cancel control to abort the active
   * turn via the Task 1 cancel path. When omitted, the Stop control is hidden.
   */
  onStop?: () => void;
  /**
   * When false/undefined the "Developer details" accordion is hidden (clean
   * layman view). The app passes `appStore.developerMode()`; tests/other
   * callers that omit it keep the detailed view (back-compatible).
   */
  developerMode?: boolean;
}

const lifecycleLabel: Record<GuiCognitionLifecycle, string> = {
  idle: "Idle",
  observing: "Running",
  planning: "Running",
  resolving: "Running",
  safety: "Running",
  awaiting_approval: "Paused for approval",
  executing: "Running",
  verifying: "Running",
  blocked: "Blocked",
  completed: "Completed",
  failed: "Failed",
  cancelled: "Cancelled",
};

const lifecycleTone = (lifecycle: GuiCognitionLifecycle) => {
  if (lifecycle === "completed") return "success";
  if (lifecycle === "blocked" || lifecycle === "failed") return "danger";
  if (lifecycle === "awaiting_approval" || lifecycle === "cancelled") return "warning";
  if (lifecycle === "executing" || lifecycle === "verifying" || lifecycle === "observing") {
    return "active";
  }
  return "neutral";
};

const formatBool = (value: boolean | undefined) =>
  value === undefined ? "unknown" : value ? "available" : "unavailable";

const confidenceLabel = (value: number | undefined) => {
  if (value === undefined) return "unknown";
  return `${Math.round(value * 100)}%`;
};

const durationLabel = (value: number | undefined) => {
  if (value === undefined) return "unknown";
  return value < 1000 ? `${Math.round(value)}ms` : `${(value / 1000).toFixed(1)}s`;
};

const formatSourceLabel = (value: string | undefined) => {
  if (!value) return "unknown";
  return value
    .replace(/^kria_/, "KRIA ")
    .replaceAll("_", " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
};

const hashPreview = (value: string | undefined) => {
  if (!value) return "not provided";
  return value.length > 16 ? `${value.slice(0, 8)}...${value.slice(-6)}` : value;
};

/** Task 12: map a sub-goal status to a coarse visual tone. */
const subGoalTone = (status: string): string => {
  const s = status.toLowerCase();
  if (s === "verified" || s === "bridged") return "done";
  if (s === "failed" || s === "bridge_failed") return "failed";
  if (s === "pending") return "pending";
  return "active";
};

/** Task 12: a small status glyph for a sub-goal. */
const subGoalIcon = (status: string): string => {
  switch (subGoalTone(status)) {
    case "done":
      return "✓";
    case "failed":
      return "✗";
    case "pending":
      return "○";
    default:
      return "◔";
  }
};

/** Task 12: a short human label for a sub-goal status. */
const subGoalStatusLabel = (status: string): string => {
  const s = status.toLowerCase();
  if (s === "verified") return "done";
  if (s === "bridged") return "ran";
  if (s === "bridge_failed") return "failed";
  if (s === "pending") return "pending";
  if (s === "in_progress") return "working";
  return s;
};

const backendReadinessLabel = (backend: GuiCognitionSessionState["actionBackend"]) => {
  if (!backend) return "unknown";
  if (backend.canExecuteActions) return "ready";
  if (backend.haltKind === "startup_warming") return "warming up";
  if (backend.haltKind === "user_disabled" || backend.selectedBackend === "automation_disabled") {
    return "disabled";
  }
  return "blocked";
};

const perceptionSummaryLabel = (session: GuiCognitionSessionState) => {
  const observation = session.observation;
  if (session.blocker?.type === "turn") return "Observation blocked";
  if (observation.activeWindowReliability === "unavailable") return "Cannot identify active window";
  if (
    observation.accessibilitySourceStatus &&
    observation.accessibilitySourceStatus !== "healthy"
  ) {
    return "Screen observed with degraded sources";
  }
  if (observation.ocrAvailable === false || (observation.probeTimeoutCount ?? 0) > 0) {
    return "Screen observed with some source blockers";
  }
  return "Screen observed";
};

export const GuiCognitionPanel: Component<GuiCognitionPanelProps> = (props) => {
  const observation = createMemo(() => props.session.observation);
  const context = createMemo(() => props.session.context);
  const goalContract = createMemo(() => props.session.goalContract);
  const action = createMemo(() => props.session.currentAction ?? props.session.executionReceipt);
  const safety = createMemo(() => props.session.safetyDecision);
  const hitlDecision = createMemo(() => props.session.hitlDecision);
  const target = createMemo(() => props.session.target);
  const targetResolution = createMemo(() => props.session.targetResolution);
  const blocker = createMemo(() => props.session.blocker);
  const actionBackend = createMemo(() => props.session.actionBackend);
  const showPlan = createMemo(() => Boolean(props.session.planSteps.length || props.session.planSummary || props.session.goalSummary || props.session.goalContract));
  const summary = createMemo(() => deriveGuiCognitionSummary(props.session));
  const isTerminal = () =>
    props.session.lifecycle === "completed" ||
    props.session.lifecycle === "blocked" ||
    props.session.lifecycle === "failed" ||
    props.session.lifecycle === "cancelled";
  // Task 10.3: the turn is "active" (abortable) until it reaches a terminal
  // lifecycle. The Stop control is shown only while active.
  const isActive = () => props.session.lifecycle !== "idle" && !isTerminal();

  return (
    <section class="gui-cognition-panel" aria-label="GUI Cognition progress">
      <div class="gui-cognition-summary">
        <div class="gui-cognition-summary-top">
          <span class={`gui-cognition-badge gui-cognition-${summary().statusTone}`}>
            {summary().statusLabel}
          </span>
          <span class="gui-cognition-summary-headline">{summary().headline}</span>
          <Show when={props.onStop && isActive()}>
            <button
              type="button"
              class="gui-cognition-stop"
              onClick={() => props.onStop?.()}
              title="Stop the active GUI Cognition turn"
              aria-label="Stop the active GUI Cognition turn"
            >
              <svg width="12" height="12" viewBox="0 0 14 14" fill="currentColor" aria-hidden="true" style="vertical-align: middle; margin-right: 4px;">
                <rect x="1" y="1" width="12" height="12" rx="2" />
              </svg>
              Stop
            </button>
          </Show>
          <Show when={props.onDismiss && isTerminal()}>
            <button type="button" class="gui-cognition-dismiss" onClick={() => props.onDismiss?.()}>
              Dismiss
            </button>
          </Show>
        </div>
        <Show when={summary().facts.length > 0}>
          <div class="gui-cognition-facts">
            <For each={summary().facts}>
              {(fact) => (
                <span class="gui-cognition-fact">
                  <span class="gui-cognition-fact-label">{fact.label}</span>
                  <span class="gui-cognition-fact-value">{fact.value}</span>
                </span>
              )}
            </For>
          </div>
        </Show>
        <Show when={summary().warnings.length > 0}>
          <ul class="gui-cognition-summary-warnings">
            <For each={summary().warnings}>{(warning) => <li>{warning}</li>}</For>
          </ul>
        </Show>
        <Show when={summary().nextStep}>
          <div class="gui-cognition-summary-next">Next: {summary().nextStep}</div>
        </Show>
        {/* Collapsible "thinking": the Brain's sanitized rationale for the current
            step (streamed live as the plan summary). Collapsed by default to
            avoid cognitive overload; never raw chain-of-thought. */}
        <Show when={props.session.planSummary}>
          <details class="gui-cognition-thinking">
            <summary class="gui-cognition-thinking-summary">🧠 Thinking</summary>
            <p class="gui-cognition-thinking-text">{props.session.planSummary}</p>
          </details>
        </Show>
        {/* Task 12: live sub-goal plan with per-goal status. Each sub-goal shows
            a state badge (pending → verified/bridged/failed) so the user sees the
            plan progress in real time. */}
        <Show when={(props.session.subGoals?.length ?? 0) > 0}>
          <ol class="gui-cognition-subgoals" aria-label="Plan sub-goals">
            <For each={props.session.subGoals}>
              {(sg) => (
                <li class={`gui-cognition-subgoal gui-cognition-subgoal-${subGoalTone(sg.status)}`}>
                  <span class="gui-cognition-subgoal-icon" aria-hidden="true">
                    {subGoalIcon(sg.status)}
                  </span>
                  <span class="gui-cognition-subgoal-text">{sg.goal}</span>
                  <span class="gui-cognition-subgoal-status">{subGoalStatusLabel(sg.status)}</span>
                </li>
              )}
            </For>
          </ol>
        </Show>
        <Show when={props.session.recoveryNote}>
          <div class="gui-cognition-recovery-note" role="status">
            🔄 {props.session.recoveryNote}
          </div>
        </Show>
      </div>

      <Show when={props.developerMode ?? true}>
      <details class="gui-cognition-details">
        <summary class="gui-cognition-details-summary">Developer details</summary>
        <div class="gui-cognition-detail-region">
      <div class="gui-cognition-header">
        <div>
          <div class="gui-cognition-title-row">
            <span class="gui-cognition-title">GUI Cognition</span>
            <span class={`gui-cognition-badge gui-cognition-${lifecycleTone(props.session.lifecycle)}`}>
              {lifecycleLabel[props.session.lifecycle]}
            </span>
          </div>
          <div class="gui-cognition-subtitle">
            Active window: {observation().activeWindow || "unknown"}
            <Show when={observation().activeWindowSource}>
              {" "}· {observation().activeWindowSource}
              <Show when={observation().activeWindowConfidence !== undefined}>
                {" "}({confidenceLabel(observation().activeWindowConfidence)})
              </Show>
              <Show when={observation().activeWindowReliability}>
                {" "}· {observation().activeWindowReliability}
              </Show>
            </Show>
          </div>
        </div>
      </div>

      <div class="gui-cognition-grid">
        <div class="gui-cognition-section">
          <span class="gui-cognition-section-label">Observation</span>
          <div class="gui-cognition-primary">{perceptionSummaryLabel(props.session)}</div>
          <Show when={observation().activeWindowBlocker}>
            <div class="gui-cognition-muted">{observation().activeWindowBlocker}</div>
          </Show>
          <Show when={observation().activeWindowAuthoritySource || observation().gnomeBridgeStatus}>
            <div class="gui-cognition-muted">
              Authority {formatSourceLabel(observation().activeWindowAuthoritySource || observation().activeWindowSource)}
              <Show when={observation().activeWindowAuthorityStatus}>
                {" "}· {observation().activeWindowAuthorityStatus}
              </Show>
              <Show when={observation().activeWindowAuthorityConfidence !== undefined}>
                {" "}· {Math.round((observation().activeWindowAuthorityConfidence ?? 0) * 100)}%
              </Show>
              <Show when={observation().gnomeBridgeStatus}>
                {" "}· GNOME bridge {observation().gnomeBridgeStatus}
              </Show>
            </div>
          </Show>
          <Show when={(observation().activeWindowFailureChain?.length ?? 0) > 0}>
            <div class="gui-cognition-muted">
              Active fallback checked {observation().activeWindowFailureChain.map((attempt) => attempt.source).join(" -> ")}
            </div>
          </Show>
          <div class="gui-cognition-metrics">
            <span>Controls {observation().visibleControlCount ?? 0}</span>
            <span>Inputs {observation().textFieldCount ?? 0}</span>
            <span>Buttons {observation().buttonCount ?? 0}</span>
            <Show when={(observation().otherControlCount ?? 0) > 0}>
              <span>Other {observation().otherControlCount}</span>
            </Show>
          </div>
          <div class="gui-cognition-muted">
            Screenshot {formatBool(observation().screenshotAvailable)} · OCR {formatBool(observation().ocrAvailable)}
            <Show when={observation().ocrBlockCount !== undefined}>
              {" "}({observation().ocrBlockCount} blocks)
            </Show>
            <Show when={observation().ocrTrust}>
              {" "}· {observation().ocrTrust}
            </Show>
            <Show when={observation().ocrInjectionCount !== undefined}>
              {" "}· injections {observation().ocrInjectionCount}
            </Show>
          </div>
          <Show when={observation().ocrBlocker}>
            <div class="gui-cognition-muted">{observation().ocrBlocker}</div>
          </Show>
          <Show when={observation().ocrEngineStatus || observation().ocrWaitForScreenshotMs !== undefined || observation().ocrTotalMs !== undefined}>
            <div class="gui-cognition-muted">
              OCR engine {observation().ocrEngineSelected || "unknown"}
              <Show when={observation().ocrEngineStatus}>
                {" "}· {observation().ocrEngineStatus}
              </Show>
              <Show when={observation().ocrImageStatus}>
                {" "}· {observation().ocrImageStatus}
              </Show>
              <Show when={observation().ocrWaitForScreenshotMs !== undefined}>
                {" "}· waited {durationLabel(observation().ocrWaitForScreenshotMs)}
              </Show>
              <Show when={observation().ocrTotalMs !== undefined}>
                {" "}· total {durationLabel(observation().ocrTotalMs)}
              </Show>
            </div>
          </Show>
          <Show when={observation().ocrFastPath || observation().ocrCacheHit !== undefined || observation().ocrRoiCount !== undefined}>
            <div class="gui-cognition-muted">
              OCR fast path {observation().ocrFastPath || "unknown"}
              <Show when={observation().ocrCacheHit !== undefined}>
                {" "}· cache {observation().ocrCacheHit ? "hit" : "miss"}
              </Show>
              <Show when={observation().ocrRoiCount !== undefined}>
                {" "}· ROI {observation().ocrRoiCount}
              </Show>
              <Show when={observation().ocrChangedRegionCount !== undefined}>
                {" "}· changed regions {observation().ocrChangedRegionCount}
              </Show>
            </div>
          </Show>
          <div class="gui-cognition-muted">
            Accessibility {formatBool(observation().accessibilityAvailable)}
            <Show when={observation().accessibilityNodeCount !== undefined || observation().accessibilityControlCount !== undefined}>
              {" "}· {observation().accessibilityNodeCount ?? 0} nodes · {observation().accessibilityControlCount ?? observation().visibleAccessibleControlCount ?? 0} controls
            </Show>
          </div>
          <Show when={observation().trustedControlCount !== undefined || observation().notExecutableControlCount !== undefined}>
            <div class="gui-cognition-muted">
              Quality trusted {observation().trustedControlCount ?? 0}
              {" "}· partial {observation().partialControlCount ?? 0}
              {" "}· not executable {observation().notExecutableControlCount ?? 0}
            </div>
          </Show>
          <Show when={observation().accessibilitySourceStatus || observation().atspiSnapshotTotalMs !== undefined}>
            <div class="gui-cognition-muted">
              AT-SPI {observation().accessibilitySourceStatus || "unknown"}
              <Show when={observation().atspiSnapshotTotalMs !== undefined}>
                {" "}· snapshot {durationLabel(observation().atspiSnapshotTotalMs)}
              </Show>
              <Show when={(observation().atspiSkippedAppCount ?? 0) > 0}>
                {" "}· skipped apps {observation().atspiSkippedAppCount}
              </Show>
              <Show when={(observation().atspiOmittedNodeCount ?? 0) > 0}>
                {" "}· omitted nodes {observation().atspiOmittedNodeCount}
              </Show>
            </div>
          </Show>
          <Show when={observation().accessibilityOverallStatus || observation().accessibilityOverallConfidence !== undefined}>
            <div class="gui-cognition-muted">
              Accessibility health {observation().accessibilityOverallStatus || "unknown"}
              <Show when={observation().accessibilityOverallConfidence !== undefined}>
                {" "}· {confidenceLabel(observation().accessibilityOverallConfidence)}
              </Show>
              <Show when={(observation().accessibilityStaleNodeCount ?? 0) > 0}>
                {" "}· stale nodes {observation().accessibilityStaleNodeCount}
              </Show>
              <Show when={(observation().accessibilityTimeoutCount ?? 0) > 0}>
                {" "}· timeouts {observation().accessibilityTimeoutCount}
              </Show>
            </div>
          </Show>
          <Show when={observation().visualControlCount !== undefined}>
            <div class="gui-cognition-muted">
              Visual controls {observation().visualControlCount ?? 0}
              <Show when={observation().visualButtonLikeCount !== undefined}>
                {" "}· button-like {observation().visualButtonLikeCount}
              </Show>
              <Show when={observation().visualMatchedCount !== undefined}>
                {" "}· matched {observation().visualMatchedCount}
              </Show>
              <Show when={observation().visualUnmatchedCount !== undefined}>
                {" "}· unmatched {observation().visualUnmatchedCount}
              </Show>
            </div>
          </Show>
          <div class="gui-cognition-muted">
            Monitors {observation().monitorCount ?? 0} · DPI {formatBool(observation().dpiAvailable)} · Focus {observation().cursorFocusKnown ? "known" : "unknown"}
          </div>
          <Show when={observation().focusSource || observation().focusConfidence !== undefined || observation().focusedControlLabel}>
            <div class="gui-cognition-muted">
              Focus source {formatSourceLabel(observation().focusSource)}
              <Show when={observation().focusConfidence !== undefined}>
                {" "}· {confidenceLabel(observation().focusConfidence)}
              </Show>
              <Show when={observation().focusReliability}>
                {" "}· {observation().focusReliability}
              </Show>
              <Show when={observation().focusAdapterStatus}>
                {" "}· {observation().focusAdapterStatus}
              </Show>
              <Show when={observation().focusLatencyMs !== undefined}>
                {" "}· {durationLabel(observation().focusLatencyMs)}
              </Show>
              <Show when={observation().editableTargetKnown !== undefined}>
                {" "}· editable {observation().editableTargetKnown ? "known" : "unknown"}
              </Show>
            </div>
          </Show>
          <Show when={observation().terminalLike}>
            <div class="gui-cognition-warning">
              Terminal-like focus detected; normal blind GUI typing remains blocked.
            </div>
          </Show>
          <Show when={observation().observationTotalMs !== undefined}>
            <div class="gui-cognition-muted">
              Observation {durationLabel(observation().observationTotalMs)}
              <Show when={observation().screenshotDurationMs !== undefined}>
                {" "}· Screenshot {durationLabel(observation().screenshotDurationMs)}
              </Show>
              <Show when={observation().slowestProbe}>
                {" "}· Slowest probe: {observation().slowestProbe} {durationLabel(observation().slowestProbeMs)}
              </Show>
              <Show when={(observation().probeTimeoutCount ?? 0) > 0}>
                {" "}· Timeouts {observation().probeTimeoutCount}
              </Show>
            </div>
          </Show>
          <Show when={observation().cachePolicy}>
            <div class="gui-cognition-muted">
              Cache {observation().cacheHit ? "hit" : "miss"}
              <Show when={observation().cacheAgeMs !== undefined}>
                {" "}· age {durationLabel(observation().cacheAgeMs)}
              </Show>
              {" "}· {observation().cachePolicy}
            </div>
          </Show>
          <Show when={(observation().observationTotalMs ?? 0) > 2000}>
            <div class="gui-cognition-warning">
              Observation is slow; {observation().slowestProbe || "a perception probe"} is the bottleneck.
            </div>
          </Show>
          <Show when={observation().disabledControlCount !== undefined && observation().disabledControlCount! > 0}>
            <div class="gui-cognition-muted">
              Disabled/hidden controls {observation().disabledControlCount}
            </div>
          </Show>
          <Show when={observation().focusedWindow}>
            <div class="gui-cognition-muted">Focused window {observation().focusedWindow}</div>
          </Show>
          <Show when={observation().focusedApp || observation().focusedControlLabel || observation().focusedControlRole}>
            <div class="gui-cognition-muted">
              <Show when={observation().focusedApp}>
                Focused app {observation().focusedApp}
              </Show>
              <Show when={observation().focusedControlLabel}>
                {" "}· Control {observation().focusedControlLabel}
              </Show>
              <Show when={observation().focusedControlRole}>
                {" "}· {observation().focusedControlRole}
              </Show>
            </div>
          </Show>
          <Show when={observation().sourceBlockers.length > 0}>
            <ul class="gui-cognition-options">
              <For each={observation().sourceBlockers.slice(0, 3)}>
                {(blocker) => <li>{blocker}</li>}
              </For>
            </ul>
          </Show>
          <Show when={observation().accessibilityRemediation.length > 0}>
            <ul class="gui-cognition-options">
              <For each={observation().accessibilityRemediation.slice(0, 2)}>
                {(item) => <li>{item}</li>}
              </For>
            </ul>
          </Show>
          <Show when={observation().screenHashPrefix}>
            <div class="gui-cognition-muted">
              Screen hash {observation().screenHashPrefix}
            </div>
          </Show>
        </div>

        <Show when={context().contextId || context().status || context().trustedControlCount !== undefined}>
          <div class="gui-cognition-section">
            <span class="gui-cognition-section-label">Context</span>
            <div class="gui-cognition-primary">
              {context().status || "ready"} · {context().freshness || "fresh"}
            </div>
            <div class="gui-cognition-metrics">
              <span>Trusted {context().trustedControlCount ?? 0}</span>
              <span>Executable {context().executableControlCount ?? 0}</span>
              <span>Disabled/hidden {context().disabledOrHiddenCount ?? observation().disabledControlCount ?? 0}</span>
            </div>
            <div class="gui-cognition-muted">
              OCR {context().ocrUntrusted ? "untrusted" : "trust unknown"}
              <Show when={context().ocrInjectionCount !== undefined}>
                {" "}· injections {context().ocrInjectionCount}
              </Show>
              <Show when={context().redactionCount !== undefined}>
                {" "}· redactions {context().redactionCount}
              </Show>
            </div>
            <Show when={context().previousContextId}>
              <div class="gui-cognition-muted">
                Previous context {context().previousContextId}
              </div>
            </Show>
            <Show when={context().deltaSummary.length > 0}>
              <div class="gui-cognition-muted">
                Changed since previous: {context().deltaSummary.join(", ")}
              </div>
            </Show>
            <Show when={context().warnings.length > 0}>
              <ul class="gui-cognition-options">
                <For each={context().warnings.slice(0, 3)}>
                  {(warning) => <li>{warning}</li>}
                </For>
              </ul>
            </Show>
            <Show when={context().sourceBlockers.length > 0}>
              <ul class="gui-cognition-options">
                <For each={context().sourceBlockers.slice(0, 3)}>
                  {(blocker) => <li>{blocker}</li>}
                </For>
              </ul>
            </Show>
          </div>
        </Show>

        <Show when={showPlan()}>
          <div class="gui-cognition-section">
            <span class="gui-cognition-section-label">Plan</span>
            <div class="gui-cognition-primary">
              {goalContract()?.goalSummary || props.session.goalSummary || props.session.planSummary || "GUI task plan"}
            </div>
            <div class="gui-cognition-muted">
              Planner {props.session.plannerMode || goalContract()?.extractorMode || "deterministic"} · Status {props.session.planStatus || props.session.planValidationStatus || "pending"} · Risk {goalContract()?.riskLevel || props.session.riskLevel || "unknown"}
            </div>
            <Show when={props.session.planGoalActionType || props.session.planPromptHash}>
              <div class="gui-cognition-muted">
                <Show when={props.session.planGoalActionType}>
                  Goal action {props.session.planGoalActionType}
                </Show>
                <Show when={props.session.planPromptHash}>
                  {" "}· Prompt {props.session.planPromptHash}
                </Show>
              </div>
            </Show>
            <Show when={props.session.llmAttempted || props.session.planValidationStatus || props.session.plannerConfidence !== undefined}>
              <div class="gui-cognition-muted">
                <Show when={props.session.llmAttempted}>
                  LLM {props.session.llmStatus || "running"}
                </Show>
                <Show when={props.session.plannerConfidence !== undefined}>
                  {" "}· Plan confidence {confidenceLabel(props.session.plannerConfidence)}
                </Show>
                <Show when={props.session.planValidationStatus}>
                  {" "}· Validation {props.session.planValidationStatus}
                </Show>
              </div>
            </Show>
            <Show when={props.session.llmFailureReason}>
              <div class="gui-cognition-muted">
                {props.session.llmFailureReason}
              </div>
            </Show>
            <Show when={props.session.planReadinessStatus || props.session.planCanProceedToTargetResolution !== undefined || props.session.planCanExecute !== undefined}>
              <div class="gui-cognition-muted">
                <Show when={props.session.planReadinessStatus}>
                  Readiness {props.session.planReadinessStatus}
                </Show>
                <Show when={props.session.planCanProceedToTargetResolution !== undefined}>
                  {" "}· {props.session.planCanProceedToTargetResolution ? "ready for target resolution" : "not ready for target resolution"}
                </Show>
                <Show when={props.session.planCanExecute === false}>
                  {" "}· execution disabled
                </Show>
              </div>
            </Show>
            <Show when={props.session.planRequiresUserApproval}>
              <div class="gui-cognition-muted">Plan requires approval before any risky action.</div>
            </Show>
            <Show when={props.session.planValidationBlockerCount !== undefined || props.session.planValidationWarningCount !== undefined}>
              <div class="gui-cognition-muted">
                Validation blockers {props.session.planValidationBlockerCount ?? 0}
                {" "}· warnings {props.session.planValidationWarningCount ?? 0}
              </div>
            </Show>
            <Show when={props.session.planBlockedReasons.length > 0}>
              <ul class="gui-cognition-options">
                <For each={props.session.planBlockedReasons}>
                  {(reason) => <li>{reason}</li>}
                </For>
              </ul>
            </Show>
            <Show when={props.session.planWarnings.length > 0}>
              <ul class="gui-cognition-options">
                <For each={props.session.planWarnings.slice(0, 3)}>
                  {(warning) => <li>{warning}</li>}
                </For>
              </ul>
            </Show>
            <Show when={goalContract()?.actionType || goalContract()?.desiredFinalState}>
              <div class="gui-cognition-muted">
                <Show when={goalContract()?.actionType}>
                  Action {goalContract()?.actionType}
                </Show>
                <Show when={goalContract()?.desiredFinalState}>
                  {" "}· Final state {goalContract()?.desiredFinalState}
                </Show>
              </div>
            </Show>
            <Show when={goalContract()?.targetAppKind || goalContract()?.targetAppHint || goalContract()?.targetWindowHint || goalContract()?.targetControlHint}>
              <div class="gui-cognition-muted">
                Target
                <Show when={goalContract()?.targetAppKind}>
                  {" "}kind {goalContract()?.targetAppKind}
                </Show>
                <Show when={goalContract()?.targetAppHint}>
                  {" "}· app {goalContract()?.targetAppHint}
                </Show>
                <Show when={goalContract()?.targetWindowHint}>
                  {" "}· window {goalContract()?.targetWindowHint}
                </Show>
                <Show when={goalContract()?.targetControlHint}>
                  {" "}· control {goalContract()?.targetControlHint}
                </Show>
              </div>
            </Show>
            <Show when={goalContract()?.querySummary || goalContract()?.textPayloadSummary}>
              <div class="gui-cognition-muted">
                <Show when={goalContract()?.querySummary}>
                  Query {goalContract()?.querySummary}
                </Show>
                <Show when={goalContract()?.textPayloadSummary}>
                  {" "}· Text {goalContract()?.textPayloadSummary}
                </Show>
              </div>
            </Show>
            <Show when={goalContract()?.promptHash || goalContract()?.queryHash || goalContract()?.textPayloadHash}>
              <div class="gui-cognition-muted">
                <Show when={goalContract()?.promptHash}>
                  Prompt hash {goalContract()?.promptHash}
                </Show>
                <Show when={goalContract()?.queryHash}>
                  {" "}· query hash {goalContract()?.queryHash}
                </Show>
                <Show when={goalContract()?.textPayloadHash}>
                  {" "}· text hash {goalContract()?.textPayloadHash}
                </Show>
              </div>
            </Show>
            <Show when={goalContract()?.extractionConfidence !== undefined}>
              <div class="gui-cognition-muted">
                Goal confidence {confidenceLabel(goalContract()?.extractionConfidence)}
                <Show when={goalContract()?.requiresUserApproval}>
                  {" "}· approval required
                </Show>
              </div>
            </Show>
            <Show when={(goalContract()?.ambiguities.length ?? 0) > 0}>
              <ul class="gui-cognition-options">
                <For each={goalContract()?.ambiguities ?? []}>
                  {(ambiguity) => <li>{ambiguity.message}</li>}
                </For>
              </ul>
            </Show>
            <Show when={(goalContract()?.sourceEvidence.length ?? 0) > 0}>
              <ul class="gui-cognition-options">
                <For each={goalContract()?.sourceEvidence.slice(0, 4) ?? []}>
                  {(evidence) => (
                    <li>
                      Evidence {evidence.source || "source"}:{evidence.field ? ` ${evidence.field}` : ""}
                      {evidence.summary ? ` · ${evidence.summary}` : ""}
                      {evidence.confidence !== undefined ? ` · ${confidenceLabel(evidence.confidence)}` : ""}
                    </li>
                  )}
                </For>
              </ul>
            </Show>
            <Show when={props.session.planSteps.length > 0}>
              <ol class="gui-cognition-steps">
                <For each={props.session.planSteps.slice(0, 4)}>
                  {(step) => <li>{step}</li>}
                </For>
              </ol>
            </Show>
            <Show when={props.session.typedPlanSteps.length > 0}>
              <ol class="gui-cognition-steps">
                <For each={props.session.typedPlanSteps.slice(0, 6)}>
                  {(step) => (
                    <li>
                      {step.stepType || "Step"}: {step.summary || "planned step"}
                      <Show when={step.verificationStrategy}>
                        {" "}· verify {step.verificationStrategy}
                      </Show>
                      <Show when={step.allowedToExecute === false}>
                        {" "}· plan only
                      </Show>
                    </li>
                  )}
                </For>
              </ol>
            </Show>
            <Show when={props.session.planStepValidationResults.length > 0}>
              <ol class="gui-cognition-steps">
                <For each={props.session.planStepValidationResults.slice(0, 6)}>
                  {(step) => (
                    <li>
                      {step.stepType || "Step"} validation: {step.status || "unknown"}
                      <Show when={step.targetResolutionRequired}>
                        {" "}· target resolution required
                      </Show>
                      <Show when={step.verificationPresent === false}>
                        {" "}· verification missing
                      </Show>
                      <Show when={step.blocker}>
                        {" "}· {step.blocker}
                      </Show>
                    </li>
                  )}
                </For>
              </ol>
            </Show>
          </div>
        </Show>

        <Show when={actionBackend()?.selectedBackend || actionBackend()?.canExecuteActions !== undefined || (actionBackend()?.blockers.length ?? 0) > 0}>
          <div class="gui-cognition-section">
            <span class="gui-cognition-section-label">Action backend</span>
            <div class="gui-cognition-primary">
              {backendReadinessLabel(actionBackend())}
              <Show when={actionBackend()?.selectedBackend}>
                {" "}· {actionBackend()?.selectedBackend}
              </Show>
            </div>
            <div class="gui-cognition-muted">
              Session {actionBackend()?.sessionType || "unknown"} · automation {actionBackend()?.automationEnabled ? "enabled" : "disabled"}
            </div>
            <div class="gui-cognition-muted">
              Vision {actionBackend()?.visionSidecar || "unknown"} · uinput {actionBackend()?.uinputDaemon || "unknown"}
              <Show when={actionBackend()?.haltKind && actionBackend()?.haltKind !== "none"}>
                {" "}· {actionBackend()?.haltKind}
              </Show>
            </div>
            <div class="gui-cognition-muted">
              uinput {formatBool(actionBackend()?.uinputAvailable)} · ydotool {formatBool(actionBackend()?.ydotoolAvailable)} · xdotool {formatBool(actionBackend()?.xdotoolAvailable)}
            </div>
            <Show when={actionBackend()?.backendProbeStatus || actionBackend()?.backendSelectionReason}>
              <div class="gui-cognition-muted">
                Probe {actionBackend()?.backendProbeStatus || "unknown"}
                <Show when={actionBackend()?.backendSelectionReason}>
                  {" "}· {actionBackend()?.backendSelectionReason}
                </Show>
              </div>
            </Show>
            <Show when={actionBackend()?.sessionType === "wayland" && actionBackend()?.xdotoolAvailable && !actionBackend()?.xdotoolUsableForActions}>
              <div class="gui-cognition-muted">
                xdotool detected but not usable for Wayland actions
              </div>
            </Show>
            <Show when={actionBackend()?.uinputSocketPath || actionBackend()?.uinputSocketAccessible !== undefined || actionBackend()?.ydotoolUsableForActions !== undefined}>
              <div class="gui-cognition-muted">
                uinput socket {formatBool(actionBackend()?.uinputSocketAccessible)}
                <Show when={actionBackend()?.uinputSocketPath}>
                  {" "}· {actionBackend()?.uinputSocketPath}
                </Show>
                {" "}· ydotool actions {formatBool(actionBackend()?.ydotoolUsableForActions)}
                {" "}· xdotool actions {formatBool(actionBackend()?.xdotoolUsableForActions)}
              </div>
            </Show>
            <Show when={actionBackend()?.globalHaltEngaged}>
              <div class="gui-cognition-muted">
                Global safety halt active
                <Show when={actionBackend()?.haltReason}>
                  {": "}{actionBackend()?.haltReason}
                </Show>
              </div>
            </Show>
            <Show when={actionBackend()?.capabilities}>
              <div class="gui-cognition-muted">
                Capabilities focus {formatBool(actionBackend()?.capabilities?.focus_field)} · type {formatBool(actionBackend()?.capabilities?.fill_field)} · click {formatBool(actionBackend()?.capabilities?.click_control)}
              </div>
            </Show>
            <Show when={(actionBackend()?.releaseConditions.length ?? 0) > 0}>
              <ul class="gui-cognition-options">
                <For each={actionBackend()?.releaseConditions.slice(0, 3) ?? []}>
                  {(condition) => <li>{condition}</li>}
                </For>
              </ul>
            </Show>
            <Show when={(actionBackend()?.backendProbeErrors.length ?? 0) > 0}>
              <ul class="gui-cognition-options">
                <For each={actionBackend()?.backendProbeErrors.slice(0, 3) ?? []}>
                  {(probeError) => <li>{probeError}</li>}
                </For>
              </ul>
            </Show>
            <Show when={(actionBackend()?.blockers.length ?? 0) > 0}>
              <ul class="gui-cognition-options">
                <For each={actionBackend()?.blockers.slice(0, 3) ?? []}>
                  {(backendBlocker) => <li>{backendBlocker}</li>}
                </For>
              </ul>
            </Show>
          </div>
        </Show>

        <Show when={target()}>
          <div class="gui-cognition-section">
            <span class="gui-cognition-section-label">Target</span>
            <div class="gui-cognition-primary">
              {target()?.label || target()?.targetType || "resolved target"}
            </div>
            <div class="gui-cognition-muted">
              {target()?.role || target()?.targetType || "control"} · Confidence {confidenceLabel(target()?.confidence)}
            </div>
            <Show when={target()?.controlId || target()?.targetHash}>
              <div class="gui-cognition-muted">
                <Show when={target()?.controlId}>ID {target()?.controlId}</Show>
                <Show when={target()?.targetHash}> · Hash {target()?.targetHash?.slice(0, 12)}</Show>
              </div>
            </Show>
            <Show when={target()?.bounds}>
              <div class="gui-cognition-muted">
                Bounds {target()?.bounds?.x},{target()?.bounds?.y} {target()?.bounds?.width}x{target()?.bounds?.height}
              </div>
            </Show>
          </div>
        </Show>

        <Show when={targetResolution()}>
          <div class="gui-cognition-section">
            <span class="gui-cognition-section-label">Target Resolution</span>
            <div class="gui-cognition-primary">
              {targetResolution()?.status || "unknown"}
            </div>
            <div class="gui-cognition-muted">
              Confidence {confidenceLabel(targetResolution()?.confidence)}
              {" "}· {targetResolution()?.canProceedToSafetyGate ? "ready for safety gate" : "not ready for safety gate"}
              {" "}· execution {targetResolution()?.canExecute ? "enabled" : "disabled"}
            </div>
            <Show when={(targetResolution()?.candidateCount ?? 0) > 0}>
              <div class="gui-cognition-muted">
                Candidates {targetResolution()?.candidateCount}
              </div>
            </Show>
            <Show when={(targetResolution()?.ambiguityReasons.length ?? 0) > 0}>
              <ul class="gui-cognition-options">
                <For each={targetResolution()?.ambiguityReasons.slice(0, 3) ?? []}>
                  {(reason) => <li>{reason}</li>}
                </For>
              </ul>
            </Show>
            <Show when={(targetResolution()?.blockers.length ?? 0) > 0}>
              <ul class="gui-cognition-options">
                <For each={targetResolution()?.blockers.slice(0, 3) ?? []}>
                  {(reason) => <li>{reason}</li>}
                </For>
              </ul>
            </Show>
          </div>
        </Show>

        <Show when={safety() || props.session.pendingApproval}>
          <div class="gui-cognition-section">
            <span class="gui-cognition-section-label">Safety</span>
            <div class="gui-cognition-primary">
              {safety()?.status || (props.session.pendingApproval ? "RequiresApproval" : "unknown")}
            </div>
            <div class="gui-cognition-muted">
              Risk {safety()?.riskLevel || props.session.pendingApproval?.riskLevel || props.session.riskLevel || "unknown"}
              <Show when={safety()?.canExecute !== undefined}>
                {" "}· execution {safety()?.canExecute ? "enabled" : "disabled"}
              </Show>
              <Show when={safety()?.canAuthorizeStep7 !== undefined}>
                {" "}· Step 7 {safety()?.canAuthorizeStep7 ? "authorized" : "not authorized"}
              </Show>
            </div>
            <Show when={safety()?.proposalHash || props.session.pendingApproval?.proposalHash}>
              <div class="gui-cognition-muted">
                Proposal {hashPreview(safety()?.proposalHash || props.session.pendingApproval?.proposalHash)}
                {" "}· target {hashPreview(safety()?.targetHash || props.session.pendingApproval?.targetHash)}
              </div>
            </Show>
            <Show when={safety()?.actionType || props.session.pendingApproval?.actionType}>
              <div class="gui-cognition-muted">
                Action {safety()?.actionType || props.session.pendingApproval?.actionType}
                <Show when={safety()?.targetLabel || props.session.pendingApproval?.targetLabel}>
                  {" "}· target {safety()?.targetLabel || props.session.pendingApproval?.targetLabel}
                </Show>
              </div>
            </Show>
            <Show when={safety()?.expectedPostcondition || props.session.pendingApproval?.expectedPostcondition}>
              <div class="gui-cognition-muted">
                Expected: {safety()?.expectedPostcondition || props.session.pendingApproval?.expectedPostcondition}
              </div>
            </Show>
            <Show when={(safety()?.reasons.length ?? 0) > 0}>
              <div class="gui-cognition-muted">{safety()?.reasons.join("; ")}</div>
            </Show>
            <Show when={(safety()?.blockers?.length ?? 0) > 0}>
              <ul class="gui-cognition-options">
                <For each={safety()?.blockers?.slice(0, 3) ?? []}>
                  {(reason) => <li>{reason}</li>}
                </For>
              </ul>
            </Show>
            <Show when={props.session.pendingApproval?.reason}>
              <div class="gui-cognition-muted">{props.session.pendingApproval?.reason}</div>
            </Show>
            <Show when={hitlDecision()}>
              <div class="gui-cognition-muted">
                HITL decision: {hitlDecision()?.decision || "unknown"}
                {" "}· Step 7 {hitlDecision()?.canAuthorizeStep7 ? "authorized" : "not authorized"}
                {" "}· execution {hitlDecision()?.canExecute ? "enabled" : "disabled"}
              </div>
              <Show when={hitlDecision()?.decisionReason}>
                <div class="gui-cognition-muted">{hitlDecision()?.decisionReason}</div>
              </Show>
            </Show>
          </div>
        </Show>

        <Show when={props.session.workflow}>
          <div class="gui-cognition-section">
            <span class="gui-cognition-section-label">Workflow</span>
            <div class="gui-cognition-primary">
              {props.session.workflow?.status || "running"}
              <Show when={props.session.workflow?.stepCount !== undefined}>
                {" "}· step {(props.session.workflow?.currentStepIndex ?? 0) + 1}/
                {props.session.workflow?.stepCount}
              </Show>
              <Show when={props.session.workflow?.completedStepCount !== undefined}>
                {" "}· {props.session.workflow?.completedStepCount} done
              </Show>
            </div>
            <Show when={(props.session.workflow?.steps?.length ?? 0) > 0}>
              <ol class="gui-cognition-options">
                <For each={props.session.workflow?.steps ?? []}>
                  {(step) => (
                    <li>
                      {step.stepType || "step"}: {step.status}
                      <Show when={(step.blockers?.length ?? 0) > 0}>
                        {" "}· {step.blockers?.[0]}
                      </Show>
                    </li>
                  )}
                </For>
              </ol>
            </Show>
            <Show when={props.session.workflow?.blockedReason}>
              <div class="gui-cognition-muted">{props.session.workflow?.blockedReason}</div>
            </Show>
          </div>
        </Show>

        <Show when={props.session.checkpoint}>
          <div class="gui-cognition-section">
            <span class="gui-cognition-section-label">Checkpoint</span>
            <div class="gui-cognition-primary">
              {props.session.checkpoint?.resumeStatus || "saved"}
              <Show when={props.session.checkpoint?.checkpointHashPrefix}>
                {" "}· {props.session.checkpoint?.checkpointHashPrefix}
              </Show>
            </div>
            <Show when={props.session.checkpoint?.completedStepCount !== undefined}>
              <div class="gui-cognition-muted">
                {props.session.checkpoint?.completedStepCount} completed
                <Show when={props.session.checkpoint?.pendingStepId}>
                  {" "}· pending {props.session.checkpoint?.pendingStepId}
                </Show>
              </div>
            </Show>
            <Show when={props.session.checkpoint?.resumeExplanation}>
              <div class="gui-cognition-muted">{props.session.checkpoint?.resumeExplanation}</div>
            </Show>
            <Show when={(props.session.checkpoint?.invalidatedApprovals?.length ?? 0) > 0}>
              <div class="gui-cognition-muted">
                approval invalidated: {props.session.checkpoint?.invalidatedApprovals?.[0]}
              </div>
            </Show>
            <Show when={(props.session.checkpoint?.duplicateActionGuards?.length ?? 0) > 0}>
              <div class="gui-cognition-muted">
                duplicate blocked: {props.session.checkpoint?.duplicateActionGuards?.[0]}
              </div>
            </Show>
          </div>
        </Show>

        <Show when={action() || props.session.verification}>
          <div class="gui-cognition-section">
            <span class="gui-cognition-section-label">Execution</span>
            <Show when={action()}>
              <div class="gui-cognition-primary">
                {action()?.actionKind || "GUI action"} {action()?.status ? `· ${action()?.status}` : ""}
              </div>
              <div class="gui-cognition-muted">
                {action()?.target || action()?.resultSummary || action()?.safeErrorSummary || "Awaiting result"}
              </div>
              <Show when={action()?.backendUsed || action()?.proposalHash || action()?.targetHash}>
                <div class="gui-cognition-muted">
                  <Show when={action()?.backendUsed}>backend {action()?.backendUsed}</Show>
                  <Show when={action()?.proposalHash}>
                    {" "}· proposal {action()?.proposalHash?.slice(0, 10)}
                  </Show>
                  <Show when={action()?.targetHash}>
                    {" "}· target {action()?.targetHash?.slice(0, 10)}
                  </Show>
                </div>
              </Show>
              <Show when={action()?.recoveryHint}>
                <div class="gui-cognition-muted">{action()?.recoveryHint}</div>
              </Show>
            </Show>
            <Show when={props.session.verification}>
              <div class="gui-cognition-muted">
                Verification {props.session.verification?.status || "pending"}
                <Show when={props.session.verification?.verificationStrategy}>
                  {" "}· {props.session.verification?.verificationStrategy}
                </Show>
                <Show when={props.session.verification?.confidence !== undefined}>
                  {" "}· {confidenceLabel(props.session.verification?.confidence)}
                </Show>
              </div>
              <Show when={props.session.verification?.postStateSummary}>
                <div class="gui-cognition-muted">
                  state: {props.session.verification?.postStateSummary}
                </div>
              </Show>
              <Show when={props.session.verification?.matchedExpectedState === false}>
                <div class="gui-cognition-muted">
                  {props.session.verification?.safeErrorSummary ||
                    "Expected post-action state was not verified."}
                </div>
              </Show>
              <Show when={props.session.verification?.recoveryHint}>
                <div class="gui-cognition-muted">{props.session.verification?.recoveryHint}</div>
              </Show>
            </Show>
          </div>
        </Show>

        <Show when={props.session.recovery}>
          <div class="gui-cognition-section">
            <span class="gui-cognition-section-label">Recovery</span>
            <div class="gui-cognition-primary">
              {props.session.recovery?.status || "assessed"}
              <Show when={props.session.recovery?.failureKind}>
                {" "}· {props.session.recovery?.failureKind}
              </Show>
            </div>
            <Show when={props.session.recovery?.recoveryActionKind}>
              <div class="gui-cognition-muted">
                action: {props.session.recovery?.recoveryActionKind}
                <Show when={props.session.recovery?.maxRetryCount !== undefined}>
                  {" "}· retry {props.session.recovery?.retryCount ?? 0}/
                  {props.session.recovery?.maxRetryCount}
                </Show>
              </div>
            </Show>
            <Show when={props.session.recovery?.safeExplanation}>
              <div class="gui-cognition-muted">{props.session.recovery?.safeExplanation}</div>
            </Show>
            <Show when={(props.session.recovery?.blockers?.length ?? 0) > 0}>
              <ul class="gui-cognition-options">
                <For each={props.session.recovery?.blockers ?? []}>
                  {(reason) => <li>{reason}</li>}
                </For>
              </ul>
            </Show>
            <Show when={props.session.recovery?.nextRecommendedState}>
              <div class="gui-cognition-muted">
                next: {props.session.recovery?.nextRecommendedState}
                {" "}· retry original:{" "}
                {props.session.recovery?.canRetryOriginalAction ? "yes" : "no"}
              </div>
            </Show>
          </div>
        </Show>

        <Show when={blocker() || props.session.recoveryOptions.length > 0}>
          <div class="gui-cognition-section gui-cognition-blocker">
            <span class="gui-cognition-section-label">
              {props.session.recoveryOptions.length > 0 ? "Recovery" : "Blocker"}
            </span>
            <div class="gui-cognition-primary">{blocker()?.reason || "Recovery options available"}</div>
            <Show when={blocker()?.clarificationQuestion}>
              <div class="gui-cognition-muted">{blocker()?.clarificationQuestion}</div>
            </Show>
            <Show when={(blocker()?.options.length ?? 0) > 0 || props.session.recoveryOptions.length > 0}>
              <ul class="gui-cognition-options">
                <For each={(blocker()?.options.length ? blocker()?.options : props.session.recoveryOptions) ?? []}>
                  {(option) => <li>{option}</li>}
                </For>
              </ul>
            </Show>
          </div>
        </Show>
      </div>
        </div>
      </details>
      </Show>
    </section>
  );
};

export default GuiCognitionPanel;
