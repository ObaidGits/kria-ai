//! n8n workflow substrate integration.
//!
//! This module intentionally keeps KRIA as the authority plane. n8n workflows
//! are versioned, allowlisted external execution targets invoked through the
//! normal `ToolRegistry` path.

pub mod callback;
pub mod catalog;
pub mod client;
pub mod config;
pub mod governance;
pub mod state;
pub mod tool;
pub mod types;

pub use callback::{parse_and_verify_callback, verify_callback_signature, N8nCallbackError};
pub use catalog::{N8nCatalog, N8nCatalogError};
pub use client::{sign_payload, N8nClient, N8nClientError};
pub use config::N8nConfig;
pub use governance::{
    evaluate_run, N8nContinuationAction, N8nGovernanceDecision, N8nVerificationStatus,
};
pub use state::{
    N8nDeadLetter, N8nInboxRecord, N8nIngestDecision, N8nWorkflowRunState, N8nWorkflowStateStore,
};
pub use types::{
    N8nCallbackEnvelope, N8nCallbackErrorClass, N8nCommandEnvelope, N8nInvocationResult,
    N8nIrreversibilityClass, N8nRunStatus, N8nTimeoutClass, N8nToolRequest, N8nWorkflowConfig,
    N8nWorkflowEnvironment, N8nWorkflowStatus, N8N_CALLBACK_SCHEMA_VERSION,
    N8N_COMMAND_SCHEMA_VERSION,
};

use crate::tools::ToolRegistry;
use std::sync::Arc;

