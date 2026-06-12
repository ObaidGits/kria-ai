pub mod checkpoint;
pub mod context;
pub mod executor;
pub mod goal_contract;
pub mod llm_planner;
pub mod perception;
pub mod planner;
pub mod recovery;
pub mod resolver;
pub mod safety;
pub mod safety_hitl;
pub mod validator;
pub mod verifier;
pub mod workflow_runtime;

use uuid::Uuid;

use self::context::{GuiContext, GuiContextBuildRequest, GuiContextBuilder};
use self::executor::{
    build_execution_request_from_proposal, validate_execution_preconditions, GuiActionBackendStatus,
    GuiActionExecution, GuiActionExecutor, GuiActionKind, GuiActionRequest, GuiExecutionMode,
    GuiExecutionAuthorizationSource, GuiExecutionPreconditionReport, GuiExecutionResult,
    GuiPayloadVault,
};
use self::goal_contract::{extract_gui_goal_contract, GuiGoalContract};
use self::llm_planner::{
    parse_llm_plan, plan_summary_json, planner_summary_json, typed_plan_steps, validate_llm_plan,
    validate_plan_for_resolution, GuiLlmPlanner, GuiLlmPlannerRequest, GuiPlanValidationReport,
    GuiPlanValidationStatus, GuiPlannerMode, GuiPlannerSelection,
};
use self::perception::{
    collect_observation, control_sample, stable_hash, GuiObservationSnapshot,
    GuiPerceptionProvider,
};
use self::planner::{
    gui_plan_steps, intent_from_goal_contract, GuiCognitionIntent, GuiCognitionIntentKind,
};
use self::recovery::{
    assess_recovery, recovery_blocked_event, should_attempt_recovery, GuiBlocker,
    GuiRecoveryActionKind, GuiRecoveryInput, GuiRecoveryResult, GuiRecoverySignals,
};
use self::resolver::{
    resolve_button, resolve_plan_targets, resolve_type_text_target, resolve_unique_text_field,
    GuiTargetResolutionSummary, TargetResolution,
};
use self::safety::{safety_for_intent, GuiSafetyStatus};
use self::safety_hitl::{
    build_action_proposal, decision_from_fixture, evaluate_safety_gate, now_ms,
    GuiActionProposal, GuiHitlDecision, GuiHitlDecisionFixture,
};
use self::validator::validate_intent;
use self::verifier::{
    select_verification_strategy, verify_post_action, verify_post_action_detailed,
    GuiPostActionVerificationRequest, GuiPostActionVerificationResult, GuiVerificationReport,
};
use self::workflow_runtime::{
    step_blocked_event, step_completed_event, step_started_event, workflow_step_kind,
    workflow_step_requires_target, GuiWorkflowRun, GuiWorkflowStepKind, GuiWorkflowStepReceipt,
};
use self::checkpoint::{
    build_checkpoint, checkpoint_hash, validate_resume, GuiCheckpointPending,
    GuiResumeObservationSignals, GuiWorkflowResumeRequest,
};

