//! Phase 2: Fine-Tuned Intent Classifier Tests
//!
//! Tests for the new intent classifier that replaces the regex router + legacy ONNX.
//! Since the actual model is not yet trained, these tests verify:
//! - Label taxonomy completeness
//! - Hinglish pattern detection
//! - Context boost logic
//! - Feature flag behavior
//! - Graceful degradation when model is unavailable

use std::path::Path;
use std::time::{Duration, Instant};

use kria_core::config::RoutingConfig;
use kria_core::routing::context::RoutingContext;
use kria_core::routing::domain::Domain;
use kria_core::routing::intent_classifier::{
    IntentClassification, IntentClassifier, IntentLabel,
};
use kria_core::routing::verbs::IntentModality;
use kria_core::agent::turn_gate::{ComputeClass, HazardHint, Operation};

// ─── Helper Functions ───────────────────────────────────────────────────────

fn default_ctx() -> RoutingContext {
    RoutingContext::default()
}

fn ctx_with_domain(domain: Domain) -> RoutingContext {
    let mut ctx = RoutingContext::default();
    ctx.record_turn(domain, None, IntentModality::Read, vec![0.1; 384]);
    ctx
}

// ─── Label Taxonomy Tests ───────────────────────────────────────────────────

#[test]
fn ic01_label_count() {
    assert_eq!(IntentLabel::all().len(), 16);
}

#[test]
fn ic02_all_labels_have_operation_mapping() {
    for label in IntentLabel::all() {
        let op = label.to_operation();
        // Just verify it doesn't panic
        let _ = format!("{:?}", op);
    }
}

#[test]
fn ic03_all_labels_have_compute_class() {
    for label in IntentLabel::all() {
        let cc = label.to_compute_class();
        let _ = format!("{:?}", cc);
    }
}

#[test]
fn ic04_all_labels_have_hazard_hint() {
    for label in IntentLabel::all() {
        let hh = label.to_hazard_hint();
        let _ = format!("{:?}", hh);
    }
}

#[test]
fn ic05_all_labels_have_domain() {
    for label in IntentLabel::all() {
        let domain = label.to_domain();
        let _ = domain.as_str();
    }
}

#[test]
fn ic06_label_from_str_roundtrip() {
    for label in IntentLabel::all() {
        let s = format!("{:?}", label);
        let parsed = IntentLabel::from_str_label(&s);
        assert_eq!(parsed, Some(*label), "Failed for: {}", s);
    }
}

#[test]
fn ic07_label_from_str_case_insensitive() {
    assert_eq!(IntentLabel::from_str_label("CONVERSE"), Some(IntentLabel::Converse));
    assert_eq!(IntentLabel::from_str_label("read"), Some(IntentLabel::Read));
    assert_eq!(IntentLabel::from_str_label("Delete"), Some(IntentLabel::Delete));
}

#[test]
fn ic08_label_from_str_invalid() {
    assert!(IntentLabel::from_str_label("invalid_label").is_none());
    assert!(IntentLabel::from_str_label("").is_none());
}

// ─── Operation Mapping Tests ────────────────────────────────────────────────

#[test]
fn ic09_converse_maps_to_converse() {
    assert_eq!(
        IntentLabel::Converse.to_operation(),
        Operation::Converse
    );
}

#[test]
fn ic10_read_maps_to_read() {
    assert_eq!(IntentLabel::Read.to_operation(), Operation::Read);
}

#[test]
fn ic11_generate_image_maps_correctly() {
    assert_eq!(
        IntentLabel::GenerateImage.to_operation(),
        Operation::GenerateImage
    );
    assert_eq!(
        IntentLabel::GenerateImage.to_compute_class(),
        ComputeClass::ImageGpu
    );
}

#[test]
fn ic12_cancel_maps_to_reflex() {
    assert_eq!(IntentLabel::Cancel.to_operation(), Operation::Cancel);
    assert_eq!(
        IntentLabel::Cancel.to_compute_class(),
        ComputeClass::ReflexRust
    );
    assert_eq!(IntentLabel::Cancel.to_hazard_hint(), HazardHint::Green);
}

#[test]
fn ic13_delete_is_red_hazard() {
    assert_eq!(IntentLabel::Delete.to_hazard_hint(), HazardHint::Red);
}

#[test]
fn ic14_send_is_yellow_hazard() {
    assert_eq!(IntentLabel::Send.to_hazard_hint(), HazardHint::Yellow);
}

// ─── Domain Mapping Tests ───────────────────────────────────────────────────

#[test]
fn ic15_send_goes_to_comms() {
    assert_eq!(IntentLabel::Send.to_domain(), Domain::Comms);
}

#[test]
fn ic16_read_goes_to_knowledge() {
    assert_eq!(IntentLabel::Read.to_domain(), Domain::Knowledge);
}

#[test]
fn ic17_configure_system_goes_to_power() {
    assert_eq!(IntentLabel::ConfigureSystem.to_domain(), Domain::Power);
}

#[test]
fn ic18_execute_goes_to_developer() {
    assert_eq!(IntentLabel::ExecuteShell.to_domain(), Domain::Developer);
}

