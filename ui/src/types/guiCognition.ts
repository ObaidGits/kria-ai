export type GuiCognitionLifecycle =
  | "idle"
  | "observing"
  | "planning"
  | "resolving"
  | "safety"
  | "awaiting_approval"
  | "executing"
  | "verifying"
  | "blocked"
  | "completed"
  | "failed"
  | "cancelled";

export interface GuiCognitionEnvelope {
  version: number;
  session_id: string;
  turn_id: string;
  workflow_id: string;
  sequence: number;
  timestamp_ms: number;
  event: GuiCognitionEvent;
}

export interface GuiCognitionProbeTiming {
  probe_name?: string;
  duration_ms?: number;
  status?: string;
  source?: string;
  cache_hit?: boolean;
  blocker_kind?: string | null;
}

export interface GuiCognitionSourceAttempt {
  source?: string;
  status?: string;
  reliability?: string;
  reason?: string | null;
}

export type GuiCognitionEvent =
  | { type: "TurnStarted"; mode_id?: string }
  | { type: "RouteConfirmed"; path?: string; llm_tool_loop?: boolean }
  | { type: "ObservationStarted"; sources?: string[]; cache_policy?: string }
  | {
      type: "ObservationBlocked";
      reason?: string;
      blockers?: Record<string, string | null | undefined>;
    }
  | {
      type: "ObservationCompleted";
      observation_id?: string;
      active_window?: string;
      active_window_source?: string;
      active_window_confidence?: number;
      active_window_reliability?: string;
      active_window_blocker?: string | null;
      active_window_authority_source?: string;
      active_window_authority_confidence?: number;
      active_window_authority_status?: string;
      gnome_bridge_status?: string;
      active_window_app?: string | null;
      active_window_app_id?: string | null;
      active_window_pid?: number | null;
      active_window_workspace?: number | null;
      active_window_monitor?: number | null;
      active_window_fullscreen?: boolean | null;
      active_window_minimized?: boolean | null;
      active_window_fallback_chain?: GuiCognitionSourceAttempt[];
      active_window_failure_chain?: GuiCognitionSourceAttempt[];
      visible_app_count?: number;
      visible_control_count?: number;
      visible_accessible_control_count?: number;
      disabled_control_count?: number;
      hidden_control_count?: number;
      trusted_control_count?: number;
      partial_control_count?: number;
      not_executable_control_count?: number;
      text_field_count?: number;
      button_count?: number;
      dialog_count?: number;
      other_control_count?: number;
      ocr_available?: boolean;
      ocr_block_count?: number;
      ocr_trust?: string;
      ocr_wait_for_screenshot_ms?: number | null;
      ocr_engine_selected?: string | null;
      ocr_engine_status?: string | null;
      ocr_image_status?: string | null;
      ocr_total_ms?: number | null;
      ocr_fast_path?: string | null;
      ocr_cache_hit?: boolean;
      ocr_roi_count?: number;
      ocr_changed_region_count?: number;
      ocr_cold_start_ms?: number | null;
      ocr_warm_start_ms?: number | null;
      ocr_benchmark_summary?: string | null;
      ocr_injection_count?: number;
      ocr_blocker?: string | null;
      accessibility_available?: boolean;
      accessibility_source_status?: string;
      accessibility_overall_status?: string;
      accessibility_overall_confidence?: number;
      accessibility_app_scores?: Array<Record<string, unknown>>;
      accessibility_stale_node_count?: number;
      accessibility_timeout_count?: number;
      accessibility_cache_hit_count?: number;
      accessibility_stale_cache_rejected_count?: number;
      accessibility_node_count?: number;
      accessibility_control_count?: number;
      atspi_snapshot_total_ms?: number | null;
      atspi_skipped_app_count?: number;
      atspi_omitted_node_count?: number;
      accessibility_remediation?: string[];
      screenshot_available?: boolean;
      screenshot_status?: string;
      screenshot_capture_ms?: number;
      screenshot_duration_ms?: number;
      screen_hash_prefix?: string;
      monitor_count?: number;
      dpi_available?: boolean;
      cursor_focus_known?: boolean;
      focused_window?: string;
      focused_app?: string | null;
      focused_control_id?: string | null;
      focused_control_label?: string | null;
      focused_control_role?: string | null;
      focused_control_bounds?: Record<string, unknown> | null;
      text_cursor_known?: boolean;
      editable_target_known?: boolean;
      terminal_like?: boolean;
      focus_source?: string;
      focus_confidence?: number;
      focus_reliability?: string;
      focus_adapter_status?: string | null;
      focus_latency_ms?: number | null;
      focus_failure_chain?: GuiCognitionSourceAttempt[];
      observation_total_ms?: number;
      slowest_probe?: string | null;
      slowest_probe_ms?: number;
      probe_timeout_count?: number;
      probe_timings?: GuiCognitionProbeTiming[];
      cache_hit?: boolean;
      cache_age_ms?: number | null;
      cache_policy?: string;
      freshness?: string;
      source_blockers?: Record<string, string | null | undefined>;
      executable_control_count?: number;
      visual_control_count?: number;
      visual_control_summary?: Record<string, unknown>;
    }
  | {
      type: "ContextBuilt";
      context_id?: string;
      observation_id?: string;
      previous_context_id?: string | null;
      previous_observation_id?: string | null;
      active_window?: string;
      screen_hash_prefix?: string | null;
      source_confidence?: Record<string, number | undefined>;
      source_trust?: Record<string, string | undefined>;
      focus?: Record<string, unknown>;
      accessibility_health?: Record<string, unknown>;
      visual_controls?: Record<string, unknown>;
      ocr_performance?: Record<string, unknown>;
      control_summary?: {
        text_fields?: number;
        buttons?: number;
        dialogs?: number;
      };
      trusted_control_count?: number;
      executable_control_count?: number;
      disabled_or_hidden_count?: number;
      ocr_untrusted?: boolean;
      ocr_injection_count?: number;
      redaction_count?: number;
      freshness?: string;
      status?: string;
      delta?: Record<string, unknown>;
      source_blockers?: string[];
      warnings?: string[];
    }
  | {
      type: "GoalContractCreated";
      contract_id?: string;
      observation_id?: string;
      context_id?: string;
      goal_summary?: string;
      prompt_hash?: string;
      intent_kind?: string;
      action_type?: string;
      target_app_kind?: string | null;
      target_app_hint?: string | null;
      target_window_hint?: string | null;
      target_control_hint?: string | null;
      query_summary?: string | null;
      query_hash?: string | null;
      text_payload_summary?: string | null;
      text_payload_hash?: string | null;
      desired_final_state?: string;
      risk_level?: string;
      requires_user_approval?: boolean;
      ambiguity_count?: number;
      ambiguities?: Array<{
        kind?: string;
        field?: string | null;
        message?: string;
      }>;
      source_evidence?: Array<{
        source?: string;
        field?: string;
        summary?: string;
        confidence?: number;
      }>;
      extraction_confidence?: number;
      extractor_mode?: string;
    }
  | {
      type: "LlmPlanningStarted";
      planner_mode?: string;
      context_id?: string;
      observation_id?: string;
    }
  | {
      type: "LlmPlanningCompleted";
      status?: string;
      model?: string | null;
      confidence?: number;
      step_count?: number;
      risk_level?: string;
    }
  | {
      type: "LlmPlanningFailed";
      status?: string;
      reason?: string;
    }
  | {
      type: "PlanCreated";
      summary?: string;
      plan_id?: string;
      goal_contract_id?: string | null;
      context_id?: string | null;
      prompt_hash?: string | null;
      goal_action_type?: string | null;
      planner_mode?: string;
      plan_status?: string;
      step_count?: number;
      risk_level?: string;
      requires_user_approval?: boolean;
      ambiguity_count?: number;
      confidence?: number;
      validation_errors?: string[];
      source_evidence?: Array<{
        source?: string;
        field?: string;
        summary?: string;
        confidence?: number;
      }>;
      steps?: string[];
      typed_steps?: GuiCognitionTypedPlanStepState[];
    }
  | {
      type: "PlanValidationCompleted";
      validation_id?: string;
      plan_id?: string;
      goal_contract_id?: string | null;
      context_id?: string | null;
      prompt_hash?: string | null;
      status?: string;
      readiness_status?: string;
      risk_level?: string | null;
      requires_user_approval?: boolean;
      can_proceed_to_target_resolution?: boolean;
      can_execute?: boolean;
      blocker_count?: number;
      warning_count?: number;
      blocked_reasons?: string[];
      warnings?: string[];
      validation_errors?: string[];
      source_evidence?: Array<{
        source?: string;
        field?: string;
        summary?: string;
        confidence?: number;
      }>;
      step_results?: Array<{
        step_id?: string;
        step_type?: string;
        status?: string;
        risk_level?: string;
        requires_approval?: boolean;
        target_resolution_required?: boolean;
        target_available?: boolean;
        verification_present?: boolean;
        precondition_status?: string;
        postcondition_status?: string;
        blocker?: string | null;
        confidence?: number;
      }>;
      confidence?: number;
    }
  | { type: "PlanBlocked"; reason?: string; clarification_question?: string; options?: string[] }
  | { type: "TargetResolutionStarted"; action_kind?: string; role?: string; query?: string }
  | {
      type: "TargetResolved";
      target_type?: string;
      label?: string;
      role?: string;
      confidence?: number;
      evidence?: string;
    }
  | {
      type: "TargetResolutionBlocked";
      reason?: string;
      candidate_count?: number;
      target_name?: string;
    }
  | {
      type: "TargetResolutionCompleted";
      resolution_id?: string;
      plan_id?: string;
      validation_id?: string;
      goal_contract_id?: string;
      context_id?: string;
      observation_id?: string;
      status?: string;
      results?: unknown[];
      resolved_target?: Record<string, unknown> | null;
      candidates?: unknown[];
      confidence?: number;
      ambiguity_count?: number;
      ambiguity_reasons?: string[];
      blocker_count?: number;
      blockers?: string[];
      can_proceed_to_safety_gate?: boolean;
      can_execute?: boolean;
      prompt_hash?: string;
    }
  | {
      type: "SafetyGateStarted";
      plan_id?: string;
      resolution_id?: string;
      mode?: string;
      can_execute?: boolean;
      prompt_hash?: string;
    }
  | {
      type: "SafetyGateCompleted";
      safety_gate_id?: string;
      proposal_id?: string;
      request_id?: string;
      proposal_hash?: string;
      target_hash?: string;
      status?: string;
      safety_status?: string;
      risk_level?: string;
      reasons?: string[];
      risk_reasons?: string[];
      requires_user_approval?: boolean;
      approval_reason?: string;
      blockers?: string[];
      warnings?: string[];
      can_request_hitl?: boolean;
      can_authorize_step7?: boolean;
      can_execute?: boolean;
      action_type?: string;
      target_label?: string;
      target_role?: string;
      expected_postcondition?: string;
      expires_at_ms?: number;
      prompt_hash?: string;
    }
  | {
      type: "ActionBackendStatus";
      global_halt_engaged?: boolean;
      halt_kind?: string;
      halt_reason?: string | null;
      release_conditions?: string[];
      startup_elapsed_ms?: number | null;
      can_observe?: boolean;
      can_plan?: boolean;
      automation_enabled?: boolean;
      vision_sidecar?: string;
      uinput_daemon?: string;
      orchestrator_available?: boolean;
      session_type?: string;
      xdotool_available?: boolean;
      ydotool_available?: boolean;
      uinput_available?: boolean;
      selected_backend?: string;
      backend_selection_reason?: string;
      backend_probe_status?: string;
      backend_probe_errors?: string[];
      input_backend_kind?: string;
      focus_supported?: boolean;
      typing_supported?: boolean;
      click_supported?: boolean;
      verification_supported?: boolean;
      xdotool_usable_for_actions?: boolean;
      ydotool_usable_for_actions?: boolean;
      uinput_socket_path?: string | null;
      uinput_socket_accessible?: boolean;
      can_execute_actions?: boolean;
      blockers?: string[];
      capabilities?: Partial<GuiCognitionActionBackendCapabilities>;
    }
  | {
      type: "ExecutionBlocked";
      execution_id?: string;
      proposal_id?: string;
      proposal_hash?: string;
      reason?: string;
      action_kind?: string;
      status?: string;
      backend_used?: string;
      selected_backend?: string;
      prompt_hash?: string;
      session_type?: string;
      global_halt_engaged?: boolean;
      halt_kind?: string;
      halt_reason?: string | null;
      release_conditions?: string[];
      blockers?: string[];
      can_retry?: boolean;
      recovery_hint?: string;
    }
  | {
      type: "HitlRequired";
      request_id?: string;
      proposal_id?: string;
      proposal_hash?: string;
      target_hash?: string;
      action_type?: string;
      target_label?: string;
      target_role?: string;
      reason?: string;
      risk_level?: string;
      risk_reasons?: string[];
      expected_postcondition?: string;
      expires_at_ms?: number;
      requires_user_approval?: boolean;
      can_authorize_step7?: boolean;
      can_execute?: boolean;
      prompt_hash?: string;
    }
  | {
      type: "HitlDecisionRecorded" | "HitlDecisionInvalidated";
      decision_id?: string;
      request_id?: string;
      proposal_id?: string;
      proposal_hash?: string;
      target_hash?: string;
      decision?: string;
      decided_at_ms?: number;
      decision_reason?: string;
      actor?: string;
      user_visible_summary_hash?: string;
      can_authorize_step7?: boolean;
      can_execute?: boolean;
    }
  | {
      type: "ActionStarted";
      execution_id?: string;
      proposal_id?: string;
      proposal_hash?: string;
      target_hash?: string;
      action_kind?: string;
      target?: string;
      backend_used?: string;
      authorization_source?: string;
      prompt_hash?: string;
    }
  | {
      type: "ActionCompleted";
      execution_id?: string;
      proposal_id?: string;
      proposal_hash?: string;
      target_hash?: string;
      action_kind?: string;
      status?: string;
      backend_used?: string;
      result_summary?: string;
      prompt_hash?: string;
    }
  | {
      type: "ActionFailed";
      execution_id?: string;
      proposal_id?: string;
      proposal_hash?: string;
      target_hash?: string;
      action_kind?: string;
      status?: string;
      backend_used?: string;
      safe_error_summary?: string;
      prompt_hash?: string;
    }
  | { type: "VerificationStarted"; verification?: string }
  | { type: "VerificationCompleted"; status?: string; confidence?: number; summary?: string }
  | {
      type: "ExecutionVerificationCompleted";
      execution_id?: string;
      proposal_id?: string;
      verification_id?: string;
      status?: string;
      postcondition_check?: string;
      verification_result?: string;
      verification_strategy?: string;
      evidence?: string[];
      pre_state_summary?: string;
      post_state_summary?: string;
      matched_expected_state?: boolean;
      target_still_present?: boolean;
      target_identity_matches?: boolean;
      confidence?: number;
      safe_error_summary?: string;
      can_retry?: boolean;
      recovery_hint?: string;
      prompt_hash?: string;
    }
  | { type: "RecoveryEvaluationStarted"; reason?: string; idempotency?: string }
  | { type: "RecoveryAttemptStarted"; reason?: string; strategy?: string }
  | { type: "RecoveryAttemptCompleted"; status?: string; summary?: string }
  | { type: "RecoveryProposed"; reason?: string; options?: string[] }
  | {
      type: "RecoveryAssessmentCompleted";
      recovery_id?: string;
      execution_id?: string;
      verification_id?: string;
      proposal_id?: string;
      failure_kind?: string;
      status?: string;
      proposed_recovery_step?: string;
      recovery_action_kind?: string;
      requires_user_approval?: boolean;
      can_recover?: boolean;
      can_execute_recovery?: boolean;
      retry_count?: number;
      max_retry_count?: number;
      blockers?: string[];
      warnings?: string[];
      safe_explanation?: string;
      recovery_hint?: string;
      prompt_hash?: string;
    }
  | {
      type: "RecoveryActionStarted";
      recovery_id?: string;
      execution_id?: string;
      recovery_action_kind?: string;
      backend_used?: string;
      prompt_hash?: string;
    }
  | {
      type: "RecoveryActionCompleted";
      recovery_id?: string;
      execution_id?: string;
      status?: string;
      recovery_action_kind?: string;
      backend_used?: string;
      verification_result?: string;
      next_recommended_state?: string;
      can_retry_original_action?: boolean;
      can_continue_workflow?: boolean;
      safe_error_summary?: string;
      prompt_hash?: string;
    }
  | {
      type: "RecoveryBlocked";
      recovery_id?: string;
      execution_id?: string;
      failure_kind?: string;
      status?: string;
      recovery_action_kind?: string;
      requires_user_approval?: boolean;
      blockers?: string[];
      safe_explanation?: string;
      recovery_hint?: string;
      verification_result?: string;
      next_recommended_state?: string;
      can_retry_original_action?: boolean;
      can_continue_workflow?: boolean;
      prompt_hash?: string;
    }
  | {
      type: "WorkflowRunStarted";
      workflow_run_id?: string;
      plan_id?: string;
      goal_contract_id?: string;
      step_count?: number;
      current_step_index?: number;
      risk_level?: string;
      requires_user_approval?: boolean;
      execution_mode?: string;
      prompt_hash?: string;
    }
  | {
      type: "WorkflowStepStarted";
      workflow_run_id?: string;
      step_id?: string;
      step_index?: number;
      step_type?: string;
      status?: string;
      current_step_index?: number;
      step_count?: number;
      prompt_hash?: string;
    }
  | {
      type: "WorkflowStepCompleted";
      workflow_run_id?: string;
      step_id?: string;
      step_index?: number;
      step_type?: string;
      status?: string;
      receipt_id?: string;
      current_step_index?: number;
      step_count?: number;
      warnings?: string[];
      prompt_hash?: string;
    }
  | {
      type: "WorkflowStepBlocked";
      workflow_run_id?: string;
      step_id?: string;
      step_index?: number;
      step_type?: string;
      status?: string;
      current_step_index?: number;
      step_count?: number;
      blockers?: string[];
      prompt_hash?: string;
    }
  | {
      type: "WorkflowRunCompleted" | "WorkflowRunBlocked" | "WorkflowRunPaused";
      workflow_run_id?: string;
      plan_id?: string;
      goal_contract_id?: string;
      status?: string;
      current_step_index?: number;
      step_count?: number;
      completed_step_count?: number;
      blocked_reason?: string;
      prompt_hash?: string;
    }
  | {
      type: "WorkflowCheckpointSaved";
      checkpoint_id?: string;
      checkpoint_hash_prefix?: string;
      workflow_run_id?: string;
      current_step_index?: number;
      step_count?: number;
      completed_step_count?: number;
      pending_step_id?: string;
      requires_user_approval?: boolean;
      can_resume?: boolean;
      prompt_hash?: string;
    }
  | {
      type: "WorkflowResumeRequested";
      resume_id?: string;
      checkpoint_id?: string;
      workflow_run_id?: string;
      reason?: string;
      prompt_hash?: string;
    }
  | {
      type: "WorkflowCheckpointLoaded";
      checkpoint_id?: string;
      checkpoint_hash_prefix?: string;
      workflow_run_id?: string;
      current_step_index?: number;
      completed_step_count?: number;
      prompt_hash?: string;
    }
  | {
      type: "WorkflowResumeValidated";
      resume_id?: string;
      checkpoint_id?: string;
      workflow_run_id?: string;
      status?: string;
      next_step_id?: string;
      next_step_index?: number;
      warnings?: string[];
      can_continue_workflow?: boolean;
      prompt_hash?: string;
    }
  | {
      type: "WorkflowResumeRejected" | "WorkflowApprovalInvalidated" | "WorkflowDuplicateActionBlocked";
      resume_id?: string;
      checkpoint_id?: string;
      workflow_run_id?: string;
      status?: string;
      invalidated_approvals?: string[];
      duplicate_action_guards?: string[];
      blockers?: string[];
      safe_explanation?: string;
      prompt_hash?: string;
    }
  | { type: "TurnCompleted"; status?: string }
  | { type: "TurnFailed"; status?: string; reason?: string; error?: string };

