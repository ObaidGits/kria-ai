//! Phase 4: Speculative Pre-Warming Tests
//!
//! Tests for the speculative routing system that pre-acquires resources
//! on partial voice transcripts to reduce perceived latency.

use std::time::{Duration, Instant};

use kria_core::routing::domain::Domain;
use kria_core::routing::speculative::{
    SpeculativeAction, SpeculativeResult, SpeculativeRouter, SpeculativeState,
};
use kria_core::routing::tool_index::ToolMatch;

// ─── Basic Speculation Tests ────────────────────────────────────────────────

#[test]
fn sp01_default_thresholds() {
    let router = SpeculativeRouter::new();
    assert_eq!(router.min_confidence, 0.7);
    assert_eq!(router.min_tokens, 2);
}

#[test]
fn sp02_custom_thresholds() {
    let router = SpeculativeRouter::with_thresholds(0.8, 3);
    assert_eq!(router.min_confidence, 0.8);
    assert_eq!(router.min_tokens, 3);
}

#[test]
fn sp03_low_confidence_waits() {
    let mut router = SpeculativeRouter::new();
    let action = router.on_partial("set volume", 0.3);
    assert_eq!(action, SpeculativeAction::Wait);
}

#[test]
fn sp04_short_text_waits() {
    let mut router = SpeculativeRouter::new();
    let action = router.on_partial("set", 0.9);
    assert_eq!(action, SpeculativeAction::Wait);
}

#[test]
fn sp05_no_active_by_default() {
    let router = SpeculativeRouter::new();
    assert!(!router.has_active());
    assert!(router.active().is_none());
}

// ─── Cancel Tests ───────────────────────────────────────────────────────────

#[test]
fn sp06_cancel_clears_active() {
    let mut router = SpeculativeRouter::new();
    router.active = Some(SpeculativeState {
        predicted_domain: Domain::SystemInfo,
        predicted_tool: None,
        confidence: 0.9,
        started_at: Instant::now(),
        partial_text: "check system".into(),
    });
    assert!(router.has_active());
    router.cancel();
    assert!(!router.has_active());
}

#[test]
fn sp07_cancel_no_op_when_empty() {
    let mut router = SpeculativeRouter::new();
    router.cancel(); // Should not panic
    assert!(!router.has_active());
}

// ─── On Final Tests ─────────────────────────────────────────────────────────

#[test]
fn sp08_on_final_no_speculation() {
    let mut router = SpeculativeRouter::new();
    let result = router.on_final("check system health", Domain::SystemInfo, None);
    assert!(matches!(result, SpeculativeResult::NoSpeculation));
}

#[test]
fn sp09_on_final_expired_treats_as_miss() {
    let mut router = SpeculativeRouter::new();
    router.active = Some(SpeculativeState {
        predicted_domain: Domain::SystemInfo,
        predicted_tool: None,
        confidence: 0.9,
        started_at: Instant::now() - Duration::from_secs(10),
        partial_text: "check".into(),
    });
    let result = router.on_final("check system health", Domain::SystemInfo, None);
    assert!(matches!(result, SpeculativeResult::Miss { .. }));
}

// ─── State Matching Tests ───────────────────────────────────────────────────

#[test]
fn sp10_state_matches_domain() {
    let state = SpeculativeState {
        predicted_domain: Domain::SystemInfo,
        predicted_tool: None,
        confidence: 0.9,
        started_at: Instant::now(),
        partial_text: "check system".into(),
    };
    assert!(state.matches(Domain::SystemInfo, None));
    assert!(!state.matches(Domain::FileOps, None));
}

#[test]
fn sp11_state_matches_tool() {
    let state = SpeculativeState {
        predicted_domain: Domain::Power,
        predicted_tool: Some(ToolMatch {
            name: "set_volume".into(),
            description: "Set volume".into(),
            category: "power".into(),
            confidence: 0.9,
            direct_execution: true,
        }),
        confidence: 0.9,
        started_at: Instant::now(),
        partial_text: "set volume".into(),
    };
    assert!(state.matches(Domain::Power, Some("set_volume")));
    assert!(!state.matches(Domain::Power, Some("set_brightness")));
    assert!(!state.matches(Domain::SystemInfo, Some("set_volume")));
}

#[test]
fn sp12_state_not_expired() {
    let state = SpeculativeState {
        predicted_domain: Domain::SystemInfo,
        predicted_tool: None,
        confidence: 0.9,
        started_at: Instant::now(),
        partial_text: "check".into(),
    };
    assert!(!state.is_expired());
}

#[test]
fn sp13_state_expired() {
    let state = SpeculativeState {
        predicted_domain: Domain::SystemInfo,
        predicted_tool: None,
        confidence: 0.9,
        started_at: Instant::now() - Duration::from_secs(10),
        partial_text: "check".into(),
    };
    assert!(state.is_expired());
}

// ─── Default Impl Tests ─────────────────────────────────────────────────────

#[test]
fn sp14_default_impl() {
    let router = SpeculativeRouter::default();
    assert_eq!(router.min_confidence, 0.7);
    assert_eq!(router.min_tokens, 2);
}

// ─── Repeated Partial Tests ─────────────────────────────────────────────────

#[test]
fn sp15_same_partial_doesnt_re_speculate() {
    let mut router = SpeculativeRouter::new();
    router.active = Some(SpeculativeState {
        predicted_domain: Domain::SystemInfo,
        predicted_tool: None,
        confidence: 0.9,
        started_at: Instant::now(),
        partial_text: "check system".into(),
    });
    // Same text should return Speculating without re-embedding
    // (only if embedding model is ready; otherwise Wait is expected)
    let action = router.on_partial("check system", 0.9);
    if kria_core::routing::embed::is_ready() {
        assert_eq!(action, SpeculativeAction::Speculating);
    } else {
        // Without embedding model, same-text optimization still short-circuits
        // but predict_domain returns None, so we get Wait
        assert_eq!(action, SpeculativeAction::Wait);
    }
}

// ─── Action Enum Tests ──────────────────────────────────────────────────────

#[test]
fn sp16_action_equality() {
    assert_eq!(SpeculativeAction::Wait, SpeculativeAction::Wait);
    assert_eq!(
        SpeculativeAction::Speculating,
        SpeculativeAction::Speculating
    );
    assert_ne!(SpeculativeAction::Wait, SpeculativeAction::Speculating);
}

#[test]
fn sp17_result_debug() {
    let result = SpeculativeResult::NoSpeculation;
    let debug = format!("{:?}", result);
    assert!(debug.contains("NoSpeculation"));
}

// ─── Latency Budget Tests ───────────────────────────────────────────────────

#[test]
fn sp18_state_creation_is_fast() {
    let start = Instant::now();
    for _ in 0..1000 {
        let _state = SpeculativeState {
            predicted_domain: Domain::SystemInfo,
            predicted_tool: None,
            confidence: 0.9,
            started_at: Instant::now(),
            partial_text: "check system health".into(),
        };
    }
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_millis(10)); // 1000 states < 10ms
}
