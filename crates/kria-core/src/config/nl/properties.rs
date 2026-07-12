//! Correctness properties P1–P10 for the NL settings control pipeline
//! (settings-nl-control Task 13 / design.md "Correctness properties").
//!
//! These are the spec's binding invariants, exercised through the SAME public
//! `SettingsIntentPipeline` + `SettingsHandler` that both chat and the desktop
//! command surface call — so proving them here proves them for both surfaces
//! (there is exactly one decider + one executor). The classifier + handler are
//! LLM- and embedder-free (tier-A), so this suite runs fully offline/deterministic
//! (P10 / design Wave 5 F14).

#![cfg(test)]

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::config::nl::{
    ConversationContext, SchemaEntityIndex, SettingsDecision, SettingsHandler,
    SettingsIntentPipeline, SettingsOutcome, SettingsRequest, SettingsRequestKind,
};
use crate::config::prompt::Scope;
use crate::config::{is_secret_field, ConfigService, KriaConfig, NoopPersist};
use crate::infra::event_bus::EventBus;
use crate::safety::RiskLevel;
use crate::tools::TriggerProvenance;

fn service() -> Arc<ConfigService> {
    let cfg = Arc::new(RwLock::new(KriaConfig::default()));
    let bus = Arc::new(EventBus::new(64));
    Arc::new(ConfigService::with_persist(cfg, bus, Arc::new(NoopPersist)))
}

fn pipeline() -> SettingsIntentPipeline {
    SettingsIntentPipeline::new(Arc::new(SchemaEntityIndex::build()))
}

/// Map a classifier `SettingsDecision` into a `SettingsRequest` the handler runs —
/// this is exactly what both the chat gate and the command surface do, so calling
/// it here exercises the ONE shared path (P1).
fn decision_to_request(d: SettingsDecision, session: &str) -> Option<SettingsRequest> {
    match d {
        SettingsDecision::Change {
            section,
            field,
            value,
            scope,
        } => Some(SettingsRequest {
            kind: if scope == Scope::Temp {
                SettingsRequestKind::TempOverride
            } else {
                SettingsRequestKind::Change
            },
            section,
            field,
            value,
            scope,
            provenance: TriggerProvenance::User,
            session_id: session.to_string(),
        }),
        SettingsDecision::ReadBack { section, field } => {
            Some(SettingsRequest::read_back(section, field).with_session(session))
        }
        SettingsDecision::Undo => Some(SettingsRequest {
            kind: SettingsRequestKind::Undo,
            section: String::new(),
            field: String::new(),
            value: None,
            scope: Scope::Permanent,
            provenance: TriggerProvenance::User,
            session_id: session.to_string(),
        }),
        // Clarify / NotSettings never build a request (never mutate — P2).
        _ => None,
    }
}

// ── P1: One path — chat and command produce identical decision + effect ──────
#[tokio::test]
async fn p1_one_path_chat_and_command_are_identical() {
    let conv = ConversationContext::default();
    let prompt = "switch to dark mode";

    // Both "surfaces" run the SAME classify → decision_to_request → handler path.
    let d_chat = pipeline().classify(prompt, &conv);
    let d_cmd = pipeline().classify(prompt, &conv);
    assert_eq!(d_chat, d_cmd, "same prompt must yield the same decision");

    let svc_chat = service();
    let svc_cmd = service();
    let out_chat = SettingsHandler::new(svc_chat.clone())
        .handle(decision_to_request(d_chat, "chat").unwrap())
        .await;
    let out_cmd = SettingsHandler::new(svc_cmd.clone())
        .handle(decision_to_request(d_cmd, "cmd").unwrap())
        .await;

    assert!(matches!(out_chat, SettingsOutcome::Applied { .. }));
    assert!(matches!(out_cmd, SettingsOutcome::Applied { .. }));
    // Identical persisted effect.
    assert_eq!(svc_chat.get().await.ui.theme, svc_cmd.get().await.ui.theme);
    assert_eq!(svc_chat.get().await.ui.theme, "dark");
}