export interface GuiCognitionObservationState {
  observationId?: string;
  contextId?: string;
  activeWindow?: string;
  activeWindowSource?: string;
  activeWindowConfidence?: number;
  activeWindowReliability?: string;
  activeWindowBlocker?: string;
  activeWindowAuthoritySource?: string;
  activeWindowAuthorityConfidence?: number;
  activeWindowAuthorityStatus?: string;
  gnomeBridgeStatus?: string;
  activeWindowApp?: string;
  activeWindowAppId?: string;
  activeWindowPid?: number;
  activeWindowWorkspace?: number;
  activeWindowMonitor?: number;
  activeWindowFullscreen?: boolean;
  activeWindowMinimized?: boolean;
  visibleAppCount?: number;
  visibleControlCount?: number;
  visibleAccessibleControlCount?: number;
  disabledControlCount?: number;
  hiddenControlCount?: number;
  trustedControlCount?: number;
  partialControlCount?: number;
  notExecutableControlCount?: number;
  textFieldCount?: number;
  buttonCount?: number;
  dialogCount?: number;
  otherControlCount?: number;
  ocrAvailable?: boolean;
  ocrBlockCount?: number;
  ocrTrust?: string;
  ocrInjectionCount?: number;
  ocrBlocker?: string;
  accessibilityAvailable?: boolean;
  accessibilitySourceStatus?: string;
  accessibilityNodeCount?: number;
  accessibilityControlCount?: number;
  atspiSnapshotTotalMs?: number;
  atspiSkippedAppCount?: number;
  atspiOmittedNodeCount?: number;
      accessibilityRemediation: string[];
      screenshotAvailable?: boolean;
  screenshotStatus?: string;
  screenshotCaptureMs?: number;
      screenshotDurationMs?: number;
  screenHashPrefix?: string;
  monitorCount?: number;
  dpiAvailable?: boolean;
  cursorFocusKnown?: boolean;
  focusedWindow?: string;
  observationTotalMs?: number;
  slowestProbe?: string;
  slowestProbeMs?: number;
  probeTimeoutCount?: number;
  probeTimings: GuiCognitionProbeTiming[];
  cacheHit?: boolean;
  cacheAgeMs?: number;
  cachePolicy?: string;
  freshness?: string;
  sourceBlockers: string[];
  activeWindowFallbackChain: GuiCognitionSourceAttempt[];
  activeWindowFailureChain: GuiCognitionSourceAttempt[];
  ocrWaitForScreenshotMs?: number;
  ocrEngineSelected?: string;
  ocrEngineStatus?: string;
  ocrImageStatus?: string;
  ocrTotalMs?: number;
  ocrFastPath?: string;
  ocrCacheHit?: boolean;
  ocrRoiCount?: number;
  ocrChangedRegionCount?: number;
  ocrColdStartMs?: number;
  ocrWarmStartMs?: number;
  ocrBenchmarkSummary?: string;
  executableControlCount?: number;
  visualControlCount?: number;
  visualButtonLikeCount?: number;
  visualMatchedCount?: number;
  visualUnmatchedCount?: number;
  accessibilityOverallStatus?: string;
  accessibilityOverallConfidence?: number;
  accessibilityStaleNodeCount?: number;
  accessibilityTimeoutCount?: number;
  accessibilityCacheHitCount?: number;
  accessibilityStaleCacheRejectedCount?: number;
  focusedApp?: string;
  focusedControlId?: string;
  focusedControlLabel?: string;
  focusedControlRole?: string;
  focusedControlBounds?: Record<string, unknown>;
  textCursorKnown?: boolean;
  editableTargetKnown?: boolean;
  terminalLike?: boolean;
  focusSource?: string;
  focusConfidence?: number;
  focusReliability?: string;
  focusAdapterStatus?: string;
  focusLatencyMs?: number;
  focusFailureChain?: GuiCognitionSourceAttempt[];
}

