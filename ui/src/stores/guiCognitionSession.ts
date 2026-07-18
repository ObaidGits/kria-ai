import { createStore, produce, reconcile } from "solid-js/store";
import type {
  GuiCognitionActionBackendCapabilities,
  GuiCognitionBlockerState,
  GuiCognitionEnvelope,
  GuiCognitionEvent,
  GuiCognitionGoalAmbiguityState,
  GuiCognitionGoalEvidenceState,
  GuiCognitionLifecycle,
  GuiCognitionPlanStepValidationState,
  GuiCognitionProbeTiming,
  GuiCognitionSessionState,
  GuiCognitionSourceAttempt,
  GuiCognitionSubGoalState,
  GuiCognitionTargetCandidateState,
  GuiCognitionTargetState,
  GuiCognitionTypedPlanStepState,
} from "../types/guiCognition";

const SECRET_PATTERN =
  /((?:api[_-]?key|token|password|passwd|secret|credential)\s*[:=]\s*)[^\s,;]+/gi;
const RAW_INJECTION_PATTERN = /\b(ignore previous instructions|system prompt|developer message)\b/i;

function emptySession(): GuiCognitionSessionState {
  return {
    lifecycle: "idle",
    lastSequence: 0,
    observation: {
      probeTimings: [],
      sourceBlockers: [],
      accessibilityRemediation: [],
      activeWindowFallbackChain: [],
      activeWindowFailureChain: [],
    },
    context: {
      sourceConfidence: {},
      sourceTrust: {},
      deltaSummary: [],
      sourceBlockers: [],
      warnings: [],
    },
    planSteps: [],
    subGoals: [],
    typedPlanSteps: [],
    planSourceEvidence: [],
    planBlockedReasons: [],
    planWarnings: [],
    planStepValidationResults: [],
    recoveryOptions: [],
    actionBackend: {
      blockers: [],
      releaseConditions: [],
      backendProbeErrors: [],
    },
  };
}

const [state, setState] = createStore<{ active: GuiCognitionSessionState }>({
  active: emptySession(),
});

export const activeGuiCognitionSession = () =>
  state.active.lifecycle === "idle" ? null : state.active;

export const hasActiveGuiCognitionSession = () => activeGuiCognitionSession() !== null;

export const guiCognitionRoutingStatus = () => {
  const session = activeGuiCognitionSession();
  if (!session) return null;

  switch (session.lifecycle) {
    case "observing":
    case "planning":
    case "resolving":
    case "safety":
    case "executing":
    case "verifying":
      return "Running";
    case "awaiting_approval":
      return "Paused for approval";
    case "blocked":
      return "Blocked";
    case "failed":
      return "Failed";
    case "completed":
      return "Completed";
    case "cancelled":
      return "Cancelled";
    default:
      return null;
  }
};

export function clearGuiCognitionSession(): void {
  setState("active", reconcile(emptySession()));
}

/**
 * Task 10.3 (Requirement 16.6 / 21.1): mark the active GUI Cognition turn as
 * cancelled. The backend cancel (Task 1 `CancelToken` via
 * `cancel_gui_cognition_turn`) halts the loop before its next action; this is
 * the optimistic UI counterpart that flips the panel into a clear "cancelled"
 * state and clears the running indicator. It never downgrades an already-final
 * state (completed/failed/blocked) so a late cancel cannot rewrite history, and
 * it is a no-op when there is no active turn.
 */
export function markGuiCognitionCancelled(reason?: string): void {
  setState(
    produce((s) => {
      const lifecycle = s.active.lifecycle;
      if (
        lifecycle === "idle" ||
        lifecycle === "completed" ||
        lifecycle === "failed" ||
        lifecycle === "blocked" ||
        lifecycle === "cancelled"
      ) {
        return;
      }
      s.active.lifecycle = "cancelled";
      s.active.updatedAt = Date.now();
      s.active.blocker = {
        type: "turn",
        reason: sanitizeText(reason, "Turn cancelled by you."),
        options: [],
      };
    })
  );
}

function sanitizeText(value: unknown, fallback = ""): string {
  if (typeof value !== "string") return fallback;
  const compact = value.replace(/\s+/g, " ").trim();
  if (!compact) return fallback;
  if (RAW_INJECTION_PATTERN.test(compact)) return "[untrusted text redacted]";
  return compact.replace(SECRET_PATTERN, "$1[redacted]").slice(0, 240);
}

function sanitizeList(value: unknown, fallback: string[] = []): string[] {
  if (!Array.isArray(value)) return fallback;
  return value
    .map((item) => sanitizeText(item))
    .filter((item) => item.length > 0)
    .slice(0, 8);
}

function sanitizeBlockerRecord(value: unknown): string[] {
  if (!value || typeof value !== "object") return [];
  return Object.entries(value as Record<string, unknown>)
    .map(([key, blocker]) => {
      const safeBlocker = sanitizeText(blocker);
      return safeBlocker ? `${sanitizeText(key)}: ${safeBlocker}` : "";
    })
    .filter((item) => item.length > 0)
    .slice(0, 8);
}

function sanitizeStringRecord(value: unknown): Record<string, string> {
  if (!value || typeof value !== "object") return {};
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>)
      .map(([key, item]) => [sanitizeText(key), sanitizeText(item)])
      .filter(([key, item]) => key.length > 0 && item.length > 0)
      .slice(0, 12)
  );
}

function sanitizeNumberRecord(value: unknown): Record<string, number> {
  if (!value || typeof value !== "object") return {};
  const entries: [string, number][] = [];
  for (const [key, item] of Object.entries(value as Record<string, unknown>)) {
    const safeKey = sanitizeText(key);
    const safeValue = numberValue(item);
    if (safeKey.length > 0 && safeValue !== undefined) {
      entries.push([safeKey, safeValue]);
    }
    if (entries.length >= 12) break;
  }
  return Object.fromEntries(entries);
}

function deltaSummary(value: unknown): string[] {
  if (!value || typeof value !== "object") return [];
  const record = value as Record<string, unknown>;
  const changedSummary = sanitizeList(record.changed_summary);
  if (changedSummary.length > 0) return changedSummary;

  return Object.entries(record)
    .filter(([key, item]) => key.endsWith("_changed") && item === true)
    .map(([key]) => sanitizeText(key.replaceAll("_", " ")))
    .slice(0, 8);
}

function sanitizeGoalAmbiguities(value: unknown): GuiCognitionGoalAmbiguityState[] {
  if (!Array.isArray(value)) return [];
  const ambiguities: GuiCognitionGoalAmbiguityState[] = [];
  for (const item of value) {
    if (ambiguities.length >= 8) break;
    if (!item || typeof item !== "object") continue;
    const record = item as Record<string, unknown>;
    const message = sanitizeText(record.message);
    if (!message) continue;
    const ambiguity: GuiCognitionGoalAmbiguityState = { message };
    const kind = sanitizeText(record.kind);
    const field = sanitizeText(record.field);
    if (kind) ambiguity.kind = kind;
    if (field) ambiguity.field = field;
    ambiguities.push(ambiguity);
  }
  return ambiguities;
}

function sanitizeGoalEvidence(value: unknown): GuiCognitionGoalEvidenceState[] {
  if (!Array.isArray(value)) return [];
  const evidence: GuiCognitionGoalEvidenceState[] = [];
  for (const item of value) {
    if (evidence.length >= 8) break;
    if (!item || typeof item !== "object") continue;
    const record = item as Record<string, unknown>;
    const source = sanitizeText(record.source);
    const field = sanitizeText(record.field);
    const summary = sanitizeText(record.summary);
    if (!source && !field && !summary) continue;
    evidence.push({
      source,
      field,
      summary,
      confidence: numberValue(record.confidence),
    });
  }
  return evidence;
}