#[derive(Debug, Clone)]
pub struct GuiTurnRequest {
    pub session_id: String,
    pub turn_id: String,
    pub workflow_id: String,
    pub message: String,
    pub route_path: String,
    pub llm_tool_loop: bool,
    pub hitl_decision_fixture: Option<GuiHitlDecisionFixture>,
    pub execution_mode: GuiExecutionMode,
    #[doc = "Step 10: when true, run the multi-step workflow runtime instead of \
             the single-proposal path. Defaults to false to preserve Step 1-9 behavior."]
    pub workflow_enabled: bool,
    #[doc = "Step 11: when set, resume the workflow from this checkpoint instead \
             of starting fresh. The runtime re-observes and revalidates first."]
    pub resume_checkpoint: Option<self::checkpoint::GuiWorkflowCheckpoint>,
    pub resume_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GuiTurnOutcome {
    pub status: String,
    pub reply: String,
    pub response: serde_json::Value,
    pub events: Vec<serde_json::Value>,
}

pub struct GuiCognitionRuntime<'a, P, E>
where
    P: GuiPerceptionProvider,
    E: GuiActionExecutor,
{
    perception: &'a P,
    executor: &'a E,
    llm_planner: Option<&'a dyn GuiLlmPlanner>,
}

impl<'a, P, E> GuiCognitionRuntime<'a, P, E>
where
    P: GuiPerceptionProvider,
    E: GuiActionExecutor,
{
    pub fn new(perception: &'a P, executor: &'a E) -> Self {
        Self {
            perception,
            executor,
            llm_planner: None,
        }
    }

    pub fn with_llm_planner(mut self, planner: Option<&'a dyn GuiLlmPlanner>) -> Self {
        self.llm_planner = planner;
        self
    }

    pub async fn run_turn(&self, request: GuiTurnRequest) -> GuiTurnOutcome {
        let mut events = Vec::new();
        events.push(serde_json::json!({
            "type": "TurnStarted",
            "mode_id": "gui_cognition",
        }));
        events.push(serde_json::json!({
            "type": "RouteConfirmed",
            "path": request.route_path,
            "llm_tool_loop": request.llm_tool_loop,
        }));

        let observation = self.observe_with_events(&mut events).await;
        let context =
            GuiContextBuilder::new().build(GuiContextBuildRequest::new(observation.clone()));
        events.push(context.context_built_event());
        let action_backend = self.executor.action_backend_status().await;
        events.push(action_backend_event(&action_backend));

        let lower_message = request.message.to_lowercase();
        let goal_report = extract_gui_goal_contract(&request.message, Some(&context));
        let goal_contract = goal_report.contract;
        let intent = intent_from_goal_contract(&request.message, &goal_contract, &lower_message);
        let deterministic_steps = gui_plan_steps(&intent, &context.observation);
        let plan_id = Uuid::new_v4().to_string();

        events.push(goal_contract.event_payload());
        let planner_request = GuiLlmPlannerRequest::from_context(
            &goal_contract,
            &context,
            deterministic_steps.clone(),
        );
        let planner_selection = self
            .select_plan_with_optional_llm(&mut events, &planner_request, &intent, &context)
            .await;
        let mut plan_event = plan_summary_json(&plan_id, &planner_selection);
        if let Some(object) = plan_event.as_object_mut() {
            object.insert("type".into(), serde_json::json!("PlanCreated"));
        }
        events.push(plan_event);
        let readiness_validation =
            validate_plan_for_resolution(&planner_selection.plan, &planner_request, &plan_id);
        events.push(readiness_validation.event_payload(&plan_id));

        let mut state = RuntimeState::new(gui_observation_reply(&context.observation));
        state.action_backend = Some(action_backend);

        if request.workflow_enabled
            && matches!(
                readiness_validation.status,
                GuiPlanValidationStatus::Valid | GuiPlanValidationStatus::ApprovalRequired
            )
        {
            self.run_workflow(
                &mut events,
                &request,
                &context,
                &goal_contract,
                &planner_selection.plan,
                &readiness_validation,
                &plan_id,
                &mut state,
            )
            .await;

            events.push(serde_json::json!({
                "type": "TurnCompleted",
                "status": state.status,
            }));

            let response = self.response_json(
                &request,
                &context,
                &goal_contract,
                &intent,
                &plan_id,
                &planner_selection,
                &readiness_validation,
                &state,
            );

            return GuiTurnOutcome {
                status: state.status,
                reply: state.reply,
                response,
                events,
            };
        }

        match readiness_validation.status {
            GuiPlanValidationStatus::Valid => {
                self.handle_target_resolution_only(
                    &mut events,
                    &context,
                    &planner_selection.plan,
                    &readiness_validation,
                    &plan_id,
                    &mut state,
                );
            }
            GuiPlanValidationStatus::ApprovalRequired => {
                self.handle_target_resolution_only(
                    &mut events,
                    &context,
                    &planner_selection.plan,
                    &readiness_validation,
                    &plan_id,
                    &mut state,
                );
            }
            GuiPlanValidationStatus::NeedsClarification
            | GuiPlanValidationStatus::Blocked
            | GuiPlanValidationStatus::Rejected => {
                state.status = "blocked".into();
                let reason = readiness_validation
                    .blocked_reasons
                    .first()
                    .cloned()
                    .or_else(|| planner_selection.plan.clarification_question.clone())
                    .unwrap_or_else(|| "Plan validation blocked target resolution.".into());
                let clarification_question = if matches!(
                    readiness_validation.status,
                    GuiPlanValidationStatus::NeedsClarification
                ) {
                    planner_selection
                        .plan
                        .clarification_question
                        .clone()
                        .or_else(|| Some("Which exact visible target should I use?".into()))
                } else {
                    planner_selection.plan.clarification_question.clone()
                };
                state.blocker = Some(GuiBlocker::new("plan_validation", reason.clone()));
                events.push(serde_json::json!({
                    "type": "PlanBlocked",
                    "reason": reason,
                    "clarification_question": clarification_question,
                }));
                state.reply =
                    "Plan validation blocked target resolution, so I stopped before execution."
                        .into();
                self.handle_target_resolution_only(
                    &mut events,
                    &context,
                    &planner_selection.plan,
                    &readiness_validation,
                    &plan_id,
                    &mut state,
                );
            }
        }

        self.handle_safety_gate(
            &mut events,
            &request,
            &context,
            &goal_contract,
            &planner_selection.plan,
            &readiness_validation,
            &plan_id,
            &mut state,
        )
        .await;

        events.push(serde_json::json!({
            "type": "TurnCompleted",
            "status": state.status,
        }));

        let response = self.response_json(
            &request,
            &context,
            &goal_contract,
            &intent,
            &plan_id,
            &planner_selection,
            &readiness_validation,
            &state,
        );

        GuiTurnOutcome {
            status: state.status,
            reply: state.reply,
            response,
            events,
        }
    }

    async fn select_plan_with_optional_llm(
        &self,
        events: &mut Vec<serde_json::Value>,
        request: &GuiLlmPlannerRequest,
        intent: &GuiCognitionIntent,
        context: &GuiContext,
    ) -> GuiPlannerSelection {
        let Some(planner) = self.llm_planner else {
            events.push(serde_json::json!({
                "type": "LlmPlanningFailed",
                "status": "unavailable",
                "reason": "LLM planner backend unavailable; deterministic plan used.",
            }));
            return GuiPlannerSelection::deterministic(request, intent, context);
        };

        events.push(serde_json::json!({
            "type": "LlmPlanningStarted",
            "planner_mode": "llm_schema",
            "context_id": request.context_id,
            "observation_id": request.observation_id,
        }));

        match planner.plan(request.clone()).await {
            Ok(raw) => match parse_llm_plan(&raw.content) {
                Ok(plan) => {
                    let validation = validate_llm_plan(&plan, request);
                    events.push(serde_json::json!({
                        "type": "LlmPlanningCompleted",
                        "status": validation.status.as_str(),
                        "model": raw.model,
                        "confidence": plan.confidence,
                        "step_count": plan.typed_steps.len().max(plan.steps.len()),
                        "risk_level": plan.risk_level,
                    }));
                    if matches!(validation.status, GuiPlanValidationStatus::Blocked) {
                        let reason =
                            validation
                                .blocked_reasons
                                .first()
                                .cloned()
                                .unwrap_or_else(|| {
                                    "LLM plan rejected by deterministic validator.".into()
                                });
                        events.push(serde_json::json!({
                            "type": "LlmPlanningFailed",
                            "status": "rejected",
                            "reason": reason,
                        }));
                        let mut fallback = GuiPlannerSelection::deterministic_fallback(
                            request,
                            intent,
                            context,
                            true,
                            "rejected",
                            validation.blocked_reasons.join("; "),
                        );
                        fallback
                            .validation
                            .warnings
                            .extend(validation.blocked_reasons);
                        fallback
                    } else {
                        GuiPlannerSelection {
                            mode: GuiPlannerMode::LlmAssisted,
                            llm_attempted: true,
                            llm_status: "completed".into(),
                            llm_failure_reason: None,
                            raw_model: raw.model,
                            plan,
                            validation,
                        }
                    }
                }
                Err(error) => {
                    let reason = sanitize_event_text(&error);
                    events.push(serde_json::json!({
                        "type": "LlmPlanningFailed",
                        "status": "rejected",
                        "reason": reason,
                    }));
                    GuiPlannerSelection::deterministic_fallback(
                        request, intent, context, true, "rejected", reason,
                    )
                }
            },
            Err(error) => {
                let reason = error.safe_reason();
                events.push(serde_json::json!({
                    "type": "LlmPlanningFailed",
                    "status": if reason.contains("unavailable") { "unavailable" } else { "failed" },
                    "reason": reason,
                }));
                GuiPlannerSelection::deterministic_fallback(
                    request,
                    intent,
                    context,
                    true,
                    "failed",
                    error.safe_reason(),
                )
            }
        }
    }

    async fn observe_with_events(
        &self,
        events: &mut Vec<serde_json::Value>,
    ) -> GuiObservationSnapshot {
        events.push(serde_json::json!({
            "type": "ObservationStarted",
            "cache_policy": self.perception.observation_cache_policy(),
            "sources": [
                "get_active_window",
                "get_desktop_state",
                "get_accessibility_capabilities",
                "accessibility_tree_summary",
                "capture_screenshot",
                "ocr",
                "monitor_layout",
                "cursor_focus",
                "find_ui_elements"
            ],
        }));
        let observation = collect_observation(
            self.perception,
            Uuid::new_v4().to_string(),
            Uuid::new_v4().to_string(),
        )
        .await;
        if !observation.has_useful_signal() {
            events.push(serde_json::json!({
                "type": "ObservationBlocked",
                "reason": "no_useful_perception_source",
                "blockers": {
                    "active_window": observation.capabilities.active_window.blocker,
                    "desktop_state": observation.capabilities.desktop_state.blocker,
                    "accessibility": observation.capabilities.accessibility.blocker,
                    "screenshot": observation.capabilities.screenshot.blocker,
                    "ocr": observation.capabilities.ocr.blocker,
                    "monitor": observation.capabilities.monitor.blocker,
                    "cursor_focus": observation.capabilities.cursor_focus.blocker,
                },
            }));
        }
        let source_blockers = source_blockers_json(&observation);
        events.push(observation_completed_event(&observation, source_blockers));
        observation
    }

    #[allow(dead_code)]
    async fn handle_intent(
        &self,
        events: &mut Vec<serde_json::Value>,
        context: &GuiContext,
        intent: &GuiCognitionIntent,
        state: &mut RuntimeState,
    ) {
        let validation = validate_intent(intent, context);

        match intent.kind {
            GuiCognitionIntentKind::Observe => {
                state.reply.push_str(
                    " No click, typing, submit, delete, or external action was executed.",
                );
            }
            GuiCognitionIntentKind::AnalyzePlan
            | GuiCognitionIntentKind::BrowserSearchPlan
            | GuiCognitionIntentKind::FillFormPlan
            | GuiCognitionIntentKind::AmbiguityCheck
            | GuiCognitionIntentKind::FocusRecovery => {
                let plan_text = gui_plan_steps(intent, &context.observation)
                    .iter()
                    .enumerate()
                    .map(|(idx, step)| format!("{}. {}", idx + 1, step))
                    .collect::<Vec<_>>()
                    .join(" ");
                state.reply = format!(
                    "{} Planned safely: {} No GUI action was executed in this planning/validation response.",
                    gui_observation_reply(&context.observation),
                    plan_text
                );
            }
            GuiCognitionIntentKind::TargetAvailabilityCheck => {
                state.status = "blocked".into();
                let reason = "No concrete target was provided for resolution, so GUI Cognition cannot safely choose or act.";
                events.push(serde_json::json!({
                    "type": "PlanBlocked",
                    "reason": "missing_or_ambiguous_target",
                    "clarification_question": "Which exact visible target should I use?",
                    "options": control_sample(&context.observation.buttons, 6),
                }));
                state.blocker = Some(
                    GuiBlocker::new("target_resolution", reason).with_candidate_count(
                        context.observation.buttons.len() + context.observation.text_fields.len(),
                    ),
                );
                state.reply =
                    format!("{reason} I stopped safely and did not execute any GUI action.");
            }
            GuiCognitionIntentKind::RiskApproval => {
                self.emit_approval_required(events, intent, state, "This request can affect external state or sensitive data, so GUI Cognition paused before execution.");
                state.reply = format!(
                    "{} Safety gate result: approval required. Reason: {}. I did not execute the risky action.",
                    gui_observation_reply(&context.observation),
                    state.blocker
                        .as_ref()
                        .map(|blocker| blocker.options.join("; "))
                        .unwrap_or_else(|| "approval required".into())
                );
            }
            GuiCognitionIntentKind::FocusInput | GuiCognitionIntentKind::SafeAction => {
                self.handle_focus_intent(events, context, state).await;
            }
            GuiCognitionIntentKind::TypeText => {
                if !validation.reasons.is_empty() {
                    self.handle_type_validation_block(
                        events,
                        intent,
                        state,
                        &validation.reasons[0],
                    );
                } else {
                    self.handle_type_intent(events, context, intent, state)
                        .await;
                }
            }
            GuiCognitionIntentKind::ClickControl => {
                if intent.control_name.is_none() {
                    self.handle_missing_click_target(events, state);
                } else {
                    self.handle_click_intent(events, context, intent, state)
                        .await;
                }
            }
        }
    }

    fn handle_target_resolution_only(
        &self,
        events: &mut Vec<serde_json::Value>,
        context: &GuiContext,
        plan: &self::llm_planner::GuiLlmPlan,
        readiness_validation: &GuiPlanValidationReport,
        plan_id: &str,
        state: &mut RuntimeState,
    ) {
        events.push(serde_json::json!({
            "type": "TargetResolutionStarted",
            "plan_id": plan_id,
            "validation_id": readiness_validation.validation_id.as_deref(),
            "mode": "step5_target_resolver",
        }));
        let can_resolve_for_approval = matches!(
            readiness_validation.status,
            GuiPlanValidationStatus::ApprovalRequired
        ) || readiness_validation.readiness_status.as_deref() == Some("approval_required");
        let summary = if readiness_validation.can_proceed_to_target_resolution || can_resolve_for_approval {
            resolve_plan_targets(plan, readiness_validation, context, plan_id)
        } else {
            GuiTargetResolutionSummary::skipped(
                plan,
                readiness_validation,
                context,
                plan_id,
                readiness_validation
                    .blocked_reasons
                    .first()
                    .cloned()
                    .unwrap_or_else(|| {
                        "Plan validation did not allow Step 5 target resolution.".into()
                    }),
            )
        };
        events.push(summary.event_payload());
        state.target_resolution = Some(summary.summary_json());
        if let Some(target) = &summary.resolved_target {
            state.target = Some(serde_json::json!({
                "label": target.label,
                "role": target.role,
                "target_type": target.target_kind,
                "control_id": target.control_id,
                "target_hash": target.target_hash,
                "bounds": target.bounds.clone(),
                "confidence": summary.confidence,
                "can_execute": false,
            }));
        }
        match summary.status.as_str() {
            "resolved" => {
                state.status = "ok".into();
                state.reply = format!(
                    "{} Target resolution completed for Step 5. I did not execute any GUI action.",
                    gui_observation_reply(&context.observation)
                );
            }
            "ambiguous" | "needs_clarification" | "blocked" | "rejected" => {
                state.status = "blocked".into();
                let reason = summary
                    .ambiguity_reasons
                    .first()
                    .cloned()
                    .or_else(|| summary.blockers.first().cloned())
                    .unwrap_or_else(|| {
                        "Target resolution did not find a safe unique target.".into()
                    });
                state.blocker = Some(GuiBlocker::new("target_resolution", reason));
                state.reply =
                    "Target resolution stopped before execution because the target is not safely resolved."
                        .into();
            }
            _ => {
                if state.status != "needs_approval" {
                    state.reply =
                        "Target resolution was skipped after plan validation. I did not execute any GUI action."
                            .into();
                }
            }
        }
    }

    async fn handle_safety_gate(
        &self,
        events: &mut Vec<serde_json::Value>,
        request: &GuiTurnRequest,
        context: &GuiContext,
        goal_contract: &GuiGoalContract,
        plan: &self::llm_planner::GuiLlmPlan,
        readiness_validation: &GuiPlanValidationReport,
        plan_id: &str,
        state: &mut RuntimeState,
    ) {
        let Some(target_resolution_value) = &state.target_resolution else {
            return;
        };
        let target_resolution: GuiTargetResolutionSummary =
            match serde_json::from_value(target_resolution_value.clone()) {
                Ok(summary) => summary,
                Err(_) => return,
            };
        if target_resolution.status == "skipped"
            && !goal_contract.requires_user_approval
            && !matches!(
                readiness_validation.status,
                GuiPlanValidationStatus::ApprovalRequired
            )
        {
            if state.status == "ok" {
                state.reply =
                    "Plan has no action target for Step 6 safety gating. I did not execute any GUI action."
                        .into();
            }
            return;
        }
        events.push(serde_json::json!({
            "type": "SafetyGateStarted",
            "plan_id": plan_id,
            "resolution_id": target_resolution.resolution_id.clone(),
            "mode": "step6_safety_hitl",
            "can_execute": false,
            "prompt_hash": goal_contract.prompt_hash.clone(),
        }));
        let proposal = build_action_proposal(
            &request.session_id,
            &request.workflow_id,
            goal_contract,
            plan_id,
            plan,
            readiness_validation,
            &target_resolution,
            context,
            now_ms(),
        );
        let safety_gate = evaluate_safety_gate(proposal, &target_resolution);
        events.push(safety_gate.event_payload());
        state.safety_gate = Some(safety_gate.summary_json());

        match safety_gate.status.as_str() {
            "safe_no_approval_required" => {
                if request.execution_mode.allows_execution() {
                    self.execute_authorized_proposal(
                        events,
                        context,
                        &safety_gate.proposal,
                        &target_resolution,
                        None,
                        GuiExecutionAuthorizationSource::SafeNoApprovalRequired,
                        request.execution_mode,
                        state,
                    )
                    .await;
                } else {
                    state.status = "ok".into();
                    state.reply = format!(
                        "{} Safety gate completed for Step 6. This low-risk proposal is authorized for Step 7 review only; I did not execute any GUI action.",
                        gui_observation_reply(&context.observation)
                    );
                }
            }
            "approval_required" => {
                events.push(safety_gate.hitl_required_event());
                state.status = "needs_approval".into();
                state.blocker = Some(
                    GuiBlocker::new(
                        "approval_required",
                        safety_gate
                            .approval_reason
                            .clone()
                            .unwrap_or_else(|| "GUI action requires approval".into()),
                    )
                    .with_options(safety_gate.proposal.risk_reasons.clone()),
                );
                state.reply = format!(
                    "{} Safety gate paused because approval required. Approval authorizes only the same fresh bound proposal for Step 7; I did not execute any GUI action.",
                    gui_observation_reply(&context.observation)
                );
                if let Some(fixture) = &request.hitl_decision_fixture {
                    let decision = decision_from_fixture(&safety_gate.proposal, fixture, now_ms());
                    if matches!(
                        decision.decision.as_str(),
                        "stale_rejected" | "hash_mismatch_rejected" | "expired"
                    ) {
                        events.push(decision.invalidated_event_payload());
                    } else {
                        events.push(decision.event_payload());
                    }
                    if decision.can_authorize_step7 {
                        state.status = "approved_for_step7".into();
                        if request.execution_mode.allows_execution() {
                            self.execute_authorized_proposal(
                                events,
                                context,
                                &safety_gate.proposal,
                                &target_resolution,
                                Some(&decision),
                                GuiExecutionAuthorizationSource::HitlApproved,
                                request.execution_mode,
                                state,
                            )
                            .await;
                        }
                    } else if decision.decision == "denied" {
                        state.status = "blocked".into();
                    }
                    state.hitl_decision = Some(decision.summary_json());
                }
            }
            "blocked" | "rejected" | "stale" => {
                state.status = "blocked".into();
                let reason = safety_gate
                    .blockers
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "Safety gate blocked this GUI proposal.".into());
                state.blocker = Some(GuiBlocker::new("safety_gate", reason.clone()));
                state.reply = format!("{reason} I did not execute any GUI action.");
            }
            _ => {}
        }
    }

