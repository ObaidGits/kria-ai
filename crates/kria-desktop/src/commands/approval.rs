//! Unified Approval Event — the single approval-event shape the Approval Center
//! consumes (kria-ui-redesign task 4.2, Req 11.1 / design.md §3.3 contract change a).
//!
//! Every human-in-the-loop moment — tool HITL, interaction decisions,
//! gui-cognition approval, and workflow resume — is expressed as one
//! [`ApprovalEnvelope`] emitted on the [`APPROVAL_REQUEST_EVENT`] channel. The
//! frontend bridge maps this single shape into the `approvalStore` queue and
//! routes the human's decision back through the runtime's own resolution
//! commands (the UI never executes the approved action itself — KRIA stays the
//! orchestration authority).
//!
//! The legacy per-source events (`{session}:approval_required`,
//! `gui_cognition:event`, `workflow:telemetry`, interaction-decision polling)
//! remain in place; this envelope is an additive, canonical superset so the
//! Approval Center has one contract to consume.
//!
//! The envelope constructors are pure and unit-tested; emission is a thin,
//! best-effort wrapper (never panics, never blocks the agent loop).

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

/// Canonical channel the Approval Center subscribes to for new approvals.
pub const APPROVAL_REQUEST_EVENT: &str = "approval://request";

/// Where an approval originated. Serializes to the kebab-case discriminants the
/// frontend `ApprovalType` union uses, so the shapes line up exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalSource {
    ToolHitl,
    InteractionDecision,
    GuiCognition,
    WorkflowResume,
}

/// Risk ramp level (Req 11.2). Serializes to the frontend `RiskLevel` union.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalRisk {
    Green,
    Yellow,
    Red,
    Black,
}

impl ApprovalRisk {
    /// Map a free-form backend risk string onto the risk ramp. Unknown values
    /// fall back to `Yellow` (the conservative "needs a look" tier) rather than
    /// silently treating an unclassified action as safe.
    pub fn from_backend_str(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "green" | "low" | "safe" | "none" => ApprovalRisk::Green,
            "yellow" | "medium" | "moderate" | "caution" => ApprovalRisk::Yellow,
            "red" | "high" | "danger" | "dangerous" => ApprovalRisk::Red,
            "black" | "critical" | "irreversible" | "destructive" => ApprovalRisk::Black,
            _ => ApprovalRisk::Yellow,
        }
    }

    /// True when the risk tier alone requires an explicit confirm on approve
    /// (Req 11.3). Mirrors the frontend `requiresExplicitConfirm` risk clause.
    /// Part of the canonical contract surface (used by tests + the frontend
    /// parity check); not every consumer is wired yet.
    #[allow(dead_code)]
    pub fn requires_explicit_confirm(self) -> bool {
        matches!(self, ApprovalRisk::Red | ApprovalRisk::Black)
    }
}

/// Routing keys the frontend resolver uses to send the human decision back to
/// the correct backend command per source (Req 11.6). Only the fields relevant
/// to a given source are populated; the rest stay `None` and are omitted from
/// the wire payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRouting {
    /// HITL request id — used by `approve_action` / `deny_action`
    /// (tool-hitl and gui-cognition proposals resolve by request id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Interaction-decision id — used by `resolve_interaction_decision`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_id: Option<String>,
    /// Workflow id — used by `workflow_hitl_respond` / `workflow_cancel`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    /// Option id to submit when the human APPROVES (interaction/workflow HITL
    /// choices carry explicit option ids rather than a bare approve verb).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approve_option_id: Option<String>,
    /// Option id to submit when the human DENIES.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deny_option_id: Option<String>,
}

/// The one approval shape the Approval Center consumes (Req 11.1). Field names
/// serialize to camelCase to match the frontend `ApprovalRequest` interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalEnvelope {
    /// Stable id for this approval (dedupe + resolution key on the frontend).
    pub id: String,
    /// Which HITL surface raised this.
    pub source: ApprovalSource,
    /// What will happen — the headline (Req 11.2).
    pub title: String,
    /// Why it is being requested — plain-language rationale (Req 11.2).
    pub description: String,
    /// Risk ramp tier (Req 11.2).
    pub risk: ApprovalRisk,
    /// Concrete effects the action will have (Req 11.2).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub effects: Vec<String>,
    /// Evidence KRIA used or produced (Req 11.2). Untrusted — the frontend
    /// sanitizes before display.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<serde_json::Value>,
    /// Whether the action is irreversible — forces an explicit confirm (Req 11.3).
    pub irreversible: bool,
    /// Grant scope options offered on approve (Req 7.3). Empty ⇒ single "once".
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub scope_options: Vec<String>,
    /// Per-source routing keys the resolver needs (Req 11.6).
    pub routing: ApprovalRouting,
    /// Original source payload, preserved verbatim for the details disclosure.
    pub payload: serde_json::Value,
    /// Creation time (epoch millis) for ordering / expiry.
    pub created_at_ms: u64,
}

