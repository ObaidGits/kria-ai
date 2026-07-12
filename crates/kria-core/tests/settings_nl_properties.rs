//! Property tests P1–P10 for the unified NL settings pipeline (settings-nl-control
//! Task 13). These assert the design's correctness properties end-to-end through
//! the PUBLIC API only (the single `SettingsIntentPipeline` + `SettingsHandler`
//! that BOTH chat and the command surface call), so they hold for every entry
//! point. Golden-set classification is covered by `config::nl::pipeline::tests`;
//! 500-field scaling by `config::nl::entity_index::tests`; flag-off inertness by
//! `agent::loop_engine::tests`.

use std::sync::Arc;

use kria_core::config::nl::{
    ConversationContext, SchemaEntityIndex, SettingsDecision, SettingsHandler,
    SettingsIntentPipeline, SettingsOutcome, SettingsRequest, SettingsRequestKind,
};
use kria_core::config::schema;
use kria_core::config::{ConfigService, KriaConfig, NoopPersist};
use kria_core::infra::event_bus::EventBus;
use kria_core::safety::RiskLevel;
use kria_core::tools::TriggerProvenance;
use tokio::sync::RwLock;

fn service() -> Arc<ConfigService> {
    let cfg = Arc::new(RwLock::new(KriaConfig::default()));
    let bus = Arc::new(EventBus::new(64));
    Arc::new(ConfigService::with_persist(cfg, bus, Arc::new(NoopPersist)))
}

fn pipeline() -> SettingsIntentPipeline {
    SettingsIntentPipeline::new(Arc::new(SchemaEntityIndex::build()))
}

// ── P1 One path: same prompt ⇒ same decision + same persisted effect ─────────
#[tokio::test]
async fn p1_one_path_chat_and_command_identical() {
    let conv = ConversationContext::default();
    let prompt = "switch to dark mode";

    // Same decision every time (deterministic classifier — the ONE decider).
    let d1 = pipeline().classify(prompt, &conv);
    let d2 = pipeline().classify(prompt, &conv);
    assert_eq!(d1, d2, "classifier must be deterministic");
    let (section, field, value) = match d1 {
        SettingsDecision::Change {
            section,
            field,
            value,
            ..
        } => (section, field, value),
        other => panic!("expected Change, got {other:?}"),
    };

    // Two independent services (surrogate for chat vs command) → identical effect.
    let svc_chat = service();
    let svc_cmd = service();
    let out_chat = SettingsHandler::new(svc_chat.clone())
        .handle(SettingsRequest::change(
            section.clone(),
            field.clone(),
            value.clone().unwrap(),
        ))
        .await;
    let out_cmd = SettingsHandler::new(svc_cmd.clone())
        .handle(SettingsRequest::change(section, field, value.unwrap()))
        .await;
    assert!(matches!(out_chat, SettingsOutcome::Applied { .. }));
    assert_eq!(out_chat, out_cmd, "same prompt ⇒ identical outcome");
    assert_eq!(svc_chat.get().await.ui.theme, svc_cmd.get().await.ui.theme);
    assert_eq!(svc_chat.get().await.ui.theme, "dark");
}

// ── P2 Separation: NotSettings never mutates; KRIA-directed imperative acts ──
#[tokio::test]
async fn p2_configuration_vs_conversation_separation() {
    let conv = ConversationContext::default();
    let p = pipeline();

    // Pure conversation → NotSettings.
    assert_eq!(
        p.classify("what is the capital of France", &conv),
        SettingsDecision::NotSettings
    );
    // User-artifact subject → NotSettings (talking about their own code).
    assert_eq!(
        p.classify("change the api key in my code", &conv),
        SettingsDecision::NotSettings
    );
    // KRIA-directed imperative on a schema field → Change.
    match p.classify("change your theme to dark", &conv) {
        SettingsDecision::Change { section, field, .. } => {
            assert_eq!((section.as_str(), field.as_str()), ("ui", "theme"));
        }
        other => panic!("expected Change, got {other:?}"),
    }
}