    async fn execute_authorized_proposal(
        &self,
        events: &mut Vec<serde_json::Value>,
        context: &GuiContext,
        proposal: &GuiActionProposal,
        target_resolution: &GuiTargetResolutionSummary,
        hitl_decision: Option<&GuiHitlDecision>,
        authorization_source: GuiExecutionAuthorizationSource,
        execution_mode: GuiExecutionMode,
        state: &mut RuntimeState,
    ) {
        let Some(backend) = state.action_backend.clone() else {
            return;
        };
        let pre_observation = context.observation.clone();
        let now = now_ms();
        let mut payload_vault = GuiPayloadVault::default();
        let execution_request = build_execution_request_from_proposal(
            proposal,
            target_resolution,
            authorization_source,
            hitl_decision.map(|decision| decision.decision_id.clone()),
            &mut payload_vault,
            now,
        );
        let precondition = validate_execution_preconditions(
            execution_mode,
            &execution_request,
            proposal,
            target_resolution,
            &backend,
            hitl_decision,
            &payload_vault,
            now,
        );
        if !precondition.can_start_action {
            let reason = precondition
                .blockers
                .first()
                .cloned()
                .unwrap_or_else(|| "Execution precondition blocked action.".into());
            let result = GuiExecutionResult {
                execution_id: execution_request.execution_id.clone(),
                proposal_id: execution_request.proposal_id.clone(),
                proposal_hash: execution_request.proposal_hash.clone(),
                action_type: execution_request.action_type.clone(),
                status: if reason.contains("expired") {
                    "stale_rejected".into()
                } else {
                    "blocked".into()
                },
                started_at_ms: now,
                completed_at_ms: now,
                backend_used: backend.selected_backend.clone(),
                precondition_check: precondition,
                postcondition_check: execution_request.expected_postcondition.clone(),
                verification_result: "blocked_before_action".into(),
                error_code: Some("precondition_blocked".into()),
                safe_error_summary: Some(sanitize_event_text(&reason)),
                can_retry: false,
                recovery_hint: Some("Re-observe, re-resolve the target, and request a fresh authorization.".into()),
                prompt_hash: execution_request.prompt_hash.clone(),
            };
            events.push(result.blocked_event_payload(&backend));
            events.push(result.verification_event_payload());
            state.status = "blocked".into();
            state.execution_blocker = Some(result.summary_json());
            state.execution_result = Some(result.summary_json());
            state.blocker = Some(GuiBlocker::new("execution", reason.clone()));
            state.reply = format!("{reason} I did not execute any GUI action.");
            return;
        }

        let action_kind = GuiActionKind::from_action_type(&execution_request.action_type);
        let payload_value = match (
            execution_request.text_payload_handle.as_deref(),
            execution_request.text_payload_hash.as_deref(),
        ) {
            (Some(handle), Some(hash)) => payload_vault
                .get(handle, &execution_request.proposal_id, hash, now)
                .map(str::to_string),
            _ => None,
        };
        let is_secret_payload =
            execution_request.text_payload_hash.is_some() && payload_value.is_none();
        let expected_text = payload_value.clone();
        let target_name = proposal
            .target_label
            .clone()
            .or_else(|| proposal.target_control_id.clone())
            .unwrap_or_default();
        let role = proposal
            .target_role
            .clone()
            .unwrap_or_else(|| role_for_action(&action_kind).into());
        let action_request = GuiActionRequest {
            kind: action_kind.clone(),
            role,
            target_name: target_name.clone(),
            value: payload_value,
            execution_hint: execution_hint_for_action(&action_kind).into(),
        };

        events.push(serde_json::json!({
            "type": "ActionStarted",
            "execution_id": execution_request.execution_id,
            "proposal_id": execution_request.proposal_id,
            "proposal_hash": execution_request.proposal_hash,
            "target_hash": execution_request.target_hash,
            "action_kind": execution_request.action_type,
            "target": target_name,
            "backend_used": backend.selected_backend,
            "authorization_source": execution_request.authorization_source.as_str(),
            "prompt_hash": execution_request.prompt_hash,
        }));

        let started_at_ms = now_ms();
        let execution = self.executor.execute(action_request).await;
        let completed_at_ms = now_ms();
        let post_observation = self.observe_with_events(events).await;

        let resolved = target_resolution.resolved_target.as_ref();
        let verification_strategy =
            select_verification_strategy(&action_kind, is_secret_payload);
        let verification_request = GuiPostActionVerificationRequest {
            verification_id: format!(
                "verification-{}",
                stable_hash(&format!(
                    "{}|{}|{}",
                    execution_request.execution_id, execution_request.proposal_hash, completed_at_ms
                ))
            ),
            execution_id: execution_request.execution_id.clone(),
            proposal_id: execution_request.proposal_id.clone(),
            proposal_hash: execution_request.proposal_hash.clone(),
            action_type: execution_request.action_type.clone(),
            target_hash: execution_request.target_hash.clone(),
            stable_target_identity_hash: execution_request.stable_target_identity_hash.clone(),
            expected_postcondition: execution_request.expected_postcondition.clone(),
            verification_strategy: verification_strategy.as_str().into(),
            pre_action_context_id: pre_observation.context_id.clone(),
            post_action_observation_id: post_observation.observation_id.clone(),
            post_action_context_id: post_observation.context_id.clone(),
            started_at_ms,
            is_secret_payload,
            prompt_hash: execution_request.prompt_hash.clone(),
            target_label: proposal.target_label.clone(),
            target_role: proposal.target_role.clone(),
            target_control_id: proposal.target_control_id.clone(),
            expected_app_hint: resolved.and_then(|target| target.app_hint.clone()),
            expected_window_hint: resolved.and_then(|target| target.window_hint.clone()),
        };
        let verification = verify_post_action_detailed(
            &verification_request,
            &pre_observation,
            &post_observation,
            execution.success,
            if is_secret_payload {
                None
            } else {
                expected_text.as_deref()
            },
            completed_at_ms,
        );

        let result = GuiExecutionResult {
            execution_id: execution_request.execution_id.clone(),
            proposal_id: execution_request.proposal_id.clone(),
            proposal_hash: execution_request.proposal_hash.clone(),
            action_type: execution_request.action_type.clone(),
            status: if execution.success {
                "completed".into()
            } else {
                "failed".into()
            },
            started_at_ms,
            completed_at_ms,
            backend_used: execution.tool.clone(),
            precondition_check: GuiExecutionPreconditionReport::allowed(started_at_ms, Vec::new()),
            postcondition_check: execution_request.expected_postcondition.clone(),
            verification_result: verification.status.clone(),
            error_code: execution.error.as_ref().map(|_| "backend_failed".into()),
            safe_error_summary: execution
                .error
                .as_ref()
                .map(|value| sanitize_event_text(value)),
            can_retry: verification.can_retry,
            recovery_hint: verification.recovery_hint.clone(),
            prompt_hash: execution_request.prompt_hash.clone(),
        };
        if execution.success {
            events.push(serde_json::json!({
                "type": "ActionCompleted",
                "execution_id": result.execution_id,
                "proposal_id": result.proposal_id,
                "proposal_hash": result.proposal_hash,
                "target_hash": execution_request.target_hash,
                "action_kind": result.action_type,
                "status": "completed",
                "backend_used": result.backend_used,
                "result_summary": "Deterministic GUI action backend reported success.",
                "prompt_hash": result.prompt_hash,
            }));
            // Backend success is not final: post-action verification decides the
            // turn outcome. ActionCompleted means backend success only.
            if verification.is_verified() {
                state.status = "completed".into();
                state.reply = format!(
                    "Step 7 executed {} through deterministic backend {} and Step 8 verified the expected result ({}).",
                    result.action_type, result.backend_used, verification.verification_strategy
                );
            } else {
                state.status = verification.status.clone();
                let detail = verification
                    .safe_error_summary
                    .clone()
                    .unwrap_or_else(|| "Post-action state was not confirmed.".into());
                state.reply = format!(
                    "Step 7 executed {} through deterministic backend {}, but Step 8 post-action verification did not pass: {}",
                    result.action_type, result.backend_used, detail
                );
            }
        } else {
            events.push(serde_json::json!({
                "type": "ActionFailed",
                "execution_id": result.execution_id,
                "proposal_id": result.proposal_id,
                "proposal_hash": result.proposal_hash,
                "target_hash": execution_request.target_hash,
                "action_kind": result.action_type,
                "status": "failed",
                "backend_used": result.backend_used,
                "safe_error_summary": result.safe_error_summary,
                "prompt_hash": result.prompt_hash,
            }));
            state.status = "blocked".into();
            state.reply = result
                .safe_error_summary
                .clone()
                .unwrap_or_else(|| "Deterministic GUI action failed.".into());
        }
        events.push(verification.event_payload());
        state.action = Some(result.summary_json());
        state.execution_result = Some(result.summary_json());
        state.verification_result = Some(verification.clone());

        // Step 9: Recovery loop runs only when verification did not confirm the
        // expected state. Verified actions never trigger recovery.
        if should_attempt_recovery(&verification.status) {
            self.run_recovery_loop(
                events,
                proposal,
                target_resolution,
                hitl_decision,
                &verification,
                &backend,
                &post_observation,
                execution.success,
                now_ms(),
                state,
            )
            .await;
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_recovery_loop(
        &self,
        events: &mut Vec<serde_json::Value>,
        proposal: &GuiActionProposal,
        target_resolution: &GuiTargetResolutionSummary,
        hitl_decision: Option<&GuiHitlDecision>,
        verification: &GuiPostActionVerificationResult,
        backend: &GuiActionBackendStatus,
        post_action_observation: &GuiObservationSnapshot,
        backend_success: bool,
        now: i64,
        state: &mut RuntimeState,
    ) {
        let action_kind = GuiActionKind::from_action_type(&proposal.action_type);
        let hitl_denied = hitl_decision
            .map(|decision| decision.decision == "denied")
            .unwrap_or(false);
        let hitl_stale = hitl_decision
            .map(|decision| decision.decision != "denied" && !decision.can_authorize_step7)
            .unwrap_or(false);

        // Bounded re-resolve count for control actions: how many post-action
        // controls still match the original target label/role.
        let reresolve_candidate_count = match action_kind {
            GuiActionKind::OpenApp | GuiActionKind::SwitchWindow => 0,
            _ => proposal
                .target_label
                .as_deref()
                .map(|label| {
                    post_action_observation
                        .all_controls()
                        .iter()
                        .filter(|control| {
                            control.name.eq_ignore_ascii_case(label)
                                && proposal
                                    .target_role
                                    .as_deref()
                                    .map(|role| control.role.eq_ignore_ascii_case(role))
                                    .unwrap_or(true)
                        })
                        .count()
                })
                .unwrap_or(0),
        };

        let recovery_id = format!(
            "recovery-{}",
            stable_hash(&format!(
                "{}|{}|{}",
                verification.execution_id, verification.verification_id, now
            ))
        );
        let signals = GuiRecoverySignals {
            backend_success,
            verification_status: verification.status.clone(),
            verification_strategy: verification.verification_strategy.clone(),
            matched_expected_state: verification.matched_expected_state,
            target_still_present: verification.target_still_present,
            target_identity_matches: verification.target_identity_matches,
            modal_present: !post_action_observation.dialogs.is_empty(),
            active_window_known: post_action_observation.active_window_probe_ok
                && post_action_observation.active_window.confidence > 0.0,
            reresolve_candidate_count,
            context_stale: false,
        };
        let input = GuiRecoveryInput {
            recovery_id: recovery_id.clone(),
            execution_id: verification.execution_id.clone(),
            verification_id: verification.verification_id.clone(),
            proposal_id: proposal.proposal_id.clone(),
            proposal_hash: proposal.proposal_hash.clone(),
            target_hash: proposal.target_hash.clone(),
            action_type: proposal.action_type.clone(),
            risk_level: proposal.risk_level.clone(),
            requires_user_approval: proposal.requires_user_approval,
            hitl_denied,
            hitl_stale,
            retry_count: 0,
            prompt_hash: proposal.prompt_hash.clone(),
            signals,
        };

        let assessment = assess_recovery(&input);
        events.push(assessment.event_payload());
        state.recovery_assessment = Some(assessment.summary_json());

        let action_kind_recovery = match assessment.recovery_action_kind.as_str() {
            "ReObserve" => GuiRecoveryActionKind::ReObserve,
            "RefocusSameTarget" => GuiRecoveryActionKind::RefocusSameTarget,
            "SwitchBackToWindow" => GuiRecoveryActionKind::SwitchBackToWindow,
            "RetryIdempotentAction" => GuiRecoveryActionKind::RetryIdempotentAction,
            "ReResolveTarget" => GuiRecoveryActionKind::ReResolveTarget,
            "AskClarification" => GuiRecoveryActionKind::AskClarification,
            _ => GuiRecoveryActionKind::Stop,
        };

        if !assessment.can_execute_recovery || !action_kind_recovery.is_executable_recovery() {
            // No safe recovery action: emit RecoveryBlocked without starting one.
            // The turn status keeps the verification verdict (verification_failed
            // / inconclusive / blocked) unless recovery needs the user.
            events.push(recovery_blocked_event(&assessment));
            match assessment.status.as_str() {
                "needs_clarification" => state.status = "needs_clarification".into(),
                "needs_approval" => state.status = "needs_approval".into(),
                _ => {}
            }
            state.reply = assessment.safe_explanation.clone();
            if let Some(blocker_reason) = assessment.blockers.first() {
                state.blocker = Some(GuiBlocker::new("recovery", blocker_reason.clone()));
            }
            return;
        }

        // Safe, bounded recovery action.
        let started_at_ms = now_ms();
        let mut result = GuiRecoveryResult {
            recovery_id: recovery_id.clone(),
            execution_id: verification.execution_id.clone(),
            status: "recovered".into(),
            recovery_action_kind: action_kind_recovery.as_str().into(),
            started_at_ms,
            completed_at_ms: started_at_ms,
            backend_used: backend.selected_backend.clone(),
            post_recovery_observation_id: None,
            post_recovery_context_id: None,
            verification_result: "recovered".into(),
            safe_error_summary: None,
            next_recommended_state: "retry_original_action".into(),
            can_retry_original_action: true,
            can_continue_workflow: false,
            prompt_hash: proposal.prompt_hash.clone(),
        };
        events.push(result.started_event_payload());

        if matches!(action_kind_recovery, GuiRecoveryActionKind::ReObserve) {
            // Re-observe only: always safe, never touches the input backend.
            let post_recovery = self.observe_with_events(events).await;
            result.completed_at_ms = now_ms();
            result.backend_used = "observation".into();
            result.post_recovery_observation_id = Some(post_recovery.observation_id.clone());
            result.post_recovery_context_id = Some(post_recovery.context_id.clone());
            result.verification_result = "reobserved".into();
            result.status = "recovered".into();
            result.next_recommended_state = "replan".into();
            events.push(result.completed_event_payload());
            state.recovery_result = Some(result.summary_json());
            state.status = "recovered".into();
            state.reply = assessment.safe_explanation.clone();
            return;
        }

        // Input-backend recovery: re-run one idempotent action on the same target.
        let recovery_action_request = self.build_recovery_action_request(
            &action_kind_recovery,
            &action_kind,
            proposal,
            target_resolution,
        );
        let recovery_execution = self.executor.execute(recovery_action_request).await;
        let post_recovery = self.observe_with_events(events).await;
        result.completed_at_ms = now_ms();
        result.backend_used = recovery_execution.tool.clone();
        result.post_recovery_observation_id = Some(post_recovery.observation_id.clone());
        result.post_recovery_context_id = Some(post_recovery.context_id.clone());

        let recovery_strategy = select_verification_strategy(&action_kind, false);
        let recovery_verification_request = GuiPostActionVerificationRequest {
            verification_id: format!("recovery-verify-{}", stable_hash(&recovery_id)),
            execution_id: verification.execution_id.clone(),
            proposal_id: proposal.proposal_id.clone(),
            proposal_hash: proposal.proposal_hash.clone(),
            action_type: proposal.action_type.clone(),
            target_hash: proposal.target_hash.clone(),
            stable_target_identity_hash: None,
            expected_postcondition: proposal.expected_postcondition.clone(),
            verification_strategy: recovery_strategy.as_str().into(),
            pre_action_context_id: post_action_observation.context_id.clone(),
            post_action_observation_id: post_recovery.observation_id.clone(),
            post_action_context_id: post_recovery.context_id.clone(),
            started_at_ms,
            is_secret_payload: false,
            prompt_hash: proposal.prompt_hash.clone(),
            target_label: proposal.target_label.clone(),
            target_role: proposal.target_role.clone(),
            target_control_id: proposal.target_control_id.clone(),
            expected_app_hint: target_resolution
                .resolved_target
                .as_ref()
                .and_then(|target| target.app_hint.clone()),
            expected_window_hint: target_resolution
                .resolved_target
                .as_ref()
                .and_then(|target| target.window_hint.clone()),
        };
        let recovery_verification = verify_post_action_detailed(
            &recovery_verification_request,
            post_action_observation,
            &post_recovery,
            recovery_execution.success,
            None,
            now_ms(),
        );
        result.verification_result = recovery_verification.status.clone();

        if recovery_execution.success && recovery_verification.is_verified() {
            result.status = "recovered".into();
            result.next_recommended_state = "retry_original_action".into();
            result.can_retry_original_action = true;
            events.push(result.completed_event_payload());
            state.recovery_result = Some(result.summary_json());
            state.status = "recovered".into();
            state.reply = format!(
                "KRIA recovered safely via {} and restored the expected state.",
                result.recovery_action_kind
            );
        } else {
            result.status = "blocked".into();
            result.next_recommended_state = "stop".into();
            result.can_retry_original_action = false;
            result.safe_error_summary = Some(
                "Bounded recovery did not restore the expected state; stopping safely.".into(),
            );
            events.push(result.completed_event_payload());
            state.recovery_result = Some(result.summary_json());
            state.status = "blocked".into();
            state.reply =
                "KRIA attempted one safe recovery but could not confirm the expected state, so it stopped."
                    .into();
            state.blocker = Some(GuiBlocker::new(
                "recovery",
                "bounded recovery did not restore the expected state",
            ));
        }
    }

    fn build_recovery_action_request(
        &self,
        recovery_kind: &GuiRecoveryActionKind,
        original_kind: &GuiActionKind,
        proposal: &GuiActionProposal,
        target_resolution: &GuiTargetResolutionSummary,
    ) -> GuiActionRequest {
        let kind = match recovery_kind {
            GuiRecoveryActionKind::RefocusSameTarget => GuiActionKind::FocusField,
            GuiRecoveryActionKind::SwitchBackToWindow => GuiActionKind::SwitchWindow,
            _ => original_kind.clone(),
        };
        let target_name = match recovery_kind {
            GuiRecoveryActionKind::SwitchBackToWindow => target_resolution
                .resolved_target
                .as_ref()
                .and_then(|target| target.window_hint.clone())
                .or_else(|| {
                    target_resolution
                        .resolved_target
                        .as_ref()
                        .and_then(|target| target.app_hint.clone())
                })
                .or_else(|| proposal.target_label.clone())
                .unwrap_or_else(|| proposal.action_type.clone()),
            _ => proposal
                .target_label
                .clone()
                .or_else(|| proposal.target_control_id.clone())
                .unwrap_or_else(|| proposal.action_type.clone()),
        };
        let role = proposal
            .target_role
            .clone()
            .unwrap_or_else(|| role_for_action(&kind).into());
        GuiActionRequest {
            kind: kind.clone(),
            role,
            target_name,
            value: None,
            execution_hint: execution_hint_for_action(&kind).into(),
        }
    }

    fn emit_approval_required(
        &self,
        events: &mut Vec<serde_json::Value>,
        intent: &GuiCognitionIntent,
        state: &mut RuntimeState,
        reason: &str,
    ) {
        state.status = "needs_approval".into();
        let reasons = if intent.risk_reasons.is_empty() {
            vec!["user requested approval before action".to_string()]
        } else {
            intent.risk_reasons.clone()
        };
        let safety = safety_for_intent(intent);
        events.push(serde_json::json!({
            "type": "SafetyGateCompleted",
            "status": safety.status.as_event_status(),
            "risk_level": safety.risk_level,
            "reasons": safety.reasons,
        }));
        events.push(serde_json::json!({
            "type": "HitlRequired",
            "risk_level": intent.risk_level,
            "reason": reason,
        }));
        state.blocker = Some(GuiBlocker::new("approval_required", reason).with_options(reasons));
    }

    #[allow(dead_code)]
    async fn handle_focus_intent(
        &self,
        events: &mut Vec<serde_json::Value>,
        context: &GuiContext,
        state: &mut RuntimeState,
    ) {
        events.push(serde_json::json!({
            "type": "TargetResolutionStarted",
            "action_kind": GuiActionKind::FocusField.as_str(),
            "role": "text",
        }));
        match resolve_unique_text_field(context) {
            TargetResolution::Resolved(target) => {
                let request = GuiActionRequest {
                    kind: GuiActionKind::FocusField,
                    role: "text".into(),
                    target_name: target.name.clone(),
                    value: None,
                    execution_hint: "click_ui_element".into(),
                };
                state.target = Some(serde_json::json!({
                    "role": target.role,
                    "name": target.name,
                    "confidence": target.confidence,
                    "evidence": target.evidence,
                }));
                self.execute_and_verify(events, request, state, 0.72, |success, target, error| {
                    if success {
                        format!("Focused the visible text field '{}' and re-observed the GUI. Verification: focused action completed with post-action observation.", target)
                    } else {
                        format!("I found the text field '{}', but focus execution failed: {}. I stopped safely.", target, error.unwrap_or_else(|| "unknown error".into()))
                    }
                }).await;
            }
            TargetResolution::Missing {
                reason,
                candidate_count,
            }
            | TargetResolution::Ambiguous {
                reason,
                candidate_count,
            } => {
                self.emit_target_block(events, state, &reason, candidate_count, None);
                state.reply = format!("{reason} I did not focus or type anything.");
            }
        }
    }

    #[allow(dead_code)]
    fn handle_type_validation_block(
        &self,
        events: &mut Vec<serde_json::Value>,
        intent: &GuiCognitionIntent,
        state: &mut RuntimeState,
        reason: &str,
    ) {
        if intent.requires_approval || reason.contains("terminal") {
            let risk_reason = if intent.requires_approval {
                "typing request appears sensitive or risky"
            } else {
                reason
            };
            state.status = "needs_approval".into();
            events.push(serde_json::json!({
                "type": "SafetyGateCompleted",
                "status": "RequiresApproval",
                "risk_level": "high",
                "reasons": [risk_reason],
            }));
            state.blocker = Some(GuiBlocker::new("safety", risk_reason));
            state.reply = format!("{risk_reason}. I did not type anything.");
        } else {
            state.status = "needs_clarification".into();
            state.blocker = Some(GuiBlocker::new("missing_text", reason));
            events.push(serde_json::json!({
                "type": "PlanBlocked",
                "reason": "missing_text",
                "clarification_question": "What exact text should I type?",
            }));
            state.reply =
                "Please provide the exact text to type in quotes. I did not type anything.".into();
        }
    }

    #[allow(dead_code)]
    async fn handle_type_intent(
        &self,
        events: &mut Vec<serde_json::Value>,
        context: &GuiContext,
        intent: &GuiCognitionIntent,
        state: &mut RuntimeState,
    ) {
        events.push(serde_json::json!({
            "type": "TargetResolutionStarted",
            "action_kind": GuiActionKind::FillField.as_str(),
            "role": "text",
        }));

        match resolve_type_text_target(context) {
            TargetResolution::Resolved(target) => {
                let execution_hint = if target.confidence >= 0.8 {
                    "fill_form_field"
                } else {
                    "atspi_type_into_focused"
                };
                let request = GuiActionRequest {
                    kind: GuiActionKind::FillField,
                    role: "text".into(),
                    target_name: target.name.clone(),
                    value: intent.typed_text.clone(),
                    execution_hint: execution_hint.into(),
                };
                state.target = Some(serde_json::json!({
                    "role": target.role,
                    "name": target.name,
                    "confidence": target.confidence,
                    "evidence": target.evidence,
                }));
                self.execute_and_verify(events, request, state, 0.72, |success, _target, error| {
                    if success {
                        "Typed the requested text into the resolved visible text field and re-observed the GUI. Verification completed with post-action observation.".into()
                    } else {
                        format!("Typing failed during deterministic AT-SPI execution: {}. I stopped safely.", error.unwrap_or_else(|| "unknown error".into()))
                    }
                }).await;
            }
            TargetResolution::Missing {
                reason,
                candidate_count,
            }
            | TargetResolution::Ambiguous {
                reason,
                candidate_count,
            } => {
                self.emit_target_block(events, state, &reason, candidate_count, None);
                state.reply = format!("{reason} I did not type anything.");
            }
        }
    }

    #[allow(dead_code)]
    async fn handle_click_intent(
        &self,
        events: &mut Vec<serde_json::Value>,
        context: &GuiContext,
        intent: &GuiCognitionIntent,
        state: &mut RuntimeState,
    ) {
        let control_name = intent.control_name.clone().unwrap_or_default();
        events.push(serde_json::json!({
            "type": "TargetResolutionStarted",
            "action_kind": GuiActionKind::ClickControl.as_str(),
            "role": "push button",
            "query": control_name,
        }));

        if intent.requires_approval {
            self.emit_approval_required(
                events,
                intent,
                state,
                "Click action is risky and requires explicit approval.",
            );
            state.reply = "This click may submit/send/delete/pay or otherwise affect external state. I paused and did not click anything.".into();
            return;
        }

        match resolve_button(context, &control_name) {
            TargetResolution::Resolved(target) => {
                let request = GuiActionRequest {
                    kind: GuiActionKind::ClickControl,
                    role: "push button".into(),
                    target_name: target.name.clone(),
                    value: None,
                    execution_hint: "click_ui_element".into(),
                };
                state.target = Some(serde_json::json!({
                    "role": target.role,
                    "name": target.name,
                    "confidence": target.confidence,
                    "evidence": target.evidence,
                }));
                self.execute_and_verify(events, request, state, 0.68, |success, target, error| {
                    if success {
                        format!("Clicked the resolved safe button '{}' and re-observed the GUI. Verification completed with post-action observation.", target)
                    } else {
                        format!("The target '{}' was resolved, but click execution failed: {}. I stopped safely.", target, error.unwrap_or_else(|| "unknown error".into()))
                    }
                }).await;
            }
            TargetResolution::Missing {
                reason,
                candidate_count,
            }
            | TargetResolution::Ambiguous {
                reason,
                candidate_count,
            } => {
                self.emit_target_block(
                    events,
                    state,
                    &reason,
                    candidate_count,
                    Some(control_name.clone()),
                );
                state.reply = format!(
                    "{reason} Target query: '{}'. I did not click anything.",
                    control_name
                );
            }
        }
    }

    #[allow(dead_code)]
    fn handle_missing_click_target(
        &self,
        events: &mut Vec<serde_json::Value>,
        state: &mut RuntimeState,
    ) {
        state.status = "needs_clarification".into();
        state.blocker = Some(GuiBlocker::new(
            "missing_target",
            "No button/control name was provided.",
        ));
        events.push(serde_json::json!({
            "type": "PlanBlocked",
            "reason": "missing_target",
            "clarification_question": "Which button or control should I click?",
        }));
        state.reply =
            "I need the button/control name before clicking. I did not click anything.".into();
    }

    #[allow(dead_code)]
    fn emit_target_block(
        &self,
        events: &mut Vec<serde_json::Value>,
        state: &mut RuntimeState,
        reason: &str,
        candidate_count: usize,
        target_name: Option<String>,
    ) {
        state.status = "needs_clarification".into();
        events.push(serde_json::json!({
            "type": "TargetResolutionBlocked",
            "reason": reason,
            "candidate_count": candidate_count,
        }));
        let mut blocker =
            GuiBlocker::new("target_resolution", reason).with_candidate_count(candidate_count);
        if let Some(target_name) = target_name {
            blocker = blocker.with_target_name(target_name);
        }
        state.blocker = Some(blocker);
    }

    #[allow(dead_code)]
    async fn execute_and_verify<F>(
        &self,
        events: &mut Vec<serde_json::Value>,
        request: GuiActionRequest,
        state: &mut RuntimeState,
        success_confidence: f64,
        reply_builder: F,
    ) where
        F: FnOnce(bool, String, Option<String>) -> String,
    {
        let safety = GuiSafetyStatus::Allowed;
        let target_type = match &request.kind {
            GuiActionKind::FocusField | GuiActionKind::FillField | GuiActionKind::TypeText => {
                "text_field"
            }
            GuiActionKind::ClickControl => "button",
            GuiActionKind::OpenApp => "application",
            GuiActionKind::SwitchWindow => "window",
            GuiActionKind::PressKey | GuiActionKind::Hotkey => "focused_context",
            GuiActionKind::Scroll => "scrollable",
            GuiActionKind::Copy | GuiActionKind::Paste => "focused_context",
        };
        events.push(serde_json::json!({
            "type": "TargetResolved",
            "target_type": target_type,
            "label": request.target_name,
            "confidence": state.target.as_ref().and_then(|target| target.get("confidence")).cloned().unwrap_or(serde_json::json!(0.86)),
        }));
        events.push(serde_json::json!({
            "type": "SafetyGateCompleted",
            "status": safety.as_event_status(),
            "risk_level": "low",
        }));

        if let Some(backend) = &state.action_backend {
            if !backend.supports_action(&request.kind) {
                let reason = backend.primary_blocker(&request.kind);
                let action_kind = request.kind.as_str();
                state.status = "blocked".into();
                state.execution_blocker = Some(serde_json::json!({
                    "kind": "action_backend",
                    "reason": reason,
                    "action_kind": action_kind,
                    "selected_backend": backend.selected_backend.clone(),
                    "session_type": backend.session_type.clone(),
                    "blockers": backend.blockers.clone(),
                    "global_halt_engaged": backend.global_halt_engaged,
                    "halt_kind": backend.halt_kind.clone(),
                    "halt_reason": backend.halt_reason.clone(),
                    "release_conditions": backend.release_conditions.clone(),
                    "can_observe": backend.can_observe,
                    "can_plan": backend.can_plan,
                }));
                state.blocker = Some(
                    GuiBlocker::new("action_backend", reason.clone()).with_options(
                        if backend.blockers.is_empty() {
                            vec![format!("selected backend: {}", backend.selected_backend)]
                        } else {
                            backend.blockers.clone()
                        },
                    ),
                );
                events.push(serde_json::json!({
                    "type": "ExecutionBlocked",
                    "reason": reason,
                    "action_kind": action_kind,
                    "selected_backend": backend.selected_backend.clone(),
                    "session_type": backend.session_type.clone(),
                    "global_halt_engaged": backend.global_halt_engaged,
                    "halt_kind": backend.halt_kind.clone(),
                    "halt_reason": backend.halt_reason.clone(),
                    "release_conditions": backend.release_conditions.clone(),
                    "blockers": backend.blockers.clone(),
                }));
                events.push(serde_json::json!({
                    "type": "VerificationStarted",
                    "verification": "execution_blocker",
                }));
                events.push(serde_json::json!({
                    "type": "VerificationCompleted",
                    "status": "blocked",
                    "confidence": 1.0,
                    "summary": "Action was not executed because the GUI action backend is blocked or unavailable.",
                }));
                events.push(serde_json::json!({
                    "type": "RecoveryEvaluationStarted",
                    "reason": "action_backend_blocked",
                    "idempotency": "safe_retry_after_capability_change",
                }));
                events.push(serde_json::json!({
                    "type": "RecoveryProposed",
                    "reason": reason,
                    "options": [
                        "Resolve the GUI action backend blocker, then retry.",
                        "Re-observe the screen without executing an action.",
                        "Ask the user for a different safe target."
                    ],
                }));
                state.verification = Some(GuiVerificationReport {
                    status: "blocked".into(),
                    confidence: 1.0,
                    after_observation_id: String::new(),
                });
                state.reply = reply_builder(false, request.target_name, Some(reason));
                return;
            }
        }

        events.push(serde_json::json!({
            "type": "ActionStarted",
            "action_kind": request.kind.as_str(),
            "target": request.target_name,
        }));

        let target_name = request.target_name.clone();
        let action_kind = request.kind.as_str();
        let execution = self.executor.execute(request).await;
        events.push(serde_json::json!({
            "type": "ActionCompleted",
            "action_kind": action_kind,
            "status": if execution.success { "completed" } else { "failed" },
        }));
        let post_observation = self.observe_with_events(events).await;
        events.push(serde_json::json!({
            "type": "VerificationStarted",
            "verification": "post_action_observation",
        }));
        let verification = verify_post_action(&execution, &post_observation, success_confidence);
        events.push(serde_json::json!({
            "type": "VerificationCompleted",
            "status": verification.status,
            "confidence": verification.confidence,
        }));

        state.action = Some(action_summary(&execution));
        state.verification = Some(verification);
        if !execution.success {
            state.status = "blocked".into();
            let recovery_reason = execution
                .error
                .clone()
                .unwrap_or_else(|| "GUI action execution failed".into());
            events.push(serde_json::json!({
                "type": "RecoveryEvaluationStarted",
                "reason": recovery_reason,
                "idempotency": "safe_retry_only_after_reobserve",
            }));
            events.push(serde_json::json!({
                "type": "RecoveryProposed",
                "reason": recovery_reason,
                "options": [
                    "Re-observe the screen and resolve the target again.",
                    "Retry once only if the target remains unique and the action is safe.",
                    "Ask the user for clarification if the target changed."
                ],
            }));
        }
        state.reply = reply_builder(execution.success, target_name, execution.error);
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_workflow(
        &self,
        events: &mut Vec<serde_json::Value>,
        request: &GuiTurnRequest,
        context: &GuiContext,
        goal_contract: &GuiGoalContract,
        plan: &self::llm_planner::GuiLlmPlan,
        readiness_validation: &GuiPlanValidationReport,
        plan_id: &str,
        state: &mut RuntimeState,
    ) {
        let steps = typed_plan_steps(plan);
        let mut run = GuiWorkflowRun::new(
            &request.session_id,
            &request.workflow_id,
            &request.turn_id,
            &goal_contract.contract_id,
            plan_id,
            &context.context_id,
            &steps,
            &plan.risk_level,
            plan.requires_user_approval,
            request.execution_mode.as_str(),
            &goal_contract.prompt_hash,
        );
        let mut current_context = context.clone();
        let allows_execution = request.execution_mode.allows_execution();
        let mut resume_start_index = 0usize;
        let mut resumed_completed_indices: Vec<usize> = Vec::new();

        // Step 11: resume from a checkpoint. Re-observe (already done before this
        // call) and revalidate before allowing any continuation. Fail closed.
        if let Some(cp) = request.resume_checkpoint.clone() {
            let now = now_ms();
            let resume_request = GuiWorkflowResumeRequest {
                resume_id: format!("resume-{}", stable_hash(&format!("{}|{}", cp.checkpoint_id, now))),
                checkpoint_id: cp.checkpoint_id.clone(),
                workflow_run_id: cp.workflow_run_id.clone(),
                session_id: cp.session_id.clone(),
                requested_at_ms: now,
                current_observation_id: current_context.observation_id.clone(),
                current_context_id: current_context.context_id.clone(),
                current_screen_hash_prefix: current_context
                    .observation
                    .screen_hash
                    .as_ref()
                    .map(|hash| hash.chars().take(16).collect()),
                reason: request
                    .resume_reason
                    .clone()
                    .unwrap_or_else(|| "user_resume".into()),
                prompt_hash: cp.prompt_hash.clone(),
            };
            events.push(serde_json::json!({
                "type": "WorkflowResumeRequested",
                "resume_id": resume_request.resume_id,
                "checkpoint_id": cp.checkpoint_id,
                "workflow_run_id": cp.workflow_run_id,
                "reason": resume_request.reason,
                "prompt_hash": cp.prompt_hash,
            }));
            events.push(serde_json::json!({
                "type": "WorkflowCheckpointLoaded",
                "checkpoint_id": cp.checkpoint_id,
                "checkpoint_hash_prefix": cp.checkpoint_hash.chars().take(12).collect::<String>(),
                "workflow_run_id": cp.workflow_run_id,
                "current_step_index": cp.current_step_index,
                "completed_step_count": cp.completed_step_receipts.len(),
                "can_execute": false,
                "prompt_hash": cp.prompt_hash,
            }));
            let screen_prefix: Option<String> = current_context
                .observation
                .screen_hash
                .as_ref()
                .map(|hash| hash.chars().take(16).collect());
            let screen_changed = match (
                cp.last_screen_hash_prefix.as_deref(),
                screen_prefix.as_deref(),
            ) {
                (Some(before), Some(after)) => before != after,
                _ => false,
            };
            let signals = GuiResumeObservationSignals {
                current_screen_hash_prefix: screen_prefix,
                current_active_window_hash: None,
                // Fail closed: a changed screen means the bound target identity
                // can no longer be trusted without re-resolution.
                pending_target_still_present: !screen_changed,
                pending_target_identity_matches: !screen_changed,
            };
            let recomputed = checkpoint_hash(&cp);
            let resume_result = validate_resume(&cp, &resume_request, &signals, &recomputed, None, now);

            let proceed = matches!(
                resume_result.status.as_str(),
                "resumed" | "needs_approval" | "needs_reobserve"
            );
            if proceed {
                events.push(resume_result.validated_event_payload());
                // Seed completed receipts so completed steps are not replayed.
                run.completed_step_receipts = cp.completed_step_receipts.clone();
                for receipt in &cp.completed_step_receipts {
                    if let Some(slot) = run.step_states.get_mut(receipt.step_index) {
                        slot.status = "completed".into();
                        slot.can_continue = true;
                    }
                    resumed_completed_indices.push(receipt.step_index);
                }
                resume_start_index = cp.current_step_index;
            } else {
                events.push(resume_result.rejected_event_payload());
                run.status = "blocked".into();
                run.blocked_reason = Some(resume_result.safe_explanation.clone());
                run.completed_step_receipts = cp.completed_step_receipts.clone();
                events.push(run.run_terminal_event());
                state.status = "blocked".into();
                state.reply = resume_result.safe_explanation.clone();
                state.workflow_run = Some(run.summary_json());
                if let Some(reason) = resume_result
                    .blockers
                    .first()
                    .or_else(|| resume_result.invalidated_approvals.first())
                    .or_else(|| resume_result.duplicate_action_guards.first())
                {
                    state.blocker = Some(GuiBlocker::new("resume", reason.clone()));
                }
                return;
            }
        }

        events.push(run.run_started_event());

        for index in 0..steps.len() {
            if index < resume_start_index || resumed_completed_indices.contains(&index) {
                // Already completed before the checkpoint; never replayed.
                continue;
            }
            let step = steps[index].clone();
            run.current_step_index = index;
            let mut step_state = run.step_states[index].clone();
            step_state.status = "started".into();
            step_state.started_at_ms = now_ms();
            events.push(step_started_event(&run, &step_state));

            match workflow_step_kind(&step.step_type) {
                GuiWorkflowStepKind::Observe
                | GuiWorkflowStepKind::Summarize
                | GuiWorkflowStepKind::WaitOrVerify => {
                    // Re-observe / verify-by-observation only. No executor call.
                    let observation = self.observe_with_events(events).await;
                    current_context = GuiContextBuilder::new()
                        .build(GuiContextBuildRequest::new(observation));
                    run.current_context_id = current_context.context_id.clone();
                    let observable = current_context.observation.has_useful_signal();
                    step_state.completed_at_ms = now_ms();
                    if observable {
                        step_state.status = "completed".into();
                        step_state.can_continue = true;
                        let receipt = self.workflow_receipt(
                            &run,
                            &step_state,
                            "completed",
                            None,
                            None,
                            "Re-observed/verified GUI state without executing an action.",
                            &goal_contract.prompt_hash,
                        );
                        run.completed_step_receipts.push(receipt.clone());
                        run.step_states[index] = step_state.clone();
                        events.push(step_completed_event(&run, &step_state, &receipt));
                        self.save_workflow_checkpoint(
                            events,
                            &mut run,
                            steps.get(index + 1).map(|next| next.step_id.clone()),
                            index + 1,
                            &current_context,
                            state,
                        );
                    } else {
                        step_state.status = "blocked".into();
                        step_state.blockers.push("no useful perception signal".into());
                        run.step_states[index] = step_state.clone();
                        run.status = "blocked".into();
                        run.blocked_reason = Some("no useful perception signal".into());
                        events.push(step_blocked_event(&run, &step_state));
                        break;
                    }
                }
                GuiWorkflowStepKind::AskClarification => {
                    step_state.status = "blocked".into();
                    step_state
                        .blockers
                        .push("clarification required before continuing".into());
                    run.step_states[index] = step_state.clone();
                    run.status = "paused".into();
                    run.blocked_reason = Some("clarification required".into());
                    events.push(step_blocked_event(&run, &step_state));
                    break;
                }
                GuiWorkflowStepKind::RequireApproval => {
                    step_state.status = "awaiting_approval".into();
                    step_state
                        .blockers
                        .push("explicit approval required before continuing".into());
                    run.step_states[index] = step_state.clone();
                    run.status = "paused".into();
                    run.blocked_reason = Some("approval required".into());
                    events.push(step_blocked_event(&run, &step_state));
                    break;
                }
                GuiWorkflowStepKind::Executable => {
                    // Re-observe before resolving a target if the previous step
                    // changed GUI state (or simply for every step after the first).
                    if index > 0 {
                        let observation = self.observe_with_events(events).await;
                        current_context = GuiContextBuilder::new()
                            .build(GuiContextBuildRequest::new(observation));
                    }
                    run.current_context_id = current_context.context_id.clone();

                    let step_plan_id = format!("{plan_id}-s{index}");
                    let sub_plan = single_step_plan(plan, &step);
                    let summary = self.resolve_step_target_for_workflow(
                        events,
                        &step,
                        &sub_plan,
                        readiness_validation,
                        &current_context,
                        &step_plan_id,
                        state,
                    );
                    step_state.target_resolution_id = Some(summary.resolution_id.clone());

                    if workflow_step_requires_target(&step.step_type)
                        && summary.status != "resolved"
                    {
                        let ambiguous = summary.status == "ambiguous"
                            || summary.status == "needs_clarification"
                            || summary.ambiguity_count > 0;
                        step_state.status = "blocked".into();
                        let reason = summary
                            .ambiguity_reasons
                            .first()
                            .cloned()
                            .or_else(|| summary.blockers.first().cloned())
                            .unwrap_or_else(|| "target not safely resolved".into());
                        step_state.blockers.push(reason.clone());
                        run.step_states[index] = step_state.clone();
                        run.status = if ambiguous { "paused" } else { "blocked" }.into();
                        run.blocked_reason = Some(reason);
                        events.push(step_blocked_event(&run, &step_state));
                        break;
                    }

                    // safety_only: create state + gate but never start an action.
                    if !allows_execution {
                        self.reset_step_execution_state(state);
                        self.handle_safety_gate(
                            events,
                            request,
                            &current_context,
                            goal_contract,
                            &sub_plan,
                            readiness_validation,
                            &step_plan_id,
                            state,
                        )
                        .await;
                        step_state.status = "blocked".into();
                        step_state
                            .warnings
                            .push("execution_mode is safety_only; no action started".into());
                        run.step_states[index] = step_state.clone();
                        run.status = "paused".into();
                        run.blocked_reason =
                            Some("execution_mode is safety_only".into());
                        events.push(step_blocked_event(&run, &step_state));
                        break;
                    }

                    self.reset_step_execution_state(state);
                    self.handle_safety_gate(
                        events,
                        request,
                        &current_context,
                        goal_contract,
                        &sub_plan,
                        readiness_validation,
                        &step_plan_id,
                        state,
                    )
                    .await;

                    // Pull safe IDs from the per-step state for the step record.
                    step_state.proposal_id = json_str(&state.safety_gate, "proposal_id");
                    step_state.proposal_hash = json_str(&state.safety_gate, "proposal_hash");
                    step_state.hitl_decision_id = json_str(&state.hitl_decision, "decision_id");
                    step_state.execution_id = json_str(&state.execution_result, "execution_id");
                    step_state.verification_id = state
                        .verification_result
                        .as_ref()
                        .map(|verification| verification.verification_id.clone());
                    step_state.recovery_id = json_str(&state.recovery_assessment, "recovery_id");
                    step_state.completed_at_ms = now_ms();

                    let verification_status = state
                        .verification_result
                        .as_ref()
                        .map(|verification| verification.status.clone());
                    let recovery_status = json_str(&state.recovery_result, "status");

                    let step_succeeded = matches!(state.status.as_str(), "completed" | "recovered");
                    let needs_approval = matches!(
                        state.status.as_str(),
                        "needs_approval" | "approved_for_step7"
                    ) && state.execution_result.is_none();

                    if step_succeeded {
                        step_state.status = "completed".into();
                        step_state.can_continue = true;
                        let receipt = self.workflow_receipt(
                            &run,
                            &step_state,
                            "completed",
                            verification_status.clone(),
                            recovery_status.clone(),
                            "Step executed and verified (or safely recovered) before continuing.",
                            &goal_contract.prompt_hash,
                        );
                        run.completed_step_receipts.push(receipt.clone());
                        if recovery_status.is_some() {
                            run.recovery_summary = recovery_status.clone();
                        }
                        run.step_states[index] = step_state.clone();
                        events.push(step_completed_event(&run, &step_state, &receipt));
                        self.save_workflow_checkpoint(
                            events,
                            &mut run,
                            steps.get(index + 1).map(|next| next.step_id.clone()),
                            index + 1,
                            &current_context,
                            state,
                        );
                    } else if needs_approval {
                        step_state.status = "awaiting_approval".into();
                        run.step_states[index] = step_state.clone();
                        run.status = "paused".into();
                        run.blocked_reason = Some("step requires HITL approval".into());
                        events.push(step_blocked_event(&run, &step_state));
                        break;
                    } else {
                        step_state.status = "blocked".into();
                        let reason = state
                            .blocker
                            .as_ref()
                            .map(|blocker| blocker.reason.clone())
                            .unwrap_or_else(|| "step did not complete safely".into());
                        step_state.blockers.push(reason.clone());
                        run.step_states[index] = step_state.clone();
                        run.status = "blocked".into();
                        run.blocked_reason = Some(reason);
                        if recovery_status.is_some() {
                            run.recovery_summary = recovery_status;
                        }
                        events.push(step_blocked_event(&run, &step_state));
                        break;
                    }
                }
            }
        }

        if run.status == "running" {
            run.status = "completed".into();
            run.current_step_index = run.step_count.saturating_sub(1);
        }
        // Step 11: save a checkpoint reflecting the final completed/paused/blocked
        // state (covers pause-for-HITL and block-before-next-step cases).
        let final_index = run.current_step_index;
        let final_step_id = steps.get(final_index).map(|step| step.step_id.clone());
        self.save_workflow_checkpoint(
            events,
            &mut run,
            final_step_id,
            final_index,
            &current_context,
            state,
        );
        events.push(run.run_terminal_event());

        // Reflect the workflow outcome in the turn status/reply.
        state.status = match run.status.as_str() {
            "completed" => "completed".into(),
            "paused" => "paused".into(),
            _ => "blocked".into(),
        };
        state.reply = match run.status.as_str() {
            "completed" => format!(
                "Workflow completed {} verified step(s) safely, one bound action at a time.",
                run.completed_step_receipts.len()
            ),
            "paused" => run
                .blocked_reason
                .clone()
                .map(|reason| format!("Workflow paused safely: {reason}"))
                .unwrap_or_else(|| "Workflow paused safely.".into()),
            _ => run
                .blocked_reason
                .clone()
                .map(|reason| format!("Workflow stopped safely: {reason}"))
                .unwrap_or_else(|| "Workflow stopped safely.".into()),
        };
        state.workflow_run = Some(run.summary_json());
    }

    #[allow(clippy::too_many_arguments)]
    fn save_workflow_checkpoint(
        &self,
        events: &mut Vec<serde_json::Value>,
        run: &mut GuiWorkflowRun,
        pending_step_id: Option<String>,
        pending_index: usize,
        context: &GuiContext,
        state: &mut RuntimeState,
    ) {
        run.current_step_index = pending_index.min(run.step_count.saturating_sub(1));
        let pending = GuiCheckpointPending {
            pending_step_id,
            pending_proposal_id: json_str(&state.safety_gate, "proposal_id"),
            pending_proposal_hash: json_str(&state.safety_gate, "proposal_hash"),
            pending_target_hash: json_str(&state.target, "target_hash"),
            pending_stable_target_identity_hash: None,
            pending_hitl_request_id: json_str(&state.safety_gate, "request_id"),
            approved_decision_id: json_str(&state.hitl_decision, "decision_id"),
            approved_decision_hash: json_str(&state.hitl_decision, "proposal_hash"),
        };
        let screen_prefix: Option<String> = context
            .observation
            .screen_hash
            .as_ref()
            .map(|hash| hash.chars().take(16).collect());
        let checkpoint = build_checkpoint(
            run,
            &pending,
            &context.observation_id,
            &context.context_id,
            screen_prefix,
            None,
            now_ms(),
            WORKFLOW_CHECKPOINT_TTL_MS,
        );
        events.push(checkpoint.saved_event_payload());
        state.workflow_checkpoint = Some(checkpoint.summary_json());
    }

    fn reset_step_execution_state(&self, state: &mut RuntimeState) {
        state.safety_gate = None;
        state.hitl_decision = None;
        state.action = None;
        state.execution_result = None;
        state.execution_blocker = None;
        state.verification_result = None;
        state.recovery_assessment = None;
        state.recovery_result = None;
        state.blocker = None;
    }

    #[allow(clippy::too_many_arguments)]
    fn workflow_receipt(
        &self,
        run: &GuiWorkflowRun,
        step_state: &workflow_runtime::GuiWorkflowStepState,
        status: &str,
        verification_status: Option<String>,
        recovery_status: Option<String>,
        safe_summary: &str,
        prompt_hash: &str,
    ) -> GuiWorkflowStepReceipt {
        let action_type = step_state.step_type.clone();
        let risk_level = run.risk_level.clone();
        let side_effect_kind =
            workflow_runtime::side_effect_kind_for(&action_type, &risk_level).to_string();
        let receipt_hash = workflow_runtime::compute_receipt_hash(
            &run.workflow_run_id,
            &step_state.step_id,
            step_state.step_index,
            step_state.proposal_hash.as_deref(),
            step_state.execution_id.as_deref(),
            verification_status.as_deref(),
        );
        GuiWorkflowStepReceipt {
            receipt_id: run.receipt_id(step_state.step_index),
            workflow_run_id: run.workflow_run_id.clone(),
            step_id: step_state.step_id.clone(),
            step_index: step_state.step_index,
            step_type: step_state.step_type.clone(),
            status: status.into(),
            proposal_id: step_state.proposal_id.clone(),
            action_type: Some(action_type),
            risk_level: Some(risk_level),
            side_effect_kind,
            target_hash: step_state.proposal_hash.clone(),
            proposal_hash: step_state.proposal_hash.clone(),
            execution_id: step_state.execution_id.clone(),
            verification_id: step_state.verification_id.clone(),
            verification_status,
            recovery_id: step_state.recovery_id.clone(),
            recovery_status,
            started_at_ms: step_state.started_at_ms,
            completed_at_ms: step_state.completed_at_ms,
            safe_summary: sanitize_event_text(safe_summary),
            receipt_hash,
            prompt_hash: prompt_hash.chars().take(96).collect(),
        }
    }

    fn resolve_step_target_for_workflow(
        &self,
        events: &mut Vec<serde_json::Value>,
        step: &self::llm_planner::GuiTypedPlanStep,
        sub_plan: &self::llm_planner::GuiLlmPlan,
        readiness_validation: &GuiPlanValidationReport,
        context: &GuiContext,
        step_plan_id: &str,
        state: &mut RuntimeState,
    ) -> GuiTargetResolutionSummary {
        events.push(serde_json::json!({
            "type": "TargetResolutionStarted",
            "plan_id": step_plan_id,
            "validation_id": readiness_validation.validation_id.as_deref(),
            "mode": "step10_workflow_step",
        }));
        let summary = if workflow_step_requires_target(&step.step_type) {
            resolve_plan_targets(sub_plan, readiness_validation, context, step_plan_id)
        } else {
            // App/window/key steps need no control target; synthesize a resolved
            // summary so the safety gate can proceed without a control.
            GuiTargetResolutionSummary {
                resolution_id: format!("resolution-{step_plan_id}"),
                plan_id: step_plan_id.to_string(),
                validation_id: readiness_validation.validation_id.clone(),
                goal_contract_id: sub_plan
                    .goal_contract_id
                    .clone()
                    .or(readiness_validation.goal_contract_id.clone()),
                context_id: context.context_id.clone(),
                observation_id: context.observation_id.clone(),
                status: "resolved".into(),
                results: Vec::new(),
                resolved_target: None,
                can_proceed_to_safety_gate: true,
                can_execute: false,
                blocker_count: 0,
                blockers: Vec::new(),
                ambiguity_count: 0,
                ambiguity_reasons: Vec::new(),
                confidence: step.confidence,
                prompt_hash: sub_plan.prompt_hash.clone(),
            }
        };
        events.push(summary.event_payload());
        state.target_resolution = Some(summary.summary_json());
        if let Some(target) = &summary.resolved_target {
            state.target = Some(serde_json::json!({
                "label": target.label,
                "role": target.role,
                "target_type": target.target_kind,
                "control_id": target.control_id,
                "target_hash": target.target_hash,
                "bounds": target.bounds.clone(),
                "confidence": summary.confidence,
                "can_execute": false,
            }));
        }
        summary
    }

    fn response_json(
        &self,
        request: &GuiTurnRequest,
        context: &GuiContext,
        goal_contract: &GuiGoalContract,
        intent: &GuiCognitionIntent,
        plan_id: &str,
        planner_selection: &GuiPlannerSelection,
        plan_validation: &GuiPlanValidationReport,
        state: &RuntimeState,
    ) -> serde_json::Value {
        let observation = &context.observation;
        let perception = perception_summary_json(observation);
        let plan = plan_summary_json(plan_id, planner_selection);
        serde_json::json!({
            "status": state.status,
            "reply": state.reply,
            "gui_cognition": {
                "mode_id": "gui_cognition",
                "workflow_id": request.workflow_id,
                "turn_id": request.turn_id,
                "observation_id": observation.observation_id,
                "context_id": context.context_id,
                "path": request.route_path,
                "llm_tool_loop": request.llm_tool_loop,
                "intent": intent.kind.as_str(),
                "risk_level": intent.risk_level,
                "requires_approval": intent.requires_approval,
                "risk_reasons": intent.risk_reasons,
                "perception": perception,
                "context": context.context_summary(),
                "goal_contract": goal_contract.response_summary(),
                "planner": planner_summary_json(planner_selection),
                "plan": plan,
                "plan_validation": plan_validation.summary_json(plan_id),
                "target_resolution": state.target_resolution.clone().unwrap_or(serde_json::Value::Null),
                "target": state.target.clone().unwrap_or(serde_json::Value::Null),
                "safety_gate": state.safety_gate.clone().unwrap_or(serde_json::Value::Null),
                "hitl_decision": state.hitl_decision.clone().unwrap_or(serde_json::Value::Null),
                "action": state.action.clone().unwrap_or(serde_json::Value::Null),
                "execution": state.execution_result.clone().unwrap_or(serde_json::Value::Null),
                "action_backend": state.action_backend.as_ref().map(action_backend_summary).unwrap_or(serde_json::Value::Null),
                "execution_blocker": state.execution_blocker.clone().unwrap_or(serde_json::Value::Null),
                "verification": state
                    .verification_result
                    .as_ref()
                    .map(GuiPostActionVerificationResult::summary_json)
                    .or_else(|| state.verification.as_ref().map(verification_summary))
                    .unwrap_or(serde_json::Value::Null),
                "recovery_assessment": state.recovery_assessment.clone().unwrap_or(serde_json::Value::Null),
                "recovery": state.recovery_result.clone().unwrap_or(serde_json::Value::Null),
                "workflow_run": state.workflow_run.clone().unwrap_or(serde_json::Value::Null),
                "workflow_checkpoint": state.workflow_checkpoint.clone().unwrap_or(serde_json::Value::Null),
                "blocker": state.blocker.as_ref().map(blocker_summary).unwrap_or(serde_json::Value::Null),
            }
        })
    }
}

const WORKFLOW_CHECKPOINT_TTL_MS: i64 = 10 * 60 * 1000;

fn single_step_plan(
    plan: &self::llm_planner::GuiLlmPlan,
    step: &self::llm_planner::GuiTypedPlanStep,
) -> self::llm_planner::GuiLlmPlan {
    let mut sub_plan = plan.clone();
    sub_plan.typed_steps = vec![step.clone()];
    sub_plan.steps = Vec::new();
    sub_plan
}

fn json_str(value: &Option<serde_json::Value>, key: &str) -> Option<String> {
    value
        .as_ref()
        .and_then(|object| object.get(key))
        .and_then(serde_json::Value::as_str)
        .map(|text| text.to_string())
}

fn sanitize_event_text(value: &str) -> String {
    value
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(180)
        .collect()
}

fn role_for_action(kind: &GuiActionKind) -> &'static str {
    match kind {
        GuiActionKind::OpenApp => "application",
        GuiActionKind::SwitchWindow => "window",
        GuiActionKind::FocusField | GuiActionKind::FillField | GuiActionKind::TypeText => "text",
        GuiActionKind::ClickControl => "push button",
        GuiActionKind::PressKey | GuiActionKind::Hotkey => "focused_context",
        GuiActionKind::Scroll => "scrollable",
        GuiActionKind::Copy | GuiActionKind::Paste => "focused_context",
    }
}

fn execution_hint_for_action(kind: &GuiActionKind) -> &'static str {
    match kind {
        GuiActionKind::OpenApp => "open_application",
        GuiActionKind::SwitchWindow => "focus_window",
        GuiActionKind::FocusField | GuiActionKind::ClickControl => "click_ui_element",
        GuiActionKind::FillField | GuiActionKind::TypeText => "fill_form_field",
        GuiActionKind::PressKey | GuiActionKind::Hotkey | GuiActionKind::Copy | GuiActionKind::Paste => {
            "press_shortcut"
        }
        GuiActionKind::Scroll => "scroll",
    }
}

struct RuntimeState {
    status: String,
    reply: String,
    target_resolution: Option<serde_json::Value>,
    target: Option<serde_json::Value>,
    safety_gate: Option<serde_json::Value>,
    hitl_decision: Option<serde_json::Value>,
    action: Option<serde_json::Value>,
    execution_result: Option<serde_json::Value>,
    action_backend: Option<GuiActionBackendStatus>,
    execution_blocker: Option<serde_json::Value>,
    verification: Option<GuiVerificationReport>,
    verification_result: Option<GuiPostActionVerificationResult>,
    recovery_assessment: Option<serde_json::Value>,
    recovery_result: Option<serde_json::Value>,
    workflow_run: Option<serde_json::Value>,
    workflow_checkpoint: Option<serde_json::Value>,
    blocker: Option<GuiBlocker>,
}

impl RuntimeState {
    fn new(reply: String) -> Self {
        Self {
            status: "ok".into(),
            reply,
            target_resolution: None,
            target: None,
            safety_gate: None,
            hitl_decision: None,
            action: None,
            execution_result: None,
            action_backend: None,
            execution_blocker: None,
            verification: None,
            verification_result: None,
            recovery_assessment: None,
            recovery_result: None,
            workflow_run: None,
            workflow_checkpoint: None,
            blocker: None,
        }
    }
}

fn gui_observation_reply(observation: &GuiObservationSnapshot) -> String {
    let text_sample = control_sample(&observation.text_fields, 4);
    let button_sample = control_sample(&observation.buttons, 6);
    format!(
        "GUI Cognition mode is active on the dedicated selected-mode path. Active window: {}. Active-window source: {} ({:.0}% confidence, {} reliability). Visible applications: {}. Visible controls: {} (text fields: {}, buttons: {}, dialogs: {}, other: {}, disabled/hidden: {}). Screenshot: {}. OCR: {} ({} untrusted block summaries). Accessibility: {} ({} nodes, {} controls). Monitors: {}. Focus known: {}. Text fields seen: {}. Buttons seen: {}.",
        observation.active_window_display(),
        observation.active_window.source,
        observation.active_window.confidence * 100.0,
        observation.active_window.reliability,
        observation.visible_app_count,
        observation.visible_control_count(),
        observation.text_fields.len(),
        observation.buttons.len(),
        observation.dialogs.len(),
        observation.other_controls.len(),
        observation.disabled_control_count(),
        if observation.screenshot_available { "available" } else { "unavailable" },
        if observation.ocr_available { "available" } else { "unavailable" },
        observation.ocr_blocks.len(),
        if observation.accessibility_ok { "available" } else { "unavailable" },
        observation.accessibility.node_count,
        observation.accessibility.control_count,
        observation.monitors.len(),
        if observation.cursor_focus.keyboard_focus_known { "yes" } else { "no" },
        if text_sample.is_empty() { "none/names not exposed".into() } else { text_sample.join(", ") },
        if button_sample.is_empty() { "none/names not exposed".into() } else { button_sample.join(", ") },
    )
}

fn observation_completed_event(
    observation: &GuiObservationSnapshot,
    source_blockers: serde_json::Value,
) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    object.insert("type".into(), serde_json::json!("ObservationCompleted"));
    object.insert(
        "observation_id".into(),
        serde_json::json!(observation.observation_id),
    );
    object.insert(
        "active_window".into(),
        serde_json::json!(observation.active_window_label),
    );
    object.insert(
        "active_window_source".into(),
        serde_json::json!(observation.active_window.source),
    );
    object.insert(
        "active_window_confidence".into(),
        serde_json::json!(observation.active_window.confidence),
    );
    object.insert(
        "active_window_reliability".into(),
        serde_json::json!(observation.active_window.reliability),
    );
    object.insert(
        "active_window_blocker".into(),
        serde_json::json!(observation.active_window.blocker),
    );
    object.insert(
        "active_window_authority_source".into(),
        serde_json::json!(observation.active_window.source),
    );
    object.insert(
        "active_window_authority_confidence".into(),
        serde_json::json!(observation.active_window.confidence),
    );
    object.insert(
        "active_window_authority_status".into(),
        serde_json::json!(observation.active_window.authority_status),
    );
    object.insert(
        "gnome_bridge_status".into(),
        serde_json::json!(observation.active_window.gnome_bridge_status),
    );
    object.insert(
        "active_window_app".into(),
        serde_json::json!(observation.active_window.app_name),
    );
    object.insert(
        "active_window_app_id".into(),
        serde_json::json!(observation.active_window.app_id),
    );
    object.insert(
        "active_window_pid".into(),
        serde_json::json!(observation.active_window.pid),
    );
    object.insert(
        "active_window_workspace".into(),
        serde_json::json!(observation.active_window.workspace),
    );
    object.insert(
        "active_window_monitor".into(),
        serde_json::json!(observation.active_window.monitor),
    );
    object.insert(
        "active_window_fullscreen".into(),
        serde_json::json!(observation.active_window.fullscreen),
    );
    object.insert(
        "active_window_minimized".into(),
        serde_json::json!(observation.active_window.minimized),
    );
    object.insert(
        "active_window_observed_at_ms".into(),
        serde_json::json!(observation.active_window.observed_at_ms),
    );
    object.insert(
        "active_window_fallback_chain".into(),
        serde_json::to_value(&observation.active_window.fallback_chain)
            .unwrap_or(serde_json::Value::Null),
    );
    object.insert(
        "active_window_failure_chain".into(),
        serde_json::json!(active_window_failure_chain(observation)),
    );
    object.insert(
        "visible_app_count".into(),
        serde_json::json!(observation.visible_app_count),
    );
    object.insert(
        "visible_control_count".into(),
        serde_json::json!(observation.visible_control_count()),
    );
    object.insert(
        "visible_accessible_control_count".into(),
        serde_json::json!(observation.visible_accessible_control_count()),
    );
    object.insert(
        "disabled_control_count".into(),
        serde_json::json!(observation.disabled_control_count()),
    );
    object.insert(
        "hidden_control_count".into(),
        serde_json::json!(observation.hidden_control_count()),
    );
    object.insert(
        "trusted_control_count".into(),
        serde_json::json!(observation.control_quality_count("trusted")),
    );
    object.insert(
        "partial_control_count".into(),
        serde_json::json!(observation.control_quality_count("partial")),
    );
    object.insert(
        "not_executable_control_count".into(),
        serde_json::json!(observation.control_quality_count("not_executable")),
    );
    object.insert(
        "text_field_count".into(),
        serde_json::json!(observation.text_fields.len()),
    );
    object.insert(
        "button_count".into(),
        serde_json::json!(observation.buttons.len()),
    );
    object.insert(
        "dialog_count".into(),
        serde_json::json!(observation.dialogs.len()),
    );
    object.insert(
        "other_control_count".into(),
        serde_json::json!(observation.other_controls.len()),
    );
    object.insert(
        "ocr_available".into(),
        serde_json::json!(observation.ocr_available),
    );
    object.insert(
        "ocr_block_count".into(),
        serde_json::json!(observation.ocr_blocks.len()),
    );
    object.insert("ocr_trust".into(), serde_json::json!("untrusted"));
    object.insert(
        "ocr_wait_for_screenshot_ms".into(),
        serde_json::json!(observation.ocr_diagnostics.wait_for_screenshot_ms),
    );
    object.insert(
        "ocr_engine_selected".into(),
        serde_json::json!(observation.ocr_diagnostics.engine_selected),
    );
    object.insert(
        "ocr_engine_status".into(),
        serde_json::json!(observation.ocr_diagnostics.engine_status),
    );
    object.insert(
        "ocr_image_status".into(),
        serde_json::json!(observation.ocr_diagnostics.image_status),
    );
    object.insert(
        "ocr_total_ms".into(),
        serde_json::json!(observation.ocr_diagnostics.total_ms),
    );
    object.insert(
        "ocr_fast_path".into(),
        serde_json::json!(observation.ocr_diagnostics.fast_path),
    );
    object.insert(
        "ocr_cache_hit".into(),
        serde_json::json!(observation.ocr_diagnostics.cache_hit),
    );
    object.insert(
        "ocr_roi_count".into(),
        serde_json::json!(observation.ocr_diagnostics.roi_count),
    );
    object.insert(
        "ocr_changed_region_count".into(),
        serde_json::json!(observation.ocr_diagnostics.changed_region_count),
    );
    object.insert(
        "ocr_cold_start_ms".into(),
        serde_json::json!(observation.ocr_diagnostics.cold_start_ms),
    );
    object.insert(
        "ocr_warm_start_ms".into(),
        serde_json::json!(observation.ocr_diagnostics.warm_start_ms),
    );
    object.insert(
        "ocr_benchmark_summary".into(),
        serde_json::json!(observation.ocr_diagnostics.benchmark_summary),
    );
    object.insert(
        "ocr_injection_count".into(),
        serde_json::json!(observation
            .ocr_blocks
            .iter()
            .filter(|block| block.injection_suspected)
            .count()),
    );
    object.insert(
        "ocr_blocker".into(),
        serde_json::json!(observation.capabilities.ocr.blocker),
    );
    object.insert(
        "accessibility_available".into(),
        serde_json::json!(observation.accessibility_ok),
    );
    object.insert(
        "accessibility_source_status".into(),
        serde_json::json!(observation.accessibility.source_status),
    );
    object.insert(
        "accessibility_overall_status".into(),
        serde_json::json!(observation.accessibility.overall_status),
    );
    object.insert(
        "accessibility_overall_confidence".into(),
        serde_json::json!(observation.accessibility.overall_confidence),
    );
    object.insert(
        "accessibility_app_scores".into(),
        serde_json::to_value(&observation.accessibility.app_scores)
            .unwrap_or(serde_json::Value::Null),
    );
    object.insert(
        "accessibility_stale_node_count".into(),
        serde_json::json!(observation.accessibility.stale_node_count),
    );
    object.insert(
        "accessibility_timeout_count".into(),
        serde_json::json!(observation.accessibility.timeout_count),
    );
    object.insert(
        "accessibility_cache_hit_count".into(),
        serde_json::json!(observation.accessibility.cache_hit_count),
    );
    object.insert(
        "accessibility_stale_cache_rejected_count".into(),
        serde_json::json!(observation.accessibility.stale_cache_rejected_count),
    );
    object.insert(
        "accessibility_node_count".into(),
        serde_json::json!(observation.accessibility.node_count),
    );
    object.insert(
        "accessibility_control_count".into(),
        serde_json::json!(observation.accessibility.control_count),
    );
    object.insert(
        "atspi_snapshot_total_ms".into(),
        serde_json::json!(observation.accessibility.snapshot_total_ms),
    );
    object.insert(
        "atspi_skipped_app_count".into(),
        serde_json::json!(observation.accessibility.skipped_app_count),
    );
    object.insert(
        "atspi_omitted_node_count".into(),
        serde_json::json!(observation.accessibility.omitted_node_count),
    );
    object.insert(
        "accessibility_remediation".into(),
        serde_json::json!(observation.accessibility.remediation),
    );
    object.insert(
        "screenshot_available".into(),
        serde_json::json!(observation.screenshot_available),
    );
    object.insert(
        "screenshot_status".into(),
        serde_json::json!(if observation.screenshot_available {
            "available"
        } else {
            "unavailable"
        }),
    );
    object.insert(
        "screenshot_capture_ms".into(),
        serde_json::json!(probe_duration_ms(observation, "capture_screenshot")),
    );
    object.insert(
        "screenshot_duration_ms".into(),
        serde_json::json!(probe_duration_ms(observation, "capture_screenshot")),
    );
    object.insert(
        "screen_hash_prefix".into(),
        serde_json::json!(observation
            .screen_hash
            .as_ref()
            .map(|hash| hash.chars().take(16).collect::<String>())),
    );
    object.insert(
        "monitor_count".into(),
        serde_json::json!(observation.monitors.len()),
    );
    object.insert(
        "dpi_available".into(),
        serde_json::json!(!observation.monitors.is_empty()),
    );
    object.insert(
        "cursor_focus_known".into(),
        serde_json::json!(observation.cursor_focus.keyboard_focus_known),
    );
    object.insert(
        "focused_window".into(),
        serde_json::json!(observation.cursor_focus.focused_window_label),
    );
    object.insert(
        "focused_app".into(),
        serde_json::json!(observation.cursor_focus.focused_app),
    );
    object.insert(
        "focused_control_id".into(),
        serde_json::json!(observation.cursor_focus.focused_control_id),
    );
    object.insert(
        "focused_control_label".into(),
        serde_json::json!(observation.cursor_focus.focused_control_label),
    );
    object.insert(
        "focused_control_role".into(),
        serde_json::json!(observation.cursor_focus.focused_control_role),
    );
    object.insert(
        "focused_control_bounds".into(),
        serde_json::json!(observation.cursor_focus.focused_control_bounds),
    );
    object.insert(
        "text_cursor_known".into(),
        serde_json::json!(observation.cursor_focus.text_cursor_known),
    );
    object.insert(
        "editable_target_known".into(),
        serde_json::json!(observation.cursor_focus.editable_target_known),
    );
    object.insert(
        "terminal_like".into(),
        serde_json::json!(observation.cursor_focus.terminal_like),
    );
    object.insert(
        "focus_source".into(),
        serde_json::json!(observation.cursor_focus.source),
    );
    object.insert(
        "focus_confidence".into(),
        serde_json::json!(observation.cursor_focus.confidence),
    );
    object.insert(
        "focus_reliability".into(),
        serde_json::json!(observation.cursor_focus.reliability),
    );
    object.insert(
        "focus_adapter_status".into(),
        serde_json::json!(observation.cursor_focus.adapter_status),
    );
    object.insert(
        "focus_latency_ms".into(),
        serde_json::json!(observation.cursor_focus.latency_ms),
    );
    object.insert(
        "focus_failure_chain".into(),
        serde_json::to_value(&observation.cursor_focus.failure_chain)
            .unwrap_or(serde_json::Value::Null),
    );
    object.insert(
        "active_window_probe_ok".into(),
        serde_json::json!(observation.active_window_probe_ok),
    );
    object.insert(
        "desktop_state_probe_ok".into(),
        serde_json::json!(observation.desktop_state_probe_ok),
    );
    object.insert(
        "capabilities_probe_ok".into(),
        serde_json::json!(observation.capabilities_probe_ok),
    );
    object.insert(
        "observation_total_ms".into(),
        serde_json::json!(observation.timing.total_ms),
    );
    object.insert(
        "slowest_probe".into(),
        serde_json::json!(observation.timing.slowest_probe),
    );
    object.insert(
        "slowest_probe_ms".into(),
        serde_json::json!(observation.timing.slowest_probe_ms),
    );
    object.insert(
        "probe_timeout_count".into(),
        serde_json::json!(observation.timing.probe_timeout_count),
    );
    object.insert(
        "probe_timings".into(),
        serde_json::to_value(&observation.timing.probe_timings).unwrap_or(serde_json::Value::Null),
    );
    object.insert(
        "cache_hit".into(),
        serde_json::json!(observation.cache.cache_hit),
    );
    object.insert(
        "cache_age_ms".into(),
        serde_json::json!(observation.cache.cache_age_ms),
    );
    object.insert(
        "cache_policy".into(),
        serde_json::json!(observation.cache.cache_policy),
    );
    object.insert(
        "freshness".into(),
        serde_json::json!(observation.cache.freshness),
    );
    object.insert("source_blockers".into(), source_blockers);
    object.insert(
        "control_samples".into(),
        serde_json::json!(control_detail_sample(observation, 12)),
    );
    object.insert(
        "executable_control_count".into(),
        serde_json::json!(observation
            .all_controls()
            .iter()
            .filter(|control| control.is_executable_candidate())
            .count()),
    );
    object.insert(
        "visual_control_count".into(),
        serde_json::json!(observation.visual_controls.len()),
    );
    object.insert(
        "visual_control_summary".into(),
        visual_control_summary(observation),
    );
    serde_json::Value::Object(object)
}