export interface GuiCognitionContextState {
  contextId?: string;
  observationId?: string;
  previousContextId?: string;
  previousObservationId?: string;
  activeWindow?: string;
  status?: string;
  freshness?: string;
  screenHashPrefix?: string;
  trustedControlCount?: number;
  executableControlCount?: number;
  disabledOrHiddenCount?: number;
  ocrUntrusted?: boolean;
  ocrInjectionCount?: number;
  redactionCount?: number;
  sourceConfidence: Record<string, number>;
  sourceTrust: Record<string, string>;
  deltaSummary: string[];
  sourceBlockers: string[];
  warnings: string[];
  focus?: Record<string, unknown>;
  accessibilityHealth?: Record<string, unknown>;
  visualControls?: Record<string, unknown>;
  ocrPerformance?: Record<string, unknown>;
}

export interface GuiCognitionTargetState {
  label?: string;
  role?: string;
  targetType?: string;
  controlId?: string;
  targetHash?: string;
  bounds?: Record<string, number>;
  enabled?: boolean;
  visible?: boolean;
  focused?: boolean;
  source?: string;
  confidence?: number;
  evidence?: string;
}

export interface GuiCognitionTargetCandidateState {
  candidateId?: string;
  controlId?: string;
  targetHash?: string;
  label?: string;
  role?: string;
  bounds?: Record<string, number>;
  visible?: boolean;
  enabled?: boolean;
  focused?: boolean;
  quality?: string;
  sources: string[];
  confidence?: number;
  rejectionReason?: string;
}

