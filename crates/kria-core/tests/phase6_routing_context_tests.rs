//! Phase 1: Context-Aware Routing Tests
//!
//! Tests for the routing context module which enables:
//! - Topic continuation (carry domain across turns)
//! - Correction detection ("no, I meant X")
//! - Context enrichment for short/ambiguous inputs

use std::time::{Duration, Instant};

use kria_core::config::RoutingConfig;
use kria_core::routing::context::{
    detect_correction, enrich_with_context, EnrichmentReason, RoutingContext,
};
use kria_core::routing::decide::{self, DecideInput, RouteDecision};
use kria_core::routing::domain::Domain;
use kria_core::routing::verbs::{IntentModality, ModalityResult};

// ─── Helper Functions ───────────────────────────────────────────────────────

fn default_modality() -> ModalityResult {
    ModalityResult {
        primary: IntentModality::Unknown,
        all: vec![],
        destructive: false,
        imperative_verb_count: 0,
    }
}

fn default_config() -> RoutingConfig {
    RoutingConfig::default()
}

fn make_context_with_domain(domain: Domain) -> RoutingContext {
    let mut ctx = RoutingContext::default();
    ctx.record_turn(domain, None, IntentModality::Read, vec![0.1; 384]);
    ctx
}

// ─── Topic Continuation Tests ───────────────────────────────────────────────

#[test]
fn ctx01_topic_continuation_carries_domain() {
    let mut ctx = RoutingContext::default();
    ctx.record_turn(
        Domain::SystemInfo,
        Some("check_system_health".into()),
        IntentModality::Read,
        vec![0.1; 384],
    );

    let enriched = enrich_with_context("also check disk", &ctx);
    assert!(enriched.text.contains("system")); // context injected
    assert_eq!(enriched.reason, EnrichmentReason::TopicContinuation);
}

#[test]
fn ctx02_correction_detection() {
    let ctx = RoutingContext {
        last_domain: Some(Domain::SystemInfo),
        ..Default::default()
    };
    let signal = detect_correction("no I meant the network", &ctx);
    assert!(signal.is_correction());
}

#[test]
fn ctx03_stale_context_not_used() {
    let mut ctx = RoutingContext::default();
    ctx.record_turn(
        Domain::SystemInfo,
        None,
        IntentModality::Read,
        vec![0.1; 384],
    );
    // Manually set last_turn_at to 120s ago to simulate stale context
    ctx.last_turn_at = Some(Instant::now() - Duration::from_secs(120));

    let enriched = enrich_with_context("check disk", &ctx);
    assert!(!enriched.text.contains("context")); // stale, no enrichment
}

#[test]
fn ctx04_long_input_not_enriched() {
    let mut ctx = RoutingContext::default();
    ctx.record_turn(
        Domain::SystemInfo,
        None,
        IntentModality::Read,
        vec![0.1; 384],
    );

    let long_text = "a".repeat(100);
    let enriched = enrich_with_context(&long_text, &ctx);
    assert_eq!(enriched.text, long_text); // too long, no enrichment
}

#[test]
fn ctx05_multi_turn_hinglish_continuation() {
    let mut ctx = RoutingContext::default();
    ctx.record_turn(
        Domain::SystemInfo,
        None,
        IntentModality::Read,
        vec![0.1; 384],
    );
    ctx.turn_count_in_topic = 2;

    let enriched = enrich_with_context("uska status bhi dikhao", &ctx);
    assert!(enriched.text.contains("system")); // Hinglish carry
}

#[test]
fn ctx06_context_resets_on_topic_change() {
    let mut ctx = RoutingContext::default();
    ctx.record_turn(
        Domain::SystemInfo,
        None,
        IntentModality::Read,
        vec![0.1; 384],
    );
    ctx.turn_count_in_topic = 3;

    // Simulate routing to different domain
    ctx.record_turn(Domain::Comms, None, IntentModality::Send, vec![0.2; 384]);
    assert_eq!(ctx.turn_count_in_topic, 1); // reset to 1 for new topic
}

#[test]
fn ctx07_correction_boosts_previous_domain() {
    let ctx = RoutingContext {
        last_domain: Some(Domain::SystemInfo),
        correction_pending: true,
        turn_count_in_topic: 1,
        ..Default::default()
    };

    let domain_sims = vec![
        (Domain::Knowledge, 0.5),
        (Domain::SystemInfo, 0.4),
        (Domain::FileOps, 0.3),
    ];

    let input = DecideInput {
        domain_sims: &domain_sims,
        ood_distribution: &[0.3, 0.35, 0.4, 0.45, 0.5],
        modality: &default_modality(),
        segments: &["no I meant the network".to_string()],
        segment_sims: &[],
        config: &default_config(),
        context: &ctx,
    };

    let decision = decide::decide(&input);
    // Should route to SystemInfo (correction boost), not Knowledge
    assert!(
        matches!(decision, RouteDecision::SingleDomain(Domain::SystemInfo)),
        "Expected SystemInfo, got {:?}",
        decision
    );
}