fn source_blockers_json(observation: &GuiObservationSnapshot) -> serde_json::Value {
    serde_json::json!({
        "active_window": observation.capabilities.active_window.blocker,
        "desktop_state": observation.capabilities.desktop_state.blocker,
        "accessibility": observation.capabilities.accessibility.blocker,
        "screenshot": observation.capabilities.screenshot.blocker,
        "ocr": observation.capabilities.ocr.blocker,
        "monitor": observation.capabilities.monitor.blocker,
        "cursor_focus": observation.capabilities.cursor_focus.blocker,
    })
}

fn perception_summary_json(observation: &GuiObservationSnapshot) -> serde_json::Value {
    let mut value = observation_completed_event(observation, source_blockers_json(observation));
    if let Some(object) = value.as_object_mut() {
        object.remove("type");
        object.insert(
            "capabilities".into(),
            serde_json::to_value(&observation.capabilities).unwrap_or(serde_json::Value::Null),
        );
        object.insert(
            "text_field_sample".into(),
            serde_json::json!(control_sample(&observation.text_fields, 6)),
        );
        object.insert(
            "button_sample".into(),
            serde_json::json!(control_sample(&observation.buttons, 8)),
        );
        object.insert(
            "control_quality_summary".into(),
            control_quality_summary(observation),
        );
    }
    value
}