export interface GuiCognitionTargetResolutionState {
  resolutionId?: string;
  planId?: string;
  validationId?: string;
  status?: string;
  confidence?: number;
  candidateCount?: number;
  candidates: GuiCognitionTargetCandidateState[];
  ambiguityCount?: number;
  ambiguityReasons: string[];
  blockerCount?: number;
  blockers: string[];
  canProceedToSafetyGate?: boolean;
  canExecute?: boolean;
  promptHash?: string;
}

export interface GuiCognitionSafetyState {
  safetyGateId?: string;
  proposalId?: string;
  requestId?: string;
  proposalHash?: string;
  targetHash?: string;
  status?: string;
  safetyStatus?: string;
  riskLevel?: string;
  reasons: string[];
  riskReasons?: string[];
  approvalReason?: string;
  blockers?: string[];
  warnings?: string[];
  canRequestHitl?: boolean;
  canAuthorizeStep7?: boolean;
  canExecute?: boolean;
  actionType?: string;
  targetLabel?: string;
  targetRole?: string;
  expectedPostcondition?: string;
  expiresAtMs?: number;
  promptHash?: string;
}

export interface GuiCognitionActionState {
  executionId?: string;
  proposalId?: string;
  proposalHash?: string;
  targetHash?: string;
  actionKind?: string;
  target?: string;
  status?: string;
  backendUsed?: string;
  authorizationSource?: string;
  resultSummary?: string;
  safeErrorSummary?: string;
  canRetry?: boolean;
  recoveryHint?: string;
}

