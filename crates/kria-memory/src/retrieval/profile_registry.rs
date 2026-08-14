//! Profile registry and online-mutation prohibition (design §6.3, task F3.4.6).
//!
//! Enforces that:
//! * Profile weights are immutable compile-time constants.
//! * No online/user-request feedback path may mutate profile weights.
//! * Unknown or stale profile IDs fall back to the approved profile.
//! * Partially available strategies contribute 0 — weight NOT redistributed.

use crate::retrieval::{
    classifier::QueryClassV2,
    rrf_profile::{
        FusionProfile, PROFILE_ACTIVE_GOAL, PROFILE_ENTITY_RELATION, PROFILE_EXACT_PHRASE,
        PROFILE_EXPLORATORY, PROFILE_IDENTIFIER, PROFILE_TEMPORAL,
    },
};

// ── ApprovedProfile ───────────────────────────────────────────────────────────

/// The currently approved (active) profile for a query class.
///
/// Only the offline activation gate (`profile_eval::check_activation`) may
/// advance the approved profile; no runtime/feedback path may do so.
#[derive(Debug, Clone)]
pub struct ApprovedProfile {
    /// Profile ID of the currently approved profile.
    pub profile_id: &'static str,
    /// The activation evidence ID that authorized this profile (opaque string,
    /// may be "builtin" for the initial v1 profiles).
    pub activation_evidence_id: String,
}

// ── ProfileRegistry ───────────────────────────────────────────────────────────

/// A read-only registry of approved profiles.
///
/// The registry is constructed at startup from compile-time constants.
/// There is no method for updating weights at runtime.
pub struct ProfileRegistry {
    // (no mutable state; uses const profiles from rrf_profile)
}

impl ProfileRegistry {
    /// Create the registry with the built-in v1 approved profiles.
    pub fn new_v1() -> Self {
        Self {}
    }

    /// Get the approved profile for a given query class.
    /// Always succeeds — v1 profiles cover all classes.
    pub fn get_approved(&self, class: &QueryClassV2) -> &'static FusionProfile {
        match class {
            QueryClassV2::Identifier => &PROFILE_IDENTIFIER,
            QueryClassV2::ExactPhrase => &PROFILE_EXACT_PHRASE,
            QueryClassV2::EntityRelation => &PROFILE_ENTITY_RELATION,
            QueryClassV2::Temporal => &PROFILE_TEMPORAL,
            QueryClassV2::ActiveGoal => &PROFILE_ACTIVE_GOAL,
            QueryClassV2::Exploratory => &PROFILE_EXPLORATORY,
        }
    }

    /// Attempt to look up a profile by ID string. Returns `None` when the ID is
    /// unknown or stale (not in the current approved registry).
    pub fn get_by_id(&self, profile_id: &str) -> Option<&'static FusionProfile> {
        match profile_id {
            "rrf-id-v1" => Some(&PROFILE_IDENTIFIER),
            "rrf-exact-v1" => Some(&PROFILE_EXACT_PHRASE),
            "rrf-graph-v1" => Some(&PROFILE_ENTITY_RELATION),
            "rrf-time-v1" => Some(&PROFILE_TEMPORAL),
            "rrf-goal-v1" => Some(&PROFILE_ACTIVE_GOAL),
            "rrf-general-v1" => Some(&PROFILE_EXPLORATORY),
            _ => None,
        }
    }

    /// Resolve a profile by ID with fallback to the approved profile for a class.
    ///
    /// When `profile_id` is unknown or stale, logs a fallback reason and returns
    /// the currently approved profile for `class` — NEVER panics, NEVER mutates.
    pub fn resolve_with_fallback(
        &self,
        profile_id: &str,
        class: &QueryClassV2,
    ) -> (&'static FusionProfile, FallbackReason) {
        match self.get_by_id(profile_id) {
            Some(profile) => (profile, FallbackReason::NoFallback),
            None => {
                let approved = self.get_approved(class);
                (
                    approved,
                    FallbackReason::UnknownProfileId(profile_id.to_string()),
                )
            }
        }
    }

    /// Attempt to apply online feedback to update weights.
    ///
    /// This method ALWAYS returns an error — online feedback is strictly prohibited
    /// from mutating weights. The only path to changed weights is the offline
    /// activation gate.
    pub fn apply_online_feedback(&self, _feedback: &OnlineFeedback) -> OnlineMutationError {
        OnlineMutationError {
            message: "online feedback is not permitted to mutate profile weights; use the offline activation gate".to_string(),
        }
    }
}