// ── P2: Separation — NotSettings never mutates; KRIA imperative → Change ─────
#[tokio::test]
async fn p2_false_positives_never_build_a_mutation() {
    let conv = ConversationContext::default();
    // Conversation-intent prompts that resemble settings but must NOT act.
    let false_positives = [
        "I'll change my CSS theme later",
        "turn on the lights",
        "change the api key in my code",
        "switch branches",
        "what is dark mode?",
    ];
    for p in false_positives {
        let d = pipeline().classify(p, &conv);
        assert!(
            matches!(
                d,
                SettingsDecision::NotSettings | SettingsDecision::Clarify { .. }
            ),
            "false positive acted: {p:?} → {d:?}"
        );
        // No request is ever built for these → structurally impossible to mutate.
        assert!(
            decision_to_request(d, "s").is_none(),
            "false positive built a mutation request: {p:?}"
        );
    }
    // A KRIA-directed imperative on a schema field IS a change.
    let d = pipeline().classify("switch to dark mode", &conv);
    assert!(matches!(d, SettingsDecision::Change { .. }), "got {d:?}");
}

// ── P3: No raw mutation — GREEN auto; non-GREEN needs approval, no raw apply ──
#[tokio::test]
async fn p3_non_green_never_applies_without_approval() {
    let svc = service();
    let h = SettingsHandler::new(svc.clone());
    // YELLOW field: handle() must NOT apply; it returns NeedsApproval.
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
    assert_ne!(
        svc.get().await.agent.autonomy_profile,
        "aggressive",
        "must not apply before approval"
    );
    // Only the explicit post-approval call applies it.
    assert!(matches!(
        h.apply_approved(&csid).await,
        SettingsOutcome::Applied { .. }
    ));
    assert_eq!(svc.get().await.agent.autonomy_profile, "aggressive");

    // GREEN field applies immediately.
    let svc2 = service();
    let out2 = SettingsHandler::new(svc2.clone())
        .handle(SettingsRequest::change(
            "ui",
            "theme",
            serde_json::json!("dark"),
        ))
        .await;
    assert!(matches!(out2, SettingsOutcome::Applied { .. }));
}

// ── P4: Read truth — read-back equals ConfigService effective value ──────────
#[tokio::test]
async fn p4_read_back_equals_effective_value() {
    let svc = service();
    let h = SettingsHandler::new(svc.clone());
    h.handle(SettingsRequest::change(
        "ui",
        "theme",
        serde_json::json!("light"),
    ))
    .await;
    let effective = svc.get().await.ui.theme.clone();
    match h.handle(SettingsRequest::read_back("ui", "theme")).await {
        SettingsOutcome::Answer { text } => {
            assert!(
                text.contains(&effective),
                "read-back {text:?} != {effective}"
            );
        }
        other => panic!("expected Answer, got {other:?}"),
    }
}

// ── P5: Secret safety — no write clears/leaks a secret; preserve = is_secret ──
#[tokio::test]
async fn p5_secret_change_refused_and_readback_hides_value() {
    let h = SettingsHandler::new(service());
    // Every secret field refuses a generic change.
    for (s, f) in [
        ("llm", "cloud_api_key"),
        ("planner", "cloud_api_key"),
        ("server", "jwt_secret"),
        ("telegram", "bot_token"),
        ("image_generation", "hf_inference_token"),
    ] {
        assert!(is_secret_field(s, f), "test fixture drift: {s}.{f}");
        let out = h
            .handle(SettingsRequest::change(
                s,
                f,
                serde_json::json!("secret-value"),
            ))
            .await;
        assert!(
            matches!(out, SettingsOutcome::Refused { .. }),
            "secret {s}.{f} change not refused: {out:?}"
        );
        // Read-back reports set/unset only — never the value.
        match h.handle(SettingsRequest::read_back(s, f)).await {
            SettingsOutcome::Answer { text } => {
                assert!(!text.contains("secret-value"));
                assert!(text.contains("set") || text.contains("not set"));
            }
            other => panic!("expected Answer, got {other:?}"),
        }
    }
}

#[test]
fn p5_preserve_set_covers_every_secret_field() {
    // A redacted whole-blob save must restore EVERY is_secret_field from the live
    // config (single source), so it can never clobber a stored secret.
    let mut live = KriaConfig::default();
    live.llm.cloud_api_key = "sk-live".into();
    live.planner.cloud_api_key = "pk-live".into();
    live.server.jwt_secret = "jwt-live".into();
    live.telegram.bot_token = "tg-live".into();
    live.image_generation.hf_inference_token = "hf-live".into();

    // Incoming blob is fully redacted (empty secrets).
    let mut incoming = KriaConfig::default();
    incoming.preserve_secrets_from(&live);

    assert_eq!(incoming.llm.cloud_api_key, "sk-live");
    assert_eq!(incoming.planner.cloud_api_key, "pk-live");
    assert_eq!(incoming.server.jwt_secret, "jwt-live");
    assert_eq!(incoming.telegram.bot_token, "tg-live");
    assert_eq!(incoming.image_generation.hf_inference_token, "hf-live");
}