fn probe_duration_ms(observation: &GuiObservationSnapshot, probe_name: &str) -> Option<u64> {
    observation
        .timing
        .probe_timings
        .iter()
        .find(|timing| timing.probe_name == probe_name)
        .map(|timing| timing.duration_ms)
}

fn control_quality_summary(observation: &GuiObservationSnapshot) -> serde_json::Value {
    serde_json::json!({
        "trusted": observation.control_quality_count("trusted"),
        "partial": observation.control_quality_count("partial"),
        "not_executable": observation.control_quality_count("not_executable"),
        "executable": observation
            .all_controls()
            .iter()
            .filter(|control| control.is_executable_candidate())
            .count(),
    })
}

fn visual_control_summary(observation: &GuiObservationSnapshot) -> serde_json::Value {
    let matched_count = observation
        .visual_controls
        .iter()
        .filter(|control| control.matched_control_id.is_some())
        .count();
    serde_json::json!({
        "detected": observation.visual_controls.len(),
        "matched": matched_count,
        "unmatched": observation.visual_controls.len().saturating_sub(matched_count),
        "button_like": observation
            .visual_controls
            .iter()
            .filter(|control| {
                matches!(
                    control.control_type.as_str(),
                    "button" | "link" | "toggle" | "menu" | "tab"
                )
            })
            .count(),
        "false_positive_risk": "supporting_visual_only",
    })
}