function sanitizeTypedPlanSteps(value: unknown): GuiCognitionTypedPlanStepState[] {
  if (!Array.isArray(value)) return [];
  return value
    .map((item): GuiCognitionTypedPlanStepState | null => {
      if (!item || typeof item !== "object") return null;
      const record = item as Record<string, unknown>;
      return {
        stepId: sanitizeText(record.step_id),
        stepType: sanitizeText(record.step_type),
        summary: sanitizeText(record.summary),
        targetAppHint: sanitizeText(record.target_app_hint),
        targetWindowHint: sanitizeText(record.target_window_hint),
        targetControlHint: sanitizeText(record.target_control_hint),
        textPayloadSummary: sanitizeText(record.text_payload_summary),
        textPayloadHash: sanitizeText(record.text_payload_hash),
        expectedPrecondition: sanitizeText(record.expected_precondition),
        expectedPostcondition: sanitizeText(record.expected_postcondition),
        verificationStrategy: sanitizeText(record.verification_strategy),
        riskLevel: sanitizeText(record.risk_level),
        requiresApproval: booleanValue(record.requires_approval),
        allowedToExecute: booleanValue(record.allowed_to_execute),
        confidence: numberValue(record.confidence),
        reason: sanitizeText(record.reason),
      } satisfies GuiCognitionTypedPlanStepState;
    })
    .filter((item): item is GuiCognitionTypedPlanStepState => item !== null)
    .slice(0, 8);
}

function sanitizePlanStepValidationResults(value: unknown): GuiCognitionPlanStepValidationState[] {
  if (!Array.isArray(value)) return [];
  return value
    .map((item): GuiCognitionPlanStepValidationState | null => {
      if (!item || typeof item !== "object") return null;
      const record = item as Record<string, unknown>;
      return {
        stepId: sanitizeText(record.step_id),
        stepType: sanitizeText(record.step_type),
        status: sanitizeText(record.status),
        riskLevel: sanitizeText(record.risk_level),
        requiresApproval: booleanValue(record.requires_approval),
        targetResolutionRequired: booleanValue(record.target_resolution_required),
        targetAvailable: booleanValue(record.target_available),
        verificationPresent: booleanValue(record.verification_present),
        preconditionStatus: sanitizeText(record.precondition_status),
        postconditionStatus: sanitizeText(record.postcondition_status),
        blocker: sanitizeText(record.blocker),
        confidence: numberValue(record.confidence),
      } satisfies GuiCognitionPlanStepValidationState;
    })
    .filter((item): item is GuiCognitionPlanStepValidationState => item !== null)
    .slice(0, 8);
}

function sanitizeBounds(value: unknown): Record<string, number> | undefined {
  if (!value || typeof value !== "object") return undefined;
  const record = value as Record<string, unknown>;
  const x = numberValue(record.x);
  const y = numberValue(record.y);
  const width = numberValue(record.width);
  const height = numberValue(record.height);
  if ([x, y, width, height].some((item) => item === undefined)) return undefined;
  return { x: x!, y: y!, width: width!, height: height! };
}

function sanitizeTargetRecord(value: unknown): GuiCognitionTargetState | undefined {
  if (!value || typeof value !== "object") return undefined;
  const record = value as Record<string, unknown>;
  const target: GuiCognitionTargetState = {
    label: sanitizeText(record.label),
    role: sanitizeText(record.role),
    targetType: sanitizeText(record.target_kind ?? record.target_type),
    controlId: sanitizeText(record.control_id),
    targetHash: sanitizeText(record.target_hash),
    bounds: sanitizeBounds(record.bounds),
    enabled: booleanValue(record.enabled),
    visible: booleanValue(record.visible),
    focused: booleanValue(record.focused),
    source: sanitizeText(record.source),
  };
  return Object.values(target).some((item) => item !== undefined && item !== "")
    ? target
    : undefined;
}

function sanitizeTargetCandidates(value: unknown): GuiCognitionTargetCandidateState[] {
  if (!Array.isArray(value)) return [];
  return value
    .map((item): GuiCognitionTargetCandidateState | null => {
      if (!item || typeof item !== "object") return null;
      const record = item as Record<string, unknown>;
      return {
        candidateId: sanitizeText(record.candidate_id),
        controlId: sanitizeText(record.control_id),
        targetHash: sanitizeText(record.target_hash),
        label: sanitizeText(record.label),
        role: sanitizeText(record.role),
        bounds: sanitizeBounds(record.bounds),
        visible: booleanValue(record.visible),
        enabled: booleanValue(record.enabled),
        focused: booleanValue(record.focused),
        quality: sanitizeText(record.quality),
        sources: sanitizeList(record.sources),
        confidence: numberValue(record.final_confidence ?? record.confidence),
        rejectionReason: sanitizeText(record.rejection_reason),
      } satisfies GuiCognitionTargetCandidateState;
    })
    .filter((item): item is GuiCognitionTargetCandidateState => item !== null)
    .slice(0, 8);
}

function sanitizeProbeTimings(value: unknown): GuiCognitionProbeTiming[] {
  if (!Array.isArray(value)) return [];
  const timings: GuiCognitionProbeTiming[] = [];
  for (const item of value) {
    if (timings.length >= 12) break;
    if (!item || typeof item !== "object") continue;
    const record = item as Record<string, unknown>;
    const probeName = sanitizeText(record.probe_name);
    const durationMs = numberValue(record.duration_ms);
    if (!probeName || durationMs === undefined) continue;
    timings.push({
      probe_name: probeName,
      duration_ms: durationMs,
      status: sanitizeText(record.status),
      source: sanitizeText(record.source),
      cache_hit: booleanValue(record.cache_hit),
      blocker_kind: sanitizeText(record.blocker_kind),
    });
  }
  return timings;
}

function sanitizeSourceAttempts(value: unknown): GuiCognitionSourceAttempt[] {
  if (!Array.isArray(value)) return [];
  const attempts: GuiCognitionSourceAttempt[] = [];
  for (const item of value) {
    if (attempts.length >= 8) break;
    if (!item || typeof item !== "object") continue;
    const record = item as Record<string, unknown>;
    const source = sanitizeText(record.source);
    if (!source) continue;
    attempts.push({
      source,
      status: sanitizeText(record.status),
      reliability: sanitizeText(record.reliability),
      reason: sanitizeText(record.reason),
    });
  }
  return attempts;
}

