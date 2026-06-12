use std::collections::HashMap;

use super::context::{GuiContext, GuiContextFreshness};
use super::llm_planner::{typed_plan_steps, GuiLlmPlan, GuiPlanValidationReport, GuiTypedPlanStep};
use super::perception::{
    matching_controls, sanitize_gui_text, stable_hash, GuiBounds, GuiControlSummary,
};

const TARGET_TEXT_LIMIT: usize = 120;
const RESOLVED_THRESHOLD: f64 = 0.85;
const CLARIFY_THRESHOLD: f64 = 0.60;
const AMBIGUITY_GAP: f64 = 0.10;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedGuiTarget {
    pub role: String,
    pub name: String,
    pub target_type: String,
    pub confidence: f64,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TargetResolution {
    Resolved(ResolvedGuiTarget),
    Missing {
        reason: String,
        candidate_count: usize,
    },
    Ambiguous {
        reason: String,
        candidate_count: usize,
    },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiTargetResolutionSummary {
    pub resolution_id: String,
    pub plan_id: String,
    pub validation_id: Option<String>,
    pub goal_contract_id: Option<String>,
    pub context_id: String,
    pub observation_id: String,
    pub status: String,
    pub results: Vec<GuiTargetResolutionResult>,
    #[serde(default)]
    pub resolved_target: Option<GuiResolvedTarget>,
    pub can_proceed_to_safety_gate: bool,
    pub can_execute: bool,
    pub blocker_count: usize,
    pub blockers: Vec<String>,
    pub ambiguity_count: usize,
    pub ambiguity_reasons: Vec<String>,
    pub confidence: f64,
    pub prompt_hash: Option<String>,
}

impl GuiTargetResolutionSummary {
    pub fn event_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "TargetResolutionCompleted",
            "resolution_id": self.resolution_id,
            "plan_id": self.plan_id,
            "validation_id": self.validation_id,
            "goal_contract_id": self.goal_contract_id,
            "context_id": self.context_id,
            "observation_id": self.observation_id,
            "status": self.status,
            "results": self.results,
            "resolved_target": self.resolved_target,
            "candidates": self.primary_candidates(),
            "confidence": self.confidence,
            "ambiguity_count": self.ambiguity_count,
            "ambiguity_reasons": self.ambiguity_reasons,
            "blocker_count": self.blocker_count,
            "blockers": self.blockers,
            "can_proceed_to_safety_gate": self.can_proceed_to_safety_gate,
            "can_execute": false,
            "prompt_hash": self.prompt_hash,
        })
    }

    pub fn summary_json(&self) -> serde_json::Value {
        let mut payload = self.event_payload();
        if let Some(object) = payload.as_object_mut() {
            object.remove("type");
        }
        payload
    }

    pub fn skipped(
        plan: &GuiLlmPlan,
        validation: &GuiPlanValidationReport,
        context: &GuiContext,
        plan_id: &str,
        reason: impl Into<String>,
    ) -> Self {
        let reason = sanitize_reason(reason);
        Self {
            resolution_id: format!("resolution-{plan_id}"),
            plan_id: plan_id.to_string(),
            validation_id: validation.validation_id.clone(),
            goal_contract_id: plan
                .goal_contract_id
                .clone()
                .or(validation.goal_contract_id.clone()),
            context_id: context.context_id.clone(),
            observation_id: context.observation_id.clone(),
            status: "skipped".into(),
            results: Vec::new(),
            resolved_target: None,
            can_proceed_to_safety_gate: false,
            can_execute: false,
            blocker_count: 1,
            blockers: vec![reason],
            ambiguity_count: 0,
            ambiguity_reasons: Vec::new(),
            confidence: 0.0,
            prompt_hash: plan.prompt_hash.clone().or(validation.prompt_hash.clone()),
        }
    }

    fn primary_candidates(&self) -> Vec<GuiTargetCandidate> {
        self.results
            .iter()
            .find(|result| !result.candidates.is_empty())
            .map(|result| result.candidates.clone())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiTargetResolutionResult {
    pub step_id: String,
    pub step_type: String,
    pub target_query: String,
    pub target_kind: String,
    pub status: String,
    #[serde(default)]
    pub resolved_target: Option<GuiResolvedTarget>,
    #[serde(default)]
    pub candidates: Vec<GuiTargetCandidate>,
    pub confidence: f64,
    pub requires_approval: bool,
    pub can_proceed_to_safety_gate: bool,
    pub can_execute: bool,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub ambiguity_reasons: Vec<String>,
    #[serde(default)]
    pub source_evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiResolvedTarget {
    pub control_id: String,
    pub target_hash: String,
    pub label: String,
    pub role: String,
    pub target_kind: String,
    pub app_hint: Option<String>,
    pub window_hint: Option<String>,
    pub bounds: Option<GuiBounds>,
    pub enabled: bool,
    pub visible: bool,
    pub focused: bool,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiTargetCandidate {
    pub candidate_id: String,
    pub control_id: String,
    pub target_hash: String,
    pub label: String,
    pub role: String,
    pub bounds: Option<GuiBounds>,
    pub app_hint: Option<String>,
    pub window_hint: Option<String>,
    pub visible: bool,
    pub enabled: bool,
    pub focused: bool,
    pub quality: String,
    pub sources: Vec<String>,
    pub identity_confidence: f64,
    pub role_confidence: f64,
    pub label_confidence: f64,
    pub bounds_confidence: f64,
    pub state_confidence: f64,
    pub app_window_confidence: f64,
    pub focus_confidence: f64,
    pub final_confidence: f64,
    #[serde(default)]
    pub rejection_reason: Option<String>,
}

pub fn resolve_plan_targets(
    plan: &GuiLlmPlan,
    validation: &GuiPlanValidationReport,
    context: &GuiContext,
    plan_id: &str,
) -> GuiTargetResolutionSummary {
    let can_resolve_for_approval = matches!(
        validation.status,
        super::llm_planner::GuiPlanValidationStatus::ApprovalRequired
    ) || validation.readiness_status.as_deref() == Some("approval_required");
    if !validation.can_proceed_to_target_resolution && !can_resolve_for_approval {
        return GuiTargetResolutionSummary::skipped(
            plan,
            validation,
            context,
            plan_id,
            "Plan validation did not allow Step 5 target resolution.",
        );
    }

    let steps = typed_plan_steps(plan);
    let candidates = collect_target_candidates(context);
    let mut prior_resolutions: HashMap<String, GuiResolvedTarget> = HashMap::new();
    let mut results = Vec::new();

    for (index, step) in steps.iter().enumerate() {
        if !step_should_emit_resolution(step) {
            continue;
        }
        let has_prior_app_prerequisite = steps[..index].iter().any(|prior| {
            prior.step_type == "OpenApp"
                || (prior.step_type == "SwitchWindow"
                    && prior
                        .target_app_hint
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty()))
        });
        let result = resolve_step_target(
            plan,
            step,
            context,
            &candidates,
            &prior_resolutions,
            has_prior_app_prerequisite,
        );
        if let Some(target) = result.resolved_target.clone() {
            prior_resolutions.insert(step.step_id.clone(), target);
        }
        results.push(result);
    }

    if results.is_empty() {
        return GuiTargetResolutionSummary::skipped(
            plan,
            validation,
            context,
            plan_id,
            "Plan has no target-resolution-relevant typed steps.",
        );
    }

    let mut blockers = Vec::new();
    let mut ambiguity_reasons = Vec::new();
    let mut resolved_target = None;
    let mut fallback_resolved_target = None;
    let mut confidence = 0.0_f64;

    for result in &results {
        blockers.extend(result.blockers.iter().cloned());
        ambiguity_reasons.extend(result.ambiguity_reasons.iter().cloned());
        if fallback_resolved_target.is_none() {
            fallback_resolved_target = result.resolved_target.clone();
        }
        if resolved_target.is_none()
            && result
                .resolved_target
                .as_ref()
                .is_some_and(is_control_resolved_target)
        {
            resolved_target = result.resolved_target.clone();
        }
        confidence = confidence.max(result.confidence);
    }
    if resolved_target.is_none() {
        resolved_target = fallback_resolved_target;
    }

    let status = aggregate_status(&results);
    let can_proceed_to_safety_gate = status == "resolved"
        && results.iter().all(|result| {
            result.can_proceed_to_safety_gate || !is_action_target_step(&result.step_type)
        });

    GuiTargetResolutionSummary {
        resolution_id: format!("resolution-{plan_id}"),
        plan_id: plan_id.to_string(),
        validation_id: validation.validation_id.clone(),
        goal_contract_id: plan
            .goal_contract_id
            .clone()
            .or(validation.goal_contract_id.clone()),
        context_id: context.context_id.clone(),
        observation_id: context.observation_id.clone(),
        status,
        results,
        resolved_target,
        can_proceed_to_safety_gate,
        can_execute: false,
        blocker_count: blockers.len(),
        blockers,
        ambiguity_count: ambiguity_reasons.len(),
        ambiguity_reasons,
        confidence: confidence.clamp(0.0, 1.0),
        prompt_hash: plan.prompt_hash.clone().or(validation.prompt_hash.clone()),
    }
}

pub fn collect_target_candidates(context: &GuiContext) -> Vec<GuiTargetCandidate> {
    context
        .fused_controls
        .iter()
        .enumerate()
        .map(|(index, control)| candidate_from_control(context, control, index))
        .collect()
}

pub fn named_text_fields(context: &GuiContext) -> Vec<GuiControlSummary> {
    context
        .executable_text_fields()
        .iter()
        .filter(|field| !field.name.trim().is_empty())
        .cloned()
        .collect()
}

pub fn resolve_unique_text_field(context: &GuiContext) -> TargetResolution {
    let named_fields = named_text_fields(context);
    if named_fields.len() == 1 {
        let target = &named_fields[0];
        TargetResolution::Resolved(ResolvedGuiTarget {
            role: target.role.clone(),
            name: target.name.clone(),
            target_type: "text_field".into(),
            confidence: 0.86,
            evidence: "single named accessible text field".into(),
        })
    } else if named_fields.is_empty() {
        let reason = if context.observation.text_fields.is_empty() {
            "No visible accessible text field was found."
        } else {
            "Text fields exist, but their accessible names are not exposed, so I cannot safely choose one."
        };
        TargetResolution::Missing {
            reason: reason.into(),
            candidate_count: named_fields.len(),
        }
    } else {
        TargetResolution::Ambiguous {
            reason: "Multiple named text fields are visible, so choosing one would be a guess."
                .into(),
            candidate_count: named_fields.len(),
        }
    }
}

pub fn resolve_type_text_target(context: &GuiContext) -> TargetResolution {
    let executable_fields = context.executable_text_fields();
    if executable_fields.is_empty() {
        return TargetResolution::Missing {
            reason: "No visible accessible text field was found.".into(),
            candidate_count: 0,
        };
    }

    let named_fields = named_text_fields(context);
    if executable_fields.len() > 1 && named_fields.len() != 1 {
        return TargetResolution::Ambiguous {
            reason: "Multiple text fields are visible and no unique labeled target was found."
                .into(),
            candidate_count: executable_fields.len(),
        };
    }

    if named_fields.len() == 1 {
        let target = &named_fields[0];
        TargetResolution::Resolved(ResolvedGuiTarget {
            role: target.role.clone(),
            name: target.name.clone(),
            target_type: "text_field".into(),
            confidence: 0.86,
            evidence: "single named text field".into(),
        })
    } else {
        TargetResolution::Resolved(ResolvedGuiTarget {
            role: "text".into(),
            name: "focused/first visible text field".into(),
            target_type: "text_field".into(),
            confidence: 0.68,
            evidence: "single visible/focused text field".into(),
        })
    }
}

pub fn resolve_button(context: &GuiContext, control_name: &str) -> TargetResolution {
    let executable_buttons = context.executable_buttons();
    let matches = matching_controls(&executable_buttons, control_name);
    if matches.len() == 1 {
        let target = &matches[0];
        TargetResolution::Resolved(ResolvedGuiTarget {
            role: target.role.clone(),
            name: target.name.clone(),
            target_type: "button".into(),
            confidence: 0.88,
            evidence: "single matching accessible button".into(),
        })
    } else if matches.is_empty() {
        TargetResolution::Missing {
            reason: "No matching accessible button/control was found.".into(),
            candidate_count: 0,
        }
    } else {
        TargetResolution::Ambiguous {
            reason: "Multiple matching buttons/controls were found.".into(),
            candidate_count: matches.len(),
        }
    }
}

fn resolve_step_target(
    plan: &GuiLlmPlan,
    step: &GuiTypedPlanStep,
    context: &GuiContext,
    candidates: &[GuiTargetCandidate],
    prior_resolutions: &HashMap<String, GuiResolvedTarget>,
    has_prior_app_prerequisite: bool,
) -> GuiTargetResolutionResult {
    if contains_raw_coordinate(&step.summary)
        || contains_raw_coordinate(&step.reason)
        || step
            .target_control_hint
            .as_deref()
            .is_some_and(contains_raw_coordinate)
    {
        return blocked_result(
            step,
            "unknown",
            "rejected",
            "Raw coordinate target instructions are not allowed in Step 5.",
        );
    }

    if should_defer_until_planned_app(step, context, has_prior_app_prerequisite) {
        return deferred_result(
            step,
            target_kind_for_deferred_step(step),
            "Target resolution is deferred until the planned app/window step changes the GUI context.",
        );
    }

    match step.step_type.as_str() {
        "OpenApp" => resolve_app_step(step),
        "SwitchWindow" => resolve_window_step(step, context),
        "FocusField" => resolve_control_step(step, context, candidates, "text_field", None),
        "TypeText" => resolve_type_text_step(step, context, candidates, prior_resolutions),
        "ClickControl" => {
            resolve_control_step(step, context, candidates, target_kind_for_click(step), None)
        }
        "PressKey" => resolve_press_key_step(step, context, prior_resolutions),
        "RequireApproval" => approval_result(step),
        "AskClarification" => clarification_result(step),
        "WaitForState" | "VerifyState" | "SummarizeVisibleContent" => metadata_result(step),
        _ => {
            let fallback = if plan.goal_action_type.as_deref() == Some("browser_search") {
                "browser search target metadata only"
            } else {
                "unsupported target step"
            };
            blocked_result(step, "unknown", "blocked", fallback)
        }
    }
}

fn resolve_app_step(step: &GuiTypedPlanStep) -> GuiTargetResolutionResult {
    let app = step
        .target_app_hint
        .as_deref()
        .map(sanitize_target_text)
        .filter(|value| !value.is_empty());
    if app.is_none() {
        return blocked_result(
            step,
            "app",
            "needs_clarification",
            "OpenApp has no app hint to resolve.",
        );
    }
    GuiTargetResolutionResult {
        step_id: sanitize_target_text(&step.step_id),
        step_type: sanitize_target_text(&step.step_type),
        target_query: app.clone().unwrap_or_default(),
        target_kind: "app".into(),
        status: "resolved".into(),
        resolved_target: Some(GuiResolvedTarget {
            control_id: format!("app:{}", stable_hash(app.as_deref().unwrap_or_default())),
            target_hash: stable_hash(&format!(
                "app|{}|{}",
                app.as_deref().unwrap_or_default(),
                step.step_id
            )),
            label: app.unwrap_or_default(),
            role: "application".into(),
            target_kind: "app".into(),
            app_hint: step.target_app_hint.as_deref().map(sanitize_target_text),
            window_hint: step.target_window_hint.as_deref().map(sanitize_target_text),
            bounds: None,
            enabled: true,
            visible: false,
            focused: false,
            source: "goal_contract".into(),
        }),
        candidates: Vec::new(),
        confidence: 0.9,
        requires_approval: step.requires_approval,
        can_proceed_to_safety_gate: true,
        can_execute: false,
        blockers: Vec::new(),
        ambiguity_reasons: Vec::new(),
        source_evidence: vec!["app hint from typed plan".into()],
    }
}

fn resolve_window_step(step: &GuiTypedPlanStep, context: &GuiContext) -> GuiTargetResolutionResult {
    let query = step
        .target_window_hint
        .as_deref()
        .or(step.target_app_hint.as_deref())
        .map(sanitize_target_text)
        .unwrap_or_default();
    if query.is_empty() {
        return blocked_result(
            step,
            "window",
            "needs_clarification",
            "SwitchWindow has no app/window hint to resolve.",
        );
    }
    let confidence = if context
        .active_window
        .label
        .to_lowercase()
        .contains(&query.to_lowercase())
    {
        0.9
    } else {
        0.72
    };
    GuiTargetResolutionResult {
        step_id: sanitize_target_text(&step.step_id),
        step_type: sanitize_target_text(&step.step_type),
        target_query: query.clone(),
        target_kind: "window".into(),
        status: if confidence >= RESOLVED_THRESHOLD {
            "resolved".into()
        } else {
            "needs_clarification".into()
        },
        resolved_target: (confidence >= RESOLVED_THRESHOLD).then(|| GuiResolvedTarget {
            control_id: format!("window:{}", stable_hash(&query)),
            target_hash: stable_hash(&format!("window|{}|{}", query, context.context_id)),
            label: query,
            role: "window".into(),
            target_kind: "window".into(),
            app_hint: step.target_app_hint.as_deref().map(sanitize_target_text),
            window_hint: step.target_window_hint.as_deref().map(sanitize_target_text),
            bounds: None,
            enabled: true,
            visible: true,
            focused: false,
            source: "active_window_context".into(),
        }),
        candidates: Vec::new(),
        confidence,
        requires_approval: step.requires_approval,
        can_proceed_to_safety_gate: confidence >= RESOLVED_THRESHOLD,
        can_execute: false,
        blockers: if confidence >= RESOLVED_THRESHOLD {
            Vec::new()
        } else {
            vec!["Window hint is not uniquely matched to the active window context.".into()]
        },
        ambiguity_reasons: Vec::new(),
        source_evidence: vec!["active window context".into()],
    }
}

fn resolve_type_text_step(
    step: &GuiTypedPlanStep,
    context: &GuiContext,
    candidates: &[GuiTargetCandidate],
    prior_resolutions: &HashMap<String, GuiResolvedTarget>,
) -> GuiTargetResolutionResult {
    if step
        .text_payload_summary
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
        && step
            .text_payload_hash
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        return blocked_result(
            step,
            "text_field",
            "blocked",
            "TypeText has no safe text payload summary or hash.",
        );
    }

    if let Some(previous) = prior_resolutions
        .values()
        .find(|target| role_group(&target.role) == "editable")
        .cloned()
    {
        return resolved_from_prior(step, previous);
    }

    if context.focus_state.editable_target_known {
        if let Some(control_id) = &context.focus_state.focused_control_id {
            if let Some(candidate) = candidates
                .iter()
                .find(|candidate| &candidate.control_id == control_id)
            {
                return resolved_candidate_result(step, candidate.clone(), "text_field");
            }
        }
    }

    resolve_control_step(
        step,
        context,
        candidates,
        "text_field",
        Some("editable target"),
    )
}

fn resolve_press_key_step(
    step: &GuiTypedPlanStep,
    context: &GuiContext,
    prior_resolutions: &HashMap<String, GuiResolvedTarget>,
) -> GuiTargetResolutionResult {
    if context.focus_state.keyboard_focus_known
        || prior_resolutions
            .values()
            .any(|target| role_group(&target.role) == "editable")
    {
        GuiTargetResolutionResult {
            step_id: sanitize_target_text(&step.step_id),
            step_type: sanitize_target_text(&step.step_type),
            target_query: "focused context".into(),
            target_kind: "focused_context".into(),
            status: "resolved".into(),
            resolved_target: None,
            candidates: Vec::new(),
            confidence: context.focus_state.confidence.max(0.82).clamp(0.0, 1.0),
            requires_approval: step.requires_approval,
            can_proceed_to_safety_gate: true,
            can_execute: false,
            blockers: Vec::new(),
            ambiguity_reasons: Vec::new(),
            source_evidence: vec!["focus authority or prior focus-field resolution".into()],
        }
    } else {
        blocked_result(
            step,
            "focused_context",
            "blocked",
            "PressKey requires known focus or a prior resolved editable target.",
        )
    }
}

fn approval_result(step: &GuiTypedPlanStep) -> GuiTargetResolutionResult {
    GuiTargetResolutionResult {
        step_id: sanitize_target_text(&step.step_id),
        step_type: sanitize_target_text(&step.step_type),
        target_query: "approval required".into(),
        target_kind: "none".into(),
        status: "resolved".into(),
        resolved_target: None,
        candidates: Vec::new(),
        confidence: 0.95,
        requires_approval: true,
        can_proceed_to_safety_gate: true,
        can_execute: false,
        blockers: Vec::new(),
        ambiguity_reasons: Vec::new(),
        source_evidence: vec!["approval metadata only".into()],
    }
}

fn clarification_result(step: &GuiTypedPlanStep) -> GuiTargetResolutionResult {
    GuiTargetResolutionResult {
        step_id: sanitize_target_text(&step.step_id),
        step_type: sanitize_target_text(&step.step_type),
        target_query: step
            .target_control_hint
            .as_deref()
            .map(sanitize_target_text)
            .unwrap_or_else(|| "clarification required".into()),
        target_kind: "none".into(),
        status: "needs_clarification".into(),
        resolved_target: None,
        candidates: Vec::new(),
        confidence: 0.0,
        requires_approval: step.requires_approval,
        can_proceed_to_safety_gate: false,
        can_execute: false,
        blockers: vec!["Plan asks for clarification before target resolution.".into()],
        ambiguity_reasons: vec!["clarification_requested".into()],
        source_evidence: vec!["typed plan clarification step".into()],
    }
}

fn metadata_result(step: &GuiTypedPlanStep) -> GuiTargetResolutionResult {
    GuiTargetResolutionResult {
        step_id: sanitize_target_text(&step.step_id),
        step_type: sanitize_target_text(&step.step_type),
        target_query: sanitize_target_text(&step.expected_postcondition),
        target_kind: "none".into(),
        status: "resolved".into(),
        resolved_target: None,
        candidates: Vec::new(),
        confidence: step.confidence.clamp(0.0, 0.9),
        requires_approval: step.requires_approval,
        can_proceed_to_safety_gate: true,
        can_execute: false,
        blockers: Vec::new(),
        ambiguity_reasons: Vec::new(),
        source_evidence: vec!["state/summary step has no GUI target".into()],
    }
}

fn resolve_control_step(
    step: &GuiTypedPlanStep,
    context: &GuiContext,
    candidates: &[GuiTargetCandidate],
    target_kind: &str,
    fallback_query: Option<&str>,
) -> GuiTargetResolutionResult {
    let target_query = target_query_for_step(step, fallback_query);
    if target_query.is_empty() || generic_target_without_label(&target_query, step) {
        return blocked_result(
            step,
            target_kind,
            "needs_clarification",
            "Control target hint is generic or missing; Step 5 will not guess.",
        );
    }

    if matches!(context.freshness, GuiContextFreshness::Stale) {
        return blocked_result(
            step,
            target_kind,
            "blocked",
            "GUI context is stale; target resolution requires a fresh observation.",
        );
    }

    let role_groups = expected_role_groups(step, target_kind);
    let mut scored = candidates
        .iter()
        .filter_map(|candidate| {
            let mut scored = candidate.clone();
            score_candidate_for_step(step, &target_query, &role_groups, context, &mut scored);
            let keep = if step.step_type == "ClickControl" {
                scored.role_confidence > 0.0 && scored.label_confidence > 0.0
            } else {
                scored.role_confidence > 0.0
                    && (scored.label_confidence > 0.0
                        || generic_editable_query(&normalized_target_query(&target_query, step)))
            };
            if keep {
                Some(scored)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .final_confidence
            .partial_cmp(&left.final_confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    decide_resolution_status(step, target_kind, target_query, context, scored)
}

fn decide_resolution_status(
    step: &GuiTypedPlanStep,
    target_kind: &str,
    target_query: String,
    context: &GuiContext,
    candidates: Vec<GuiTargetCandidate>,
) -> GuiTargetResolutionResult {
    if candidates.is_empty() {
        return blocked_result(
            step,
            target_kind,
            "blocked",
            "No matching GUI target candidates were found.",
        );
    }

    let top = candidates[0].clone();
    let mut ambiguity_reasons = ambiguity_reasons_for(&candidates, context);
    let mut blockers = Vec::new();

    if let Some(reason) = &top.rejection_reason {
        blockers.push(reason.clone());
    }
    if top.rejection_reason.is_some() && top.final_confidence < CLARIFY_THRESHOLD {
        return GuiTargetResolutionResult {
            step_id: sanitize_target_text(&step.step_id),
            step_type: sanitize_target_text(&step.step_type),
            target_query,
            target_kind: target_kind.into(),
            status: "blocked".into(),
            resolved_target: None,
            confidence: top.final_confidence,
            requires_approval: step.requires_approval,
            can_proceed_to_safety_gate: false,
            can_execute: false,
            blockers,
            ambiguity_reasons: Vec::new(),
            source_evidence: top.sources.clone(),
            candidates,
        };
    }

    let ambiguous = !ambiguity_reasons.is_empty();
    if ambiguous {
        return GuiTargetResolutionResult {
            step_id: sanitize_target_text(&step.step_id),
            step_type: sanitize_target_text(&step.step_type),
            target_query,
            target_kind: target_kind.into(),
            status: "ambiguous".into(),
            resolved_target: None,
            confidence: top.final_confidence,
            requires_approval: step.requires_approval,
            can_proceed_to_safety_gate: false,
            can_execute: false,
            blockers,
            ambiguity_reasons: std::mem::take(&mut ambiguity_reasons),
            source_evidence: top.sources.clone(),
            candidates,
        };
    }

    if top.final_confidence >= RESOLVED_THRESHOLD && top.rejection_reason.is_none() {
        let resolved_target = resolved_target_from_candidate(&top, target_kind);
        return GuiTargetResolutionResult {
            step_id: sanitize_target_text(&step.step_id),
            step_type: sanitize_target_text(&step.step_type),
            target_query,
            target_kind: target_kind.into(),
            status: "resolved".into(),
            resolved_target: Some(resolved_target),
            confidence: top.final_confidence,
            requires_approval: step.requires_approval,
            can_proceed_to_safety_gate: true,
            can_execute: false,
            blockers,
            ambiguity_reasons,
            source_evidence: top.sources.clone(),
            candidates,
        };
    }

    let status = if top.final_confidence >= CLARIFY_THRESHOLD {
        "needs_clarification"
    } else {
        "blocked"
    };
    blockers.push(if status == "needs_clarification" {
        "Best target candidate is below the safe resolution threshold.".into()
    } else {
        "No safe executable target candidate reached minimum confidence.".into()
    });
    GuiTargetResolutionResult {
        step_id: sanitize_target_text(&step.step_id),
        step_type: sanitize_target_text(&step.step_type),
        target_query,
        target_kind: target_kind.into(),
        status: status.into(),
        resolved_target: None,
        confidence: top.final_confidence,
        requires_approval: step.requires_approval,
        can_proceed_to_safety_gate: false,
        can_execute: false,
        blockers,
        ambiguity_reasons,
        source_evidence: top.sources.clone(),
        candidates,
    }
}

fn score_candidate_for_step(
    step: &GuiTypedPlanStep,
    target_query: &str,
    role_groups: &[&'static str],
    context: &GuiContext,
    candidate: &mut GuiTargetCandidate,
) {
    let candidate_group = role_group(&candidate.role);
    candidate.role_confidence = if role_groups.contains(&candidate_group) {
        1.0
    } else {
        0.0
    };

    candidate.label_confidence = label_match_confidence(target_query, &candidate.label, step);
    candidate.state_confidence = if candidate.enabled && candidate.visible {
        1.0
    } else {
        0.2
    };
    candidate.bounds_confidence = if candidate.bounds.is_some() { 1.0 } else { 0.0 };
    candidate.app_window_confidence = if candidate_app_window_matches(step, candidate, context) {
        1.0
    } else if active_window_unknown(context) {
        0.75
    } else {
        0.45
    };
    candidate.focus_confidence = if candidate.focused { 1.0 } else { 0.45 };

    let mut confidence = 0.30 * candidate.role_confidence
        + 0.25 * candidate.label_confidence
        + 0.20 * candidate.state_confidence
        + 0.15 * candidate.bounds_confidence
        + 0.05 * candidate.app_window_confidence
        + 0.05 * candidate.focus_confidence;

    let mut cap = 1.0_f64;
    if candidate.bounds.is_none() && is_control_step(&step.step_type) {
        cap = cap.min(0.59);
        candidate.rejection_reason = Some("control target has no bounds".into());
    }
    if source_only(candidate, "visual_detector")
        || source_only(candidate, "fixture_visual_detector")
    {
        cap = cap.min(0.59);
        candidate.rejection_reason = Some("visual-only target is supporting evidence only".into());
    }
    if source_only(candidate, "ocr_label_evidence") || source_only(candidate, "ocr") {
        cap = cap.min(0.59);
        candidate.rejection_reason = Some("ocr_only_not_executable".into());
    }
    if candidate.quality == "partial" {
        cap = cap.min(0.84);
    }
    if candidate.quality == "not_executable" {
        cap = cap.min(0.59);
        candidate
            .rejection_reason
            .get_or_insert_with(|| "candidate is marked not_executable".into());
    }
    if !candidate.enabled || !candidate.visible {
        cap = cap.min(0.59);
        candidate.rejection_reason = Some("candidate is hidden or disabled".into());
    }
    if active_window_unknown(context) {
        cap = cap.min(0.88);
    }
    confidence = confidence.min(cap);
    candidate.final_confidence = confidence.clamp(0.0, 1.0);
}

fn candidate_from_control(
    context: &GuiContext,
    control: &GuiControlSummary,
    index: usize,
) -> GuiTargetCandidate {
    let label = sanitize_target_text(&control.name);
    let role = sanitize_target_text(&control.role);
    let bounds_hash = control
        .bounds
        .as_ref()
        .map(|bounds| {
            format!(
                "{}:{}:{}:{}",
                bounds.x, bounds.y, bounds.width, bounds.height
            )
        })
        .unwrap_or_else(|| "no-bounds".into());
    let target_hash = stable_hash(&format!(
        "{}|{}|{}|{}|{}",
        control.control_id,
        role,
        stable_hash(&label),
        stable_hash(&bounds_hash),
        context.context_id
    ));
    let mut sources = if control.sources.is_empty() {
        vec![sanitize_target_text(&control.source)]
    } else {
        control
            .sources
            .iter()
            .map(|source| sanitize_target_text(source))
            .collect::<Vec<_>>()
    };
    if control.focused && !sources.iter().any(|source| source == "focus_authority") {
        sources.push("focus_authority".into());
    }
    if control.in_active_window && !sources.iter().any(|source| source == "active_window") {
        sources.push("active_window".into());
    }
    GuiTargetCandidate {
        candidate_id: format!("candidate-{index}"),
        control_id: sanitize_target_text(&control.control_id),
        target_hash,
        label,
        role,
        bounds: control.bounds.clone(),
        app_hint: context
            .active_window
            .app_name
            .as_ref()
            .map(|value| sanitize_target_text(value)),
        window_hint: Some(sanitize_target_text(&context.active_window.label)),
        visible: control.visible,
        enabled: control.enabled,
        focused: control.focused,
        quality: sanitize_target_text(&control.quality),
        sources,
        identity_confidence: control.identity_confidence.clamp(0.0, 1.0),
        role_confidence: 0.0,
        label_confidence: 0.0,
        bounds_confidence: control.bounds_confidence.clamp(0.0, 1.0),
        state_confidence: control.state_confidence.clamp(0.0, 1.0),
        app_window_confidence: if control.in_active_window { 1.0 } else { 0.45 },
        focus_confidence: if control.focused { 1.0 } else { 0.0 },
        final_confidence: 0.0,
        rejection_reason: control.rejection_reason.as_deref().map(sanitize_reason),
    }
}

fn resolved_target_from_candidate(
    candidate: &GuiTargetCandidate,
    target_kind: &str,
) -> GuiResolvedTarget {
    GuiResolvedTarget {
        control_id: candidate.control_id.clone(),
        target_hash: candidate.target_hash.clone(),
        label: candidate.label.clone(),
        role: candidate.role.clone(),
        target_kind: target_kind.into(),
        app_hint: candidate.app_hint.clone(),
        window_hint: candidate.window_hint.clone(),
        bounds: candidate.bounds.clone(),
        enabled: candidate.enabled,
        visible: candidate.visible,
        focused: candidate.focused,
        source: candidate.sources.join("+"),
    }
}

fn resolved_candidate_result(
    step: &GuiTypedPlanStep,
    candidate: GuiTargetCandidate,
    target_kind: &str,
) -> GuiTargetResolutionResult {
    GuiTargetResolutionResult {
        step_id: sanitize_target_text(&step.step_id),
        step_type: sanitize_target_text(&step.step_type),
        target_query: target_query_for_step(step, None),
        target_kind: target_kind.into(),
        status: "resolved".into(),
        resolved_target: Some(resolved_target_from_candidate(&candidate, target_kind)),
        candidates: vec![candidate.clone()],
        confidence: candidate.final_confidence.max(RESOLVED_THRESHOLD),
        requires_approval: step.requires_approval,
        can_proceed_to_safety_gate: true,
        can_execute: false,
        blockers: Vec::new(),
        ambiguity_reasons: Vec::new(),
        source_evidence: candidate.sources,
    }
}

fn resolved_from_prior(
    step: &GuiTypedPlanStep,
    target: GuiResolvedTarget,
) -> GuiTargetResolutionResult {
    GuiTargetResolutionResult {
        step_id: sanitize_target_text(&step.step_id),
        step_type: sanitize_target_text(&step.step_type),
        target_query: target.label.clone(),
        target_kind: target.target_kind.clone(),
        status: "resolved".into(),
        resolved_target: Some(target),
        candidates: Vec::new(),
        confidence: 0.9,
        requires_approval: step.requires_approval,
        can_proceed_to_safety_gate: true,
        can_execute: false,
        blockers: Vec::new(),
        ambiguity_reasons: Vec::new(),
        source_evidence: vec!["prior FocusField target resolution".into()],
    }
}

fn blocked_result(
    step: &GuiTypedPlanStep,
    target_kind: &str,
    status: &str,
    reason: impl Into<String>,
) -> GuiTargetResolutionResult {
    GuiTargetResolutionResult {
        step_id: sanitize_target_text(&step.step_id),
        step_type: sanitize_target_text(&step.step_type),
        target_query: target_query_for_step(step, None),
        target_kind: target_kind.into(),
        status: status.into(),
        resolved_target: None,
        candidates: Vec::new(),
        confidence: 0.0,
        requires_approval: step.requires_approval,
        can_proceed_to_safety_gate: false,
        can_execute: false,
        blockers: vec![sanitize_reason(reason)],
        ambiguity_reasons: if status == "needs_clarification" {
            vec!["missing_or_ambiguous_target".into()]
        } else {
            Vec::new()
        },
        source_evidence: Vec::new(),
    }
}

fn deferred_result(
    step: &GuiTypedPlanStep,
    target_kind: &str,
    reason: impl Into<String>,
) -> GuiTargetResolutionResult {
    GuiTargetResolutionResult {
        step_id: sanitize_target_text(&step.step_id),
        step_type: sanitize_target_text(&step.step_type),
        target_query: target_query_for_step(step, None),
        target_kind: target_kind.into(),
        status: "deferred".into(),
        resolved_target: None,
        candidates: Vec::new(),
        confidence: 0.72,
        requires_approval: step.requires_approval,
        can_proceed_to_safety_gate: true,
        can_execute: false,
        blockers: Vec::new(),
        ambiguity_reasons: Vec::new(),
        source_evidence: vec![sanitize_reason(reason)],
    }
}

fn aggregate_status(results: &[GuiTargetResolutionResult]) -> String {
    for status in ["rejected", "blocked", "ambiguous", "needs_clarification"] {
        if results.iter().any(|result| result.status == status) {
            return status.into();
        }
    }
    if results.iter().any(|result| result.status == "resolved") {
        "resolved".into()
    } else {
        "skipped".into()
    }
}

fn should_defer_until_planned_app(
    step: &GuiTypedPlanStep,
    context: &GuiContext,
    has_prior_app_prerequisite: bool,
) -> bool {
    if !has_prior_app_prerequisite {
        return false;
    }
    if !matches!(
        step.step_type.as_str(),
        "FocusField" | "TypeText" | "ClickControl" | "PressKey"
    ) {
        return false;
    }
    let Some(target_app_hint) = step.target_app_hint.as_deref() else {
        return false;
    };
    let target_app_hint = target_app_hint.trim();
    if target_app_hint.is_empty() {
        return false;
    }
    if active_context_matches_app_hint(context, target_app_hint) {
        return false;
    }
    true
}

fn active_context_matches_app_hint(context: &GuiContext, app_hint: &str) -> bool {
    let hint = app_hint.to_lowercase();
    let active_app = context
        .active_window
        .app_name
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    let active_label = context.active_window.label.to_lowercase();
    if hint == "browser" {
        return ["chrome", "chromium", "firefox", "brave", "browser"]
            .iter()
            .any(|needle| active_app.contains(needle) || active_label.contains(needle));
    }
    active_app.contains(&hint) || active_label.contains(&hint)
}

fn target_kind_for_deferred_step(step: &GuiTypedPlanStep) -> &'static str {
    match step.step_type.as_str() {
        "FocusField" | "TypeText" => "text_field",
        "ClickControl" => target_kind_for_click(step),
        "PressKey" => "focused_context",
        _ => "unknown",
    }
}

fn ambiguity_reasons_for(candidates: &[GuiTargetCandidate], context: &GuiContext) -> Vec<String> {
    let mut reasons = Vec::new();
    if candidates.len() >= 2
        && (candidates[0].final_confidence - candidates[1].final_confidence).abs() <= AMBIGUITY_GAP
    {
        reasons.push("top_candidates_within_confidence_gap".into());
    }

    let top = &candidates[0];
    let duplicate_count = candidates
        .iter()
        .filter(|candidate| {
            normalized_label(&candidate.label) == normalized_label(&top.label)
                && role_group(&candidate.role) == role_group(&top.role)
                && candidate.final_confidence >= CLARIFY_THRESHOLD
        })
        .count();
    if duplicate_count > 1 && !candidates.iter().any(|candidate| candidate.focused) {
        reasons.push("same_label_same_role_multiple_targets".into());
    }

    if active_window_unknown(context) && duplicate_count > 1 {
        reasons.push("active_window_unknown_with_duplicate_targets".into());
    }
    reasons
}

fn label_match_confidence(query: &str, label: &str, step: &GuiTypedPlanStep) -> f64 {
    let normalized_query = normalized_target_query(query, step);
    let normalized_label = normalized_label(label);
    if normalized_query.is_empty() || normalized_label.is_empty() {
        return 0.0;
    }
    if matches!(step.step_type.as_str(), "FocusField" | "TypeText")
        && generic_editable_query(&normalized_query)
    {
        return 0.72;
    }
    if normalized_query == normalized_label {
        return 1.0;
    }
    let query_tokens = normalized_query.split_whitespace().collect::<Vec<_>>();
    let label_tokens = normalized_label.split_whitespace().collect::<Vec<_>>();
    if query_tokens == label_tokens {
        return 0.95;
    }
    if query_tokens
        .iter()
        .all(|token| label_tokens.iter().any(|candidate| candidate == token))
    {
        return 0.78;
    }
    if normalized_label.contains(&normalized_query) || normalized_query.contains(&normalized_label)
    {
        return 0.66;
    }
    0.0
}

fn generic_editable_query(value: &str) -> bool {
    let tokens = value.split_whitespace().collect::<Vec<_>>();
    !tokens.is_empty()
        && tokens.iter().all(|token| {
            matches!(
                *token,
                "visible" | "text" | "input" | "field" | "box" | "editable"
            )
        })
}

fn target_query_for_step(step: &GuiTypedPlanStep, fallback: Option<&str>) -> String {
    step.target_control_hint
        .as_deref()
        .or(step.target_window_hint.as_deref())
        .or(step.target_app_hint.as_deref())
        .or(fallback)
        .map(sanitize_target_text)
        .unwrap_or_default()
}

fn target_kind_for_click(step: &GuiTypedPlanStep) -> &'static str {
    let query = step
        .target_control_hint
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    if query.contains("checkbox") || query.contains("toggle") || query.contains("switch") {
        "checkbox"
    } else if query.contains("tab") {
        "tab"
    } else if query.contains("link") {
        "link"
    } else if query.contains("menu") || query.contains("dropdown") {
        "menu"
    } else {
        "button"
    }
}

fn expected_role_groups(step: &GuiTypedPlanStep, target_kind: &str) -> Vec<&'static str> {
    match step.step_type.as_str() {
        "FocusField" | "TypeText" => vec!["editable"],
        "ClickControl" => match target_kind {
            "checkbox" => vec!["selectable"],
            "tab" => vec!["tab_like"],
            "link" => vec!["navigation_link"],
            "menu" => vec!["menu_like"],
            _ => vec![
                "button_like",
                "selectable",
                "navigation_link",
                "tab_like",
                "menu_like",
            ],
        },
        _ => vec!["app_window"],
    }
}

fn role_group(role: &str) -> &'static str {
    let role = role.to_lowercase();
    if [
        "searchbox",
        "textbox",
        "text",
        "entry",
        "input",
        "editor",
        "textarea",
    ]
    .iter()
    .any(|needle| role.contains(needle))
    {
        "editable"
    } else if ["button", "push button", "submit", "image button"]
        .iter()
        .any(|needle| role.contains(needle))
    {
        "button_like"
    } else if ["checkbox", "radio", "switch", "toggle"]
        .iter()
        .any(|needle| role.contains(needle))
    {
        "selectable"
    } else if ["link", "hyperlink"]
        .iter()
        .any(|needle| role.contains(needle))
    {
        "navigation_link"
    } else if role.contains("tab") {
        "tab_like"
    } else if ["menu", "combo", "dropdown"]
        .iter()
        .any(|needle| role.contains(needle))
    {
        "menu_like"
    } else if ["application", "window", "dialog"]
        .iter()
        .any(|needle| role.contains(needle))
    {
        "app_window"
    } else {
        "unknown"
    }
}

fn generic_target_without_label(query: &str, step: &GuiTypedPlanStep) -> bool {
    let normalized = normalized_target_query(query, step);
    matches!(
        normalized.as_str(),
        "" | "button" | "field" | "input" | "box" | "control" | "text field"
    )
}

fn normalized_target_query(value: &str, step: &GuiTypedPlanStep) -> String {
    let role_explicit = step.target_control_hint.as_deref().is_some_and(|hint| {
        let hint = hint.to_lowercase();
        [
            "button", "field", "input", "box", "checkbox", "tab", "link", "menu",
        ]
        .iter()
        .any(|needle| hint.contains(needle))
    });
    normalize_label(value, role_explicit)
}

fn normalized_label(value: &str) -> String {
    normalize_label(value, false)
}

fn normalize_label(value: &str, strip_generic_suffixes: bool) -> String {
    let mut normalized = value
        .to_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if strip_generic_suffixes {
        let tokens = normalized
            .split_whitespace()
            .filter(|token| !matches!(*token, "button" | "field" | "input" | "box" | "control"))
            .collect::<Vec<_>>();
        normalized = tokens.join(" ");
    }
    normalized
}

fn candidate_app_window_matches(
    step: &GuiTypedPlanStep,
    candidate: &GuiTargetCandidate,
    context: &GuiContext,
) -> bool {
    let expected = step
        .target_app_hint
        .as_deref()
        .or(step.target_window_hint.as_deref())
        .map(|value| value.to_lowercase());
    if let Some(expected) = expected {
        return candidate
            .app_hint
            .as_deref()
            .is_some_and(|value| value.to_lowercase().contains(&expected))
            || candidate
                .window_hint
                .as_deref()
                .is_some_and(|value| value.to_lowercase().contains(&expected));
    }
    candidate
        .window_hint
        .as_deref()
        .is_some_and(|value| value == context.active_window.label)
        || candidate.app_window_confidence >= 0.75
}

fn active_window_unknown(context: &GuiContext) -> bool {
    context.active_window.reliability == "unavailable"
        || context.active_window.authority_status == "unavailable"
        || context.active_window.confidence < 0.40
        || context
            .active_window
            .label
            .trim()
            .eq_ignore_ascii_case("unknown")
}

fn source_only(candidate: &GuiTargetCandidate, needle: &str) -> bool {
    let authority_sources = candidate
        .sources
        .iter()
        .filter(|source| !matches!(source.as_str(), "active_window" | "focus_authority"))
        .collect::<Vec<_>>();
    !authority_sources.is_empty()
        && authority_sources
            .iter()
            .all(|source| source.to_lowercase().contains(needle))
}

fn step_should_emit_resolution(step: &GuiTypedPlanStep) -> bool {
    matches!(
        step.step_type.as_str(),
        "OpenApp"
            | "SwitchWindow"
            | "FocusField"
            | "TypeText"
            | "ClickControl"
            | "PressKey"
            | "RequireApproval"
            | "AskClarification"
            | "WaitForState"
            | "VerifyState"
            | "SummarizeVisibleContent"
    )
}

fn is_action_target_step(step_type: &str) -> bool {
    matches!(
        step_type,
        "OpenApp" | "SwitchWindow" | "FocusField" | "TypeText" | "ClickControl" | "PressKey"
    )
}

fn is_control_step(step_type: &str) -> bool {
    matches!(step_type, "FocusField" | "TypeText" | "ClickControl")
}

fn is_control_resolved_target(target: &GuiResolvedTarget) -> bool {
    matches!(
        target.target_kind.as_str(),
        "control" | "text_field" | "button" | "checkbox" | "tab" | "menu" | "link"
    )
}

fn contains_raw_coordinate(value: &str) -> bool {
    let lower = value.to_lowercase();
    lower.contains("at coordinate")
        || lower.contains("screen position")
        || lower.contains("mouse move")
        || lower.contains("absolute pixel")
        || lower.contains("x=") && lower.contains("y=")
        || lower.split_whitespace().any(|token| {
            token
                .split_once(',')
                .is_some_and(|(x, y)| x.parse::<i32>().is_ok() && y.parse::<i32>().is_ok())
        })
}

fn sanitize_target_text(value: &str) -> String {
    sanitize_gui_text(value, TARGET_TEXT_LIMIT).text
}

fn sanitize_reason(value: impl Into<String>) -> String {
    sanitize_gui_text(&value.into(), 180).text
}