export interface GuiCognitionActionBackendCapabilities {
  observe: boolean;
  focus_field: boolean;
  fill_field: boolean;
  click_control: boolean;
  post_action_observe: boolean;
  verify: boolean;
  recovery_focus: boolean;
  recovery_modal: boolean;
}

export interface GuiCognitionActionBackendState {
  globalHaltEngaged?: boolean;
  haltKind?: string;
  haltReason?: string;
  releaseConditions: string[];
  startupElapsedMs?: number;
  canObserve?: boolean;
  canPlan?: boolean;
  automationEnabled?: boolean;
  visionSidecar?: string;
  uinputDaemon?: string;
  orchestratorAvailable?: boolean;
  sessionType?: string;
  xdotoolAvailable?: boolean;
  ydotoolAvailable?: boolean;
  uinputAvailable?: boolean;
  selectedBackend?: string;
  backendSelectionReason?: string;
  backendProbeStatus?: string;
  backendProbeErrors: string[];
  inputBackendKind?: string;
  focusSupported?: boolean;
  typingSupported?: boolean;
  clickSupported?: boolean;
  verificationSupported?: boolean;
  xdotoolUsableForActions?: boolean;
  ydotoolUsableForActions?: boolean;
  uinputSocketPath?: string;
  uinputSocketAccessible?: boolean;
  canExecuteActions?: boolean;
  blockers: string[];
  capabilities?: Partial<GuiCognitionActionBackendCapabilities>;
}