// ── P3 No raw mutation: non-GREEN never auto-applies (goes through approval) ──
#[tokio::test]
async fn p3_no_raw_mutation_for_non_green() {
    let svc = service();
    let h = SettingsHandler::new(svc.clone());
    // YELLOW field: must return NeedsApproval, NOT Applied — config unchanged.
    let out = h
        .handle(SettingsRequest::change(
            "agent",
            "autonomy_profile",
            serde_json::json!("aggressive"),
        ))
        .await;
    let csid = match out {
        SettingsOutcome::NeedsApproval {
            change_set_id,
            risk,
            ..
        } => {
            assert_ne!(risk, RiskLevel::Green, "non-GREEN must gate");
            change_set_id
        }
        other => panic!("expected NeedsApproval, got {other:?}"),
    };
    assert_ne!(svc.get().await.agent.autonomy_profile, "aggressive");
    // Only after explicit approval does it persist.
    assert!(matches!(
        h.apply_approved(&csid).await,
        SettingsOutcome::Applied { .. }
    ));
    assert_eq!(svc.get().await.agent.autonomy_profile, "aggressive");
}

// ── P4 Read truth: read-back == ConfigService effective value ────────────────
#[tokio::test]
async fn p4_read_back_reflects_config_service() {
    let svc = service();
    let h = SettingsHandler::new(svc.clone());
    h.handle(SettingsRequest::change(
        "ui",
        "theme",
        serde_json::json!("dark"),
    ))
    .await;
    let effective = svc.read_field("ui", "theme").await.unwrap();
    match h.handle(SettingsRequest::read_back("ui", "theme")).await {
        SettingsOutcome::Answer { text } => {
            assert!(
                text.contains(effective.as_str().unwrap()),
                "read-back '{text}' must reflect effective value {effective:?}"
            );
        }
        other => panic!("expected Answer, got {other:?}"),
    }
}

// ── P5 Secret safety: no write path clears/leaks a secret ────────────────────
#[tokio::test]
async fn p5_secret_safety() {
    let h = SettingsHandler::new(service());
    // Direct change to a secret field is refused (never plaintext-written).
    assert!(matches!(
        h.handle(SettingsRequest::change(
            "llm",
            "cloud_api_key",
            serde_json::json!("sk-should-not-apply")
        ))
        .await,
        SettingsOutcome::Refused { .. }
    ));
    // Read-back of a secret reports set/unset only — never the value.
    match h
        .handle(SettingsRequest::read_back("llm", "cloud_api_key"))
        .await
    {
        SettingsOutcome::Answer { text } => {
            assert!(!text.contains("sk-"), "secret value leaked: {text}");
            assert!(text.contains("set") || text.contains("not set"));
        }
        other => panic!("expected Answer, got {other:?}"),
    }

    // Whole-blob preserve: preserve_secrets_from restores EVERY is_secret_field
    // from the live config (a redacted incoming blob can never clobber).
    let mut live = KriaConfig::default();
    live.llm.cloud_api_key = "sk-live".into();
    live.server.jwt_secret = "jwt-live".into();
    live.telegram.bot_token = "tg-live".into();
    let mut incoming = KriaConfig::default(); // redacted/empty secrets
    incoming.preserve_secrets_from(&live);
    assert_eq!(incoming.llm.cloud_api_key, "sk-live");
    assert_eq!(incoming.server.jwt_secret, "jwt-live");
    assert_eq!(incoming.telegram.bot_token, "tg-live");
}

// ── P6 Injection wall: non-User provenance never mutates ─────────────────────
#[tokio::test]
async fn p6_injection_wall() {
    let svc = service();
    let h = SettingsHandler::new(svc.clone());
    let out = h
        .handle(
            SettingsRequest::change("ui", "theme", serde_json::json!("dark"))
                .with_provenance(TriggerProvenance::ExternalContent),
        )
        .await;
    assert!(matches!(out, SettingsOutcome::Refused { .. }));
    assert_ne!(
        svc.get().await.ui.theme,
        "dark",
        "injection must not mutate"
    );
}