fn active_window_failure_chain(observation: &GuiObservationSnapshot) -> Vec<serde_json::Value> {
    observation
        .active_window
        .fallback_chain
        .iter()
        .filter(|attempt| attempt.status != "matched")
        .map(|attempt| {
            serde_json::json!({
                "source": attempt.source,
                "status": attempt.status,
                "reliability": attempt.reliability,
                "reason": attempt.reason,
            })
        })
        .collect()
}

fn control_detail_sample(
    observation: &GuiObservationSnapshot,
    limit: usize,
) -> Vec<serde_json::Value> {
    observation
        .all_controls()
        .into_iter()
        .take(limit)
        .map(|control| {
            serde_json::json!({
                "id": control.control_id,
                "role": control.role,
                "label": control.name,
                "bounds": control.bounds,
                "enabled": control.enabled,
                "visible": control.visible,
                "focused": control.focused,
                "source": control.source,
                "confidence": control.confidence,
                "quality": control.quality,
                "label_source": control.label_source,
                "state_source": control.state_source,
                "rejection_reason": control.rejection_reason,
                "identity_confidence": control.identity_confidence,
                "bounds_confidence": control.bounds_confidence,
                "state_confidence": control.state_confidence,
                "executable_confidence": control.executable_confidence,
                "sources": control.sources,
            })
        })
        .collect()
}

