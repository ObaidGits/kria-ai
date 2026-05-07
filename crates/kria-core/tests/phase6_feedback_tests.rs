//! Phase 5: Online Learning Feedback Loop Tests
//!
//! Tests for the feedback collection and centroid adjustment system.

use std::collections::HashMap;

use kria_core::routing::domain::Domain;
use kria_core::routing::feedback::{
    adjust_centroids, detect_outcome, FeedbackCollector, RoutingFeedback, RoutingOutcome,
};

// ─── Feedback Collector Tests ───────────────────────────────────────────────

#[test]
fn fb01_record_feedback() {
    let mut collector = FeedbackCollector::default_config();
    let feedback = RoutingFeedback {
        input_text_hash: 12345,
        domain_selected: Domain::SystemInfo,
        tool_selected: Some("check_health".into()),
        intent_source: "FastEmbedSemanticRouter".into(),
        confidence: 0.85,
        outcome: RoutingOutcome::Success,
        timestamp: 1000000,
        session_id: "test".into(),
        embedding: vec![0.1; 10],
    };
    collector.record(feedback);
    assert_eq!(collector.buffer_len(), 1);
}

#[test]
fn fb02_flush_at_capacity() {
    let dir = std::env::temp_dir().join("kria_feedback_test_flush");
    let mut collector = FeedbackCollector::new(dir.to_str().unwrap(), 3, 0.01);
    for i in 0..6 {
        collector.record(RoutingFeedback {
            input_text_hash: i,
            domain_selected: Domain::SystemInfo,
            tool_selected: None,
            intent_source: "test".into(),
            confidence: 0.9,
            outcome: RoutingOutcome::Success,
            timestamp: 1000,
            session_id: "test".into(),
            embedding: vec![],
        });
    }
    // After 6 records with capacity 3: flushed at 3, then 3 more, then flushed again
    // Buffer should be empty after flush
    assert_eq!(collector.buffer_len(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fb03_flush_creates_file() {
    let dir = std::env::temp_dir().join("kria_feedback_test_file");
    let mut collector = FeedbackCollector::new(dir.to_str().unwrap(), 100, 0.01);
    collector.record(RoutingFeedback {
        input_text_hash: 1,
        domain_selected: Domain::SystemInfo,
        tool_selected: None,
        intent_source: "test".into(),
        confidence: 0.9,
        outcome: RoutingOutcome::Success,
        timestamp: 1000,
        session_id: "test".into(),
        embedding: vec![],
    });
    collector.flush_to_disk();

    let history = collector.load_history();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].domain_selected, Domain::SystemInfo);
    let _ = std::fs::remove_dir_all(&dir);
}

// ─── Centroid Adjustment Tests ──────────────────────────────────────────────

#[test]
fn fb04_success_nudge() {
    let mut centroids = HashMap::new();
    centroids.insert(Domain::SystemInfo, vec![1.0, 0.0, 0.0]);
    let original = centroids[&Domain::SystemInfo].clone();

    let feedback = vec![RoutingFeedback {
        input_text_hash: 1,
        domain_selected: Domain::SystemInfo,
        tool_selected: None,
        intent_source: "test".into(),
        confidence: 0.9,
        outcome: RoutingOutcome::Success,
        timestamp: 1000,
        session_id: "test".into(),
        embedding: vec![0.0, 1.0, 0.0],
    }];

    let report = adjust_centroids(&feedback, &mut centroids, 0.1);
    assert_eq!(report.success_nudges, 1);
    assert_ne!(centroids[&Domain::SystemInfo], original);
}

#[test]
fn fb05_correction_push_pull() {
    let mut centroids = HashMap::new();
    centroids.insert(Domain::SystemInfo, vec![1.0, 0.0, 0.0]);
    centroids.insert(Domain::Knowledge, vec![0.0, 1.0, 0.0]);

    let feedback = vec![RoutingFeedback {
        input_text_hash: 1,
        domain_selected: Domain::SystemInfo,
        tool_selected: None,
        intent_source: "test".into(),
        confidence: 0.9,
        outcome: RoutingOutcome::Corrected {
            correct_domain: Domain::Knowledge,
            correct_tool: None,
        },
        timestamp: 1000,
        session_id: "test".into(),
        embedding: vec![0.5, 0.5, 0.0],
    }];

    let report = adjust_centroids(&feedback, &mut centroids, 0.1);
    assert_eq!(report.correction_pushes, 1);
    assert_eq!(report.correction_pulls, 1);
}

#[test]
fn fb06_normalization_preserved() {
    let mut centroids = HashMap::new();
    centroids.insert(Domain::SystemInfo, vec![1.0, 0.0, 0.0]);

    // Apply many adjustments
    for i in 0..100 {
        let feedback = vec![RoutingFeedback {
            input_text_hash: i,
            domain_selected: Domain::SystemInfo,
            tool_selected: None,
            intent_source: "test".into(),
            confidence: 0.9,
            outcome: RoutingOutcome::Success,
            timestamp: 1000,
            session_id: "test".into(),
            embedding: vec![0.0, 1.0, 0.0],
        }];
        adjust_centroids(&feedback, &mut centroids, 0.01);
    }

    // Verify normalization is preserved
    let c = &centroids[&Domain::SystemInfo];
    let norm: f32 = c.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 0.001, "Norm should be ~1.0, got {}", norm);
}

// ─── Outcome Detection Tests ────────────────────────────────────────────────