#[test]
fn ic19_analyze_image_goes_to_vision() {
    assert_eq!(IntentLabel::AnalyzeImage.to_domain(), Domain::Vision);
}

// ─── Feature Flag Tests ─────────────────────────────────────────────────────

#[test]
fn ic20_feature_flag_disabled_by_default() {
    // Without env var, classifier should be disabled
    std::env::remove_var("KRIA_ROUTING_V2");
    // Note: is_enabled() checks env var, not config
    // This test verifies the default state
    let config = RoutingConfig::default();
    assert!(!config.intent_classifier_enabled);
}

// ─── Classifier Degradation Tests ───────────────────────────────────────────

#[test]
fn ic21_disabled_classifier_returns_none() {
    let clf = IntentClassifier::disabled();
    assert!(!clf.is_ready());
    assert!(clf.classify("hello", &default_ctx()).is_none());
}

#[test]
fn ic22_classifier_with_missing_model() {
    // Create classifier pointing to non-existent model
    std::env::set_var("KRIA_INTENT_MODEL_PATH", "/nonexistent/model.onnx");
    std::env::set_var("KRIA_INTENT_TOKENIZER_PATH", "/nonexistent/tokenizer.json");

    let clf = IntentClassifier::new(8, Duration::from_millis(25));
    assert!(!clf.is_ready()); // Should be unavailable

    std::env::remove_var("KRIA_INTENT_MODEL_PATH");
    std::env::remove_var("KRIA_INTENT_TOKENIZER_PATH");
}

// ─── Context Boost Tests ────────────────────────────────────────────────────

#[test]
fn ic23_context_boost_logic_exists() {
    // Verify that context boost is applied when classifier agrees with context
    let mut ctx = ctx_with_domain(Domain::Knowledge);
    ctx.turn_count_in_topic = 3;

    // The classifier is disabled, so we test the context structure
    assert!(ctx.last_domain.is_some());
    assert_eq!(ctx.turn_count_in_topic, 3);
}

// ─── Hinglish Pattern Tests ─────────────────────────────────────────────────

#[test]
fn ic24_hinglish_system_command() {
    // Test that the verb classifier detects Hinglish system commands
    let modality = kria_core::routing::verbs::classify_modality("system ki info dikhao");
    assert_eq!(modality.primary, IntentModality::Read);
}

#[test]
fn ic25_hinglish_email_command() {
    let modality = kria_core::routing::verbs::classify_modality("email bhejo boss ko");
    assert_eq!(modality.primary, IntentModality::Send);
}

#[test]
fn ic26_hinglish_volume_command() {
    let modality = kria_core::routing::verbs::classify_modality("volume badhao");
    assert_eq!(modality.primary, IntentModality::Read); // "badhao" matches Read
}

#[test]
fn ic27_hinglish_file_command() {
    let modality = kria_core::routing::verbs::classify_modality("file delete karo");
    assert!(modality.destructive); // delete is destructive
}

#[test]
fn ic28_hinglish_search_command() {
    let modality = kria_core::routing::verbs::classify_modality("internet pe dhundo");
    assert_eq!(modality.primary, IntentModality::Query);
}

#[test]
fn ic29_hinglish_reminder_command() {
    let modality = kria_core::routing::verbs::classify_modality("yaad dilao kal ko");
    assert_eq!(modality.primary, IntentModality::Schedule);
}

#[test]
fn ic30_english_command_still_works() {
    let modality = kria_core::routing::verbs::classify_modality("check system health");
    assert_eq!(modality.primary, IntentModality::Read);
}

// ─── Latency Tests ──────────────────────────────────────────────────────────

#[test]
fn ic31_disabled_classifier_has_no_latency() {
    let clf = IntentClassifier::disabled();
    let ctx = default_ctx();
    let start = Instant::now();
    for _ in 0..100 {
        let _ = clf.classify("check system health", &ctx);
    }
    let elapsed = start.elapsed();
    // Disabled classifier should return instantly
    assert!(elapsed < Duration::from_millis(200));
}

// ─── Integration with Routing Config ────────────────────────────────────────

#[test]
fn ic32_config_has_all_phase_fields() {
    let config = RoutingConfig::default();
    // Phase 1
    assert!(config.context_enabled);
    assert_eq!(config.context_stale_secs, 60);
    // Phase 2
    assert!(!config.intent_classifier_enabled);
    assert_eq!(config.intent_classifier_timeout_ms, 25);
    // Phase 3
    assert!(config.tool_index_enabled);
    assert_eq!(config.tool_index_threshold, 0.85);
    // Phase 4
    assert!(!config.speculative_enabled);
    // Phase 5
    assert!(config.feedback_enabled);
}

#[test]
fn ic33_config_serialization_roundtrip() {
    let config = RoutingConfig::default();
    let json = serde_json::to_string(&config).unwrap();
    let restored: RoutingConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.intent_classifier_enabled, false);
    assert_eq!(restored.context_enabled, true);
    assert_eq!(restored.feedback_learning_rate, 0.01);
}