export interface GuiCognitionVerificationState {
  status?: string;
  confidence?: number;
  summary?: string;
  verificationStrategy?: string;
  evidence?: string[];
  preStateSummary?: string;
  postStateSummary?: string;
  matchedExpectedState?: boolean;
  targetStillPresent?: boolean;
  targetIdentityMatches?: boolean;
  safeErrorSummary?: string;
  recoveryHint?: string;
  canRetry?: boolean;
}

export interface GuiCognitionBlockerState {
  type: "plan" | "target" | "execution" | "turn";
  reason: string;
  clarificationQuestion?: string;
  options: string[];
  candidateCount?: number;
}

export interface GuiCognitionWorkflowStepView {
  stepId?: string;
  stepIndex: number;
  stepType?: string;
  status: string;
  blockers?: string[];
  warnings?: string[];
  receiptId?: string;
}

export interface GuiCognitionWorkflowState {
  workflowRunId?: string;
  status?: string;
  currentStepIndex?: number;
  stepCount?: number;
  completedStepCount?: number;
  blockedReason?: string;
  riskLevel?: string;
  requiresUserApproval?: boolean;
  executionMode?: string;
  steps: GuiCognitionWorkflowStepView[];
}

export interface GuiCognitionCheckpointState {
  checkpointId?: string;
  checkpointHashPrefix?: string;
  currentStepIndex?: number;
  stepCount?: number;
  completedStepCount?: number;
  pendingStepId?: string;
  requiresUserApproval?: boolean;
  canResume?: boolean;
  resumeStatus?: string;
  resumeReason?: string;
  nextStepId?: string;
  invalidatedApprovals?: string[];
  duplicateActionGuards?: string[];
  resumeExplanation?: string;
}

