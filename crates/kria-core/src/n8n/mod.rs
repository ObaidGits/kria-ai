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
pub mod input_adaptation;
pub mod matching;
pub mod metadata_enrichment;
pub mod readiness;
pub mod runtime_profiles;
pub mod schema;
pub mod state;
pub mod tool;
pub mod types;
pub mod workflow_registry;
pub mod workflow_validation;

pub use callback::{parse_and_verify_callback, verify_callback_signature, N8nCallbackError};
pub use catalog::{N8nCatalog, N8nCatalogError};
pub use client::{sign_payload, N8nClient, N8nClientError};
pub use config::{N8nConfig, N8nManagedDockerConfig, N8nRuntimeMode};
pub use governance::{
    evaluate_run, N8nContinuationAction, N8nGovernanceDecision, N8nVerificationStatus,
};
pub use input_adaptation::{
    analyze_n8n_input_capability, build_n8n_binary_input_aware_copy_plan,
    build_n8n_code_input_aware_copy_plan, build_n8n_input_aware_copy_plan, N8nBinaryInputCopyPlan,
    N8nBinaryInputReport, N8nBinaryInputReview, N8nBranchReport, N8nCodeLiteralHint,
    N8nCodeNodeClassification, N8nCodeNodeReport, N8nCodePatchPlan, N8nCodePatchReview,
    N8nCodePatchedNode, N8nInputAwareChangedParameter, N8nInputAwareCopyPlan,
    N8nInputAwareMappingReview, N8nInputCapability, N8nInputCapabilityReport,
    N8nInputParameterCandidate, N8nInputSurfaceType, N8nOutputNodeCandidate,
    N8nOutputSelectionReport, N8nV5CapabilityStatus, N8N_INPUT_ADAPTATION_SCHEMA_VERSION,
};
pub use matching::{
    build_n8n_suggested_input_payload, mark_n8n_input_payload_confirmed,
    parse_n8n_workflow_run_reference, resolve_n8n_workflow_reference, N8nWorkflowMatchCandidate,
    N8nWorkflowReferenceMatch, WorkflowCandidate, WorkflowConfirmationFlow, WorkflowRankingEngine,
    WorkflowSuggestionResponse,
};
pub use metadata_enrichment::{
    build_n8n_metadata_enrichment_prompt, parse_metadata_suggestion, profile_with_enrichment,
    profile_with_heuristic_metadata_fallback, redacted_workflow_summary,
    safety_merge_metadata_suggestion, N8nMetadataEnrichmentPrompt, N8nRedactionReport,
    N8N_METADATA_ENRICHMENT_SCHEMA_VERSION,
};
pub use readiness::{
    evaluate_stage3_readiness, workflow_has_stage3_ready_metadata, N8nReadinessGateCheck,
    N8nReadinessGateEvidence, N8nStage3ReadinessReport, N8N_STAGE3_REQUIRED_WORKFLOW_COUNT,
};
pub use runtime_profiles::{
    analyze_n8n_runtime_profile, analyze_n8n_runtime_profiles, default_runtime_profile_store_path,
    delete_runtime_profile, load_runtime_profile_store_at, mark_profile_drift, raw_workflow_hash,
    save_runtime_profile_store_at, semantic_workflow_hash, upsert_runtime_profile,
    N8nCredentialStatus, N8nMetadataEnrichmentProvenance, N8nMetadataSuggestion, N8nOutputStrategy,
    N8nResultMode, N8nRuntimeHitlStrategy, N8nRuntimeProfileDraft, N8nRuntimeProfileStatus,
    N8nRuntimeProfileStore, N8nRuntimeProfileStoreError, N8nRuntimeRiskEstimate,
    N8nTriggerStrategy, N8N_RUNTIME_PROFILE_SCHEMA_VERSION,
};
pub use schema::{
    input_payload_validation_issues, schema_error_issues, validate_n8n_input_payload,
    validate_n8n_output_evidence, N8nSchemaValidationError,
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
pub use workflow_registry::{
    default_workflow_registry_store_path, delete_workflow_registry_record,
    load_workflow_registry_store_at, migrate_missing_toml_workflows_to_registry_store,
    migrate_toml_workflows_to_registry_at, migrate_toml_workflows_to_registry_store,
    registry_has_workflow_parity, save_workflow_registry_store_at, upsert_workflow_registry_record,
    workflow_registry_records, workflow_registry_workflows, N8nWorkflowRegistryRecord,
    N8nWorkflowRegistryStore, N8nWorkflowRegistryStoreError,
    N8N_WORKFLOW_REGISTRY_AUTHORING_SOURCE, N8N_WORKFLOW_REGISTRY_MIGRATED_SOURCE,
    N8N_WORKFLOW_REGISTRY_ROLLBACK_SOURCE, N8N_WORKFLOW_REGISTRY_SCHEMA_VERSION,
    N8N_WORKFLOW_REGISTRY_UI_SOURCE,
};
pub use workflow_validation::{
    infer_webhook_endpoint_path, validate_n8n_workflow_json, validate_n8n_workflow_json_str,
    workflow_validation_summary, N8nWorkflowValidationCheck, N8nWorkflowValidationCheckStatus,
    N8nWorkflowValidationOptions, N8nWorkflowValidationReport, N8nWorkflowValidationReportStatus,
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

    // Resolve signing secret from config → env → file (security: never commit to VCS)
    let mut config = config.with_resolved_secret();
    if config.workflows.is_empty() {
        if let Ok(store) = load_workflow_registry_store_at(&default_workflow_registry_store_path())
        {
            config.workflows = workflow_registry_workflows(&store);
        }
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
    use serial_test::serial;
    use std::ffi::OsString;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::tempdir;
    use wiremock::matchers::{header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct EnvVarGuard {
        key: &'static str,
        original: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let original = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, original }
        }

        fn remove(key: &'static str) -> Self {
            let original = std::env::var_os(key);
            std::env::remove_var(key);
            Self { key, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn test_unix_ms() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default()
    }

    fn workflow(status: N8nWorkflowStatus) -> N8nWorkflowConfig {
        N8nWorkflowConfig {
            workflow_id: "jira_fetch".into(),
            workflow_version: "v1".into(),
            display_name: "Fetch Jira".into(),
            endpoint_path: "/webhook/jira-fetch".into(),
            status,
            risk_tier: RiskLevel::Green,
            owner: "kria-test".into(),
            requires_callback: Some(true),
            input_schema_ref: "schemas/n8n/test_workflow.input.json".into(),
            output_schema_ref: "schemas/n8n/test_workflow.output.json".into(),
            credential_requirements: vec!["none".into()],
            hitl_policy: "none".into(),
            category: "diagnostic".into(),
            description: "Safe callback test workflow".into(),
            example_prompts: vec!["Run jira_fetch".into()],
            tags: vec!["diagnostic".into()],
            aliases: vec!["test workflow".into(), "kria test workflow".into()],
            data_scope: vec!["jira_read".into()],
            expected_evidence: vec!["summary".into()],
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
            evidence: serde_json::json!({"result": "ok", "summary": "ok", "sequence": sequence_number}),
            side_effects: vec!["jira_read".into()],
            error_class: None,
            occurred_at_ms: test_unix_ms(),
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

    fn required_review_workflow() -> N8nWorkflowConfig {
        let mut workflow = workflow(N8nWorkflowStatus::Approved);
        workflow.workflow_id = "slack_post_update".into();
        workflow.display_name = "Slack Update Poster".into();
        workflow.endpoint_path = "/webhook/kria-slack-post-update".into();
        workflow.risk_tier = RiskLevel::Yellow;
        workflow.hitl_policy = "required_review".into();
        workflow.input_schema_ref = "schemas/n8n/slack_post_update.input.json".into();
        workflow.output_schema_ref = "schemas/n8n/slack_post_update.output.json".into();
        workflow.expected_evidence = vec![
            "result".into(),
            "message_ref".into(),
            "confirmed_by_user".into(),
        ];
        workflow
    }

    fn slack_callback(event_id: &str, evidence: serde_json::Value) -> N8nCallbackEnvelope {
        N8nCallbackEnvelope {
            schema_version: N8N_CALLBACK_SCHEMA_VERSION.into(),
            correlation_id: "slack_turn_1".into(),
            causation_id: "slack_turn_1".into(),
            event_id: event_id.into(),
            sequence_number: 1,
            workflow_id: "slack_post_update".into(),
            workflow_version: "v1".into(),
            n8n_run_id: "slack_run_123".into(),
            status: N8nRunStatus::Completed,
            evidence,
            side_effects: Vec::new(),
            error_class: None,
            occurred_at_ms: test_unix_ms(),
        }
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
    fn catalog_rejects_disabled_workflow_execution() {
        let config = N8nConfig {
            enabled: true,
            base_url: "http://127.0.0.1:5678".into(),
            signing_secret: "secret".into(),
            workflows: vec![workflow(N8nWorkflowStatus::Disabled)],
            ..Default::default()
        };
        let catalog = N8nCatalog::new(config).unwrap();
        let err = catalog.resolve("jira_fetch", Some("v1")).unwrap_err();
        assert!(matches!(
            err,
            N8nCatalogError::WorkflowNotApproved {
                workflow_id,
                status: N8nWorkflowStatus::Disabled
            } if workflow_id == "jira_fetch"
        ));
    }

    #[test]
    fn workflow_approval_metadata_reports_missing_fields() {
        let workflow = N8nWorkflowConfig {
            workflow_id: "draft".into(),
            workflow_version: "v1".into(),
            display_name: "Draft".into(),
            endpoint_path: "/webhook/draft".into(),
            ..Default::default()
        };

        let missing = workflow.missing_approval_metadata();

        assert!(missing.contains(&"owner"));
        assert!(missing.contains(&"requires_callback"));
        assert!(missing.contains(&"input_schema_ref"));
        assert!(missing.contains(&"output_schema_ref"));
        assert!(missing.contains(&"expected_evidence"));
        assert!(missing.contains(&"credential_requirements"));
        assert!(missing.contains(&"data_scope"));
        assert!(missing.contains(&"hitl_policy"));
        assert!(missing.contains(&"category"));
        assert!(missing.contains(&"example_prompts"));
        assert!(!workflow.is_ready_for_approval());
    }

    #[test]
    fn workflow_approval_metadata_accepts_complete_contract() {
        let workflow = workflow(N8nWorkflowStatus::Draft);

        assert!(workflow.missing_approval_metadata().is_empty());
        assert!(workflow.is_ready_for_approval());
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
    fn callback_parser_rejects_stale_callback() {
        let catalog = catalog_with_workflow();
        let mut event = callback_event("event_1", 1, N8nRunStatus::Running);
        event.occurred_at_ms = test_unix_ms().saturating_sub(301_000);
        let body = serde_json::to_vec(&event).unwrap();
        let signature = sign_payload(b"secret", &body).unwrap();
        let err = parse_and_verify_callback(&catalog, &body, &signature).unwrap_err();

        assert!(matches!(err, N8nCallbackError::CallbackTooOld { .. }));
    }

    #[test]
    fn callback_parser_rejects_future_callback_beyond_skew() {
        let catalog = catalog_with_workflow();
        let mut event = callback_event("event_1", 1, N8nRunStatus::Running);
        event.occurred_at_ms = test_unix_ms() + 31_000;
        let body = serde_json::to_vec(&event).unwrap();
        let signature = sign_payload(b"secret", &body).unwrap();
        let err = parse_and_verify_callback(&catalog, &body, &signature).unwrap_err();

        assert!(matches!(err, N8nCallbackError::CallbackFromFuture { .. }));
    }

    #[test]
    #[serial]
    fn n8n_config_migrates_literal_signing_secret_to_local_file() {
        let temp_home = tempdir().unwrap();
        let _home_guard = EnvVarGuard::set("HOME", temp_home.path());
        let _secret_guard = EnvVarGuard::remove("KRIA_N8N_SIGNING_SECRET");
        let mut config = N8nConfig {
            signing_secret: "legacy-secret".into(),
            ..Default::default()
        };

        let path = config
            .migrate_literal_signing_secret_to_file()
            .unwrap()
            .unwrap();

        assert_eq!(path, temp_home.path().join(".kria/secrets/n8n.key"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().trim(),
            "legacy-secret"
        );
        assert!(config.signing_secret.is_empty());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    #[serial]
    fn n8n_config_migrates_literal_api_key_to_local_file() {
        let temp_home = tempdir().unwrap();
        let _home_guard = EnvVarGuard::set("HOME", temp_home.path());
        let _api_guard = EnvVarGuard::remove("KRIA_N8N_API_KEY");
        let mut config = N8nConfig {
            api_key: "legacy-api-key".into(),
            ..Default::default()
        };

        let path = config.migrate_literal_api_key_to_file().unwrap().unwrap();

        assert_eq!(path, temp_home.path().join(".kria/secrets/n8n_api_key"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().trim(),
            "legacy-api-key"
        );
        assert!(config.api_key.is_empty());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    #[serial]
    fn n8n_config_rejects_literal_secret_when_migration_fails() {
        let temp_home = tempdir().unwrap();
        let home_file = temp_home.path().join("home-is-file");
        std::fs::write(&home_file, "not a directory").unwrap();
        let _home_guard = EnvVarGuard::set("HOME", &home_file);
        let _secret_guard = EnvVarGuard::remove("KRIA_N8N_SIGNING_SECRET");
        let config = N8nConfig {
            signing_secret: "legacy-secret".into(),
            ..Default::default()
        };

        let resolved = config.with_resolved_secret();

        assert!(resolved.signing_secret.is_empty());
    }

    #[test]
    #[serial]
    fn n8n_config_resolves_env_then_file_before_legacy_literal() {
        let temp_home = tempdir().unwrap();
        let _home_guard = EnvVarGuard::set("HOME", temp_home.path());
        let _secret_guard = EnvVarGuard::remove("KRIA_N8N_SIGNING_SECRET");
        let secret_path = temp_home.path().join(".kria/secrets/n8n.key");
        std::fs::create_dir_all(secret_path.parent().unwrap()).unwrap();
        std::fs::write(&secret_path, "file-secret\n").unwrap();

        let config = N8nConfig {
            signing_secret: "legacy-secret".into(),
            ..Default::default()
        };

        assert_eq!(config.resolve_signing_secret(), "file-secret");

        std::env::set_var("KRIA_N8N_SIGNING_SECRET", "env-secret");
        assert_eq!(config.resolve_signing_secret(), "env-secret");
    }

    #[test]
    #[serial]
    fn n8n_config_resolves_configured_api_key_env_then_file_then_manual() {
        let temp_home = tempdir().unwrap();
        let _home_guard = EnvVarGuard::set("HOME", temp_home.path());
        let _api_guard = EnvVarGuard::remove("KRIA_TEST_N8N_API_KEY");
        let api_key_path = temp_home.path().join(".kria/secrets/custom_n8n_api_key");
        std::fs::create_dir_all(api_key_path.parent().unwrap()).unwrap();
        std::fs::write(&api_key_path, "file-api-key\n").unwrap();

        let config = N8nConfig {
            api_key: "manual-api-key".into(),
            api_key_env: "KRIA_TEST_N8N_API_KEY".into(),
            api_key_file: "~/.kria/secrets/custom_n8n_api_key".into(),
            ..Default::default()
        };

        assert_eq!(config.resolve_api_key(), "file-api-key");

        std::env::set_var("KRIA_TEST_N8N_API_KEY", "env-api-key");
        assert_eq!(config.resolve_api_key(), "env-api-key");

        std::env::remove_var("KRIA_TEST_N8N_API_KEY");
        std::fs::remove_file(api_key_path).unwrap();
        assert_eq!(config.resolve_api_key(), "manual-api-key");

        let resolved = config.with_resolved_secret();
        assert_eq!(resolved.api_key, "manual-api-key");
    }

    #[test]
    fn n8n_config_defaults_to_external_runtime_mode() {
        let config = N8nConfig::default();
        assert_eq!(config.config_version, 2);
        assert_eq!(config.mode, N8nRuntimeMode::External);
        assert!(!config.auto_start);
        assert_eq!(config.managed_docker.container_name, "kria-n8n");
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

    #[test]
    fn governance_requires_review_and_output_schema_for_yellow_workflows() {
        let workflow = required_review_workflow();

        let store = N8nWorkflowStateStore::default();
        assert_eq!(
            store.ingest(slack_callback(
                "slack_event_1",
                serde_json::json!({"result": "harness complete"})
            )),
            N8nIngestDecision::Accepted
        );
        let run = store.get("slack_turn_1").unwrap();
        let decision = evaluate_run(Some(&workflow), &run);
        assert_eq!(
            decision.verification_status,
            N8nVerificationStatus::HumanReviewRequired
        );

        let store = N8nWorkflowStateStore::default();
        assert_eq!(
            store.ingest(slack_callback(
                "slack_event_2",
                serde_json::json!({"result": "harness complete", "confirmed_by_user": true})
            )),
            N8nIngestDecision::Accepted
        );
        let run = store.get("slack_turn_1").unwrap();
        let decision = evaluate_run(Some(&workflow), &run);
        assert_eq!(
            decision.verification_status,
            N8nVerificationStatus::NeedsMoreEvidence
        );
        assert!(decision
            .missing_evidence
            .iter()
            .any(|issue| issue.contains("message_ref")));

        let store = N8nWorkflowStateStore::default();
        assert_eq!(
            store.ingest(slack_callback(
                "slack_event_3",
                serde_json::json!({
                    "result": "harness complete",
                    "message_ref": "harness-slack-1",
                    "confirmed_by_user": true
                })
            )),
            N8nIngestDecision::Accepted
        );
        let run = store.get("slack_turn_1").unwrap();
        let decision = evaluate_run(Some(&workflow), &run);
        assert_eq!(
            decision.verification_status,
            N8nVerificationStatus::Verified
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
    async fn client_rejects_missing_schema_inputs_and_missing_review_confirmation() {
        let config = N8nConfig {
            enabled: true,
            base_url: "http://127.0.0.1:5678".into(),
            signing_secret: "secret".into(),
            workflows: vec![required_review_workflow()],
            ..Default::default()
        };
        let catalog = Arc::new(N8nCatalog::new(config).unwrap());
        let client = N8nClient::new(catalog).unwrap();

        let err = client
            .invoke(N8nToolRequest {
                workflow_id: "slack_post_update".into(),
                input_payload: serde_json::json!({}),
                ..Default::default()
            })
            .await
            .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("channel"));
        assert!(message.contains("message"));

        let err = client
            .invoke(N8nToolRequest {
                workflow_id: "slack_post_update".into(),
                input_payload: serde_json::json!({
                    "channel": "#team",
                    "message": "Build passed",
                    "source_prompt": "Post 'Build passed' to #team"
                }),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("confirmed_by_user must be true"));
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

    #[test]
    fn tool_request_accepts_legacy_workflow_name_alias() {
        let request: N8nToolRequest = serde_json::from_value(serde_json::json!({
            "workflow_name": "gmail_inbox_digest",
            "input_payload": {
                "source_prompt": "run gmail_inbox_digest"
            }
        }))
        .unwrap();

        assert_eq!(request.workflow_id, "gmail_inbox_digest");
        assert_eq!(
            request.input_payload["source_prompt"],
            "run gmail_inbox_digest"
        );
    }
}