#[test]
fn fb07_rephrase_detection() {
    let r = detect_outcome(
        Domain::SystemInfo,
        Some("check_health"),
        Some("do it again"),
        true,
        None,
    );
    assert!(matches!(r, RoutingOutcome::Rephrased));

    let r2 = detect_outcome(
        Domain::SystemInfo,
        Some("check_health"),
        Some("phir se karo"),
        true,
        None,
    );
    assert!(matches!(r2, RoutingOutcome::Rephrased));

    // Not a rephrase
    let r3 = detect_outcome(
        Domain::SystemInfo,
        Some("check_health"),
        Some("check memory too"),
        true,
        None,
    );
    assert!(matches!(r3, RoutingOutcome::Success));
}

#[test]
fn fb08_correction_detection() {
    let r = detect_outcome(
        Domain::SystemInfo,
        Some("check_health"),
        Some("no i meant the network"),
        true,
        None,
    );
    assert!(matches!(r, RoutingOutcome::Corrected { .. }));
}

#[test]
fn fb09_tool_error_detection() {
    let r = detect_outcome(
        Domain::SystemInfo,
        Some("check_health"),
        None,
        false,
        Some("permission denied".into()),
    );
    match r {
        RoutingOutcome::ToolError { error } => assert_eq!(error, "permission denied"),
        _ => panic!("Expected ToolError"),
    }
}

#[test]
fn fb10_success_detection() {
    let r = detect_outcome(
        Domain::SystemInfo,
        Some("check_health"),
        Some("what about memory"),
        true,
        None,
    );
    assert!(matches!(r, RoutingOutcome::Success));
}

// ─── BargedIn Detection Tests ───────────────────────────────────────────────

#[test]
fn fb11_barged_in_not_detected_by_default() {
    // BargedIn detection is not implemented via text analysis
    // It requires audio/pipeline signals, so we just verify
    // the enum variant exists and can be created
    let outcome = RoutingOutcome::BargedIn;
    assert!(matches!(outcome, RoutingOutcome::BargedIn));
}

// ─── Empty Feedback Tests ───────────────────────────────────────────────────

#[test]
fn fb12_empty_feedback_no_adjustment() {
    let mut centroids = HashMap::new();
    centroids.insert(Domain::SystemInfo, vec![1.0, 0.0, 0.0]);
    let original = centroids.clone();

    let report = adjust_centroids(&[], &mut centroids, 0.1);
    assert_eq!(report.total_adjusted, 0);
    assert_eq!(centroids, original);
}

#[test]
fn fb13_empty_embedding_skipped() {
    let mut centroids = HashMap::new();
    centroids.insert(Domain::SystemInfo, vec![1.0, 0.0, 0.0]);
    let original = centroids.clone();

    let feedback = vec![RoutingFeedback {
        input_text_hash: 1,
        domain_selected: Domain::SystemInfo,
        tool_selected: None,
        intent_source: "test".into(),
        confidence: 0.9,
        outcome: RoutingOutcome::Success,
        timestamp: 1000,
        session_id: "test".into(),
        embedding: vec![], // Empty embedding
    }];

    let report = adjust_centroids(&feedback, &mut centroids, 0.1);
    assert_eq!(report.total_adjusted, 0);
    assert_eq!(centroids, original);
}

// ─── Multi-Domain Adjustment Tests ──────────────────────────────────────────

#[test]
fn fb14_barged_in_weak_negative() {
    let mut centroids = HashMap::new();
    centroids.insert(Domain::SystemInfo, vec![1.0, 0.0, 0.0]);
    let original = centroids[&Domain::SystemInfo].clone();

    let feedback = vec![RoutingFeedback {
        input_text_hash: 1,
        domain_selected: Domain::SystemInfo,
        tool_selected: None,
        intent_source: "test".into(),
        confidence: 0.9,
        outcome: RoutingOutcome::BargedIn,
        timestamp: 1000,
        session_id: "test".into(),
        embedding: vec![0.0, 1.0, 0.0],
    }];

    let report = adjust_centroids(&feedback, &mut centroids, 0.1);
    assert_eq!(report.total_adjusted, 1);
    // BargedIn uses -0.3x learning rate, so centroid should move slightly
    assert_ne!(centroids[&Domain::SystemInfo], original);
}

#[test]
fn fb15_rephrase_weak_negative() {
    let mut centroids = HashMap::new();
    centroids.insert(Domain::SystemInfo, vec![1.0, 0.0, 0.0]);
    let original = centroids[&Domain::SystemInfo].clone();

    let feedback = vec![RoutingFeedback {
        input_text_hash: 1,
        domain_selected: Domain::SystemInfo,
        tool_selected: None,
        intent_source: "test".into(),
        confidence: 0.9,
        outcome: RoutingOutcome::Rephrased,
        timestamp: 1000,
        session_id: "test".into(),
        embedding: vec![0.0, 1.0, 0.0],
    }];

    let report = adjust_centroids(&feedback, &mut centroids, 0.1);
    assert_eq!(report.rephrase_pushes, 1);
    assert_ne!(centroids[&Domain::SystemInfo], original);
}

// ─── Config Integration Tests ───────────────────────────────────────────────

#[test]
fn fb16_config_has_feedback_fields() {
    let config = kria_core::config::RoutingConfig::default();
    assert!(config.feedback_enabled);
    assert_eq!(config.feedback_learning_rate, 0.01);
    assert_eq!(config.feedback_max_buffer, 1000);
}