export interface GuiCognitionRecoveryState {
  status?: string;
  failureKind?: string;
  recoveryActionKind?: string;
  proposedRecoveryStep?: string;
  requiresUserApproval?: boolean;
  canRecover?: boolean;
  canExecuteRecovery?: boolean;
  retryCount?: number;
  maxRetryCount?: number;
  blockers?: string[];
  warnings?: string[];
  safeExplanation?: string;
  recoveryHint?: string;
  verificationResult?: string;
  nextRecommendedState?: string;
  canRetryOriginalAction?: boolean;
  canContinueWorkflow?: boolean;
}

export interface GuiCognitionApprovalState {
  requestId?: string;
  proposalId?: string;
  proposalHash?: string;
  targetHash?: string;
  actionType?: string;
  targetLabel?: string;
  targetRole?: string;
  reason?: string;
  riskLevel?: string;
  riskReasons?: string[];
  expectedPostcondition?: string;
  expiresAtMs?: number;
  canAuthorizeStep7?: boolean;
  canExecute?: boolean;
}

export interface GuiCognitionHitlDecisionState {
  decisionId?: string;
  requestId?: string;
  proposalId?: string;
  proposalHash?: string;
  targetHash?: string;
  decision?: string;
  decidedAtMs?: number;
  decisionReason?: string;
  actor?: string;
  userVisibleSummaryHash?: string;
  canAuthorizeStep7?: boolean;
  canExecute?: boolean;
}

export interface GuiCognitionGoalAmbiguityState {
  kind?: string;
  field?: string;
  message: string;
}