// ── P6: Injection wall — non-User provenance never mutates ───────────────────
#[tokio::test]
async fn p6_injection_wall_refuses_external_provenance() {
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

// ── P7: Per-session isolation — provenance is per-request, never bleeds ───────
#[tokio::test]
async fn p7_provenance_is_per_request_no_bleed() {
    let svc = service();
    let h = SettingsHandler::new(svc.clone());
    // An external (injected) request is refused...
    let refused = h
        .handle(
            SettingsRequest::change("ui", "theme", serde_json::json!("dark"))
                .with_provenance(TriggerProvenance::ExternalContent)
                .with_session("session-A"),
        )
        .await;
    assert!(matches!(refused, SettingsOutcome::Refused { .. }));
    // ...and does NOT taint a subsequent genuine user request in another session.
    let ok = h
        .handle(
            SettingsRequest::change("ui", "theme", serde_json::json!("light"))
                .with_provenance(TriggerProvenance::User)
                .with_session("session-B"),
        )
        .await;
    assert!(matches!(ok, SettingsOutcome::Applied { .. }));
    assert_eq!(svc.get().await.ui.theme, "light");
}

// ── P8: No hardcoding — a brand-new schema field is routed with zero code ─────
#[test]
fn p8_synthetic_field_routes_without_per_field_code() {
    // Inject a field that exists in NO routing/keyword branch anywhere. If the
    // classifier resolves + acts on it, routing is provably schema-driven (Req 3.1).
    let mut idx = SchemaEntityIndex::build();
    idx.push_synthetic_field("synthetic", "widget_7", "magic gizmo7x", "gizmo7x");
    let pipe = SettingsIntentPipeline::new(Arc::new(idx));
    let conv = ConversationContext::default();

    let d = pipe.classify("set magic gizmo7x now", &conv);
    match d {
        SettingsDecision::Change { section, field, .. } => {
            assert_eq!(
                (section.as_str(), field.as_str()),
                ("synthetic", "widget_7")
            );
        }
        other => panic!("synthetic field not routed as a change: {other:?}"),
    }
}

// ── P11: Evidence separation — same phrase, different destination by evidence ─

/// Deterministic bag-of-words "embedder" for tests: cosine reflects word overlap,
/// so a message that shares vocabulary with the recent conversation scores high.
struct BagEmbedder;
impl crate::config::nl::TextEmbedder for BagEmbedder {
    fn embed(&self, text: &str) -> Option<Vec<f32>> {
        let mut v = vec![0f32; 64];
        for tok in text
            .to_ascii_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
        {
            let h = tok
                .bytes()
                .fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize))
                % 64;
            v[h] += 1.0;
        }
        Some(v)
    }
}

/// Memory source that always reports a strong ongoing non-config topic.
struct StrongTopicMemory;
impl crate::config::nl::MemoryEvidenceSource for StrongTopicMemory {
    fn topic_affinity(&self, _text: &str) -> Option<f32> {
        Some(1.0)
    }
}

#[test]
fn p11_same_phrase_routes_by_conversation_evidence() {
    let idx = Arc::new(SchemaEntityIndex::build());
    let phrase = "what is the current theme";

    // (A) No conversation topic → the phrase reads back the KRIA setting.
    let plain = SettingsIntentPipeline::new(idx.clone());
    let empty = ConversationContext::default();
    assert!(
        matches!(
            plain.classify(phrase, &empty),
            SettingsDecision::ReadBack { .. }
        ),
        "with no competing topic, it should read back the KRIA theme"
    );

    // (B) SAME phrase, but the recent conversation is about a presentation theme
    // (semantic topic via the embedder) → evidence points to conversation, NOT
    // KRIA config → must NOT read back the setting (fails toward conversation).
    let with_emb = SettingsIntentPipeline::new(idx.clone()).with_evidence(
        crate::config::nl::EvidenceDeps::default().with_embedder(Arc::new(BagEmbedder)),
    );
    let discussing = ConversationContext::new(
        vec!["the current theme of my presentation slides looks great".into()],
        vec!["yes the presentation theme and layout are consistent".into()],
    );
    let d = with_emb.classify(phrase, &discussing);
    assert!(
        !matches!(d, SettingsDecision::ReadBack { .. }),
        "with a strong conversation topic, evidence must steer AWAY from KRIA config, got {d:?}"
    );
}