#[allow(dead_code)]
fn action_summary(execution: &GuiActionExecution) -> serde_json::Value {
    serde_json::json!({
        "success": execution.success,
        "tool": execution.tool,
        "error": execution.error,
        "evidence": execution.evidence,
    })
}

fn action_backend_event(status: &GuiActionBackendStatus) -> serde_json::Value {
    let capabilities =
        serde_json::to_value(&status.capabilities).unwrap_or_else(|_| serde_json::json!({}));
    serde_json::json!({
        "type": "ActionBackendStatus",
        "global_halt_engaged": status.global_halt_engaged,
        "halt_kind": status.halt_kind,
        "halt_reason": status.halt_reason,
        "release_conditions": status.release_conditions,
        "startup_elapsed_ms": status.startup_elapsed_ms,
        "can_observe": status.can_observe,
        "can_plan": status.can_plan,
        "automation_enabled": status.automation_enabled,
        "vision_sidecar": status.vision_sidecar,
        "uinput_daemon": status.uinput_daemon,
        "orchestrator_available": status.orchestrator_available,
        "session_type": status.session_type,
        "xdotool_available": status.xdotool_available,
        "ydotool_available": status.ydotool_available,
        "uinput_available": status.uinput_available,
        "selected_backend": status.selected_backend,
        "backend_selection_reason": status.backend_selection_reason,
        "backend_probe_status": status.backend_probe_status,
        "backend_probe_errors": status.backend_probe_errors,
        "input_backend_kind": status.input_backend_kind,
        "focus_supported": status.focus_supported,
        "typing_supported": status.typing_supported,
        "click_supported": status.click_supported,
        "verification_supported": status.verification_supported,
        "xdotool_usable_for_actions": status.xdotool_usable_for_actions,
        "ydotool_usable_for_actions": status.ydotool_usable_for_actions,
        "uinput_socket_path": status.uinput_socket_path,
        "uinput_socket_accessible": status.uinput_socket_accessible,
        "can_execute_actions": status.can_execute_actions,
        "blockers": status.blockers,
        "capabilities": capabilities,
    })
}

