use chrono::{Datelike, Duration, Local, SecondsFormat, TimeZone, Utc};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::agent::response_parser::{
    extract_text_response, parse_tool_calls_with_known, ParsedToolCall,
};
use crate::agent::result_synthesizer::{ResultSynthesizer, VerificationOutcome};
use crate::agent::turn_context::{TurnAdmission, TurnAdmissionDecision, TurnAdmissionError};
use crate::agent::turn_gate::{Operation, ResourcePlan, TurnGate};
use crate::agent::turn_memory::{detect_satisfaction, ExecutionTarget, TurnMemory};
use crate::infra::isolation::run_isolated;
use crate::infra::pipeline_trace::{
    log_pipeline_step, sanitize_json_for_logs, sanitize_text_for_logs,
};
use crate::llm::budget::{
    check_inter_tool_budget, check_tool_result_budget, BudgetCheckResult, ContextBudgets,
    TurnTokenLedger,
};
use crate::llm::orchestrator::vision_strategy::VisionMode;
use crate::llm::orchestrator::vram_budget::{calculate_safe_visual_tokens, estimate_visual_tokens};
use crate::llm::tokenize::count_tokens;
use crate::llm::{
    ChatMessage, ImageAttachment, LlmResponse, ModelRouter, ToolSchema,
    LLM_TOOL_RESULT_TOKEN_BUDGET, TOOL_RESULT_MAX_CHARS,
};
use crate::mcp::payload_shaper::shape_for_llm;
use crate::safety::audit::{DecidedBy, Decision};
use crate::safety::hitl::{ApprovalResponse, HitlGateway};
use crate::safety::{AuditLogger, PolicyEngine, RiskLevel, RollbackManager};
use crate::tools::mount_manager::{google_meet_fallback_metadata, ToolMountManager};
use crate::tools::registry::{ToolDef, ToolRegistry};

mod helpers;
mod injection_gate;
mod intent_extractors;
mod intent_fallback;
mod response_helpers;

use helpers::*;
use intent_extractors::*;
use intent_fallback::*;
use response_helpers::*;

#[cfg(test)]
thread_local! {
    static TEST_N8N_WORKFLOWS_FOR_DISPATCH: std::cell::RefCell<Option<Vec<crate::n8n::N8nWorkflowConfig>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
struct TestN8nWorkflowDispatchGuard;

#[cfg(test)]
impl Drop for TestN8nWorkflowDispatchGuard {
    fn drop(&mut self) {
        TEST_N8N_WORKFLOWS_FOR_DISPATCH.with(|slot| {
            *slot.borrow_mut() = None;
        });
    }
}

#[cfg(test)]
fn set_test_n8n_workflows_for_dispatch(
    workflows: Vec<crate::n8n::N8nWorkflowConfig>,
) -> TestN8nWorkflowDispatchGuard {
    TEST_N8N_WORKFLOWS_FOR_DISPATCH.with(|slot| {
        *slot.borrow_mut() = Some(workflows);
    });
    TestN8nWorkflowDispatchGuard
}

fn load_n8n_workflows_for_dispatch() -> Vec<crate::n8n::N8nWorkflowConfig> {
    #[cfg(test)]
    if let Some(workflows) = TEST_N8N_WORKFLOWS_FOR_DISPATCH.with(|slot| slot.borrow().clone()) {
        return workflows;
    }

    if let Ok(store) = crate::n8n::load_workflow_registry_store_at(
        &crate::n8n::default_workflow_registry_store_path(),
    ) {
        let workflows = crate::n8n::workflow_registry_workflows(&store);
        if !workflows.is_empty() {
            return workflows;
        }
    }

    crate::config::KriaConfig::load(None)
        .map(|config| config.n8n.workflows)
        .unwrap_or_default()
}

const KRIA_DETERMINISTIC_NOTICE_TOOL: &str = "__kria_deterministic_notice";

fn deterministic_notice_tool(message: String) -> Option<(String, serde_json::Value)> {
    Some((
        KRIA_DETERMINISTIC_NOTICE_TOOL.to_string(),
        serde_json::json!({
            "message": message,
        }),
    ))
}

fn deterministic_notice_message(params: &serde_json::Value) -> String {
    params
        .get("message")
        .and_then(|value| value.as_str())
        .unwrap_or("The request was handled locally.")
        .to_string()
}

fn is_n8n_workflow_list_query(user_text: &str) -> bool {
    crate::n8n::is_n8n_workflow_inventory_query(user_text)
}

fn n8n_workflow_list_notice() -> String {
    let workflows = load_n8n_workflows_for_dispatch();
    crate::n8n::n8n_workflow_inventory_notice(&workflows)
}

fn n8n_match_summary(matches: &[crate::n8n::N8nWorkflowMatchCandidate]) -> String {
    if matches.is_empty() {
        return "No n8n workflows are currently registered in KRIA.".to_string();
    }

    matches
        .iter()
        .map(|workflow| {
            format!(
                "{} ({}, status={})",
                workflow.display_name, workflow.workflow_id, workflow.status
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn n8n_route_notice(route: &crate::n8n::N8nChatRouteDecision) -> String {
    match route.status {
        crate::n8n::N8nChatRouteStatus::ListWorkflows => {
            return n8n_workflow_list_notice();
        }
        crate::n8n::N8nChatRouteStatus::UseOtherTool => {
            return route.message.clone();
        }
        _ => {}
    }

    if route.candidates.is_empty() {
        return route.message.clone();
    }

    let candidates = route
        .candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let mut line = format!(
                "{}. {} ({}) — {} confidence, risk {}",
                index + 1,
                candidate.display_name,
                candidate.workflow_id,
                candidate.confidence_label,
                candidate.risk_tier
            );
            if !candidate.missing_inputs.is_empty() {
                line.push_str(&format!(
                    ", missing input: {}",
                    candidate.missing_inputs.join(", ")
                ));
            }
            if !candidate.blockers.is_empty() {
                line.push_str(&format!(", blocked: {}", candidate.blockers.join("; ")));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("; ");
    let actions = if route.next_actions.is_empty() {
        "Confirm with: Confirm workflow <workflow_id>".to_string()
    } else {
        format!("Next: {}", route.next_actions.join(" | "))
    };
    format!("{} {candidates}. {actions}.", route.message)
}

fn is_direct_manual_n8n_profile(execution_profile: &TurnExecutionProfile) -> bool {
    execution_profile.is_manual_tool_override()
        && execution_profile
            .tool_lock
            .as_deref()
            .is_some_and(|tool| tool == "n8n_invoke_workflow")
}

fn workflow_name_mentioned_in_prompt(
    workflow: &crate::n8n::N8nWorkflowConfig,
    prompt: &str,
) -> bool {
    let prompt = prompt.to_ascii_lowercase();
    let mut keys = vec![
        workflow.workflow_id.to_ascii_lowercase(),
        workflow.display_name.to_ascii_lowercase(),
    ];
    keys.extend(
        workflow
            .aliases
            .iter()
            .map(|alias| alias.to_ascii_lowercase()),
    );

    keys.into_iter()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty() && key.len() >= 3)
        .any(|key| prompt.contains(&key))
}

fn resolve_manual_n8n_workflow<'a>(
    workflows: &'a [crate::n8n::N8nWorkflowConfig],
    user_text: &str,
) -> Option<&'a crate::n8n::N8nWorkflowConfig> {
    let mut references = Vec::new();
    if let Some(reference) =
        crate::n8n::WorkflowConfirmationFlow::parse_confirmation_reference(user_text)
    {
        references.push(reference);
    }
    if let Some(reference) = crate::n8n::parse_n8n_workflow_run_reference(user_text) {
        references.push(reference);
    }
    references.push(user_text.trim().to_string());

    for reference in references {
        if reference.trim().is_empty() {
            continue;
        }
        if let crate::n8n::N8nWorkflowReferenceMatch::Unique { workflow, .. } =
            crate::n8n::resolve_n8n_workflow_reference(workflows, &reference)
        {
            return Some(workflow);
        }
    }

    let mentioned = workflows
        .iter()
        .filter(|workflow| workflow_name_mentioned_in_prompt(workflow, user_text))
        .collect::<Vec<_>>();
    if mentioned.len() == 1 {
        return mentioned.first().copied();
    }

    let response = crate::n8n::WorkflowRankingEngine::new(workflows.to_vec()).suggest(user_text);
    if response.candidates.len() == 1
        && !response.ambiguous
        && !response.hard_prompt
        && response.candidates[0].confidence >= 0.70
    {
        if let crate::n8n::N8nWorkflowReferenceMatch::Unique { workflow, .. } =
            crate::n8n::resolve_n8n_workflow_reference(
                workflows,
                &response.candidates[0].workflow_id,
            )
        {
            return Some(workflow);
        }
    }

    None
}

fn try_manual_n8n_direct_dispatch(
    user_text: &str,
    previous_user_text: Option<&str>,
    execution_profile: &TurnExecutionProfile,
) -> Option<(String, serde_json::Value)> {
    if !is_direct_manual_n8n_profile(execution_profile) {
        return None;
    }

    let workflows = load_n8n_workflows_for_dispatch();
    let workflow = resolve_manual_n8n_workflow(&workflows, user_text)?;

    if crate::n8n::WorkflowConfirmationFlow::workflow_requires_confirmation(workflow)
        && crate::n8n::WorkflowConfirmationFlow::parse_confirmation_reference(user_text).is_none()
    {
        return deterministic_notice_tool(format!(
            "Workflow \"{}\" needs an explicit review confirmation before execution. Confirm with: Confirm workflow {}.",
            workflow.display_name, workflow.workflow_id
        ));
    }

    let prompt_context = if crate::n8n::WorkflowConfirmationFlow::parse_confirmation_reference(
        user_text,
    )
    .is_some()
    {
        previous_user_text
            .filter(|text| !text.trim().is_empty())
            .unwrap_or(user_text)
    } else {
        user_text
    };
    let input_payload =
        crate::n8n::build_n8n_suggested_input_payload(workflow, prompt_context, true);

    tracing::info!(
        target: "n8n_routing",
        routing = "manual_direct_n8n",
        workflow_id = %workflow.workflow_id,
        workflow_version = %workflow.workflow_version,
        prompt_preview = %sanitize_text_for_logs(user_text, 180),
        "Manual n8n mode resolved workflow without LLM routing"
    );

    Some((
        "n8n_invoke_workflow".to_string(),
        serde_json::json!({
            "workflow_id": &workflow.workflow_id,
            "workflow_version": &workflow.workflow_version,
            "input_payload": input_payload,
        }),
    ))
}

fn try_deterministic_dispatch_with_profile(
    user_text: &str,
    previous_user_text: Option<&str>,
    execution_profile: &TurnExecutionProfile,
) -> Option<(String, serde_json::Value)> {
    try_manual_n8n_direct_dispatch(user_text, previous_user_text, execution_profile)
        .or_else(|| try_deterministic_dispatch_with_context(user_text, previous_user_text))
}

/// Try to deterministically extract a tool name + parameters from user text WITHOUT
/// calling the LLM. Returns Some((tool_name, params)) when the prompt clearly maps
/// to a specific tool with extractable parameters.
///
/// This is the deterministic dispatch fast-path — critical for LLM-independence
/// on simple operations like mkdir, whoami, list files, etc.
///
/// Unlike the previous version, this scans the prompt directly and selects the
/// best tool, ignoring whatever tool_hint the router suggested. This is more
/// robust because router classification can be wrong while patterns in the
/// prompt are unambiguous.
fn try_deterministic_dispatch(user_text: &str) -> Option<(String, serde_json::Value)> {
    try_deterministic_dispatch_with_context(user_text, None)
}

// NOTE: The deterministic `config_prompt_control_enabled` / `try_config_prompt_dispatch`
// / `build_turn_override` deciders were REMOVED (settings-nl-control Task 16 / Wave 5
// F15). Settings intent is now classified in exactly ONE place — the unified
// `config::nl` pipeline driven by `run_settings_stage` below — so there is no second
// decider competing with (or bypassing the HITL gate of) the shared handler.

// ─── settings-nl-control Wave 3: first-stage NL settings gate ────────────────

/// True when the unified NL settings pipeline is enabled. Delegates to the single
/// source of truth (`config::nl::nl_settings_enabled`) — default ON, opt out with
/// `KRIA_NL_SETTINGS=0`.
fn nl_settings_enabled() -> bool {
    crate::config::nl::nl_settings_enabled()
}

/// FastEmbed-backed embedder seam for the settings evidence model (Wave 2).
/// Returns `None` when the embedding model isn't loaded → the classifier uses its
/// lexical tier (graceful degradation).
struct RoutingTextEmbedder;
impl crate::config::nl::TextEmbedder for RoutingTextEmbedder {
    fn embed(&self, text: &str) -> Option<Vec<f32>> {
        crate::routing::embed::embed_batch(&[text])
            .ok()
            .and_then(|mut v| v.pop())
    }
}

/// Evidence dependencies for the chat settings stage: the FastEmbed embedder for
/// semantic conversation-topic evidence. Memory evidence is a future seam.
fn settings_evidence_deps() -> crate::config::nl::EvidenceDeps {
    crate::config::nl::EvidenceDeps::default()
        .with_embedder(std::sync::Arc::new(RoutingTextEmbedder))
}

/// Per-process conversational provider-configuration sessions (Wave 4). Keyed by
/// chat session id; TTL-expiring; isolated across sessions.
static SETTINGS_FLOW_STORE: once_cell::sync::Lazy<crate::config::nl::FlowStore> =
    once_cell::sync::Lazy::new(crate::config::nl::FlowStore::new);

/// Cached, schema-derived entity index (built once — Req 12.1).
static SETTINGS_ENTITY_INDEX: once_cell::sync::Lazy<
    std::sync::Arc<crate::config::nl::SchemaEntityIndex>,
> = once_cell::sync::Lazy::new(|| {
    std::sync::Arc::new(crate::config::nl::SchemaEntityIndex::build())
});

/// Build the classifier's ConversationContext from the recent message history
/// (the real per-turn state — NEW-12). Last few user + assistant texts.
fn build_settings_conversation_context(
    messages: &[ChatMessage],
) -> crate::config::nl::ConversationContext {
    let mut recent_user = Vec::new();
    let mut recent_assistant = Vec::new();
    for m in messages.iter().rev() {
        match m.role.as_str() {
            "user" if recent_user.len() < 4 => recent_user.push(m.content.clone()),
            "assistant" if recent_assistant.len() < 4 => recent_assistant.push(m.content.clone()),
            _ => {}
        }
        if recent_user.len() >= 4 && recent_assistant.len() >= 4 {
            break;
        }
    }
    recent_user.reverse();
    recent_assistant.reverse();
    crate::config::nl::ConversationContext::new(recent_user, recent_assistant)
}

/// Chat-surface approval driver: emits `StreamEvent::ApprovalRequired` and blocks
/// on the loop's `HitlGateway` — the SAME gate every RED tool uses (Req 4.4).
struct ChatSettingsApprovalDriver {
    hitl: Arc<HitlGateway>,
    event_tx: mpsc::UnboundedSender<StreamEvent>,
}

#[async_trait::async_trait]
impl crate::config::nl::ApprovalDriver for ChatSettingsApprovalDriver {
    async fn request(
        &self,
        section: &str,
        field: &str,
        value: &serde_json::Value,
        risk: RiskLevel,
    ) -> crate::config::nl::ApprovalDecision {
        use crate::config::nl::ApprovalDecision;
        let request_id = HitlGateway::generate_request_id();
        let description = format!("Change {section}.{field} to {value}");
        let args = serde_json::json!({ "section": section, "field": field, "value": value });
        let _ = self.event_tx.send(StreamEvent::ApprovalRequired {
            request_id: request_id.clone(),
            action: "config_patch".to_string(),
            risk_level: risk.as_str().to_string(),
            parameters: args.clone(),
        });
        match self
            .hitl
            .request_approval_with_id(&request_id, "config_patch", args, risk, &description, false)
            .await
        {
            crate::safety::hitl::ApprovalResponse::Approved => ApprovalDecision::Approved,
            crate::safety::hitl::ApprovalResponse::Denied => ApprovalDecision::Denied,
            _ => ApprovalDecision::Timeout,
        }
    }
}

/// Result of the first-stage settings gate.
enum SettingsStageResult {
    /// The turn was fully handled as a settings operation; the loop returns.
    Claimed,
    /// A settings clause was handled; continue the turn with this remaining text.
    ContinueWith(String),
    /// Not a settings turn; proceed with the normal pipeline unchanged.
    Pass,
}

/// Render a `SettingsOutcome` to the chat stream. `finish` emits `Done` (closes the
/// turn); when false only a `Token` note is emitted (multi-intent — turn continues).
fn render_settings_outcome(
    outcome: &crate::config::nl::SettingsOutcome,
    event_tx: &mpsc::UnboundedSender<StreamEvent>,
    finish: bool,
) {
    use crate::config::nl::SettingsOutcome;
    let msg = match outcome {
        SettingsOutcome::Applied { message, .. } => message.clone(),
        SettingsOutcome::Answer { text } => text.clone(),
        SettingsOutcome::Clarify { question } => question.clone(),
        SettingsOutcome::Refused { reason } => reason.clone(),
        SettingsOutcome::TempApplied { section, field, .. } => {
            format!("Applied {section}.{field} for this request only.")
        }
        SettingsOutcome::Undone { section, field } => {
            format!("Reverted {section}.{field} to its previous value.")
        }
        SettingsOutcome::NothingToUndo => "There's no recent settings change to undo.".to_string(),
        SettingsOutcome::NeedsApproval { section, field, .. } => {
            // resolve() already drove approval; this arm is unexpected but safe.
            format!("Awaiting approval for {section}.{field}.")
        }
    };
    let _ = event_tx.send(StreamEvent::Token(msg.clone()));
    if finish {
        let _ = event_tx.send(StreamEvent::Done(msg));
    }
}

/// Detect a "[settings clause] and/then [task]" multi-intent prompt. Returns the
/// trailing task text when it is a NON-settings remainder (so the settings clause
/// is applied and the turn continues for the task — Wave 5 F6). `None` ⇒ full-claim.
fn settings_multi_intent_remainder(
    text: &str,
    conv: &crate::config::nl::ConversationContext,
    pipeline: &crate::config::nl::SettingsIntentPipeline,
) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let sep = [" and then ", " then ", " and "]
        .iter()
        .filter_map(|s| lower.find(s).map(|i| (i, s.len())))
        .min_by_key(|(i, _)| *i)?;
    let tail = text[sep.0 + sep.1..].trim().to_string();
    if tail.split_whitespace().count() < 2 {
        return None;
    }
    // Only continue when the remainder is a genuine task, not another setting.
    match pipeline.classify(&tail, conv) {
        crate::config::nl::SettingsDecision::NotSettings => Some(tail),
        _ => None,
    }
}

impl AgentLoop {
    /// First-stage NL settings gate (settings-nl-control Wave 3). Runs the shared
    /// `SettingsIntentPipeline`; on a settings decision, executes the shared
    /// `SettingsHandler` through the real HITL gate and renders the outcome. This
    /// is the ONE path chat uses (the `config_prompt` command uses the same handler).
    async fn run_settings_stage(
        &self,
        session_id: &str,
        last_user_text: &str,
        messages: &[ChatMessage],
        event_tx: &mpsc::UnboundedSender<StreamEvent>,
    ) -> SettingsStageResult {
        use crate::config::nl::{
            SettingsDecision, SettingsHandler, SettingsIntentPipeline, SettingsRequest,
            SettingsRequestKind,
        };
        use crate::config::prompt::Scope;

        let Some(config_service) = self.tool_registry.config_service() else {
            return SettingsStageResult::Pass;
        };

        // ── Wave 4: conversational provider configuration (multi-turn) ───────
        // If a config session is active for this chat, or the message starts one,
        // the flow engine owns the turn (ask/confirm/correct/cancel/commit).
        {
            use crate::config::nl::{FlowEngine, FlowOutcome};
            if SETTINGS_FLOW_STORE.active(session_id).is_some()
                || FlowEngine::detects_start(last_user_text)
            {
                match FlowEngine::step(&SETTINGS_FLOW_STORE, session_id, last_user_text) {
                    FlowOutcome::NotAFlow => {}
                    FlowOutcome::Ask { message }
                    | FlowOutcome::Confirm { summary: message }
                    | FlowOutcome::Invalid { message }
                    | FlowOutcome::Cancelled { message } => {
                        let _ = event_tx.send(StreamEvent::Token(message.clone()));
                        let _ = event_tx.send(StreamEvent::Done(message));
                        return SettingsStageResult::Claimed;
                    }
                    FlowOutcome::Commit { draft, .. } => {
                        let handler = SettingsHandler::new(config_service.clone())
                            .with_audit(self.audit_logger.clone());
                        let outcome = handler.commit_provider(&draft).await;
                        render_settings_outcome(&outcome, event_tx, true);
                        return SettingsStageResult::Claimed;
                    }
                }
            }
        }

        let conv = build_settings_conversation_context(messages);
        // Evidence-based intent (Wave 2): attach the FastEmbed embedder so semantic
        // conversation-topic evidence can steer an AMBIGUOUS settings-like phrase
        // away from KRIA config when it really continues the discussion. Degrades
        // gracefully (embed returns None) when FastEmbed is unavailable.
        let pipeline = SettingsIntentPipeline::new(SETTINGS_ENTITY_INDEX.clone())
            .with_evidence(settings_evidence_deps());
        let (decision, trace) = pipeline.classify_traced(last_user_text, &conv);
        // Persist the routing decision for production diagnosability (R7.3/L4).
        crate::config::nl::diagnostics::record(session_id, last_user_text, &trace);

        let handler = SettingsHandler::new(config_service).with_audit(self.audit_logger.clone());

        match decision {
            SettingsDecision::NotSettings => SettingsStageResult::Pass,
            SettingsDecision::Clarify { question } => {
                let _ = event_tx.send(StreamEvent::Token(question.clone()));
                let _ = event_tx.send(StreamEvent::Done(question));
                SettingsStageResult::Claimed
            }
            SettingsDecision::Undo => {
                let outcome = handler
                    .handle(SettingsRequest {
                        kind: SettingsRequestKind::Undo,
                        section: String::new(),
                        field: String::new(),
                        value: None,
                        scope: Scope::Permanent,
                        provenance: crate::tools::TriggerProvenance::User,
                        session_id: session_id.to_string(),
                    })
                    .await;
                render_settings_outcome(&outcome, event_tx, true);
                SettingsStageResult::Claimed
            }
            SettingsDecision::ReadBack { section, field } => {
                let outcome = handler
                    .handle(SettingsRequest::read_back(section, field).with_session(session_id))
                    .await;
                render_settings_outcome(&outcome, event_tx, true);
                SettingsStageResult::Claimed
            }
            SettingsDecision::Info(query) => {
                // Answer-from-system (catalog/help/explain/recent) — no LLM, no mutation.
                let outcome = handler.info(&query).await;
                render_settings_outcome(&outcome, event_tx, true);
                SettingsStageResult::Claimed
            }
            SettingsDecision::Change {
                section,
                field,
                value,
                scope,
            } => {
                // Temp override (e.g. "generate this image using local AI"): install a
                // turn-scoped RequestOverride and DO NOT claim — the actual tool runs
                // this turn and reads it via effective_config (Task 10).
                if scope == Scope::Temp {
                    if let Some(v) = value {
                        let mut ov = crate::config::RequestOverride::new();
                        if ov.set(&section, &field, v).is_ok() {
                            self.tool_registry
                                .set_turn_override(std::sync::Arc::new(ov));
                        }
                    }
                    return SettingsStageResult::Pass;
                }

                let multi = settings_multi_intent_remainder(last_user_text, &conv, &pipeline);
                let req = SettingsRequest {
                    kind: SettingsRequestKind::Change,
                    section,
                    field,
                    value,
                    scope: Scope::Permanent,
                    provenance: crate::tools::TriggerProvenance::User,
                    session_id: session_id.to_string(),
                };
                let driver = ChatSettingsApprovalDriver {
                    hitl: self.hitl_gateway.clone(),
                    event_tx: event_tx.clone(),
                };
                let outcome = handler.resolve(req, &driver).await;

                match multi {
                    // Multi-intent: note the settings result, then continue the turn
                    // with the trailing task (Wave 5 F6).
                    Some(remainder) => {
                        render_settings_outcome(&outcome, event_tx, false);
                        SettingsStageResult::ContinueWith(remainder)
                    }
                    None => {
                        render_settings_outcome(&outcome, event_tx, true);
                        SettingsStageResult::Claimed
                    }
                }
            }
        }
    }
}

fn try_deterministic_dispatch_with_context(
    user_text: &str,
    previous_user_text: Option<&str>,
) -> Option<(String, serde_json::Value)> {
    let lower = user_text.to_ascii_lowercase();

    // ── Prompt-driven settings control (settings-config-revamp Task 12/13) ──
    // Deterministically route recognized settings COMMANDS to the `config_patch`
    // tool instead of relying on the LLM to pick it. Gated behind
    // KRIA_CONFIG_PROMPT_CONTROL so default behaviour is byte-for-byte unchanged.
    // Fail-toward-query: only intercept when the analyzer is confident it is an
    // Act with a schema-grounded field AND a concrete value was extractable; a
    // Clarify emits a single question; anything else falls through to normal flow.
    // config_patch enforces its own injection wall + risk gate (GREEN auto-applies;
    // YELLOW/RED return NeedsApproval — risky changes are never silently applied).
    // NOTE: settings intent is no longer deterministically pre-dispatched here — the
    // unified `run_settings_stage` gate (which runs earlier in the turn, before this
    // deterministic router) is the single decider (settings-nl-control Task 16).

    // ── Workflow discovery ("What workflows can I run?") ────────────────────
    // Returns the list of configured n8n workflows.
    // Instead of calling a tool, we build the response directly here since
    // the workflow list is static config data.
    if is_n8n_workflow_list_query(user_text) {
        return deterministic_notice_tool(n8n_workflow_list_notice());
    }

    // ── System info queries (highest specificity) ──────────────────────────
    if lower.contains("what is my")
        || lower.contains("what's my")
        || lower.contains("tell me my")
        || (lower.contains("my") && lower.contains("current"))
    {
        // Check for system info keywords
        let mut info_parts = Vec::new();
        if lower.contains("username") || lower.contains("user name") || lower.contains("whoami") {
            info_parts.push("Username: $(whoami)");
        }
        if lower.contains("hostname") || lower.contains("host name") {
            info_parts.push("Hostname: $(hostname)");
        }
        if lower.contains("kernel") {
            info_parts.push("Kernel: $(uname -r)");
        }
        if lower.contains("os") || lower.contains("operating system") || lower.contains("distro") {
            info_parts.push("OS: $(lsb_release -ds 2>/dev/null || cat /etc/os-release | grep PRETTY_NAME | cut -d'\\\"' -f2)");
        }
        if lower.contains("shell") {
            info_parts.push("Shell: $SHELL");
        }
        if !info_parts.is_empty() {
            let cmd = info_parts
                .iter()
                .map(|p| format!("echo \"{}\"", p))
                .collect::<Vec<_>>()
                .join("; ");
            return Some((
                "execute_bash".to_string(),
                serde_json::json!({
                    "command": cmd,
                    "timeout": 10,
                }),
            ));
        }
    }

    // ── Disk space queries ──────────────────────────────────────────────────
    if lower.contains("disk space")
        || lower.contains("free space")
        || (lower.contains("how much") && lower.contains("disk"))
        || (lower.contains("check") && lower.contains("disk"))
        || (lower.contains("storage")
            && (lower.contains("how much")
                || lower.contains("available")
                || lower.contains("free")))
    {
        let cmd = if lower.contains("root") || lower.contains("/ ") || lower.contains("partition") {
            "df -h /"
        } else {
            "df -h"
        };
        return Some((
            "execute_bash".to_string(),
            serde_json::json!({
                "command": cmd,
                "timeout": 10,
            }),
        ));
    }

    // ── List files in directory (must require explicit list/files keywords) ───
    // Must explicitly mention "files" to avoid false matching on "Show me the file contents"
    if (lower.contains("list all files")
        || lower.contains("list files")
        || lower.contains("show files")
        || lower.contains("show all files")
        || lower.contains("ls /tmp"))
        && lower.contains("/tmp")
    {
        // Match "start with 'X'" / "starting with 'X'" / "starts with 'X'"
        if let Some(cap) =
            regex::Regex::new(r#"(?i)start[s]?(?:ing)?\s+with\s+['"]?([^'"\s]+)['"]?"#)
                .ok()
                .and_then(|re| re.captures(user_text))
        {
            let prefix = cap
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            if !prefix.is_empty() {
                return Some((
                    "execute_bash".to_string(),
                    serde_json::json!({
                        "command": format!("ls -la /tmp/ 2>/dev/null | grep -E '^[^.].*\\b{}' || echo 'No files matching {} found'", prefix, prefix),
                        "timeout": 10,
                    }),
                ));
            }
        }
        // Default: list all files in /tmp
        return Some((
            "execute_bash".to_string(),
            serde_json::json!({
                "command": "ls -la /tmp/",
                "timeout": 10,
            }),
        ));
    }

    // ── n8n workflow invocation ────────────────────────────────────────────
    // Stage 3 first slice: deterministic metadata ranking only. Workflow
    // prompts produce suggestions and require explicit confirmation. The only
    // chat path that invokes n8n is "Confirm workflow <workflow_id>".
    if let Some(reference) =
        crate::n8n::WorkflowConfirmationFlow::parse_confirmation_reference(user_text)
    {
        let workflows = load_n8n_workflows_for_dispatch();
        match crate::n8n::resolve_n8n_workflow_reference(&workflows, &reference) {
            crate::n8n::N8nWorkflowReferenceMatch::Unique { workflow, .. } => {
                let prompt_context = previous_user_text
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or(user_text);
                let input_payload =
                    crate::n8n::build_n8n_suggested_input_payload(workflow, prompt_context, true);
                return Some((
                    "n8n_invoke_workflow".to_string(),
                    serde_json::json!({
                        "workflow_id": &workflow.workflow_id,
                        "workflow_version": &workflow.workflow_version,
                        "input_payload": input_payload,
                    }),
                ));
            }
            crate::n8n::N8nWorkflowReferenceMatch::Ambiguous { matches } => {
                return deterministic_notice_tool(format!(
                    "That confirmation still matches more than one workflow. Confirm with an exact workflow ID: {}.",
                    n8n_match_summary(&matches)
                ));
            }
            crate::n8n::N8nWorkflowReferenceMatch::NoMatch { available } => {
                return deterministic_notice_tool(format!(
                    "Workflow \"{}\" was not found. Confirm with an approved workflow ID. Available workflows: {}.",
                    reference,
                    n8n_match_summary(&available)
                ));
            }
        }
    }

    // BUG #1 FIX (n8n misrouting root cause #2, category D: Dispatcher issue):
    // `parse_n8n_workflow_run_reference` matches the bare "run "/"execute "/"retry "
    // prefix on ANY prompt with no content check at all — unlike the second n8n
    // dispatch block below, this earlier branch was never gated by
    // `prompt_looks_like_non_n8n_tool_intent`. "Run the skill oc_fake_skill..."
    // starts with "run " and was routed straight into n8n workflow resolution,
    // producing a confusing "not_found"/"blocked" n8n response instead of a
    // clean "no such skill" answer from the OpenClaw path. Apply the same
    // intent exclusion here that already guards the fallback n8n block.
    if let Some(reference) = crate::n8n::parse_n8n_workflow_run_reference(user_text) {
        if crate::n8n::prompt_looks_like_non_n8n_tool_intent(user_text) {
            // Not an n8n workflow reference — fall through to normal routing
            // (e.g. OpenClaw skill invocation) instead of resolving against n8n.
        } else {
            let workflows = load_n8n_workflows_for_dispatch();
            let route = crate::n8n::WorkflowRankingEngine::new(workflows).route_chat(
                crate::n8n::N8nChatRouteRequest {
                    prompt: user_text.to_string(),
                    previous_user_prompt: previous_user_text.map(str::to_string),
                    manual_n8n_mode: false,
                    safe_auto_run_enabled: false,
                    workflows: Vec::new(),
                },
            );
            if matches!(route.status, crate::n8n::N8nChatRouteStatus::UseOtherTool) {
                return None;
            }
            let _ = reference;
            return deterministic_notice_tool(n8n_route_notice(&route));
        }
    }

    if !crate::n8n::prompt_looks_like_non_n8n_tool_intent(user_text) {
        let workflows = load_n8n_workflows_for_dispatch();
        if !workflows.is_empty() {
            let route = crate::n8n::WorkflowRankingEngine::new(workflows).route_chat(
                crate::n8n::N8nChatRouteRequest {
                    prompt: user_text.to_string(),
                    previous_user_prompt: previous_user_text.map(str::to_string),
                    manual_n8n_mode: false,
                    safe_auto_run_enabled: false,
                    workflows: Vec::new(),
                },
            );
            if !route.candidates.is_empty()
                && !matches!(route.status, crate::n8n::N8nChatRouteStatus::UseOtherTool)
            {
                return deterministic_notice_tool(n8n_route_notice(&route));
            }
        }
    }

    // ── Browser search ─────────────────────────────────────────────────────
    // "Search for X on Google [using the browser]"
    // "Search Google for X"
    if let Some(cap) = regex::Regex::new(r#"(?i)search\s+(?:for\s+)?['"]?([^'"]+?)['"]?\s+(?:on\s+)?(?:google|youtube|bing|duckduckgo)"#).ok()
        .and_then(|re| re.captures(user_text))
    {
        let query = cap.get(1)?.as_str().trim().to_string();
        if !query.is_empty() && !query.contains("\n") && query.len() < 200 {
            let lower_text = user_text.to_lowercase();
            let site = if lower_text.contains("youtube") { "youtube" }
                else if lower_text.contains("bing") { "bing" }
                else { "google" };
            return Some(("browser_search".to_string(), serde_json::json!({
                "query": query,
                "site": site,
            })));
        }
    }

    // ── Create directory (with optional subfolders + README) ───────────────
    // "Create a project folder called kria-eval-test in /tmp with src, tests, and docs subfolders, and a README.md file"
    if let Some(cap) = regex::Regex::new(r"(?i)\b(?:create|make|mkdir)\s+(?:a\s+)?(?:project\s+|test\s+|new\s+)?(?:folder|directory|dir)\s+(?:called\s+|named\s+)?([a-zA-Z0-9_.\-]+)\s+(?:in|at)\s+(\S+)").ok()
        .and_then(|re| re.captures(user_text))
    {
        let folder_name = cap.get(1)?.as_str().to_string();
        let parent = cap.get(2)?.as_str().trim_end_matches('/').to_string();
        let full_path = format!("{}/{}", parent, folder_name);

        // Check if subfolders are requested
        let mut subfolders: Vec<String> = Vec::new();
        if let Some(sub_cap) = regex::Regex::new(r"(?i)with\s+([a-zA-Z0-9_,\s]+?)\s+(?:subfolders?|sub-folders?|subdirectories|subdirs)").ok()
            .and_then(|re| re.captures(user_text))
        {
            let sub_str = sub_cap.get(1)?.as_str();
            for s in sub_str.split(|c: char| c == ',' || c.is_whitespace()) {
                let s = s.trim().trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-');
                if !s.is_empty() && s.to_lowercase() != "and" {
                    subfolders.push(s.to_string());
                }
            }
        }

        // Build a single bash command that creates everything
        let mut cmd_parts = vec![format!("mkdir -p '{}'", full_path)];
        for sub in &subfolders {
            cmd_parts.push(format!("mkdir -p '{}/{}'", full_path, sub));
        }
        // README.md
        if lower.contains("readme") {
            cmd_parts.push(format!("touch '{}/README.md'", full_path));
            cmd_parts.push(format!("echo '# {}' > '{}/README.md'", folder_name, full_path));
        }
        cmd_parts.push(format!("ls -la '{}'", full_path));

        let cmd = cmd_parts.join(" && ");
        return Some(("execute_bash".to_string(), serde_json::json!({
            "command": cmd,
            "timeout": 15,
        })));
    }

    // ── Simple create_directory: "create folder /path" ─────────────────────
    if let Some(cap) = regex::Regex::new(r"(?i)\b(?:create|make|mkdir)\s+(?:a\s+)?(?:folder|directory|dir)\s+(?:at\s+|in\s+)?(/[a-zA-Z0-9_.\-/]+)").ok()
        .and_then(|re| re.captures(user_text))
    {
        let path = cap.get(1)?.as_str().to_string();
        return Some(("create_directory".to_string(), serde_json::json!({
            "path": path,
            "recursive": true,
        })));
    }

    // ── Create [language] file at path (MUST be checked BEFORE generic file pattern) ──
    // "Create a Rust file at /tmp/greet.rs with a main function that prints 'Greetings from KRIA'"
    // Also handles "Create a Python file at /tmp/X.py that prints 'hello', run it, and show me the output"
    let lang_pattern = regex::Regex::new(r"(?i)\b(?:create|write)\s+(?:a\s+)?(python|rust|js|javascript|typescript|ts|go|c|cpp|c\+\+|bash|shell|ruby|java)\s+(?:file|script)\s+(?:at\s+)?(/[a-zA-Z0-9_./\-]+)").ok();
    if let Some(re) = lang_pattern {
        if let Some(cap) = re.captures(user_text) {
            let language = cap.get(1)?.as_str().to_lowercase();
            let path = cap.get(2)?.as_str().to_string();

            // Detect "run it" / "execute it" / "show output" intent
            let wants_run = lower.contains(" run it")
                || lower.contains("run the")
                || lower.contains("execute it")
                || lower.contains("show me the output")
                || lower.contains("show the output")
                || lower.contains("show output");

            // Extract quoted print/output content
            let print_content = regex::Regex::new(r#"(?i)\bprints?\s+['"]([^'"]+)['"]"#)
                .ok()
                .and_then(|re| re.captures(user_text))
                .and_then(|c| c.get(1).map(|m| m.as_str().to_string()));

            // Detect specific algorithm requests
            let wants_fibonacci = lower.contains("fibonacci");
            let wants_primes = lower.contains("prime number") || lower.contains("primes");

            let content = match (language.as_str(), print_content.as_deref(), wants_fibonacci, wants_primes) {
                ("python", _, true, _) => {
                    // Detect "up to N" / "first N" / "10"
                    let limit = regex::Regex::new(r"\b(?:up to|first|the first)\s+(\d+)\b").ok()
                        .and_then(|re| re.captures(user_text))
                        .and_then(|c| c.get(1)?.as_str().parse::<i64>().ok())
                        .unwrap_or(100);
                    format!(
                        "def fib_up_to(n):\n    a, b = 0, 1\n    result = []\n    while a <= n:\n        result.append(a)\n        a, b = b, a + b\n    return result\n\nprint(fib_up_to({}))\n",
                        limit
                    )
                }
                ("python", _, _, true) => {
                    let limit = regex::Regex::new(r"\b(?:first|the first|up to)\s+(\d+)\b").ok()
                        .and_then(|re| re.captures(user_text))
                        .and_then(|c| c.get(1)?.as_str().parse::<i64>().ok())
                        .unwrap_or(10);
                    format!(
                        "def is_prime(n):\n    if n < 2: return False\n    for i in range(2, int(n**0.5) + 1):\n        if n % i == 0: return False\n    return True\n\nprimes = []\nn = 2\nwhile len(primes) < {}:\n    if is_prime(n):\n        primes.append(n)\n    n += 1\nprint(primes)\n",
                        limit
                    )
                }
                ("rust", Some(text), _, _) => format!("fn main() {{\n    println!(\"{}\");\n}}\n", text),
                ("rust", None, _, _) => "fn main() {\n    println!(\"Hello from KRIA\");\n}\n".to_string(),
                ("python", Some(text), _, _) => format!("print(\"{}\")\n", text),
                ("python", None, _, _) => "print(\"Hello from KRIA\")\n".to_string(),
                ("javascript" | "js", Some(text), _, _) => format!("console.log(\"{}\");\n", text),
                ("javascript" | "js", None, _, _) => "console.log(\"Hello from KRIA\");\n".to_string(),
                ("bash" | "shell", Some(text), _, _) => format!("#!/bin/bash\necho \"{}\"\n", text),
                ("bash" | "shell", None, _, _) => "#!/bin/bash\necho \"Hello from KRIA\"\n".to_string(),
                ("go", Some(text), _, _) => format!("package main\n\nimport \"fmt\"\n\nfunc main() {{\n    fmt.Println(\"{}\")\n}}\n", text),
                ("go", None, _, _) => "package main\n\nimport \"fmt\"\n\nfunc main() {\n    fmt.Println(\"Hello from KRIA\")\n}\n".to_string(),
                _ => return None, // Unknown language — fall back to ReAct
            };

            // If user wants to run + show output, use execute_bash to write+run+capture in one step
            if wants_run {
                // Escape single quotes in content for safe heredoc
                let runner = match language.as_str() {
                    "python" => "python3",
                    "rust" => "rustc",
                    "javascript" | "js" => "node",
                    "bash" | "shell" => "bash",
                    "go" => "go run",
                    _ => return None,
                };
                let cmd = if language == "rust" {
                    // Rust needs compile + run
                    format!(
                        "cat > '{}' << 'KRIA_EOF'\n{}\nKRIA_EOF\nrustc '{}' -o /tmp/_kria_rust_bin && /tmp/_kria_rust_bin",
                        path, content, path
                    )
                } else if language == "bash" || language == "shell" {
                    format!(
                        "cat > '{}' << 'KRIA_EOF'\n{}\nKRIA_EOF\nchmod +x '{}' && bash '{}'",
                        path, content, path, path
                    )
                } else {
                    format!(
                        "cat > '{}' << 'KRIA_EOF'\n{}\nKRIA_EOF\necho '--- Running ---' && {} '{}'",
                        path, content, runner, path
                    )
                };
                return Some((
                    "execute_bash".to_string(),
                    serde_json::json!({
                        "command": cmd,
                        "timeout": 20,
                    }),
                ));
            }

            return Some((
                "write_file".to_string(),
                serde_json::json!({
                    "path": path,
                    "content": content,
                }),
            ));
        }
    }

    // ── Create file with content (deterministic patterns) ───────────────────
    // "Create a file at /tmp/X.txt with three lines: line 1 says 'Task started'..."
    if let Some(cap) = regex::Regex::new(
        r"(?i)\b(?:create|write|save)\s+(?:a\s+)?file\s+(?:at\s+)?(/[a-zA-Z0-9_./\-]+)",
    )
    .ok()
    .and_then(|re| re.captures(user_text))
    {
        let path = cap.get(1)?.as_str().to_string();
        // Try to extract content
        let content = if lower.contains("three lines:") || lower.contains("lines:") {
            let mut lines = Vec::new();
            let re_line = regex::Regex::new(r#"line\s+\d+\s+says?\s+['"]([^'"]+)['"]"#).ok()?;
            for line_cap in re_line.captures_iter(user_text) {
                if let Some(m) = line_cap.get(1) {
                    lines.push(m.as_str().to_string());
                }
            }
            if lines.is_empty() {
                return None; // Can't extract meaningful content
            }
            lines.join("\n")
        } else if let Some(c) =
            regex::Regex::new(r#"(?i)\bwith\s+(?:the\s+)?contents?\s+['"]([^'"]+)['"]"#)
                .ok()
                .and_then(|re| re.captures(user_text))
        {
            c.get(1)?.as_str().to_string()
        } else {
            return None; // No deterministic content extraction
        };
        return Some((
            "write_file".to_string(),
            serde_json::json!({
                "path": path,
                "content": content,
            }),
        ));
    }

    None
}

/// Legacy wrapper for backwards compatibility (currently unused after refactor).
#[allow(dead_code)]
fn try_deterministic_extract(_tool_hint: &str, user_text: &str) -> Option<serde_json::Value> {
    try_deterministic_dispatch(user_text).map(|(_, params)| params)
}

/// Format tool errors into user-friendly messages.
/// Rules:
/// - Never expose internal tool names (n8n_invoke_workflow, execute_bash, etc.)
/// - Extract actionable information from the raw error
/// - For n8n: suggest available workflows when unknown workflow requested
/// PRODUCTION HARDENING FIX (Phase 10: error system audit). Compute the
/// `result` payload for a `StreamEvent::ToolEnd` from a completed
/// `ToolResult`. On success, forwards `data` unchanged (preserves the
/// existing, correct contract for successful tool calls). On failure, folds
/// the real `error` message into the payload as `{"error": "..."}` instead of
/// forwarding `data` (which `ToolResult::err` always sets to `Value::Null`) —
/// otherwise the frontend's raw result display has nothing to show and falls
/// back to a generic "unknown error", hiding the actual failure reason from
/// the user regardless of what really happened.
fn tool_end_result_payload(tool_result: &crate::infra::isolation::ToolResult) -> serde_json::Value {
    if !tool_result.success {
        if let Some(err) = tool_result.error.as_ref() {
            return serde_json::json!({ "error": err });
        }
    }
    tool_result.data.clone()
}

fn format_tool_error_for_user(tool_name: &str, raw_error: &str) -> String {
    let lower = raw_error.to_lowercase();

    // n8n-specific errors
    if tool_name == "n8n_invoke_workflow" {
        if lower.contains("unknown n8n workflow") {
            // Extract workflow name from error
            let wf_name = raw_error
                .split('\'')
                .nth(1)
                .unwrap_or("the requested workflow");
            return format!(
                "⚠️ Workflow '{}' not found.\n\nTo see available workflows, ask: \"What workflows can I run?\"",
                wf_name
            );
        }
        if lower.contains("not approved") {
            return "⚠️ This workflow exists but hasn't been approved yet. Use the Dashboard to approve it.".to_string();
        }
        if lower.contains("connection refused") || lower.contains("connect error") {
            return "⚠️ Cannot reach n8n. Make sure n8n is running (docker start n8n).".to_string();
        }
        if lower.contains("404") || lower.contains("not registered") {
            return "⚠️ n8n webhook not active. Activate the workflow in n8n's editor.".to_string();
        }
        // Generic n8n error
        return format!(
            "⚠️ Workflow failed: {}",
            raw_error.replace("n8n workflow invocation failed: ", "")
        );
    }

    // Shell errors
    if tool_name == "execute_bash" || tool_name == "execute_python" {
        if lower.contains("command not found") {
            return format!("⚠️ Command not found: {}", raw_error);
        }
        if lower.contains("permission denied") {
            return "⚠️ Permission denied. Try with a different path or check file permissions."
                .to_string();
        }
        return format!("⚠️ {}", raw_error);
    }

    // Generic fallback — still don't show tool name
    format!("⚠️ {}", raw_error)
}

/// Format tool results into human-readable output.
/// Extracts meaningful content from structured tool responses instead of
/// dumping raw JSON with exit_code/stdout/stderr fields.
fn format_tool_result_for_user(tool_name: &str, data: &serde_json::Value) -> String {
    // Shell tools (execute_bash, execute_python) — extract stdout
    if tool_name == "execute_bash"
        || tool_name == "execute_python"
        || tool_name == "execute_powershell"
    {
        let stdout = data.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
        let stderr = data.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
        let exit_code = data.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);

        if exit_code != 0 && !stderr.is_empty() {
            return format!("⚠️ Command failed (exit {}):\n{}", exit_code, stderr.trim());
        }

        let output = stdout.trim();
        if output.is_empty() && stderr.trim().is_empty() {
            return "✓ Command completed (no output)".to_string();
        }

        if output.is_empty() {
            return stderr.trim().to_string();
        }

        // Clean output — no wrapper, just the content
        output.to_string()
    }
    // File tools — show path-based confirmation
    else if tool_name == "write_file" || tool_name == "create_directory" {
        let path = data
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| data.get("created").and_then(|v| v.as_str()));
        if let Some(p) = path {
            return format!("✓ Created: {}", p);
        }
        "✓ Done".to_string()
    }
    // Disk/system tools — format as readable summary
    else if tool_name == "get_disk_space" {
        if let Some(disks) = data.get("disks").and_then(|v| v.as_array()) {
            let lines: Vec<String> = disks
                .iter()
                .filter_map(|d| {
                    let mount = d.get("mount")?.as_str()?;
                    let total = d.get("total_gb")?.as_u64()?;
                    let available = d.get("available_gb")?.as_u64()?;
                    let used_pct = if total > 0 {
                        ((total - available) * 100) / total
                    } else {
                        0
                    };
                    Some(format!(
                        "  {} — {} GB free of {} GB ({}% used)",
                        mount, available, total, used_pct
                    ))
                })
                .collect();
            return format!("Disk space:\n{}", lines.join("\n"));
        }
        serde_json::to_string_pretty(data).unwrap_or_default()
    }
    // Default: try to extract meaningful string content
    else {
        if let Some(s) = data.as_str() {
            return s.to_string();
        }
        // For objects, try common result fields
        if let Some(msg) = data.get("message").and_then(|v| v.as_str()) {
            return msg.to_string();
        }
        if let Some(result) = data.get("result").and_then(|v| v.as_str()) {
            return result.to_string();
        }
        // Fallback: compact JSON (but truncated)
        let json_str = serde_json::to_string_pretty(data).unwrap_or_default();
        if json_str.len() > 500 {
            format!("{}...", &json_str[..500])
        } else {
            json_str
        }
    }
}

/// Format n8n workflow invocation result for the user.
/// Rules:
/// - Never show raw JSON webhook ack ({"received":true} is meaningless to user)
/// - Never show internal tool names
/// - Never duplicate the ⏳ indicator (it's already emitted before invoke)
/// - Just confirm workflow was triggered and we're awaiting results
fn format_n8n_result(data: &serde_json::Value) -> String {
    let workflow_id = data
        .get("workflow_id")
        .and_then(|v| v.as_str())
        .unwrap_or("workflow");
    let accepted = data
        .get("accepted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if !accepted {
        return format!(
            "⚠️ Workflow '{}' was not accepted. Check if it's active in n8n.",
            workflow_id
        );
    }

    // Simple clean confirmation — no raw JSON, no tracking ID clutter
    format!(
        "Workflow '{}' triggered successfully. Awaiting results...",
        workflow_id
    )
}

fn strip_notice_prefix(summary: &str) -> &str {
    summary
        .trim()
        .strip_prefix("Proceeding with:")
        .unwrap_or(summary.trim())
        .trim()
}

fn strip_label_prefix<'a>(value: &'a str, label: &str) -> Option<&'a str> {
    let value = value.trim();
    let prefix = value.get(..label.len())?;
    if prefix.eq_ignore_ascii_case(label) {
        Some(value[label.len()..].trim())
    } else {
        None
    }
}

fn format_autonomy_notice_for_user(summary: &str) -> String {
    let clean = strip_notice_prefix(summary);
    if clean.is_empty() {
        return "Starting the requested task.".to_string();
    }

    if let Some(task) = strip_label_prefix(clean, "Coding workflow:") {
        if task.is_empty() {
            return "Starting coding workflow. I will create the code, run it when requested, and report the result.".to_string();
        }
        return format!(
            "Starting coding workflow. I will create the code, run it when requested, and report the final result.\nTask: {}",
            task
        );
    }

    if let Some(task) = strip_label_prefix(clean, "GUI workflow:") {
        if task.is_empty() {
            return "Starting GUI workflow. I will report each major result and any blocker."
                .to_string();
        }
        return format!(
            "Starting GUI workflow. I will report each major result and any blocker.\nTask: {}",
            task
        );
    }

    format!("Starting task: {}", clean)
}

fn contains_token(text: &str, needle: &str) -> bool {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| token == needle)
}

fn should_force_browser_search_for_gui_launch_query(query: &str) -> bool {
    let lower = query.to_ascii_lowercase();

    let looks_like_editor_coding_workflow = (lower.contains("open code")
        || lower.contains("launch code")
        || lower.contains("open vscode")
        || lower.contains("launch vscode")
        || lower.contains("visual studio code"))
        && [
            "write", "program", "script", "function", "code", "run", "execute", "compile", "debug",
            "save",
        ]
        .iter()
        .any(|marker| lower.contains(marker));
    if looks_like_editor_coding_workflow {
        return false;
    }

    lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("www.")
        || lower.contains("search for")
        || lower.contains("search ")
        || lower.contains("google")
        || lower.contains("youtube")
        || lower.contains("website")
        || lower.contains("url")
        || contains_token(&lower, "browser")
        || contains_token(&lower, "chrome")
        || contains_token(&lower, "chromium")
        || contains_token(&lower, "firefox")
        || contains_token(&lower, "brave")
        || contains_token(&lower, "edge")
}

fn gui_action_label(action: &str) -> &'static str {
    match action {
        "write_file" => "Write generated file",
        "execute_bash" => "Run command and capture output",
        "open_application_with_file" => "Open the created file",
        "open_application" => "Open application",
        "browser_search" | "managed_browser_navigate" | "open_url" => "Open browser target",
        "click_element" | "click_ui_element" | "click_mouse" => "Click target",
        "type_text" => "Type requested text",
        "press_shortcut" => "Press shortcut",
        "focus_window" => "Focus target window",
        _ => "Run workflow step",
    }
}

fn format_gui_workflow_start_for_user(
    workflow: &crate::agent::htn_executor::GuiWorkflow,
) -> String {
    let total = workflow.sub_goals.len();
    let has_terminal_run = workflow
        .sub_goals
        .iter()
        .any(|goal| goal.action == "execute_bash");
    let mut lines = Vec::with_capacity(total + 1);
    lines.push(format!(
        "{} {} step{} planned.",
        if has_terminal_run {
            "Starting coding workflow."
        } else {
            "Starting GUI workflow."
        },
        total,
        if total == 1 { "" } else { "s" }
    ));
    for goal in &workflow.sub_goals {
        lines.push(format!(
            "Step {}/{}: {}.",
            goal.step,
            total,
            gui_action_label(&goal.action)
        ));
    }
    lines.join("\n")
}

fn emit_gui_workflow_initial_task_steps(
    event_tx: &mpsc::UnboundedSender<StreamEvent>,
    workflow: &crate::agent::htn_executor::GuiWorkflow,
) {
    let total = workflow.sub_goals.len() as u32;
    for goal in &workflow.sub_goals {
        let status = if goal.step == 1 {
            TaskStepStatus::Running
        } else {
            TaskStepStatus::Starting
        };
        let _ = event_tx.send(StreamEvent::TaskStep(TaskStep {
            index: goal.step as u32,
            total: Some(total),
            description: gui_action_label(&goal.action).to_string(),
            status,
        }));
    }
}

fn emit_gui_workflow_final_task_steps(
    event_tx: &mpsc::UnboundedSender<StreamEvent>,
    workflow: &crate::agent::htn_executor::GuiWorkflow,
    result: &crate::agent::htn_executor::WorkflowResult,
) {
    let total = workflow.sub_goals.len() as u32;
    let failed_step = if result.success {
        None
    } else {
        Some(result.completed_steps.saturating_add(1))
    };
    for goal in &workflow.sub_goals {
        let status = if goal.step <= result.completed_steps {
            TaskStepStatus::Done
        } else if failed_step == Some(goal.step) {
            TaskStepStatus::Failed
        } else {
            TaskStepStatus::Skipped
        };
        let _ = event_tx.send(StreamEvent::TaskStep(TaskStep {
            index: goal.step as u32,
            total: Some(total),
            description: gui_action_label(&goal.action).to_string(),
            status,
        }));
    }
}

fn artifact_summary_for_user(paths: &[std::path::PathBuf]) -> Option<String> {
    if paths.is_empty() {
        return None;
    }
    let mut lines = vec!["Created files:".to_string()];
    for path in paths.iter().take(5) {
        lines.push(format!("- {}", path.display()));
    }
    if paths.len() > 5 {
        lines.push(format!("- {} more file(s)", paths.len() - 5));
    }
    Some(lines.join("\n"))
}

fn cap_output_for_user(raw: &str, max_chars: usize) -> String {
    let trimmed = raw.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let clipped: String = trimmed.chars().take(max_chars).collect();
    format!(
        "{}\n[output truncated after {} characters]",
        clipped, max_chars
    )
}

fn output_preview_from_artifacts(paths: &[std::path::PathBuf]) -> Option<String> {
    paths
        .iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("output_") && name.ends_with(".txt"))
                .unwrap_or(false)
        })
        .find_map(|path| std::fs::read_to_string(path).ok())
        .map(|content| cap_output_for_user(&content, 2000))
        .filter(|content| !content.trim().is_empty())
}

fn format_gui_workflow_success_for_user(
    result: &crate::agent::htn_executor::WorkflowResult,
    observable_narrative: Option<&str>,
) -> String {
    let mut lines = vec![format!(
        "Task completed. KRIA verified {} step{} in {}ms.",
        result.completed_steps,
        if result.completed_steps == 1 { "" } else { "s" },
        result.duration_ms
    )];
    if let Some(narrative) = observable_narrative.filter(|value| !value.trim().is_empty()) {
        lines.push(narrative.trim().to_string());
    }
    if let Some(artifacts) = artifact_summary_for_user(&result.created_artifacts) {
        lines.push(artifacts);
    }
    if let Some(output) = output_preview_from_artifacts(&result.created_artifacts) {
        lines.push(format!("Captured output:\n```\n{}\n```", output));
    }
    lines.join("\n\n")
}

#[allow(dead_code)] // Legacy: kept for rollback compatibility
fn observable_narrative_requires_partial_completion(observable_narrative: Option<&str>) -> bool {
    observable_narrative
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value.starts_with('⚠')
                || value.contains("Expected outcome not yet visible")
                || value.contains("not yet visible")
        })
        .unwrap_or(false)
}

#[allow(dead_code)] // Legacy: kept for rollback compatibility
fn format_gui_workflow_partial_for_user(
    result: &crate::agent::htn_executor::WorkflowResult,
    observable_narrative: Option<&str>,
) -> String {
    let mut lines = vec![format!(
        "Task partially completed. KRIA verified {} step{} structurally in {}ms, but the required visible outcome was not verified.",
        result.completed_steps,
        if result.completed_steps == 1 { "" } else { "s" },
        result.duration_ms
    )];
    if let Some(narrative) = observable_narrative.filter(|value| !value.trim().is_empty()) {
        lines.push(narrative.trim().to_string());
    }
    if let Some(artifacts) = artifact_summary_for_user(&result.created_artifacts) {
        lines.push(artifacts);
    }
    if let Some(output) = output_preview_from_artifacts(&result.created_artifacts) {
        lines.push(format!("Captured output:\n```\n{}\n```", output));
    }
    lines.push(
        "KRIA did not silently downgrade this to full GUI success; retry visible surfacing or inspect the artifacts above."
            .to_string(),
    );
    lines.join("\n\n")
}

fn format_gui_workflow_failure_for_user(
    result: &crate::agent::htn_executor::WorkflowResult,
) -> String {
    let detail = result.error.as_deref().unwrap_or("unknown error").trim();
    let mut lines = vec![format!(
        "Task did not fully complete. KRIA verified {} of {} step{} before stopping.",
        result.completed_steps,
        result.total_steps,
        if result.total_steps == 1 { "" } else { "s" }
    )];
    if result.completed_steps >= 2 && detail.contains("open_application_with_file") {
        lines.push(
            "The code was written and executed, but KRIA could not open the created file/output in the requested application."
                .to_string(),
        );
    }
    lines.push(format!("Failure: {}", detail));
    if let Some(artifacts) = artifact_summary_for_user(&result.created_artifacts) {
        lines.push(artifacts);
    }
    if let Some(output) = output_preview_from_artifacts(&result.created_artifacts) {
        lines.push(format!("Captured output:\n```\n{}\n```", output));
    }
    lines.push("No further actions were executed after this failure.".to_string());
    lines.join("\n\n")
}

/// Build context-aware recovery options when a GUI workflow fails.
/// Examines the error message and creates clickable buttons for the UI.
fn build_workflow_failure_recovery_options(
    error: &str,
    user_text: &str,
    artifacts: &[std::path::PathBuf],
) -> Vec<RecoveryOption> {
    let mut options = Vec::new();
    let lower = error.to_lowercase();

    // App not installed
    if lower.contains("not found in the installed app registry")
        || lower.contains("application not found")
    {
        // Extract app name from error
        let app_name =
            if let Some(re) = regex::Regex::new(r"application '([^']+)' is not found").ok() {
                re.captures(error)
                    .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
            } else {
                None
            };

        if let Some(app) = app_name {
            options.push(RecoveryOption {
                label: format!("Install {}", app),
                action_prompt: format!("Install {} on this system", app),
                style: "primary",
            });
        }
        options.push(RecoveryOption {
            label: "Use a different app".into(),
            action_prompt: "Suggest an alternative application I can use".into(),
            style: "secondary",
        });
    }

    // Step timeout — suggest retry or alternative
    if lower.contains("timed out") {
        options.push(RecoveryOption {
            label: "Retry the task".into(),
            action_prompt: user_text.to_string(),
            style: "primary",
        });
        if !artifacts.is_empty() {
            // If files were created, offer to open them
            if let Some(first) = artifacts.first() {
                options.push(RecoveryOption {
                    label: format!(
                        "Open the generated file ({})",
                        first.file_name().and_then(|n| n.to_str()).unwrap_or("file")
                    ),
                    action_prompt: format!("Open the file {}", first.display()),
                    style: "primary",
                });
            }
        }
    }

    // Permission denied
    if lower.contains("permission denied") || lower.contains("access denied") {
        options.push(RecoveryOption {
            label: "Try with a different path".into(),
            action_prompt: "Suggest a path I can write to without elevated permissions".into(),
            style: "primary",
        });
    }

    // GUI / uinput unavailable
    if lower.contains("gui_uinput_unavailable") || lower.contains("uinput") {
        options.push(RecoveryOption {
            label: "Enable GUI Automation in Settings".into(),
            action_prompt: "How do I enable GUI Automation in KRIA settings?".into(),
            style: "primary",
        });
        options.push(RecoveryOption {
            label: "Use file-based approach instead".into(),
            action_prompt: format!(
                "{} (use file-based approach, no keyboard injection needed)",
                user_text
            ),
            style: "secondary",
        });
    }

    // HITL denied
    if lower.contains("hitl_denied") || lower.contains("approval timed out") {
        options.push(RecoveryOption {
            label: "Try again (I'll watch for the approval prompt)".into(),
            action_prompt: user_text.to_string(),
            style: "primary",
        });
    }

    // Always offer rephrase + cancel
    if !options.is_empty() {
        options.push(RecoveryOption {
            label: "Rephrase or simplify".into(),
            action_prompt: "Let me rephrase this more simply.".into(),
            style: "secondary",
        });
    }

    options
}

/// Produce a TaskStep event reflecting the current package flow state after
/// a tool result has been observed. Returns None if no step is relevant.
///
/// Package flow steps:
///   Step 1: Search for package
///   Step 2: Check if already installed
///   Step 3: Install / Uninstall
///   Step 4: Verify installation state
fn package_flow_step_event(
    flow: &PackageFlowState,
    completed_tool: &str,
    success: bool,
) -> Option<TaskStep> {
    // Determine total steps based on intent
    let total: u32 = match flow.intent {
        PackageIntent::Install => 4,   // search → check → install → verify
        PackageIntent::Uninstall => 3, // check → uninstall → verify
    };

    let status = if success {
        TaskStepStatus::Done
    } else {
        TaskStepStatus::Failed
    };

    match completed_tool {
        "search_package" => {
            let found = flow.search_found.unwrap_or(false);
            Some(TaskStep {
                index: 1,
                total: Some(total),
                description: if found {
                    format!("Found '{}' in repositories", flow.package_name)
                } else {
                    format!("'{}' not found in repositories", flow.package_name)
                },
                status: if found {
                    TaskStepStatus::Done
                } else {
                    TaskStepStatus::Failed
                },
            })
        }

        "check_package_installed" => {
            let step_index = match flow.intent {
                PackageIntent::Install => {
                    if flow.action_attempted {
                        4
                    } else {
                        2
                    }
                }
                PackageIntent::Uninstall => {
                    if flow.action_attempted {
                        3
                    } else {
                        1
                    }
                }
            };

            let installed = flow
                .postcheck_installed
                .or(flow.precheck_installed)
                .unwrap_or(false);

            let description = if flow.action_attempted {
                // Post-action verification
                match flow.intent {
                    PackageIntent::Install => {
                        if installed {
                            format!("Verified: '{}' is installed", flow.package_name)
                        } else {
                            format!("Verification failed: '{}' not installed", flow.package_name)
                        }
                    }
                    PackageIntent::Uninstall => {
                        if !installed {
                            format!("Verified: '{}' is removed", flow.package_name)
                        } else {
                            format!(
                                "Verification failed: '{}' still installed",
                                flow.package_name
                            )
                        }
                    }
                }
            } else {
                // Pre-action check
                match flow.intent {
                    PackageIntent::Install => {
                        if installed {
                            format!("'{}' is already installed", flow.package_name)
                        } else {
                            format!("'{}' is not installed — will install", flow.package_name)
                        }
                    }
                    PackageIntent::Uninstall => {
                        if installed {
                            format!("'{}' is installed — will uninstall", flow.package_name)
                        } else {
                            format!("'{}' is not installed — nothing to do", flow.package_name)
                        }
                    }
                }
            };

            Some(TaskStep {
                index: step_index,
                total: Some(total),
                description,
                status,
            })
        }

        "install_package" => Some(TaskStep {
            index: 3,
            total: Some(total),
            description: if success {
                format!("Installed '{}'", flow.package_name)
            } else {
                format!("Failed to install '{}'", flow.package_name)
            },
            status,
        }),

        "uninstall_package" => Some(TaskStep {
            index: 2,
            total: Some(total),
            description: if success {
                format!("Uninstalled '{}'", flow.package_name)
            } else {
                format!("Failed to uninstall '{}'", flow.package_name)
            },
            status,
        }),

        _ => None,
    }
}

/// Classify any tool failure and produce structured recovery options./// Returns None if the failure is not actionable (e.g. generic LLM errors).
/// This is 100% deterministic — no LLM calls.
fn classify_tool_failure(
    tool_name: &str,
    error: &str,
    args: &serde_json::Value,
) -> Option<(String, String, Vec<RecoveryOption>)> {
    let err_lower = error.to_ascii_lowercase();

    // ── Fleet / remote command failures ──────────────────────────────────
    if tool_name == "execute_fleet_command" {
        let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
        let target = args.get("target").and_then(|v| v.as_str()).unwrap_or("VM");

        // Docker not installed
        if err_lower.contains("docker: command not found")
            || err_lower.contains("docker: not found")
            || (err_lower.contains("docker") && err_lower.contains("not found"))
        {
            return Some((
                format!("Docker is not installed on {target}"),
                error.to_string(),
                vec![
                    RecoveryOption {
                        label: "Install Docker on VM".into(),
                        action_prompt: format!(
                            "Install Docker on {target} using the appropriate package manager"
                        ),
                        style: "primary",
                    },
                    RecoveryOption {
                        label: "Check what's installed".into(),
                        action_prompt: format!(
                            "List installed packages on {target} to see what container tools are available"
                        ),
                        style: "secondary",
                    },
                ],
            ));
        }

        // Docker daemon not running
        if err_lower.contains("cannot connect to the docker daemon")
            || err_lower.contains("docker daemon is not running")
            || (err_lower.contains("docker") && err_lower.contains("is the docker daemon running"))
        {
            return Some((
                format!("Docker daemon is not running on {target}"),
                error.to_string(),
                vec![
                    RecoveryOption {
                        label: "Start Docker service".into(),
                        action_prompt: format!(
                            "Start the Docker service on {target} with: sudo systemctl start docker"
                        ),
                        style: "primary",
                    },
                    RecoveryOption {
                        label: "Enable Docker on boot".into(),
                        action_prompt: format!(
                            "Enable Docker to start automatically on {target}: sudo systemctl enable docker"
                        ),
                        style: "secondary",
                    },
                ],
            ));
        }

        // Permission denied (non-SSH)
        if err_lower.contains("permission denied") && !err_lower.contains("publickey") {
            return Some((
                format!("Permission denied running command on {target}"),
                error.to_string(),
                vec![
                    RecoveryOption {
                        label: "Run with sudo".into(),
                        action_prompt: format!(
                            "Run this command with sudo on {target}: sudo {command}"
                        ),
                        style: "primary",
                    },
                    RecoveryOption {
                        label: "Check user permissions".into(),
                        action_prompt: format!(
                            "Check what groups and permissions the current user has on {target}"
                        ),
                        style: "secondary",
                    },
                ],
            ));
        }

        // Service not found / not installed
        if err_lower.contains("unit") && err_lower.contains("not found") {
            return Some((
                format!("Service not found on {target}"),
                error.to_string(),
                vec![RecoveryOption {
                    label: "List running services".into(),
                    action_prompt: format!("List all running systemd services on {target}"),
                    style: "primary",
                }],
            ));
        }

        // Non-zero exit with stderr — generic command failure
        if !err_lower.is_empty() && err_lower.contains("non-zero status") {
            return Some((
                format!("Command failed on {target}"),
                error.to_string(),
                vec![
                    RecoveryOption {
                        label: "Retry command".into(),
                        action_prompt: format!(
                            "Try running this command again on {target}: {command}"
                        ),
                        style: "primary",
                    },
                    RecoveryOption {
                        label: "Check system logs".into(),
                        action_prompt: format!(
                            "Show recent system logs on {target} to diagnose the failure"
                        ),
                        style: "secondary",
                    },
                ],
            ));
        }

        return None;
    }

    // ── File operation failures ───────────────────────────────────────────
    if matches!(
        tool_name,
        "read_file" | "write_file" | "delete_file" | "copy_file" | "move_file"
    ) {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("source").and_then(|v| v.as_str()))
            .unwrap_or("the file");

        if err_lower.contains("no such file") || err_lower.contains("not found") {
            return Some((
                format!("File not found: {path}"),
                error.to_string(),
                vec![
                    RecoveryOption {
                        label: "Search for file".into(),
                        action_prompt: format!(
                            "Search for a file named '{}' on this system",
                            std::path::Path::new(path)
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or(path)
                        ),
                        style: "primary",
                    },
                    RecoveryOption {
                        label: "List directory".into(),
                        action_prompt: format!(
                            "List files in '{}'",
                            std::path::Path::new(path)
                                .parent()
                                .and_then(|p| p.to_str())
                                .unwrap_or(".")
                        ),
                        style: "secondary",
                    },
                ],
            ));
        }

        if err_lower.contains("permission denied") {
            return Some((
                format!("Permission denied accessing: {path}"),
                error.to_string(),
                vec![RecoveryOption {
                    label: "Check file permissions".into(),
                    action_prompt: format!("Show permissions for: {path}"),
                    style: "primary",
                }],
            ));
        }

        return None;
    }

    // ── Package management failures ───────────────────────────────────────
    if matches!(
        tool_name,
        "install_package" | "uninstall_package" | "search_package"
    ) {
        let pkg = args
            .get("name")
            .or_else(|| args.get("query"))
            .and_then(|v| v.as_str())
            .unwrap_or("the package");

        if err_lower.contains("not found")
            || err_lower.contains("no candidates")
            || err_lower.contains("unable to locate")
        {
            return Some((
                format!("Package '{pkg}' not found in repositories"),
                error.to_string(),
                vec![
                    RecoveryOption {
                        label: "Search with different name".into(),
                        action_prompt: format!(
                            "Search for packages related to '{pkg}' to find the correct package name"
                        ),
                        style: "primary",
                    },
                    RecoveryOption {
                        label: "Update package list".into(),
                        action_prompt: "Update the package repository list and try again".into(),
                        style: "secondary",
                    },
                ],
            ));
        }

        if err_lower.contains("permission denied") || err_lower.contains("are you root") {
            return Some((
                format!("Need elevated permissions to install '{pkg}'"),
                error.to_string(),
                vec![RecoveryOption {
                    label: "Retry with sudo".into(),
                    action_prompt: format!("Install '{pkg}' using sudo"),
                    style: "primary",
                }],
            ));
        }

        return None;
    }

    // ── Shell execution failures ──────────────────────────────────────────
    if tool_name == "execute_bash" || tool_name == "execute_python" {
        let command = args
            .get("command")
            .or_else(|| args.get("code"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if err_lower.contains("command not found") || err_lower.contains(": not found") {
            // Extract the missing command name
            let missing_cmd = err_lower
                .split(':')
                .next()
                .unwrap_or(command)
                .trim()
                .split_whitespace()
                .last()
                .unwrap_or(command);
            return Some((
                format!("Command not found: {missing_cmd}"),
                error.to_string(),
                vec![
                    RecoveryOption {
                        label: format!("Install {missing_cmd}"),
                        action_prompt: format!("Install the '{missing_cmd}' tool/package"),
                        style: "primary",
                    },
                    RecoveryOption {
                        label: "Check alternatives".into(),
                        action_prompt: format!(
                            "Find an alternative to '{missing_cmd}' that is already installed"
                        ),
                        style: "secondary",
                    },
                ],
            ));
        }

        if err_lower.contains("permission denied") {
            return Some((
                "Permission denied".into(),
                error.to_string(),
                vec![RecoveryOption {
                    label: "Run with sudo".into(),
                    action_prompt: format!("Run this command with sudo: {command}"),
                    style: "primary",
                }],
            ));
        }

        return None;
    }

    // ── Web / network failures ────────────────────────────────────────────
    if matches!(
        tool_name,
        "fetch_webpage" | "fetch_article" | "web_search" | "searxng_search"
    ) {
        if err_lower.contains("timeout") || err_lower.contains("timed out") {
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("the URL");
            return Some((
                "Request timed out".into(),
                error.to_string(),
                vec![
                    RecoveryOption {
                        label: "Retry".into(),
                        action_prompt: format!("Try fetching {url} again"),
                        style: "primary",
                    },
                    RecoveryOption {
                        label: "Check internet connection".into(),
                        action_prompt: "Check if the internet connection is working".into(),
                        style: "secondary",
                    },
                ],
            ));
        }

        if err_lower.contains("connection refused") || err_lower.contains("unreachable") {
            return Some((
                "Cannot reach the server".into(),
                error.to_string(),
                vec![RecoveryOption {
                    label: "Check internet connection".into(),
                    action_prompt: "Check if the internet connection is working".into(),
                    style: "primary",
                }],
            ));
        }

        return None;
    }

    // No actionable recovery for this failure
    None
}

/// Classify a fleet connectivity failure and produce structured recovery options./// This is model-agnostic: all logic is deterministic, no LLM calls.
fn classify_fleet_connectivity_failure(
    target: &str,
    error: &str,
    original_tool: &str,
    original_args: &serde_json::Value,
) -> (String, String, Vec<RecoveryOption>) {
    let error_lower = error.to_ascii_lowercase();
    let command = original_args
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let (context, detail, mut options) = if error_lower.contains("host unreachable")
        || error_lower.contains("no route to host")
        || error_lower.contains("network unreachable")
    {
        (
            format!("Cannot reach {target} — host is unreachable"),
            format!("Network error: {error}"),
            vec![
                RecoveryOption {
                    label: "Check VM status".into(),
                    action_prompt: format!("Check if {target} is online and reachable"),
                    style: "primary",
                },
                RecoveryOption {
                    label: "List enrolled VMs".into(),
                    action_prompt: "Show me all my enrolled VMs and their status".into(),
                    style: "secondary",
                },
            ],
        )
    } else if error_lower.contains("connection refused") {
        (
            format!("{target} refused the connection — SSH may not be running"),
            format!("SSH connection refused: {error}"),
            vec![
                RecoveryOption {
                    label: "Check VM status".into(),
                    action_prompt: format!("Check if {target} is online and SSH is running"),
                    style: "primary",
                },
                RecoveryOption {
                    label: "List enrolled VMs".into(),
                    action_prompt: "Show me all my enrolled VMs and their status".into(),
                    style: "secondary",
                },
            ],
        )
    } else if error_lower.contains("timed out") || error_lower.contains("timeout") {
        (
            format!("{target} is not responding — connection timed out"),
            format!("SSH timeout: {error}"),
            vec![
                RecoveryOption {
                    label: "Retry connection".into(),
                    action_prompt: format!("Try connecting to {target} again and run: {command}"),
                    style: "primary",
                },
                RecoveryOption {
                    label: "Check VM status".into(),
                    action_prompt: format!("Check if {target} is online"),
                    style: "secondary",
                },
            ],
        )
    } else if error_lower.contains("permission denied") || error_lower.contains("publickey") {
        (
            format!("SSH authentication failed for {target}"),
            format!("Permission denied (publickey): {error}"),
            vec![
                RecoveryOption {
                    label: "Check SSH keys".into(),
                    action_prompt: format!(
                        "Check SSH key configuration for {target} and show enrollment status"
                    ),
                    style: "primary",
                },
                RecoveryOption {
                    label: "Re-enroll VM".into(),
                    action_prompt: format!("Show me how to re-enroll {target} with KRIA fleet"),
                    style: "secondary",
                },
            ],
        )
    } else if error_lower.contains("no target")
        || error_lower.contains("no enrolled")
        || error_lower.contains("no ready")
    {
        (
            "No VM is enrolled or connected".into(),
            "KRIA fleet has no ready targets. Enroll a VM first.".into(),
            vec![
                RecoveryOption {
                    label: "How to enroll a VM".into(),
                    action_prompt: "How do I enroll a VM with KRIA fleet?".into(),
                    style: "primary",
                },
                RecoveryOption {
                    label: "List enrolled VMs".into(),
                    action_prompt: "Show me all my enrolled VMs".into(),
                    style: "secondary",
                },
            ],
        )
    } else {
        (
            format!("Could not connect to {target}"),
            error.to_string(),
            vec![
                RecoveryOption {
                    label: "Check VM status".into(),
                    action_prompt: format!("Check if {target} is online and reachable"),
                    style: "primary",
                },
                RecoveryOption {
                    label: "List enrolled VMs".into(),
                    action_prompt: "Show me all my enrolled VMs and their status".into(),
                    style: "secondary",
                },
            ],
        )
    };

    // Always add a "Retry" option if there was a specific command
    if !command.is_empty() && original_tool == "execute_fleet_command" {
        options.push(RecoveryOption {
            label: "Retry when connected".into(),
            action_prompt: format!("Once {target} is connected, run this command on it: {command}"),
            style: "secondary",
        });
    }

    (context, detail, options)
}

fn format_tool_satisfaction_summary(turn_memory: &TurnMemory) -> String {
    use crate::agent::result_synthesizer::ResultSynthesizer;

    let completed = turn_memory.get_completed_actions();
    if completed.is_empty() {
        return "Done. The requested action completed successfully.".into();
    }

    let synthesizer = ResultSynthesizer::default();

    if completed.len() == 1 {
        let action = &completed[0];
        // Use the synthesizer to produce a proper human-readable response
        let tool_result = crate::infra::isolation::ToolResult::ok(action.result_data.clone());
        let synthesized = synthesizer.synthesize(&action.tool_name, &tool_result, None);
        return synthesized.human_readable;
    }

    // Multiple tools: synthesize each and combine
    let mut out = String::new();
    for action in completed {
        let tool_result = crate::infra::isolation::ToolResult::ok(action.result_data.clone());
        let synthesized = synthesizer.synthesize(&action.tool_name, &tool_result, None);
        out.push_str(&synthesized.human_readable);
        out.push('\n');
    }
    out
}

fn build_message_preview(messages: &[ChatMessage], max_messages: usize) -> serde_json::Value {
    let start = messages.len().saturating_sub(max_messages);
    let preview: Vec<serde_json::Value> = messages
        .iter()
        .skip(start)
        .map(|m| {
            let content_chars = m.content.chars().count();
            let content_preview = if m.role.eq_ignore_ascii_case("system") {
                format!("[system prompt omitted; {content_chars} chars]")
            } else {
                sanitize_text_for_logs(&m.content, 160)
            };

            serde_json::json!({
                "role": m.role,
                "name": m.name,
                "has_images": m.has_images(),
                "content": content_preview,
                "content_chars": content_chars,
            })
        })
        .collect();

    serde_json::Value::Array(preview)
}

const MAX_ROUTED_TOOL_SCHEMAS_PER_TURN: usize = 8;

fn extract_user_context_block(system_prompt: &str) -> Option<String> {
    const USER_CONTEXT_HEADER: &str = "## User Context";
    const RESPONSE_MARKER: &str = "Respond naturally.";

    let start = system_prompt.find(USER_CONTEXT_HEADER)?;
    let after_header = &system_prompt[start + USER_CONTEXT_HEADER.len()..];
    let end = after_header
        .find(RESPONSE_MARKER)
        .unwrap_or(after_header.len());
    let block = after_header[..end].trim();
    if block.is_empty() {
        None
    } else {
        Some(block.to_string())
    }
}

fn build_filtered_tool_schema_catalog(tool_schemas: &[ToolSchema]) -> String {
    if tool_schemas.is_empty() {
        return "No tools are enabled for this turn. Reply conversationally unless a tool-enabled follow-up is required.".to_string();
    }

    let mut lines = Vec::with_capacity(tool_schemas.len() + 2);
    lines.push(format!(
        "Only the following {} routed tool(s) are enabled for this turn.",
        tool_schemas.len()
    ));
    lines.push(
        "Use exact tool names. Function schemas are provided separately by the runtime."
            .to_string(),
    );

    for schema in tool_schemas {
        lines.push(format!(
            "- {}: {}",
            schema.name,
            sanitize_text_for_logs(&schema.description, 120)
        ));
    }

    lines.join("\n")
}

fn rewrite_system_prompt_tools_block(
    system_prompt: &str,
    tool_schemas: &[ToolSchema],
    is_live_fact: bool,
) -> String {
    // ── New path: use typed prompt compiler ──
    // This produces semantically equivalent output to the legacy implementation
    // but with deterministic ordering, budget awareness, and omission auditing.
    let assembled = crate::agent::prompt_compiler::compile_system_prompt(
        system_prompt,
        tool_schemas,
        is_live_fact,
        0, // 0 = default budget (8192 chars)
    );
    assembled.text
}

/// Legacy implementation preserved for reference during migration.
/// Remove after one release cycle once the new compiler is validated.
#[allow(dead_code)]
fn _legacy_rewrite_system_prompt_tools_block(
    system_prompt: &str,
    tool_schemas: &[ToolSchema],
    is_live_fact: bool,
) -> String {
    let user_context = extract_user_context_block(system_prompt);
    let mut rebuilt = String::with_capacity(2800);
    rebuilt.push_str(
        "You are K.R.I.A., a desktop AI assistant.\n\n\
## Core Rules\n\
1. Use tools when the user asks for actions or live data; otherwise answer conversationally.\n\
2. Never invent tool outputs. If a tool fails, report the failure and retry with a sensible alternative.\n\
3. Do not ask for confirmation when intent is clear. Execute the best matching tool.\n\
4. Keep responses concise and grounded in available evidence.\n\
5. Match the user's language.\n\
6. For web/info lookup use dedicated web/news tools, not browser-opening tools unless user explicitly asks to open a browser.\n\n\
## Enabled Tools\n",
    );
    rebuilt.push_str(&build_filtered_tool_schema_catalog(tool_schemas));

    // Layer 3: Neutral date injection - operational, no epistemic handicapping
    rebuilt.push_str(&format!(
        "\n\n## System State\nCurrent date: {}. \
        Verify time-sensitive facts (political offices, prices, scores, recent events) \
        using the enabled search tools before synthesizing an answer.\n",
        chrono::Local::now().format("%A, %B %d, %Y")
    ));

    // CRITICAL instruction for live-fact queries: trust search results over training data
    if is_live_fact {
        rebuilt.push_str(
            "\n**CRITICAL - LIVE FACT MODE ACTIVE**: \
            When search results are shown above, you MUST base your answer SOLELY on those results. \
            If search results contradict your training data, TRUST THE SEARCH RESULTS. \
            Sources marked with [WARNING: SOURCE DATE UNKNOWN] should be treated as uncertain. \
            Do not blend training data with search results. Answer strictly from the provided search evidence.\n"
        );
    }

    if let Some(context) = user_context {
        rebuilt.push_str("\n\n## User Context\n");
        rebuilt.push_str(&sanitize_text_for_logs(&context, 1200));
    }

    rebuilt.push_str(
        "\n\nWhen tools are needed, emit:\n\
<tool_call>\n\
{\"name\":\"tool_name\",\"arguments\":{\"param\":\"value\"}}\n\
</tool_call>\n\
Then continue with grounded results.",
    );
    rebuilt
}

fn truncate_text_for_context(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }
    if max_chars <= 40 {
        return text.chars().take(max_chars).collect();
    }

    let head_budget = (max_chars * 3) / 4;
    let tail_budget = max_chars.saturating_sub(head_budget).saturating_sub(24);
    let head: String = text.chars().take(head_budget).collect();
    let tail: String = if tail_budget > 0 {
        text.chars()
            .rev()
            .take(tail_budget)
            .collect::<String>()
            .chars()
            .rev()
            .collect()
    } else {
        String::new()
    };
    let omitted = char_count.saturating_sub(head_budget + tail_budget);
    if tail.is_empty() {
        format!("{head}\n...[truncated {omitted} chars]")
    } else {
        format!("{head}\n...[truncated {omitted} chars]\n{tail}")
    }
}

fn compact_messages_for_chat(messages: &mut Vec<ChatMessage>) {
    compact_messages_with_budgets(messages, &ContextBudgets::local_4k());
}

/// Provider-aware message compaction. Uses `ContextBudgets` for all thresholds.
fn compact_messages_with_budgets(messages: &mut Vec<ChatMessage>, budgets: &ContextBudgets) {
    if messages.is_empty() {
        return;
    }

    let mut latest_user_idx = messages.iter().rposition(|m| m.role == "user");

    for (idx, msg) in messages.iter_mut().enumerate() {
        if msg.role.eq_ignore_ascii_case("system") {
            // System prompt cap: scale with context window (up to 2× base)
            let max_chars = if idx == 0 {
                (budgets.system_reserve * 4).max(3_500)
            } else {
                1_000
            };
            msg.content = truncate_text_for_context(&msg.content, max_chars);
            continue;
        }

        if Some(idx) == latest_user_idx {
            // Latest user message: always preserve up to 2× item cap
            msg.content =
                truncate_text_for_context(&msg.content, budgets.history_item_char_cap * 2);
            continue;
        }

        msg.content = truncate_text_for_context(&msg.content, budgets.history_item_char_cap);
    }

    let mut total_chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();
    while total_chars > budgets.history_char_budget && messages.len() > 2 {
        let removable_idx = messages.iter().enumerate().skip(1).find_map(|(idx, msg)| {
            if msg.role.eq_ignore_ascii_case("system") || Some(idx) == latest_user_idx {
                None
            } else {
                Some(idx)
            }
        });

        let Some(idx) = removable_idx else {
            break;
        };

        total_chars = total_chars.saturating_sub(messages[idx].content.chars().count());
        messages.remove(idx);

        if let Some(user_idx) = latest_user_idx {
            if idx < user_idx {
                latest_user_idx = Some(user_idx - 1);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct VisualTokenCapDecision {
    hard_cap: u32,
    safe_cap: u32,
    free_vram_mb: u64,
    safety_margin_mb: u64,
    vision_mode: VisionMode,
}

fn add_tool_if_available(
    allowed_tool_names: &HashSet<String>,
    selected: &mut HashSet<String>,
    name: &str,
) {
    if allowed_tool_names.contains(name) {
        selected.insert(name.to_string());
    }
}

fn fallback_routed_tool_candidates(
    user_text: &str,
    intent_hint: Option<&str>,
    allowed_tool_names: &HashSet<String>,
) -> HashSet<String> {
    let mut selected = HashSet::new();
    let lower = user_text.to_ascii_lowercase();

    if let Some(hint) = intent_hint.map(str::trim).filter(|s| !s.is_empty()) {
        add_tool_if_available(allowed_tool_names, &mut selected, hint);
    }

    let wants_installed_list = lower.contains("installed app")
        || lower.contains("installed apps")
        || lower.contains("installed application")
        || lower.contains("installed applications")
        || lower.contains("installed package")
        || lower.contains("installed packages")
        || lower.contains("installed programs")
        || (lower.contains("list")
            && (lower.contains("apps")
                || lower.contains("applications")
                || lower.contains("packages")
                || lower.contains("programs"))
            && lower.contains("installed"));

    if lower.contains("install")
        || lower.contains("uninstall")
        || lower.contains("package")
        || wants_installed_list
    {
        for tool in [
            "list_installed_packages",
            "search_package",
            "check_package_installed",
            "install_package",
            "uninstall_package",
        ] {
            add_tool_if_available(allowed_tool_names, &mut selected, tool);
        }
    }

    if lower.contains("news") || lower.contains("headline") {
        add_tool_if_available(allowed_tool_names, &mut selected, "search_news");
    }

    // ── Web search vs GUI launch disambiguation ───────────────────────────
    // Use the GuiIntentClassifier to distinguish:
    //   "search for X online" → web_search / searxng_search
    //   "open chrome and search for X" → browser_search
    //   "search for X on youtube" → browser_search (site navigation)
    // This avoids the keyword collision where "search" appears in both intents.
    {
        use crate::routing::gui_intent::{classify_gui_intent, GuiIntent};
        let gui_score = classify_gui_intent(user_text);

        match gui_score.intent {
            GuiIntent::GuiLaunch => {
                // User wants to open a browser/app — route to GUI tools
                add_tool_if_available(allowed_tool_names, &mut selected, "browser_search");
                add_tool_if_available(allowed_tool_names, &mut selected, "open_application");
                add_tool_if_available(allowed_tool_names, &mut selected, "open_url");
            }
            GuiIntent::InfoRetrieval => {
                // User wants information — route to search tools
                add_tool_if_available(allowed_tool_names, &mut selected, "web_search");
                add_tool_if_available(allowed_tool_names, &mut selected, "searxng_search");
            }
            GuiIntent::Ambiguous => {
                // Ambiguous: expose both sets and let the LLM + system prompt decide.
                // The system prompt's rule 32 will guide the LLM correctly.
                if lower.contains("search")
                    || lower.contains("look up")
                    || lower.contains("find information")
                    || lower.contains("web")
                {
                    add_tool_if_available(allowed_tool_names, &mut selected, "web_search");
                    add_tool_if_available(allowed_tool_names, &mut selected, "searxng_search");
                }
            }
        }
    }

    if lower.contains("file") || lower.contains("folder") || lower.contains("directory") {
        for tool in [
            "mcp_fs_search_files",
            "search_files",
            "find_files_by_pattern",
            "list_directory",
            "list_files",
            "mcp_fs_list_directory",
        ] {
            add_tool_if_available(allowed_tool_names, &mut selected, tool);
        }
    }

    // Webpage / URL analysis: when the user references a URL or talks about
    // analyzing/summarizing a page, expose fetch_webpage and fetch_article.
    let mentions_url = lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("www.")
        || lower.contains("url")
        || lower.contains("link")
        || lower.contains("webpage")
        || lower.contains("web page")
        || lower.contains("website");
    let analysis_intent = lower.contains("analyze")
        || lower.contains("summarize")
        || lower.contains("summarise")
        || lower.contains("read")
        || lower.contains("fetch")
        || lower.contains("scrape")
        || lower.contains("extract");
    if mentions_url || (analysis_intent && lower.contains("page")) {
        for tool in ["fetch_webpage", "fetch_article"] {
            add_tool_if_available(allowed_tool_names, &mut selected, tool);
        }
    }

    // Docker / container inspection.
    // If the query references a remote target (VM, server, remote host), route to
    // execute_fleet_command so the command runs on the enrolled target.
    // Otherwise route to local execute_bash.
    if lower.contains("docker") || lower.contains("container") {
        let is_remote = lower.contains(" vm")
            || lower.contains(" vms")
            || lower.contains("virtual machine")
            || lower.contains("remote")
            || lower.contains("server")
            || lower.contains("ssh")
            || lower.contains("fleet")
            || lower.contains("enrolled");

        if is_remote {
            add_tool_if_available(allowed_tool_names, &mut selected, "execute_fleet_command");
            add_tool_if_available(allowed_tool_names, &mut selected, "check_device_health");
            add_tool_if_available(allowed_tool_names, &mut selected, "get_fleet_overview");
        } else {
            add_tool_if_available(allowed_tool_names, &mut selected, "execute_bash");
            add_tool_if_available(allowed_tool_names, &mut selected, "execute_python");
        }
    }

    // Git operations: expose git tools when git intent is detected.
    let git_signals = [
        "git ",
        "branch",
        "commit",
        "merge",
        "rebase",
        "checkout",
        "stash",
        "repo",
        "repository",
    ];
    if git_signals.iter().any(|s| lower.contains(s)) {
        for tool in [
            "git_status",
            "git_log",
            "git_diff",
            "git_branch_list",
            "git_commit",
            "git_checkout",
            "git_stash",
            "git_push",
            "git_pull",
            "git_fetch",
            "git_merge",
            "git_remote",
        ] {
            add_tool_if_available(allowed_tool_names, &mut selected, tool);
        }
    }

    if lower.contains("image")
        || lower.contains("draw")
        || lower.contains("generate")
        || lower.contains("art")
    {
        add_tool_if_available(allowed_tool_names, &mut selected, "generate_image");
    }

    if looks_like_google_workspace_request(&lower) {
        for tool in [
            "gw_gmail_inbox",
            "gw_gmail_search",
            "gw_gmail_read",
            "gw_gmail_send",
            "gw_calendar_search",
            "gw_calendar_create",
            "gw_drive_search",
            "gw_drive_read",
            "gw_docs_read",
            "gw_docs_edit",
        ] {
            add_tool_if_available(allowed_tool_names, &mut selected, tool);
        }
    }

    selected
}

fn score_tool_relevance(query_text: &str, schema: &ToolSchema) -> i32 {
    let query = query_text.to_ascii_lowercase();
    let name = schema.name.to_ascii_lowercase();
    let description = schema.description.to_ascii_lowercase();
    let mut score = 0;

    for token in query
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 3)
    {
        if name.contains(token) {
            score += 6;
        }
        if description.contains(token) {
            score += 2;
        }
    }

    if (query.contains("install") || query.contains("uninstall") || query.contains("package"))
        && schema.name.contains("package")
    {
        score += 8;
    }
    if query.contains("news") && schema.name == "search_news" {
        score += 10;
    }
    if (query.contains("search") || query.contains("web"))
        && (schema.name == "web_search" || schema.name == "searxng_search")
    {
        score += 8;
    }
    if query.contains("image") && schema.name == "generate_image" {
        score += 10;
    }

    // ── GUI-intent aware boosting ─────────────────────────────────────────
    // Use the GuiIntentClassifier to boost browser_search for GUI-launch queries
    // and suppress it for info-retrieval queries. This prevents the word "search"
    // from incorrectly boosting web_search/searxng_search for GUI queries.
    {
        use crate::routing::gui_intent::{classify_gui_intent, GuiIntent};
        let gui = classify_gui_intent(query_text);
        match gui.intent {
            GuiIntent::GuiLaunch => {
                if schema.name == "browser_search" {
                    score += 12; // Strong boost — GUI launch should always prefer browser_search
                }
                if schema.name == "web_search" || schema.name == "searxng_search" {
                    score -= 6; // Suppress web search tools for GUI queries
                }
            }
            GuiIntent::InfoRetrieval => {
                if schema.name == "browser_search" {
                    score -= 4; // Suppress browser_search for info queries
                }
                if schema.name == "web_search" || schema.name == "searxng_search" {
                    score += 4; // Boost web search for info queries
                }
            }
            GuiIntent::Ambiguous => {
                // No adjustment — let other signals decide
            }
        }
    }

    score
}

/// A semantic injection candidate from the tool embedding index.
#[derive(Debug, Clone)]
pub struct SemanticInjection {
    pub name: String,
    pub cosine_similarity: f32,
}

/// Prepend marker for tools injected via cross-domain semantic search.
const SEMANTIC_OVERRIDE_PREFIX: &str = "[HIGH RELEVANCE OVERRIDE] - ";

#[allow(clippy::too_many_arguments)]
fn select_routed_tool_schemas(
    all_tool_schemas: &[ToolSchema],
    query_text: &str,
    direct_tool_hint: Option<&str>,
    selected_tool_names: &HashSet<String>,
    fallback_tool_names: &HashSet<String>,
    forced_tool_name: Option<&str>,
    tool_lock_name: Option<&str>,
    _conversation_only: bool,
    semantic_injections: &[SemanticInjection],
) -> Vec<ToolSchema> {
    // ── Phase A: Build the ONNX-domain include set ──────────────────────
    let mut include_names: HashSet<String> = if direct_tool_hint.is_some() {
        HashSet::new()
    } else {
        selected_tool_names.clone()
    };
    let mut pinned_names: HashSet<String> = HashSet::new();
    if let Some(tool) = direct_tool_hint.map(str::trim).filter(|s| !s.is_empty()) {
        include_names.insert(tool.to_string());
        pinned_names.insert(tool.to_string());
    }
    if let Some(tool) = forced_tool_name.map(str::trim).filter(|s| !s.is_empty()) {
        include_names.insert(tool.to_string());
        pinned_names.insert(tool.to_string());
    }
    if let Some(tool) = tool_lock_name.map(str::trim).filter(|s| !s.is_empty()) {
        include_names.insert(tool.to_string());
        pinned_names.insert(tool.to_string());
    }
    if include_names.is_empty() {
        include_names.extend(fallback_tool_names.iter().cloned());
        pinned_names.extend(fallback_tool_names.iter().cloned());
    }

    // ── Phase B: Inject semantic Top-K + fallback candidates ────────────
    // These cross domain boundaries — the whole point of hybrid assembly.
    let domain_tool_names: HashSet<String> = include_names.clone();

    for inj in semantic_injections {
        include_names.insert(inj.name.clone());
    }
    for name in fallback_tool_names {
        include_names.insert(name.clone());
    }

    // ── Phase C: Build filtered list with attention hack ────────────────
    // When a web-search tool is forced (e.g. by the live-fact classifier),
    // exclude GUI/browser-opening tools so the LLM doesn't get confused
    // between information retrieval and browser automation.
    let forced_is_search = forced_tool_name
        .map(|t| matches!(t, "searxng_search" | "web_search" | "search_news"))
        .unwrap_or(false);
    // Symmetric exclusion: when a GUI-launch tool is forced (browser_search,
    // open_application, etc.), exclude info-retrieval tools so the LLM doesn't
    // tack on `web_search` / `search_news` after a successful launch and turn
    // a GUI workflow into an info-retrieval ReAct loop.
    let forced_is_gui_launch = forced_tool_name
        .map(|t| {
            matches!(
                t,
                "browser_search" | "open_application" | "open_url" | "open_application_with_file"
            )
        })
        .unwrap_or(false);
    let gui_tools_to_exclude: &[&str] = if forced_is_search {
        &["browser_search", "open_application", "open_url"]
    } else {
        &[]
    };
    let search_tools_to_exclude: &[&str] = if forced_is_gui_launch {
        &["web_search", "searxng_search", "search_news"]
    } else {
        &[]
    };

    let filtered: Vec<ToolSchema> = if include_names.is_empty() {
        Vec::new()
    } else {
        all_tool_schemas
            .iter()
            .filter(|schema| {
                include_names.contains(&schema.name)
                    && !gui_tools_to_exclude.contains(&schema.name.as_str())
                    && !search_tools_to_exclude.contains(&schema.name.as_str())
            })
            .map(|schema| {
                // Attention hack: if this tool was NOT in the original ONNX
                // domain set, prepend the override marker so the LLM's
                // attention mechanism prioritises it over domain-default tools.
                let was_domain = domain_tool_names.contains(&schema.name);
                let was_semantic = semantic_injections.iter().any(|i| i.name == schema.name);
                let was_fallback = fallback_tool_names.contains(&schema.name);
                let is_injected = !was_domain && (was_semantic || was_fallback);

                if is_injected {
                    let mut boosted = schema.clone();
                    if !boosted.description.starts_with(SEMANTIC_OVERRIDE_PREFIX) {
                        boosted.description =
                            format!("{}{}", SEMANTIC_OVERRIDE_PREFIX, boosted.description);
                    }
                    boosted
                } else {
                    schema.clone()
                }
            })
            .collect()
    };

    // ── Phase D: Rank by relevance score ────────────────────────────────
    let mut ranked: Vec<(bool, i32, ToolSchema)> = filtered
        .into_iter()
        .map(|schema| {
            let pinned = pinned_names.contains(&schema.name);
            let score = score_tool_relevance(query_text, &schema);
            (pinned, score, schema)
        })
        .collect();

    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.name.cmp(&b.2.name))
    });

    if ranked.len() > MAX_ROUTED_TOOL_SCHEMAS_PER_TURN {
        ranked.truncate(MAX_ROUTED_TOOL_SCHEMAS_PER_TURN);
    }

    ranked.into_iter().map(|(_, _, schema)| schema).collect()
}

fn build_tool_calls_preview(tool_calls: &[ParsedToolCall]) -> serde_json::Value {
    let preview: Vec<serde_json::Value> = tool_calls
        .iter()
        .map(|call| {
            serde_json::json!({
                "name": call.name,
                "arguments": sanitize_json_for_logs(&call.arguments, 220, 8),
            })
        })
        .collect();

    serde_json::Value::Array(preview)
}

fn build_tool_call_history_content(tool_calls: &[ParsedToolCall]) -> String {
    tool_calls
        .iter()
        .map(|call| {
            format!(
                "<tool_call>\n{{\"name\":\"{}\",\"arguments\":{}}}\n</tool_call>",
                call.name, call.arguments
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn tool_choice_label(name: &str) -> String {
    match name {
        "search_news" => "News Search".into(),
        "web_search" | "searxng_search" => "Web Search".into(),
        "search_files" | "find_files_by_pattern" | "mcp_fs_search_files" => "File Search".into(),
        "open_application" => "Open App".into(),
        "open_url" => "Open URL".into(),
        "browser_search" => "Browser Search".into(),
        "send_message" => "Send Message".into(),
        "close_application" | "kill_process" => "Close App".into(),
        "gw_gmail_inbox" | "gw_gmail_search" | "gw_gmail_read" | "gw_gmail_send"
        | "gw_gmail_delete" => "Gmail".into(),
        "gw_calendar_today" | "gw_calendar_search" | "gw_calendar_create"
        | "gw_calendar_delete" => "Google Calendar".into(),
        "gw_drive_search" | "gw_drive_list" | "gw_drive_read" | "gw_drive_delete" => {
            "Google Drive".into()
        }
        "gw_docs_create" | "gw_docs_read" | "gw_docs_edit" => "Google Docs".into(),
        "gw_sheets_create" | "gw_sheets_read" | "gw_sheets_edit" => "Google Sheets".into(),
        "gw_slides_create" | "gw_slides_read" => "Google Slides".into(),
        "gw_forms_list" | "gw_forms_create" => "Google Forms".into(),
        other if other.starts_with("mcp_") && other.contains("colab") => "Google Colab".into(),
        other => other.to_string(),
    }
}

fn push_tool_choice_candidate(
    candidates: &mut Vec<ToolChoiceCandidate>,
    allowed_tool_names: &HashSet<String>,
    name: &str,
    reason: &str,
    confidence: f32,
) {
    if !allowed_tool_names.contains(name) {
        return;
    }

    if candidates.iter().any(|c| c.name == name) {
        return;
    }

    candidates.push(ToolChoiceCandidate {
        name: name.to_string(),
        label: tool_choice_label(name),
        reason: reason.to_string(),
        confidence,
    });
}

fn build_tool_choice_candidates(
    user_text: &str,
    allowed_tool_names: &HashSet<String>,
    primary_hint: Option<&str>,
    confidence: f32,
) -> Vec<ToolChoiceCandidate> {
    let mut candidates: Vec<ToolChoiceCandidate> = Vec::new();
    let lower = user_text.to_lowercase();

    if let Some(primary) = primary_hint {
        push_tool_choice_candidate(
            &mut candidates,
            allowed_tool_names,
            primary,
            "Primary match from intent classifier",
            confidence,
        );
    }

    if lower.contains("news") || lower.contains("headline") {
        push_tool_choice_candidate(
            &mut candidates,
            allowed_tool_names,
            "search_news",
            "Best for current events and corroborated headlines",
            0.62,
        );
    }

    if lower.contains("search") || lower.contains("online") || lower.contains("web") {
        push_tool_choice_candidate(
            &mut candidates,
            allowed_tool_names,
            "web_search",
            "Best for broad web lookups",
            0.60,
        );
        push_tool_choice_candidate(
            &mut candidates,
            allowed_tool_names,
            "searxng_search",
            "Best for self-hosted/privacy web lookups",
            0.58,
        );
    }

    if lower.contains("file") || lower.contains("folder") || lower.contains("directory") {
        push_tool_choice_candidate(
            &mut candidates,
            allowed_tool_names,
            "mcp_fs_search_files",
            "Best for workspace/filesystem search",
            0.61,
        );
        push_tool_choice_candidate(
            &mut candidates,
            allowed_tool_names,
            "find_files_by_pattern",
            "Best for local file pattern lookup",
            0.57,
        );
    }

    if looks_like_google_workspace_request(&lower) {
        for tool in [
            "gw_gmail_inbox",
            "gw_gmail_search",
            "gw_gmail_send",
            "gw_calendar_search",
            "gw_calendar_create",
            "gw_drive_list",
            "gw_drive_search",
            "gw_docs_read",
            "gw_sheets_read",
            "gw_slides_read",
            "gw_forms_list",
        ] {
            push_tool_choice_candidate(
                &mut candidates,
                allowed_tool_names,
                tool,
                "Google Workspace request detected",
                0.56,
            );
        }
    }

    if looks_like_colab_request(&lower) {
        for tool in allowed_tool_names
            .iter()
            .filter(|name| name.starts_with("mcp_") && name.contains("colab"))
            .take(6)
        {
            push_tool_choice_candidate(
                &mut candidates,
                allowed_tool_names,
                tool,
                "Google Colab request detected",
                0.56,
            );
        }
    }

    candidates.truncate(6);
    candidates
}

fn build_grounding_count_note(tool_name: &str, tool_result: &serde_json::Value) -> Option<String> {
    if !tool_name.starts_with("gw_") {
        return None;
    }

    let payload = tool_result.get("data").unwrap_or(tool_result);
    let requested = payload.get("requested_count").and_then(|v| v.as_u64())?;
    let returned = payload
        .get("returned_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(requested);

    if let Some(visible) = payload
        .get("llm_visible_message_count")
        .and_then(|v| v.as_u64())
    {
        if visible < returned {
            return Some(format!(
                "GROUNDING_NOTE: requested {requested} item(s), returned {returned} grounded item(s), but only {visible} row(s) are visible in this context. Do NOT invent or duplicate hidden rows; enumerate at most {visible} visible row(s) and mention that additional rows were omitted."
            ));
        }
    }

    Some(format!(
        "GROUNDING_NOTE: requested {requested} item(s), returned {returned} grounded item(s). Never claim or enumerate more than {returned}."
    ))
}

const LLM_GMAIL_MESSAGES_CHAR_BUDGET: usize = 3500;
const LLM_GMAIL_PREVIEW_MAX_CHARS: usize = 220;
const LLM_GMAIL_FIELD_MAX_CHARS: usize = 160;
const LLM_GMAIL_WARNING_MAX_CHARS: usize = 180;
const LLM_GMAIL_WARNING_LIMIT: usize = 3;

fn compact_text_for_llm(raw: &str, max_chars: usize) -> String {
    let filtered: String = raw
        .chars()
        .filter(|ch| {
            !matches!(
                *ch,
                '\u{034F}' | '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}'
            )
        })
        .collect();
    let collapsed = filtered.split_whitespace().collect::<Vec<_>>().join(" ");

    let trimmed = collapsed.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }

    let mut truncated: String = trimmed.chars().take(max_chars).collect();
    truncated.push_str("...");
    truncated
}

fn first_non_empty_string_field(
    message: &serde_json::Value,
    keys: &[&str],
    max_chars: usize,
) -> Option<String> {
    keys.iter().find_map(|key| {
        message
            .get(*key)
            .and_then(|v| v.as_str())
            .map(|v| compact_text_for_llm(v, max_chars))
            .filter(|v| !v.is_empty())
    })
}

fn compact_gmail_message_for_llm(message: &serde_json::Value) -> serde_json::Value {
    if !message.is_object() {
        return message.clone();
    }

    let mut compacted = serde_json::Map::new();

    if let Some(subject) = first_non_empty_string_field(
        message,
        &["subject", "title", "summary"],
        LLM_GMAIL_FIELD_MAX_CHARS,
    ) {
        compacted.insert("subject".into(), serde_json::Value::String(subject));
    }

    if let Some(from) = first_non_empty_string_field(
        message,
        &["from", "sender", "organizer"],
        LLM_GMAIL_FIELD_MAX_CHARS,
    ) {
        compacted.insert("from".into(), serde_json::Value::String(from));
    }

    if let Some(date) = first_non_empty_string_field(
        message,
        &["date", "updated", "created"],
        LLM_GMAIL_FIELD_MAX_CHARS,
    ) {
        compacted.insert("date".into(), serde_json::Value::String(date));
    }

    if let Some(id) = first_non_empty_string_field(
        message,
        &["id", "messageId", "message_id", "threadId", "thread_id"],
        LLM_GMAIL_FIELD_MAX_CHARS,
    ) {
        compacted.insert("id".into(), serde_json::Value::String(id));
    }

    if let Some(preview) = first_non_empty_string_field(
        message,
        &[
            "preview",
            "snippet",
            "description",
            "text",
            "content",
            "body",
        ],
        LLM_GMAIL_PREVIEW_MAX_CHARS,
    ) {
        compacted.insert("preview".into(), serde_json::Value::String(preview));
    }

    if let Some(url) = first_non_empty_string_field(
        message,
        &["url", "htmlLink", "webViewLink", "alternateLink"],
        LLM_GMAIL_FIELD_MAX_CHARS,
    ) {
        compacted.insert("url".into(), serde_json::Value::String(url));
    }

    serde_json::Value::Object(compacted)
}

fn compact_gmail_messages_for_llm(
    messages: &[serde_json::Value],
) -> (Vec<serde_json::Value>, usize) {
    let mut visible = Vec::new();
    let mut used_chars = 0usize;
    let mut omitted = 0usize;

    for (index, message) in messages.iter().enumerate() {
        let compacted = compact_gmail_message_for_llm(message);
        let chunk_len = compacted.to_string().len();

        if index == 0 || used_chars + chunk_len <= LLM_GMAIL_MESSAGES_CHAR_BUDGET {
            used_chars += chunk_len;
            visible.push(compacted);
        } else {
            omitted += 1;
        }
    }

    (visible, omitted)
}

fn compact_gmail_payload_for_llm(payload: &serde_json::Value) -> serde_json::Value {
    let Some(payload_obj) = payload.as_object() else {
        return payload.clone();
    };

    let mut compacted = payload_obj.clone();

    if let Some(query) = compacted.get("query").and_then(|v| v.as_str()) {
        compacted.insert(
            "query".into(),
            serde_json::Value::String(compact_text_for_llm(query, LLM_GMAIL_FIELD_MAX_CHARS)),
        );
    }

    if let Some(warnings) = compacted.get("warnings").and_then(|v| v.as_array()) {
        let compacted_warnings: Vec<serde_json::Value> = warnings
            .iter()
            .take(LLM_GMAIL_WARNING_LIMIT)
            .filter_map(|warning| warning.as_str())
            .map(|warning| {
                serde_json::Value::String(compact_text_for_llm(
                    warning,
                    LLM_GMAIL_WARNING_MAX_CHARS,
                ))
            })
            .collect();
        compacted.insert(
            "warnings".into(),
            serde_json::Value::Array(compacted_warnings),
        );
    }

    let messages = compacted
        .get("messages")
        .or_else(|| compacted.get("results"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if !messages.is_empty() {
        let total = messages.len();
        let (visible_messages, omitted_messages) = compact_gmail_messages_for_llm(&messages);
        compacted.insert(
            "messages".into(),
            serde_json::Value::Array(visible_messages.clone()),
        );
        compacted.insert(
            "llm_visible_message_count".into(),
            serde_json::json!(visible_messages.len()),
        );
        if omitted_messages > 0 {
            compacted.insert(
                "llm_omitted_message_count".into(),
                serde_json::json!(omitted_messages),
            );
            compacted.insert(
                "warnings".into(),
                match compacted.get("warnings").and_then(|v| v.as_array()) {
                    Some(existing) => {
                        let mut merged = existing.clone();
                        merged.push(serde_json::Value::String(format!(
                            "{} Gmail message(s) omitted from LLM context to stay within context budget.",
                            omitted_messages
                        )));
                        serde_json::Value::Array(merged)
                    }
                    None => serde_json::Value::Array(vec![serde_json::Value::String(format!(
                        "{} Gmail message(s) omitted from LLM context to stay within context budget.",
                        omitted_messages
                    ))]),
                },
            );
        } else {
            compacted.remove("llm_omitted_message_count");
        }
        compacted.insert("count".into(), serde_json::json!(total));
    }

    serde_json::Value::Object(compacted)
}

fn compact_tool_result_for_llm(
    tool_name: &str,
    tool_result: &serde_json::Value,
) -> serde_json::Value {
    let is_gmail_tool = matches!(tool_name, "gw_gmail_inbox" | "gw_gmail_search");
    if !is_gmail_tool {
        return tool_result.clone();
    }

    if tool_result
        .get("provider")
        .and_then(|v| v.as_str())
        .map(|provider| provider.eq_ignore_ascii_case("google_workspace"))
        .unwrap_or(false)
    {
        let mut envelope = tool_result.clone();
        if let Some(env_obj) = envelope.as_object_mut() {
            env_obj.remove("raw_text");
            if let Some(payload) = env_obj.get_mut("data") {
                *payload = compact_gmail_payload_for_llm(payload);
            }
        }
        return envelope;
    }

    compact_gmail_payload_for_llm(tool_result)
}

/// Strict mode freshness pruning for search results.
/// Drops any result entry where the date field is older than max_age_days.
/// Handles both SearxNG format (`published_date`) and search_news format (`published`).
/// For live-fact queries, undated snippets are marked with a warning (not dropped)
/// to prevent context loss while still signaling uncertainty to the LLM.
fn prune_stale_search_results(mut val: serde_json::Value, max_age_days: i64) -> serde_json::Value {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(max_age_days);

    // Handle both SearxNG format (results array) and search_news format
    let results_path = if val.get("results").is_some() {
        "results"
    } else if val.get("articles").is_some() {
        "articles"
    } else {
        return val; // No recognized results array, return as-is
    };

    /// Extract the date from a result entry, checking both `published_date` (SearxNG)
    /// and `published` (search_news) field names.
    fn extract_date(r: &serde_json::Value) -> Option<chrono::DateTime<chrono::Utc>> {
        r.get("published_date")
            .or_else(|| r.get("published")) // search_news uses "published"
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
    }

    /// Check if a result entry has any date field at all.
    fn has_date_field(r: &serde_json::Value) -> bool {
        let pd = r.get("published_date").and_then(|v| v.as_str());
        let p = r.get("published").and_then(|v| v.as_str());
        pd.is_some() || p.is_some()
    }

    let (pruned_count, warned_count) = if let Some(results) = val[results_path].as_array_mut() {
        let _original_count = results.len();
        let mut pruned = 0;
        let mut warned = 0;

        for r in results.iter_mut() {
            let is_undated = !has_date_field(r);
            let is_stale = extract_date(r).map(|dt| dt < cutoff).unwrap_or(false);

            if is_stale {
                // Drop stale dated results
                pruned += 1;
            } else if is_undated {
                // Mark undated results with warning instead of dropping
                // This prevents context loss when SearxNG doesn't populate publishedDate
                if let Some(snippet) = r.get_mut("snippet") {
                    if let Some(snippet_str) = snippet.as_str() {
                        *snippet = serde_json::Value::String(format!(
                            "[WARNING: SOURCE DATE UNKNOWN - VERIFY FRESHNESS] {}",
                            snippet_str
                        ));
                        warned += 1;
                    }
                }
            }
        }

        // Actually remove stale results
        results.retain(|r| {
            extract_date(r).map(|dt| dt >= cutoff).unwrap_or(true) // Keep undated (they're now warned)
        });

        (pruned, warned)
    } else {
        (0, 0)
    };

    if pruned_count > 0 || warned_count > 0 {
        tracing::info!(
            pruned = pruned_count,
            warned = warned_count,
            max_age_days = max_age_days,
            "Layer 4 freshness pruning: dropped stale results, warned undated results"
        );
    }

    // Update count field if present (after mutable borrow is released)
    // Store the results length before borrowing val again
    let new_count = val[results_path].as_array().map(|arr| arr.len());
    if let Some(count) = val.get_mut("count") {
        if let Some(len) = new_count {
            *count = serde_json::Value::Number(serde_json::Number::from(len));
        }
    }

    val
}

fn extract_preprocessed_image_attachments(
    tool_data: &serde_json::Value,
    default_mime_type: &str,
) -> Option<Vec<ImageAttachment>> {
    let analysis = tool_data.get("analysis").unwrap_or(tool_data);

    let thumbnail_attachment = analysis
        .get("thumbnail_base64")
        .or_else(|| tool_data.get("thumbnail_base64"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|thumb_b64| ImageAttachment {
            data: thumb_b64.to_string(),
            mime_type: analysis
                .get("thumbnail_mime_type")
                .or_else(|| tool_data.get("thumbnail_mime_type"))
                .and_then(|v| v.as_str())
                .filter(|m| !m.trim().is_empty())
                .unwrap_or(default_mime_type)
                .to_string(),
        });

    if let Some(items) = analysis.get("selected_images").and_then(|v| v.as_array()) {
        let mut attachments = Vec::new();
        let mut has_global_frame = false;
        for item in items {
            let data = item
                .get("data_base64")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if data.is_empty() {
                continue;
            }

            let mime_type = item
                .get("mime_type")
                .and_then(|v| v.as_str())
                .filter(|m| !m.trim().is_empty())
                .unwrap_or(default_mime_type)
                .to_string();

            if item
                .get("kind")
                .and_then(|v| v.as_str())
                .map(|kind| kind.eq_ignore_ascii_case("global"))
                .unwrap_or(false)
            {
                has_global_frame = true;
            }

            attachments.push(ImageAttachment {
                data: data.to_string(),
                mime_type,
            });
        }

        if !has_global_frame {
            if let Some(thumb) = thumbnail_attachment.clone() {
                attachments.push(thumb);
            }
        }

        if !attachments.is_empty() {
            return Some(attachments);
        }
    }

    if let Some(thumb) = thumbnail_attachment {
        return Some(vec![thumb]);
    }

    None
}

// ─── Colab workflow state machine ────────────────────────────────────────────

/// What the user ultimately wants to do in Google Colab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColabIntent {
    /// Create a new .ipynb notebook (via Google Drive, then open in Colab).
    CreateNotebook,
    /// Open an existing notebook URL in Colab.
    OpenNotebook,
    /// Execute code in the currently active Colab notebook.
    ExecuteCode,
    /// General Colab request that needs the browser bridge but nothing specific.
    Generic,
}

/// Multi-step state machine that orchestrates the Colab workflow:
///   1. For CreateNotebook: drive_create → open_colab_browser_connection
///   2. For OpenNotebook / ExecuteCode / Generic: open_colab_browser_connection → (execute_cell)
#[derive(Debug, Clone)]
struct ColabFlowState {
    intent: ColabIntent,
    /// Notebook title supplied by the user (for CreateNotebook).
    notebook_title: Option<String>,
    /// Code supplied by the user (for ExecuteCode).
    code_snippet: Option<String>,
    /// Whether Drive file creation was attempted (CreateNotebook only).
    drive_create_attempted: bool,
    /// Whether Drive file creation succeeded and what the file ID is.
    drive_file_id: Option<String>,
    /// Whether open_colab_browser_connection has been called.
    browser_open_attempted: bool,
    /// Whether the browser session is confirmed connected.
    browser_connected: bool,
    /// Whether a code execute call has been dispatched.
    execute_attempted: bool,
}

impl ColabFlowState {
    fn from_user_text(text: &str) -> Option<Self> {
        let (intent, title, code) = detect_colab_intent(text)?;
        Some(Self {
            intent,
            notebook_title: title,
            code_snippet: code,
            drive_create_attempted: false,
            drive_file_id: None,
            browser_open_attempted: false,
            browser_connected: false,
            execute_attempted: false,
        })
    }

    /// Drive-create tool call for CreateNotebook flow.
    fn drive_create_call(&self) -> ParsedToolCall {
        let title = self
            .notebook_title
            .as_deref()
            .unwrap_or("Untitled Notebook");
        // gworkspace MCP creates a Google Doc; we use the same pattern but
        // flag it as an ipynb by appending the extension in the title.
        let full_title = if title.ends_with(".ipynb") {
            title.to_string()
        } else {
            format!("{}.ipynb", title)
        };
        ParsedToolCall {
            name: "gw_drive_create".into(),
            arguments: serde_json::json!({
                "title": full_title,
                "mime_type": "application/vnd.google.colab",
            }),
        }
    }

    /// Browser-connection bootstrap call.
    fn browser_open_call() -> ParsedToolCall {
        ParsedToolCall {
            name: "mcp_colab-mcp_open_colab_browser_connection".into(),
            arguments: serde_json::json!({}),
        }
    }

    /// Execute-cell call (only for ExecuteCode intent).
    fn execute_call(&self) -> Option<ParsedToolCall> {
        let code = self.code_snippet.as_deref()?;
        Some(ParsedToolCall {
            name: "mcp_colab-mcp_execute_cell".into(),
            arguments: serde_json::json!({ "code": code }),
        })
    }

    /// Returns the next forced calls for this workflow, if any.
    fn next_required_calls(
        &self,
        allowed_tool_names: &std::collections::HashSet<String>,
    ) -> Vec<ParsedToolCall> {
        // Step 1 (CreateNotebook only): create the Drive file first.
        if self.intent == ColabIntent::CreateNotebook && !self.drive_create_attempted {
            let call = self.drive_create_call();
            if allowed_tool_names.contains(&call.name) {
                return vec![call];
            }
            // Drive tool not available — fall through to browser open.
        }

        // Step 2: open the browser connection (once Drive file exists or not needed).
        if !self.browser_open_attempted {
            let call = Self::browser_open_call();
            if allowed_tool_names.contains(&call.name) {
                return vec![call];
            }
        }

        // Step 3 (ExecuteCode only): execute after browser is confirmed connected.
        if self.intent == ColabIntent::ExecuteCode
            && self.browser_connected
            && !self.execute_attempted
        {
            if let Some(call) = self.execute_call() {
                if allowed_tool_names.contains(&call.name) {
                    return vec![call];
                }
            }
        }

        vec![]
    }

    fn observe_tool_result(
        &mut self,
        call: &ParsedToolCall,
        success: bool,
        data: &serde_json::Value,
    ) {
        match call.name.as_str() {
            "gw_drive_create" => {
                self.drive_create_attempted = true;
                if success {
                    self.drive_file_id = data
                        .get("id")
                        .or_else(|| data.get("file_id"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
            }
            n if n.contains("open_colab_browser_connection") => {
                self.browser_open_attempted = true;
                // The tool returns {result: true/false}.
                let connected = data
                    .get("result")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(success);
                self.browser_connected = connected;
            }
            n if n.contains("execute_cell") => {
                self.execute_attempted = true;
            }
            _ => {}
        }
    }

    fn status_summary(&self) -> String {
        match self.intent {
            ColabIntent::CreateNotebook => {
                if self.browser_connected {
                    format!(
                        "Notebook '{}' created on Drive and opened in Colab.",
                        self.notebook_title.as_deref().unwrap_or("Untitled")
                    )
                } else if self.drive_create_attempted {
                    format!(
                        "Notebook '{}' created on Drive. Opening Colab browser...",
                        self.notebook_title.as_deref().unwrap_or("Untitled")
                    )
                } else {
                    "Creating notebook on Google Drive...".into()
                }
            }
            ColabIntent::OpenNotebook => {
                if self.browser_connected {
                    "Colab notebook opened in browser.".into()
                } else {
                    "Opening Colab browser connection...".into()
                }
            }
            ColabIntent::ExecuteCode => {
                if self.execute_attempted {
                    "Code dispatched to Colab.".into()
                } else if self.browser_connected {
                    "Browser connected. Executing code...".into()
                } else {
                    "Connecting to Colab browser...".into()
                }
            }
            ColabIntent::Generic => {
                if self.browser_connected {
                    "Colab browser connection established.".into()
                } else {
                    "Connecting to Colab browser...".into()
                }
            }
        }
    }
}

/// Detect whether the user text is a Colab-related request and classify its intent.
/// Returns `(ColabIntent, optional_title, optional_code)` or `None` if not Colab.
fn detect_colab_intent(text: &str) -> Option<(ColabIntent, Option<String>, Option<String>)> {
    let lower = text.to_ascii_lowercase();

    let is_colab = lower.contains("colab")
        || lower.contains("google colab")
        || (lower.contains("notebook")
            && (lower.contains("python") || lower.contains("jupyter") || lower.contains("ipynb")));

    if !is_colab {
        return None;
    }

    // Create intent
    let is_create = [
        "create",
        "new",
        "make",
        "start a",
        "open a new",
        "banao",
        "bana",
    ]
    .iter()
    .any(|kw| lower.contains(kw));

    if is_create {
        // Extract notebook title if present
        let title = infer_title(text, "")
            .pipe_nonempty()
            .or_else(|| extract_notebook_title_from_text(text));
        return Some((ColabIntent::CreateNotebook, title, None));
    }

    // Execute intent
    let is_execute = [
        "run", "execute", "chalao", "chala", "print(", "import ", "code:",
    ]
    .iter()
    .any(|kw| lower.contains(kw));

    if is_execute {
        let code = extract_code_from_text(text);
        return Some((ColabIntent::ExecuteCode, None, code));
    }

    // Open intent
    let is_open = [
        "open",
        "kholo",
        "kho do",
        "launch",
        "set as active",
        "active",
    ]
    .iter()
    .any(|kw| lower.contains(kw));

    if is_open {
        return Some((ColabIntent::OpenNotebook, None, None));
    }

    // Generic Colab request
    Some((ColabIntent::Generic, None, None))
}

/// Attempt to extract a notebook title from text like "named X" or "called X".
fn extract_notebook_title_from_text(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    for marker in ["named ", "called ", "name ", "title "] {
        if let Some(idx) = lower.find(marker) {
            let rest = text[idx + marker.len()..].trim();
            let title = rest
                .split(|c: char| {
                    matches!(c, ' ') && !rest[..rest.find(c).unwrap_or(0)].ends_with('.')
                })
                .next()
                .unwrap_or(rest)
                .trim_matches(|c: char| c == '"' || c == '\'' || c == '`' || c == '.')
                .trim();
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }

    // Try quoted text
    if let Some(caps) = QUOTED_TEXT_RE.captures(text) {
        if let Some(m) = caps.get(1).or_else(|| caps.get(2)) {
            let t = m.as_str().trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }

    None
}

/// Extract inline code from a user request (for execute intent).
fn extract_code_from_text(text: &str) -> Option<String> {
    // Fenced code block
    if let Some(caps) = FENCED_CODE_BLOCK_RE.captures(text) {
        if let Some(m) = caps.get(1) {
            let code = m.as_str().trim();
            if !code.is_empty() {
                return Some(code.to_string());
            }
        }
    }

    // Backtick inline
    if let Some(caps) = QUOTED_TEXT_RE.captures(text) {
        if let Some(m) = caps.get(1).or_else(|| caps.get(2)) {
            let code = m.as_str().trim();
            if code.contains('\n') || code.contains('(') {
                return Some(code.to_string());
            }
        }
    }

    // "run: ..." or "execute: ..."
    let lower = text.to_ascii_lowercase();
    for marker in ["run:", "execute:", "code:"] {
        if let Some(idx) = lower.find(marker) {
            let rest = text[idx + marker.len()..].trim();
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }

    None
}

/// Helper: turn a `String` into `Option<String>`, returning `None` if empty.
trait PipeNonEmpty {
    fn pipe_nonempty(self) -> Option<String>;
}
impl PipeNonEmpty for String {
    fn pipe_nonempty(self) -> Option<String> {
        if self.is_empty() {
            None
        } else {
            Some(self)
        }
    }
}

#[derive(Debug, Clone)]
struct PackageFlowState {
    intent: PackageIntent,
    query: String,
    package_name: String,
    search_done: bool,
    search_found: Option<bool>,
    search_preferred_source: Option<String>,
    precheck_done: bool,
    precheck_installed: Option<bool>,
    precheck_source: Option<String>,
    action_attempted: bool,
    action_success: Option<bool>,
    postcheck_done: bool,
    postcheck_installed: Option<bool>,
}

/// Deterministic MARKETPLACE capability flow — the CPP analogue of
/// [`PackageFlowState`], but for KRIA skills/tools/capabilities instead of OS
/// software packages. When a small local model hesitates (asks "local or VM?"
/// instead of acting) on a clear "install/search a tool/skill" request, this
/// forces the correct provider-neutral marketplace tool. A single forced call is
/// enough: `install_capability` internally searches the marketplace and installs
/// the best match; `search_marketplace` returns ranked candidates. No per-prompt
/// special casing — it triggers on generic capability nouns + install/search verbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CapabilityFlowIntent {
    Search,
    Install,
}

struct CapabilityFlowState {
    intent: CapabilityFlowIntent,
    query: String,
    done: bool,
}

impl CapabilityFlowState {
    fn from_user_text(user_text: &str) -> Option<Self> {
        if is_remote_command_context(user_text) {
            return None;
        }
        // Only for requests that clearly concern a KRIA capability/skill/tool
        // (not OS packages, not arbitrary chat).
        if !refers_to_marketplace_capability(user_text) {
            return None;
        }
        let lower = user_text.to_lowercase();
        let install = [
            "install", "add ", "get me", "download", "set up", "setup", "enable ",
        ]
        .iter()
        .any(|m| lower.contains(m));
        let search = [
            "search", "find", "look for", "browse", "discover", "is there", "any ",
        ]
        .iter()
        .any(|m| lower.contains(m));
        let intent = if install {
            CapabilityFlowIntent::Install
        } else if search {
            CapabilityFlowIntent::Search
        } else {
            return None;
        };
        let query = extract_capability_query(user_text)?;
        Some(Self {
            intent,
            query,
            done: false,
        })
    }

    fn tool_name(&self) -> &'static str {
        match self.intent {
            CapabilityFlowIntent::Search => "search_marketplace",
            CapabilityFlowIntent::Install => "install_capability",
        }
    }

    fn next_required_calls(
        &self,
        allowed_tool_names: &std::collections::HashSet<String>,
    ) -> Vec<ParsedToolCall> {
        if self.done {
            return vec![];
        }
        let name = self.tool_name();
        if !allowed_tool_names.contains(name) {
            return vec![];
        }
        vec![ParsedToolCall {
            name: name.to_string(),
            arguments: serde_json::json!({ "query": self.query }),
        }]
    }

    fn observe_tool_result(&mut self, call: &ParsedToolCall) {
        if call.name == self.tool_name() {
            self.done = true;
        }
    }

    fn status_summary(&self) -> String {
        match self.intent {
            CapabilityFlowIntent::Search => "Searching the capability marketplace".to_string(),
            CapabilityFlowIntent::Install => {
                "Installing the best-matching capability from the marketplace".to_string()
            }
        }
    }
}

impl PackageFlowState {
    fn from_user_text(user_text: &str) -> Option<Self> {
        let intent = detect_package_intent(user_text)?;
        let query = extract_package_query(user_text, intent)?;
        let package_name = query.split_whitespace().next()?.to_string();
        Some(Self {
            intent,
            query,
            package_name,
            search_done: false,
            search_found: None,
            search_preferred_source: None,
            precheck_done: false,
            precheck_installed: None,
            precheck_source: None,
            action_attempted: false,
            action_success: None,
            postcheck_done: false,
            postcheck_installed: None,
        })
    }

    fn action_tool_name(&self) -> &'static str {
        match self.intent {
            PackageIntent::Install => "install_package",
            PackageIntent::Uninstall => "uninstall_package",
        }
    }

    fn check_call(&self) -> ParsedToolCall {
        ParsedToolCall {
            name: "check_package_installed".into(),
            arguments: serde_json::json!({ "name": self.package_name }),
        }
    }

    fn action_call(&self) -> ParsedToolCall {
        let mut arguments = serde_json::json!({ "name": self.package_name });
        if let Some(source) = self.source_for_action() {
            arguments["source"] = serde_json::Value::String(source);
        }
        ParsedToolCall {
            name: self.action_tool_name().into(),
            arguments,
        }
    }

    fn search_call(&self) -> ParsedToolCall {
        ParsedToolCall {
            name: "search_package".into(),
            arguments: serde_json::json!({ "query": self.query }),
        }
    }

    fn should_take_action(&self) -> Option<bool> {
        match self.intent {
            PackageIntent::Install => self.precheck_installed.map(|installed| !installed),
            PackageIntent::Uninstall => self.precheck_installed,
        }
    }

    fn source_for_action(&self) -> Option<String> {
        match self.intent {
            PackageIntent::Install => self
                .search_preferred_source
                .clone()
                .or_else(|| self.precheck_source.clone()),
            PackageIntent::Uninstall => self.precheck_source.clone(),
        }
    }

    fn next_required_calls(&self) -> Vec<ParsedToolCall> {
        if matches!(self.intent, PackageIntent::Install) {
            if !self.search_done {
                return vec![self.search_call()];
            }
            // If the package was not found during search, stop forcing actions.
            if matches!(self.search_found, Some(false)) {
                return vec![];
            }
            // If search failed and we have no reliable result, avoid loops.
            if self.search_found.is_none() {
                return vec![];
            }
        }

        if !self.precheck_done {
            return vec![self.check_call()];
        }
        // If precheck failed and we have no reliable installed flag, avoid loops.
        if self.precheck_installed.is_none() {
            return vec![];
        }

        match self.intent {
            PackageIntent::Install => {
                if matches!(self.should_take_action(), Some(true)) {
                    if !self.action_attempted {
                        return vec![self.action_call()];
                    }
                    // Always re-check after an install attempt.
                    if !self.postcheck_done {
                        return vec![self.check_call()];
                    }
                }
            }
            PackageIntent::Uninstall => {
                if matches!(self.precheck_installed, Some(false)) {
                    return vec![];
                }
                if !self.action_attempted {
                    return vec![self.action_call()];
                }
                // Always re-check after each uninstall attempt.
                if !self.postcheck_done {
                    return vec![self.check_call()];
                }
                // If still installed, try uninstalling again using the latest observed source.
                if matches!(self.postcheck_installed, Some(true)) {
                    return vec![self.action_call()];
                }
            }
        }

        vec![]
    }

    fn observe_tool_result(
        &mut self,
        call: &ParsedToolCall,
        success: bool,
        data: &serde_json::Value,
    ) {
        match call.name.as_str() {
            "search_package" => {
                self.search_done = true;
                self.search_found = data
                    .get("count")
                    .and_then(|v| v.as_u64())
                    .map(|count| count > 0);
                self.search_preferred_source = data
                    .get("results")
                    .and_then(|v| v.as_array())
                    .and_then(|results| {
                        let target = self.package_name.to_lowercase();
                        results
                            .iter()
                            .find(|row| {
                                row.get("name")
                                    .and_then(|v| v.as_str())
                                    .map(|name| {
                                        let n = name.to_lowercase();
                                        n == target
                                            || n.starts_with(&(target.clone() + "-"))
                                            || n.contains(&target)
                                    })
                                    .unwrap_or(false)
                            })
                            .or_else(|| results.first())
                    })
                    .and_then(|row| row.get("source"))
                    .and_then(|v| v.as_str())
                    .and_then(normalize_package_source_for_action);
            }
            "check_package_installed" => {
                let installed = data.get("installed").and_then(|v| v.as_bool());
                let source = data
                    .get("source")
                    .and_then(|v| v.as_str())
                    .and_then(normalize_package_source_for_action);
                if !self.precheck_done {
                    self.precheck_done = true;
                    self.precheck_installed = installed;
                    self.precheck_source = source;
                } else if self.action_attempted {
                    self.postcheck_done = true;
                    self.postcheck_installed = installed;
                    self.precheck_source = source.or_else(|| self.precheck_source.clone());
                } else {
                    // A repeated pre-check still refreshes observed state.
                    self.precheck_installed = installed;
                    self.precheck_source = source.or_else(|| self.precheck_source.clone());
                }
            }
            "install_package" if matches!(self.intent, PackageIntent::Install) => {
                self.action_attempted = true;
                self.action_success = Some(success);
                self.postcheck_done = false;
                self.postcheck_installed = None;
            }
            "uninstall_package" if matches!(self.intent, PackageIntent::Uninstall) => {
                self.action_attempted = true;
                self.action_success = Some(success);
                self.postcheck_done = false;
                self.postcheck_installed = None;
            }
            _ => {}
        }
    }

    fn verified_summary(&self) -> Option<String> {
        match self.intent {
            PackageIntent::Install => {
                if matches!(self.precheck_installed, Some(true)) {
                    return Some(format!(
                        "Verified: '{}' is already installed.",
                        self.package_name
                    ));
                }
                if !self.action_attempted || !self.postcheck_done {
                    return None;
                }
                match self.postcheck_installed {
                    Some(true) => Some(format!(
                        "Verified: '{}' is installed after the install attempt.",
                        self.package_name
                    )),
                    Some(false) => Some(format!(
                        "Verification result: '{}' is still not installed after the install attempt.",
                        self.package_name
                    )),
                    None => Some(format!(
                        "Install attempt completed for '{}', but final verification could not determine installed state.",
                        self.package_name
                    )),
                }
            }
            PackageIntent::Uninstall => {
                if matches!(self.precheck_installed, Some(false)) {
                    return Some(format!(
                        "Verified: '{}' is not installed.",
                        self.package_name
                    ));
                }
                if !self.action_attempted || !self.postcheck_done {
                    return None;
                }
                match self.postcheck_installed {
                    Some(false) => Some(format!(
                        "Verified: '{}' is not installed after the uninstall attempt.",
                        self.package_name
                    )),
                    Some(true) => Some(format!(
                        "Verification result: '{}' is still installed after the uninstall attempt.",
                        self.package_name
                    )),
                    None => Some(format!(
                        "Uninstall attempt completed for '{}', but final verification could not determine installed state.",
                        self.package_name
                    )),
                }
            }
        }
    }
}

/// A single recovery action the user can take from the UI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RecoveryOption {
    /// Short button label (e.g. "Connect VM", "Retry", "Install Docker")
    pub label: String,
    /// The message that will be sent to the agent when the user clicks this button
    pub action_prompt: String,
    /// Visual style hint for the UI: "primary" | "secondary" | "danger"
    pub style: &'static str,
}

/// A step in a multi-step task execution plan.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskStep {
    /// Step index (1-based)
    pub index: u32,
    /// Total steps in the plan (None if unknown)
    pub total: Option<u32>,
    /// Short description of this step
    pub description: String,
    /// Step status
    pub status: TaskStepStatus,
}

#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStepStatus {
    /// Step is about to start
    Starting,
    /// Step is in progress
    Running,
    /// Step completed successfully
    Done,
    /// Step failed
    Failed,
    /// Step was skipped
    Skipped,
}

/// Events emitted during agent loop execution.
#[derive(Debug, Clone, serde::Serialize)]
pub enum StreamEvent {
    /// Marks the admitted turn identity for this stream.
    TurnAccepted { session_id: String, turn_id: String },
    /// Text token from the LLM.
    Token(String),
    /// Tool is being called.
    ToolStart {
        name: String,
        params: serde_json::Value,
    },
    /// Tool completed.
    ToolEnd {
        name: String,
        result: serde_json::Value,
        success: bool,
        /// Full human-readable markdown response (tables, lists, paragraphs)
        #[serde(skip_serializing_if = "Option::is_none")]
        human_readable: Option<String>,
        /// One-line conversational summary (for collapsed UI badge)
        #[serde(skip_serializing_if = "Option::is_none")]
        conversational_summary: Option<String>,
        /// Execution metadata (optional, for status/metrics display)
        #[serde(skip_serializing_if = "Option::is_none")]
        execution_metadata: Option<serde_json::Value>,
    },
    /// Structured recovery options emitted when a prerequisite check fails.
    /// The UI renders these as clickable action buttons so the user can
    /// resolve the issue without typing a follow-up message.
    RecoveryOptions {
        /// Short description of what failed (e.g. "VM not reachable")
        context: String,
        /// Diagnostic detail (e.g. "SSH Timeout connecting to vm1")
        detail: String,
        /// Ordered list of recovery actions the user can take
        options: Vec<RecoveryOption>,
    },
    /// A step in a multi-step task execution plan.
    /// Emitted before and after each significant step so the UI can show
    /// live progress (e.g. "Step 1/3: Checking VM connectivity").
    TaskStep(TaskStep),
    /// Mid-execution heartbeat / progress update from a long-running tool.
    /// `call_id` matches the `name` field of the surrounding `ToolStart`/`ToolEnd`.
    /// `percent` is `None` when progress is indeterminate.
    ToolProgress {
        call_id: String,
        message: String,
        percent: Option<u8>,
    },
    /// A chunk of the **full** MCP payload streamed directly to the UI.
    /// The LLM only ever sees the compact summary; the UI can render full data
    /// by reassembling these chunks.
    ToolPayloadChunk {
        call_id: String,
        seq: u32,
        is_final: bool,
        data: serde_json::Value,
    },
    /// Waiting for HITL approval.
    ApprovalRequired {
        request_id: String,
        action: String,
        risk_level: String,
        parameters: serde_json::Value,
    },
    /// Approval result.
    ApprovalResult { action: String, approved: bool },
    /// Tool choice confirmation required for low-confidence routing.
    ToolChoiceRequired {
        query: String,
        confidence: f32,
        min_confidence: f32,
        candidates: Vec<ToolChoiceCandidate>,
    },
    /// Planning step.
    Plan(String),
    /// Error.
    Error(String),
    /// Final response text.
    Done(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnExecutionMode {
    Assistant,
    PromptLab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptLabToolSelectionStrategy {
    DirectLockedTool,
    RoutedWithinLock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnExecutionProfile {
    pub mode: TurnExecutionMode,
    pub app_lock: Option<String>,
    pub tool_lock: Option<String>,
    pub prompt_lab_strategy: PromptLabToolSelectionStrategy,
}

impl TurnExecutionProfile {
    pub fn assistant() -> Self {
        Self::default()
    }

    pub fn manual_tool(
        app_lock: Option<String>,
        tool_lock: Option<String>,
        prompt_lab_strategy: PromptLabToolSelectionStrategy,
    ) -> Self {
        Self {
            mode: TurnExecutionMode::Assistant,
            app_lock: app_lock
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| !value.is_empty()),
            tool_lock: tool_lock
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            prompt_lab_strategy,
        }
    }

    pub fn prompt_lab(
        app_lock: Option<String>,
        tool_lock: Option<String>,
        prompt_lab_strategy: PromptLabToolSelectionStrategy,
    ) -> Self {
        Self {
            mode: TurnExecutionMode::PromptLab,
            app_lock: app_lock
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| !value.is_empty()),
            tool_lock: tool_lock
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            prompt_lab_strategy,
        }
    }

    fn is_prompt_lab(&self) -> bool {
        matches!(self.mode, TurnExecutionMode::PromptLab)
    }

    pub fn is_manual_tool_override(&self) -> bool {
        matches!(self.mode, TurnExecutionMode::Assistant)
            && (self.app_lock.is_some() || self.tool_lock.is_some())
    }

    pub fn is_gui_cognition_override(&self) -> bool {
        if !self.is_manual_tool_override() {
            return false;
        }

        self.app_lock
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase())
            .is_some_and(|value| {
                matches!(
                    value.as_str(),
                    "gui" | "gui_cognition" | "gui-cognition" | "desktop_gui"
                )
            })
    }

    fn uses_direct_strategy(&self) -> bool {
        (self.is_prompt_lab() || self.is_manual_tool_override())
            && matches!(
                self.prompt_lab_strategy,
                PromptLabToolSelectionStrategy::DirectLockedTool
            )
    }

    pub fn allows_tool_name(&self, tool_name: &str) -> bool {
        if self.tool_lock.is_none() && self.app_lock.is_none() {
            return true;
        }

        if let Some(tool_lock) = self.tool_lock.as_deref() {
            return tool_name == tool_lock;
        }

        if let Some(app_lock) = self.app_lock.as_deref() {
            return tool_matches_lab_app_lock(tool_name, app_lock);
        }

        true
    }
}

impl Default for TurnExecutionProfile {
    fn default() -> Self {
        Self {
            mode: TurnExecutionMode::Assistant,
            app_lock: None,
            tool_lock: None,
            prompt_lab_strategy: PromptLabToolSelectionStrategy::RoutedWithinLock,
        }
    }
}

fn tool_matches_lab_app_lock(tool_name: &str, app_lock: &str) -> bool {
    let lower = app_lock.to_ascii_lowercase();
    let tool_name_lower = tool_name.to_ascii_lowercase();

    match lower.as_str() {
        "gmail" => tool_name_lower.starts_with("gw_gmail_"),
        "drive" => tool_name_lower.starts_with("gw_drive_"),
        "docs" => tool_name_lower.starts_with("gw_docs_"),
        "sheets" => tool_name_lower.starts_with("gw_sheets_"),
        "calendar" => tool_name_lower.starts_with("gw_calendar_"),
        "slides" => tool_name_lower.starts_with("gw_slides_"),
        "forms" => tool_name_lower.starts_with("gw_forms_"),
        "google" | "gworkspace" | "google_workspace" => tool_name_lower.starts_with("gw_"),
        "n8n" => tool_name_lower == "n8n_invoke_workflow",
        // A6: OpenClaw is a single semantic tool `"openclaw"` (+ introspection
        // `"list_installed_skills"`), NOT per-skill `oc_*` tools anymore. The old
        // `starts_with("oc_")`-only rule blocked the only tools that can satisfy an
        // OpenClaw request when the user locks Tool Mode to "OpenClaw". Keep `oc_*`
        // for backward compat with any legacy per-skill registration.
        "openclaw" | "claw" => {
            tool_name_lower == "openclaw"
                || tool_name_lower == "list_installed_skills"
                || tool_name_lower.starts_with("oc_")
        }
        "gui" | "gui_cognition" | "gui-cognition" | "desktop_gui" => {
            matches!(
                tool_name_lower.as_str(),
                "open_application"
                    | "open_application_with_file"
                    | "get_active_window"
                    | "list_windows"
                    | "move_window"
                    | "resize_window"
                    | "maximize_window"
                    | "minimize_window"
                    | "tile_windows"
                    | "click_mouse"
                    | "type_text"
                    | "press_shortcut"
                    | "release_all"
                    | "focus_window"
                    | "system_sleep"
                    | "click_ui_element"
                    | "fill_form_field"
                    | "detect_dialog"
                    | "dismiss_dialog"
                    | "get_desktop_state"
                    | "check_app_responding"
                    | "find_ui_elements"
                    | "get_accessibility_capabilities"
                    | "accessibility_doctor"
            )
        }
        "image" | "image_generation" | "image-generation" => tool_name_lower == "generate_image",
        "github" => {
            tool_name_lower.starts_with("git_")
                || (tool_name_lower.starts_with("mcp_") && tool_name_lower.contains("github"))
        }
        "filesystem" | "file_system" | "files" => matches!(
            tool_name_lower.as_str(),
            "read_file"
                | "search_files"
                | "list_directory"
                | "get_file_info"
                | "calculate_dir_size"
                | "write_file"
                | "create_directory"
                | "rename_file"
                | "copy_file"
                | "delete_file"
                | "delete_directory"
                | "move_file"
                | "search_file_contents"
                | "find_files_by_pattern"
                | "get_project_structure"
                | "count_lines_of_code"
                | "diff_files"
                | "find_todos"
                | "analyze_code"
        ),
        "docker" => {
            tool_name_lower.contains("docker")
                // A6: OpenClaw skills run in Docker containers, so "docker" mode must
                // reach the semantic OpenClaw tool + introspection (same fix as the
                // "openclaw" arm), not just the legacy `oc_*` per-skill names.
                || tool_name_lower == "openclaw"
                || tool_name_lower == "list_installed_skills"
                || tool_name_lower.starts_with("oc_")
                || tool_name_lower == "n8n_invoke_workflow"
        }
        "browser" => matches!(
            tool_name_lower.as_str(),
            "browser_search"
                | "open_url"
                | "managed_browser_navigate"
                | "web_search"
                | "searxng_search"
                | "fetch_webpage"
                | "check_url_status"
                | "get_news"
        ),
        "slack" => {
            tool_name_lower.contains("slack")
                || tool_name_lower == "n8n_invoke_workflow"
                || tool_name_lower == "send_message"
        }
        "colab" | "google_colab" | "notebook" => {
            tool_name_lower.starts_with("mcp_") && tool_name_lower.contains("colab")
        }
        _ => {
            if let Some(prefix) = lower.strip_prefix("mcp_") {
                tool_name_lower.starts_with(&format!("mcp_{}", prefix))
            } else {
                false
            }
        }
    }
}

fn tool_allowed_by_execution_profile(profile: &TurnExecutionProfile, tool_name: &str) -> bool {
    profile.allows_tool_name(tool_name)
}

/// Deterministic 128-bit FNV-1a-style hash over a session string → stable UUID
/// for the cognitive `MemorySystem` (which keys sessions by UUID). No `uuid` v5
/// feature dependency; same session string → same memory session.
fn stable_session_uuid(session_id: &str) -> uuid::Uuid {
    let mut hi: u64 = 0xcbf2_9ce4_8422_2325;
    let mut lo: u64 = 0x8422_2325_cbf2_9ce4;
    for b in session_id.as_bytes() {
        hi ^= *b as u64;
        hi = hi.wrapping_mul(0x100_0000_01b3);
        lo = lo.rotate_left(7) ^ (*b as u64);
        lo = lo.wrapping_mul(0x100_0000_01b3);
    }
    uuid::Uuid::from_u128(((hi as u128) << 64) | lo as u128)
}

#[cfg(test)]
mod stable_session_uuid_tests {
    use super::stable_session_uuid;

    #[test]
    fn deterministic_and_distinct() {
        // Determinism: same session string → same memory session UUID across
        // calls/restarts (must match the desktop `memory_session_uuid` gate).
        let a1 = stable_session_uuid("session-abc");
        let a2 = stable_session_uuid("session-abc");
        assert_eq!(a1, a2);
        // Distinct sessions map to distinct UUIDs.
        assert_ne!(a1, stable_session_uuid("session-xyz"));
        // Non-nil.
        assert_ne!(a1, uuid::Uuid::nil());
    }
}

/// Grounding retrieved for a turn: the injected block, the contributing memory
/// ids (for worth credit), and the retrieval class + winning strategy (for
/// adaptive-RRF reinforcement on turn success).
pub struct MemoryGrounding {
    pub block: String,
    pub memory_ids: Vec<uuid::Uuid>,
    pub query_class: crate::memory::retriever::QueryClass,
    pub top_strategy: Option<crate::memory::retrieval_opt::Strategy>,
}

/// The core ReAct agent loop.
pub struct AgentLoop {
    model_router: Arc<ModelRouter>,
    tool_registry: Arc<ToolRegistry>,
    mount_manager: Arc<tokio::sync::RwLock<ToolMountManager>>,
    policy_engine: Arc<PolicyEngine>,
    hitl_gateway: Arc<HitlGateway>,
    audit_logger: Arc<AuditLogger>,
    #[allow(dead_code)]
    rollback_mgr: Arc<RollbackManager>,
    /// Semantic router — None until initialised (falls back to regex router).
    semantic_router: Option<Arc<crate::routing::Router>>,
    /// Tool-level semantic index for direct execution fast path.
    tool_index: Option<Arc<crate::routing::tool_index::SharedToolIndex>>,
    /// Feedback collector for online learning.
    feedback_collector:
        Option<Arc<tokio::sync::Mutex<crate::routing::feedback::FeedbackCollector>>>,
    /// Session vector store for document RAG context injection.
    pub doc_store: Option<Arc<crate::preprocessing::SessionVectorStore>>,
    max_tool_rounds: usize,
    hardware_tier: String,
    min_confidence_to_act: f32,
    clarify_threshold: f32,
    /// Per-session admission gate with supersession-aware cancellation.
    turn_admission: Arc<TurnAdmission>,
    /// Top-level planning boundary (Phase 3 scaffold).
    turn_gate: Arc<TurnGate>,
    /// Optional failover router — when present, wraps model_router with FSM-based
    /// provider failover. When absent, model_router is used directly (existing behavior).
    failover_router: Option<Arc<crate::llm::failover::FailoverRouter>>,
    /// Optional execution verifier — when present, validates tool results after execution.
    /// When absent, tool results are accepted as-is (existing behavior).
    execution_verifier: Option<Arc<dyn crate::agent::execution_verifier::ExecutionVerifier>>,
    /// Result synthesizer — transforms raw tool outputs into intelligent responses.
    result_synthesizer: ResultSynthesizer,
    /// PSDG handle — when present, injects semantic desktop context into system prompts
    /// for automation/shell/IDE-relevant turns. Fire-and-forget writes only.
    world_model: Option<crate::agent::psdg::PsdgHandle>,
    /// Optional LLM-powered intent compiler for complex GUI intents.
    ///
    /// `RuleIntentCompiler` handles common patterns (<5ms, no LLM).
    /// When set, this compiler is tried as a fallback for `Verb::Other` results
    /// that the rule compiler cannot classify, enabling semantic normalization of
    /// complex multi-step GUI automation intents.
    intent_compiler: Option<Arc<dyn crate::agent::intent_compiler::IntentCompiler>>,
    /// Optional session manager for ReAct workflow checkpoint persistence.
    /// When set, each tool execution round is checkpointed to disk.
    session_manager: Option<Arc<crate::agent::workflow_session::SessionManager>>,
    /// Optional health registry for runtime observability event counting.
    health_registry: Option<Arc<crate::infra::health::HealthRegistry>>,
    /// Optional execution transparency layer for per-turn ReAct lineage tracing.
    /// When set, each ReAct turn creates a trace and each tool execution is recorded.
    transparency_layer: Option<crate::agent::execution_transparency::ExecutionTransparencyLayer>,
    /// Optional observable completion engine — verifies human-visible outcomes after
    /// the ReAct tool loop finishes. Infers outcomes from user intent; skipped for
    /// Silent/Converse turns. PSDG fast-path keeps most checks under 10ms.
    observable_completion:
        Option<std::sync::Arc<crate::agent::observable_completion::ObservableCompletionEngine>>,
    /// Optional workflow expectation engine — classifies workflow category at turn
    /// start using keyword + operation heuristics for semantic context alignment.
    workflow_expectation:
        Option<std::sync::Arc<crate::agent::workflow_expectation::WorkflowExpectationEngine>>,
    /// Optional collaborative autonomy engine — per-turn autonomy decision; surfaces
    /// advisory notices for novel or low-confidence operations before tool loop starts.
    collaborative_autonomy:
        Option<std::sync::Arc<crate::agent::collaborative_autonomy::CollaborativeAutonomyEngine>>,
    /// Optional workflow continuation runtime — classifies tool failures as interruptions
    /// and plans bounded recovery. Records blockers in the transparency layer.
    continuation_runtime:
        Option<std::sync::Arc<crate::agent::workflow_continuation::WorkflowContinuationRuntime>>,
    // ── Batch 3: Persistent Operational Desktop Cognition Runtime ────────────
    /// Optional cognition event bus — emits typed operational events to subscribers.
    cognition_bus: Option<std::sync::Arc<crate::agent::cognition_event_bus::CognitionEventBus>>,
    /// Optional operational context tracker — maintains bounded workflow history.
    operational_context:
        Option<std::sync::Arc<crate::agent::operational_context::OperationalContextTracker>>,
    /// Optional procedural workflow memory — extracts and stores skill patterns.
    procedural_memory:
        Option<std::sync::Arc<crate::agent::procedural_memory::ProceduralWorkflowMemory>>,
    /// Optional persistent goal runtime — goals that survive restarts.
    goal_runtime: Option<std::sync::Arc<crate::agent::goal_runtime::PersistentGoalRuntime>>,
    /// Optional operational suggestions engine — rate-limited proactive suggestions.
    suggestions_engine:
        Option<std::sync::Arc<crate::agent::operational_suggestions::OperationalSuggestionsEngine>>,
    /// Optional desktop awareness runtime — unified live operational state.
    desktop_awareness:
        Option<std::sync::Arc<crate::agent::desktop_awareness::DesktopAwarenessRuntime>>,
    /// Optional unified cognitive memory backbone. When attached, the loop
    /// grounds reasoning with retrieved long-term memory and records turn/tool
    /// outcomes through the Write Policy — making every entry point (desktop,
    /// server, telegram) memory-driven without per-caller wiring.
    memory_system: Option<std::sync::Arc<crate::memory::api::MemorySystem>>,
}

impl AgentLoop {
    pub fn new(
        model_router: Arc<ModelRouter>,
        tool_registry: Arc<ToolRegistry>,
        mount_manager: Arc<tokio::sync::RwLock<ToolMountManager>>,
        policy_engine: Arc<PolicyEngine>,
        hitl_gateway: Arc<HitlGateway>,
        audit_logger: Arc<AuditLogger>,
        rollback_mgr: Arc<RollbackManager>,
    ) -> Self {
        Self {
            model_router,
            tool_registry,
            mount_manager,
            policy_engine,
            hitl_gateway,
            audit_logger,
            rollback_mgr,
            semantic_router: None,
            tool_index: None,
            feedback_collector: None,
            doc_store: None,
            max_tool_rounds: 10,
            hardware_tier: "standard".into(),
            min_confidence_to_act: 0.55,
            clarify_threshold: 0.40,
            turn_admission: Arc::new(TurnAdmission::new()),
            turn_gate: Arc::new(TurnGate::new()),
            failover_router: None,
            execution_verifier: None,
            result_synthesizer: ResultSynthesizer::default(),
            world_model: None,
            intent_compiler: None,
            session_manager: None,
            health_registry: None,
            transparency_layer: None,
            observable_completion: None,
            workflow_expectation: None,
            collaborative_autonomy: None,
            continuation_runtime: None,
            cognition_bus: None,
            operational_context: None,
            procedural_memory: None,
            goal_runtime: None,
            suggestions_engine: None,
            desktop_awareness: None,
            memory_system: None,
        }
    }

    /// Attach the unified cognitive [`MemorySystem`](crate::memory::api::MemorySystem).
    pub fn with_memory_system(
        mut self,
        memory_system: std::sync::Arc<crate::memory::api::MemorySystem>,
    ) -> Self {
        self.memory_system = Some(memory_system);
        self
    }

    /// Retrieve relevant long-term memory for `query` as a grounding block to
    /// inject into the LLM context (design §10 read surface), returning the
    /// formatted block plus the ids of the contributing memories (for turn-end
    /// Memory-Worth credit assignment). Returns `None` when no memory system is
    /// attached, nothing relevant is found, or retrieval degrades (L8).
    /// Best-effort — never blocks the turn.
    /// Observe the user's message into cognitive memory through the Write Policy
    /// (event → derived memory → retrieval + background cognition), so EVERY
    /// host — desktop, server, Telegram, WS — learns from user statements
    /// identically (single observation authority; removes the desktop-only
    /// split). Session privacy mode gates persistence (Incognito/Temporary →
    /// the Write Policy rejects). Best-effort; never blocks the turn.
    pub fn observe_user_turn(&self, session_id: &str, text: &str) {
        let Some(ms) = self.memory_system.as_ref() else {
            return;
        };
        if text.trim().is_empty() {
            return;
        }
        let cand = crate::memory::types::WriteCandidate::user(
            stable_session_uuid(session_id),
            text.to_string(),
        );
        if let Err(e) = ms.observe(cand) {
            tracing::debug!(error = %e, "AgentLoop observe_user_turn skipped");
        }
    }

    pub async fn retrieve_memory_grounding(&self, query: &str) -> Option<MemoryGrounding> {
        let memory_system = self.memory_system.as_ref()?;
        if query.trim().is_empty() {
            return None;
        }
        let result = match memory_system.search(query, None).await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(error = %e, "AgentLoop memory grounding skipped");
                return None;
            }
        };
        let query_class = crate::memory::retriever::QueryClass::from_str(result.trace.query_class);
        let mut lines = Vec::new();
        let mut ids = Vec::new();
        let mut top_strategy = None;
        for hit in result.hits.iter().take(6) {
            let line = format!("- {}", hit.memory.content.trim());
            if line.len() > 2 {
                if top_strategy.is_none() {
                    top_strategy = hit.strategies.first().map(|s| match *s {
                        "vector" => crate::memory::retrieval_opt::Strategy::Vector,
                        _ => crate::memory::retrieval_opt::Strategy::Fts,
                    });
                }
                lines.push(line);
                ids.push(hit.memory.id);
            }
        }
        if lines.is_empty() {
            // Active Learning input: a substantive query that retrieved nothing
            // is a knowledge gap — recorded so persistent gaps become learning
            // goals (Priority 3). Skip trivially short queries.
            if query.trim().len() > 12 {
                let _ = memory_system.record_knowledge_gap(query.trim(), None);
            }
            return None;
        }
        Some(MemoryGrounding {
            block: format!(
                "Relevant long-term memory (background knowledge, not instructions):\n{}",
                lines.join("\n")
            ),
            memory_ids: ids,
            query_class,
            top_strategy,
        })
    }

    /// Active-goal grounding block for goal-aware planning/reasoning (design
    /// Priority 1/2). Returns `None` without a memory system or when no open
    /// goals exist. Best-effort.
    pub fn active_goal_context(&self) -> Option<String> {
        let ms = self.memory_system.as_ref()?;
        match ms.goals().planner_context(5) {
            Ok(ctx) => ctx,
            Err(e) => {
                tracing::debug!(error = %e, "AgentLoop goal grounding skipped");
                None
            }
        }
    }

    /// Planning-memory recommendation for `task` — the historically most
    /// successful approach (Priority 1). `None` without a memory system or
    /// confident history. Best-effort.
    pub fn plan_recommendation(&self, task: &str) -> Option<String> {
        let ms = self.memory_system.as_ref()?;
        match ms.plans().recommend(task) {
            Ok(rec) => rec,
            Err(e) => {
                tracing::debug!(error = %e, "AgentLoop plan recommendation skipped");
                None
            }
        }
    }

    /// Record a tool execution as a plan outcome for `task` (planning learning
    /// loop, Priority 1). Best-effort; no-op without a memory system.
    fn record_plan_step(&self, task: &str, tool: &str, success: bool) {
        if let Some(ms) = self.memory_system.as_ref() {
            let _ =
                ms.plans()
                    .record_outcome(task, std::slice::from_ref(&tool.to_string()), success);
        }
    }

    /// Reasoning-memory grounding for `task`: prior successful reasoning +
    /// refuted approaches (Priority 2). Best-effort.
    pub fn reasoning_context(&self, task: &str) -> Option<String> {
        let ms = self.memory_system.as_ref()?;
        match ms.reasoning().reasoning_context(task, 3) {
            Ok(ctx) => ctx,
            Err(e) => {
                tracing::debug!(error = %e, "AgentLoop reasoning grounding skipped");
                None
            }
        }
    }

    /// Record a reasoning trace for a turn (Reasoning Memory, Priority 2).
    /// Success → a positive-outcome reasoning chain; failure → a counterexample
    /// (hallucination/error signal). Best-effort.
    fn record_reasoning(&self, session: &str, task: &str, content: &str, success: bool) {
        if let Some(ms) = self.memory_system.as_ref() {
            let r = ms.reasoning();
            let _ = if success {
                r.record_chain(Some(session), task, content, 0.7, true)
            } else {
                r.record_counterexample(Some(session), task, content)
            };
        }
    }

    /// Credit-assign a turn outcome to the memories that grounded it (learning
    /// loop, design §22.3). Positive on turn success, negative on failure.
    fn credit_grounding(&self, ids: &[uuid::Uuid], positive: bool) {
        if ids.is_empty() {
            return;
        }
        if let Some(ms) = self.memory_system.as_ref() {
            ms.reward_memories(ids, positive);
        }
    }

    /// Record a completed turn/tool outcome through the Write Policy so
    /// procedural/episodic/capability knowledge accrues from real executions
    /// (design §46.1). Best-effort; no-op without an attached memory system.
    pub fn record_agent_outcome(&self, session_id: &str, source_label: &str, outcome: &str) {
        let Some(memory_system) = self.memory_system.as_ref() else {
            return;
        };
        if outcome.trim().is_empty() {
            return;
        }
        let session_uuid = stable_session_uuid(session_id);
        let source = crate::memory::types::Source::Tool(source_label.to_string());
        if let Err(e) = memory_system.record_tool_outcome(session_uuid, source, outcome.to_string())
        {
            tracing::debug!(error = %e, "AgentLoop record_agent_outcome skipped");
        }
    }

    /// Whether a cognitive memory system is attached.
    pub fn has_memory_system(&self) -> bool {
        self.memory_system.is_some()
    }

    /// Attach an initialised semantic Router.
    pub fn with_semantic_router(mut self, router: Arc<crate::routing::Router>) -> Self {
        self.semantic_router = Some(router);
        self
    }

    /// Attach a tool-level semantic index for direct execution.
    pub fn with_tool_index(
        mut self,
        index: Arc<crate::routing::tool_index::SharedToolIndex>,
    ) -> Self {
        self.tool_index = Some(index);
        self
    }

    /// Attach a session vector store for document RAG retrieval.
    pub fn with_doc_store(mut self, store: Arc<crate::preprocessing::SessionVectorStore>) -> Self {
        self.doc_store = Some(store);
        self
    }

    /// Attach a feedback collector for online learning.
    pub fn with_feedback_collector(
        mut self,
        collector: Arc<tokio::sync::Mutex<crate::routing::feedback::FeedbackCollector>>,
    ) -> Self {
        self.feedback_collector = Some(collector);
        self
    }

    /// Try direct tool execution via semantic tool index (Phase 3 fast path).
    /// Categories of the tools the semantic router selected this turn — the
    /// "domain" the cross-domain injection gate scores agreement against (Wave 5).
    fn tool_categories_for(
        &self,
        routed_names: &std::collections::HashSet<String>,
        _schemas: &[ToolSchema],
    ) -> std::collections::HashSet<String> {
        if routed_names.is_empty() {
            return std::collections::HashSet::new();
        }
        self.tool_registry
            .list_for_tier(&self.hardware_tier)
            .into_iter()
            .filter(|d| routed_names.contains(&d.name))
            .map(|d| d.category)
            .collect()
    }

    /// Returns Some(tool_schema) if a high-confidence direct match is found.
    async fn try_direct_tool_match(&self, query_text: &str) -> Option<ToolSchema> {
        let tool_index = self.tool_index.as_ref()?;
        if !crate::config::RoutingConfig::default().tool_index_enabled {
            return None;
        }
        let tier = &self.hardware_tier;
        let match_result = tool_index.match_by_text(query_text, tier).await?;
        if !match_result.direct_execution {
            return None;
        }
        // Find the matching ToolSchema
        let schema = self
            .tool_registry
            .list_defs()
            .iter()
            .find(|def| def.name == match_result.name)
            .map(|def| ToolSchema {
                name: def.name.clone(),
                description: def.description.clone(),
                parameters: def.to_function_schema(),
            });
        schema
    }

    /// Override the maximum tool rounds for a single user turn.
    pub fn with_max_tool_rounds(mut self, max_tool_rounds: usize) -> Self {
        if max_tool_rounds > 0 {
            self.max_tool_rounds = max_tool_rounds;
        }
        self
    }

    /// Set the hardware tier used for tool visibility and execution gating.
    pub fn with_hardware_tier(mut self, hardware_tier: impl Into<String>) -> Self {
        let tier = hardware_tier.into();
        if !tier.trim().is_empty() {
            self.hardware_tier = tier;
        }
        self
    }

    /// Configure confidence thresholds for autonomous intent fallback.
    pub fn with_confidence_thresholds(
        mut self,
        min_confidence_to_act: f32,
        clarify_threshold: f32,
    ) -> Self {
        if (0.0..=1.0).contains(&min_confidence_to_act) {
            self.min_confidence_to_act = min_confidence_to_act;
        }
        if (0.0..=1.0).contains(&clarify_threshold) {
            self.clarify_threshold = clarify_threshold;
        }
        self
    }

    /// Attach a failover router for deterministic provider failover.
    ///
    /// When set, the agent loop uses the failover router to select backends
    /// instead of calling `model_router` directly. The failover router wraps
    /// `model_router` and adds FSM-based health tracking and automatic
    /// local→cloud failover.
    ///
    /// When not set (default), `model_router` is used directly — no behavioral change.
    pub fn with_failover_router(
        mut self,
        router: Arc<crate::llm::failover::FailoverRouter>,
    ) -> Self {
        self.failover_router = Some(router);
        self
    }

    /// Attach an execution verifier for post-execution result validation.
    ///
    /// When set, the agent loop calls the verifier after tool execution for
    /// non-trivial tools (file operations, process launches, etc.).
    /// The verifier NEVER retries or replans — it only validates and logs.
    ///
    /// When not set (default), tool results are accepted as-is — no behavioral change.
    pub fn with_execution_verifier(
        mut self,
        verifier: Arc<dyn crate::agent::execution_verifier::ExecutionVerifier>,
    ) -> Self {
        self.execution_verifier = Some(verifier);
        self
    }

    /// Attach an LLM-powered intent compiler for complex GUI automation intents.
    ///
    /// `RuleIntentCompiler` (always active) handles common verbs (<5ms, no LLM).
    /// This compiler is invoked as a fallback ONLY when `RuleIntentCompiler` returns
    /// `Verb::Other` — i.e., the rule-based path cannot classify the intent.
    ///
    /// Use `LlmIntentCompiler::new(backend)` from `agent::intent_compiler_llm`.
    /// The LLM compiler uses a structured JSON prompt and is bounded to 512 tokens.
    ///
    /// When not set, only rule-based compilation is active — no behavioral change.
    pub fn with_intent_compiler(
        mut self,
        compiler: Arc<dyn crate::agent::intent_compiler::IntentCompiler>,
    ) -> Self {
        self.intent_compiler = Some(compiler);
        self
    }

    /// Attach a session manager for ReAct workflow checkpoint persistence.
    ///
    /// When set, the agent loop persists a `WorkflowSession` checkpoint after
    /// each tool execution round, enabling recovery and continuation across
    /// interruptions or crashes.
    pub fn with_session_manager(
        mut self,
        manager: Arc<crate::agent::workflow_session::SessionManager>,
    ) -> Self {
        self.session_manager = Some(manager);
        self
    }

    /// Attach a health registry for runtime observability event counting.
    pub fn with_health_registry(
        mut self,
        health: Arc<crate::infra::health::HealthRegistry>,
    ) -> Self {
        self.health_registry = Some(health);
        self
    }

    /// Attach an execution transparency layer for per-turn ReAct lineage tracing.
    ///
    /// When set, each turn begins a `WorkflowTrace` and each tool call is recorded
    /// as a completed stage, providing observable execution lineage across all
    /// ReAct tool rounds.
    pub fn with_transparency_layer(
        mut self,
        layer: crate::agent::execution_transparency::ExecutionTransparencyLayer,
    ) -> Self {
        self.transparency_layer = Some(layer);
        self
    }

    /// Attach an observable completion engine for human-visible workflow verification.
    ///
    /// When set, the agent loop verifies expected human-visible outcomes after all
    /// tools have executed. Outcomes are inferred from user intent and verified via
    /// PSDG fast-path + bounded live probes. Only active when non-Silent outcomes
    /// are inferred. Use `ObservableCompletionEngine::new(psdg)` from
    /// `agent::observable_completion`.
    pub fn with_observable_completion(
        mut self,
        engine: std::sync::Arc<crate::agent::observable_completion::ObservableCompletionEngine>,
    ) -> Self {
        self.observable_completion = Some(engine);
        self
    }

    /// Attach a workflow expectation engine for semantic workflow classification.
    ///
    /// When set, the agent loop classifies the workflow category at turn start
    /// using keyword + operation heuristics from `WorkflowExpectationEngine::classify()`.
    /// Category and expected outcomes are logged for transparency.
    pub fn with_workflow_expectation(
        mut self,
        engine: std::sync::Arc<crate::agent::workflow_expectation::WorkflowExpectationEngine>,
    ) -> Self {
        self.workflow_expectation = Some(engine);
        self
    }

    /// Attach a collaborative autonomy engine for per-turn autonomy decisions.
    ///
    /// When set, the agent loop consults the engine before the tool loop and
    /// surfaces advisory notices (`ProceedWithNotice`) or clarification hints
    /// (`Clarify`, `Escalate`) as `StreamEvent::Plan` events. Non-blocking —
    /// does not gate tool execution (HITL + PolicyEngine handle safety gating).
    pub fn with_collaborative_autonomy(
        mut self,
        engine: std::sync::Arc<crate::agent::collaborative_autonomy::CollaborativeAutonomyEngine>,
    ) -> Self {
        self.collaborative_autonomy = Some(engine);
        self
    }

    /// Attach a workflow continuation runtime for interruption-aware recovery.
    ///
    /// When set, tool failures are classified as interruptions (`classify_interruption()`)
    /// and bounded recovery plans are generated (`plan_recovery()`). Recovery plans
    /// are logged via `log_pipeline_step` and recorded as blockers in the transparency
    /// layer for full audit lineage. Bounded to `MAX_RECOVERY_DEPTH` retries.
    pub fn with_continuation_runtime(
        mut self,
        runtime: std::sync::Arc<crate::agent::workflow_continuation::WorkflowContinuationRuntime>,
    ) -> Self {
        self.continuation_runtime = Some(runtime);
        self
    }

    // ── Batch 3 builder methods ───────────────────────────────────────────────

    /// Attach a cognition event bus for typed operational event fan-out.
    pub fn with_cognition_bus(
        mut self,
        bus: std::sync::Arc<crate::agent::cognition_event_bus::CognitionEventBus>,
    ) -> Self {
        self.cognition_bus = Some(bus);
        self
    }

    /// Attach an operational context tracker.
    pub fn with_operational_context(
        mut self,
        tracker: std::sync::Arc<crate::agent::operational_context::OperationalContextTracker>,
    ) -> Self {
        self.operational_context = Some(tracker);
        self
    }

    /// Attach a procedural workflow memory.
    pub fn with_procedural_memory(
        mut self,
        memory: std::sync::Arc<crate::agent::procedural_memory::ProceduralWorkflowMemory>,
    ) -> Self {
        self.procedural_memory = Some(memory);
        self
    }

    /// Attach a persistent goal runtime.
    pub fn with_goal_runtime(
        mut self,
        runtime: std::sync::Arc<crate::agent::goal_runtime::PersistentGoalRuntime>,
    ) -> Self {
        self.goal_runtime = Some(runtime);
        self
    }

    /// Attach an operational suggestions engine.
    pub fn with_suggestions_engine(
        mut self,
        engine: std::sync::Arc<crate::agent::operational_suggestions::OperationalSuggestionsEngine>,
    ) -> Self {
        self.suggestions_engine = Some(engine);
        self
    }

    /// Attach a desktop awareness runtime.
    pub fn with_desktop_awareness(
        mut self,
        runtime: std::sync::Arc<crate::agent::desktop_awareness::DesktopAwarenessRuntime>,
    ) -> Self {
        self.desktop_awareness = Some(runtime);
        self
    }

    /// Attach a PSDG handle for persistent semantic desktop context injection.
    ///
    /// When set, the agent loop injects a compact semantic desktop context block
    /// (focused app, browser URL, IDE workspace, etc.) into system prompts for
    /// automation/shell/IDE-relevant turns.
    ///
    /// Context injection is:
    /// - **Bounded**: max `MAX_CONTEXT_FACTS` facts, confidence ≥ 0.5
    /// - **Selective**: only injected for `Automate`, `ExecuteShell`, `Write`, `Clarify`
    /// - **Non-blocking**: snapshot read takes < 1ms from Mutex<Connection>
    ///
    /// When not set (default), no context injection — no behavioral change.
    pub fn with_world_model(mut self, psdg: crate::agent::psdg::PsdgHandle) -> Self {
        self.world_model = Some(psdg);
        self
    }

    /// Cancel all in-flight work for `session_id`.
    ///
    /// Safe to call from any thread/task.  If no turn is active for the session
    /// this is a no-op.
    pub fn cancel_session(&self, session_id: &str) {
        self.turn_admission.cancel_session(session_id);
    }

    /// Submit explicit user feedback for a routing decision.
    ///
    /// Embeds `user_text`, builds a `RoutingFeedback` record with the given outcome,
    /// immediately nudges the live domain centroids, and persists the update to disk.
    /// This is the entry point for the "Wrong tool" / "Try differently" UI buttons.
    ///
    /// Returns `true` if the centroid was actually nudged (embedding available + router ready).
    pub async fn submit_routing_feedback(
        &self,
        user_text: &str,
        domain: crate::routing::domain::Domain,
        outcome: crate::routing::feedback::RoutingOutcome,
        tool_selected: Option<String>,
        session_id: &str,
        learning_rate: f32,
    ) -> bool {
        use std::hash::{Hash, Hasher};

        // Embed the user text to get the query vector
        let embedding = crate::routing::embed::embed_batch(&[user_text])
            .ok()
            .and_then(|mut v| v.pop())
            .unwrap_or_default();

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        user_text.hash(&mut hasher);

        let feedback = crate::routing::feedback::RoutingFeedback {
            input_text_hash: hasher.finish(),
            domain_selected: domain,
            tool_selected,
            intent_source: "explicit_user_feedback".into(),
            confidence: 1.0, // explicit feedback is maximum confidence signal
            outcome,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            session_id: session_id.to_string(),
            embedding: embedding.clone(),
        };

        // Persist to the feedback collector buffer
        if let Some(ref collector) = self.feedback_collector {
            let mut c = collector.lock().await;
            c.record(feedback.clone());
        }

        // Apply immediately to the live router (Gaps 2 + 3 closed)
        if let Some(ref router) = self.semantic_router {
            let report = router.apply_feedback(&feedback, learning_rate).await;
            tracing::info!(
                domain = ?domain,
                outcome = ?feedback.outcome,
                total_adjusted = report.total_adjusted,
                "[AgentLoop] explicit feedback applied to router"
            );
            return report.total_adjusted > 0;
        }

        false
    }

    /// Return the local LLM backend used for semantic memory parsing.
    pub fn memory_parser_backend(&self) -> Option<Arc<dyn crate::llm::LlmBackend>> {
        self.model_router.get_local()
    }

    /// Fast stale-turn invalidation check for async callbacks.
    pub fn is_turn_active(&self, session_id: &str, turn_id: &str) -> bool {
        self.turn_admission.is_active(session_id, turn_id)
    }

    /// Returns a clone of the HITL gateway so that remote transports (e.g.
    /// Telegram) can resolve pending approval requests without direct access
    /// to `AgentLoop` internals.
    pub fn hitl_gateway(&self) -> Arc<HitlGateway> {
        Arc::clone(&self.hitl_gateway)
    }

    /// Best-effort pre-flight cap for visual tokens before `analyze_image`.
    async fn compute_visual_token_cap(&self) -> VisualTokenCapDecision {
        let mut safety_margin_mb = 512u64;
        let mut profile = crate::config::ModelProfile::default();
        let mut current_ngl = 0u32;
        let mut vision_enabled = self.model_router.has_vision();

        if let Some(mgr) = self.model_router.orchestrator_server_manager() {
            safety_margin_mb = mgr.safety_margin_mb();
            profile = mgr.model_profile();
            let (ngl, _ctx) = mgr.current_params();
            current_ngl = ngl;
            vision_enabled = mgr.current_vision_enabled();
        }

        let vision_mode = match (vision_enabled, current_ngl) {
            (false, _) => VisionMode::Disabled,
            (true, 0) => VisionMode::CpuVision,
            (true, ngl) if ngl < profile.vision_min_ngl => VisionMode::ReducedGpu,
            (true, _) => VisionMode::FullGpu,
        };

        if !vision_mode.has_vision() {
            return VisualTokenCapDecision {
                hard_cap: 0,
                safe_cap: 0,
                free_vram_mb: 0,
                safety_margin_mb,
                vision_mode,
            };
        }

        // HRA Phase A1: read free VRAM from the single telemetry hub's last published snapshot when
        // available (no extra device context); fall back to a one-off profiler read otherwise.
        let free_vram_mb = match crate::resource::global_telemetry_hub() {
            Some(hub) => hub
                .latest()
                .gpus
                .first()
                .map(|g| g.free_vram_mb)
                .unwrap_or(0),
            None => {
                crate::platform::vram::build_profiler()
                    .snapshot()
                    .await
                    .free_mb
            }
        };
        let safe_cap = calculate_safe_visual_tokens(
            free_vram_mb,
            safety_margin_mb,
            &profile,
            0, // Conservative fallback until live KV occupancy is exposed.
        );

        let mode_cap = match vision_mode.max_image_dimension() {
            0 => u32::MAX, // full-resolution mode
            dim => estimate_visual_tokens(dim, dim, 14),
        };

        let cap = if safe_cap == 0 {
            // If telemetry is unavailable, fall back to the mode cap.
            if mode_cap == u32::MAX {
                4096
            } else {
                mode_cap
            }
        } else if mode_cap == u32::MAX {
            safe_cap
        } else {
            safe_cap.min(mode_cap)
        };

        VisualTokenCapDecision {
            hard_cap: cap.max(64),
            safe_cap,
            free_vram_mb,
            safety_margin_mb,
            vision_mode,
        }
    }

    /// Run the agent loop for a single user turn.
    /// Returns a channel of StreamEvents.
    pub async fn run(
        &self,
        session_id: &str,
        messages: &mut Vec<ChatMessage>,
        event_tx: mpsc::UnboundedSender<StreamEvent>,
    ) {
        self.run_with_profile(session_id, messages, event_tx, None)
            .await;
    }

    /// Run the agent loop for a single user turn with an optional execution profile.
    pub async fn run_with_profile(
        &self,
        session_id: &str,
        messages: &mut Vec<ChatMessage>,
        event_tx: mpsc::UnboundedSender<StreamEvent>,
        execution_profile: Option<TurnExecutionProfile>,
    ) {
        let execution_profile = execution_profile.unwrap_or_default();

        let last_user_text = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.clone())
            .unwrap_or_default();
        let previous_user_text = messages
            .iter()
            .rev()
            .filter(|m| m.role == "user")
            .nth(1)
            .map(|m| m.content.clone());
        let explicit_queue_requested = user_requested_explicit_queue(&last_user_text);

        // ── Memory-driven reasoning (design §10 read surface) ─────────────────
        // Ground the turn with relevant long-term memory retrieved from the
        // unified MemorySystem, injected once right after the base system prompt.
        // No-op when no memory system is attached or nothing relevant is found.
        // This makes EVERY entry point (desktop, server, telegram) memory-driven
        // without per-caller wiring.
        // Planning-memory grounding: inject the historically most successful
        // approach for this task so the planner prefers what worked (Priority 1).
        if let Some(plan_hint) = self.plan_recommendation(&last_user_text) {
            let insert_pos = usize::from(
                messages
                    .first()
                    .map(|m| m.role.eq_ignore_ascii_case("system"))
                    .unwrap_or(false),
            );
            messages.insert(
                insert_pos,
                ChatMessage {
                    role: "system".into(),
                    content: plan_hint,
                    name: None,
                    images: None,
                },
            );
        }

        // Reasoning-memory grounding: prior successful reasoning + refuted
        // approaches for this task (Priority 2).
        if let Some(reason_ctx) = self.reasoning_context(&last_user_text) {
            let insert_pos = usize::from(
                messages
                    .first()
                    .map(|m| m.role.eq_ignore_ascii_case("system"))
                    .unwrap_or(false),
            );
            messages.insert(
                insert_pos,
                ChatMessage {
                    role: "system".into(),
                    content: reason_ctx,
                    name: None,
                    images: None,
                },
            );
        }

        // Goal-aware grounding: inject the active goal stack so the planner /
        // reasoner pursue standing goals (design Priority 1/2).
        if let Some(goal_ctx) = self.active_goal_context() {
            let insert_pos = usize::from(
                messages
                    .first()
                    .map(|m| m.role.eq_ignore_ascii_case("system"))
                    .unwrap_or(false),
            );
            messages.insert(
                insert_pos,
                ChatMessage {
                    role: "system".into(),
                    content: goal_ctx,
                    name: None,
                    images: None,
                },
            );
        }

        // Memory ids that grounded this turn — credited (Memory Worth) at the
        // first tool outcome so useful memories strengthen and misleading ones
        // weaken over time (learning loop, design §22.3). The retrieval class +
        // winning strategy are reinforced too (adaptive RRF, Priority 1).
        let mut grounding_memory_ids: Vec<uuid::Uuid> = Vec::new();
        let mut grounding_retrieval: Option<(
            crate::memory::retriever::QueryClass,
            crate::memory::retrieval_opt::Strategy,
        )> = None;
        let mut grounding_credited = false;
        // Observe the user turn into cognitive memory through the ONE authority
        // (Write Policy). Every host inherits this — desktop, server, Telegram,
        // WS — so user-stated facts are learned uniformly (H1). Enrichment is
        // async, so this does not pollute the current turn's own grounding.
        self.observe_user_turn(session_id, &last_user_text);
        if let Some(grounding) = self.retrieve_memory_grounding(&last_user_text).await {
            grounding_memory_ids = grounding.memory_ids;
            if let Some(strategy) = grounding.top_strategy {
                grounding_retrieval = Some((grounding.query_class, strategy));
            }
            let insert_pos = usize::from(
                messages
                    .first()
                    .map(|m| m.role.eq_ignore_ascii_case("system"))
                    .unwrap_or(false),
            );
            messages.insert(
                insert_pos,
                ChatMessage {
                    role: "system".into(),
                    content: grounding.block,
                    name: None,
                    images: None,
                },
            );
        }

        // ── Per-turn ReAct session checkpoint (for recovery / continuation) ────
        let mut react_session = self.session_manager.as_ref().map(|mgr| {
            let s = crate::agent::workflow_session::WorkflowSession::new(
                session_id.to_string(),
                last_user_text.clone(),
                "ReAct".to_string(),
            );
            // Save initial empty checkpoint so the session exists on disk immediately.
            let _ = mgr.save(&s);
            s
        });

        // ── Per-turn transparency trace ────────────────────────────────────────
        // Creates a WorkflowTrace for this ReAct turn so execution lineage is
        // observable via the transparency layer. Each tool round is recorded as
        // a completed stage for audit and human oversight.
        let react_trace_id = format!("react-{}", session_id);
        if let Some(ref layer) = self.transparency_layer {
            let react_tree = crate::agent::goal_tree::GoalTree {
                workflow_id: react_trace_id.clone(),
                description: last_user_text.chars().take(120).collect(),
                stages: vec![],
                completion: crate::agent::goal_tree::CompletionContract::AllStagesPassed,
                global_abort: vec![],
                max_total_duration_sec: 300,
                preconditions: vec![],
            };
            layer.begin_trace(&react_tree);
            tracing::debug!(
                target: "execution_transparency",
                session = session_id,
                trace_id = %react_trace_id,
                "ReAct transparency trace started"
            );
        }

        // ── Per-turn admission + cancellation tree ─────────────────────────────
        let turn_id = Uuid::new_v4().to_string();
        let turn_tree = match self.turn_admission.admit_or_enqueue_turn(
            session_id.to_string(),
            turn_id.clone(),
            explicit_queue_requested,
        ) {
            Ok(TurnAdmissionDecision::Admitted(cancellation)) => cancellation,
            Ok(TurnAdmissionDecision::Queued { depth }) => {
                let _ = event_tx.send(StreamEvent::Plan(format!(
                    "Current turn is busy. Queued this request at position {depth}."
                )));

                match self
                    .turn_admission
                    .wait_for_turn_activation(session_id, &turn_id)
                    .await
                {
                    Some(cancellation) => {
                        let _ = event_tx
                            .send(StreamEvent::Plan("Starting queued turn now.".to_string()));
                        cancellation
                    }
                    None => {
                        let canceled_msg =
                            "Queued request was canceled before execution.".to_string();
                        let _ = event_tx.send(StreamEvent::Error(canceled_msg.clone()));
                        let _ = event_tx.send(StreamEvent::Done(canceled_msg));
                        return;
                    }
                }
            }
            Err(TurnAdmissionError::QueueFull { limit, .. }) => {
                let queue_full_msg = format!(
                    "A turn is already running and the queue is full (limit {limit}). Please wait and try again."
                );
                let _ = event_tx.send(StreamEvent::Error(queue_full_msg.clone()));
                let _ = event_tx.send(StreamEvent::Done(queue_full_msg));
                return;
            }
        };
        let turn_tools_cancel = turn_tree.tools.clone();
        let turn_sidecar_cancel = turn_tree.sidecar.clone();
        let turn_mcp_cancel = turn_tree.mcp.clone();
        let turn_image_cancel = turn_tree.image.clone();

        // Injection wall (Req 9, settings-nl-control Task 3, fixes NEW-5): provenance
        // taint is PER-TURN (this local), NOT a global on the shared registry — so it
        // cannot bleed across concurrent turns/sessions. Starts false (User); flips to
        // true when an external-content tool runs THIS turn (set at dispatch), after
        // which config mutations in this turn are treated as ExternalContent and refused.
        let turn_external_taint = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Clear any leftover turn-scoped RequestOverride from a previous turn.
        self.tool_registry.clear_turn_override();

        // Turn-scoped TEMPORARY overrides ("... for this one") are now installed by
        // the unified `run_settings_stage` gate below (settings-nl-control Task 10),
        // which classifies a Temp-scope Change and calls `set_turn_override` — the
        // single decider. The old `build_turn_override` pre-dispatch was removed here.
        let turn_id_for_checks = turn_id.clone();
        let turn_admission_for_async = Arc::clone(&self.turn_admission);
        let session_id_for_async = session_id.to_string();
        let turn_id_for_async = turn_id_for_checks.clone();

        // Guard: clear this turn only if it is still active on function exit.
        struct TurnGuard {
            admission: Arc<TurnAdmission>,
            session_id: String,
            turn_id: String,
        }
        impl Drop for TurnGuard {
            fn drop(&mut self) {
                self.admission
                    .complete_turn(&self.session_id, &self.turn_id);
            }
        }
        let _turn_guard = TurnGuard {
            admission: Arc::clone(&self.turn_admission),
            session_id: session_id.to_string(),
            turn_id,
        };

        let is_turn_active = || {
            self.turn_admission
                .is_active(session_id, &turn_id_for_checks)
        };
        let return_if_stale = || {
            if is_turn_active() {
                false
            } else {
                log_pipeline_step(
                    session_id,
                    "stale_turn_dropped",
                    "Turn became stale; dropping in-flight result",
                    Some(serde_json::json!({
                        "turn_id": turn_id_for_checks,
                    })),
                );
                let _ = event_tx.send(StreamEvent::Done("Turn cancelled.".into()));
                true
            }
        };

        let _ = event_tx.send(StreamEvent::TurnAccepted {
            session_id: session_id.to_string(),
            turn_id: turn_id_for_checks.clone(),
        });

        // ── settings-nl-control Wave 3: first-stage NL settings gate ──────────
        // Runs BEFORE IntentGate/forcing/deterministic dispatch so recognized
        // settings intents (change/read-back/undo/temp) are handled by the single
        // shared pipeline+handler through the real HITL gate — never misrouted to
        // browser/search/GUI. `NotSettings` ⇒ untouched flow (byte-for-byte legacy
        // when the flag is off).
        let mut last_user_text = last_user_text;
        let settings_turn_has_images = messages.last().is_some_and(|m| m.has_images());
        if nl_settings_enabled() && !settings_turn_has_images {
            match self
                .run_settings_stage(session_id, &last_user_text, messages, &event_tx)
                .await
            {
                SettingsStageResult::Claimed => return,
                SettingsStageResult::ContinueWith(remainder) => {
                    last_user_text = remainder;
                }
                SettingsStageResult::Pass => {}
            }
        }

        // ── Per-turn error-loop guards ─────────────────────────────────────────
        // Maps call_dedup_hash(tool, args) -> (failure_count, last_error_msg).
        let mut failed_calls: HashMap<u64, (u8, String)> = HashMap::new();
        // Count of *consecutive* tool failures this turn (reset on any success).
        let mut consecutive_failures: u8 = 0;
        const MAX_CONSECUTIVE_FAILURES: u8 = 3;

        // ── Phase 4: Per-turn token ledger + provider-aware budgets ──────────
        // The ledger tracks all token categories across the full turn.
        // Budgets scale with the active provider's context window.
        let turn_ledger = TurnTokenLedger::new();

        // Derive context window from the orchestrator if available, else use config default.
        let active_context_window = self
            .model_router
            .orchestrator_server_manager()
            .map(|mgr| {
                let (_, ctx) = mgr.current_params();
                ctx as usize
            })
            .unwrap_or(4096);

        let context_budgets = ContextBudgets::for_context_window(active_context_window);

        // Backward-compat: keep turn_tool_tokens as a simple alias for the ledger's tool total.
        // This avoids touching every reference to turn_tool_tokens in the loop body.
        // The ledger is the authoritative source; turn_tool_tokens is derived from it.
        let mut turn_tool_tokens: usize = 0;

        // Check if the user message contains images and route accordingly
        let has_images = messages.last().is_some_and(|m| m.has_images());
        let mut routing_focus_text = routing_focus_text_from_user_content(&last_user_text);

        // ── IntentGate: Conversation-First Routing Guard ──────────────────────
        // Runs BEFORE live-fact classification, tool routing, and provider escalation.
        // If the gate classifies the input as conversational, the entire tool pipeline
        // is suppressed and the LLM responds directly.
        let gate_thresholds = crate::agent::intent_gate::ConfidenceThresholds::from_env();
        let gate_decision = crate::agent::intent_gate::classify(
            &routing_focus_text,
            &gate_thresholds,
            None, // semantic router result not yet available at this point
        );

        // ── Active Turn Memory ────────────────────────────────────────────────
        // Tracks completed actions, memoized results, and execution target.
        // Used for task satisfaction detection and duplicate call prevention.
        let primary_target = ExecutionTarget::infer(&routing_focus_text, "");
        let mut turn_memory = TurnMemory::new(&routing_focus_text, primary_target);

        log_pipeline_step(
            session_id,
            "intent_gate",
            "IntentGate classification",
            Some(gate_decision.to_json()),
        );

        // Fast-path: conversational inputs bypass all tool routing
        if gate_decision.fast_path {
            tracing::info!(
                session = session_id,
                intent = gate_decision.intent.as_str(),
                confidence = gate_decision.confidence,
                reason = gate_decision.reason,
                "IntentGate: conversational fast-path activated — suppressing tool pipeline"
            );
            // Skip live-fact injection, tool routing, and intent fallback.
            // The LLM will respond directly with the full context.
            // Fall through to the LLM call with no tool schemas injected.
            // We signal this by NOT modifying routing_focus_text and setting
            // a flag that suppresses the intent fallback later.
        }

        // Clarification path: ambiguous inputs ask for clarification
        let gate_requires_clarification =
            gate_decision.clarification_required && !gate_decision.fast_path;

        // Layer 2: Deterministic Tool Forcing for live-fact queries
        // MUST run BEFORE tool_lock prefix injection so the classifier sees clean user text
        // SKIP if IntentGate already classified as conversational fast-path
        let is_live_fact = if gate_decision.fast_path
            || execution_profile.is_manual_tool_override()
            || is_n8n_workflow_list_query(&routing_focus_text)
        {
            false // Never force live-fact search on conversational, manual, or local n8n inventory queries.
        } else {
            crate::routing::live_fact::is_live_fact_query(&routing_focus_text)
        };
        if is_live_fact && extract_forced_tool_directive(&routing_focus_text).is_none() {
            // Force searxng_search as the primary tool for live-fact queries
            // search_news is also made available via the pinned tool list
            routing_focus_text = format!("#tool:searxng_search {}", routing_focus_text);
            tracing::info!(
                original_query = %routing_focus_text_from_user_content(&last_user_text),
                forced_query = %routing_focus_text,
                "LiveFactClassifier: forced searxng_search via #tool: directive"
            );
        }

        // Layer 2b: Deterministic Tool Forcing for GUI-launch queries
        // When the user says "open chrome and search for X", "launch firefox", etc.,
        // force browser_search directly — bypasses LLM tool selection which would
        // otherwise pick web_search/searxng_search due to training priors.
        // Uses the GuiIntentClassifier (structural signal scoring, not keyword lists).
        // SKIP if already forced by live-fact or tool_lock.
        if !gate_decision.fast_path && extract_forced_tool_directive(&routing_focus_text).is_none()
        {
            use crate::routing::gui_intent::{classify_gui_intent, GuiIntent};
            let gui = classify_gui_intent(&routing_focus_text);
            if gui.intent == GuiIntent::GuiLaunch
                && should_force_browser_search_for_gui_launch_query(&routing_focus_text)
            {
                routing_focus_text = format!("#tool:browser_search {}", routing_focus_text);
                tracing::info!(
                    original_query = %routing_focus_text_from_user_content(&last_user_text),
                    gui_score = gui.net_score,
                    "GuiIntentClassifier: forced browser_search via #tool: directive"
                );
            }
        }

        // Now apply tool_lock prefix (after live-fact check)
        if execution_profile.uses_direct_strategy() {
            if let Some(tool_lock) = execution_profile.tool_lock.as_deref() {
                if extract_forced_tool_directive(&routing_focus_text).is_none() {
                    routing_focus_text = format!("#tool:{} {}", tool_lock, routing_focus_text);
                }
            }
        }

        let routing_focus_lower = routing_focus_text.to_lowercase();
        let mut turn_gate_plan = self.turn_gate.plan_turn(&last_user_text, has_images);

        // ═══════════════════════════════════════════════════════════════════════════
        // RFC v2 P1: Intent Compilation - Semantic normalization before routing
        //
        // Batch 1 wiring: RuleIntentCompiler is always the fast path (<5ms, no LLM).
        // When RuleIntentCompiler returns Verb::Other and a LlmIntentCompiler is
        // attached via `.with_intent_compiler()`, the LLM compiler is invoked as
        // a fallback to handle complex multi-verb GUI intents.
        // ═══════════════════════════════════════════════════════════════════════════
        let compiled_spec = {
            use crate::agent::intent_compiler::{IntentCompiler, Verb};
            use crate::agent::intent_compiler_llm::RuleIntentCompiler;

            let rule_compiler = RuleIntentCompiler;
            let rule_result = rule_compiler
                .compile(&last_user_text, &turn_gate_plan.intent)
                .await;

            // Try LLM fallback if rule compiler produced Verb::Other
            let compile_result = match rule_result {
                Ok(ref spec) if matches!(spec.primary_verb, Verb::Other(_)) => {
                    if let Some(ref llm_compiler) = self.intent_compiler {
                        tracing::debug!(
                            target: "intent_compiler",
                            session = session_id,
                            "RuleIntentCompiler returned Verb::Other — trying LLM fallback"
                        );
                        llm_compiler
                            .compile(&last_user_text, &turn_gate_plan.intent)
                            .await
                    } else {
                        rule_result
                    }
                }
                other => other,
            };

            match compile_result {
                Ok(spec) => {
                    log_pipeline_step(
                        session_id,
                        "intent_compiled",
                        "Intent normalized to GuiTaskSpec",
                        Some(serde_json::json!({
                            "primary_verb": format!("{:?}", spec.primary_verb),
                            "targets": spec.targets.len(),
                            "has_content": spec.content.is_some(),
                            "declared_preconditions": spec.declared_preconditions.len(),
                            "declared_success_criteria": spec.declared_success_criteria.len(),
                        })),
                    );

                    if !spec.ambiguities.is_empty() {
                        let question = match spec.ambiguities.first() {
                            Some(crate::agent::intent_compiler::Ambiguity::AppNotSpecified) => {
                                "Which application should I use?".to_string()
                            }
                            Some(crate::agent::intent_compiler::Ambiguity::FileNotSpecified) => {
                                "Which file should I use?".to_string()
                            }
                            Some(
                                crate::agent::intent_compiler::Ambiguity::MultipleTargetsPossible,
                            ) => "Which one should I use?".to_string(),
                            Some(crate::agent::intent_compiler::Ambiguity::ContentScopeUnclear) => {
                                "How would you like to run this?".to_string()
                            }
                            None => "Please clarify your request".to_string(),
                        };
                        let final_text = format!("🤔 {}", question);
                        let _ = event_tx.send(StreamEvent::Done(final_text.into()));
                        return;
                    }
                    Some(spec)
                }
                Err(clarify_req) => {
                    let final_text = format!("🤔 {}", clarify_req.question);
                    log_pipeline_step(
                        session_id,
                        "intent_clarify",
                        "IntentCompiler raised clarification request",
                        Some(serde_json::json!({
                            "question": clarify_req.question,
                            "options_count": clarify_req.options.len(),
                        })),
                    );
                    let _ = event_tx.send(StreamEvent::Done(final_text.into()));
                    return;
                }
            }
        };

        if let Some(spec) = compiled_spec.as_ref() {
            let semantic_analysis =
                crate::agent::semantic_workflow::analyze_semantic_workflow(spec, &last_user_text);
            let detail = serde_json::to_value(&semantic_analysis).unwrap_or_else(|err| {
                serde_json::json!({
                    "serialization_error": err.to_string(),
                    "trace": "semantic workflow analysis produced non-serializable metadata",
                })
            });
            log_pipeline_step(
                session_id,
                "semantic_workflow_analyzed",
                "Semantic workflow frame and fidelity metadata generated",
                Some(detail),
            );

            let execution_mode_decision =
                crate::agent::execution_mode_reasoner::ExecutionModeReasoner.decide(
                    spec,
                    &semantic_analysis,
                    &crate::agent::execution_mode_reasoner::EnvironmentCapabilities::unchecked_default(),
                    &crate::agent::execution_mode_reasoner::PolicyContext::default(),
                );
            let detail = serde_json::to_value(&execution_mode_decision).unwrap_or_else(|err| {
                serde_json::json!({
                    "serialization_error": err.to_string(),
                    "trace": "execution mode decision produced non-serializable metadata",
                })
            });
            log_pipeline_step(
                session_id,
                "execution_mode_decided",
                "Execution mode decision generated without changing execution behavior",
                Some(detail),
            );

            let contract_check =
                crate::agent::workflow_intent_contract::WorkflowIntentContractRegistry
                    .evaluate(&execution_mode_decision, &semantic_analysis);
            let detail = serde_json::to_value(&contract_check).unwrap_or_else(|err| {
                serde_json::json!({
                    "serialization_error": err.to_string(),
                    "trace": "workflow contract check produced non-serializable metadata",
                })
            });
            log_pipeline_step(
                session_id,
                "workflow_contract_evaluated",
                "Workflow intent contract evaluated without changing execution behavior",
                Some(detail),
            );

            let workflow_attempt_id = format!("{}:{}", session_id, turn_id_for_checks);
            let verifier_authority_assessment =
                crate::agent::verifier_authority::VerifierAuthorityEvaluator.assess(
                    &contract_check,
                    &execution_mode_decision,
                    &semantic_analysis,
                    workflow_attempt_id.clone(),
                );
            let detail =
                serde_json::to_value(&verifier_authority_assessment).unwrap_or_else(|err| {
                    serde_json::json!({
                        "serialization_error": err.to_string(),
                        "trace": "verifier authority assessment produced non-serializable metadata",
                    })
                });
            log_pipeline_step(
                session_id,
                "verifier_authority_assessed",
                "Verifier authority and freshness requirements generated without changing execution behavior",
                Some(detail),
            );

            let hybrid_synchronization_assessment =
                crate::agent::hybrid_synchronization::HybridSynchronizationEvaluator.assess(
                    &execution_mode_decision,
                    &semantic_analysis,
                    &verifier_authority_assessment,
                    workflow_attempt_id,
                );
            let detail =
                serde_json::to_value(&hybrid_synchronization_assessment).unwrap_or_else(|err| {
                    serde_json::json!({
                        "serialization_error": err.to_string(),
                        "trace": "hybrid synchronization assessment produced non-serializable metadata",
                    })
                });
            log_pipeline_step(
                session_id,
                "hybrid_synchronization_assessed",
                "Hybrid structural-visible synchronization checkpoints generated without changing execution behavior",
                Some(detail),
            );

            let browser_media_governance_assessment =
                crate::agent::browser_media_governance::BrowserMediaGovernanceEvaluator.assess(
                    &semantic_analysis,
                    &execution_mode_decision,
                    &last_user_text,
                );
            let detail =
                serde_json::to_value(&browser_media_governance_assessment).unwrap_or_else(|err| {
                    serde_json::json!({
                        "serialization_error": err.to_string(),
                        "trace": "browser/media governance assessment produced non-serializable metadata",
                    })
                });
            log_pipeline_step(
                session_id,
                "browser_media_governance_assessed",
                "Browser/media account ambiguity and visible verifier governance generated without changing execution behavior",
                Some(detail),
            );
        }

        let pure_image_analysis_turn =
            has_images && matches!(turn_gate_plan.intent.operation, Operation::AnalyzeImage);
        let wants_vision_backend =
            has_images && matches!(turn_gate_plan.resource_plan, ResourcePlan::L1Vision { .. });
        let reflex_cancel_turn = matches!(turn_gate_plan.intent.operation, Operation::Cancel)
            && matches!(turn_gate_plan.resource_plan, ResourcePlan::ReflexRust);
        let mut inline_images_allowed_for_turn = true;
        let mut inline_image_vision_mode = VisionMode::FullGpu;
        if has_images {
            let cap_probe = self.compute_visual_token_cap().await;
            inline_image_vision_mode = cap_probe.vision_mode;
            inline_images_allowed_for_turn = cap_probe.vision_mode.has_vision();
        }

        log_pipeline_step(
            session_id,
            "prompt_entered",
            "Agent loop received prompt",
            Some(serde_json::json!({
                "has_images": has_images,
                "pure_image_analysis_turn": pure_image_analysis_turn,
                "wants_vision_backend": wants_vision_backend,
                "reflex_cancel_turn": reflex_cancel_turn,
                "prompt_lab_mode": execution_profile.is_prompt_lab(),
                "manual_tool_override": execution_profile.is_manual_tool_override(),
                "prompt_lab_strategy": format!("{:?}", execution_profile.prompt_lab_strategy),
                "app_lock": execution_profile.app_lock.clone(),
                "tool_lock": execution_profile.tool_lock.clone(),
                "turn_gate": {
                    "modality": format!("{:?}", turn_gate_plan.intent.modality),
                    "operation": format!("{:?}", turn_gate_plan.intent.operation),
                    "hazard_hint": format!("{:?}", turn_gate_plan.intent.hazard_hint),
                    "compute": format!("{:?}", turn_gate_plan.intent.compute),
                    "source": format!("{:?}", turn_gate_plan.intent.source),
                    "confidence": turn_gate_plan.intent.confidence,
                    "resource_plan": format!("{:?}", turn_gate_plan.resource_plan),
                },
                "message_count": messages.len(),
                "prompt_preview": sanitize_text_for_logs(&routing_focus_text, 260),
            })),
        );

        if reflex_cancel_turn {
            let final_text = "Stopped current operation.";
            log_pipeline_step(
                session_id,
                "turn_gate_reflex_short_circuit",
                "TurnGate resolved a reflex cancel turn; skipping backend and tool routing",
                Some(serde_json::json!({
                    "turn_gate": {
                        "operation": format!("{:?}", turn_gate_plan.intent.operation),
                        "compute": format!("{:?}", turn_gate_plan.intent.compute),
                    },
                    "final_text": final_text,
                })),
            );
            let _ = event_tx.send(StreamEvent::Plan(
                "Stopping current operation immediately.".into(),
            ));
            let _ = event_tx.send(StreamEvent::Done(final_text.into()));
            return;
        }

        // ═══════════════════════════════════════════════════════════════════════════
        // Session Continuation Detection — check for resumable workflows
        // ═══════════════════════════════════════════════════════════════════════════
        // Before routing to GUI executor, check if the user's prompt matches
        // an interrupted workflow that can be continued.
        {
            if let Some((context, detail, options)) =
                detect_session_continuation_options(&last_user_text)
            {
                tracing::info!(
                    target: "session_continuation",
                    context = %context,
                    "Resumable workflow detected — surfacing continuation options"
                );
                // Surface as RecoveryOptions so the UI renders clickable buttons
                let _ = event_tx.send(StreamEvent::RecoveryOptions {
                    context,
                    detail,
                    options,
                });
                let pause_text = "I found an interrupted workflow. Please choose Continue, Start fresh, or Dismiss before KRIA runs more automation.";
                let _ = event_tx.send(StreamEvent::Token(pause_text.into()));
                let _ = event_tx.send(StreamEvent::Done(pause_text.into()));
                return;
            }
        }

        // ═══════════════════════════════════════════════════════════════════════════
        // DETERMINISTIC DISPATCH — runs BEFORE GUI routing and BEFORE ReAct loop.
        // This ensures simple operations (whoami, mkdir, ls, n8n workflows) always
        // work regardless of LLM availability or GUI routing decisions.
        // ═══════════════════════════════════════════════════════════════════════════
        if let Some((tool_name, deterministic_params)) = try_deterministic_dispatch_with_profile(
            &last_user_text,
            previous_user_text.as_deref(),
            &execution_profile,
        ) {
            if tool_name == KRIA_DETERMINISTIC_NOTICE_TOOL {
                let message = deterministic_notice_message(&deterministic_params);
                log_pipeline_step(
                    session_id,
                    "deterministic_notice_early",
                    "Prompt matched local deterministic notice — responding without LLM or tool execution",
                    Some(serde_json::json!({
                        "manual_tool_override": execution_profile.is_manual_tool_override(),
                        "app_lock": execution_profile.app_lock.clone(),
                        "tool_lock": execution_profile.tool_lock.clone(),
                    })),
                );
                let _ = event_tx.send(StreamEvent::Token(message.clone()));
                let _ = event_tx.send(StreamEvent::Done(message));
                return;
            } else if !execution_profile.allows_tool_name(&tool_name) {
                log_pipeline_step(
                    session_id,
                    "deterministic_dispatch_blocked_by_execution_profile",
                    "Deterministic tool match ignored because it is outside the active manual tool mode",
                    Some(serde_json::json!({
                        "tool": tool_name,
                        "manual_tool_override": execution_profile.is_manual_tool_override(),
                        "app_lock": execution_profile.app_lock.clone(),
                        "tool_lock": execution_profile.tool_lock.clone(),
                    })),
                );
            } else if let Some(handler) = self.tool_registry.get_handler(&tool_name) {
                log_pipeline_step(
                    session_id,
                    "deterministic_dispatch_early",
                    "Prompt matched deterministic pattern — executing before GUI/ReAct routing",
                    Some(serde_json::json!({
                        "tool": tool_name,
                    })),
                );

                // Emit in-progress indicator for n8n workflows
                if tool_name == "n8n_invoke_workflow" {
                    let wf_display = deterministic_params
                        .get("workflow_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("workflow");
                    let _ = event_tx.send(StreamEvent::Token(format!(
                        "⏳ Running workflow '{}'...\n",
                        wf_display
                    )));
                }

                let handler = handler.clone();
                let tool_context = self
                    .tool_registry
                    .make_tool_context(tokio_util::sync::CancellationToken::new());

                let tool_execution_id = Uuid::now_v7().to_string();
                let tool_execution_started = std::time::Instant::now();
                tracing::info!(
                    target: "tool_execution",
                    session = session_id,
                    execution_id = %tool_execution_id,
                    tool_name = %tool_name,
                    input_summary = %sanitize_json_for_logs(&deterministic_params, 220, 8),
                    "Tool execution started"
                );

                let dispatch_result = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    handler.execute_with_context(deterministic_params.clone(), tool_context),
                )
                .await;

                match dispatch_result {
                    Ok(result) => {
                        tracing::info!(
                            target: "tool_execution",
                            session = session_id,
                            execution_id = %tool_execution_id,
                            tool_name = %tool_name,
                            duration_ms = tool_execution_started.elapsed().as_millis(),
                            success = result.success,
                            failure_reason = %result.error.as_deref().unwrap_or("-"),
                            result_summary = %sanitize_json_for_logs(&result.data, 220, 8),
                            "Tool execution completed"
                        );
                        let summary = if result.error.is_some() {
                            // User-friendly error formatting — never expose tool names
                            format_tool_error_for_user(
                                &tool_name,
                                result.error.as_deref().unwrap_or("unknown"),
                            )
                        } else if tool_name == "n8n_invoke_workflow" {
                            format_n8n_result(&result.data)
                        } else {
                            // Smart output formatting: extract human-readable content
                            format_tool_result_for_user(&tool_name, &result.data)
                        };

                        let _ = event_tx.send(StreamEvent::Token(summary.clone()));
                        let _ = event_tx.send(StreamEvent::Done(summary));
                        return;
                    }
                    Err(_) => {
                        tracing::warn!(
                            target: "tool_execution",
                            session = session_id,
                            execution_id = %tool_execution_id,
                            tool_name = %tool_name,
                            duration_ms = tool_execution_started.elapsed().as_millis(),
                            "Tool execution timed out"
                        );
                        tracing::warn!(
                            target: "deterministic_dispatch",
                            tool = %tool_name,
                            "Early deterministic dispatch timed out — falling to normal routing"
                        );
                        // Fall through to GUI/ReAct routing below
                    }
                }
            }
        }

        // ═══════════════════════════════════════════════════════════════════════════
        // RFC 007 Phase 4: GUI HTN Routing - BYPASS ReAct loop for GUI automation
        // ═══════════════════════════════════════════════════════════════════════════
        // Routing now requires BOTH TurnGate confidence gating AND a valid
        // GuiTaskSpec from the IntentCompiler.  No substring bypasses.
        use crate::agent::gui_wiring::GuiExecutionCoordinator;

        let should_route_to_gui =
            GuiExecutionCoordinator::should_route_to_gui_executor(&turn_gate_plan)
                && compiled_spec.is_some();

        if should_route_to_gui {
            let spec = compiled_spec.as_ref().unwrap();
            tracing::info!(
                target: "gui_execution_trace",
                session = session_id,
                operation = ?turn_gate_plan.intent.operation,
                confidence = turn_gate_plan.intent.confidence,
                resource_plan = ?turn_gate_plan.resource_plan,
                prompt_preview = %sanitize_text_for_logs(&last_user_text, 180),
                "[GUI] Prompt Received -> Intent Classified -> Execution Mode Selected"
            );

            // ═══════════════════════════════════════════════════════════════════════════
            // Phase 8: Canonical Runtime Router — dispatch authority
            // ═══════════════════════════════════════════════════════════════════════════
            // The WorkflowRuntimeRouter is now the SINGLE dispatch authority.
            // It determines whether the canonical or legacy runtime handles execution.
            // RuntimeMode controls the behavior:
            //   Legacy → existing path executes (current default)
            //   Shadow → both paths run, results compared
            //   Canonical → new HybridWorkflowExecutor is authoritative
            let _router_is_react_loop = {
                use crate::agent::workflow_router::{
                    RoutingDecision, RuntimeMode, WorkflowRuntimeRouter,
                };

                // Resolve capabilities for this workflow
                let workflow_capabilities =
                    crate::agent::workflow_capability::resolve_capabilities().await;

                // Route through canonical router
                let router = WorkflowRuntimeRouter::new(RuntimeMode::Legacy);
                let routing_decision = router.route_without_registry(
                    spec,
                    &last_user_text,
                    &workflow_capabilities,
                    true, // is_gui_intent confirmed by should_route_to_gui
                );

                // Log the routing decision for observability
                log_pipeline_step(
                    session_id,
                    "workflow_router_decision",
                    "Canonical runtime router decision",
                    Some(serde_json::json!({
                        "decision": format!("{:?}", routing_decision),
                        "runtime_mode": "Legacy",
                        "capabilities": {
                            "session_type": format!("{:?}", workflow_capabilities.environment.session_type),
                            "atspi": format!("{:?}", workflow_capabilities.environment.atspi_level),
                            "uinput": workflow_capabilities.environment.uinput_available,
                            "window_confidence": workflow_capabilities.verifier.window_state_max_confidence,
                        },
                    })),
                );

                // In Legacy mode: fall through to existing execution path below.
                // In Canonical mode (future): HybridWorkflowExecutor would handle here.
                // In Shadow mode (future): both would run with comparison.
                //
                // For now, the router's decision is logged but legacy always executes.
                // This gives us observability into what the canonical runtime WOULD do
                // without changing any behavior.
                match routing_decision {
                    RoutingDecision::HitlBeforeRouting {
                        reason,
                        options,
                        context,
                    } => {
                        // HITL needed before execution — emit structured HITL telemetry
                        // This is the ONE place where pre-execution HITL is surfaced.
                        tracing::info!(
                            target: "workflow_router",
                            reason = ?reason,
                            "Pre-execution HITL triggered by capability negotiation"
                        );
                        log_pipeline_step(
                            session_id,
                            "workflow_hitl_pre_execution",
                            "HITL required before workflow execution",
                            Some(serde_json::json!({
                                "reason": format!("{:?}", reason),
                                "options_count": options.len(),
                                "context": &context,
                            })),
                        );
                        // Surface as user-visible message (legacy compatibility)
                        let hitl_msg = format!("⏸ {}", context);
                        let _ = event_tx.send(StreamEvent::Token(hitl_msg.clone()));
                        let _ = event_tx.send(StreamEvent::Done(hitl_msg));
                        return;
                    }
                    RoutingDecision::ReactLoop { reason } => {
                        // Router says this should go to ReAct — override GUI routing
                        tracing::info!(
                            target: "workflow_router",
                            reason = reason,
                            "Router overriding GUI routing → ReAct fallback"
                        );
                        log_pipeline_step(
                            session_id,
                            "workflow_router_react_override",
                            "Router redirected GUI intent to ReAct loop",
                            Some(serde_json::json!({ "reason": reason })),
                        );
                        // Fall through to ReAct loop below (don't enter GUI path)
                        // This is handled by NOT entering the coordinator block below.
                    }
                    RoutingDecision::Unroutable { reason } => {
                        tracing::warn!(
                            target: "workflow_router",
                            reason = %reason,
                            "Workflow unroutable"
                        );
                        let msg = format!("I cannot execute this workflow: {}", reason);
                        let _ = event_tx.send(StreamEvent::Error(msg.clone()));
                        let _ = event_tx.send(StreamEvent::Done(msg));
                        return;
                    }
                    RoutingDecision::CanonicalWorkflow {
                        ref planning_result,
                    } => {
                        let plan_summary = planning_result;
                        // ═══════════════════════════════════════════════════════════════
                        // STAGE 1 CANONICAL EXECUTION — Authority Transfer
                        // ═══════════════════════════════════════════════════════════════
                        // The canonical runtime is now the execution authority for
                        // eligible workflows. Check activation policy before executing.
                        use crate::agent::gui_substrate_planner::SubstratePlanner;
                        use crate::agent::workflow_activation::{
                            validate_canonical_readiness, ActivationStage,
                            CanonicalActivationPolicy, RuntimeEligibility,
                        };

                        // Check activation policy
                        let policy =
                            CanonicalActivationPolicy::at_stage(ActivationStage::FullActivation);
                        let substrate_plan = SubstratePlanner.plan(spec, &last_user_text);

                        let eligibility = policy.is_eligible(substrate_plan.substrate);

                        match eligibility {
                            RuntimeEligibility::Canonical => {
                                // Validate readiness
                                let readiness =
                                    validate_canonical_readiness(&workflow_capabilities);
                                if !readiness.is_ready() {
                                    tracing::warn!(
                                        target: "workflow_activation",
                                        "Canonical readiness check failed — falling back to legacy"
                                    );
                                    log_pipeline_step(
                                        session_id,
                                        "canonical_readiness_failed",
                                        "Readiness check failed, using legacy runtime",
                                        None,
                                    );
                                    // Fall through to legacy below
                                }
                                // For now, even eligible workflows fall through to legacy
                                // until we have sufficient shadow-mode confidence.
                                // TODO: Replace this with actual HybridWorkflowExecutor::execute()
                                // when shadow parity reaches >95%.
                                else {
                                    // ═══════════════════════════════════════════════════
                                    // CANONICAL EXECUTION — TRUE AUTHORITY TRANSFER
                                    // ═══════════════════════════════════════════════════
                                    tracing::info!(
                                        target: "workflow_activation",
                                        substrate = %plan_summary.substrate,
                                        steps = plan_summary.step_count,
                                        "Stage 1 CANONICAL EXECUTION — real authority transfer"
                                    );
                                    log_pipeline_step(
                                        session_id,
                                        "canonical_stage1_executing",
                                        "CANONICAL EXECUTION active",
                                        Some(serde_json::json!({
                                            "substrate": plan_summary.substrate,
                                            "step_count": plan_summary.step_count,
                                        })),
                                    );

                                    // Build real tool executor directly from AgentLoop resources
                                    let canonical_cancellation =
                                        tokio_util::sync::CancellationToken::new();
                                    let modality =
                                        crate::routing::verbs::classify_modality(&last_user_text);
                                    let canonical_tool_executor: std::sync::Arc<
                                        dyn crate::agent::htn_executor::ToolExecutor,
                                    > = std::sync::Arc::new(
                                        crate::agent::gui_wiring::build_policy_tool_executor(
                                            Arc::clone(&self.tool_registry),
                                            canonical_cancellation.clone(),
                                            Arc::clone(&self.policy_engine),
                                            Arc::clone(&self.hitl_gateway),
                                            Arc::clone(&self.audit_logger),
                                            session_id.to_string(),
                                            last_user_text.clone(),
                                            modality.destructive,
                                        ),
                                    );

                                    // Get outcome contract — use empty contract to avoid
                                    // blocking async runtime with build_sync()
                                    let oc = crate::agent::workflow_types::OutcomeContract::empty();
                                    let em =
                                        crate::agent::workflow_types::ExecutionMode::Structural;

                                    // Execute via canonical runtime
                                    let cr = crate::agent::workflow_executor::HybridWorkflowExecutor::execute_with_tools(
                                        &substrate_plan,
                                        oc, em,
                                        workflow_capabilities.clone(),
                                        canonical_cancellation.clone(),
                                        crate::agent::workflow_executor::ExecutorConfig::default(),
                                        Some(canonical_tool_executor),
                                    ).await;

                                    // Emit result
                                    let summary = match &cr.verdict {
                                        crate::agent::workflow_types::WorkflowVerdict::Complete =>
                                            format!("Task completed. {} step(s) via canonical runtime.", cr.step_results.len()),
                                        crate::agent::workflow_types::WorkflowVerdict::StructurallyComplete { unverified_outcomes } =>
                                            format!("Task completed structurally. Unverified: {}", unverified_outcomes.join(", ")),
                                        crate::agent::workflow_types::WorkflowVerdict::Failed { step, reason, .. } =>
                                            format!("Task failed at step {}: {}", step, reason),
                                        crate::agent::workflow_types::WorkflowVerdict::Partial { completed, total, reason } =>
                                            format!("Partial: {}/{} steps. {}", completed, total, reason),
                                        other => format!("{:?}", other),
                                    };

                                    log_pipeline_step(
                                        session_id,
                                        "canonical_execution_complete",
                                        "Canonical execution finished",
                                        Some(serde_json::json!({
                                            "verdict": format!("{:?}", cr.verdict),
                                            "duration_ms": cr.duration_ms,
                                        })),
                                    );

                                    if matches!(
                                        cr.verdict,
                                        crate::agent::workflow_types::WorkflowVerdict::Failed { .. }
                                    ) {
                                        let _ = event_tx.send(StreamEvent::Error(summary.clone()));
                                        let _ = event_tx.send(StreamEvent::Done(summary));
                                    } else {
                                        let _ = event_tx.send(StreamEvent::Token(summary.clone()));
                                        let _ = event_tx.send(StreamEvent::Done(summary));
                                    }
                                    return; // CANONICAL RUNTIME HANDLED THIS WORKFLOW
                                }
                            }
                            RuntimeEligibility::Legacy { reason } => {
                                tracing::debug!(
                                    target: "workflow_activation",
                                    reason = reason,
                                    "Workflow not eligible for canonical execution"
                                );
                            }
                        }
                        // Continue to legacy execution path below
                    }
                    RoutingDecision::LegacyGuiExecution { .. } => {
                        // Continue to legacy execution path below
                    }
                }

                // If router said ReactLoop, skip the GUI execution block
                if matches!(&routing_decision, RoutingDecision::ReactLoop { .. }) {
                    log_pipeline_step(
                        session_id,
                        "gui_htn_fallback",
                        "Router redirected to ReAct — skipping GUI execution",
                        None,
                    );
                } else {
                    // Continue with legacy GUI execution (existing code below)
                }

                // Hoist the routing decision out of this scope for enforcement below.
                matches!(&routing_decision, RoutingDecision::ReactLoop { .. })
            };

            // ═══════════════════════════════════════════════════════════════════════════
            // ROUTER AUTHORITY ENFORCEMENT: If ReactLoop was chosen, exit GUI path.
            // ═══════════════════════════════════════════════════════════════════════════
            let router_redirected_to_react = _router_is_react_loop;

            if router_redirected_to_react {
                log_pipeline_step(
                    session_id,
                    "gui_htn_react_enforcement",
                    "Router authority enforced: GUI execution skipped, falling through to ReAct loop",
                    None,
                );
                // DO NOT proceed with GUI execution — fall through to ReAct loop below.
            }

            // Only proceed with GUI execution if the router did NOT redirect to ReAct.
            if !router_redirected_to_react {
                tracing::info!(
                    target: "gui_execution_trace",
                    session = session_id,
                    primary_verb = ?spec.primary_verb,
                    target_count = spec.targets.len(),
                    "[GUI] Capability Resolution complete; preparing workflow plan"
                );
                log_pipeline_step(
                    session_id,
                    "gui_htn_routing",
                    "Routing to HTN GuiExecutor (bypassing ReAct loop)",
                    Some(serde_json::json!({
                        "turn_gate": {
                            "operation": format!("{:?}", turn_gate_plan.intent.operation),
                            "direct_tool_hint": turn_gate_plan.direct_tool_hint,
                        },
                        "primary_verb": format!("{:?}", spec.primary_verb),
                        "user_text_preview": sanitize_text_for_logs(&last_user_text, 100),
                    })),
                );

                // Create kill switch interceptor for this workflow
                let socket_path = crate::agent::gui_services::default_uinput_socket_path();
                let gui_backend = Arc::new(crate::tools::gui_automation::YdotoolBackend::new(
                    socket_path,
                ));
                let workflow_cancellation = tokio_util::sync::CancellationToken::new();
                let kill_switch =
                    Arc::new(crate::tools::gui_automation::KillSwitchInterceptor::new(
                        workflow_cancellation.clone(),
                        gui_backend,
                    ));

                // ── Batch 2: Workflow Expectation + Autonomy Pre-flight ────────────────────────────
                //
                // 1. Classify workflow category and infer expected visible outcomes.
                // 2. Consult CollaborativeAutonomyEngine to decide proceed/clarify/confirm.
                //    If the decision requires user interaction, emit a clarification and return.
                let psdg_opt = self.world_model.clone();
                let wf_category = {
                    use crate::agent::workflow_expectation::WorkflowExpectationEngine;
                    let wf_eng = WorkflowExpectationEngine::new(psdg_opt.clone());
                    wf_eng.classify(
                        &last_user_text,
                        &spec.primary_verb,
                        &spec.targets,
                        turn_gate_plan.intent.operation,
                    )
                };
                tracing::debug!(
                    target: "gui_htn_routing",
                    category = ?wf_category,
                    "Batch 2: WorkflowExpectationEngine classified workflow"
                );

                // Apply CollaborativeAutonomyEngine pre-execution gate.
                {
                    use crate::agent::collaborative_autonomy::{
                        AutonomyContext, CollaborativeAutonomyEngine,
                    };
                    let autonomy = CollaborativeAutonomyEngine::new(psdg_opt.clone());
                    let mut ctx = AutonomyContext::new(
                        turn_gate_plan.intent.operation,
                        turn_gate_plan.intent.hazard_hint,
                        turn_gate_plan.intent.confidence,
                        format!(
                            "{:?} workflow: {}",
                            wf_category,
                            sanitize_text_for_logs(&last_user_text, 80)
                        ),
                    );
                    if !spec.ambiguities.is_empty() {
                        ctx = ctx.with_ambiguities();
                    }
                    let decision = autonomy.decide(&ctx);
                    tracing::debug!(
                        target: "gui_htn_routing",
                        decision = decision.label(),
                        "Batch 2: CollaborativeAutonomyEngine decision"
                    );
                    // Confirm / Clarify decisions stop execution and ask the user.
                    if decision.requires_user_interaction() {
                        use crate::agent::collaborative_autonomy::AutonomyDecision;
                        let msg = match &decision {
                            AutonomyDecision::Clarify {
                                question, options, ..
                            } => {
                                let opts: Vec<String> =
                                    options.iter().map(|o| format!("- {}", o)).collect();
                                format!("{}\n{}", question, opts.join("\n"))
                            }
                            AutonomyDecision::Confirm {
                                question,
                                consequence_summary,
                                ..
                            } => {
                                format!(
                                    "Confirmation required: {}\n{}",
                                    question, consequence_summary
                                )
                            }
                            AutonomyDecision::Escalate { reason, guidance } => {
                                format!("Cannot proceed: {}\n{}", reason, guidance)
                            }
                            _ => "Please confirm before I proceed.".to_string(),
                        };
                        let _ = event_tx.send(StreamEvent::Token(msg.clone()));
                        let _ = event_tx.send(StreamEvent::Done(msg));
                        return;
                    }
                    // ProceedWithNotice: surface the notice but continue.
                    if let crate::agent::collaborative_autonomy::AutonomyDecision::ProceedWithNotice {
                    ref summary,
                } = decision
                {
                    let _ =
                        event_tx.send(StreamEvent::Plan(format_autonomy_notice_for_user(summary)));
                }
                }

                // Create coordinator and generate workflow
                let mut coordinator = GuiExecutionCoordinator::new(
                    Arc::clone(&self.tool_registry),
                    kill_switch,
                    Arc::clone(&self.policy_engine),
                    Arc::clone(&self.hitl_gateway),
                    Arc::clone(&self.audit_logger),
                );
                // Wire EnvironmentStateTracker so grounding facts persist to PSDG.
                if let Some(ref psdg) = self.world_model {
                    coordinator = coordinator.with_env_tracker(psdg.clone());
                    // Batch 2: also pass raw PsdgHandle to StageExecutor for stage persistence.
                    coordinator = coordinator.with_psdg(psdg.clone());
                }
                // Batch 2: wire continuation_runtime and transparency into GoalTree path.
                if let Some(ref rt) = self.continuation_runtime {
                    coordinator = coordinator.with_continuation_runtime(std::sync::Arc::clone(rt));
                }
                if let Some(ref t) = self.transparency_layer {
                    coordinator = coordinator.with_transparency(t.clone());
                }

                // ── Batch 3 Wave 1: OpGraph multi-intent workflow path ───────────────────────────────
                if let Some(tree) = coordinator
                    .generate_opgraph_workflow(&last_user_text, &turn_gate_plan.intent)
                    .await
                {
                    let _ = event_tx.send(StreamEvent::Plan(format!(
                        "Starting operational workflow: {} ({} stages)",
                        tree.description,
                        tree.stages.len()
                    )));

                    let result = coordinator
                        .execute_goal_tree(
                            &tree,
                            workflow_cancellation.clone(),
                            session_id,
                            &last_user_text,
                        )
                        .await;

                    // ── Batch 2 Step 2: Transparency trace closure ────────────────────────
                    // Complete the GoalTree transparency trace so the per-workflow
                    // lineage record is closed before we return.
                    if let Some(ref t) = self.transparency_layer {
                        t.complete_trace(&tree.workflow_id, result.success, result.error.clone());
                    }

                    // ── Batch 2 Step 2: ObservableCompletionEngine post-GoalTree check ─────
                    // Infer and verify human-visible outcomes after GoalTree execution.
                    // Non-blocking; failures surface as advisory narrative, not errors.
                    let observable_narrative = if result.success {
                        use crate::agent::observable_completion::{
                            infer_outcomes, CompletionVisibilityPolicy, ObservableCompletionEngine,
                        };
                        let oce = ObservableCompletionEngine::new(self.world_model.clone());
                        let outcomes = infer_outcomes(
                            &last_user_text,
                            &spec.primary_verb,
                            &spec.targets,
                            turn_gate_plan.intent.operation,
                        );
                        let policies: Vec<CompletionVisibilityPolicy> = outcomes
                            .into_iter()
                            .map(|o| {
                                CompletionVisibilityPolicy::for_outcome(
                                    o,
                                    turn_gate_plan.intent.operation,
                                )
                            })
                            .collect();
                        if !policies.is_empty() {
                            let aggregate = oce.verify_all(&policies).await;
                            let narrative = oce
                                .completion_narrative(&aggregate, turn_gate_plan.intent.operation);
                            if !narrative.is_empty() {
                                log_pipeline_step(
                                    session_id,
                                    "b2_goal_tree_completion_verified",
                                    &narrative,
                                    None,
                                );
                                Some(narrative)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if result.success {
                        let base_summary = format!(
                            "Completed: {} stage{} in {}ms.",
                            result.stage_results.len(),
                            if result.stage_results.len() == 1 {
                                ""
                            } else {
                                "s"
                            },
                            result.duration_ms
                        );
                        let summary_with_narrative = if let Some(narrative) = observable_narrative {
                            format!("{} {}", base_summary, narrative)
                        } else {
                            base_summary
                        };
                        // Bug 7: Append captured program output when execute_bash was used
                        // for a "run and show output" stage so the result reaches the user.
                        let summary = if let Some(ref output) = result.terminal_output {
                            format!(
                                "{}\n\nProgram output:\n```\n{}\n```",
                                summary_with_narrative,
                                output.trim()
                            )
                        } else {
                            summary_with_narrative
                        };
                        let _ = event_tx.send(StreamEvent::Token(summary.clone()));
                        let _ = event_tx.send(StreamEvent::Done(summary));
                    } else {
                        let detail = result.error.as_deref().unwrap_or("unknown error");

                        // Detect infrastructure-level failures (GLOBAL_SAFETY_HALT) and
                        // surface a human-readable explanation with remediation guidance
                        // instead of the raw internal error string.
                        let is_infra_failure = detail.contains("GLOBAL_SAFETY_HALT")
                            || detail.contains("service not ready")
                            || detail.contains("uinput=stopped")
                            || detail.contains("uinput=FAILED");

                        let summary = if is_infra_failure {
                            // Report which stages completed before the infrastructure failed
                            let succeeded: Vec<&str> = result
                            .stage_results
                            .iter()
                            .filter(|sr| {
                                matches!(
                                    &sr.outcome,
                                    crate::agent::stage_executor::StageOutcome::Passed
                                        | crate::agent::stage_executor::StageOutcome::PassedAfterRecovery
                                )
                            })
                            .map(|sr| sr.label.as_str())
                            .collect();
                            let prefix = if succeeded.is_empty() {
                                String::new()
                            } else {
                                format!("{} completed successfully. ", succeeded.join(", "))
                            };
                            format!(
                            "{}Keyboard automation is unavailable because the GUI input service \
                             (uinput daemon) is not running. KRIA is attempting to restart it \
                             automatically (up to 3 attempts). \
                             If this persists after restarting KRIA, verify that your sudoers file \
                             grants passwordless access: \
                             `<user> ALL=(ALL) NOPASSWD: /path/to/kria-uinput-daemon`",
                            prefix
                        )
                        } else {
                            format!(
                                "Operational workflow failed after {} stage{} ({}).",
                                result.stage_results.len(),
                                if result.stage_results.len() == 1 {
                                    ""
                                } else {
                                    "s"
                                },
                                detail
                            )
                        };
                        let _ = event_tx.send(StreamEvent::Error(summary));
                    }

                    return;
                }

                // First try the deterministic rule-based planner; if it cannot
                // build a concrete workflow, ask the LLM to emit an HTN JSON plan.
                // generate_workflow now returns (workflow, artifacts) so we can
                // populate WorkflowResult.created_artifacts correctly.
                let mut planned_workflow_and_artifacts: Option<(
                    crate::agent::htn_executor::GuiWorkflow,
                    Vec<std::path::PathBuf>,
                )> = coordinator
                    .generate_workflow(session_id, &turn_gate_plan.intent, spec, &last_user_text)
                    .await;

                if planned_workflow_and_artifacts.is_none() {
                    if let Some(backend) = self.model_router.route("chat").await {
                        log_pipeline_step(
                            session_id,
                            "gui_htn_llm_planner",
                            "Rule-based planner produced no workflow; invoking LLM HTN planner",
                            None,
                        );
                        match crate::agent::htn_integration::plan_gui_workflow_via_llm(
                            backend.as_ref(),
                            session_id,
                            &last_user_text,
                        )
                        .await
                        {
                            Ok(wf) => {
                                tracing::info!(
                                    target: "gui_htn_routing",
                                    task_id = %wf.task_id,
                                    steps = wf.sub_goals.len(),
                                    "LLM HTN planner produced workflow"
                                );
                                // LLM planner doesn't track artifacts
                                planned_workflow_and_artifacts = Some((wf, Vec::new()));
                            }
                            Err(e) => {
                                log_pipeline_step(
                                    session_id,
                                    "gui_htn_llm_planner_failed",
                                    "LLM HTN planner failed; falling back to ReAct loop",
                                    Some(serde_json::json!({ "error": e })),
                                );
                            }
                        }
                    } else {
                        log_pipeline_step(
                        session_id,
                        "gui_htn_llm_planner_unavailable",
                        "No chat backend available for LLM HTN planner; falling back to ReAct loop",
                        None,
                    );
                    }
                }

                if let Some((workflow, planned_artifacts)) = planned_workflow_and_artifacts {
                    tracing::info!(
                        target: "gui_execution_trace",
                        session = session_id,
                        workflow_id = %workflow.task_id,
                        steps = workflow.sub_goals.len(),
                        planned_artifacts = planned_artifacts.len(),
                        "[GUI] Workflow Plan Generated"
                    );
                    // ── Phase 3: Structured Telemetry Emission ────────────────────────────────
                    // Create telemetry emitter for this workflow and emit Started event.
                    let step_previews =
                        crate::agent::workflow_telemetry::step_previews_from_workflow(&workflow);
                    let exec_mode = crate::agent::workflow_telemetry::execution_mode_from_previews(
                        &step_previews,
                    );
                    let (telemetry_emitter, _telemetry_receiver) =
                        crate::agent::workflow_telemetry::WorkflowTelemetryEmitter::new(
                            workflow.task_id.clone(),
                            crate::agent::workflow_types::WorkflowSource::SubstrateRouter,
                        );
                    telemetry_emitter.emit_started(
                        &last_user_text,
                        &step_previews,
                        exec_mode,
                        Some((workflow.max_duration_sec as u64) * 1000),
                    );
                    log_pipeline_step(
                        session_id,
                        "workflow_telemetry_started",
                        "Structured telemetry: workflow started",
                        Some(serde_json::json!({
                            "workflow_id": workflow.task_id,
                            "steps": step_previews.len(),
                            "source": "SubstrateRouter",
                        })),
                    );

                    // Emit legacy events (backward compatibility)
                    let _ = event_tx.send(StreamEvent::Plan(format_gui_workflow_start_for_user(
                        &workflow,
                    )));
                    emit_gui_workflow_initial_task_steps(&event_tx, &workflow);

                    // Execute via HTN executor (NOT ReAct loop).
                    // Pass planned_artifacts so WorkflowResult.created_artifacts is populated.
                    tracing::info!(
                        target: "gui_execution_trace",
                        session = session_id,
                        workflow_id = %workflow.task_id,
                        "[GUI] Workflow execution starting"
                    );
                    let result = coordinator
                        .execute_workflow(
                            &workflow,
                            workflow_cancellation,
                            planned_artifacts,
                            session_id,
                            &last_user_text,
                        )
                        .await;
                    tracing::info!(
                        target: "gui_execution_trace",
                        session = session_id,
                        workflow_id = %workflow.task_id,
                        success = result.success,
                        completed_steps = result.completed_steps,
                        total_steps = result.total_steps,
                        duration_ms = result.duration_ms,
                        error = %result.error.as_deref().unwrap_or("-"),
                        "[GUI] Workflow Complete"
                    );

                    // Save session checkpoint with the actual user intent (not just task_id).
                    // This overwrites the checkpoint saved inside execute_workflow with
                    // a more informative one that includes the real user text.
                    crate::agent::gui_wiring::GuiExecutionCoordinator::save_session_checkpoint(
                        &workflow,
                        &result,
                        Some(&last_user_text),
                    )
                    .await;
                    emit_gui_workflow_final_task_steps(&event_tx, &workflow, &result);

                    // ── Batch 2: ObservableCompletionEngine post-execution visibility check ───────
                    // Verify that the expected human-visible outcomes are actually observable.
                    // This check is non-blocking; failures do not fail the workflow.
                    let observable_narrative = {
                        use crate::agent::observable_completion::{
                            infer_outcomes, CompletionVisibilityPolicy, ObservableCompletionEngine,
                        };
                        let oce = ObservableCompletionEngine::new(psdg_opt.clone());
                        let outcomes = infer_outcomes(
                            &last_user_text,
                            &spec.primary_verb,
                            &spec.targets,
                            turn_gate_plan.intent.operation,
                        );
                        let policies: Vec<CompletionVisibilityPolicy> = outcomes
                            .into_iter()
                            .map(|o| {
                                CompletionVisibilityPolicy::for_outcome(
                                    o,
                                    turn_gate_plan.intent.operation,
                                )
                            })
                            .collect();
                        if !policies.is_empty() {
                            let aggregate = oce.verify_all(&policies).await;
                            let narrative = oce
                                .completion_narrative(&aggregate, turn_gate_plan.intent.operation);
                            if !narrative.is_empty() {
                                Some(narrative)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    };

                    // ── Canonical Workflow Verdict (Phase 2) ──────────────────────────────────
                    // Replace the contradictory dual-path (format_gui_workflow_partial vs
                    // format_gui_workflow_success) with a single canonical verdict computation.
                    // This eliminates the "Task completed" + "outcome not visible" contradiction.
                    let verdict_computation =
                        crate::agent::workflow_verdict::verdict_from_legacy_result(
                            result.success,
                            result.completed_steps,
                            result.total_steps,
                            result.error.as_deref(),
                            observable_narrative.as_deref(),
                        );

                    // ── Phase 3: Emit structured completion telemetry ─────────────────────────
                    telemetry_emitter.emit_completed(
                        verdict_computation.verdict.clone(),
                        &verdict_computation.narrative,
                        result
                            .created_artifacts
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect(),
                        vec![], // continuation actions (future: derive from verdict)
                    );

                    // Emit structured telemetry for the new runtime
                    log_pipeline_step(
                        session_id,
                        "workflow_verdict",
                        "Canonical workflow verdict computed",
                        Some(serde_json::json!({
                            "verdict": format!("{:?}", verdict_computation.verdict),
                            "visibility_confidence": format!("{:?}", verdict_computation.visibility_confidence),
                            "completed_steps": result.completed_steps,
                            "total_steps": result.total_steps,
                        })),
                    );

                    // Generate user-facing summary from the canonical verdict
                    let summary = match &verdict_computation.verdict {
                        crate::agent::workflow_types::WorkflowVerdict::Complete => {
                            format_gui_workflow_success_for_user(
                                &result,
                                observable_narrative.as_deref(),
                            )
                        }
                        crate::agent::workflow_types::WorkflowVerdict::StructurallyComplete {
                            unverified_outcomes,
                        } => {
                            // Honest reporting: structural success + visibility gap
                            let mut lines = vec![format!(
                                "Task completed structurally. KRIA verified {} step{} in {}ms.",
                                result.completed_steps,
                                if result.completed_steps == 1 { "" } else { "s" },
                                result.duration_ms
                            )];
                            if !unverified_outcomes.is_empty() {
                                lines.push(format!(
                                    "Visibility unverified: {}",
                                    unverified_outcomes.join(", ")
                                ));
                            }
                            if let Some(artifacts) =
                                artifact_summary_for_user(&result.created_artifacts)
                            {
                                lines.push(artifacts);
                            }
                            if let Some(output) =
                                output_preview_from_artifacts(&result.created_artifacts)
                            {
                                lines.push(format!("Captured output:\n```\n{}\n```", output));
                            }
                            lines.join("\n\n")
                        }
                        crate::agent::workflow_types::WorkflowVerdict::Failed { .. } => {
                            format_gui_workflow_failure_for_user(&result)
                        }
                        _ => verdict_computation.narrative.clone(),
                    };

                    tracing::info!(
                        target: "gui_htn_routing",
                        success = result.success,
                        completed = result.completed_steps,
                        total = result.total_steps,
                        verdict = ?verdict_computation.verdict,
                        "Emitting GUI workflow result with canonical verdict"
                    );

                    // Emit to frontend
                    if result.success {
                        let _ = event_tx.send(StreamEvent::Token(summary.clone()));
                        let _ = event_tx.send(StreamEvent::Done(summary));
                    } else {
                        // Emit structured recovery options for failed GUI workflows
                        let error_detail = result.error.as_deref().unwrap_or("unknown error");
                        let recovery_options = build_workflow_failure_recovery_options(
                            error_detail,
                            &last_user_text,
                            &result.created_artifacts,
                        );
                        if !recovery_options.is_empty() {
                            let _ = event_tx.send(StreamEvent::RecoveryOptions {
                                context: "Workflow did not complete fully".to_string(),
                                detail: error_detail.to_string(),
                                options: recovery_options,
                            });
                        }
                        let _ = event_tx.send(StreamEvent::Error(summary.clone()));
                        let _ = event_tx.send(StreamEvent::Done(summary));
                    }

                    // EARLY RETURN - completely bypass ReAct loop
                    return;
                } else {
                    log_pipeline_step(
                        session_id,
                        "gui_htn_fallback",
                        "GUI routing detected but no workflow generated; falling back to ReAct",
                        None,
                    );
                }
            } // closes: if !router_redirected_to_react
        }

        // ═══════════════════════════════════════════════════════════════════════════
        // Standard ReAct Loop (continues for non-GUI intents)
        // ═══════════════════════════════════════════════════════════════════════════

        // ─── Deterministic Dispatch Fast-Path ──────────────────────────────────
        // For prompts that clearly map to a specific tool with extractable parameters,
        // dispatch directly without an LLM round-trip. This is critical for:
        // - System info queries (whoami, uname, hostname)
        // - Filesystem operations (mkdir, write_file, list_files)
        // - Browser searches with explicit query
        //
        // The function scans the prompt directly and selects the best tool —
        // bypassing whatever (potentially-wrong) tool_hint the router suggested.
        // When the LLM is unavailable, this path still works.
        if let Some((tool_name, deterministic_params)) = try_deterministic_dispatch_with_profile(
            &last_user_text,
            previous_user_text.as_deref(),
            &execution_profile,
        ) {
            if tool_name == KRIA_DETERMINISTIC_NOTICE_TOOL {
                let message = deterministic_notice_message(&deterministic_params);
                log_pipeline_step(
                    session_id,
                    "deterministic_notice",
                    "Prompt matched local deterministic notice — responding without LLM or tool execution",
                    Some(serde_json::json!({
                        "manual_tool_override": execution_profile.is_manual_tool_override(),
                        "app_lock": execution_profile.app_lock.clone(),
                        "tool_lock": execution_profile.tool_lock.clone(),
                    })),
                );
                let _ = event_tx.send(StreamEvent::Token(message.clone()));
                let _ = event_tx.send(StreamEvent::Done(message));
                return;
            } else if !execution_profile.allows_tool_name(&tool_name) {
                log_pipeline_step(
                    session_id,
                    "deterministic_dispatch_blocked_by_execution_profile",
                    "Deterministic tool match ignored because it is outside the active manual tool mode",
                    Some(serde_json::json!({
                        "tool": tool_name,
                        "manual_tool_override": execution_profile.is_manual_tool_override(),
                        "app_lock": execution_profile.app_lock.clone(),
                        "tool_lock": execution_profile.tool_lock.clone(),
                    })),
                );
            } else if let Some(handler) = self.tool_registry.get_handler(&tool_name) {
                log_pipeline_step(
                    session_id,
                    "deterministic_dispatch",
                    "Prompt matched a deterministic pattern — bypassing LLM",
                    Some(serde_json::json!({
                        "tool": tool_name,
                        "params_keys": deterministic_params.as_object().map(|o| o.keys().collect::<Vec<_>>()),
                    })),
                );

                // Emit in-progress indicator for n8n workflows (visible in chat)
                if tool_name == "n8n_invoke_workflow" {
                    let wf_display = deterministic_params
                        .get("workflow_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("workflow");
                    let _ = event_tx.send(StreamEvent::Token(format!(
                        "⏳ Running workflow '{}'...\n",
                        wf_display
                    )));
                }

                let handler = handler.clone();
                let tool_context = self
                    .tool_registry
                    .make_tool_context(tokio_util::sync::CancellationToken::new());

                let tool_execution_id = Uuid::now_v7().to_string();
                let tool_execution_started = std::time::Instant::now();
                tracing::info!(
                    target: "tool_execution",
                    session = session_id,
                    execution_id = %tool_execution_id,
                    tool_name = %tool_name,
                    input_summary = %sanitize_json_for_logs(&deterministic_params, 220, 8),
                    "Tool execution started"
                );

                let dispatch_result = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    handler.execute_with_context(deterministic_params.clone(), tool_context),
                )
                .await;

                match dispatch_result {
                    Ok(result) => {
                        tracing::info!(
                            target: "tool_execution",
                            session = session_id,
                            execution_id = %tool_execution_id,
                            tool_name = %tool_name,
                            duration_ms = tool_execution_started.elapsed().as_millis(),
                            success = result.success,
                            failure_reason = %result.error.as_deref().unwrap_or("-"),
                            result_summary = %sanitize_json_for_logs(&result.data, 220, 8),
                            "Tool execution completed"
                        );
                        let summary = if result.error.is_some() {
                            format!(
                                "Tool '{}' completed with error: {}",
                                tool_name,
                                result.error.as_deref().unwrap_or("unknown")
                            )
                        } else if tool_name == "n8n_invoke_workflow" {
                            // Semantic formatting for n8n workflow results
                            format_n8n_result(&result.data)
                        } else {
                            let output_str = if let Some(s) = result.data.as_str() {
                                s.to_string()
                            } else {
                                serde_json::to_string_pretty(&result.data).unwrap_or_default()
                            };
                            let truncated = if output_str.len() > 1500 {
                                format!("{}...", &output_str[..1500])
                            } else {
                                output_str
                            };
                            format!("✓ {}\n\n{}", tool_name, truncated)
                        };

                        log_pipeline_step(
                            session_id,
                            "deterministic_dispatch_complete",
                            "Direct tool dispatch succeeded — skipped LLM",
                            Some(serde_json::json!({
                                "tool": tool_name,
                                "has_error": result.error.is_some(),
                            })),
                        );

                        let _ = event_tx.send(StreamEvent::Token(summary.clone()));
                        let _ = event_tx.send(StreamEvent::Done(summary));
                        return;
                    }
                    Err(_) => {
                        tracing::warn!(
                            target: "tool_execution",
                            session = session_id,
                            execution_id = %tool_execution_id,
                            tool_name = %tool_name,
                            duration_ms = tool_execution_started.elapsed().as_millis(),
                            "Tool execution timed out"
                        );
                        tracing::warn!(
                            target: "deterministic_dispatch",
                            tool = %tool_name,
                            "Deterministic dispatch timed out — falling back to ReAct loop"
                        );
                        // Fall through to ReAct loop
                    }
                }
            } else {
                tracing::debug!(
                    target: "deterministic_dispatch",
                    tool = %tool_name,
                    "Deterministic dispatch matched but tool handler not registered"
                );
            }
        }
        // ─── End deterministic dispatch fast-path ──────────────────────────────

        // ── Failover-aware backend selection ────────────────────────────────────
        // When a FailoverRouter is attached, it intercepts route() calls and
        // applies FSM-based health tracking. When absent, we delegate directly
        // to model_router (existing behavior, zero overhead).
        //
        // The `_is_fallback` flag is used after the call to record the result
        // in the FSM via `on_call_result()`.
        let (backend, _is_fallback) = if wants_vision_backend {
            if inline_images_allowed_for_turn {
                let (b, is_fb) = if let Some(ref fr) = self.failover_router {
                    fr.route_vision().await
                } else {
                    (self.model_router.route_vision().await, false)
                };
                match b {
                    Some(b) => (b, is_fb),
                    None => {
                        log_pipeline_step(
                            session_id,
                            "backend_unavailable",
                            "No vision backend available despite enabled VisionMode; falling back to chat backend",
                            Some(serde_json::json!({
                                "requested": "vision",
                                "fallback": "chat_backend_inline_images_preserved",
                                "vision_mode": inline_image_vision_mode.as_str(),
                            })),
                        );
                        let (b2, is_fb2) = if let Some(ref fr) = self.failover_router {
                            fr.route("chat").await
                        } else {
                            (self.model_router.route("chat").await, false)
                        };
                        match b2 {
                            Some(b) => (b, is_fb2),
                            None => {
                                let _ = event_tx
                                    .send(StreamEvent::Error("no LLM backend available".into()));
                                return;
                            }
                        }
                    }
                }
            } else {
                log_pipeline_step(
                    session_id,
                    "vision_mode_disabled",
                    "VisionMode is disabled for this runtime; stripping inline images for LLM rounds",
                    Some(serde_json::json!({
                        "vision_mode": inline_image_vision_mode.as_str(),
                    })),
                );
                let (b, is_fb) = if let Some(ref fr) = self.failover_router {
                    fr.route("chat").await
                } else {
                    (self.model_router.route("chat").await, false)
                };
                match b {
                    Some(b) => (b, is_fb),
                    None => {
                        let _ =
                            event_tx.send(StreamEvent::Error("no LLM backend available".into()));
                        return;
                    }
                }
            }
        } else {
            let (b, is_fb) = if let Some(ref fr) = self.failover_router {
                fr.route("chat").await
            } else {
                (self.model_router.route("chat").await, false)
            };
            match b {
                Some(b) => (b, is_fb),
                None => {
                    log_pipeline_step(
                        session_id,
                        "backend_unavailable",
                        "No chat backend available",
                        Some(serde_json::json!({ "requested": "chat" })),
                    );
                    let _ = event_tx.send(StreamEvent::Error("no LLM backend available".into()));
                    return;
                }
            }
        };

        log_pipeline_step(
            session_id,
            "backend_selected",
            "Model backend selected",
            Some(serde_json::json!({
                "model_label": backend.model_label(),
                "capabilities": backend.capabilities(),
            })),
        );

        // Auto-mount tool groups based on user message keywords
        let mut meet_fallback_metadata: Option<serde_json::Value> = None;
        if pure_image_analysis_turn {
            log_pipeline_step(
                session_id,
                "preprocessing_skipped",
                "Skipped keyword auto-mount for pure image analysis turn",
                None,
            );
        } else if let Some(last_msg) = messages.last() {
            if last_msg.role == "user" {
                let mount_probe_text = routing_focus_text_from_user_content(&last_msg.content);
                meet_fallback_metadata = google_meet_fallback_metadata(&mount_probe_text);
                let mut mm = self.mount_manager.write().await;
                let newly = mm.auto_mount_from_message(&mount_probe_text);
                if !newly.is_empty() {
                    tracing::info!(groups = ?newly, "auto-mounted tool groups from user message");
                    log_pipeline_step(
                        session_id,
                        "preprocessing_applied",
                        "Tool auto-mount preprocessing applied",
                        Some(serde_json::json!({ "mounted_groups": newly })),
                    );
                } else {
                    log_pipeline_step(
                        session_id,
                        "preprocessing_skipped",
                        "No tool auto-mount preprocessing needed",
                        None,
                    );
                }
            }
        }

        if let Some(metadata) = meet_fallback_metadata {
            let metadata_json =
                serde_json::to_string_pretty(&metadata).unwrap_or_else(|_| metadata.to_string());
            messages.push(ChatMessage {
                role: "system".into(),
                content: format!(
                    "Google Meet fallback metadata:\n{}\nTool selection rule: when the user requests Google Meet/video-call scheduling, use Calendar conference-link mode with gw_calendar_create (and gw_calendar_search for availability checks).",
                    metadata_json
                ),
                name: None,
                images: None,
            });

            let _ = event_tx.send(StreamEvent::Plan(
                "Applying Google Meet fallback via Calendar conference-link mode metadata".into(),
            ));

            log_pipeline_step(
                session_id,
                "preprocessing_applied",
                "Google Meet fallback metadata injected",
                Some(serde_json::json!({
                    "metadata": sanitize_json_for_logs(&metadata, 220, 8),
                })),
            );
        }
        let google_workspace_intent =
            !pure_image_analysis_turn && looks_like_google_workspace_request(&routing_focus_lower);

        // ── Colab workflow: inject tool-routing guidance into context ──────────
        // This tells the LLM exactly which tools map to each Colab sub-task so
        // it never hallucinates a "colab create" verb.
        if !pure_image_analysis_turn && looks_like_colab_request(&routing_focus_lower) {
            let colab_guidance = concat!(
                "TOOL ROUTING RULES for Google Colab requests:\n",
                "1. CREATE a new Colab notebook → call `gw_drive_create` with mime_type=\"application/vnd.google.colab\", then call `mcp_colab-mcp_open_colab_browser_connection`.\n",
                "2. OPEN an existing Colab notebook / set active → call `mcp_colab-mcp_open_colab_browser_connection` (this opens the Colab tab in the browser).\n",
                "3. RUN / EXECUTE code in Colab → first ensure browser is connected via `mcp_colab-mcp_open_colab_browser_connection`, then call `mcp_colab-mcp_execute_cell` with the code.\n",
                "NEVER output plain text like 'colab create ...' — always emit a structured tool call JSON.",
            );
            messages.push(ChatMessage {
                role: "system".into(),
                content: colab_guidance.to_string(),
                name: None,
                images: None,
            });
            log_pipeline_step(
                session_id,
                "preprocessing_applied",
                "Colab tool-routing guidance injected",
                None,
            );
        }

        // Build tool schemas for the LLM (filtered by mount manager)
        let mount_mgr = self.mount_manager.read().await;
        let tool_defs = self.tool_registry.list_for_tier(&self.hardware_tier);
        let mut tool_schemas: Vec<ToolSchema> = tool_defs
            .iter()
            .filter(|d| mount_mgr.is_mounted(&d.name))
            .filter(|d| {
                if pure_image_analysis_turn {
                    is_tool_allowed_for_image_focus(d)
                } else {
                    true
                }
            })
            .filter(|d| {
                if d.name.starts_with("mcp_gworkspace_") {
                    google_workspace_intent
                } else {
                    true
                }
            })
            .filter(|d| tool_allowed_by_execution_profile(&execution_profile, &d.name))
            .map(|d| ToolSchema {
                name: d.name.clone(),
                description: d.description.clone(),
                parameters: d.to_function_schema()["function"]["parameters"].clone(),
            })
            .collect();
        tool_schemas.sort_by(|a, b| a.name.cmp(&b.name));
        let allowed_tool_names: HashSet<String> =
            tool_schemas.iter().map(|s| s.name.clone()).collect();
        drop(mount_mgr);

        let prompt_lab_direct_mode = execution_profile.uses_direct_strategy();

        log_pipeline_step(
            session_id,
            "tool_schemas_built",
            "Prepared mounted tool schemas for LLM",
            Some(serde_json::json!({
                "google_workspace_intent": google_workspace_intent,
                "pure_image_analysis_turn": pure_image_analysis_turn,
                "prompt_lab_mode": execution_profile.is_prompt_lab(),
                "manual_tool_override": execution_profile.is_manual_tool_override(),
                "prompt_lab_direct_mode": prompt_lab_direct_mode,
                "tool_count": tool_schemas.len(),
                "tool_names": tool_schemas
                    .iter()
                    .map(|schema| schema.name.clone())
                    .collect::<Vec<_>>(),
            })),
        );

        // Track tools already approved in this user-turn to avoid re-asking.
        // Key: "tool_name|args_json"
        let mut approved_this_turn: HashSet<String> = HashSet::new();
        let mut package_flow = PackageFlowState::from_user_text(&routing_focus_text);
        let mut capability_flow = CapabilityFlowState::from_user_text(&routing_focus_text);
        let mut colab_flow = ColabFlowState::from_user_text(&routing_focus_text);
        let mut intent_fallback_used = false;
        let mut had_successful_gmail_tool = false;
        let mut had_failed_gmail_tool = false;
        let mut last_successful_gmail_result: Option<serde_json::Value> = None;
        let mut last_successful_image_result: Option<serde_json::Value> = None;
        let forced_tool_directive = extract_forced_tool_directive(&routing_focus_text);
        let forced_tool_requested = forced_tool_directive.is_some();
        let forced_tool_name = forced_tool_directive.as_ref().map(|(name, _)| name.clone());
        let initial_turn_gate_tool_hint = self
            .turn_gate
            .direct_tool_hint(&turn_gate_plan, &allowed_tool_names);
        // Capture turn-level routing embedding for feedback (Gap 1 fix).
        // Embed once here and reuse for all tool calls in this turn.
        let turn_query_embedding: Vec<f32> = if self.semantic_router.is_some() {
            crate::routing::embed::embed_batch(&[routing_focus_text.as_str()])
                .ok()
                .and_then(|mut v| v.pop())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut turn_modality = if let Some(router) = &self.semantic_router {
            let ctx = self.turn_gate.context();
            let (route_decision, modality, _) =
                router.route_with_context(&routing_focus_text, ctx).await;
            // Re-evaluate IntentGate with semantic router result for better accuracy
            // This handles cases where the gate was uncertain but the router is confident
            if gate_decision.fast_path {
                // Gate already decided conversational — trust it, don't override
            } else if matches!(route_decision, crate::routing::RouteDecision::Conversation) {
                // Semantic router says conversational — log this for observability
                tracing::debug!(
                    session = session_id,
                    gate_intent = gate_decision.intent.as_str(),
                    "semantic router confirms conversational intent"
                );
            }
            modality
        } else {
            crate::routing::verbs::classify_modality(&routing_focus_text)
        };
        let base_system_prompt_template = messages
            .first()
            .filter(|m| m.role.eq_ignore_ascii_case("system"))
            .map(|m| m.content.clone());

        // ── PSDG Context Injection (Batch 1) ──────────────────────────────────
        // Inject a compact semantic desktop context block into the system prompt
        // for operations that benefit from desktop state awareness.
        // Fire-and-forget: snapshot read is sync/bounded; never blocks the turn.
        if let Some(ref psdg) = self.world_model {
            let operation = turn_gate_plan.intent.operation;
            if crate::agent::psdg::context_injector::should_inject_context(operation) {
                let snapshot = psdg.get_context_snapshot();
                if !snapshot.is_empty() {
                    // Inject into the first system message
                    if let Some(system_msg) = messages
                        .first_mut()
                        .filter(|m| m.role.eq_ignore_ascii_case("system"))
                    {
                        system_msg.content =
                            crate::agent::psdg::context_injector::inject_into_system_prompt(
                                &system_msg.content,
                                &snapshot,
                                operation,
                            );
                    }
                    tracing::debug!(
                        target: "psdg",
                        session = session_id,
                        operation = ?operation,
                        has_app = snapshot.focused_app.is_some(),
                        has_browser = snapshot.browser_url.is_some(),
                        has_ide = snapshot.ide_workspace.is_some(),
                        "PSDG context injected into system prompt"
                    );
                }
            }
        }

        log_pipeline_step(
            session_id,
            "intent_classified",
            "Intent classification complete",
            Some(serde_json::json!({
                "turn_gate_operation": format!("{:?}", turn_gate_plan.intent.operation),
                "turn_gate_source": format!("{:?}", turn_gate_plan.intent.source),
                "turn_gate_tool_hint": initial_turn_gate_tool_hint,
                "confidence": turn_gate_plan.intent.confidence,
                "forced_tool_requested": forced_tool_requested,
                "package_flow_detected": package_flow.is_some(),
                "colab_flow_detected": colab_flow.is_some(),
            })),
        );

        // ── Batch 2 Phase 1: Workflow expectation classification + outcome inference ──
        // Classifies the workflow category from user intent so expectations are
        // established before execution begins. Outcome inference is used post-loop
        // for observable completion verification (Phase 1 closure).
        let b2_workflow_category = self.workflow_expectation.as_ref().map(|eng| {
            let cat = eng.classify(
                &last_user_text,
                &crate::agent::intent_compiler::Verb::Other(String::new()),
                &[],
                turn_gate_plan.intent.operation,
            );
            tracing::debug!(
                target: "workflow_expectation",
                session = session_id,
                category = ?cat,
                operation = ?turn_gate_plan.intent.operation,
                "Batch 2: workflow classified by expectation engine"
            );
            cat
        });
        let b2_inferred_outcomes: Vec<crate::agent::observable_completion::ObservableOutcome> =
            if self.observable_completion.is_some() {
                crate::agent::observable_completion::infer_outcomes(
                    &last_user_text,
                    &crate::agent::intent_compiler::Verb::Other(String::new()),
                    &[],
                    turn_gate_plan.intent.operation,
                )
            } else {
                vec![]
            };
        if b2_workflow_category.is_some() || !b2_inferred_outcomes.is_empty() {
            log_pipeline_step(
                session_id,
                "b2_workflow_expectation",
                "Batch 2: workflow expectation and outcome inference complete",
                Some(serde_json::json!({
                    "category": format!("{:?}", b2_workflow_category),
                    "inferred_outcome_count": b2_inferred_outcomes.len(),
                    "operation": format!("{:?}", turn_gate_plan.intent.operation),
                })),
            );
        }

        // ── Batch 2 Phase 3: Per-turn collaborative autonomy decision ─────────────
        // Consults the autonomy engine with operation + confidence context.
        // Non-blocking: advisory only — HITL + PolicyEngine remain safety authority.
        if let Some(ref eng) = self.collaborative_autonomy {
            use crate::agent::collaborative_autonomy::{AutonomyContext, AutonomyDecision};
            let ctx = AutonomyContext::new(
                turn_gate_plan.intent.operation,
                crate::agent::turn_gate::HazardHint::Green,
                turn_gate_plan.intent.confidence as f32,
                last_user_text.chars().take(80).collect::<String>(),
            );
            let decision = eng.decide(&ctx);
            match &decision {
                AutonomyDecision::ProceedWithNotice { summary } => {
                    tracing::info!(
                        target: "collaborative_autonomy",
                        session = session_id,
                    %summary,
                    "Batch 2 autonomy: proceeding with notice"
                    );
                    log_pipeline_step(session_id, "b2_autonomy_notice", summary, None);
                    let _ =
                        event_tx.send(StreamEvent::Plan(format_autonomy_notice_for_user(summary)));
                }
                AutonomyDecision::Clarify { question, .. } => {
                    tracing::info!(
                        target: "collaborative_autonomy",
                        session = session_id,
                        %question,
                        "Batch 2 autonomy: clarification advisory (non-blocking)"
                    );
                    log_pipeline_step(session_id, "b2_autonomy_clarify", question, None);
                }
                AutonomyDecision::Escalate { reason, .. } => {
                    tracing::warn!(
                        target: "collaborative_autonomy",
                        session = session_id,
                        %reason,
                        "Batch 2 autonomy: escalation advisory (non-blocking)"
                    );
                    log_pipeline_step(session_id, "b2_autonomy_escalate", reason, None);
                }
                _ => {}
            }
        }

        // ── Batch 2: Pre-turn resumable workflow advisory ─────────────────────────
        // Check for paused workflows from previous sessions. Advisory-only —
        // surfaces a structured notice to the user via log_pipeline_step.
        if let Some(ref rt) = self.continuation_runtime {
            let resumable = rt.find_resumable();
            if !resumable.is_empty() {
                let hints: Vec<String> = resumable
                    .iter()
                    .take(3)
                    .map(|s| {
                        format!(
                            "'{}' ({})",
                            s.session_id,
                            s.user_intent.chars().take(60).collect::<String>()
                        )
                    })
                    .collect();
                tracing::info!(
                    target: "workflow_continuation",
                    session = session_id,
                    count = resumable.len(),
                    "Batch 2: found resumable workflows from previous sessions"
                );
                log_pipeline_step(
                    session_id,
                    "b2_resumable_found",
                    &format!(
                        "{} paused workflow(s) available for resumption: {}",
                        resumable.len(),
                        hints.join("; ")
                    ),
                    Some(serde_json::json!({ "count": resumable.len() })),
                );
            }
        }

        // ── Batch 2 Step 3: TurnGate fast-path for resume intent ─────────────────
        // If the user says "resume", "continue paused", or similar, and there are
        // resumable workflows, surface them immediately without entering the LLM loop.
        // The LLM can still call the resume_workflow tool during the round loop —
        // this fast-path is purely advisory and exits early only when unambiguous.
        if let Some(ref rt) = self.continuation_runtime {
            let text_lower = last_user_text.to_lowercase();
            let is_resume_intent = text_lower.starts_with("resume")
                || text_lower.starts_with("continue paused")
                || text_lower.starts_with("pick up where")
                || text_lower.starts_with("carry on with")
                || text_lower.starts_with("restart paused");
            if is_resume_intent {
                let resumable = rt.find_resumable();
                if !resumable.is_empty() {
                    // Surface the most recent resumable session and emit a structured response.
                    let top = &resumable[0];
                    let result = rt.resume_workflow(&top.session_id);
                    tracing::info!(
                        target: "workflow_continuation",
                        session = session_id,
                        target_session = %top.session_id,
                        success = result.success,
                        "Batch 2: TurnGate fast-path triggered resume_workflow"
                    );
                    log_pipeline_step(
                        session_id,
                        "b2_resume_fast_path",
                        &result.summary,
                        Some(serde_json::json!({
                            "session_id": top.session_id,
                            "success": result.success,
                            "next_action": format!("{:?}", result.next_action),
                        })),
                    );
                    let response = if result.success {
                        format!(
                            "Resuming workflow '{}': {}\n\nContinuation hint: {}",
                            top.session_id,
                            result.summary,
                            result
                                .session
                                .as_ref()
                                .and_then(|s| s.continuation_hint.clone())
                                .unwrap_or_else(|| "Continue from last checkpoint".into()),
                        )
                    } else {
                        format!(
                            "Could not resume workflow '{}': {}",
                            top.session_id, result.summary
                        )
                    };
                    let _ = event_tx.send(StreamEvent::Token(response.clone()));
                    let _ = event_tx.send(StreamEvent::Done(response));
                    return;
                }
            }
        }

        let mut terminated_by_satisfaction = false;
        for round in 0..self.max_tool_rounds {
            if return_if_stale() {
                return;
            }

            // ── Round-level satisfaction guard ────────────────────────────
            // If a prior round satisfied the user's goal, do not run another
            // LLM round with tool schemas. Break out of the round loop and
            // proceed to final response synthesis.
            if turn_memory.is_satisfied() {
                tracing::info!(
                    session = session_id,
                    round,
                    reason = %turn_memory.satisfaction_reason().unwrap_or(""),
                    "round_loop: turn satisfied, breaking before round start"
                );
                log_pipeline_step(
                    session_id,
                    "round_loop_satisfied",
                    "Round loop terminating: goal satisfied",
                    Some(serde_json::json!({
                        "round": round,
                        "reason": turn_memory.satisfaction_reason(),
                        "memory": turn_memory.to_json(),
                    })),
                );
                terminated_by_satisfaction = true;
                break;
            }

            let round_user_text: String = messages
                .iter()
                .rev()
                .find(|m| m.role == "user")
                .map(|m| m.content.clone())
                .unwrap_or_else(|| routing_focus_text_from_user_content(&last_user_text));
            let round_focus_text = routing_focus_text_from_user_content(&round_user_text);
            let mut routed_tool_names: HashSet<String> = HashSet::new();
            let mut conversation_only_route = false;

            if let Some(router) = &self.semantic_router {
                let ctx = self.turn_gate.context();
                let (decision, modality, trace) =
                    router.route_with_context(&round_focus_text, ctx).await;
                turn_modality = modality;
                conversation_only_route =
                    matches!(decision, crate::routing::RouteDecision::Conversation);
                routed_tool_names.extend(trace.selected_tools);
            }
            let round_direct_tool_hint = self
                .turn_gate
                .direct_tool_hint(&turn_gate_plan, &allowed_tool_names);

            // ── AUTHORITATIVE FAST-PATH GUARD ──────────────────────────────
            // If IntentGate classified this turn as conversational fast-path,
            // hard-block ALL tool routing, retrieval, semantic injection, and
            // schema selection. The LLM gets ZERO tool schemas and answers
            // directly. This is the single source of authority for "no tools
            // on conversational prompts."
            //
            // Also blocks if the turn goal is already satisfied — no need to
            // re-route tools after the user's request has been fulfilled.
            let suppress_all_tool_routing = gate_decision.fast_path || turn_memory.is_satisfied();

            let fallback_tool_names = if suppress_all_tool_routing {
                HashSet::new()
            } else {
                fallback_routed_tool_candidates(
                    &round_focus_text,
                    round_direct_tool_hint.as_deref(),
                    &allowed_tool_names,
                )
            };

            // ── Cross-domain semantic tool injection (Hybrid Assembly) ──
            // Query the FastEmbed index for the Top-3 most semantically
            // relevant tools **regardless of ONNX domain boundaries**.
            // This runs unconditionally so the results are available for
            // the override instruction below.
            let semantic_injections: Vec<SemanticInjection> = if suppress_all_tool_routing
                || execution_profile.is_manual_tool_override()
            {
                Vec::new()
            } else if self.tool_index.is_some() && !pure_image_analysis_turn {
                if let Some(ref tool_index) = self.tool_index {
                    // Fetch a wider candidate pool so the gate can reason about
                    // candidate competition, then decide with fused evidence
                    // (domain agreement + negative evidence + competition) rather
                    // than a flat confidence floor (Wave 5).
                    let matches = tool_index
                        .top_k_by_text(&round_focus_text, 5, &self.hardware_tier)
                        .await;
                    let candidates: Vec<injection_gate::InjectionCandidate> = matches
                        .into_iter()
                        .map(|m| injection_gate::InjectionCandidate {
                            name: m.name,
                            category: m.category,
                            confidence: m.confidence,
                        })
                        .collect();
                    let domain_categories =
                        self.tool_categories_for(&routed_tool_names, &tool_schemas);
                    let evidence = injection_gate::InjectionEvidence {
                        domain_categories,
                        conversation_only: conversation_only_route,
                    };
                    let decision = injection_gate::gate(
                        candidates,
                        &evidence,
                        &injection_gate::InjectionParams::default(),
                    );
                    // Persist the explainable routing decision (observability).
                    if !decision.trace.is_empty() {
                        tracing::debug!(
                            target: "tool_injection",
                            accepted = ?decision.accepted.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
                            trace = ?decision.trace.iter().map(|t| format!("{}:{:.2}:{}:{}", t.name, t.score, t.domain_agree, t.reason)).collect::<Vec<_>>(),
                            conversation_only = conversation_only_route,
                            "cross-domain injection gate decision"
                        );
                    }
                    decision
                        .accepted
                        .into_iter()
                        .map(|c| SemanticInjection {
                            name: c.name,
                            cosine_similarity: c.confidence,
                        })
                        .collect()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            if !semantic_injections.is_empty() {
                tracing::debug!(
                    tools = ?semantic_injections.iter().map(|i| format!("{} ({:.2})", i.name, i.cosine_similarity)).collect::<Vec<_>>(),
                    "Semantic tool injection candidates"
                );
            }

            let round_tool_schemas = if pure_image_analysis_turn || suppress_all_tool_routing {
                Vec::new()
            } else {
                // Phase 3: Try direct tool match first (skip LLM)
                let direct_semantic_match = if execution_profile.is_manual_tool_override() {
                    None
                } else {
                    self.try_direct_tool_match(&round_focus_text).await
                };
                if let Some(direct_schema) = direct_semantic_match {
                    if allowed_tool_names.contains(&direct_schema.name)
                        && execution_profile.allows_tool_name(&direct_schema.name)
                    {
                        tracing::info!(
                            tool = %direct_schema.name,
                            manual_tool_override = execution_profile.is_manual_tool_override(),
                            "Direct tool match via semantic index — skipping LLM"
                        );
                        vec![direct_schema]
                    } else {
                        tracing::info!(
                            tool = %direct_schema.name,
                            manual_tool_override = execution_profile.is_manual_tool_override(),
                            "Direct tool match ignored because it is outside the active execution profile"
                        );
                        select_routed_tool_schemas(
                            &tool_schemas,
                            &round_focus_text,
                            round_direct_tool_hint.as_deref(),
                            &routed_tool_names,
                            &fallback_tool_names,
                            forced_tool_name.as_deref(),
                            execution_profile.tool_lock.as_deref(),
                            conversation_only_route,
                            &semantic_injections,
                        )
                    }
                } else {
                    select_routed_tool_schemas(
                        &tool_schemas,
                        &round_focus_text,
                        round_direct_tool_hint.as_deref(),
                        &routed_tool_names,
                        &fallback_tool_names,
                        forced_tool_name.as_deref(),
                        execution_profile.tool_lock.as_deref(),
                        conversation_only_route,
                        &semantic_injections,
                    )
                }
            };

            if suppress_all_tool_routing {
                log_pipeline_step(
                    session_id,
                    "tool_routing_suppressed",
                    "Authoritative fast-path: tool routing fully suppressed",
                    Some(serde_json::json!({
                        "round": round,
                        "reason": if gate_decision.fast_path { "intent_gate_fast_path" } else { "turn_satisfied" },
                        "intent": gate_decision.intent.as_str(),
                    })),
                );
            }

            // ── Cross-domain override instruction ──────────────────────────────
            // When semantic injection brings in tools that the ONNX domain
            // didn't select, inject an explicit system instruction so the LLM
            // prioritises them over the domain-default tools.
            {
                let domain_names = &routed_tool_names;
                let injected_names: Vec<&str> = round_tool_schemas
                    .iter()
                    .filter(|s| {
                        !domain_names.contains(&s.name)
                            && (semantic_injections.iter().any(|i| i.name == s.name)
                                || fallback_tool_names.contains(&s.name))
                    })
                    .map(|s| s.name.as_str())
                    .collect();
                if !injected_names.is_empty() {
                    let override_msg = format!(
                        "CRITICAL TOOL OVERRIDE: The following tool(s) are semantically \
                         matched to the user's request and MUST be preferred over any \
                         web/news/search tools: {}. \
                         Use them first. Only fall back to web search if these tools fail.",
                        injected_names.join(", ")
                    );
                    messages.push(ChatMessage {
                        role: "system".into(),
                        content: override_msg,
                        name: None,
                        images: None,
                    });
                }
            }

            if let Some(template) = base_system_prompt_template.as_ref() {
                if let Some(system_msg) = messages
                    .first_mut()
                    .filter(|m| m.role.eq_ignore_ascii_case("system"))
                {
                    system_msg.content = rewrite_system_prompt_tools_block(
                        template,
                        &round_tool_schemas,
                        is_live_fact,
                    );
                } else {
                    messages.insert(
                        0,
                        ChatMessage {
                            role: "system".into(),
                            content: rewrite_system_prompt_tools_block(
                                template,
                                &round_tool_schemas,
                                is_live_fact,
                            ),
                            name: None,
                            images: None,
                        },
                    );
                }
            }

            let total_chars_before_compaction: usize =
                messages.iter().map(|m| m.content.chars().count()).sum();
            compact_messages_for_chat(messages);
            let total_chars_after_compaction: usize =
                messages.iter().map(|m| m.content.chars().count()).sum();
            if total_chars_after_compaction < total_chars_before_compaction {
                log_pipeline_step(
                    session_id,
                    "llm_context_compacted",
                    "Compacted message history to fit context budget",
                    Some(serde_json::json!({
                        "round": round,
                        "before_chars": total_chars_before_compaction,
                        "after_chars": total_chars_after_compaction,
                        "message_count": messages.len(),
                    })),
                );
            }

            let llm_tool_schemas: Option<&[ToolSchema]> =
                if pure_image_analysis_turn || suppress_all_tool_routing {
                    // Fast-path or satisfied: send NO tool schemas to the LLM
                    None
                } else {
                    Some(round_tool_schemas.as_slice())
                };

            // ── Document RAG context injection ─────────────────────────────────
            // If the session has uploaded documents, retrieve the most relevant
            // chunks and inject them as a system message before the LLM call.
            if let Some(ref doc_store) = self.doc_store {
                if doc_store.has_documents(session_id).await {
                    let chunks = doc_store.query(session_id, &round_user_text).await;
                    if !chunks.is_empty() {
                        let context_text =
                            crate::preprocessing::RetrievedChunk::format_context(&chunks);
                        // Determine insert position before any mutable borrow of messages
                        let has_system = messages
                            .first()
                            .map(|m| m.role.eq_ignore_ascii_case("system"))
                            .unwrap_or(false);
                        let inject_pos = if has_system { 1 } else { 0 };
                        messages.insert(
                            inject_pos,
                            ChatMessage {
                                role: "system".into(),
                                content: context_text,
                                name: None,
                                images: None,
                            },
                        );
                        log_pipeline_step(
                            session_id,
                            "doc_rag_injected",
                            "Document RAG context injected",
                            Some(serde_json::json!({
                                "chunks": chunks.len(),
                                "round": round,
                            })),
                        );
                    }
                }
            }

            let mut llm_messages = messages.clone();
            let should_strip_images_for_round = has_images && !inline_images_allowed_for_turn;
            if should_strip_images_for_round {
                for message in &mut llm_messages {
                    if message.has_images() {
                        message.images = None;
                    }
                }
            }
            let round_has_images = llm_messages.iter().any(|message| message.has_images());

            log_pipeline_step(
                session_id,
                "llm_input_prepared",
                "Prepared LLM request payload",
                Some(serde_json::json!({
                    "round": round,
                    "tool_schema_count": llm_tool_schemas.map(|schemas| schemas.len()).unwrap_or(0),
                    "routed_tool_count": routed_tool_names.len(),
                    "fallback_tool_count": fallback_tool_names.len(),
                    "direct_hint_tool": round_direct_tool_hint,
                    "images_stripped_for_round": should_strip_images_for_round,
                    "history_message_count": messages.len(),
                    "messages_preview": build_message_preview(&llm_messages, 6),
                })),
            );

            // Call LLM
            let response = match backend
                .chat(&llm_messages, llm_tool_schemas, 0.7, 4096)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    let error_text = e.to_string();
                    if round_has_images && looks_like_vision_unavailable_error(&error_text) {
                        log_pipeline_step(
                            session_id,
                            "vision_runtime_fallback",
                            "Vision runtime unavailable; keeping VisionMode-driven inline-image policy (no blind stripping)",
                            Some(serde_json::json!({
                                "round": round,
                                "vision_mode": inline_image_vision_mode.as_str(),
                                "error": sanitize_text_for_logs(&error_text, 260),
                            })),
                        );
                        let _ = event_tx.send(StreamEvent::Plan(
                            "Vision runtime is temporarily unavailable; using OCR/image tools fallback."
                                .into(),
                        ));
                        LlmResponse {
                            content: String::new(),
                            model: backend.model_label().to_string(),
                            usage: None,
                            tool_calls: None,
                        }
                    } else {
                        log_pipeline_step(
                            session_id,
                            "llm_error",
                            "LLM call failed",
                            Some(serde_json::json!({
                                "round": round,
                                "error": sanitize_text_for_logs(&error_text, 260),
                            })),
                        );
                        // Emit structured recovery options when LLM is unavailable.
                        // The UI will render these as clickable action buttons.
                        if round == 0 {
                            let user_message = format!(
                                "I couldn't complete this request — the AI model is currently unavailable.\n\n\
                                 You can try one of these alternatives, or rephrase the request."
                            );
                            // Detect what category of action the user wanted, suggest concrete recoveries
                            let lower_text = last_user_text.to_lowercase();
                            let mut options: Vec<RecoveryOption> = Vec::new();

                            // App / browser opening
                            if lower_text.contains("open ") || lower_text.contains("launch ") {
                                options.push(RecoveryOption {
                                    label: "Try again with explicit app name".into(),
                                    action_prompt: "Tell me which specific app to open (e.g., 'Open Chrome', 'Open gedit')".into(),
                                    style: "primary",
                                });
                            }
                            // File operations
                            if lower_text.contains("write") || lower_text.contains("create file") {
                                options.push(RecoveryOption {
                                    label: "Specify exact file path".into(),
                                    action_prompt: "Give me the full path: 'Write a Python script at /tmp/X.py that ...'".into(),
                                    style: "primary",
                                });
                            }
                            // Search
                            if lower_text.contains("search") || lower_text.contains("find") {
                                options.push(RecoveryOption {
                                    label: "Open browser to search".into(),
                                    action_prompt: format!(
                                        "Open the browser and search for: {}",
                                        last_user_text.chars().take(100).collect::<String>()
                                    ),
                                    style: "primary",
                                });
                            }
                            // Retry
                            options.push(RecoveryOption {
                                label: "Retry the same request".into(),
                                action_prompt: last_user_text.clone(),
                                style: "secondary",
                            });
                            // Check LLM health
                            options.push(RecoveryOption {
                                label: "Check AI backend status".into(),
                                action_prompt: "What is the status of the AI backend?".into(),
                                style: "secondary",
                            });

                            let _ = event_tx.send(StreamEvent::RecoveryOptions {
                                context: "AI model unavailable".to_string(),
                                detail: format!(
                                    "Backend error: {}",
                                    sanitize_text_for_logs(&error_text, 200)
                                ),
                                options,
                            });
                            let _ = event_tx.send(StreamEvent::Error(user_message));
                        } else {
                            let _ = event_tx.send(StreamEvent::Error(format!("LLM error: {e}")));
                        }
                        return;
                    }
                }
            };

            if return_if_stale() {
                return;
            }

            log_pipeline_step(
                session_id,
                "llm_response_received",
                "LLM response received",
                Some(serde_json::json!({
                    "round": round,
                    "model": response.model.clone(),
                    "usage": response.usage.as_ref().map(|u| serde_json::json!({
                        "prompt_tokens": u.prompt_tokens,
                        "completion_tokens": u.completion_tokens,
                        "total_tokens": u.total_tokens,
                    })),
                    "native_tool_calls": response
                        .tool_calls
                        .as_ref()
                        .map(|v| v.len())
                        .unwrap_or(0),
                    "content_preview": sanitize_text_for_logs(&response.content, 320),
                })),
            );

            // Phase 4: Record provider-reported usage in the ledger (exact counts when available)
            if let Some(ref usage) = response.usage {
                turn_ledger.record_provider_usage(usage.prompt_tokens, usage.completion_tokens);
            }

            // Parse tool calls from response — prefer native function-calling format
            // (returned by llama.cpp / OpenAI), fall back to text-embedded format.
            // Pattern 7 (Python-style fallback) fires last, only for single-required-param tools.
            let parse_mode = if response.tool_calls.is_some() {
                "native_function_call"
            } else {
                "text_pattern_fallback"
            };

            let mut tool_calls: Vec<ParsedToolCall> = if let Some(native) = &response.tool_calls {
                native
                    .iter()
                    .filter_map(|tc| {
                        let name = tc["function"]["name"].as_str()?.to_string();
                        let arguments: serde_json::Value = tc["function"]["arguments"]
                            .as_str()
                            .and_then(|s| serde_json::from_str(s).ok())
                            .unwrap_or_else(|| tc["function"]["arguments"].clone());
                        Some(ParsedToolCall { name, arguments })
                    })
                    .collect()
            } else {
                // Build the single-required-param lookup for Pattern 7
                let single_param_tools: Vec<(String, String)> = self
                    .tool_registry
                    .list_defs()
                    .into_iter()
                    .filter_map(|d| {
                        let required: Vec<_> = d.parameters.iter().filter(|p| p.required).collect();
                        if required.len() == 1 {
                            Some((d.name.clone(), required[0].name.clone()))
                        } else {
                            None
                        }
                    })
                    .collect();
                let known: Vec<(&str, &str)> = single_param_tools
                    .iter()
                    .map(|(n, p)| (n.as_str(), p.as_str()))
                    .collect();
                parse_tool_calls_with_known(&response.content, &known)
            };
            let text_response_raw = extract_text_response(&response.content);
            let text_response = sanitize_assistant_text_response(&text_response_raw);

            // ── GUI-launch tool call interception ─────────────────────────────
            // When a #tool:browser_search directive is active and the LLM returned
            // web_search / searxng_search / search_news instead, replace them with
            // the correct browser_search call. This handles the case where the LLM's
            // training prior overrides the system prompt instruction.
            if forced_tool_name.as_deref() == Some("browser_search")
                && !tool_calls.is_empty()
                && tool_calls.iter().all(|tc| {
                    matches!(
                        tc.name.as_str(),
                        "web_search" | "searxng_search" | "search_news"
                    )
                })
                && allowed_tool_names.contains("browser_search")
            {
                let clean_query = forced_tool_directive
                    .as_ref()
                    .map(|(_, q)| q.as_str())
                    .unwrap_or(&last_user_text);
                let (search_query, site) = extract_browser_search_intent(clean_query);
                let mut args = serde_json::json!({ "query": search_query });
                if let Some(s) = site {
                    args["site"] = serde_json::Value::String(s);
                }
                tracing::info!(
                    session = session_id,
                    original_calls = ?tool_calls.iter().map(|tc| tc.name.as_str()).collect::<Vec<_>>(),
                    "GUI-launch interception: replacing LLM tool calls with browser_search"
                );
                tool_calls = vec![ParsedToolCall {
                    name: "browser_search".into(),
                    arguments: args,
                }];
            }

            log_pipeline_step(
                session_id,
                "tool_calls_parsed",
                "Parsed tool calls from LLM response",
                Some(serde_json::json!({
                    "round": round,
                    "parse_mode": parse_mode,
                    "tool_call_count": tool_calls.len(),
                    "tool_calls": build_tool_calls_preview(&tool_calls),
                    "text_response_preview": sanitize_text_for_logs(&text_response, 320),
                })),
            );

            let mut synthetic_package_calls = false;
            let mut synthetic_colab_calls = false;
            let mut synthetic_intent_calls = false;
            let mut synthetic_capability_calls = false;
            if tool_calls.is_empty() {
                if let Some(flow) = package_flow.as_ref() {
                    let fallback_calls = flow.next_required_calls();
                    if !fallback_calls.is_empty() {
                        synthetic_package_calls = true;
                        tool_calls = fallback_calls;
                        log_pipeline_step(
                            session_id,
                            "synthetic_package_calls",
                            "Injected package workflow tool calls",
                            Some(serde_json::json!({
                                "round": round,
                                "tool_calls": build_tool_calls_preview(&tool_calls),
                            })),
                        );
                        let _ = event_tx.send(StreamEvent::Plan(
                            "Enforcing package workflow with pre/post verification".into(),
                        ));
                    }
                }
            }

            // MARKETPLACE capability flow: when the request is clearly about
            // installing/searching a KRIA skill/tool/capability and the model
            // produced no tool call, force the correct provider-neutral
            // marketplace tool (search_marketplace / install_capability) instead
            // of letting the turn stall or drift to OS package tools / web search.
            if tool_calls.is_empty() {
                if let Some(flow) = capability_flow.as_ref() {
                    let cap_calls = flow.next_required_calls(&allowed_tool_names);
                    if !cap_calls.is_empty() {
                        synthetic_capability_calls = true;
                        let status = flow.status_summary();
                        tool_calls = cap_calls;
                        log_pipeline_step(
                            session_id,
                            "synthetic_capability_calls",
                            "Injected marketplace capability tool call",
                            Some(serde_json::json!({
                                "round": round,
                                "tool_calls": build_tool_calls_preview(&tool_calls),
                            })),
                        );
                        let _ = event_tx.send(StreamEvent::Plan(status));
                    }
                }
            }

            // Colab workflow: inject next required Colab step if LLM produced no calls.
            if tool_calls.is_empty() {
                if let Some(flow) = colab_flow.as_ref() {
                    let colab_calls = flow.next_required_calls(&allowed_tool_names);
                    if !colab_calls.is_empty() {
                        synthetic_colab_calls = true;
                        let status = flow.status_summary();
                        tool_calls = colab_calls;
                        log_pipeline_step(
                            session_id,
                            "synthetic_colab_calls",
                            "Injected Colab workflow tool calls",
                            Some(serde_json::json!({
                                "round": round,
                                "tool_calls": build_tool_calls_preview(&tool_calls),
                            })),
                        );
                        let _ = event_tx.send(StreamEvent::Plan(status));
                    }
                }
            }

            if tool_calls.is_empty() && !intent_fallback_used {
                // IntentGate: suppress intent fallback for conversational fast-path inputs
                // and for inputs where the gate requires clarification instead of execution.
                let gate_suppresses_fallback = gate_decision.fast_path
                    || gate_requires_clarification
                    || (!gate_decision.execution_permitted && !forced_tool_requested);

                if gate_suppresses_fallback {
                    tracing::debug!(
                        session = session_id,
                        intent = gate_decision.intent.as_str(),
                        fast_path = gate_decision.fast_path,
                        execution_permitted = gate_decision.execution_permitted,
                        "IntentGate: suppressing intent fallback injection"
                    );
                } else {
                    let intent_fallback_query =
                        resolve_intent_fallback_query(&routing_focus_text, messages);
                    let fallback_plan =
                        self.turn_gate.plan_turn(&intent_fallback_query, has_images);
                    let fallback_confidence = fallback_plan
                        .intent
                        .confidence
                        .max(turn_gate_plan.intent.confidence);
                    let fallback_calls: Vec<ParsedToolCall> = self
                        .turn_gate
                        .fallback_tool_hints(&fallback_plan, &allowed_tool_names)
                        .into_iter()
                        .filter_map(|hint| {
                            build_fallback_call_for_hint(
                                &hint,
                                &intent_fallback_query,
                                &allowed_tool_names,
                            )
                        })
                        .collect();

                    if !fallback_calls.is_empty() {
                        if forced_tool_requested
                            || fallback_confidence >= self.min_confidence_to_act
                        {
                            intent_fallback_used = true;
                            synthetic_intent_calls = true;
                            turn_gate_plan = fallback_plan;
                            let names: Vec<&str> =
                                fallback_calls.iter().map(|c| c.name.as_str()).collect();
                            let plan_message = if intent_fallback_query == routing_focus_text {
                                format!(
                                    "No tool call returned; applying turn_gate fallback via {}",
                                    names.join(", "),
                                )
                            } else {
                                format!(
                                "No tool call returned; applying context-aware turn_gate fallback via {}",
                                names.join(", "),
                            )
                            };
                            let _ = event_tx.send(StreamEvent::Plan(plan_message));
                            tool_calls = fallback_calls;
                            log_pipeline_step(
                                session_id,
                                "synthetic_intent_call",
                                "Injected intent fallback tool call",
                                Some(serde_json::json!({
                                    "round": round,
                                    "fallback_query": sanitize_text_for_logs(&intent_fallback_query, 220),
                                    "source": "turn_gate",
                                    "confidence": fallback_confidence,
                                    "tool_calls": build_tool_calls_preview(&tool_calls),
                                })),
                            );
                        } else if fallback_confidence >= self.clarify_threshold {
                            let fallback_primary_hint = self
                                .turn_gate
                                .direct_tool_hint(&fallback_plan, &allowed_tool_names);
                            let candidates = build_tool_choice_candidates(
                                &intent_fallback_query,
                                &allowed_tool_names,
                                fallback_primary_hint.as_deref(),
                                fallback_confidence,
                            );

                            if !candidates.is_empty() {
                                log_pipeline_step(
                                    session_id,
                                    "tool_choice_required",
                                    "Low-confidence route needs user tool choice",
                                    Some(serde_json::json!({
                                        "round": round,
                                        "fallback_query": sanitize_text_for_logs(&intent_fallback_query, 220),
                                        "confidence": fallback_confidence,
                                        "candidate_count": candidates.len(),
                                    })),
                                );
                                let _ = event_tx.send(StreamEvent::ToolChoiceRequired {
                                    query: intent_fallback_query.clone(),
                                    confidence: fallback_confidence,
                                    min_confidence: self.min_confidence_to_act,
                                    candidates,
                                });
                                let _ = event_tx.send(StreamEvent::Done(
                                    "Please choose a tool so I can continue this request.".into(),
                                ));
                                return;
                            }
                        }
                    }
                } // end else (gate_suppresses_fallback)
            }

            // If no tool calls, we're done
            if tool_calls.is_empty() {
                if return_if_stale() {
                    return;
                }

                log_pipeline_step(
                    session_id,
                    "no_tool_calls",
                    "No tool calls returned for this round",
                    Some(serde_json::json!({
                        "round": round,
                        "synthetic_package_calls": synthetic_package_calls,
                        "synthetic_colab_calls": synthetic_colab_calls,
                        "synthetic_intent_calls": synthetic_intent_calls,
                    })),
                );

                if let Some(flow) = package_flow.as_ref() {
                    if let Some(summary) = flow.verified_summary() {
                        log_pipeline_step(
                            session_id,
                            "final_output_ready",
                            "Using package-flow verification summary",
                            Some(serde_json::json!({
                                "round": round,
                                "final_preview": sanitize_text_for_logs(&summary, 260),
                            })),
                        );
                        let _ = event_tx.send(StreamEvent::Token(summary.clone()));
                        let _ = event_tx.send(StreamEvent::Done(summary));
                        return;
                    }
                }
                let mut final_text = if had_successful_gmail_tool && !had_failed_gmail_tool {
                    strip_spurious_gmail_error_lines(&text_response)
                } else {
                    text_response.clone()
                };

                log_pipeline_step(
                    session_id,
                    "final_formatting_started",
                    "Preparing final assistant output",
                    Some(serde_json::json!({
                        "round": round,
                        "had_successful_gmail_tool": had_successful_gmail_tool,
                        "had_failed_gmail_tool": had_failed_gmail_tool,
                        "text_preview": sanitize_text_for_logs(&final_text, 280),
                    })),
                );

                if had_successful_gmail_tool && !had_failed_gmail_tool && !final_text.is_empty() {
                    let has_placeholder_scaffold = contains_gmail_placeholder_scaffold(&final_text);
                    let has_raw_payload = looks_like_raw_gmail_payload_json(final_text.trim());
                    let has_duplicate_rows = contains_duplicate_gmail_rows(&final_text);
                    let should_force_grounded =
                        has_placeholder_scaffold || has_raw_payload || has_duplicate_rows;

                    if should_force_grounded {
                        if let Some(grounded_summary) = last_successful_gmail_result
                            .as_ref()
                            .and_then(build_grounded_gmail_message_list_summary)
                        {
                            tracing::warn!(
                                has_images,
                                round,
                                has_placeholder_scaffold,
                                has_raw_payload,
                                has_duplicate_rows,
                                "LLM returned non-grounded Gmail response; replacing with grounded summary"
                            );
                            log_pipeline_step(
                                session_id,
                                "final_formatting_adjusted",
                                "Replaced non-grounded Gmail output with grounded summary",
                                Some(serde_json::json!({
                                    "round": round,
                                    "has_placeholder_scaffold": has_placeholder_scaffold,
                                    "has_raw_payload": has_raw_payload,
                                    "has_duplicate_rows": has_duplicate_rows,
                                })),
                            );
                            final_text = grounded_summary;
                        }
                    }
                }

                if !final_text.is_empty() {
                    log_pipeline_step(
                        session_id,
                        "final_output_ready",
                        "Final assistant response ready",
                        Some(serde_json::json!({
                            "round": round,
                            "final_preview": sanitize_text_for_logs(&final_text, 320),
                            "final_chars": final_text.chars().count(),
                        })),
                    );
                    let _ = event_tx.send(StreamEvent::Token(final_text.clone()));
                    let _ = event_tx.send(StreamEvent::Done(final_text));
                } else if had_successful_gmail_tool && !had_failed_gmail_tool {
                    if let Some(summary) = last_successful_gmail_result
                        .as_ref()
                        .and_then(build_grounded_gmail_count_summary)
                    {
                        tracing::info!(
                            has_images,
                            round,
                            "LLM returned empty response with no tool calls; using grounded Gmail count summary"
                        );
                        log_pipeline_step(
                            session_id,
                            "final_output_ready",
                            "Using grounded Gmail count summary fallback",
                            Some(serde_json::json!({
                                "round": round,
                                "final_preview": sanitize_text_for_logs(&summary, 260),
                            })),
                        );
                        let _ = event_tx.send(StreamEvent::Token(summary.clone()));
                        let _ = event_tx.send(StreamEvent::Done(summary));
                    } else {
                        let fallback =
                            "I could not generate a response for this request. Please try again."
                                .to_string();
                        tracing::warn!(
                            has_images,
                            round,
                            "LLM returned empty response with no tool calls and no grounded Gmail summary"
                        );
                        log_pipeline_step(
                            session_id,
                            "final_output_fallback",
                            "Generated generic fallback due empty grounded response",
                            Some(serde_json::json!({
                                "round": round,
                                "final_preview": sanitize_text_for_logs(&fallback, 200),
                            })),
                        );
                        let _ = event_tx.send(StreamEvent::Token(fallback.clone()));
                        let _ = event_tx.send(StreamEvent::Done(fallback));
                    }
                } else {
                    let fallback =
                        "I could not generate a response for this request. Please try again."
                            .to_string();
                    tracing::warn!(
                        has_images,
                        round,
                        "LLM returned empty response with no tool calls"
                    );
                    log_pipeline_step(
                        session_id,
                        "final_output_fallback",
                        "Generated generic fallback due empty response",
                        Some(serde_json::json!({
                            "round": round,
                            "final_preview": sanitize_text_for_logs(&fallback, 200),
                        })),
                    );
                    let _ = event_tx.send(StreamEvent::Token(fallback.clone()));
                    let _ = event_tx.send(StreamEvent::Done(fallback));
                }
                return;
            }

            // Add assistant message to history
            if !synthetic_package_calls && !synthetic_intent_calls && !synthetic_capability_calls {
                messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: build_tool_call_history_content(&tool_calls),
                    name: None,
                    images: None,
                });

                log_pipeline_step(
                    session_id,
                    "assistant_tool_history_added",
                    "Added assistant tool-call turn to history",
                    Some(serde_json::json!({
                        "round": round,
                        "tool_calls": build_tool_calls_preview(&tool_calls),
                    })),
                );
            }

            // Execute each tool call
            for call in &tool_calls {
                if return_if_stale() {
                    return;
                }

                // ── Task satisfaction check: stop loop if goal already met ────
                if turn_memory.is_satisfied() {
                    tracing::warn!(
                        session = session_id,
                        tool = %call.name,
                        reason = %turn_memory.satisfaction_reason().unwrap_or(""),
                        "🛑 SKIPPING TOOL - goal already satisfied, breaking tool loop"
                    );
                    log_pipeline_step(
                        session_id,
                        "tool_skipped_satisfied",
                        "Skipping tool call because goal is already satisfied",
                        Some(serde_json::json!({
                            "round": round,
                            "skipped_tool": call.name.clone(),
                            "reason": turn_memory.satisfaction_reason(),
                        })),
                    );
                    break;
                }

                // ── Search dedup guard: prevent redundant parallel search calls ─
                // If a search tool already succeeded this turn, skip additional
                // search tools for the same turn (web_search + search_news both
                // firing for the same query is wasteful and confusing).
                let is_search_tool = matches!(
                    call.name.as_str(),
                    "web_search" | "searxng_search" | "search_news"
                );
                if is_search_tool {
                    if let Some(prior_search) = turn_memory.successful_search_this_turn() {
                        if prior_search != call.name.as_str() {
                            tracing::info!(
                                session = session_id,
                                tool = %call.name,
                                prior_search = %prior_search,
                                "search_dedup_guard: skipping redundant search — {} already succeeded this turn",
                                prior_search
                            );
                            log_pipeline_step(
                                session_id,
                                "search_dedup_skipped",
                                "Search dedup guard: skipping redundant search tool",
                                Some(serde_json::json!({
                                    "round": round,
                                    "tool": call.name.clone(),
                                    "prior_search": prior_search,
                                    "reason": "a search tool already succeeded this turn",
                                })),
                            );
                            messages.push(ChatMessage {
                                role: "tool".into(),
                                content: format!(
                                    "SEARCH_DEDUP: '{}' skipped — '{}' already returned results this turn. Use those results.",
                                    call.name, prior_search
                                ),
                                name: Some(call.name.clone()),
                                images: None,
                            });
                            continue;
                        }
                    }
                }

                // ── GUI-last policy enforcement ────────────────────────────────
                // If the LLM picked a GUI/Browser tool but a higher-priority
                // alternative (API/MCP/CLI) is available in the catalog, redirect
                // the LLM to use the better tool. This is the authoritative
                // execution-mode preference: API > MCP > CLI > Browser > GUI.
                let call_profile = crate::mcp::capability_registry::capability_profile(&call.name);
                if call_profile.is_last_resort() {
                    if let Some(better_alt) =
                        crate::mcp::capability_registry::find_better_alternative(
                            &call.name,
                            &allowed_tool_names,
                        )
                    {
                        tracing::info!(
                            session = session_id,
                            tool = %call.name,
                            better_alternative = %better_alt,
                            "gui_last_policy: blocking last-resort tool — better alternative available"
                        );
                        log_pipeline_step(
                            session_id,
                            "gui_last_policy_blocked",
                            "GUI-last policy: blocking last-resort tool with better alternative",
                            Some(serde_json::json!({
                                "round": round,
                                "tool": call.name.clone(),
                                "execution_mode": format!("{:?}", call_profile.execution_mode),
                                "alternative": better_alt,
                                "policy": "API > MCP > CLI > Browser > GUI",
                            })),
                        );
                        messages.push(ChatMessage {
                            role: "tool".into(),
                            content: format!(
                                "EXECUTION_REDIRECT: '{}' is a last-resort tool. \
                                Use '{}' instead — it accomplishes the same goal via a more \
                                reliable execution path (API/MCP/CLI). Retry with the better tool.",
                                call.name, better_alt
                            ),
                            name: Some(call.name.clone()),
                            images: None,
                        });
                        continue;
                    }
                }

                let mut execution_args = call.arguments.clone();
                if call.name == "analyze_image" {
                    let cap_decision = self.compute_visual_token_cap().await;
                    let mut payload_obj = match &execution_args {
                        serde_json::Value::Object(obj) => obj.clone(),
                        serde_json::Value::String(raw) => {
                            serde_json::from_str::<serde_json::Value>(raw)
                                .ok()
                                .and_then(|v| v.as_object().cloned())
                                .unwrap_or_default()
                        }
                        _ => serde_json::Map::new(),
                    };
                    payload_obj.insert(
                        "hard_visual_token_cap".to_string(),
                        serde_json::json!(cap_decision.hard_cap),
                    );
                    execution_args = serde_json::Value::Object(payload_obj);

                    tracing::info!(
                        hard_visual_token_cap = cap_decision.hard_cap,
                        safe_visual_token_cap = cap_decision.safe_cap,
                        free_vram_mb = cap_decision.free_vram_mb,
                        safety_margin_mb = cap_decision.safety_margin_mb,
                        vision_mode = %cap_decision.vision_mode,
                        "agent_loop: injected hard_visual_token_cap for analyze_image pre-flight"
                    );
                }

                let tool_execution_id = Uuid::now_v7().to_string();
                let tool_execution_started = std::time::Instant::now();
                tracing::info!(
                    target: "tool_execution",
                    session = session_id,
                    execution_id = %tool_execution_id,
                    tool_name = %call.name,
                    input_summary = %sanitize_json_for_logs(&execution_args, 220, 8),
                    "Tool execution started"
                );

                log_pipeline_step(
                    session_id,
                    "tool_call_started",
                    "Beginning tool execution",
                    Some(serde_json::json!({
                        "round": round,
                        "tool": call.name.clone(),
                        "arguments": sanitize_json_for_logs(&execution_args, 220, 8),
                    })),
                );

                // Never execute tools outside the current mounted+tier visible set.
                if !allowed_tool_names.contains(&call.name) {
                    let unavailable_msg = format!(
                        "tool '{}' is not available for current hardware tier '{}' or mounted tool groups",
                        call.name, self.hardware_tier
                    );

                    log_pipeline_step(
                        session_id,
                        "tool_call_rejected",
                        "Tool blocked by tier/mount gating",
                        Some(serde_json::json!({
                            "round": round,
                            "tool": call.name.clone(),
                            "reason": sanitize_text_for_logs(&unavailable_msg, 220),
                        })),
                    );

                    let _ = event_tx.send(StreamEvent::ToolEnd {
                        name: call.name.clone(),
                        result: serde_json::json!({ "error": unavailable_msg }),
                        success: false,
                        human_readable: None,
                        conversational_summary: None,
                        execution_metadata: None,
                    });
                    if let Some(flow) = package_flow.as_mut() {
                        flow.observe_tool_result(call, false, &serde_json::Value::Null);
                    }
                    if let Some(flow) = capability_flow.as_mut() {
                        flow.observe_tool_result(call);
                    }
                    messages.push(ChatMessage {
                        role: "tool".into(),
                        content: format!(
                            "TOOL_ERROR: '{}' is not available in this context (tier/mount gating).",
                            call.name
                        ),
                        name: Some(call.name.clone()),
                        images: None,
                    });
                    continue;
                }

                let _ = event_tx.send(StreamEvent::ToolStart {
                    name: call.name.clone(),
                    params: execution_args.clone(),
                });

                // ── Fleet pre-flight: check VM connectivity before executing ──
                // When the LLM wants to run a command on a remote target, first
                // verify the target is reachable. If not, emit RecoveryOptions
                // so the user gets clickable buttons instead of a raw error.
                if call.name == "execute_fleet_command" {
                    if let Some(health_handler) =
                        self.tool_registry.get_handler("check_device_health")
                    {
                        let target_hint = execution_args
                            .get("target")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        let health_args = serde_json::json!({
                            "target": target_hint.clone().unwrap_or_default()
                        });
                        let health_handler = health_handler.clone();
                        let health_cancel = turn_mcp_cancel.clone();
                        let health_ctx =
                            self.tool_registry.make_tool_context(health_cancel.clone());

                        tracing::info!(
                            session = session_id,
                            target = ?target_hint,
                            "fleet_preflight: checking VM connectivity before execute_fleet_command"
                        );

                        // Step 1: connectivity check
                        let _ = event_tx.send(StreamEvent::TaskStep(TaskStep {
                            index: 1,
                            total: Some(2),
                            description: format!(
                                "Checking connectivity to {}",
                                target_hint.as_deref().unwrap_or("VM")
                            ),
                            status: TaskStepStatus::Running,
                        }));

                        let health_result = run_isolated(
                            "tool:check_device_health",
                            std::time::Duration::from_secs(20),
                            health_cancel,
                            None,
                            move || async move {
                                health_handler
                                    .execute_with_context(health_args, health_ctx)
                                    .await
                            },
                        )
                        .await;

                        if !health_result.success {
                            let error_raw = health_result.error.as_deref().unwrap_or("unreachable");
                            let target_label = target_hint.as_deref().unwrap_or("VM");

                            let _ = event_tx.send(StreamEvent::TaskStep(TaskStep {
                                index: 1,
                                total: Some(2),
                                description: format!(
                                    "Connectivity check failed for {target_label}"
                                ),
                                status: TaskStepStatus::Failed,
                            }));

                            // Classify the connectivity failure
                            let (context, detail, options) = classify_fleet_connectivity_failure(
                                target_label,
                                error_raw,
                                &call.name,
                                &execution_args,
                            );

                            log_pipeline_step(
                                session_id,
                                "fleet_preflight_failed",
                                "VM connectivity check failed; emitting RecoveryOptions",
                                Some(serde_json::json!({
                                    "target": target_label,
                                    "error": error_raw,
                                    "options_count": options.len(),
                                })),
                            );

                            let _ = event_tx.send(StreamEvent::RecoveryOptions {
                                context: context.clone(),
                                detail: detail.clone(),
                                options,
                            });

                            // Inject a structured message into LLM context so it
                            // knows to explain the situation, not retry blindly.
                            messages.push(ChatMessage {
                                role: "tool".into(),
                                content: format!(
                                    "FLEET_PREFLIGHT_FAILED: {context}\nDetail: {detail}\n\
                                    The user has been shown recovery options. \
                                    Explain what happened and what they can do next. \
                                    Do NOT retry the fleet command automatically."
                                ),
                                name: Some("check_device_health".into()),
                                images: None,
                            });

                            // Skip the actual fleet command execution
                            continue;
                        }

                        let _ = event_tx.send(StreamEvent::TaskStep(TaskStep {
                            index: 1,
                            total: Some(2),
                            description: format!(
                                "{} is reachable",
                                target_hint.as_deref().unwrap_or("VM")
                            ),
                            status: TaskStepStatus::Done,
                        }));

                        // Step 2: executing command
                        let command_preview = execution_args
                            .get("command")
                            .and_then(|v| v.as_str())
                            .map(|c| {
                                if c.len() > 60 {
                                    format!("{}…", &c[..60])
                                } else {
                                    c.to_string()
                                }
                            })
                            .unwrap_or_else(|| "command".to_string());

                        let _ = event_tx.send(StreamEvent::TaskStep(TaskStep {
                            index: 2,
                            total: Some(2),
                            description: format!("Running: {command_preview}"),
                            status: TaskStepStatus::Running,
                        }));

                        tracing::info!(
                            session = session_id,
                            target = ?target_hint,
                            "fleet_preflight: VM reachable, proceeding with execute_fleet_command"
                        );
                    }
                }

                // ── Colab browser-connection gate ────────────────────────────
                // If the LLM emits an execute_cell call but the browser connection
                // has not been established yet, transparently prepend the bootstrap
                // call so code never fires into a disconnected session.
                if call.name.contains("execute_cell") && call.name.contains("colab") {
                    let already_connected = colab_flow
                        .as_ref()
                        .map(|f| f.browser_connected)
                        .unwrap_or(false);
                    if !already_connected
                        && allowed_tool_names
                            .contains("mcp_colab-mcp_open_colab_browser_connection")
                    {
                        let _ = event_tx.send(StreamEvent::Plan(
                            "Colab browser not connected — establishing connection first.".into(),
                        ));
                        let bootstrap = ColabFlowState::browser_open_call();
                        // Inject bootstrap ahead of execute — push current call back.
                        // We handle this by bumping execute_cell to the next round
                        // after the browser is confirmed via observe_tool_result.
                        // Replace current call slice with [bootstrap_call, original_call].
                        // The simplest way: execute bootstrap now via recursive inject.
                        // We'll just replace the current `call` reference by mutating
                        // the iteration — instead, mark as gate-injected and continue.
                        let _ = event_tx.send(StreamEvent::ToolStart {
                            name: bootstrap.name.clone(),
                            params: bootstrap.arguments.clone(),
                        });
                        // AUTHORITY FIX: Colab bootstrap must pass through policy + audit,
                        // same as every other tool. Green-tier (no HITL needed) but audit
                        // visibility is mandatory — no silent execution bypasses allowed.
                        let bootstrap_policy = self.policy_engine.evaluate_with_modality_hint(
                            &bootstrap.name,
                            &bootstrap.arguments,
                            false, // non-destructive
                        );
                        self.audit_logger.log(
                            session_id,
                            &bootstrap.name,
                            &bootstrap.arguments,
                            bootstrap_policy.risk_level,
                            crate::safety::audit::Decision::AutoExecuted,
                            crate::safety::audit::DecidedBy::Policy,
                        );
                        let gate_result = if bootstrap_policy.blocked {
                            tracing::warn!(
                                target: "authority_trace",
                                tool = %bootstrap.name,
                                reason = %bootstrap_policy.reason,
                                "Colab bootstrap blocked by policy"
                            );
                            crate::infra::isolation::ToolResult::err(format!(
                                "POLICY_BLOCKED: {}",
                                bootstrap_policy.reason
                            ))
                        } else if let Some(gate_handler) =
                            self.tool_registry.get_handler(&bootstrap.name)
                        {
                            let gate_handler = gate_handler.clone();
                            let gate_args = bootstrap.arguments.clone();
                            let gate_cancel = turn_mcp_cancel.clone();
                            let gate_context =
                                self.tool_registry.make_tool_context(gate_cancel.clone());
                            run_isolated(
                                "tool:mcp_colab-mcp_open_colab_browser_connection",
                                std::time::Duration::from_secs(60),
                                gate_cancel,
                                None,
                                move || async move {
                                    gate_handler
                                        .execute_with_context(gate_args, gate_context)
                                        .await
                                },
                            )
                            .await
                        } else {
                            crate::infra::isolation::ToolResult::err(
                                "open_colab_browser_connection handler not found".to_string(),
                            )
                        };
                        if let Some(flow) = colab_flow.as_mut() {
                            flow.observe_tool_result(
                                &bootstrap,
                                gate_result.success,
                                &gate_result.data,
                            );
                        }
                        let _ = event_tx.send(StreamEvent::ToolEnd {
                            name: bootstrap.name.clone(),
                            result: gate_result.data.clone(),
                            success: gate_result.success,
                            human_readable: None,
                            conversational_summary: None,
                            execution_metadata: None,
                        });
                        messages.push(ChatMessage {
                            role: "tool".into(),
                            content: serde_json::to_string(&gate_result.data).unwrap_or_default(),
                            name: Some(bootstrap.name.clone()),
                            images: None,
                        });
                        if !gate_result.success {
                            messages.push(ChatMessage {
                                role: "system".into(),
                                content: "Colab browser connection failed. Cannot execute cell."
                                    .into(),
                                name: None,
                                images: None,
                            });
                            continue;
                        }
                    }
                }

                // Policy check — pass destructive hint from semantic router modality
                // INVARIANT: policy evaluation MUST run before any tool execution.
                // This is the single authority gate — any bypass makes it past here
                // only through an explicit exception (Colab bootstrap handles its own
                // policy check above). Never remove this call.
                let decision = self.policy_engine.evaluate_with_modality_hint(
                    &call.name,
                    &execution_args,
                    turn_modality.destructive,
                );

                // RUNTIME AUTHORITY ASSERTION: policy result must be structurally sound.
                // In debug builds, panic immediately if the policy result is nonsensical
                // (both blocked AND requires_approval is logically invalid).
                debug_assert!(
                    !(decision.blocked && decision.requires_approval),
                    "policy invariant violated: tool='{}' cannot be both blocked and require approval",
                    call.name
                );

                log_pipeline_step(
                    session_id,
                    "policy_evaluated",
                    "Policy evaluation completed for tool call",
                    Some(serde_json::json!({
                        "round": round,
                        "tool": call.name.clone(),
                        "risk_level": decision.risk_level.as_str(),
                        "requires_approval": decision.requires_approval,
                        "blocked": decision.blocked,
                        "reason": sanitize_text_for_logs(&decision.reason, 220),
                    })),
                );

                if decision.blocked {
                    // BLACK tier — always denied
                    self.audit_logger.log(
                        session_id,
                        &call.name,
                        &execution_args,
                        RiskLevel::Black,
                        Decision::Blocked,
                        DecidedBy::Hardcoded,
                    );
                    let _ = event_tx.send(StreamEvent::ToolEnd {
                        name: call.name.clone(),
                        result: serde_json::json!({ "error": "blocked by safety policy" }),
                        success: false,
                        human_readable: None,
                        conversational_summary: None,
                        execution_metadata: None,
                    });
                    messages.push(ChatMessage {
                        role: "tool".into(),
                        content: format!(
                            "Tool '{}' blocked by safety policy: {}",
                            call.name, decision.reason
                        ),
                        name: Some(call.name.clone()),
                        images: None,
                    });

                    log_pipeline_step(
                        session_id,
                        "tool_call_blocked",
                        "Tool call blocked by safety policy",
                        Some(serde_json::json!({
                            "round": round,
                            "tool": call.name.clone(),
                            "reason": sanitize_text_for_logs(&decision.reason, 220),
                        })),
                    );

                    continue;
                }

                if decision.requires_approval {
                    // RED tier — needs HITL approval (but skip if same tool+args already approved this turn)
                    let dedup_key = format!("{}|{}", call.name, execution_args);
                    let already_approved = approved_this_turn.contains(&dedup_key);

                    if already_approved {
                        // Already approved earlier in this turn — auto-proceed, log it
                        self.audit_logger.log(
                            session_id,
                            &call.name,
                            &execution_args,
                            decision.risk_level,
                            Decision::Approved,
                            DecidedBy::Policy,
                        );

                        log_pipeline_step(
                            session_id,
                            "approval_reused",
                            "Reused earlier approval for identical tool call",
                            Some(serde_json::json!({
                                "round": round,
                                "tool": call.name.clone(),
                            })),
                        );
                    } else {
                        // Generate the request ID up front so the frontend receives the
                        // same ID that the HITL gateway stores in its pending map.
                        let request_id = HitlGateway::generate_request_id();

                        let _ = event_tx.send(StreamEvent::ApprovalRequired {
                            request_id: request_id.clone(),
                            action: call.name.clone(),
                            risk_level: decision.risk_level.as_str().into(),
                            parameters: execution_args.clone(),
                        });

                        log_pipeline_step(
                            session_id,
                            "approval_requested",
                            "Approval requested for RED-tier tool call",
                            Some(serde_json::json!({
                                "round": round,
                                "tool": call.name.clone(),
                                "request_id": request_id.clone(),
                                "risk_level": decision.risk_level.as_str(),
                            })),
                        );

                        let approval = self
                            .hitl_gateway
                            .request_approval_with_id(
                                &request_id,
                                &call.name,
                                execution_args.clone(),
                                decision.risk_level,
                                &format!("Execute {} with params: {}", call.name, execution_args),
                                true,
                            )
                            .await;

                        let (audit_decision, decided_by, approved, denial_reason) = match approval {
                            ApprovalResponse::Approved => {
                                (Decision::Approved, DecidedBy::UserGui, true, "")
                            }
                            ApprovalResponse::Denied => (
                                Decision::Denied,
                                DecidedBy::UserGui,
                                false,
                                "denied by user",
                            ),
                            ApprovalResponse::Timeout => (
                                Decision::Timeout,
                                DecidedBy::Timeout,
                                false,
                                "approval timed out — user did not respond",
                            ),
                        };

                        self.audit_logger.log(
                            session_id,
                            &call.name,
                            &execution_args,
                            decision.risk_level,
                            audit_decision,
                            decided_by,
                        );

                        let _ = event_tx.send(StreamEvent::ApprovalResult {
                            action: call.name.clone(),
                            approved,
                        });

                        log_pipeline_step(
                            session_id,
                            "approval_result",
                            "Approval decision received",
                            Some(serde_json::json!({
                                "round": round,
                                "tool": call.name.clone(),
                                "approved": approved,
                            })),
                        );

                        if !approved {
                            // Emit ToolEnd so the UI shows the tool as failed (not just pending).
                            let _ = event_tx.send(StreamEvent::ToolEnd {
                                name: call.name.clone(),
                                result: serde_json::json!({ "error": denial_reason }),
                                success: false,
                                human_readable: None,
                                conversational_summary: None,
                                execution_metadata: None,
                            });
                            messages.push(ChatMessage {
                                role: "tool".into(),
                                content: format!(
                                    "TOOL_ERROR: '{}' was NOT executed — {}. \
                                     The operation did not happen. \
                                     You MUST tell the user the action failed and why.",
                                    call.name, denial_reason
                                ),
                                name: Some(call.name.clone()),
                                images: None,
                            });

                            log_pipeline_step(
                                session_id,
                                "tool_call_denied",
                                "Tool call not executed due denied/timeout approval",
                                Some(serde_json::json!({
                                    "round": round,
                                    "tool": call.name.clone(),
                                    "reason": denial_reason,
                                })),
                            );

                            continue;
                        }

                        // Remember this approval for the rest of this turn
                        approved_this_turn.insert(dedup_key);

                        // Create rollback snapshot for RED actions
                        // (actual file backup happens inside specific tool handlers)
                    }
                }

                // ── Dedup guard: abort on repeated identical failure ───────────
                let call_hash = call_dedup_hash(&call.name, &execution_args);

                // ── Memoization: skip identical successful calls ──────────────
                // If this exact call (same tool + same args) already succeeded this
                // turn, return the cached result instead of re-executing.
                if let Some(cached_result) = turn_memory.check_memo(call_hash) {
                    tracing::debug!(
                        session = session_id,
                        tool = %call.name,
                        "turn_memory: returning memoized result for identical call"
                    );
                    log_pipeline_step(
                        session_id,
                        "tool_memoized",
                        "Returning memoized result for identical successful call",
                        Some(serde_json::json!({
                            "round": round,
                            "tool": call.name.clone(),
                            "cached_result_preview": &cached_result[..cached_result.len().min(100)],
                        })),
                    );
                    // Inject the cached result as a tool message and continue
                    messages.push(ChatMessage {
                        role: "tool".into(),
                        content: format!("[MEMOIZED] {}", cached_result),
                        name: Some(call.name.clone()),
                        images: None,
                    });
                    continue;
                }

                if let Some((fail_count, cached_err)) = failed_calls.get(&call_hash) {
                    if *fail_count >= 1 {
                        let abort_msg = format!(
                            "repeated_identical_failure: '{}' with the same arguments already \
                             failed in this turn: {}. Aborting to prevent an infinite loop.",
                            call.name, cached_err
                        );
                        tracing::warn!(
                            session = session_id,
                            tool = %call.name,
                            "dedup guard: aborting duplicate failed call"
                        );
                        log_pipeline_step(
                            session_id,
                            "tool_retry_blocked",
                            "Blocked duplicate failed tool call",
                            Some(serde_json::json!({
                                "round": round,
                                "tool": call.name.clone(),
                                "fail_count": fail_count,
                                "cached_error": cached_err,
                            })),
                        );
                        let _ = event_tx.send(StreamEvent::Error(abort_msg.clone()));
                        return;
                    }
                }
                // ── Turn budget guard: skip tool if cumulative tokens exhausted ─
                // Phase 4: Use provider-aware budget from context_budgets
                if check_tool_result_budget(turn_tool_tokens, &context_budgets) {
                    let budget_msg = format!(
                        "TOOL_BUDGET_EXHAUSTED: turn tool-output token budget ({}) \
                         reached; skipping '{}'. Summarise what you have and answer the user.",
                        context_budgets.turn_tool_budget, call.name
                    );
                    tracing::warn!(
                        session = session_id,
                        turn_tool_tokens,
                        tool = %call.name,
                        budget = context_budgets.turn_tool_budget,
                        context_window = active_context_window,
                        "turn tool-output budget exhausted; skipping tool"
                    );
                    messages.push(ChatMessage {
                        role: "tool".into(),
                        content: budget_msg,
                        name: Some(call.name.clone()),
                        images: None,
                    });
                    continue;
                }

                // ── Heartbeat: emit ToolProgress every 2 s while tool runs ─────
                let hb_cancel = CancellationToken::new();
                let hb_cancel_clone = hb_cancel.clone();
                let hb_tx = event_tx.clone();
                let hb_tool = call.name.clone();
                let hb_admission = Arc::clone(&turn_admission_for_async);
                let hb_session_id = session_id_for_async.clone();
                let hb_turn_id = turn_id_for_async.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
                    interval.tick().await; // skip the immediate first tick
                    loop {
                        tokio::select! {
                            biased;
                            _ = hb_cancel_clone.cancelled() => break,
                            _ = interval.tick() => {
                                if !hb_admission.is_active(&hb_session_id, &hb_turn_id) {
                                    break;
                                }
                                let _ = hb_tx.send(StreamEvent::ToolProgress {
                                    call_id: hb_tool.clone(),
                                    message: format!("⏳ {} is still running…", hb_tool),
                                    percent: None,
                                });
                            }
                        }
                    }
                });

                // Execute the tool
                let mut tool_result = if let Some(handler) =
                    self.tool_registry.get_handler(&call.name)
                {
                    let handler = handler.clone();
                    let args = execution_args.clone();

                    // ── Phase 3: Preflight validation ────────────────────────
                    // Run before spawning any subprocess. Fail fast on obviously
                    // dangerous or malformed arguments. Non-blocking, deterministic.
                    let preflight = crate::tools::preflight::run_preflight(&call.name, &args);
                    if !preflight.allowed {
                        let reason = preflight
                            .blocked_reason
                            .unwrap_or_else(|| "preflight validation failed".to_string());
                        tracing::warn!(
                            tool = %call.name,
                            reason = %reason,
                            "preflight blocked tool execution"
                        );
                        log_pipeline_step(
                            session_id,
                            "preflight_blocked",
                            "Preflight validation blocked tool execution",
                            Some(serde_json::json!({
                                "round": round,
                                "tool": call.name.clone(),
                                "reason": reason.clone(),
                            })),
                        );
                        hb_cancel.cancel();
                        crate::infra::isolation::ToolResult::err(format!(
                            "PREFLIGHT_BLOCKED: {reason}"
                        ))
                    } else {
                        // Log any preflight warnings (non-blocking)
                        for warning in &preflight.warnings {
                            tracing::info!(
                                tool = %call.name,
                                warning = %warning,
                                "preflight warning"
                            );
                        }

                        // ── Execution Authority: target validation ────────────
                        // Validate that this tool is allowed to execute on the
                        // resolved target. Blocks dangerous cross-target mismatches.
                        //
                        // Phase 1 pre-check: Use new environment-aware validation
                        // to prevent browser/desktop/MCP category errors from
                        // blocking legitimate tool execution. The new validator
                        // maps Browser/Mcp/CloudProvider targets to Host environment
                        // before checking, eliminating the category conflation bug.
                        let env_precheck =
                            crate::agent::execution_environment::validate_environment(
                                &call.name,
                                turn_memory.primary_target,
                            );
                        if !env_precheck.is_allowed() {
                            if let crate::agent::execution_environment::EnvironmentValidation::Blocked {
                                reason, ..
                            } = &env_precheck {
                                tracing::warn!(
                                    tool = %call.name,
                                    reason = %reason,
                                    "execution_environment: environment validation blocked"
                                );
                                log_pipeline_step(
                                    session_id,
                                    "execution_environment_blocked",
                                    "New environment validation blocked tool",
                                    Some(serde_json::json!({
                                        "round": round,
                                        "tool": call.name.clone(),
                                        "reason": reason,
                                    })),
                                );
                            }
                            // Fall through to legacy validation — it may have
                            // additional context (clarification questions, etc.)
                            // that the new validator doesn't yet provide.
                        }

                        let authority_result =
                            crate::agent::execution_authority::check_execution_authority_with_params(
                                &call.name,
                                &routing_focus_text,
                                turn_memory.primary_target,
                                Some(&call.arguments),
                            );

                        // Phase 1 override: If legacy validation blocks but new
                        // environment validation allows, OVERRIDE the block.
                        // This eliminates the browser-target-mismatch class of bugs
                        // while preserving all other legacy safety checks.
                        let authority_result = match &authority_result {
                            crate::agent::execution_authority::ValidationResult::Blocked {
                                reason,
                                ..
                            } if env_precheck.is_allowed()
                                && reason.contains("Target mismatch") =>
                            {
                                tracing::info!(
                                    tool = %call.name,
                                    legacy_reason = %reason,
                                    "execution_authority: legacy block OVERRIDDEN by new environment validation (category error fix)"
                                );
                                log_pipeline_step(
                                    session_id,
                                    "execution_authority_override",
                                    "Legacy target-mismatch block overridden by environment validation",
                                    Some(serde_json::json!({
                                        "round": round,
                                        "tool": call.name.clone(),
                                        "legacy_reason": reason,
                                        "new_environment": format!("{:?}", crate::agent::execution_environment::to_environment(turn_memory.primary_target)),
                                    })),
                                );
                                // Authorize with the correct environment binding
                                crate::agent::execution_authority::ValidationResult::Authorized(
                                    crate::agent::execution_authority::ExecutionBinding {
                                        target: turn_memory.primary_target,
                                        confidence: 0.90,
                                        source: crate::agent::execution_authority::BindingSource::ContextInferred,
                                        is_destructive: false,
                                        is_explicit: false,
                                    }
                                )
                            }
                            _ => authority_result,
                        };

                        match &authority_result {
                            crate::agent::execution_authority::ValidationResult::Blocked { reason, suggested_clarification } => {
                                let block_msg = format!("EXECUTION_BLOCKED: {reason}");
                                tracing::warn!(
                                    tool = %call.name,
                                    reason = %reason,
                                    "execution_authority: target mismatch blocked"
                                );
                                log_pipeline_step(
                                    session_id,
                                    "execution_authority_blocked",
                                    "Execution authority blocked tool — target mismatch",
                                    Some(serde_json::json!({
                                        "round": round,
                                        "tool": call.name.clone(),
                                        "reason": reason,
                                        "clarification": suggested_clarification,
                                    })),
                                );
                                hb_cancel.cancel();
                                // Inject clarification hint into messages so LLM can explain
                                if let Some(clarification) = suggested_clarification {
                                    messages.push(crate::llm::ChatMessage {
                                        role: "system".into(),
                                        content: format!(
                                            "EXECUTION_BLOCKED: {}. Clarification: {}",
                                            reason, clarification
                                        ),
                                        name: None,
                                        images: None,
                                    });
                                }
                                crate::infra::isolation::ToolResult::err(block_msg)
                            }
                            crate::agent::execution_authority::ValidationResult::NeedsClarification { question, options } => {
                                tracing::info!(
                                    tool = %call.name,
                                    question = %question,
                                    "execution_authority: clarification needed"
                                );
                                log_pipeline_step(
                                    session_id,
                                    "execution_authority_clarify",
                                    "Execution authority needs clarification before proceeding",
                                    Some(serde_json::json!({
                                        "round": round,
                                        "tool": call.name.clone(),
                                        "question": question,
                                        "options": options,
                                    })),
                                );
                                hb_cancel.cancel();
                                // Inject clarification question into messages
                                messages.push(crate::llm::ChatMessage {
                                    role: "system".into(),
                                    content: format!(
                                        "CLARIFICATION_NEEDED before executing '{}': {}",
                                        call.name, question
                                    ),
                                    name: None,
                                    images: None,
                                });
                                crate::infra::isolation::ToolResult::err(format!(
                                    "CLARIFICATION_NEEDED: {question}"
                                ))
                            }
                            crate::agent::execution_authority::ValidationResult::Authorized(binding) => {
                                log_pipeline_step(
                                    session_id,
                                    "execution_authority_ok",
                                    "Execution authority validated",
                                    Some(serde_json::json!({
                                        "round": round,
                                        "tool": call.name.clone(),
                                        "target": binding.target.as_str(),
                                        "confidence": binding.confidence,
                                        "source": binding.source.as_str(),
                                        "is_destructive": binding.is_destructive,
                                        "is_explicit": binding.is_explicit,
                                    })),
                                );

                        // Long-running tools get extended timeouts
                        let timeout_secs = match call.name.as_str() {
                            "install_application"
                            | "uninstall_application"
                            | "update_all_packages"
                            | "install_package"
                            | "uninstall_package"
                            | "execute_fleet_command" => 300,
                            "generate_image" => 300,
                            "search_news" | "fetch_article" => 60,
                            "execute_bash" | "execute_python" | "execute_powershell" => 120,
                            "download_file" => 120,
                            _ => 30,
                        };
                        let execution_cancel = if call.name == "generate_image" {
                            turn_image_cancel.clone()
                        } else if call.name.starts_with("mcp_") {
                            turn_mcp_cancel.clone()
                        } else if is_sidecar_backed_tool_name(&call.name) {
                            turn_sidecar_cancel.clone()
                        } else {
                            turn_tools_cancel.clone()
                        };
                        let isolation_name = format!("tool:{}", call.name);
                        // Injection wall (Req 9, per-turn — Task 3): if THIS turn has
                        // already run an external-content tool, config-mutating tools see
                        // ExternalContent and refuse. Provenance is decided BEFORE the
                        // tool runs; then this tool (if external) taints later calls.
                        let call_provenance = if turn_external_taint
                            .load(std::sync::atomic::Ordering::SeqCst)
                        {
                            crate::tools::TriggerProvenance::ExternalContent
                        } else {
                            crate::tools::TriggerProvenance::User
                        };
                        if self.tool_registry.is_external_content_tool(&call.name) {
                            turn_external_taint
                                .store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        let tool_context = self
                            .tool_registry
                            .make_tool_context_with_provenance(
                                execution_cancel.clone(),
                                call_provenance,
                            );
                        run_isolated(
                            &isolation_name,
                            std::time::Duration::from_secs(timeout_secs),
                            execution_cancel,
                            None,
                            move || async move {
                                handler.execute_with_context(args, tool_context).await
                            },
                        )
                        .await
                    } // end Authorized arm
                    } // end authority match
                    } // end preflight else
                } else {
                    crate::infra::isolation::ToolResult::err(format!("unknown tool: {}", call.name))
                };

                if return_if_stale() {
                    return;
                }

                // Stop the heartbeat task.
                hb_cancel.cancel();

                tracing::info!(
                    target: "tool_execution",
                    session = session_id,
                    execution_id = %tool_execution_id,
                    tool_name = %call.name,
                    duration_ms = tool_execution_started.elapsed().as_millis(),
                    success = tool_result.success,
                    failure_reason = %tool_result.error.as_deref().unwrap_or("-"),
                    result_summary = %sanitize_json_for_logs(&tool_result.data, 220, 8),
                    "Tool execution completed"
                );

                // ── Phase 3: Post-execution verification ─────────────────────
                // Validate tool results for non-trivial tools when a verifier is attached.
                // The verifier NEVER retries, replans, or mutates state — it only logs.
                let verification_outcome = if tool_result.success {
                    if let Some(ref verifier) = self.execution_verifier {
                        let verifiability =
                            infer_verifiability_for_tool(&call.name, &execution_args, &tool_result);
                        if let Some(leaf) = verifiability {
                            let outcome = verifier.verify(&leaf).await;
                            tracing::info!(
                                tool = %call.name,
                                verified = outcome.verified,
                                confidence = outcome.confidence,
                                evidence = %outcome.evidence,
                                latency_ms = outcome.latency_ms,
                                "execution_verifier: result"
                            );
                            log_pipeline_step(
                                session_id,
                                "execution_verified",
                                "Post-execution verification completed",
                                Some(serde_json::json!({
                                    "round": round,
                                    "tool": call.name.clone(),
                                    "verified": outcome.verified,
                                    "confidence": outcome.confidence,
                                    "evidence": outcome.evidence,
                                    "latency_ms": outcome.latency_ms,
                                })),
                            );
                            if !outcome.verified {
                                // AUDIT FIX: verification failure must gate the result.
                                // Downgrade the tool result so the LLM sees failure,
                                // not a falsely-successful outcome.
                                self.audit_logger.log(
                                    session_id,
                                    &call.name,
                                    &execution_args,
                                    decision.risk_level,
                                    crate::safety::audit::Decision::Blocked,
                                    crate::safety::audit::DecidedBy::Verification,
                                );
                                tracing::warn!(
                                    target: "authority_trace",
                                    tool = %call.name,
                                    evidence = %outcome.evidence,
                                    "execution verifier blocked: claimed success but verification failed"
                                );
                                tool_result = crate::infra::isolation::ToolResult::err(format!(
                                    "VERIFICATION_FAILED: tool reported success but post-execution verification failed: {}",
                                    outcome.evidence
                                ));
                                log_pipeline_step(
                                    session_id,
                                    "execution_verification_blocked",
                                    "Tool result blocked by post-execution verification failure",
                                    Some(serde_json::json!({
                                        "round": round,
                                        "tool": call.name.clone(),
                                        "evidence": outcome.evidence,
                                        "confidence": outcome.confidence,
                                    })),
                                );
                            }
                            Some(VerificationOutcome {
                                verified: outcome.verified,
                                confidence: outcome.confidence as f64,
                                evidence: outcome.evidence,
                            })
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                // ── Phase 3.5: Result Synthesis ──────────────────────────────
                // Transform raw tool output into intelligent user-facing response.
                // This separates conversational summary from debug/raw payload.
                let synthesized = self.result_synthesizer.synthesize(
                    &call.name,
                    &tool_result,
                    verification_outcome,
                );

                tracing::debug!(
                    tool = %call.name,
                    outcome = ?synthesized.execution_metadata.outcome,
                    item_count = ?synthesized.execution_metadata.item_count,
                    summary_preview = %sanitize_text_for_logs(&synthesized.conversational_summary, 120),
                    "result_synthesizer: generated conversational summary"
                );

                log_pipeline_step(
                    session_id,
                    "result_synthesized",
                    "Tool result synthesized into conversational response",
                    Some(serde_json::json!({
                        "round": round,
                        "tool": call.name.clone(),
                        "outcome": synthesized.execution_metadata.outcome,
                        "item_count": synthesized.execution_metadata.item_count,
                        "summary_preview": sanitize_text_for_logs(&synthesized.conversational_summary, 200),
                    })),
                );

                // ── Session Checkpoint + Observability ──────────────────────────
                if let (Some(ref mut session), Some(ref mgr)) =
                    (&mut react_session, &self.session_manager)
                {
                    session.add_step(crate::agent::workflow_session::SessionStep {
                        step: round,
                        action: call.name.clone(),
                        params: execution_args.clone(),
                        success: tool_result.success,
                        evidence: tool_result.data.to_string().chars().take(500).collect(),
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0),
                    });
                    let _ = mgr.save(session);
                }
                // ── Transparency layer: record tool execution as completed stage ─
                if let Some(ref layer) = self.transparency_layer {
                    use crate::agent::stage_executor::StageOutcome;
                    let stage_outcome = if tool_result.success {
                        StageOutcome::Passed
                    } else {
                        StageOutcome::Failed {
                            reason: tool_result
                                .data
                                .as_str()
                                .unwrap_or("tool execution failed")
                                .chars()
                                .take(120)
                                .collect(),
                        }
                    };
                    layer.update_stage(
                        &react_trace_id,
                        round as u32,
                        &call.name,
                        &stage_outcome,
                        1, // attempts
                        0, // recovery_attempts
                        0, // duration_ms (not tracked per-tool in ReAct)
                        if tool_result.success { 0.9 } else { 0.0 },
                    );
                }
                if let Some(ref health) = self.health_registry {
                    health.inc_events(1);
                }

                // ── Turn Memory: record success + check satisfaction ───────────
                if tool_result.success {
                    let result_preview: String =
                        tool_result.data.to_string().chars().take(200).collect();
                    let tool_target = ExecutionTarget::infer(&routing_focus_text, &call.name);
                    // Pass full result data so satisfaction summary can synthesize properly
                    turn_memory.record_success_with_data(
                        &call.name,
                        call_hash,
                        &result_preview,
                        tool_result.data.clone(),
                        tool_target,
                    );

                    // Core-level tool memory (design §46.1): record the successful
                    // execution through the Write Policy so procedural/capability
                    // knowledge accrues for EVERY entry point (server, telegram,
                    // desktop). Dedup coalesces with any caller-side recording.
                    self.record_agent_outcome(
                        session_id,
                        &call.name,
                        &format!("tool {} succeeded: {}", call.name, result_preview),
                    );

                    // Learning loop: credit the grounding memories positively —
                    // they informed a turn that produced a successful action.
                    if !grounding_credited {
                        self.credit_grounding(&grounding_memory_ids, true);
                        // Adaptive RRF: the winning retrieval strategy for this
                        // query class grounded a successful turn (Priority 1).
                        if let (Some(ms), Some((class, strat))) =
                            (self.memory_system.as_ref(), grounding_retrieval)
                        {
                            ms.reinforce_retrieval(class, strat);
                        }
                        grounding_credited = true;
                    }

                    // Planning learning loop: the tool worked for this task.
                    self.record_plan_step(&routing_focus_text, &call.name, true);
                    // Reasoning memory: a chain that reached a successful action.
                    self.record_reasoning(
                        session_id,
                        &routing_focus_text,
                        &format!("used {} → success: {}", call.name, result_preview),
                        true,
                    );

                    // Check if the user's goal is now satisfied
                    if !turn_memory.is_satisfied() {
                        tracing::debug!(
                            session = session_id,
                            tool = %call.name,
                            goal = %turn_memory.goal,
                            "checking satisfaction for tool execution"
                        );
                        if let Some(reason) = detect_satisfaction(&turn_memory, &call.name, true) {
                            turn_memory.mark_satisfied(reason.clone());
                            tracing::warn!(
                                session = session_id,
                                tool = %call.name,
                                reason = %reason,
                                "🎯 SATISFACTION DETECTED - goal met, will skip remaining tools"
                            );
                            log_pipeline_step(
                                session_id,
                                "goal_satisfied",
                                "Task satisfaction detected — tool loop will terminate after this round",
                                Some(serde_json::json!({
                                    "round": round,
                                    "tool": call.name.clone(),
                                    "reason": reason,
                                    "memory": turn_memory.to_json(),
                                })),
                            );
                        } else {
                            tracing::debug!(
                                session = session_id,
                                tool = %call.name,
                                "satisfaction not detected for this tool"
                            );
                        }
                    }
                }

                // Phase 5: Record routing feedback for online learning
                if let Some(ref feedback_collector) = self.feedback_collector {
                    // Resolve the actual domain from the turn-gate plan
                    let domain_for_feedback = crate::routing::domain::category_to_domain(
                        &call
                            .name
                            .split('_')
                            .next()
                            .unwrap_or("conversation")
                            .to_lowercase(),
                    );
                    let outcome = crate::routing::feedback::detect_outcome(
                        domain_for_feedback,
                        Some(&call.name),
                        None, // next_text unknown at this point
                        tool_result.success,
                        tool_result.error.as_deref(),
                    );
                    let mut collector = feedback_collector.lock().await;
                    collector.record(crate::routing::feedback::RoutingFeedback {
                        input_text_hash: {
                            use std::hash::{Hash, Hasher};
                            let mut hasher = std::collections::hash_map::DefaultHasher::new();
                            routing_focus_text.hash(&mut hasher);
                            hasher.finish()
                        },
                        domain_selected: domain_for_feedback,
                        tool_selected: Some(call.name.clone()),
                        intent_source: format!("{:?}", turn_gate_plan.intent.source),
                        confidence: turn_gate_plan.intent.confidence,
                        outcome,
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        session_id: session_id.to_string(),
                        embedding: turn_query_embedding.clone(), // Gap 1 fixed
                    });
                }

                // ── Update error-loop counters ─────────────────────────────────
                if tool_result.success {
                    consecutive_failures = 0;
                } else {
                    let err_text = tool_result
                        .error
                        .clone()
                        .unwrap_or_else(|| "unknown error".to_string());

                    // Core-level failure memory (design §46.1 / "remember
                    // failures"): record the failed execution through the Write
                    // Policy so corrections/avoidance knowledge accrues for every
                    // entry point. Marked as a Failure by the governance
                    // classifier via the "failed" keyword in the content.
                    self.record_agent_outcome(
                        session_id,
                        &call.name,
                        &format!("tool {} failed: {}", call.name, err_text),
                    );

                    // Learning loop: weaken grounding memories that informed a
                    // turn whose action failed (negative credit, soft signal).
                    if !grounding_credited {
                        self.credit_grounding(&grounding_memory_ids, false);
                        grounding_credited = true;
                    }

                    // Planning learning loop: the tool failed for this task.
                    self.record_plan_step(&routing_focus_text, &call.name, false);
                    // Reasoning memory: a counterexample (this approach failed).
                    self.record_reasoning(
                        session_id,
                        &routing_focus_text,
                        &format!("used {} → failed: {}", call.name, err_text),
                        false,
                    );

                    let entry = failed_calls
                        .entry(call_hash)
                        .or_insert((0, err_text.clone()));
                    entry.0 += 1;
                    entry.1 = err_text;
                    consecutive_failures += 1;

                    let replanned = self.turn_gate.replan_after_error(
                        &turn_gate_plan,
                        &round_focus_text,
                        has_images,
                        &call.name,
                        &entry.1,
                    );
                    turn_gate_plan = replanned;

                    log_pipeline_step(
                        session_id,
                        "executor_replan_requested",
                        "Tool failure triggered TurnGate replanning",
                        Some(serde_json::json!({
                            "round": round,
                            "failed_tool": call.name.clone(),
                            "error": sanitize_text_for_logs(&entry.1, 220),
                            "replanned_operation": format!("{:?}", turn_gate_plan.intent.operation),
                            "replanned_compute": format!("{:?}", turn_gate_plan.intent.compute),
                            "replanned_confidence": turn_gate_plan.intent.confidence,
                        })),
                    );
                    let _ = event_tx.send(StreamEvent::Plan(format!(
                        "Replanning via TurnGate after '{}' failed.",
                        call.name
                    )));

                    if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                        tracing::warn!(
                            session = session_id,
                            consecutive_failures,
                            "3 consecutive tool failures — injecting corrective prompt"
                        );
                        log_pipeline_step(
                            session_id,
                            "consecutive_failures_threshold",
                            "3 consecutive tool failures; injecting corrective system message",
                            Some(serde_json::json!({ "round": round })),
                        );
                        // Inject a corrective system message so the LLM knows to
                        // stop using tools and answer with what it has.
                        messages.push(ChatMessage {
                            role: "system".into(),
                            content: "SYSTEM: 3 consecutive tool executions have failed. \
                                      Stop issuing tool calls. Respond to the user using \
                                      whatever information you have, or ask the user for \
                                      guidance to resolve the problem."
                                .to_string(),
                            name: None,
                            images: None,
                        });
                        // Reset so we don't inject repeatedly.
                        consecutive_failures = 0;
                    }
                }

                if let Some(flow) = capability_flow.as_mut() {
                    flow.observe_tool_result(call);
                }

                if let Some(flow) = package_flow.as_mut() {
                    flow.observe_tool_result(call, tool_result.success, &tool_result.data);

                    // ── Package flow step progress ─────────────────────────
                    // Emit TaskStep events so the UI shows live progress for
                    // multi-step package operations (search→check→install→verify).
                    // Steps are deterministic based on the flow's current state.
                    let pkg_step = package_flow_step_event(flow, &call.name, tool_result.success);
                    if let Some(step) = pkg_step {
                        let _ = event_tx.send(StreamEvent::TaskStep(step));
                    }
                }

                if let Some(flow) = colab_flow.as_mut() {
                    flow.observe_tool_result(call, tool_result.success, &tool_result.data);
                }

                if is_gmail_tool_name(&call.name) {
                    if tool_result.success {
                        had_successful_gmail_tool = true;
                        last_successful_gmail_result = Some(tool_result.data.clone());
                    } else {
                        had_failed_gmail_tool = true;
                    }
                }

                if call.name == "generate_image" && tool_result.success {
                    last_successful_image_result = Some(tool_result.data.clone());
                }

                // For generate_image failures: emit a structured user-visible message
                // and skip the LLM round so the user gets clear feedback immediately.
                if call.name == "generate_image" && !tool_result.success {
                    let failure_msg = build_image_failure_response(&tool_result.data);
                    tracing::warn!(
                        session = session_id,
                        "generate_image failed; returning structured failure to user"
                    );
                    let _ = event_tx.send(StreamEvent::Token(failure_msg.clone()));
                    let _ = event_tx.send(StreamEvent::Done(failure_msg));
                    return;
                }

                // Log GREEN/YELLOW auto-executed
                if !decision.requires_approval {
                    let eval_synthetic_approval = std::env::var("KRIA_EVAL_MODE").is_ok()
                        && decision.reason.contains("EvalHarness auto-approved");
                    let (audit_decision, decided_by) = if eval_synthetic_approval {
                        (Decision::Approved, DecidedBy::Hardcoded)
                    } else {
                        (Decision::AutoExecuted, DecidedBy::Policy)
                    };

                    self.audit_logger.log(
                        session_id,
                        &call.name,
                        &call.arguments,
                        decision.risk_level,
                        audit_decision,
                        decided_by,
                    );
                }

                // Build the string the LLM will see.
                // IMPORTANT: if the tool failed, send the error — not "null" —
                // so the LLM knows to report the failure instead of hallucinating.
                //
                // For successful results we apply a two-stage budget strategy:
                //   1. Shape the raw payload (drop bodies/base64, truncate strings)
                //      using the domain-aware shaper.  Gmail payloads first go
                //      through the existing compact_tool_result_for_llm() path.
                //   2. Count tokens via llama.cpp /tokenize; if still over the
                //      per-tool budget, re-shape with a tighter char budget.
                //   3. Hard char-cap as a final safety net.
                let mut extracted_tool_images =
                    if tool_result.success && call.name == "analyze_image" {
                        extract_preprocessed_image_attachments(&tool_result.data, "image/jpeg")
                    } else {
                        None
                    };
                let extracted_tool_image_count = extracted_tool_images
                    .as_ref()
                    .map(|imgs| imgs.len())
                    .unwrap_or(0);
                if !inline_images_allowed_for_turn {
                    extracted_tool_images = None;
                }

                let llm_tool_result = compact_tool_result_for_llm(&call.name, &tool_result.data);

                // Layer 4: Freshness pruning for live-fact queries
                // Uses 30-day window for stable facts (political offices, etc.) — 7-day was
                // too aggressive and dropped Wikipedia/encyclopedic sources that are highly
                // relevant even if older. Also matches web_search tool (not just searxng_search).
                let llm_tool_result = if is_live_fact
                    && matches!(
                        call.name.as_str(),
                        "searxng_search" | "web_search" | "search_news"
                    ) {
                    prune_stale_search_results(llm_tool_result, 30)
                } else {
                    llm_tool_result
                };

                let result_str = if !tool_result.success {
                    let err_msg = tool_result
                        .error
                        .as_deref()
                        .unwrap_or("tool execution failed with no details");
                    format!("TOOL_ERROR: {err_msg}")
                } else {
                    // ── Context Bomb mitigation ────────────────────────────
                    // Per-tool char budget derived from token budget.
                    let char_budget = LLM_TOOL_RESULT_TOKEN_BUDGET * 4; // ~4 chars/token heuristic

                    // Stream the full payload to the UI via ToolPayloadChunk so
                    // the user always sees complete data while the LLM only gets
                    // the compact summary.
                    let full_payload_str = llm_tool_result.to_string();
                    if full_payload_str.len() > char_budget {
                        // Emit a single final chunk with full data for UI rendering.
                        let _ = event_tx.send(StreamEvent::ToolPayloadChunk {
                            call_id: call.name.clone(),
                            seq: 0,
                            is_final: true,
                            data: llm_tool_result.clone(),
                        });
                    }

                    // Stage 1: structural shaping.
                    let shaped = shape_for_llm(&call.name, &llm_tool_result, char_budget);
                    let mut shaped_str = shaped.value.to_string();

                    // Stage 2: token counting — tighten budget if needed.
                    let tokenizer_url = backend.tokenizer_base_url();
                    let token_count = count_tokens(&shaped_str, &tokenizer_url).await;
                    if token_count > LLM_TOOL_RESULT_TOKEN_BUDGET {
                        // Re-shape with a char budget proportional to how much
                        // we need to shrink.
                        let tighter =
                            (char_budget * LLM_TOOL_RESULT_TOKEN_BUDGET / token_count).max(512);
                        let reshaped = shape_for_llm(&call.name, &llm_tool_result, tighter);
                        shaped_str = reshaped.value.to_string();
                    }

                    // Stage 3: hard char cap as final safety net.
                    if shaped_str.len() > TOOL_RESULT_MAX_CHARS {
                        format!("{}...<truncated>", &shaped_str[..TOOL_RESULT_MAX_CHARS])
                    } else {
                        shaped_str
                    }
                };

                // Update the cumulative turn token counter.
                let result_tokens = count_tokens(&result_str, &backend.tokenizer_base_url()).await;
                turn_tool_tokens = turn_tool_tokens.saturating_add(result_tokens);
                // Phase 4: Update ledger with tool result tokens
                let ledger_total = turn_ledger.add_tool_result(result_tokens);
                let _ = ledger_total; // used for logging below

                // ── Phase 4: Inter-tool budget check ─────────────────────────
                // Check if cumulative context growth requires compaction or loop break.
                // This catches context explosion from many tool calls in a single turn.
                match check_inter_tool_budget(&messages, &context_budgets) {
                    BudgetCheckResult::Ok => {}
                    BudgetCheckResult::CompactRequired => {
                        turn_ledger.record_compaction();
                        tracing::warn!(
                            session = session_id,
                            round,
                            tool = %call.name,
                            context_window = active_context_window,
                            "inter-tool budget: context at 75%+, compacting history"
                        );
                        compact_messages_with_budgets(messages, &context_budgets);
                    }
                    BudgetCheckResult::ExhaustedBreak => {
                        turn_ledger.record_compaction();
                        tracing::error!(
                            session = session_id,
                            round,
                            tool = %call.name,
                            context_window = active_context_window,
                            "inter-tool budget: context at 87.5%+, breaking tool loop"
                        );
                        compact_messages_with_budgets(messages, &context_budgets);
                        messages.push(ChatMessage {
                            role: "system".into(),
                            content: "Context budget exhausted. Summarize results so far and respond to the user.".into(),
                            name: None,
                            images: None,
                        });
                        break;
                    }
                }

                // Auto-route: if tool result contains a file path, check if a
                // precognitive tool should process it automatically
                let auto_enrichment = self
                    .auto_route_file_result(&call.name, &tool_result.data)
                    .await;

                log_pipeline_step(
                    session_id,
                    "tool_result_ready",
                    "Tool execution completed",
                    Some(serde_json::json!({
                        "round": round,
                        "tool": call.name.clone(),
                        "success": tool_result.success,
                        "error": tool_result
                            .error
                            .as_ref()
                            .map(|e| sanitize_text_for_logs(e, 220)),
                        "migrated_tool_images": extracted_tool_image_count,
                        "result_preview": sanitize_json_for_logs(&tool_result.data, 220, 8),
                        "result_tokens": result_tokens,
                        "turn_tool_tokens_total": turn_tool_tokens,
                        // Phase 4: ledger snapshot for observability
                        "ledger": turn_ledger.snapshot().to_json(),
                        "context_window": active_context_window,
                        "pressure": context_budgets.pressure_level(turn_ledger.total_estimated()).as_str(),
                        "auto_enriched": auto_enrichment.is_some(),
                    })),
                );

                // PRODUCTION HARDENING FIX (Phase 10: error system audit): on a
                // failed tool call, `tool_result.data` is always `Value::Null`
                // (see `ToolResult::err`'s constructor) — the REAL error string
                // lives in the separate `tool_result.error` field, which this
                // event never forwarded. The frontend's raw "Result:" display
                // reads `result.error`, so every failed tool call showed the
                // generic "unknown error" fallback regardless of what actually
                // went wrong (root-caused via a real OpenClaw registry-empty
                // failure that displayed as "unknown error" instead of the true
                // "No suitable skill found: No enabled skills found in registry").
                // `human_readable`/`conversational_summary` already carried the
                // real message correctly (via `ResultSynthesizer::synthesize_failure`
                // reading `tool_result.error` directly) — this fix makes the raw
                // `result` payload consistent with those, instead of silently
                // dropping the one field that actually explains the failure.
                let tool_end_result = tool_end_result_payload(&tool_result);
                let _ = event_tx.send(StreamEvent::ToolEnd {
                    name: call.name.clone(),
                    result: tool_end_result,
                    success: tool_result.success,
                    human_readable: Some(synthesized.human_readable.clone()),
                    conversational_summary: Some(synthesized.conversational_summary.clone()),
                    execution_metadata: Some(
                        serde_json::to_value(&synthesized.execution_metadata).unwrap_or_default(),
                    ),
                });

                let tool_msg = if let Some(enrichment) = auto_enrichment {
                    format!(
                        "{}\n\n[Auto-enriched via sidecar]\n{}",
                        result_str, enrichment
                    )
                } else {
                    result_str
                };

                // Issue 6 fix: For live-fact queries, inject a CRITICAL instruction
                // directly into the tool result message for search tools. This places
                // the instruction RIGHT NEXT to the search results where the LLM can
                // see it, rather than only in the system prompt which may be far away.
                let tool_msg = if is_live_fact
                    && matches!(
                        call.name.as_str(),
                        "searxng_search" | "web_search" | "search_news"
                    )
                    && tool_result.success
                {
                    format!(
                        "[SYSTEM: LIVE FACT MODE — You MUST answer from these search results ONLY. \
                        Do NOT use training data that contradicts these results. \
                        Trust the search evidence above all else.]\n{}",
                        tool_msg
                    )
                } else {
                    tool_msg
                };

                let tool_msg =
                    if let Some(note) = build_grounding_count_note(&call.name, &llm_tool_result) {
                        format!("{tool_msg}\n\n{note}")
                    } else {
                        tool_msg
                    };

                // ── Hallucination guard: failed extraction ────────────────────
                // If fetch_webpage or ingest_document failed, or returned an
                // EXTRACTION_FAILED marker, inject an explicit anti-hallucination
                // instruction so the LLM does NOT fabricate content.
                let tool_msg = if !tool_result.success
                    && matches!(
                        call.name.as_str(),
                        "fetch_webpage" | "fetch_article" | "ingest_document"
                    ) {
                    format!(
                        "{tool_msg}\n\nEXTRACTION_FAILED: The content of this URL/document could \
                        not be retrieved or extracted. You MUST NOT fabricate, invent, or guess \
                        the content. Tell the user the extraction failed and ask them to provide \
                        the content directly, or try a different URL."
                    )
                } else if tool_result.success
                    && call.name == "fetch_webpage"
                    && tool_result
                        .data
                        .get("content")
                        .and_then(|v| v.as_str())
                        .map(|c| c.starts_with("EXTRACTION_FAILED"))
                        .unwrap_or(false)
                {
                    format!(
                        "{tool_msg}\n\nEXTRACTION_FAILED: No readable article content could be \
                        extracted from this page (likely a JavaScript-heavy SPA, paywall, or \
                        bot-detection page). You MUST NOT fabricate the content. \
                        Tell the user the page could not be read and suggest alternatives."
                    )
                } else {
                    tool_msg
                };

                messages.push(ChatMessage {
                    role: "tool".into(),
                    content: tool_msg,
                    name: Some(call.name.clone()),
                    images: extracted_tool_images,
                });

                // ── Gap 1 & 4: Command-level failure diagnosis ────────────────
                // When a tool fails, classify the failure deterministically and
                // emit structured RecoveryOptions so the UI shows clickable buttons.
                // This is model-agnostic — no LLM calls, pure pattern matching.
                if !tool_result.success {
                    let error_text = tool_result.error.as_deref().unwrap_or("");
                    if let Some(recovery) =
                        classify_tool_failure(&call.name, error_text, &execution_args)
                    {
                        tracing::info!(
                            session = session_id,
                            tool = %call.name,
                            context = %recovery.0,
                            "tool_failure_recovery: emitting structured recovery options"
                        );
                        let _ = event_tx.send(StreamEvent::RecoveryOptions {
                            context: recovery.0,
                            detail: recovery.1,
                            options: recovery.2,
                        });
                    }
                    // ── Batch 2 Phase 4: Interruption classification + recovery planning ─
                    // Classifies the tool failure as a known interruption class and
                    // plans a bounded recovery action (depth ≤ MAX_RECOVERY_DEPTH).
                    // Plans are logged for audit and recorded as transparency blockers.
                    if let Some(ref rt) = self.continuation_runtime {
                        use crate::agent::workflow_continuation::InterruptionContext;
                        let interruption_ctx = InterruptionContext {
                            current_stage_label: Some(call.name.clone()),
                            ..Default::default()
                        };
                        let interruption = rt.classify_interruption(&interruption_ctx);
                        if !matches!(
                            interruption,
                            crate::agent::workflow_continuation::InterruptionClass::Unknown
                        ) {
                            let plan = rt.plan_recovery(&interruption, consecutive_failures);
                            tracing::info!(
                                target: "workflow_continuation",
                                session = session_id,
                                tool = %call.name,
                                interruption = %interruption.user_message(),
                                "Batch 2: interruption classified; recovery plan ready"
                            );
                            log_pipeline_step(
                                session_id,
                                "b2_recovery_planned",
                                &plan.explanation,
                                Some(serde_json::json!({
                                    "tool": call.name.clone(),
                                    "interruption": interruption.user_message(),
                                    "primary_action": format!("{:?}", plan.primary_action),
                                    "consecutive_failures": consecutive_failures,
                                })),
                            );
                            if let Some(ref layer) = self.transparency_layer {
                                layer.record_blocker(
                                    &react_trace_id,
                                    round as u32,
                                    interruption.user_message(),
                                    plan.explanation.clone(),
                                );
                            }
                            // ── Batch 2: Pause workflow on human-intervention / escalation ─
                            // Writes a crash-safe pause checkpoint to disk so the workflow
                            // can be resumed after the user resolves the blocker.
                            let needs_pause = matches!(
                                plan.primary_action,
                                crate::agent::workflow_continuation::RecoveryAction::RequestHumanIntervention { .. }
                                | crate::agent::workflow_continuation::RecoveryAction::Escalate { .. }
                            );
                            if needs_pause {
                                use crate::agent::workflow_session::WorkflowSession;
                                let react_session = WorkflowSession::new(
                                    session_id.to_string(),
                                    last_user_text.chars().take(200).collect::<String>(),
                                    "ReAct".to_string(),
                                );
                                let wf_cat_str = format!(
                                    "{:?}",
                                    b2_workflow_category
                                        .as_ref()
                                        .map(|c| format!("{:?}", c))
                                        .unwrap_or_else(|| "Unknown".into())
                                );
                                let _ = rt.pause_workflow(
                                    session_id,
                                    &react_session,
                                    interruption.clone(),
                                    &wf_cat_str,
                                );
                                tracing::warn!(
                                    target: "workflow_continuation",
                                    session = session_id,
                                    tool = %call.name,
                                    "Batch 2: ReAct workflow paused — awaiting human intervention"
                                );
                                log_pipeline_step(
                                    session_id,
                                    "b2_workflow_paused",
                                    &format!("Workflow paused: {}", plan.explanation),
                                    None,
                                );
                            }
                        }
                    }
                } else if call.name == "execute_fleet_command" {
                    // Mark step 2 done after successful fleet command
                    let _ = event_tx.send(StreamEvent::TaskStep(TaskStep {
                        index: 2,
                        total: Some(2),
                        description: "Command completed successfully".into(),
                        status: TaskStepStatus::Done,
                    }));
                }
            }

            // ── Image-generation early exit ────────────────────────────────────────
            // When generate_image succeeded this round, skip the round-N LLM summary
            // call entirely — that call would crash the GPU with ctx=2048 + 167 schemas.
            // Instead, emit a pre-built confirmation response and return immediately.
            if let Some(ref img_data) = last_successful_image_result {
                if return_if_stale() {
                    return;
                }

                let summary = build_image_success_response(img_data);
                log_pipeline_step(
                    session_id,
                    "final_output_ready",
                    "Image generation succeeded; skipping LLM summary call",
                    Some(serde_json::json!({
                        "round": round,
                        "final_preview": sanitize_text_for_logs(&summary, 280),
                    })),
                );
                let _ = event_tx.send(StreamEvent::Token(summary.clone()));
                let _ = event_tx.send(StreamEvent::Done(summary));
                return;
            }

            log_pipeline_step(
                session_id,
                "round_completed",
                "Round completed with tool outputs appended; continuing loop",
                Some(serde_json::json!({
                    "round": round,
                    "history_message_count": messages.len(),
                })),
            );
        }

        // ── Batch 2 Phase 5: Complete transparency trace ──────────────────────────
        // Closes the per-turn WorkflowTrace opened in begin_trace() at turn start.
        // Records overall success/failure for PSDG audit persistence.
        {
            let react_turn_succeeded = failed_calls.is_empty();
            if let Some(ref layer) = self.transparency_layer {
                layer.complete_trace(
                    &react_trace_id,
                    react_turn_succeeded,
                    if react_turn_succeeded {
                        None
                    } else {
                        Some(format!(
                            "{} tool(s) had failures during this turn",
                            failed_calls.len()
                        ))
                    },
                );
                tracing::debug!(
                    target: "execution_transparency",
                    session = session_id,
                    trace_id = %react_trace_id,
                    success = react_turn_succeeded,
                    "Batch 2: ReAct transparency trace completed"
                );
            }
        }

        // ── Batch 2 Phase 1 (closure): Observable completion verification ─────────
        // Verifies that expected human-visible outcomes are observable after the
        // full tool loop. Only runs when non-Silent outcomes were inferred at turn
        // start and the observable_completion engine is wired. PSDG fast-path
        // ensures browser/IDE/file checks are < 10ms. Surfaces a notice to the
        // user when expected outcomes are not yet visible.
        if !b2_inferred_outcomes.is_empty() {
            if let Some(ref eng) = self.observable_completion {
                use crate::agent::observable_completion::CompletionVisibilityPolicy;
                let policies: Vec<CompletionVisibilityPolicy> = b2_inferred_outcomes
                    .iter()
                    .map(|o| {
                        CompletionVisibilityPolicy::for_outcome(
                            o.clone(),
                            turn_gate_plan.intent.operation,
                        )
                    })
                    .collect();
                let agg = eng.verify_all(&policies).await;
                tracing::info!(
                    target: "observable_completion",
                    session = session_id,
                    all_visible = agg.all_required_visible,
                    confidence = agg.overall_confidence,
                    surfacing_needed = agg.surfacing_needed,
                    "Batch 2: observable completion check after ReAct loop"
                );
                log_pipeline_step(
                    session_id,
                    "b2_observable_completion",
                    "Batch 2: observable completion verification after ReAct loop",
                    Some(serde_json::json!({
                        "all_required_visible": agg.all_required_visible,
                        "overall_confidence": agg.overall_confidence,
                        "surfacing_needed": agg.surfacing_needed,
                        "outcome_count": agg.per_outcome.len(),
                    })),
                );
                if agg.surfacing_needed && !agg.all_required_visible {
                    let narrative = eng.completion_narrative(&agg, turn_gate_plan.intent.operation);
                    if !narrative.is_empty() {
                        tracing::warn!(
                            target: "observable_completion",
                            session = session_id,
                            %narrative,
                            "Batch 2: expected outcome not yet visible — surfacing to user"
                        );
                        let _ = event_tx.send(StreamEvent::Plan(narrative));
                    }
                }
            }
        }

        if terminated_by_satisfaction {
            log_pipeline_step(
                session_id,
                "loop_terminated_satisfied",
                "Agent loop terminated early: goal satisfied",
                Some(serde_json::json!({
                    "reason": turn_memory.satisfaction_reason(),
                })),
            );

            // BUG #7 FIX (category L: State Management issue). Root cause: this
            // bare `if !is_turn_active() { return; }` returned with ZERO events
            // emitted, unlike the established `return_if_stale()` pattern used
            // everywhere else in this function (which always emits
            // `StreamEvent::Done("Turn cancelled.")` before returning). A turn
            // that reached here with a stale admission state — e.g. because an
            // unusual/ambiguous prompt like "Decompress this concept..." routed
            // through a slower path and got superseded mid-flight — produced
            // NO response and NO error: a silent, undiagnosable dropped turn.
            // Emit the same terminal event the rest of the function already
            // uses so a stale turn is always visibly resolved, never silent.
            if !is_turn_active() {
                log_pipeline_step(
                    session_id,
                    "stale_turn_dropped_at_satisfaction",
                    "Turn became stale before satisfaction summary could be emitted",
                    Some(serde_json::json!({ "turn_id": turn_id_for_checks })),
                );
                let _ = event_tx.send(StreamEvent::Done("Turn cancelled.".into()));
                return;
            }

            // Build a summary from completed actions instead of generic message
            let summary = format_tool_satisfaction_summary(&turn_memory);
            let _ = event_tx.send(StreamEvent::Done(summary));
            return;
        }

        log_pipeline_step(
            session_id,
            "max_rounds_reached",
            "Agent loop reached max tool rounds",
            Some(serde_json::json!({
                "max_tool_rounds": self.max_tool_rounds,
            })),
        );

        // BUG #7 FIX: same silent-drop gap as above, at the max-rounds exit path.
        if !is_turn_active() {
            log_pipeline_step(
                session_id,
                "stale_turn_dropped_at_max_rounds",
                "Turn became stale before max-rounds error could be emitted",
                Some(serde_json::json!({ "turn_id": turn_id_for_checks })),
            );
            let _ = event_tx.send(StreamEvent::Done("Turn cancelled.".into()));
            return;
        }

        let _ = event_tx.send(StreamEvent::Error(format!(
            "max tool rounds ({}) reached",
            self.max_tool_rounds
        )));
    }

    /// Check if a tool result contains a file path that should be auto-routed
    /// to a precognitive processor for enrichment.
    async fn auto_route_file_result(
        &self,
        tool_name: &str,
        result: &serde_json::Value,
    ) -> Option<String> {
        // Only auto-route results from file-related tools, not from precognitive tools themselves
        if tool_name.starts_with("image_")
            || tool_name.starts_with("document_")
            || tool_name.starts_with("code_")
            || tool_name.starts_with("audio_")
            || tool_name.starts_with("web_")
            || tool_name.starts_with("embeddings_")
        {
            return None;
        }

        // Look for a file path in the result
        let path = result
            .get("path")
            .or_else(|| result.get("file_path"))
            .or_else(|| result.get("output_path"))
            .and_then(|v| v.as_str())?;

        // Determine the target precognitive tool based on extension
        let ext = path.rsplit('.').next()?.to_lowercase();
        let target_tool = match ext.as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tiff" | "svg" => "image_analyze",
            "pdf" | "docx" | "doc" | "csv" | "tsv" | "xlsx" => "document_extract",
            "py" | "rs" | "js" | "ts" | "jsx" | "tsx" | "go" | "java" | "c" | "cpp" | "h"
            | "rb" | "cs" => "code_analyze_ast",
            "wav" | "mp3" | "ogg" | "flac" | "m4a" => "audio_preprocess",
            _ => return None,
        };

        // Execute the precognitive tool
        if let Some(handler) = self.tool_registry.get_handler(target_tool) {
            let params = serde_json::json!({"file_path": path});
            let handler = handler.clone();
            let tool_context = self
                .tool_registry
                .make_tool_context(CancellationToken::new());
            match tokio::time::timeout(
                std::time::Duration::from_secs(30),
                handler.execute_with_context(params, tool_context),
            )
            .await
            {
                Ok(result) if result.success => {
                    // Return summary only to save tokens
                    result
                        .data
                        .get("summary")
                        .and_then(|s| s.as_str())
                        .map(|summary| format!("[{}] {}", target_tool, summary))
                }
                _ => None,
            }
        } else {
            None
        }
    }
}

/// Detect if the current user prompt matches an interrupted workflow that can be continued.
///
/// Returns `Some((context, detail, options))` if a resumable session is found.
/// The options are rendered as clickable buttons in the UI via `StreamEvent::RecoveryOptions`.
fn detect_session_continuation_options(
    user_text: &str,
) -> Option<(String, String, Vec<RecoveryOption>)> {
    use crate::agent::workflow_session::SessionManager;

    let manager = SessionManager::new();
    let continuable = manager.find_continuable();

    if continuable.is_empty() {
        return None;
    }

    let user_lower = user_text.to_ascii_lowercase();
    let trimmed_lower = user_lower.trim();
    if trimmed_lower.starts_with("start over:")
        || trimmed_lower.starts_with("start fresh:")
        || trimmed_lower.starts_with("continue previous workflow:")
        || trimmed_lower == "start fresh"
        || trimmed_lower == "dismiss"
    {
        return None;
    }

    // Keywords that explicitly signal continuation intent
    let continuation_keywords = [
        "continue previous",
        "continue the workflow",
        "continue the task",
        "resume previous",
        "resume the workflow",
        "resume where",
        "where did we leave",
        "where did i leave",
        "where did you leave",
        "what happened earlier",
        "last workflow",
        "previous workflow",
        "previous task",
    ];
    let wants_continuation = continuation_keywords
        .iter()
        .any(|kw| user_lower.contains(kw));

    // Only proceed if the user EXPLICITLY signals they want to continue.
    // Previously we also matched on "any 5+ char word overlap with previous intent"
    // which produced false positives — common words like "create", "open", "file"
    // appear in many prompts and would falsely flag unrelated turns as continuations.
    if !wants_continuation {
        return None;
    }

    // Find the most recent continuable session with a real user intent
    let most_recent = continuable.into_iter().find(|s| {
        let intent = s.user_intent.to_ascii_lowercase();
        !intent.starts_with("substrate-") && !intent.starts_with("rule-") && intent.len() > 20
    })?;

    let steps_done = most_recent.completed_steps.len();
    let error_summary = most_recent
        .error
        .as_deref()
        .map(|e| &e[..e.len().min(80)])
        .unwrap_or("unknown error");

    let context = format!(
        "Interrupted workflow found: \"{}\"",
        &most_recent.user_intent[..most_recent.user_intent.len().min(60)]
    );
    let detail = format!(
        "Completed {} step(s) before stopping. Last error: {}",
        steps_done, error_summary
    );

    let options = vec![
        RecoveryOption {
            label: "Continue from where it stopped".into(),
            action_prompt: format!("Continue previous workflow: {}", most_recent.user_intent),
            style: "primary",
        },
        RecoveryOption {
            label: "Start fresh".into(),
            action_prompt: format!("Start over: {}", most_recent.user_intent),
            style: "secondary",
        },
        RecoveryOption {
            label: "Dismiss".into(),
            action_prompt: String::new(),
            style: "ghost",
        },
    ];

    Some((context, detail, options))
}

#[cfg(test)]
mod tests;