#[test]
fn ctx08_no_context_preserves_standard_routing() {
    let ctx = RoutingContext::default(); // empty context
    let enriched = enrich_with_context("open Chrome", &ctx);
    assert_eq!(enriched.text, "open Chrome"); // no change
}

#[test]
fn ctx09_serialization_roundtrip() {
    let ctx = RoutingContext {
        last_domain: Some(Domain::FileOps),
        last_tool: Some("read_file".into()),
        turn_count_in_topic: 3,
        ..Default::default()
    };
    let json = serde_json::to_string(&ctx).unwrap();
    let restored: RoutingContext = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.last_domain, Some(Domain::FileOps));
    assert_eq!(restored.turn_count_in_topic, 3);
}

#[test]
fn ctx10_latency_under_budget() {
    let ctx = RoutingContext::default();
    let start = Instant::now();
    for _ in 0..1000 {
        let _ = enrich_with_context("check system status", &ctx);
    }
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_millis(10)); // 1000 enrichments < 10ms
}

// ─── Additional Context Tests ───────────────────────────────────────────────

#[test]
fn ctx11_topic_continuation_shortcuts_decision() {
    let ctx = RoutingContext {
        last_domain: Some(Domain::SystemInfo),
        turn_count_in_topic: 3,
        ..Default::default()
    };

    let domain_sims = vec![
        (Domain::SystemInfo, 0.35),
        (Domain::Knowledge, 0.33),
        (Domain::FileOps, 0.2),
    ];

    let input = DecideInput {
        domain_sims: &domain_sims,
        ood_distribution: &[0.3, 0.35, 0.4, 0.45, 0.5],
        modality: &default_modality(),
        segments: &["also check the network".to_string()],
        segment_sims: &[],
        config: &default_config(),
        context: &ctx,
    };

    let decision = decide::decide(&input);
    // Topic continuation should shortcut to SystemInfo
    assert!(
        matches!(decision, RouteDecision::SingleDomain(Domain::SystemInfo)),
        "Expected SystemInfo, got {:?}",
        decision
    );
}

#[test]
fn ctx12_correction_without_prev_domain_no_boost() {
    let ctx = RoutingContext {
        correction_pending: true,
        ..Default::default()
    };

    let domain_sims = vec![(Domain::Knowledge, 0.6), (Domain::SystemInfo, 0.4)];

    let input = DecideInput {
        domain_sims: &domain_sims,
        ood_distribution: &[0.3, 0.35, 0.4, 0.45, 0.5],
        modality: &default_modality(),
        segments: &["no I meant the network".to_string()],
        segment_sims: &[],
        config: &default_config(),
        context: &ctx,
    };

    let decision = decide::decide(&input);
    // No previous domain → standard routing (Knowledge wins)
    assert!(
        matches!(decision, RouteDecision::SingleDomain(Domain::Knowledge)),
        "Expected Knowledge, got {:?}",
        decision
    );
}

#[test]
fn ctx13_context_enrichment_improves_embedding() {
    let mut ctx = RoutingContext::default();
    ctx.record_turn(Domain::Power, None, IntentModality::Read, vec![0.1; 384]);
    ctx.turn_count_in_topic = 1;

    let enriched = enrich_with_context("badha do", &ctx);
    // "badha do" means "increase" in Hindi — should carry Power domain anchor
    // Power anchor: "shutdown reboot sleep or lock the computer"
    assert!(enriched.text.contains("shutdown") || enriched.text.contains("context"));
}

#[test]
fn ctx14_context_records_embedding() {
    let mut ctx = RoutingContext::default();
    let embedding = vec![0.1, 0.2, 0.3, 0.4, 0.5];
    ctx.record_turn(
        Domain::SystemInfo,
        None,
        IntentModality::Read,
        embedding.clone(),
    );

    assert_eq!(ctx.last_embedding, Some(embedding));
}

#[test]
fn ctx15_context_clears_correction_after_record() {
    let mut ctx = RoutingContext::default();
    ctx.record_turn(
        Domain::SystemInfo,
        None,
        IntentModality::Read,
        vec![0.1; 10],
    );
    ctx.set_correction_pending();
    assert!(ctx.correction_pending);

    // Recording a turn clears the correction flag
    ctx.record_turn(
        Domain::SystemInfo,
        None,
        IntentModality::Read,
        vec![0.1; 10],
    );
    assert!(!ctx.correction_pending);
}