// ── FallbackReason ────────────────────────────────────────────────────────────

/// Reason why a profile fallback occurred.
#[derive(Debug, Clone, PartialEq)]
pub enum FallbackReason {
    /// Profile ID was recognized and matched — no fallback needed.
    NoFallback,
    /// Profile ID was not found in the approved registry.
    UnknownProfileId(String),
    /// Profile ID was recognized but is stale (superseded by a newer approved profile).
    StaleProfileId(String),
}

// ── OnlineFeedback ────────────────────────────────────────────────────────────

/// Feedback signal from a user request or runtime event.
///
/// This type exists only to represent the data that is forbidden from causing
/// online weight mutations. It is accepted by `apply_online_feedback` solely
/// to return an error.
#[derive(Debug, Clone)]
pub struct OnlineFeedback {
    pub profile_id: String,
    pub query_class: String,
    pub feedback_kind: String,
}

// ── OnlineMutationError ───────────────────────────────────────────────────────

/// Error returned when attempting online feedback weight mutation.
#[derive(Debug, Clone, PartialEq)]
pub struct OnlineMutationError {
    pub message: String,
}

impl std::fmt::Display for OnlineMutationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for OnlineMutationError {}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retrieval::{
        rrf_fusion::{
            fuse_candidates, StrategyAvailability, StrategyCandidate, StrategyInput, StrategyKind,
        },
        rrf_profile::{
            DEFAULT_RRF_K, PROFILE_IDENTIFIER,
        },
    };

    // ── Registry construction ─────────────────────────────────────────────────

    #[test]
    fn new_v1_registry_has_all_classes() {
        let reg = ProfileRegistry::new_v1();
        // get_approved must return a non-null (valid) profile for all 6 classes
        let classes = [
            QueryClassV2::Identifier,
            QueryClassV2::ExactPhrase,
            QueryClassV2::EntityRelation,
            QueryClassV2::Temporal,
            QueryClassV2::ActiveGoal,
            QueryClassV2::Exploratory,
        ];
        for class in &classes {
            let profile = reg.get_approved(class);
            // Profile ID must be non-empty (sanity: a valid profile was returned)
            assert!(
                !profile.profile_id.is_empty(),
                "get_approved returned profile with empty id for class {:?}",
                class
            );
        }
    }

    #[test]
    fn get_approved_returns_correct_profile_id() {
        let reg = ProfileRegistry::new_v1();
        assert_eq!(
            reg.get_approved(&QueryClassV2::Identifier).profile_id,
            "rrf-id-v1"
        );
        assert_eq!(
            reg.get_approved(&QueryClassV2::ExactPhrase).profile_id,
            "rrf-exact-v1"
        );
        assert_eq!(
            reg.get_approved(&QueryClassV2::EntityRelation).profile_id,
            "rrf-graph-v1"
        );
        assert_eq!(
            reg.get_approved(&QueryClassV2::Temporal).profile_id,
            "rrf-time-v1"
        );
        assert_eq!(
            reg.get_approved(&QueryClassV2::ActiveGoal).profile_id,
            "rrf-goal-v1"
        );
        assert_eq!(
            reg.get_approved(&QueryClassV2::Exploratory).profile_id,
            "rrf-general-v1"
        );
    }

    // ── get_by_id ─────────────────────────────────────────────────────────────

    #[test]
    fn get_by_id_known_profiles_are_found() {
        let reg = ProfileRegistry::new_v1();
        assert!(reg.get_by_id("rrf-id-v1").is_some(), "rrf-id-v1 not found");
        assert!(
            reg.get_by_id("rrf-exact-v1").is_some(),
            "rrf-exact-v1 not found"
        );
        assert!(
            reg.get_by_id("rrf-graph-v1").is_some(),
            "rrf-graph-v1 not found"
        );
        assert!(
            reg.get_by_id("rrf-time-v1").is_some(),
            "rrf-time-v1 not found"
        );
        assert!(
            reg.get_by_id("rrf-goal-v1").is_some(),
            "rrf-goal-v1 not found"
        );
        assert!(
            reg.get_by_id("rrf-general-v1").is_some(),
            "rrf-general-v1 not found"
        );
    }

    #[test]
    fn get_by_id_unknown_id_returns_none() {
        let reg = ProfileRegistry::new_v1();
        assert!(reg.get_by_id("rrf-unknown-v99").is_none());
    }

    #[test]
    fn get_by_id_stale_id_returns_none() {
        let reg = ProfileRegistry::new_v1();
        // "rrf-id-v0" would be a plausible old/stale ID not in the current registry
        assert!(reg.get_by_id("rrf-id-v0").is_none());
    }

    // ── resolve_with_fallback ─────────────────────────────────────────────────

    #[test]
    fn resolve_with_fallback_known_id_no_fallback() {
        let reg = ProfileRegistry::new_v1();
        let (profile, reason) = reg.resolve_with_fallback("rrf-id-v1", &QueryClassV2::Identifier);
        assert_eq!(profile.profile_id, "rrf-id-v1");
        assert_eq!(reason, FallbackReason::NoFallback);
    }

    #[test]
    fn resolve_with_fallback_unknown_id_falls_back_to_approved() {
        let reg = ProfileRegistry::new_v1();
        let (profile, reason) =
            reg.resolve_with_fallback("rrf-unknown-v99", &QueryClassV2::Exploratory);
        // Must fall back to Exploratory's approved profile
        assert_eq!(profile.profile_id, "rrf-general-v1");
        assert_eq!(
            reason,
            FallbackReason::UnknownProfileId("rrf-unknown-v99".to_string())
        );
    }

    #[test]
    fn resolve_with_fallback_unknown_id_returns_class_approved_not_arbitrary() {
        let reg = ProfileRegistry::new_v1();
        // Unknown ID + Temporal class → must return PROFILE_TEMPORAL specifically
        let (profile, reason) =
            reg.resolve_with_fallback("rrf-unknown-v99", &QueryClassV2::Temporal);
        assert_eq!(
            profile.profile_id, "rrf-time-v1",
            "fallback must return the class-specific approved profile, not an arbitrary one"
        );
        assert_eq!(
            reason,
            FallbackReason::UnknownProfileId("rrf-unknown-v99".to_string())
        );
    }

    // ── online mutation prohibition ───────────────────────────────────────────

    #[test]
    fn online_feedback_always_returns_error() {
        let reg = ProfileRegistry::new_v1();
        let feedback = OnlineFeedback {
            profile_id: "rrf-id-v1".to_string(),
            query_class: "identifier".to_string(),
            feedback_kind: "thumbs_up".to_string(),
        };
        let err = reg.apply_online_feedback(&feedback);
        // The result is always an error regardless of the feedback content
        assert!(!err.message.is_empty());
    }

    #[test]
    fn online_mutation_error_message_mentions_offline_gate() {
        let reg = ProfileRegistry::new_v1();
        let feedback = OnlineFeedback {
            profile_id: "rrf-general-v1".to_string(),
            query_class: "exploratory".to_string(),
            feedback_kind: "click".to_string(),
        };
        let err = reg.apply_online_feedback(&feedback);
        assert!(
            err.message.contains("offline activation gate"),
            "error message must mention 'offline activation gate', got: {}",
            err.message
        );
    }

    // ── immutability / compile-time constants ─────────────────────────────────

    #[test]
    fn profile_weights_are_immutable_at_compile_time() {
        // Documentation test: verify the compile-time constant hasn't been altered.
        // PROFILE_IDENTIFIER.weights.fts is defined as 2.0 in rrf_profile.rs.
        assert_eq!(
            PROFILE_IDENTIFIER.weights.fts, 2.0,
            "PROFILE_IDENTIFIER.weights.fts must remain 2.0 (compile-time constant)"
        );
    }

    // ── partial availability / weight non-redistribution ─────────────────────

    #[test]
    fn partial_availability_zero_contribution_not_redistributed() {
        // Use rrf_fusion::fuse_candidates with one strategy Unavailable.
        // Verify: available strategy score == w/(k+1), NOT == (w1+w2)/(k+1).
        // This confirms weight non-redistribution at the integration level.
        let profile = &PROFILE_IDENTIFIER;
        // FTS available at rank 1, Vector unavailable
        let strategies = vec![
            StrategyInput {
                strategy: StrategyKind::Fts,
                availability: StrategyAvailability::Available,
                candidates: vec![StrategyCandidate {
                    semantic_id: "id-a".to_string(),
                    content_version: "v1".to_string(),
                    rank: 1,
                }],
            },
            StrategyInput {
                strategy: StrategyKind::Vector,
                availability: StrategyAvailability::Unavailable,
                candidates: vec![],
            },
        ];
        let results = fuse_candidates(&strategies, profile).unwrap();
        assert_eq!(results.len(), 1);

        // The FTS contribution must exactly equal fts_weight / (k + 1)
        let expected_fts = profile.weights.fts / (DEFAULT_RRF_K + 1.0);
        // The redistributed (incorrect) value would be (fts_w + vec_w) / (k + 1)
        let redistributed_score =
            (profile.weights.fts + profile.weights.vector) / (DEFAULT_RRF_K + 1.0);

        assert!(
            (results[0].rrf_score - expected_fts).abs() < 1e-6,
            "score should be {expected_fts} (no redistribution), got {}",
            results[0].rrf_score
        );
        assert_ne!(
            results[0].rrf_score, redistributed_score,
            "score must NOT equal redistributed value {redistributed_score}"
        );
        // Vector contribution must be exactly 0
        assert_eq!(results[0].contributions.vector, 0.0);
    }

    // ── stale ID fallback ─────────────────────────────────────────────────────

    #[test]
    fn stale_profile_id_fallback_returns_approved_not_none() {
        // "rrf-id-v0" is a plausible stale ID — must fall back to Identifier's approved
        let reg = ProfileRegistry::new_v1();
        let (profile, reason) = reg.resolve_with_fallback("rrf-id-v0", &QueryClassV2::Identifier);
        // Must NOT panic or return None — falls back to class-approved profile
        assert_eq!(
            profile.profile_id, "rrf-id-v1",
            "stale ID must fall back to Identifier's approved profile"
        );
        assert_eq!(
            reason,
            FallbackReason::UnknownProfileId("rrf-id-v0".to_string()),
            "stale ID must produce UnknownProfileId fallback reason"
        );
    }

    #[test]
    fn all_known_profile_ids_resolve_without_fallback() {
        let reg = ProfileRegistry::new_v1();
        let known_ids = [
            ("rrf-id-v1", QueryClassV2::Identifier),
            ("rrf-exact-v1", QueryClassV2::ExactPhrase),
            ("rrf-graph-v1", QueryClassV2::EntityRelation),
            ("rrf-time-v1", QueryClassV2::Temporal),
            ("rrf-goal-v1", QueryClassV2::ActiveGoal),
            ("rrf-general-v1", QueryClassV2::Exploratory),
        ];
        for (id, class) in &known_ids {
            let (_, reason) = reg.resolve_with_fallback(id, class);
            assert_eq!(
                reason,
                FallbackReason::NoFallback,
                "known profile ID '{}' must resolve without fallback",
                id
            );
        }
    }
}