impl ApprovalEnvelope {
    /// Build a tool-HITL approval from the same data the legacy
    /// `{session}:approval_required` event carries. Resolves via
    /// `approve_action` / `deny_action` keyed by `request_id`.
    pub fn tool_hitl(
        request_id: impl Into<String>,
        tool_name: impl Into<String>,
        risk_level: &str,
        args: serde_json::Value,
        reason: &str,
        created_at_ms: u64,
    ) -> Self {
        let request_id = request_id.into();
        let tool_name = tool_name.into();
        let risk = ApprovalRisk::from_backend_str(risk_level);
        let description = if reason.trim().is_empty() {
            format!("KRIA wants to run {tool_name}.")
        } else {
            reason.to_string()
        };
        Self {
            id: request_id.clone(),
            source: ApprovalSource::ToolHitl,
            title: format!("Run {tool_name}"),
            description,
            risk,
            effects: Vec::new(),
            evidence: None,
            irreversible: risk == ApprovalRisk::Black,
            scope_options: vec!["once".to_string(), "session".to_string()],
            routing: ApprovalRouting {
                request_id: Some(request_id),
                ..Default::default()
            },
            payload: args,
            created_at_ms,
        }
    }

    /// Build a gui-cognition approval. Resolves via `approve_action` /
    /// `deny_action` keyed by `request_id` (the gui-cognition HITL proposal id).
    ///
    /// Canonical contract surface for the gui-cognition source (design.md §3.3);
    /// the gui-cognition emit seam adopts it as that path migrates. Exercised by
    /// the envelope unit tests + the frontend parity contract.
    #[allow(dead_code)]
    pub fn gui_cognition(
        request_id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
        risk_level: &str,
        payload: serde_json::Value,
        created_at_ms: u64,
    ) -> Self {
        let request_id = request_id.into();
        let risk = ApprovalRisk::from_backend_str(risk_level);
        Self {
            id: request_id.clone(),
            source: ApprovalSource::GuiCognition,
            title: title.into(),
            description: description.into(),
            risk,
            effects: Vec::new(),
            evidence: None,
            irreversible: risk == ApprovalRisk::Black,
            scope_options: vec!["once".to_string()],
            routing: ApprovalRouting {
                request_id: Some(request_id),
                ..Default::default()
            },
            payload,
            created_at_ms,
        }
    }

    /// Build an interaction-decision approval. Resolves via
    /// `resolve_interaction_decision` keyed by `decision_id` + chosen option id.
    ///
    /// Canonical contract surface for the interaction-decision source; the
    /// decision emit seam adopts it as that path migrates. Exercised by tests.
    #[allow(dead_code)]
    pub fn interaction_decision(
        decision_id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
        risk_level: &str,
        approve_option_id: Option<String>,
        deny_option_id: Option<String>,
        payload: serde_json::Value,
        created_at_ms: u64,
    ) -> Self {
        let decision_id = decision_id.into();
        let risk = ApprovalRisk::from_backend_str(risk_level);
        Self {
            id: decision_id.clone(),
            source: ApprovalSource::InteractionDecision,
            title: title.into(),
            description: description.into(),
            risk,
            effects: Vec::new(),
            evidence: None,
            irreversible: risk == ApprovalRisk::Black,
            scope_options: vec!["once".to_string()],
            routing: ApprovalRouting {
                decision_id: Some(decision_id),
                approve_option_id,
                deny_option_id,
                ..Default::default()
            },
            payload,
            created_at_ms,
        }
    }

    /// Build a workflow-resume approval. Resolves via `workflow_hitl_respond`
    /// (approve/deny option) or `workflow_cancel`, keyed by `workflow_id`.
    ///
    /// Canonical contract surface for the workflow-resume source; the workflow
    /// HITL emit seam adopts it as that path migrates. Exercised by tests.
    #[allow(dead_code)]
    pub fn workflow_resume(
        workflow_id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
        risk_level: &str,
        approve_option_id: Option<String>,
        deny_option_id: Option<String>,
        payload: serde_json::Value,
        created_at_ms: u64,
    ) -> Self {
        let workflow_id = workflow_id.into();
        let risk = ApprovalRisk::from_backend_str(risk_level);
        Self {
            id: format!("workflow:{workflow_id}"),
            source: ApprovalSource::WorkflowResume,
            title: title.into(),
            description: description.into(),
            risk,
            effects: Vec::new(),
            evidence: None,
            irreversible: risk == ApprovalRisk::Black,
            scope_options: vec!["once".to_string()],
            routing: ApprovalRouting {
                workflow_id: Some(workflow_id),
                approve_option_id,
                deny_option_id,
                ..Default::default()
            },
            payload,
            created_at_ms,
        }
    }
}