pub fn register_into_tool_registry(
    registry: &ToolRegistry,
    config: N8nConfig,
) -> Result<Option<Arc<N8nClient>>, N8nClientError> {
    if !config.enabled {
        return Ok(None);
    }

    let catalog = Arc::new(N8nCatalog::new(config)?);
    let client = Arc::new(N8nClient::new(catalog)?);
    tool::register(registry, client.clone());
    Ok(Some(client))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::RiskLevel;
    use std::sync::Arc;
    use wiremock::matchers::{header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn workflow(status: N8nWorkflowStatus) -> N8nWorkflowConfig {
        N8nWorkflowConfig {
            workflow_id: "jira_fetch".into(),
            workflow_version: "v1".into(),
            display_name: "Fetch Jira".into(),
            endpoint_path: "/webhook/jira-fetch".into(),
            status,
            risk_tier: RiskLevel::Green,
            ..Default::default()
        }
    }

    fn callback_event(
        event_id: &str,
        sequence_number: u64,
        status: N8nRunStatus,
    ) -> N8nCallbackEnvelope {
        N8nCallbackEnvelope {
            schema_version: N8N_CALLBACK_SCHEMA_VERSION.into(),
            correlation_id: "turn_1".into(),
            causation_id: "turn_1".into(),
            event_id: event_id.into(),
            sequence_number,
            workflow_id: "jira_fetch".into(),
            workflow_version: "v1".into(),
            n8n_run_id: "run_123".into(),
            status,
            evidence: serde_json::json!({"summary": "ok", "sequence": sequence_number}),
            side_effects: vec!["jira_read".into()],
            error_class: None,
            occurred_at_ms: 1,
        }
    }

    fn catalog_with_workflow() -> N8nCatalog {
        let config = N8nConfig {
            enabled: true,
            base_url: "http://127.0.0.1:5678".into(),
            signing_secret: "secret".into(),
            workflows: vec![workflow(N8nWorkflowStatus::Approved)],
            ..Default::default()
        };
        N8nCatalog::new(config).unwrap()
    }

    #[test]
    fn catalog_rejects_disabled_integration() {
        let err = N8nCatalog::new(N8nConfig::default()).unwrap_err();
        assert!(matches!(err, N8nCatalogError::Disabled));
    }

    #[test]
    fn catalog_requires_approved_workflow() {
        let config = N8nConfig {
            enabled: true,
            base_url: "http://127.0.0.1:5678".into(),
            signing_secret: "secret".into(),
            workflows: vec![workflow(N8nWorkflowStatus::Draft)],
            ..Default::default()
        };
        let catalog = N8nCatalog::new(config).unwrap();
        let err = catalog.resolve("jira_fetch", Some("v1")).unwrap_err();
        assert!(matches!(err, N8nCatalogError::WorkflowNotApproved { .. }));
    }

    #[test]
    fn catalog_rejects_version_mismatch() {
        let config = N8nConfig {
            enabled: true,
            base_url: "http://127.0.0.1:5678".into(),
            signing_secret: "secret".into(),
            workflows: vec![workflow(N8nWorkflowStatus::Approved)],
            ..Default::default()
        };
        let catalog = N8nCatalog::new(config).unwrap();
        let err = catalog.resolve("jira_fetch", Some("v2")).unwrap_err();
        assert!(matches!(
            err,
            N8nCatalogError::WorkflowVersionMismatch { .. }
        ));
    }

    #[test]
    fn signing_is_stable_for_same_payload() {
        let a = sign_payload(b"secret", br#"{"a":1}"#).unwrap();
        let b = sign_payload(b"secret", br#"{"a":1}"#).unwrap();
        assert_eq!(a, b);
        assert!(a.starts_with("sha256="));
    }

    #[test]
    fn callback_parser_accepts_signed_current_workflow_event() {
        let catalog = catalog_with_workflow();
        let body =
            serde_json::to_vec(&callback_event("event_1", 1, N8nRunStatus::Running)).unwrap();
        let signature = sign_payload(b"secret", &body).unwrap();

        let parsed = parse_and_verify_callback(&catalog, &body, &signature).unwrap();

        assert_eq!(parsed.workflow_id, "jira_fetch");
        assert_eq!(parsed.workflow_version, "v1");
        assert_eq!(parsed.correlation_id, "turn_1");
        assert_eq!(parsed.status, N8nRunStatus::Running);
    }

    #[test]
    fn callback_parser_rejects_invalid_signature() {
        let catalog = catalog_with_workflow();
        let body =
            serde_json::to_vec(&callback_event("event_1", 1, N8nRunStatus::Running)).unwrap();
        let err = parse_and_verify_callback(&catalog, &body, "sha256=bad")
            .expect_err("invalid signatures must fail closed");

        assert!(matches!(err, N8nCallbackError::InvalidSignature));
    }

    #[test]
    fn callback_parser_rejects_version_mismatch() {
        let catalog = catalog_with_workflow();
        let mut event = callback_event("event_1", 1, N8nRunStatus::Running);
        event.workflow_version = "v2".into();
        let body = serde_json::to_vec(&event).unwrap();
        let signature = sign_payload(b"secret", &body).unwrap();
        let err = parse_and_verify_callback(&catalog, &body, &signature).unwrap_err();

        assert!(matches!(
            err,
            N8nCallbackError::Catalog(N8nCatalogError::WorkflowVersionMismatch { .. })
        ));
    }

    #[test]
    fn workflow_state_store_rejects_duplicate_and_out_of_order_events() {
        let store = N8nWorkflowStateStore::default();

        assert_eq!(
            store.ingest(callback_event("event_1", 1, N8nRunStatus::Running)),
            N8nIngestDecision::Accepted
        );
        assert_eq!(
            store.ingest(callback_event("event_1", 1, N8nRunStatus::Running)),
            N8nIngestDecision::Duplicate
        );
        assert_eq!(
            store.ingest(callback_event("event_0", 0, N8nRunStatus::Running)),
            N8nIngestDecision::OutOfOrder
        );

        let run = store.get("turn_1").unwrap();
        assert_eq!(run.last_sequence_number, 1);
        assert_eq!(run.status, N8nRunStatus::Running);
        assert_eq!(store.dead_letters().len(), 2);
    }

    #[test]
    fn workflow_state_store_preserves_terminal_state() {
        let store = N8nWorkflowStateStore::default();

        assert_eq!(
            store.ingest(callback_event("event_1", 1, N8nRunStatus::Running)),
            N8nIngestDecision::Accepted
        );
        assert_eq!(
            store.ingest(callback_event("event_2", 2, N8nRunStatus::Completed)),
            N8nIngestDecision::Accepted
        );
        assert_eq!(
            store.ingest(callback_event("event_3", 3, N8nRunStatus::Running)),
            N8nIngestDecision::TerminalAlreadyReached
        );

        let run = store.get("turn_1").unwrap();
        assert_eq!(run.status, N8nRunStatus::Completed);
        assert!(run.terminal);
        assert_eq!(store.dead_letters().len(), 1);
    }

    #[test]
    fn governance_requires_expected_evidence_before_continuation() {
        let store = N8nWorkflowStateStore::default();
        let mut workflow = workflow(N8nWorkflowStatus::Approved);
        workflow.expected_evidence = vec!["ticket_summary".into(), "created_tasks".into()];

        assert_eq!(
            store.ingest(callback_event("event_1", 1, N8nRunStatus::Completed)),
            N8nIngestDecision::Accepted
        );
        let run = store.get("turn_1").unwrap();
        let decision = evaluate_run(Some(&workflow), &run);

        assert_eq!(
            decision.verification_status,
            N8nVerificationStatus::NeedsMoreEvidence
        );
        assert_eq!(
            decision.continuation_action,
            N8nContinuationAction::PauseForHitl
        );
        assert_eq!(decision.missing_evidence.len(), 2);
    }

    #[test]
    fn governance_allows_continuation_when_evidence_contract_is_satisfied() {
        let store = N8nWorkflowStateStore::default();
        let mut workflow = workflow(N8nWorkflowStatus::Approved);
        workflow.expected_evidence = vec!["summary".into()];

        assert_eq!(
            store.ingest(callback_event("event_1", 1, N8nRunStatus::Completed)),
            N8nIngestDecision::Accepted
        );
        let run = store.get("turn_1").unwrap();
        let decision = evaluate_run(Some(&workflow), &run);

        assert_eq!(
            decision.verification_status,
            N8nVerificationStatus::Verified
        );
        assert_eq!(
            decision.continuation_action,
            N8nContinuationAction::ContinueWorkflow
        );
    }

    #[tokio::test]
    async fn client_invokes_approved_workflow_with_signed_envelope() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/webhook/jira-fetch"))
            .and(header_exists("x-kria-signature"))
            .and(header_exists("x-kria-correlation-id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "accepted": true,
                "run_id": "run_123"
            })))
            .mount(&server)
            .await;

        let config = N8nConfig {
            enabled: true,
            base_url: server.uri(),
            signing_secret: "secret".into(),
            workflows: vec![workflow(N8nWorkflowStatus::Approved)],
            ..Default::default()
        };
        let catalog = Arc::new(N8nCatalog::new(config).unwrap());
        let client = N8nClient::new(catalog).unwrap();

        let result = client
            .invoke(N8nToolRequest {
                workflow_id: "jira_fetch".into(),
                workflow_version: Some("v1".into()),
                input_payload: serde_json::json!({"project": "KRIA"}),
                correlation_id: Some("turn_1".into()),
                idempotency_key: Some("idem_1".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(result.accepted);
        assert_eq!(result.workflow_id, "jira_fetch");
        assert_eq!(result.workflow_version, "v1");
        assert_eq!(result.correlation_id, "turn_1");
        assert_eq!(result.idempotency_key, "idem_1");
        assert_eq!(result.response["run_id"], "run_123");
    }

    #[tokio::test]
    async fn client_rejects_payloads_over_configured_limit() {
        let config = N8nConfig {
            enabled: true,
            base_url: "http://127.0.0.1:5678".into(),
            signing_secret: "secret".into(),
            max_payload_bytes: 32,
            workflows: vec![workflow(N8nWorkflowStatus::Approved)],
            ..Default::default()
        };
        let catalog = Arc::new(N8nCatalog::new(config).unwrap());
        let client = N8nClient::new(catalog).unwrap();

        let err = client
            .invoke(N8nToolRequest {
                workflow_id: "jira_fetch".into(),
                input_payload: serde_json::json!({"large": "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"}),
                ..Default::default()
            })
            .await
            .unwrap_err();

        assert!(matches!(err, N8nClientError::PayloadTooLarge { .. }));
    }
}
