//! Prompt-driven settings command (settings-nl-control Task 11 — command parity).
//!
//! `config_prompt` is now a THIN caller of the SAME shared pipeline + handler that
//! chat uses (`SettingsIntentPipeline` + `SettingsHandler`), so a prompt behaves
//! identically whether typed in normal chat or the settings box (Req 1.1/1.4,
//! Property P1). There is no duplicate classifier / undo / keyword logic here
//! anymore (fixes RC4/NEW-6/NEW-13).
//!
//! Approval-gated (YELLOW/RED/BLACK) changes are driven through the existing
//! `HitlGateway` via a command-side [`ApprovalDriver`] that emits
//! `agent:approval_required` (answered by `approve_action`/`deny_action`) —
//! mirroring the loop's `ChatSettingsApprovalDriver`.

use super::*;
use kria_core::config::nl::{
    ApprovalDecision, ApprovalDriver, ConversationContext, SchemaEntityIndex, SettingsDecision,
    SettingsHandler, SettingsIntentPipeline, SettingsOutcome, SettingsRequest, SettingsRequestKind,
};
use kria_core::config::prompt::Scope;
use kria_core::safety::hitl::{ApprovalResponse, HitlGateway};
use kria_core::safety::RiskLevel;
use kria_core::tools::TriggerProvenance;
use std::sync::{Arc, OnceLock};

/// Cached, schema-derived entity index (built once — Req 12.1). Mirrors the loop's
/// `SETTINGS_ENTITY_INDEX`; both build from the same `FieldMeta` registry.
fn command_entity_index() -> Arc<SchemaEntityIndex> {
    static INDEX: OnceLock<Arc<SchemaEntityIndex>> = OnceLock::new();
    INDEX
        .get_or_init(|| Arc::new(SchemaEntityIndex::build()))
        .clone()
}

/// Conversational provider-configuration sessions for the command surface (Wave 4).
fn command_flow_store() -> &'static kria_core::config::nl::FlowStore {
    static STORE: OnceLock<kria_core::config::nl::FlowStore> = OnceLock::new();
    STORE.get_or_init(kria_core::config::nl::FlowStore::new)
}

/// Command-surface approval driver: emits `agent:approval_required` (the same event
/// the agent stream uses) and blocks on the shared `HitlGateway` — the SAME gate
/// every RED tool + chat settings change uses (Req 4.3/4.4).
struct CommandSettingsApprovalDriver {
    app: AppHandle,
    hitl: Arc<HitlGateway>,
}

pub(crate) async fn request_settings_approval(
    app: &AppHandle,
    hitl: &HitlGateway,
    section: &str,
    field: &str,
    value: &serde_json::Value,
    risk: RiskLevel,
) -> ApprovalResponse {
    let request_id = HitlGateway::generate_request_id();
    let description = format!("Change {section}.{field} to {value}");
    let args = serde_json::json!({ "section": section, "field": field, "value": value });
    let envelope = super::approval::ApprovalEnvelope::tool_hitl(
        request_id.clone(),
        "config_patch",
        risk.as_str(),
        args.clone(),
        &description,
        super::approval::now_ms(),
    );
    let rx = hitl
        .prepare_approval_with_id(&request_id, "config_patch", args, risk, &description, false)
        .await;
    super::approval::emit_approval_request(app, &envelope);
    hitl.await_prepared_approval(&request_id, rx).await
}

#[async_trait::async_trait]
impl ApprovalDriver for CommandSettingsApprovalDriver {
    async fn request(
        &self,
        section: &str,
        field: &str,
        value: &serde_json::Value,
        risk: RiskLevel,
    ) -> ApprovalDecision {
        match request_settings_approval(&self.app, &self.hitl, section, field, value, risk).await {
            ApprovalResponse::Approved => ApprovalDecision::Approved,
            ApprovalResponse::Denied => ApprovalDecision::Denied,
            ApprovalResponse::Timeout => ApprovalDecision::Timeout,
        }
    }
}

/// Map a `SettingsOutcome` to the JSON shape the frontend already expects.
fn outcome_to_json(outcome: SettingsOutcome) -> serde_json::Value {
    match outcome {
        SettingsOutcome::Applied {
            section,
            field,
            version,
            ..
        } => serde_json::json!({
            "status": "applied", "section": section, "field": field, "version": version,
        }),
        SettingsOutcome::Answer { text } => serde_json::json!({
            "status": "answer", "message": text,
        }),
        SettingsOutcome::Clarify { question } => serde_json::json!({
            "status": "clarify", "question": question,
        }),
        SettingsOutcome::Refused { reason } => serde_json::json!({
            "status": "refused", "reason": reason,
        }),
        SettingsOutcome::TempApplied {
            section,
            field,
            value,
        } => serde_json::json!({
            "status": "temp_requested", "section": section, "field": field, "value": value,
        }),
        SettingsOutcome::Undone { section, field } => serde_json::json!({
            "status": "undone", "section": section, "field": field,
        }),
        SettingsOutcome::NothingToUndo => serde_json::json!({ "status": "nothing_to_undo" }),
        // resolve() already drove approval; a bare NeedsApproval is unexpected.
        SettingsOutcome::NeedsApproval { section, field, .. } => serde_json::json!({
            "status": "needs_approval", "section": section, "field": field,
        }),
    }
}