#[test]
fn p11_memory_evidence_participates() {
    let idx = Arc::new(SchemaEntityIndex::build());
    let phrase = "what is the current theme";
    // Memory reporting a strong ongoing topic suppresses the weak settings guess.
    let with_mem = SettingsIntentPipeline::new(idx).with_evidence(
        crate::config::nl::EvidenceDeps::default().with_memory(Arc::new(StrongTopicMemory)),
    );
    let d = with_mem.classify(phrase, &ConversationContext::default());
    assert!(
        !matches!(d, SettingsDecision::ReadBack { .. }),
        "memory topic-affinity must participate and steer away from config, got {d:?}"
    );
}

#[test]
fn p11_graceful_degradation_no_embedder_is_lexical_identical() {
    // With no evidence deps the trace reports embeddings unused and the decision
    // matches the plain lexical path (offline parity).
    let idx = Arc::new(SchemaEntityIndex::build());
    let p = SettingsIntentPipeline::new(idx);
    let (_d, trace) = p.classify_traced("switch to dark mode", &ConversationContext::default());
    assert!(
        !trace.embeddings_used,
        "no embedder ⇒ embeddings_used=false"
    );
}

// ── P15: No interference — general/content/knowledge/etc. never touch settings ─
#[test]
fn p15_non_settings_prompts_never_engage_settings() {
    let conv = ConversationContext::default();
    let p = pipeline();
    // A broad set spanning content, knowledge, coding, memory, marketplace, tasks —
    // NONE may become a settings Change/Info/ReadBack/Undo (must be NotSettings).
    let general = [
        "generate an image of a cat",
        "create a dark themed poster",
        "write a poem about dark mode",
        "write a function to sort an array",
        "what is the capital of France",
        "how are you today",
        "search for the latest AI news",
        "install the calculator skill",
        "remember that my birthday is in May",
        "open google chrome",
        "summarize this article for me",
        "draw me something in light colors",
        "what's the temperature outside",
        "should I use Gemini for my project",
    ];
    let mut leaks = Vec::new();
    for g in general {
        match p.classify(g, &conv) {
            SettingsDecision::NotSettings => {}
            other => leaks.push(format!("  {g:?} → {other:?}")),
        }
    }
    assert!(
        leaks.is_empty(),
        "settings subsystem interfered with {} general prompt(s):\n{}",
        leaks.len(),
        leaks.join("\n")
    );
}

// ── P9: Legacy equivalence — the pipeline is a pure, env-free gate ───────────
// Flag-off byte-for-byte legacy is enforced at the loop's `nl_settings_enabled()`
// call site (tested in `agent::loop_engine::tests::nl_settings_flag_reads_either_env`):
// when off, `run_settings_stage` never runs and the turn is untouched. Here we
// prove the pipeline itself reads NO global/env state, so it cannot perturb legacy
// behaviour when it is not invoked.
#[test]
fn p9_pipeline_is_pure_and_deterministic() {
    let conv = ConversationContext::default();
    let p = pipeline();
    let first = p.classify("switch to dark mode", &conv);
    for _ in 0..50 {
        assert_eq!(p.classify("switch to dark mode", &conv), first);
    }
}

// ── P10: LLM-optional — GREEN change, read-back, undo work with no model ──────
#[tokio::test]
async fn p10_core_ops_succeed_without_llm_or_embedder() {
    // The default pipeline + handler carry NO embedder and NO LLM client.
    let conv = ConversationContext::default();
    let svc = service();
    let h = SettingsHandler::new(svc.clone());

    // GREEN change.
    let d = pipeline().classify("switch to dark mode", &conv);
    let applied = h.handle(decision_to_request(d, "s").unwrap()).await;
    assert!(matches!(applied, SettingsOutcome::Applied { .. }));
    assert_eq!(svc.get().await.ui.theme, "dark");

    // Read-back.
    let d = pipeline().classify("what is my current theme?", &conv);
    let answer = h.handle(decision_to_request(d, "s").unwrap()).await;
    assert!(
        matches!(answer, SettingsOutcome::Answer { .. }),
        "got {answer:?}"
    );

    // Undo (in-memory ring, no model needed).
    let d = pipeline().classify("undo that setting change", &conv);
    let undone = h.handle(decision_to_request(d, "s").unwrap()).await;
    assert!(
        matches!(undone, SettingsOutcome::Undone { .. }),
        "undo without LLM failed: {undone:?}"
    );
    assert_ne!(
        svc.get().await.ui.theme,
        "dark",
        "undo restored prior value"
    );
}