/// Current time in epoch milliseconds. Saturates to 0 on the (impossible) pre-
/// epoch clock rather than panicking — approval emission must never unwrap.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Emit a unified approval request to the frontend. Best-effort: a failed emit
/// is logged and swallowed so a webview hiccup can never stall the agent loop
/// or the safety gate (the runtime still holds the pending HITL state).
pub fn emit_approval_request(app: &AppHandle, envelope: &ApprovalEnvelope) {
    // Cache unresolved canonical envelopes for webviews created after this emit
    // and update tray badge. Presentation state cannot approve or execute.
    crate::windows::register_pending_approval(app, &envelope.id, envelope);
    if let Err(err) = app.emit(APPROVAL_REQUEST_EVENT, envelope) {
        tracing::warn!(
            target: "approval_center",
            error = %err,
            approval_id = %envelope.id,
            "failed to emit unified approval request"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_maps_known_and_unknown_strings() {
        assert_eq!(ApprovalRisk::from_backend_str("Low"), ApprovalRisk::Green);
        assert_eq!(
            ApprovalRisk::from_backend_str("MEDIUM"),
            ApprovalRisk::Yellow
        );
        assert_eq!(ApprovalRisk::from_backend_str("high"), ApprovalRisk::Red);
        assert_eq!(
            ApprovalRisk::from_backend_str("destructive"),
            ApprovalRisk::Black
        );
        // Unknown → conservative Yellow, never silently Green.
        assert_eq!(ApprovalRisk::from_backend_str("wat"), ApprovalRisk::Yellow);
    }

    #[test]
    fn explicit_confirm_gate_matches_frontend_rule() {
        assert!(!ApprovalRisk::Green.requires_explicit_confirm());
        assert!(!ApprovalRisk::Yellow.requires_explicit_confirm());
        assert!(ApprovalRisk::Red.requires_explicit_confirm());
        assert!(ApprovalRisk::Black.requires_explicit_confirm());
    }

    #[test]
    fn tool_hitl_envelope_carries_request_id_routing() {
        let env = ApprovalEnvelope::tool_hitl(
            "req-42",
            "shell.run",
            "high",
            serde_json::json!({ "cmd": "ls" }),
            "Needs to inspect the directory",
            1_000,
        );
        assert_eq!(env.id, "req-42");
        assert_eq!(env.source, ApprovalSource::ToolHitl);
        assert_eq!(env.risk, ApprovalRisk::Red);
        assert_eq!(env.routing.request_id.as_deref(), Some("req-42"));
        assert_eq!(env.description, "Needs to inspect the directory");
        assert!(!env.irreversible);
    }

    #[test]
    fn tool_hitl_black_risk_is_irreversible_and_has_default_description() {
        let env = ApprovalEnvelope::tool_hitl(
            "req-1",
            "disk.format",
            "destructive",
            serde_json::Value::Null,
            "   ",
            0,
        );
        assert_eq!(env.risk, ApprovalRisk::Black);
        assert!(env.irreversible);
        assert_eq!(env.description, "KRIA wants to run disk.format.");
    }

    #[test]
    fn workflow_resume_envelope_namespaces_id_and_routes_by_workflow() {
        let env = ApprovalEnvelope::workflow_resume(
            "wf-7",
            "Resume backup workflow",
            "A step needs your approval",
            "yellow",
            Some("approve".into()),
            Some("deny".into()),
            serde_json::json!({ "step": 3 }),
            5,
        );
        assert_eq!(env.id, "workflow:wf-7");
        assert_eq!(env.source, ApprovalSource::WorkflowResume);
        assert_eq!(env.routing.workflow_id.as_deref(), Some("wf-7"));
        assert_eq!(env.routing.approve_option_id.as_deref(), Some("approve"));
    }

    #[test]
    fn envelope_serializes_to_camelcase_frontend_shape() {
        let env = ApprovalEnvelope::interaction_decision(
            "dec-1",
            "Pick a target",
            "Two windows match",
            "green",
            Some("opt-a".into()),
            Some("opt-cancel".into()),
            serde_json::json!({}),
            9,
        );
        let value = serde_json::to_value(&env).unwrap();
        assert_eq!(value["id"], "dec-1");
        assert_eq!(value["source"], "interaction-decision");
        assert_eq!(value["risk"], "green");
        assert_eq!(value["createdAtMs"], 9);
        assert_eq!(value["routing"]["decisionId"], "dec-1");
        assert_eq!(value["routing"]["approveOptionId"], "opt-a");
        // Empty collections are omitted so the payload stays lean.
        assert!(value.get("effects").is_none());
        assert!(value.get("evidence").is_none());
    }
}