function numberValue(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function booleanValue(value: unknown): boolean | undefined {
  return typeof value === "boolean" ? value : undefined;
}

function sanitizeCapabilities(value: unknown): Partial<GuiCognitionActionBackendCapabilities> | undefined {
  if (!value || typeof value !== "object") return undefined;
  const record = value as Record<string, unknown>;
  const result: Partial<GuiCognitionActionBackendCapabilities> = {};
  for (const key of [
    "observe",
    "focus_field",
    "fill_field",
    "click_control",
    "post_action_observe",
    "verify",
    "recovery_focus",
    "recovery_modal",
  ] as const) {
    const item = booleanValue(record[key]);
    if (item !== undefined) result[key] = item;
  }
  return result;
}

function eventType(event: GuiCognitionEvent | Record<string, unknown> | undefined): string {
  return typeof event?.type === "string" ? event.type : "";
}

function shouldAcceptEnvelope(envelope: GuiCognitionEnvelope): boolean {
  if (envelope.version !== 1) return false;
  if (!envelope.turn_id || !envelope.workflow_id || !Number.isFinite(envelope.sequence)) {
    return false;
  }

  const incomingType = eventType(envelope.event);
  if (incomingType === "TurnStarted") return true;
  if (!state.active.turnId) return false;
  if (envelope.turn_id !== state.active.turnId) return false;
  return envelope.sequence > state.active.lastSequence;
}

function lifecycleForTerminal(status: string | undefined): GuiCognitionLifecycle {
  const normalized = (status ?? "").toLowerCase();
  if (normalized.includes("fail")) return "failed";
  if (normalized.includes("block") || normalized.includes("clarification")) return "blocked";
  if (normalized.includes("approval")) return "awaiting_approval";
  return "completed";
}

function setBlocker(blocker: GuiCognitionBlockerState): void {
  setState(
    produce((s) => {
      s.active.blocker = blocker;
      s.active.lifecycle = "blocked";
    })
  );
}

export function handleGuiCognitionEvent(envelope: GuiCognitionEnvelope): void {
  if (!shouldAcceptEnvelope(envelope)) return;

  const event = envelope.event;
  const now = envelope.timestamp_ms || Date.now();

  if (event.type === "TurnStarted") {
    setState("active", reconcile({
      ...emptySession(),
      lifecycle: "planning",
      sessionId: envelope.session_id,
      turnId: envelope.turn_id,
      workflowId: envelope.workflow_id,
      lastSequence: envelope.sequence,
      startedAt: now,
      updatedAt: now,
    }));
    return;
  }

  setState(
    produce((s) => {
      s.active.lastSequence = envelope.sequence;
      s.active.updatedAt = now;
    })
  );

  switch (event.type) {
    case "RouteConfirmed":
      setState(
        produce((s) => {
          s.active.routePath = sanitizeText(event.path);
          s.active.llmToolLoop = booleanValue(event.llm_tool_loop);
        })
      );
      break;

    case "ObservationStarted":
      setState("active", "lifecycle", "observing");
      break;

    case "ObservationBlocked":
      setBlocker({
        type: "turn",
        reason: sanitizeText(event.reason, "Observation blocked"),
        options: sanitizeBlockerRecord(event.blockers),
      });
      break;

    case "ObservationCompleted":
      setState(
        produce((s) => {
          s.active.lifecycle = "planning";
          s.active.observation.observationId = sanitizeText(event.observation_id);
          s.active.observation.activeWindow = sanitizeText(event.active_window, "unknown");
          s.active.observation.activeWindowSource = sanitizeText(event.active_window_source);
          s.active.observation.activeWindowConfidence = numberValue(event.active_window_confidence);
          s.active.observation.activeWindowReliability = sanitizeText(event.active_window_reliability);
          s.active.observation.activeWindowBlocker = sanitizeText(event.active_window_blocker);
          s.active.observation.activeWindowAuthoritySource = sanitizeText(event.active_window_authority_source);
          s.active.observation.activeWindowAuthorityConfidence = numberValue(event.active_window_authority_confidence);
          s.active.observation.activeWindowAuthorityStatus = sanitizeText(event.active_window_authority_status);
          s.active.observation.gnomeBridgeStatus = sanitizeText(event.gnome_bridge_status);
          s.active.observation.activeWindowApp = sanitizeText(event.active_window_app);
          s.active.observation.activeWindowAppId = sanitizeText(event.active_window_app_id);
          s.active.observation.activeWindowPid = numberValue(event.active_window_pid);
          s.active.observation.activeWindowWorkspace = numberValue(event.active_window_workspace);
          s.active.observation.activeWindowMonitor = numberValue(event.active_window_monitor);
          s.active.observation.activeWindowFullscreen = booleanValue(event.active_window_fullscreen);
          s.active.observation.activeWindowMinimized = booleanValue(event.active_window_minimized);
          s.active.observation.activeWindowFallbackChain = sanitizeSourceAttempts(event.active_window_fallback_chain);
          s.active.observation.activeWindowFailureChain = sanitizeSourceAttempts(event.active_window_failure_chain);
          s.active.observation.visibleAppCount = numberValue(event.visible_app_count);
          s.active.observation.visibleControlCount = numberValue(event.visible_control_count);
          s.active.observation.visibleAccessibleControlCount = numberValue(event.visible_accessible_control_count);
          s.active.observation.disabledControlCount = numberValue(event.disabled_control_count);
          s.active.observation.hiddenControlCount = numberValue(event.hidden_control_count);
          s.active.observation.trustedControlCount = numberValue(event.trusted_control_count);
          s.active.observation.partialControlCount = numberValue(event.partial_control_count);
          s.active.observation.notExecutableControlCount = numberValue(event.not_executable_control_count);
          s.active.observation.textFieldCount = numberValue(event.text_field_count);
          s.active.observation.buttonCount = numberValue(event.button_count);
          s.active.observation.dialogCount = numberValue(event.dialog_count);
          s.active.observation.otherControlCount = numberValue(event.other_control_count);
          s.active.observation.ocrAvailable = booleanValue(event.ocr_available);
          s.active.observation.ocrBlockCount = numberValue(event.ocr_block_count);
          s.active.observation.ocrTrust = sanitizeText(event.ocr_trust);
          s.active.observation.ocrWaitForScreenshotMs = numberValue(event.ocr_wait_for_screenshot_ms);
          s.active.observation.ocrEngineSelected = sanitizeText(event.ocr_engine_selected);
          s.active.observation.ocrEngineStatus = sanitizeText(event.ocr_engine_status);
          s.active.observation.ocrImageStatus = sanitizeText(event.ocr_image_status);
          s.active.observation.ocrTotalMs = numberValue(event.ocr_total_ms);
          s.active.observation.ocrFastPath = sanitizeText(event.ocr_fast_path);
          s.active.observation.ocrCacheHit = booleanValue(event.ocr_cache_hit);
          s.active.observation.ocrRoiCount = numberValue(event.ocr_roi_count);
          s.active.observation.ocrChangedRegionCount = numberValue(event.ocr_changed_region_count);
          s.active.observation.ocrColdStartMs = numberValue(event.ocr_cold_start_ms);
          s.active.observation.ocrWarmStartMs = numberValue(event.ocr_warm_start_ms);
          s.active.observation.ocrBenchmarkSummary = sanitizeText(event.ocr_benchmark_summary);
          s.active.observation.ocrInjectionCount = numberValue(event.ocr_injection_count);
          s.active.observation.ocrBlocker = sanitizeText(event.ocr_blocker);
          s.active.observation.accessibilityAvailable = booleanValue(event.accessibility_available);
          s.active.observation.accessibilitySourceStatus = sanitizeText(event.accessibility_source_status);
          s.active.observation.accessibilityOverallStatus = sanitizeText(event.accessibility_overall_status);
          s.active.observation.accessibilityOverallConfidence = numberValue(event.accessibility_overall_confidence);
          s.active.observation.accessibilityStaleNodeCount = numberValue(event.accessibility_stale_node_count);
          s.active.observation.accessibilityTimeoutCount = numberValue(event.accessibility_timeout_count);
          s.active.observation.accessibilityCacheHitCount = numberValue(event.accessibility_cache_hit_count);
          s.active.observation.accessibilityStaleCacheRejectedCount = numberValue(event.accessibility_stale_cache_rejected_count);
          s.active.observation.accessibilityNodeCount = numberValue(event.accessibility_node_count);
          s.active.observation.accessibilityControlCount = numberValue(event.accessibility_control_count);
          s.active.observation.atspiSnapshotTotalMs = numberValue(event.atspi_snapshot_total_ms);
          s.active.observation.atspiSkippedAppCount = numberValue(event.atspi_skipped_app_count);
          s.active.observation.atspiOmittedNodeCount = numberValue(event.atspi_omitted_node_count);
          s.active.observation.accessibilityRemediation = sanitizeList(event.accessibility_remediation);
          s.active.observation.screenshotAvailable = booleanValue(event.screenshot_available);
          s.active.observation.screenshotStatus = sanitizeText(event.screenshot_status);
          s.active.observation.screenshotCaptureMs = numberValue(event.screenshot_capture_ms);
          s.active.observation.screenshotDurationMs = numberValue(event.screenshot_duration_ms);
          s.active.observation.screenHashPrefix = sanitizeText(event.screen_hash_prefix);
          s.active.observation.monitorCount = numberValue(event.monitor_count);
          s.active.observation.dpiAvailable = booleanValue(event.dpi_available);
          s.active.observation.cursorFocusKnown = booleanValue(event.cursor_focus_known);
          s.active.observation.focusedWindow = sanitizeText(event.focused_window);
          s.active.observation.focusedApp = sanitizeText(event.focused_app);
          s.active.observation.focusedControlId = sanitizeText(event.focused_control_id);
          s.active.observation.focusedControlLabel = sanitizeText(event.focused_control_label);
          s.active.observation.focusedControlRole = sanitizeText(event.focused_control_role);
          if (event.focused_control_bounds && typeof event.focused_control_bounds === "object") {
            s.active.observation.focusedControlBounds = event.focused_control_bounds as Record<string, unknown>;
          }
          s.active.observation.textCursorKnown = booleanValue(event.text_cursor_known);
          s.active.observation.editableTargetKnown = booleanValue(event.editable_target_known);
          s.active.observation.terminalLike = booleanValue(event.terminal_like);
          s.active.observation.focusSource = sanitizeText(event.focus_source);
          s.active.observation.focusConfidence = numberValue(event.focus_confidence);
          s.active.observation.focusReliability = sanitizeText(event.focus_reliability);
          s.active.observation.focusAdapterStatus = sanitizeText(event.focus_adapter_status);
          s.active.observation.focusLatencyMs = numberValue(event.focus_latency_ms);
          s.active.observation.focusFailureChain = sanitizeSourceAttempts(event.focus_failure_chain);
          s.active.observation.observationTotalMs = numberValue(event.observation_total_ms);
          s.active.observation.slowestProbe = sanitizeText(event.slowest_probe);
          s.active.observation.slowestProbeMs = numberValue(event.slowest_probe_ms);
          s.active.observation.probeTimeoutCount = numberValue(event.probe_timeout_count);
          s.active.observation.probeTimings = sanitizeProbeTimings(event.probe_timings);
          s.active.observation.cacheHit = booleanValue(event.cache_hit);
          s.active.observation.cacheAgeMs = numberValue(event.cache_age_ms);
          s.active.observation.cachePolicy = sanitizeText(event.cache_policy);
          s.active.observation.freshness = sanitizeText(event.freshness);
          s.active.observation.sourceBlockers = sanitizeBlockerRecord(event.source_blockers);
          s.active.observation.executableControlCount = numberValue(event.executable_control_count);
          s.active.observation.visualControlCount = numberValue(event.visual_control_count);
          if (event.visual_control_summary && typeof event.visual_control_summary === "object") {
            const summary = event.visual_control_summary as Record<string, unknown>;
            s.active.observation.visualButtonLikeCount = numberValue(summary.button_like);
            s.active.observation.visualMatchedCount = numberValue(summary.matched);
            s.active.observation.visualUnmatchedCount = numberValue(summary.unmatched);
          }
        })
      );
      break;

    case "ContextBuilt":
      setState(
        produce((s) => {
          s.active.observation.contextId = sanitizeText(event.context_id);
          s.active.context = {
            contextId: sanitizeText(event.context_id),
            observationId: sanitizeText(event.observation_id),
            previousContextId: sanitizeText(event.previous_context_id),
            previousObservationId: sanitizeText(event.previous_observation_id),
            activeWindow: sanitizeText(event.active_window),
            status: sanitizeText(event.status, "ready"),
            freshness: sanitizeText(event.freshness, "fresh"),
            screenHashPrefix: sanitizeText(event.screen_hash_prefix),
            trustedControlCount: numberValue(event.trusted_control_count),
            executableControlCount: numberValue(event.executable_control_count),
            disabledOrHiddenCount: numberValue(event.disabled_or_hidden_count),
            ocrUntrusted: booleanValue(event.ocr_untrusted),
            ocrInjectionCount: numberValue(event.ocr_injection_count),
            redactionCount: numberValue(event.redaction_count),
            sourceConfidence: sanitizeNumberRecord(event.source_confidence),
            sourceTrust: sanitizeStringRecord(event.source_trust),
            deltaSummary: deltaSummary(event.delta),
            sourceBlockers: sanitizeList(event.source_blockers),
            warnings: sanitizeList(event.warnings),
            focus: event.focus && typeof event.focus === "object" ? event.focus as Record<string, unknown> : undefined,
            accessibilityHealth: event.accessibility_health && typeof event.accessibility_health === "object" ? event.accessibility_health as Record<string, unknown> : undefined,
            visualControls: event.visual_controls && typeof event.visual_controls === "object" ? event.visual_controls as Record<string, unknown> : undefined,
            ocrPerformance: event.ocr_performance && typeof event.ocr_performance === "object" ? event.ocr_performance as Record<string, unknown> : undefined,
          };
        })
      );
      break;

    case "GoalContractCreated":
      setState(
        produce((s) => {
          s.active.lifecycle = "planning";
          s.active.goalSummary = sanitizeText(event.goal_summary);
          s.active.intentKind = sanitizeText(event.intent_kind);
          s.active.riskLevel = sanitizeText(event.risk_level);
          const ambiguities = sanitizeGoalAmbiguities(event.ambiguities);
          s.active.goalContract = {
            contractId: sanitizeText(event.contract_id),
            observationId: sanitizeText(event.observation_id),
            contextId: sanitizeText(event.context_id),
            goalSummary: sanitizeText(event.goal_summary),
            promptHash: sanitizeText(event.prompt_hash),
            intentKind: sanitizeText(event.intent_kind),
            actionType: sanitizeText(event.action_type),
            targetAppKind: sanitizeText(event.target_app_kind),
            targetAppHint: sanitizeText(event.target_app_hint),
            targetWindowHint: sanitizeText(event.target_window_hint),
            targetControlHint: sanitizeText(event.target_control_hint),
            querySummary: sanitizeText(event.query_summary),
            queryHash: sanitizeText(event.query_hash),
            textPayloadSummary: sanitizeText(event.text_payload_summary),
            textPayloadHash: sanitizeText(event.text_payload_hash),
            desiredFinalState: sanitizeText(event.desired_final_state),
            riskLevel: sanitizeText(event.risk_level),
            requiresUserApproval: booleanValue(event.requires_user_approval),
            ambiguityCount: numberValue(event.ambiguity_count),
            ambiguities,
            sourceEvidence: sanitizeGoalEvidence(event.source_evidence),
            extractionConfidence: numberValue(event.extraction_confidence),
            extractorMode: sanitizeText(event.extractor_mode),
          };
        })
      );
      break;

    case "LlmPlanningStarted":
      setState(
        produce((s) => {
          s.active.lifecycle = "planning";
          s.active.llmAttempted = true;
          s.active.llmStatus = "running";
          s.active.plannerMode = sanitizeText(event.planner_mode, "llm_assisted");
        })
      );
      break;

    case "LlmPlanningCompleted":
      setState(
        produce((s) => {
          s.active.lifecycle = "planning";
          s.active.llmAttempted = true;
          s.active.llmStatus = sanitizeText(event.status, "completed");
          s.active.plannerConfidence = numberValue(event.confidence);
          s.active.riskLevel = sanitizeText(event.risk_level, s.active.riskLevel);
        })
      );
      break;

    case "LlmPlanningFailed":
      setState(
        produce((s) => {
          s.active.lifecycle = "planning";
          s.active.llmAttempted = true;
          s.active.llmStatus = sanitizeText(event.status, "failed");
          s.active.llmFailureReason = sanitizeText(
            event.reason,
            "LLM planner unavailable; deterministic fallback used."
          );
          s.active.plannerMode = "deterministic_fallback";
        })
      );
      break;

    case "PlanCreated":
      setState(
        produce((s) => {
          s.active.lifecycle = "planning";
          s.active.planId = sanitizeText(event.plan_id);
          s.active.planGoalContractId = sanitizeText(event.goal_contract_id);
          s.active.planContextId = sanitizeText(event.context_id);
          s.active.planPromptHash = sanitizeText(event.prompt_hash);
          s.active.planGoalActionType = sanitizeText(event.goal_action_type);
          s.active.planStatus = sanitizeText(event.plan_status);
          s.active.planAmbiguityCount = numberValue(event.ambiguity_count);
          s.active.planSummary = sanitizeText(event.summary);
          s.active.plannerMode = sanitizeText(event.planner_mode);
          s.active.riskLevel = sanitizeText(event.risk_level, s.active.riskLevel);
          s.active.plannerConfidence = numberValue(event.confidence) ?? s.active.plannerConfidence;
          s.active.planRequiresUserApproval = booleanValue(event.requires_user_approval);
          s.active.planSteps = sanitizeList(event.steps);
          // Task 12: seed the live sub-goal tracker from the plan steps (each
          // starts "pending"; SubGoalUpdated flips them to verified/bridged/etc.).
          const seededGoals = sanitizeList(event.steps);
          if (seededGoals.length > 0) {
            s.active.subGoals = seededGoals.map((goal, index) => ({
              index,
              total: seededGoals.length,
              goal,
              status: "pending",
            }));
          }
          s.active.typedPlanSteps = sanitizeTypedPlanSteps(event.typed_steps);
          s.active.planSourceEvidence = sanitizeGoalEvidence(event.source_evidence);
          if (Array.isArray(event.validation_errors) && event.validation_errors.length > 0) {
            s.active.planBlockedReasons = sanitizeList(event.validation_errors);
          }
        })
      );
      break;

    case "SubGoalUpdated":
      setState(
        produce((s) => {
          const index = numberValue(event.index);
          const total = numberValue(event.total);
          const goal = sanitizeText(event.goal);
          const status = sanitizeText(event.status, "in_progress") ?? "in_progress";
          if (index === undefined) return;
          if (!Array.isArray(s.active.subGoals)) s.active.subGoals = [];
          // Ensure the slot exists (the plan may not have been seeded yet).
          const existing = s.active.subGoals.find((g) => g.index === index);
          if (existing) {
            existing.status = status;
            if (goal) existing.goal = goal;
            if (total !== undefined) existing.total = total;
          } else {
            s.active.subGoals.push({
              index,
              total: total ?? index + 1,
              goal: goal ?? `Step ${index + 1}`,
              status,
            });
            s.active.subGoals.sort((a, b) => a.index - b.index);
          }
          // A sub-goal advancing clears any stale recovery note.
          if (status === "verified") s.active.recoveryNote = undefined;
        })
      );
      break;

    case "RecoveryAttempted":
      setState(
        produce((s) => {
          const rung = sanitizeText(event.rung, "recovery");
          // Show recovery as benign in-progress, never a hard failure (R11.x/11.4).
          s.active.recoveryNote =
            rung === "grounded_reobserve"
              ? "Looking closer at the screen to recover…"
              : rung === "exhausted"
                ? undefined
                : `Recovering (${rung})…`;
        })
      );
      break;

    case "PlanValidationCompleted":
      setState(
        produce((s) => {
          s.active.lifecycle = "planning";
          s.active.planValidationStatus = sanitizeText(event.status, "valid");
          s.active.planReadinessStatus = sanitizeText(event.readiness_status, s.active.planValidationStatus);
          s.active.planCanProceedToTargetResolution = booleanValue(event.can_proceed_to_target_resolution);
          s.active.planCanExecute = booleanValue(event.can_execute);
          s.active.planValidationBlockerCount = numberValue(event.blocker_count);
          s.active.planValidationWarningCount = numberValue(event.warning_count);
          s.active.planRequiresUserApproval =
            booleanValue(event.requires_user_approval) ?? s.active.planRequiresUserApproval;
          s.active.riskLevel = sanitizeText(event.risk_level, s.active.riskLevel);
          s.active.plannerConfidence = numberValue(event.confidence) ?? s.active.plannerConfidence;
          const blockedReasons = sanitizeList(event.blocked_reasons);
          const validationErrors = sanitizeList(event.validation_errors);
          s.active.planBlockedReasons =
            blockedReasons.length > 0 ? blockedReasons : validationErrors;
          s.active.planWarnings = sanitizeList(event.warnings);
          s.active.planStepValidationResults = sanitizePlanStepValidationResults(event.step_results);
          if ((event.source_evidence?.length ?? 0) > 0) {
            s.active.planSourceEvidence = sanitizeGoalEvidence(event.source_evidence);
          }
          if (s.active.planBlockedReasons.length > 0) {
            s.active.blocker = {
              type: "plan",
              reason: s.active.planBlockedReasons[0],
              options: s.active.planBlockedReasons.slice(1),
            };
          }
        })
      );
      break;

    case "PlanBlocked":
      setBlocker({
        type: "plan",
        reason: sanitizeText(event.reason, "Plan blocked"),
        clarificationQuestion: sanitizeText(event.clarification_question),
        options: sanitizeList(event.options),
      });
      break;

    case "TargetResolutionStarted":
      setState(
        produce((s) => {
          s.active.lifecycle = "resolving";
          s.active.currentAction = {
            actionKind: sanitizeText(event.action_kind),
            target: sanitizeText(event.query || event.role),
            status: "resolving",
          };
        })
      );
      break;

    case "TargetResolved":
      setState(
        produce((s) => {
          s.active.lifecycle = "safety";
          s.active.target = {
            label: sanitizeText(event.label),
            role: sanitizeText(event.role),
            targetType: sanitizeText(event.target_type),
            confidence: numberValue(event.confidence),
            evidence: sanitizeText(event.evidence),
          };
        })
      );
      break;

    case "TargetResolutionBlocked":
      setBlocker({
        type: "target",
        reason: sanitizeText(event.reason, "Target resolution blocked"),
        options: event.target_name ? [sanitizeText(event.target_name)] : [],
        candidateCount: numberValue(event.candidate_count),
      });
      break;

    case "TargetResolutionCompleted":
      setState(
        produce((s) => {
          const status = sanitizeText(event.status);
          const resolvedTarget = sanitizeTargetRecord(event.resolved_target);
          const candidates = sanitizeTargetCandidates(event.candidates);
          s.active.lifecycle =
            status === "resolved"
              ? "safety"
              : status === "skipped"
                ? s.active.lifecycle
                : "blocked";
          s.active.targetResolution = {
            resolutionId: sanitizeText(event.resolution_id),
            planId: sanitizeText(event.plan_id),
            validationId: sanitizeText(event.validation_id),
            status,
            confidence: numberValue(event.confidence),
            candidateCount: candidates.length,
            candidates,
            ambiguityCount: numberValue(event.ambiguity_count),
            ambiguityReasons: sanitizeList(event.ambiguity_reasons),
            blockerCount: numberValue(event.blocker_count),
            blockers: sanitizeList(event.blockers),
            canProceedToSafetyGate: booleanValue(event.can_proceed_to_safety_gate),
            canExecute: booleanValue(event.can_execute),
            promptHash: sanitizeText(event.prompt_hash),
          };
          if (resolvedTarget) {
            resolvedTarget.confidence = numberValue(event.confidence);
            s.active.target = resolvedTarget;
          }
          if (status && status !== "resolved" && status !== "skipped") {
            s.active.blocker = {
              type: "target",
              reason:
                s.active.targetResolution.ambiguityReasons[0] ||
                s.active.targetResolution.blockers[0] ||
                "Target resolution did not produce a safe unique target.",
              options: candidates
                .map((candidate) => candidate.label || candidate.role || candidate.controlId || "")
                .filter((item) => item.length > 0)
                .slice(0, 4),
              candidateCount: candidates.length,
            };
          }
        })
      );
      break;

    case "SafetyGateCompleted":
      setState(
        produce((s) => {
          const status = sanitizeText(event.status);
          const safetyStatus = sanitizeText(event.safety_status, status);
          s.active.lifecycle = safetyStatus.toLowerCase().includes("approval")
            ? "awaiting_approval"
            : safetyStatus.toLowerCase().includes("block") || safetyStatus.toLowerCase().includes("reject")
              ? "blocked"
              : "safety";
          s.active.safetyDecision = {
            safetyGateId: sanitizeText(event.safety_gate_id),
            proposalId: sanitizeText(event.proposal_id),
            requestId: sanitizeText(event.request_id),
            proposalHash: sanitizeText(event.proposal_hash),
            targetHash: sanitizeText(event.target_hash),
            status,
            safetyStatus,
            riskLevel: sanitizeText(event.risk_level),
            reasons: sanitizeList(event.reasons),
            riskReasons: sanitizeList(event.risk_reasons),
            approvalReason: sanitizeText(event.approval_reason),
            blockers: sanitizeList(event.blockers),
            warnings: sanitizeList(event.warnings),
            canRequestHitl: booleanValue(event.can_request_hitl),
            canAuthorizeStep7: booleanValue(event.can_authorize_step7),
            canExecute: booleanValue(event.can_execute),
            actionType: sanitizeText(event.action_type),
            targetLabel: sanitizeText(event.target_label),
            targetRole: sanitizeText(event.target_role),
            expectedPostcondition: sanitizeText(event.expected_postcondition),
            expiresAtMs: numberValue(event.expires_at_ms),
            promptHash: sanitizeText(event.prompt_hash),
          };
          s.active.riskLevel = sanitizeText(event.risk_level, s.active.riskLevel);
        })
      );
      break;

    case "ActionBackendStatus":
      setState(
        produce((s) => {
          s.active.actionBackend = {
            globalHaltEngaged: booleanValue(event.global_halt_engaged),
            haltKind: sanitizeText(event.halt_kind),
            haltReason: sanitizeText(event.halt_reason),
            releaseConditions: sanitizeList(event.release_conditions),
            startupElapsedMs: numberValue(event.startup_elapsed_ms),
            canObserve: booleanValue(event.can_observe),
            canPlan: booleanValue(event.can_plan),
            automationEnabled: booleanValue(event.automation_enabled),
            visionSidecar: sanitizeText(event.vision_sidecar),
            uinputDaemon: sanitizeText(event.uinput_daemon),
            orchestratorAvailable: booleanValue(event.orchestrator_available),
            sessionType: sanitizeText(event.session_type),
            xdotoolAvailable: booleanValue(event.xdotool_available),
            ydotoolAvailable: booleanValue(event.ydotool_available),
            uinputAvailable: booleanValue(event.uinput_available),
            selectedBackend: sanitizeText(event.selected_backend),
            backendSelectionReason: sanitizeText(event.backend_selection_reason),
            backendProbeStatus: sanitizeText(event.backend_probe_status),
            backendProbeErrors: sanitizeList(event.backend_probe_errors),
            inputBackendKind: sanitizeText(event.input_backend_kind),
            focusSupported: booleanValue(event.focus_supported),
            typingSupported: booleanValue(event.typing_supported),
            clickSupported: booleanValue(event.click_supported),
            verificationSupported: booleanValue(event.verification_supported),
            xdotoolUsableForActions: booleanValue(event.xdotool_usable_for_actions),
            ydotoolUsableForActions: booleanValue(event.ydotool_usable_for_actions),
            uinputSocketPath: sanitizeText(event.uinput_socket_path),
            uinputSocketAccessible: booleanValue(event.uinput_socket_accessible),
            canExecuteActions: booleanValue(event.can_execute_actions),
            blockers: sanitizeList(event.blockers),
            capabilities: sanitizeCapabilities(event.capabilities),
          };
        })
      );
      break;

    case "ExecutionBlocked":
      setState(
        produce((s) => {
          s.active.lifecycle = "blocked";
          const reason = sanitizeText(event.reason, "GUI action execution blocked");
          const blockedAction = {
            executionId: sanitizeText(event.execution_id),
            proposalId: sanitizeText(event.proposal_id),
            proposalHash: sanitizeText(event.proposal_hash),
            actionKind: sanitizeText(event.action_kind),
            target: sanitizeText(event.selected_backend),
            status: sanitizeText(event.status, "blocked"),
            backendUsed: sanitizeText(event.backend_used ?? event.selected_backend),
            safeErrorSummary: reason,
            canRetry: booleanValue(event.can_retry),
            recoveryHint: sanitizeText(event.recovery_hint),
          };
          s.active.currentAction = blockedAction;
          s.active.executionReceipt = blockedAction;
          s.active.blocker = {
            type: "execution",
            reason,
            options: sanitizeList(event.blockers),
          };
          s.active.actionBackend = {
            ...(s.active.actionBackend ?? { blockers: [], releaseConditions: [], backendProbeErrors: [] }),
            globalHaltEngaged: booleanValue(event.global_halt_engaged) ?? s.active.actionBackend?.globalHaltEngaged,
            haltKind: sanitizeText(event.halt_kind, s.active.actionBackend?.haltKind ?? ""),
            haltReason: sanitizeText(event.halt_reason, s.active.actionBackend?.haltReason ?? ""),
            releaseConditions: sanitizeList(event.release_conditions, s.active.actionBackend?.releaseConditions ?? []),
            sessionType: sanitizeText(event.session_type, s.active.actionBackend?.sessionType ?? ""),
            selectedBackend: sanitizeText(event.selected_backend, s.active.actionBackend?.selectedBackend ?? ""),
            canExecuteActions: false,
            blockers: sanitizeList(event.blockers),
          };
        })
      );
      break;

    case "HitlRequired":
      setState(
        produce((s) => {
          s.active.lifecycle = "awaiting_approval";
          s.active.pendingApproval = {
            requestId: sanitizeText(event.request_id),
            proposalId: sanitizeText(event.proposal_id),
            proposalHash: sanitizeText(event.proposal_hash),
            targetHash: sanitizeText(event.target_hash),
            actionType: sanitizeText(event.action_type),
            targetLabel: sanitizeText(event.target_label),
            targetRole: sanitizeText(event.target_role),
            reason: sanitizeText(event.reason, "Approval required"),
            riskLevel: sanitizeText(event.risk_level, s.active.riskLevel),
            riskReasons: sanitizeList(event.risk_reasons),
            expectedPostcondition: sanitizeText(event.expected_postcondition),
            expiresAtMs: numberValue(event.expires_at_ms),
            canAuthorizeStep7: booleanValue(event.can_authorize_step7),
            canExecute: booleanValue(event.can_execute),
          };
        })
      );
      break;

    case "HitlDecisionRecorded":
    case "HitlDecisionInvalidated":
      setState(
        produce((s) => {
          const decision = sanitizeText(event.decision);
          s.active.lifecycle = decision === "approved"
            ? "awaiting_approval"
            : decision.includes("reject") || decision === "denied" || decision === "expired"
              ? "blocked"
              : s.active.lifecycle;
          s.active.hitlDecision = {
            decisionId: sanitizeText(event.decision_id),
            requestId: sanitizeText(event.request_id),
            proposalId: sanitizeText(event.proposal_id),
            proposalHash: sanitizeText(event.proposal_hash),
            targetHash: sanitizeText(event.target_hash),
            decision,
            decidedAtMs: numberValue(event.decided_at_ms),
            decisionReason: sanitizeText(event.decision_reason),
            actor: sanitizeText(event.actor),
            userVisibleSummaryHash: sanitizeText(event.user_visible_summary_hash),
            canAuthorizeStep7: booleanValue(event.can_authorize_step7),
            canExecute: booleanValue(event.can_execute),
          };
        })
      );
      break;

    case "ActionStarted":
      setState(
        produce((s) => {
          s.active.lifecycle = "executing";
          s.active.currentAction = {
            executionId: sanitizeText(event.execution_id),
            proposalId: sanitizeText(event.proposal_id),
            proposalHash: sanitizeText(event.proposal_hash),
            targetHash: sanitizeText(event.target_hash),
            actionKind: sanitizeText(event.action_kind),
            target: sanitizeText(event.target),
            status: "running",
            backendUsed: sanitizeText(event.backend_used),
            authorizationSource: sanitizeText(event.authorization_source),
          };
        })
      );
      break;

    case "ActionCompleted":
      setState(
        produce((s) => {
          s.active.lifecycle = "verifying";
          s.active.executionReceipt = {
            executionId: sanitizeText(event.execution_id),
            proposalId: sanitizeText(event.proposal_id),
            proposalHash: sanitizeText(event.proposal_hash),
            targetHash: sanitizeText(event.target_hash),
            actionKind: sanitizeText(event.action_kind),
            status: sanitizeText(event.status, "completed"),
            backendUsed: sanitizeText(event.backend_used),
            resultSummary: sanitizeText(event.result_summary),
          };
        })
      );
      break;

    case "ActionFailed":
      setState(
        produce((s) => {
          s.active.lifecycle = "failed";
          const summary = sanitizeText(event.safe_error_summary, "Deterministic GUI action failed");
          s.active.executionReceipt = {
            executionId: sanitizeText(event.execution_id),
            proposalId: sanitizeText(event.proposal_id),
            proposalHash: sanitizeText(event.proposal_hash),
            targetHash: sanitizeText(event.target_hash),
            actionKind: sanitizeText(event.action_kind),
            status: sanitizeText(event.status, "failed"),
            backendUsed: sanitizeText(event.backend_used),
            safeErrorSummary: summary,
            resultSummary: summary,
          };
          s.active.blocker = {
            type: "execution",
            reason: summary,
            options: [],
          };
        })
      );
      break;

    case "VerificationStarted":
      setState("active", "lifecycle", "verifying");
      break;

    case "VerificationCompleted":
      setState(
        produce((s) => {
          s.active.lifecycle = "verifying";
          s.active.verification = {
            status: sanitizeText(event.status),
            confidence: numberValue(event.confidence),
            summary: sanitizeText(event.summary),
          };
        })
      );
      break;

    case "ExecutionVerificationCompleted":
      setState(
        produce((s) => {
          const verificationStatus = sanitizeText(event.status).toLowerCase();
          s.active.lifecycle =
            verificationStatus === "verified"
              ? "completed"
              : verificationStatus === "verification_failed" || verificationStatus === "blocked"
                ? "failed"
                : s.active.lifecycle;
          s.active.verification = {
            status: sanitizeText(event.status),
            confidence: numberValue(event.confidence),
            summary: sanitizeText(
              event.safe_error_summary || event.verification_result || event.postcondition_check
            ),
            verificationStrategy: sanitizeText(event.verification_strategy),
            evidence: Array.isArray(event.evidence)
              ? event.evidence.map((value) => sanitizeText(value)).filter(Boolean)
              : undefined,
            preStateSummary: sanitizeText(event.pre_state_summary),
            postStateSummary: sanitizeText(event.post_state_summary),
            matchedExpectedState: booleanValue(event.matched_expected_state),
            targetStillPresent: booleanValue(event.target_still_present),
            targetIdentityMatches: booleanValue(event.target_identity_matches),
            safeErrorSummary: sanitizeText(event.safe_error_summary),
            recoveryHint: sanitizeText(event.recovery_hint),
            canRetry: booleanValue(event.can_retry),
          };
          s.active.executionReceipt = {
            ...(s.active.executionReceipt ?? {}),
            executionId: sanitizeText(event.execution_id, s.active.executionReceipt?.executionId ?? ""),
            proposalId: sanitizeText(event.proposal_id, s.active.executionReceipt?.proposalId ?? ""),
            status: sanitizeText(event.status, s.active.executionReceipt?.status ?? ""),
            canRetry: booleanValue(event.can_retry),
            recoveryHint: sanitizeText(event.recovery_hint),
          };
        })
      );
      break;

    case "RecoveryEvaluationStarted":
      setState(
        produce((s) => {
          s.active.lifecycle = "verifying";
          const reason = sanitizeText(event.reason);
          if (reason) {
            s.active.recoveryOptions = [reason];
          }
        })
      );
      break;

    case "RecoveryAttemptStarted":
      setState("active", "lifecycle", "executing");
      break;

    case "RecoveryAttemptCompleted":
      setState(
        produce((s) => {
          s.active.lifecycle = sanitizeText(event.status).toLowerCase().includes("fail")
            ? "blocked"
            : "verifying";
        })
      );
      break;

    case "RecoveryProposed":
      setState(
        produce((s) => {
          s.active.lifecycle = "blocked";
          s.active.recoveryOptions = sanitizeList(event.options);
          s.active.blocker = {
            type: "execution",
            reason: sanitizeText(event.reason, "Recovery proposed"),
            options: sanitizeList(event.options),
          };
        })
      );
      break;

    case "RecoveryAssessmentCompleted":
      setState(
        produce((s) => {
          s.active.recovery = {
            ...(s.active.recovery ?? {}),
            status: sanitizeText(event.status),
            failureKind: sanitizeText(event.failure_kind),
            recoveryActionKind: sanitizeText(event.recovery_action_kind),
            proposedRecoveryStep: sanitizeText(event.proposed_recovery_step),
            requiresUserApproval: booleanValue(event.requires_user_approval),
            canRecover: booleanValue(event.can_recover),
            canExecuteRecovery: booleanValue(event.can_execute_recovery),
            retryCount: numberValue(event.retry_count),
            maxRetryCount: numberValue(event.max_retry_count),
            blockers: sanitizeList(event.blockers),
            warnings: sanitizeList(event.warnings),
            safeExplanation: sanitizeText(event.safe_explanation),
            recoveryHint: sanitizeText(event.recovery_hint),
          };
        })
      );
      break;

    case "RecoveryActionStarted":
      setState(
        produce((s) => {
          s.active.lifecycle = "executing";
          s.active.recovery = {
            ...(s.active.recovery ?? {}),
            recoveryActionKind: sanitizeText(
              event.recovery_action_kind,
              s.active.recovery?.recoveryActionKind ?? ""
            ),
          };
        })
      );
      break;

    case "RecoveryActionCompleted":
      setState(
        produce((s) => {
          s.active.lifecycle = "completed";
          s.active.recovery = {
            ...(s.active.recovery ?? {}),
            status: sanitizeText(event.status, "recovered"),
            recoveryActionKind: sanitizeText(
              event.recovery_action_kind,
              s.active.recovery?.recoveryActionKind ?? ""
            ),
            verificationResult: sanitizeText(event.verification_result),
            nextRecommendedState: sanitizeText(event.next_recommended_state),
            canRetryOriginalAction: booleanValue(event.can_retry_original_action),
            canContinueWorkflow: booleanValue(event.can_continue_workflow),
          };
        })
      );
      break;

    case "RecoveryBlocked":
      setState(
        produce((s) => {
          // Recovery could not safely act. Keep the verification verdict in the
          // lifecycle unless the recovery needs the user.
          const status = sanitizeText(event.status).toLowerCase();
          if (status === "needs_clarification" || status === "needs_approval") {
            s.active.lifecycle = "blocked";
          }
          s.active.recovery = {
            ...(s.active.recovery ?? {}),
            status: sanitizeText(event.status, s.active.recovery?.status ?? "blocked"),
            failureKind: sanitizeText(event.failure_kind, s.active.recovery?.failureKind ?? ""),
            recoveryActionKind: sanitizeText(
              event.recovery_action_kind,
              s.active.recovery?.recoveryActionKind ?? ""
            ),
            requiresUserApproval: booleanValue(event.requires_user_approval),
            blockers: sanitizeList(event.blockers),
            safeExplanation: sanitizeText(event.safe_explanation, s.active.recovery?.safeExplanation ?? ""),
            recoveryHint: sanitizeText(event.recovery_hint),
            verificationResult: sanitizeText(event.verification_result),
            nextRecommendedState: sanitizeText(event.next_recommended_state),
            canRetryOriginalAction: booleanValue(event.can_retry_original_action),
            canContinueWorkflow: booleanValue(event.can_continue_workflow),
          };
          const reason = sanitizeText(event.safe_explanation)
            || (sanitizeList(event.blockers)[0] ?? "");
          if (reason) {
            s.active.blocker = {
              type: "execution",
              reason,
              options: sanitizeList(event.blockers),
            };
          }
        })
      );
      break;

    case "WorkflowRunStarted":
      setState(
        produce((s) => {
          s.active.lifecycle = "executing";
          s.active.workflow = {
            workflowRunId: sanitizeText(event.workflow_run_id),
            status: "running",
            currentStepIndex: numberValue(event.current_step_index),
            stepCount: numberValue(event.step_count),
            completedStepCount: 0,
            riskLevel: sanitizeText(event.risk_level),
            requiresUserApproval: booleanValue(event.requires_user_approval),
            executionMode: sanitizeText(event.execution_mode),
            steps: [],
          };
        })
      );
      break;

    case "WorkflowStepStarted":
      setState(
        produce((s) => {
          const workflow = s.active.workflow ?? { steps: [] };
          const stepIndex = numberValue(event.step_index) ?? workflow.steps.length;
          const view = {
            stepId: sanitizeText(event.step_id),
            stepIndex,
            stepType: sanitizeText(event.step_type),
            status: sanitizeText(event.status, "started"),
            blockers: [],
            warnings: [],
          };
          const existing = workflow.steps.findIndex((step) => step.stepIndex === stepIndex);
          if (existing >= 0) {
            workflow.steps[existing] = view;
          } else {
            workflow.steps.push(view);
          }
          workflow.currentStepIndex = numberValue(event.current_step_index) ?? stepIndex;
          s.active.workflow = workflow;
        })
      );
      break;

    case "WorkflowStepCompleted":
      setState(
        produce((s) => {
          const workflow = s.active.workflow ?? { steps: [] };
          const stepIndex = numberValue(event.step_index) ?? 0;
          const view = workflow.steps.find((step) => step.stepIndex === stepIndex);
          if (view) {
            view.status = sanitizeText(event.status, "completed");
            view.receiptId = sanitizeText(event.receipt_id);
            view.warnings = sanitizeList(event.warnings);
          }
          workflow.completedStepCount = (workflow.completedStepCount ?? 0) + 1;
          s.active.workflow = workflow;
        })
      );
      break;

    case "WorkflowStepBlocked":
      setState(
        produce((s) => {
          const workflow = s.active.workflow ?? { steps: [] };
          const stepIndex = numberValue(event.step_index) ?? 0;
          const view = workflow.steps.find((step) => step.stepIndex === stepIndex);
          if (view) {
            view.status = sanitizeText(event.status, "blocked");
            view.blockers = sanitizeList(event.blockers);
          }
          s.active.workflow = workflow;
        })
      );
      break;

    case "WorkflowRunCompleted":
    case "WorkflowRunBlocked":
    case "WorkflowRunPaused":
      setState(
        produce((s) => {
          const workflow = s.active.workflow ?? { steps: [] };
          workflow.status = sanitizeText(event.status);
          workflow.currentStepIndex = numberValue(event.current_step_index);
          workflow.stepCount = numberValue(event.step_count) ?? workflow.stepCount;
          workflow.completedStepCount =
            numberValue(event.completed_step_count) ?? workflow.completedStepCount;
          workflow.blockedReason = sanitizeText(event.blocked_reason);
          s.active.workflow = workflow;
          s.active.lifecycle =
            event.type === "WorkflowRunCompleted"
              ? "completed"
              : event.type === "WorkflowRunPaused"
                ? "blocked"
                : "blocked";
        })
      );
      break;

    case "WorkflowCheckpointSaved":
      setState(
        produce((s) => {
          s.active.checkpoint = {
            ...(s.active.checkpoint ?? {}),
            checkpointId: sanitizeText(event.checkpoint_id),
            checkpointHashPrefix: sanitizeText(event.checkpoint_hash_prefix),
            currentStepIndex: numberValue(event.current_step_index),
            stepCount: numberValue(event.step_count),
            completedStepCount: numberValue(event.completed_step_count),
            pendingStepId: sanitizeText(event.pending_step_id),
            requiresUserApproval: booleanValue(event.requires_user_approval),
            canResume: booleanValue(event.can_resume),
          };
        })
      );
      break;

    case "WorkflowResumeRequested":
      setState(
        produce((s) => {
          s.active.checkpoint = {
            ...(s.active.checkpoint ?? {}),
            checkpointId: sanitizeText(event.checkpoint_id, s.active.checkpoint?.checkpointId ?? ""),
            resumeReason: sanitizeText(event.reason),
            resumeStatus: "requested",
          };
        })
      );
      break;

    case "WorkflowCheckpointLoaded":
      setState(
        produce((s) => {
          s.active.checkpoint = {
            ...(s.active.checkpoint ?? {}),
            checkpointId: sanitizeText(event.checkpoint_id, s.active.checkpoint?.checkpointId ?? ""),
            checkpointHashPrefix: sanitizeText(
              event.checkpoint_hash_prefix,
              s.active.checkpoint?.checkpointHashPrefix ?? ""
            ),
            currentStepIndex: numberValue(event.current_step_index),
            completedStepCount: numberValue(event.completed_step_count),
            resumeStatus: "loaded",
          };
        })
      );
      break;

    case "WorkflowResumeValidated":
      setState(
        produce((s) => {
          s.active.checkpoint = {
            ...(s.active.checkpoint ?? {}),
            resumeStatus: sanitizeText(event.status, "resumed"),
            nextStepId: sanitizeText(event.next_step_id),
          };
        })
      );
      break;

    case "WorkflowResumeRejected":
    case "WorkflowApprovalInvalidated":
    case "WorkflowDuplicateActionBlocked":
      setState(
        produce((s) => {
          s.active.checkpoint = {
            ...(s.active.checkpoint ?? {}),
            resumeStatus: sanitizeText(event.status, "rejected"),
            invalidatedApprovals: sanitizeList(event.invalidated_approvals),
            duplicateActionGuards: sanitizeList(event.duplicate_action_guards),
            resumeExplanation: sanitizeText(event.safe_explanation),
          };
          const reason = sanitizeText(event.safe_explanation)
            || (sanitizeList(event.blockers)[0] ?? "");
          if (reason) {
            s.active.blocker = {
              type: "execution",
              reason,
              options: sanitizeList(event.blockers),
            };
          }
          s.active.lifecycle = "blocked";
        })
      );
      break;

    case "TurnCompleted":
      setState(
        produce((s) => {
          s.active.finalStatus = sanitizeText(event.status, "completed");
          s.active.lifecycle = lifecycleForTerminal(event.status);
        })
      );
      break;

    case "TurnFailed":
      setState(
        produce((s) => {
          const reason = sanitizeText(event.reason || event.error || event.status, "Turn failed");
          s.active.lifecycle = "failed";
          s.active.finalStatus = sanitizeText(event.status, "failed");
          s.active.blocker = {
            type: "turn",
            reason,
            options: [],
          };
        })
      );
      break;
  }
}

export function __resetGuiCognitionSessionForTests(): void {
  clearGuiCognitionSession();
}