// ── P7 Per-session isolation: pending changes keyed per change-set, no bleed ─
#[tokio::test]
async fn p7_per_session_isolation() {
    let svc = service();
    let h = SettingsHandler::new(svc.clone());
    // Session A raises a gated change (NeedsApproval).
    let a = h
        .handle(
            SettingsRequest::change("agent", "autonomy_profile", serde_json::json!("aggressive"))
                .with_session("session-A"),
        )
        .await;
    let a_id = match a {
        SettingsOutcome::NeedsApproval { change_set_id, .. } => change_set_id,
        other => panic!("expected NeedsApproval, got {other:?}"),
    };
    // A wrong / other-session change-set id cannot apply A's change.
    assert!(matches!(
        h.apply_approved("session-B-bogus-id").await,
        SettingsOutcome::Refused { .. }
    ));
    assert_ne!(svc.get().await.agent.autonomy_profile, "aggressive");
    // The correct id still works exactly once.
    assert!(matches!(
        h.apply_approved(&a_id).await,
        SettingsOutcome::Applied { .. }
    ));
    assert!(matches!(
        h.apply_approved(&a_id).await,
        SettingsOutcome::Refused { .. }
    ));
}

// ── P8 No hardcoding: every prompt-changeable annotated field is auto-routable
//    from its own synonyms with ZERO per-field routing code. ──────────────────
#[test]
fn p8_no_hardcoding_schema_driven_routing() {
    let idx = SchemaEntityIndex::build();
    let mut checked = 0;
    for (section, field) in schema::all_fields() {
        let meta = schema::field_meta(&section, &field);
        if !meta.prompt_changeable || meta.synonyms.is_empty() {
            continue;
        }
        // Feed each synonym; the field MUST appear among the resolved candidates
        // purely because it is annotated — no code mentions this field by name.
        for syn in meta.synonyms {
            let cands = idx.resolve(syn);
            assert!(
                cands
                    .iter()
                    .any(|c| c.section == section && c.field == field),
                "synonym {syn:?} did not resolve to its own field {section}.{field}"
            );
        }
        checked += 1;
    }
    assert!(
        checked >= 5,
        "expected several annotated fields, got {checked}"
    );
}

// ── P10 LLM-optional: GREEN change, read-back, undo all work with no LLM ──────
// The pipeline + handler here run with NO embedder and NO LLM client (tier-A
// lexical only), proving settings control survives model outages.
#[tokio::test]
async fn p10_llm_optional_offline_path() {
    let conv = ConversationContext::default();
    let p = pipeline();
    let svc = service();
    let h = SettingsHandler::new(svc.clone());

    // GREEN change (offline).
    match p.classify("switch to dark mode", &conv) {
        SettingsDecision::Change {
            section,
            field,
            value,
            ..
        } => {
            let out = h
                .handle(SettingsRequest::change(section, field, value.unwrap()))
                .await;
            assert!(matches!(out, SettingsOutcome::Applied { .. }));
        }
        other => panic!("expected Change, got {other:?}"),
    }
    assert_eq!(svc.get().await.ui.theme, "dark");

    // Read-back (offline).
    assert!(matches!(
        p.classify("what is my current theme?", &conv),
        SettingsDecision::ReadBack { .. }
    ));
    match h.handle(SettingsRequest::read_back("ui", "theme")).await {
        SettingsOutcome::Answer { text } => assert!(text.contains("dark")),
        other => panic!("expected Answer, got {other:?}"),
    }

    // Undo (offline) — restores the prior value.
    assert_eq!(
        p.classify("undo that setting", &conv),
        SettingsDecision::Undo
    );
    let out = h
        .handle(SettingsRequest {
            kind: SettingsRequestKind::Undo,
            section: String::new(),
            field: String::new(),
            value: None,
            scope: kria_core::config::prompt::Scope::Permanent,
            provenance: TriggerProvenance::User,
            session_id: String::new(),
        })
        .await;
    assert!(matches!(out, SettingsOutcome::Undone { .. }), "got {out:?}");
    assert_ne!(svc.get().await.ui.theme, "dark");
}