fn action_backend_summary(status: &GuiActionBackendStatus) -> serde_json::Value {
    let capabilities =
        serde_json::to_value(&status.capabilities).unwrap_or_else(|_| serde_json::json!({}));
    serde_json::json!({
        "global_halt_engaged": status.global_halt_engaged,
        "halt_kind": status.halt_kind,
        "halt_reason": status.halt_reason,
        "release_conditions": status.release_conditions,
        "startup_elapsed_ms": status.startup_elapsed_ms,
        "can_observe": status.can_observe,
        "can_plan": status.can_plan,
        "automation_enabled": status.automation_enabled,
        "vision_sidecar": status.vision_sidecar,
        "uinput_daemon": status.uinput_daemon,
        "orchestrator_available": status.orchestrator_available,
        "session_type": status.session_type,
        "xdotool_available": status.xdotool_available,
        "ydotool_available": status.ydotool_available,
        "uinput_available": status.uinput_available,
        "selected_backend": status.selected_backend,
        "backend_selection_reason": status.backend_selection_reason,
        "backend_probe_status": status.backend_probe_status,
        "backend_probe_errors": status.backend_probe_errors,
        "input_backend_kind": status.input_backend_kind,
        "focus_supported": status.focus_supported,
        "typing_supported": status.typing_supported,
        "click_supported": status.click_supported,
        "verification_supported": status.verification_supported,
        "xdotool_usable_for_actions": status.xdotool_usable_for_actions,
        "ydotool_usable_for_actions": status.ydotool_usable_for_actions,
        "uinput_socket_path": status.uinput_socket_path,
        "uinput_socket_accessible": status.uinput_socket_accessible,
        "can_execute_actions": status.can_execute_actions,
        "blockers": status.blockers,
        "capabilities": capabilities,
    })
}

fn verification_summary(report: &GuiVerificationReport) -> serde_json::Value {
    serde_json::json!({
        "status": report.status,
        "confidence": report.confidence,
        "after_observation_id": report.after_observation_id,
    })
}

fn blocker_summary(blocker: &GuiBlocker) -> serde_json::Value {
    serde_json::json!({
        "kind": blocker.kind,
        "reason": blocker.reason,
        "candidate_count": blocker.candidate_count,
        "target_name": blocker.target_name,
        "options": blocker.options,
        "clarification_question": blocker.clarification_question,
    })
}