export interface GuiCognitionGoalEvidenceState {
  source?: string;
  field?: string;
  summary?: string;
  confidence?: number;
}

export interface GuiCognitionGoalContractState {
  contractId?: string;
  observationId?: string;
  contextId?: string;
  goalSummary?: string;
  promptHash?: string;
  intentKind?: string;
  actionType?: string;
  targetAppKind?: string;
  targetAppHint?: string;
  targetWindowHint?: string;
  targetControlHint?: string;
  querySummary?: string;
  queryHash?: string;
  textPayloadSummary?: string;
  textPayloadHash?: string;
  desiredFinalState?: string;
  riskLevel?: string;
  requiresUserApproval?: boolean;
  ambiguityCount?: number;
  ambiguities: GuiCognitionGoalAmbiguityState[];
  sourceEvidence: GuiCognitionGoalEvidenceState[];
  extractionConfidence?: number;
  extractorMode?: string;
}

export interface GuiCognitionTypedPlanStepState {
  stepId?: string;
  stepType?: string;
  summary?: string;
  targetAppHint?: string;
  targetWindowHint?: string;
  targetControlHint?: string;
  textPayloadSummary?: string;
  textPayloadHash?: string;
  expectedPrecondition?: string;
  expectedPostcondition?: string;
  verificationStrategy?: string;
  riskLevel?: string;
  requiresApproval?: boolean;
  allowedToExecute?: boolean;
  confidence?: number;
  reason?: string;
}

export interface GuiCognitionPlanStepValidationState {
  stepId?: string;
  stepType?: string;
  status?: string;
  riskLevel?: string;
  requiresApproval?: boolean;
  targetResolutionRequired?: boolean;
  targetAvailable?: boolean;
  verificationPresent?: boolean;
  preconditionStatus?: string;
  postconditionStatus?: string;
  blocker?: string;
  confidence?: number;
}

export interface GuiCognitionSessionState {
  lifecycle: GuiCognitionLifecycle;
  sessionId?: string;
  turnId?: string;
  workflowId?: string;
  lastSequence: number;
  startedAt?: number;
  updatedAt?: number;
  finalStatus?: string;
  routePath?: string;
  llmToolLoop?: boolean;
  observation: GuiCognitionObservationState;
  context: GuiCognitionContextState;
  goalContract?: GuiCognitionGoalContractState;
  goalSummary?: string;
  intentKind?: string;
  plannerMode?: string;
  llmAttempted?: boolean;
  llmStatus?: string;
  llmFailureReason?: string;
  plannerConfidence?: number;
  planId?: string;
  planGoalContractId?: string;
  planContextId?: string;
  planPromptHash?: string;
  planGoalActionType?: string;
  planStatus?: string;
  planAmbiguityCount?: number;
  planValidationStatus?: string;
  planReadinessStatus?: string;
  planCanProceedToTargetResolution?: boolean;
  planCanExecute?: boolean;
  planValidationBlockerCount?: number;
  planValidationWarningCount?: number;
  planBlockedReasons: string[];
  planWarnings: string[];
  planStepValidationResults: GuiCognitionPlanStepValidationState[];
  planRequiresUserApproval?: boolean;
  planSummary?: string;
  planSteps: string[];
  typedPlanSteps: GuiCognitionTypedPlanStepState[];
  planSourceEvidence: GuiCognitionGoalEvidenceState[];
  riskLevel?: string;
  targetResolution?: GuiCognitionTargetResolutionState;
  target?: GuiCognitionTargetState;
  safetyDecision?: GuiCognitionSafetyState;
  actionBackend?: GuiCognitionActionBackendState;
  pendingApproval?: GuiCognitionApprovalState;
  hitlDecision?: GuiCognitionHitlDecisionState;
  currentAction?: GuiCognitionActionState;
  executionReceipt?: GuiCognitionActionState;
  verification?: GuiCognitionVerificationState;
  blocker?: GuiCognitionBlockerState;
  recoveryOptions: string[];
  recovery?: GuiCognitionRecoveryState;
  workflow?: GuiCognitionWorkflowState;
  checkpoint?: GuiCognitionCheckpointState;
}

export interface GuiCognitionHitlMetadata {
  kind?: string;
  proposal_id?: string;
  proposal_hash?: string;
  workflow_id?: string;
  action_kind?: string;
  target_label?: string;
  target_role?: string;
  active_window?: string;
  risk_level?: string;
  consequence?: string;
  action_hash?: string;
  target_hash?: string;
  expires_at_ms?: number;
  evidence_summary?: string;
  can_execute?: boolean;
}