#[tauri::command]
pub async fn config_prompt(
    prompt: String,
    app: AppHandle,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    if !kria_core::config::nl::nl_settings_enabled() {
        // NL settings control is ON by default; this only triggers if the operator
        // explicitly opted out via KRIA_NL_SETTINGS=0.
        return Ok(serde_json::json!({
            "status": "disabled",
            "message": "Prompt-driven settings are disabled (KRIA_NL_SETTINGS=0)."
        }));
    }

    // ── Wave 4: conversational provider configuration (multi-turn slot-filling) ─
    {
        use kria_core::config::nl::{FlowEngine, FlowOutcome};
        if command_flow_store().active("command").is_some() || FlowEngine::detects_start(&prompt) {
            match FlowEngine::step(command_flow_store(), "command", &prompt) {
                FlowOutcome::NotAFlow => {}
                FlowOutcome::Ask { message }
                | FlowOutcome::Confirm { summary: message }
                | FlowOutcome::Invalid { message }
                | FlowOutcome::Cancelled { message } => {
                    return Ok(serde_json::json!({ "status": "answer", "message": message }));
                }
                FlowOutcome::Commit { draft, .. } => {
                    let handler = SettingsHandler::new(state.config_service.clone())
                        .with_audit(state.audit_logger.clone());
                    let outcome = handler.commit_provider(&draft).await;
                    return Ok(outcome_to_json(outcome));
                }
            }
        }
    }

    // Cross-session referential recall ("same as yesterday", "what did I set last
    // week?") needs the memory subsystem (out of scope). Answer honestly and return
    // the audit-backed change history as the concrete alternative. This is a command-
    // surface affordance the chat gate doesn't need (no history viewer in chat).
    let low = prompt.trim().to_ascii_lowercase();
    let cross_session = [
        "yesterday",
        "last week",
        "last time",
        "earlier today",
        "the other day",
        "previously",
        "before i",
        "day before",
    ]
    .iter()
    .any(|k| low.contains(k));
    if cross_session {
        let history = state.audit_logger.config_change_history(20);
        return Ok(serde_json::json!({
            "status": "cross_session_recall_unavailable",
            "message": "Recalling a setting from a previous session by memory isn't available yet (that needs the memory upgrade). Here is the recorded config-change history instead — you can re-apply any of these.",
            "history": history,
        }));
    }

    // ── The ONE shared decider: classify with the same pipeline chat uses ──
    // The command surface has no chat history, so conversation context is empty.
    let conv = ConversationContext::default();
    let pipeline = SettingsIntentPipeline::new(command_entity_index());
    let (decision, trace) = pipeline.classify_traced(&prompt, &conv);
    kria_core::config::nl::diagnostics::record("command", &prompt, &trace);

    let handler =
        SettingsHandler::new(state.config_service.clone()).with_audit(state.audit_logger.clone());

    let outcome = match decision {
        SettingsDecision::NotSettings => {
            return Ok(serde_json::json!({ "status": "not_a_change" }))
        }
        SettingsDecision::Clarify { question } => {
            return Ok(serde_json::json!({ "status": "clarify", "question": question }))
        }
        SettingsDecision::Undo => {
            handler
                .handle(SettingsRequest {
                    kind: SettingsRequestKind::Undo,
                    section: String::new(),
                    field: String::new(),
                    value: None,
                    scope: Scope::Permanent,
                    provenance: TriggerProvenance::User,
                    session_id: String::new(),
                })
                .await
        }
        SettingsDecision::ReadBack { section, field } => {
            handler
                .handle(SettingsRequest::read_back(section, field))
                .await
        }
        SettingsDecision::Info(query) => handler.info(&query).await,
        SettingsDecision::Change {
            section,
            field,
            value,
            scope,
        } => {
            // A standalone command has no agent turn to attach a turn-scoped override
            // to, so a temporary override is reported for the caller/agent to apply
            // within a turn (parity with the legacy `temp_requested` shape).
            if scope == Scope::Temp {
                return Ok(serde_json::json!({
                    "status": "temp_requested",
                    "section": section,
                    "field": field,
                    "value": value,
                }));
            }
            let driver = CommandSettingsApprovalDriver {
                app: app.clone(),
                hitl: state.hitl.clone(),
            };
            handler
                .resolve(
                    SettingsRequest {
                        kind: SettingsRequestKind::Change,
                        section,
                        field,
                        value,
                        scope: Scope::Permanent,
                        provenance: TriggerProvenance::User,
                        session_id: String::new(),
                    },
                    &driver,
                )
                .await
        }
    };

    Ok(outcome_to_json(outcome))
}
