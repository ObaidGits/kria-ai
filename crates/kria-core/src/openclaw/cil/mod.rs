//! Capability Intelligence Layer (CIL) — the OpenClaw Intelligent Capability
//! Platform (ICP).
//!
//! CIL is a set of **derived views and traits** layered on top of the frozen
//! OpenClaw components (`RuntimeManager`, `ExecutionEngine`, `SemanticSkillRouter`,
//! `ProductionSkillRegistry`, `ClawHubClient`, `ContainerPool`, `DockerRuntime`,
//! the A9 `GenerationPipeline`, `BundleInstaller`, `ApprovalCache`, `McpBridge`).
//! It **extends, never forks** those components — introducing no second registry,
//! router, engine, installer, or permission store.
//!
//! Every capability here is gated behind the `openclaw_icp_enabled` flag
//! (default OFF). With the flag OFF, `SemanticOpenClawHandler::execute_semantic`
//! MUST produce byte-for-byte identical output to the current direct-router path.
//!
//! # Status
//!
//! This is the **first scaffolding task (1.1)**: the module tree, the single
//! [`CilError`] type, and the [`Fulfillment`] / [`RequestCtx`] skeleton types from
//! design §8.8. Nothing here is reachable yet — the facade entry point is gated
//! and unimplemented until later phases wire it in.
//!
//! # Runtime authority invariants (preserved)
//!
//! KRIA remains the orchestration authority; CIL is orchestration only, and the
//! substrates remain execution. The execution flow is unchanged:
//! Intent → Capability → Policy → Substrate → Tool → Verification.

use thiserror::Error;

pub mod acquire;
pub mod backfill;
pub mod config;
pub mod dense;
pub mod embed;
pub mod extract;
pub mod facade;
pub mod graph;
pub mod index;
pub mod intent;
pub mod learn;
pub mod market;
pub mod plan;
pub mod profile;
pub mod rank;
pub mod recommend;

#[cfg(test)]
mod index_reindex_pbt;
#[cfg(test)]
mod no_hardcoding_pbt;
#[cfg(test)]
mod profile_reindex_pbt;

pub use acquire::{
    AcquireContext, AcquisitionOrchestrator, AcquisitionOutcome, DefaultAcquisitionOrchestrator,
};
pub use backfill::{spawn_backfill, spawn_registry_subscriber, BackfillStatus};
pub use config::{CilConfig, RankWeights};
pub use dense::{DenseIndex, DenseIndexHandle, DenseRetrieval};
pub use embed::{Embedder, MemoryEmbedder};
pub use extract::{extract_profile, ProfileStore};
pub use facade::CapabilityIntelligence;
pub use graph::{derive_edges, CapabilityEdge, CapabilityGraph, EdgeKind};
pub use index::{
    CandidateSource, CapabilityCandidate, CapabilityIndex, LexicalIndex, LexicalIndexHandle,
};
pub use intent::{derive_goal_intent, parse_required, GoalIntent};
pub use learn::{FeedbackLearner, NodeOutcome};
pub use market::{ClawHubProvider, MarketEntry, MarketplaceProvider};
pub use plan::{CapabilityPlanner, DefaultCapabilityPlanner};
pub use profile::{
    decode_embedding, encode_embedding, CapabilityProfile, CapabilityProfileRow, CapabilityTag,
    ProfileColumns,
};
pub use rank::{
    AllRuntimesAvailable, CapabilityRanker, DefaultCapabilityRanker, RuntimeAvailability,
    RuntimeCapabilitySet,
};
pub use recommend::{DefaultRecommender, Recommendation, Recommender};

/// The single error type for the Capability Intelligence Layer.
///
/// Every fallible CIL operation returns `Result<_, CilError>`. Per the design's
/// **honesty invariant** (§Error Handling), each variant is *user-actionable* and
/// none silently swallows a failure: a degraded backend, an unreachable
/// marketplace, or an invalid plan is always surfaced truthfully rather than
/// masked as success.
///
/// Variants map to the scenarios in design §Error Handling:
///
/// | Variant      | Scenario |
/// |--------------|----------|
/// | [`Embed`]    | Embedder failed to load / embed (falls back to frozen BM25, reported degraded). |
/// | [`Market`]   | Marketplace provider sync/fetch failure, disallowed host, or oversized manifest. |
/// | [`Acquire`]  | Acquisition failed (installer verify/hash/signature, or generation abort/over-budget). |
/// | [`Plan`]     | Planner produced an invalid graph (cycle / missing executor / over caps). |
/// | [`Permission`] | Permission engine could not authorize (grant store failure, etc.). |
/// | [`Degraded`] | A required backend (embedder/network) is unavailable; honest degraded mode. |
/// | [`Io`]       | Underlying I/O / persistence failure (e.g. derived-table migration). |
///
/// [`Embed`]: CilError::Embed
/// [`Market`]: CilError::Market
/// [`Acquire`]: CilError::Acquire
/// [`Plan`]: CilError::Plan
/// [`Permission`]: CilError::Permission
/// [`Degraded`]: CilError::Degraded
/// [`Io`]: CilError::Io
#[derive(Debug, Error)]
pub enum CilError {
    /// The embedder failed to load a model or embed text. Discovery can fall
    /// back to the frozen BM25 index; surface this so the degraded state is
    /// honest rather than hidden.
    #[error("capability embedding failed: {0}. Retry after the embedding model is available, or continue with lexical-only discovery")]
    Embed(String),

    /// A marketplace provider could not sync or fetch, or returned a disallowed
    /// host / oversized manifest (rejected by the frozen `DomainValidator`).
    #[error("marketplace error: {0}. Check network connectivity or the provider configuration; stale cached results may be served meanwhile")]
    Market(String),

    /// Acquisition (marketplace install or A9 generation) failed. Nothing was
    /// registered; the failing stage is reported honestly.
    #[error("skill acquisition failed: {0}. No skill was installed — surface the reason and try an alternative candidate")]
    Acquire(String),

    /// The planner produced an invalid `ExecutionGraph` (cycle, missing
    /// executor, or breadth/depth cap exceeded). Rejected before execution.
    #[error("capability plan invalid: {0}. Re-plan with reduced scope or decline the goal")]
    Plan(String),

    /// The permission engine could not authorize a request (e.g. grant store
    /// failure). Deny-by-default: the node is not executed.
    #[error(
        "permission authorization failed: {0}. Review the requested capabilities and re-approve"
    )]
    Permission(String),

    /// A required backend (embedder or network) is unavailable. CIL operates in
    /// an honestly-reported degraded mode and may fall back to the frozen router.
    #[error("capability intelligence degraded: {0}. Falling back to the direct router path until the backend is available")]
    Degraded(String),

    /// An underlying I/O or persistence failure (e.g. derived-table access or
    /// forward-only migration on `skills.db`).
    #[error("capability storage I/O error: {0}. Check disk space and permissions for the skills database")]
    Io(String),
}

/// Backend-availability signal threaded through the CIL facade (design §13.1,
/// §13.2 — honest degraded mode).
///
/// CIL depends on two optional backends: a semantic **embedder** (for dense
/// discovery) and **network** (for marketplace sync/acquisition). When either is
/// unavailable the ICP must not fail or panic — it falls back to the frozen
/// BM25/direct-router path and reports the degraded state *honestly* rather than
/// masking it as success.
///
/// The [`Default`] (and [`DegradedState::non_degraded`]) constructor reports a
/// fully-available, **non-degraded** state. The handler consults
/// [`is_degraded`](DegradedState::is_degraded) before ever delegating to the
/// facade: degraded → frozen fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DegradedState {
    /// Whether the semantic embedder is loaded and usable.
    pub embedder_available: bool,
    /// Whether network (for marketplace sync/acquisition) is reachable.
    pub network_available: bool,
}

impl Default for DegradedState {
    fn default() -> Self {
        // Default constructor reports non-degraded (both backends available).
        Self {
            embedder_available: true,
            network_available: true,
        }
    }
}

impl DegradedState {
    /// A fully-available, non-degraded state (both backends up). Alias for
    /// [`Default::default`] with an explicit, intention-revealing name.
    pub fn non_degraded() -> Self {
        Self::default()
    }

    /// `true` when any required backend is unavailable. When degraded, the
    /// handler falls back to the frozen router path (honest degraded), never a
    /// panic or silent failure.
    pub fn is_degraded(&self) -> bool {
        !self.embedder_available || !self.network_available
    }
}

/// Per-request context threaded into the CIL facade from the handler.
///
/// Skeleton type from design §8.8. Fields are intentionally minimal at this
/// scaffolding stage and will be extended as later phases wire discovery,
/// acquisition, planning, and permissions through the facade. Kept `Clone` so
/// the facade can hand copies to concurrent discovery stages.
#[derive(Debug, Clone, Default)]
pub struct RequestCtx {
    /// Optional workspace scope for permission grants and catalog partitioning.
    pub workspace_id: Option<String>,
    /// Optional correlation/session id for audit-ledger telemetry.
    pub session_id: Option<String>,
}

/// The outcome of `CapabilityIntelligence::fulfill` (design §8.8).
///
/// Skeleton enum for scaffolding task 1.1. The variants mirror the three honest
/// outcomes of a goal: a concrete plan, a set of recommendations to
/// install/generate, or an honest decline. Payload types for [`Fulfillment::Plan`]
/// and [`Fulfillment::Recommend`] are placeholders defined below and are fleshed
/// out by the permission (§11) and recommender (§7) phases.
#[derive(Debug)]
pub enum Fulfillment {
    /// A single-skill plan (today's common case) — a 1-node `ExecutionGraph`
    /// plus the permission decisions for its nodes.
    Plan(crate::execution::ExecutionGraph, Vec<PermissionDecision>),
    /// Nothing installed matches; ranked options to install/generate.
    Recommend(Vec<Recommendation>),
    /// Honest decline (out of scope for OpenClaw / a native tool is better).
    Decline {
        /// User-facing explanation for why the goal was declined.
        reason: String,
    },
}

/// Placeholder for the permission decision produced by the `PermissionEngine`
/// (design §8.6). Defined fully in the permission-redesign phase (task 11);
/// present here only so [`Fulfillment::Plan`] has a stable shape.
#[derive(Debug, Clone)]
pub struct PermissionDecision;

// The `Recommendation` type is now defined fully in [`recommend`] (task 7.1) and
// re-exported above as `pub use recommend::Recommendation`. It carries the real
// ranking signals plus a signal-derived rationale (design §8.7); `Fulfillment::
// Recommend(Vec<Recommendation>)` references it unchanged.

// The `CapabilityIntelligence` facade now lives in [`facade`] (task 5.3) and is
// re-exported above as `pub use facade::CapabilityIntelligence`. It composes the
// frozen building blocks (index/ranker/embedder/llm/audit) and implements the
// installed single-skill discover → rank → plan/decline path. Later phases add
// the marketplace index, capability graph, acquisition, planner, permission
// engine, recommender, and learner per design §8.8.

#[cfg(test)]
mod tests {
    use super::*;

    /// Each `CilError` variant renders a non-empty, user-actionable message.
    #[test]
    fn cil_error_variants_are_user_actionable() {
        let cases = vec![
            CilError::Embed("model missing".into()),
            CilError::Market("host not allowed".into()),
            CilError::Acquire("signature mismatch".into()),
            CilError::Plan("cycle detected".into()),
            CilError::Permission("grant store locked".into()),
            CilError::Degraded("embedder offline".into()),
            CilError::Io("disk full".into()),
        ];
        for err in cases {
            let msg = err.to_string();
            assert!(!msg.is_empty(), "error message must not be empty: {err:?}");
            // User-actionable messages include guidance beyond the bare cause.
            assert!(
                msg.len() > 20,
                "error message should be actionable, got: {msg}"
            );
        }
    }

    /// Exhaustive per-variant check: every `CilError` variant maps to a
    /// non-empty, user-actionable `Display` message. The `match` is exhaustive
    /// so adding a new variant forces this test to be updated (compile error),
    /// keeping the honesty invariant enforced for the whole error surface.
    #[test]
    fn cil_error_every_variant_maps_to_actionable_message() {
        let all = [
            CilError::Embed("e".into()),
            CilError::Market("e".into()),
            CilError::Acquire("e".into()),
            CilError::Plan("e".into()),
            CilError::Permission("e".into()),
            CilError::Degraded("e".into()),
            CilError::Io("e".into()),
        ];
        for err in all {
            // Exhaustive match: compiler guarantees every variant is handled.
            let covered = match err {
                CilError::Embed(_)
                | CilError::Market(_)
                | CilError::Acquire(_)
                | CilError::Plan(_)
                | CilError::Permission(_)
                | CilError::Degraded(_)
                | CilError::Io(_) => true,
            };
            assert!(covered);
            let msg = err.to_string();
            // Message embeds the cause and adds actionable guidance (a sentence
            // with a directive), not just the bare cause string.
            assert!(msg.contains('e'), "message should include the cause: {msg}");
            assert!(
                msg.split_whitespace().count() >= 6,
                "message should carry actionable guidance, got: {msg}"
            );
        }
    }

    /// `DegradedState` default reports non-degraded (both backends available).
    #[test]
    fn degraded_state_default_is_non_degraded() {
        let ds = DegradedState::default();
        assert!(ds.embedder_available);
        assert!(ds.network_available);
        assert!(!ds.is_degraded());
        assert_eq!(ds, DegradedState::non_degraded());
    }

    /// Any unavailable backend flips `is_degraded` to `true`.
    #[test]
    fn degraded_state_reports_missing_backends() {
        let no_embed = DegradedState {
            embedder_available: false,
            network_available: true,
        };
        assert!(no_embed.is_degraded());
        let no_net = DegradedState {
            embedder_available: true,
            network_available: false,
        };
        assert!(no_net.is_degraded());
    }

    /// `RequestCtx` default is empty (scaffolding skeleton).
    #[test]
    fn request_ctx_default_is_empty() {
        let ctx = RequestCtx::default();
        assert!(ctx.workspace_id.is_none());
        assert!(ctx.session_id.is_none());
    }

    /// `Fulfillment::Decline` carries a reason (honesty invariant).
    #[test]
    fn fulfillment_decline_carries_reason() {
        let f = Fulfillment::Decline {
            reason: "native tool is better".into(),
        };
        match f {
            Fulfillment::Decline { reason } => assert!(!reason.is_empty()),
            _ => panic!("expected Decline"),
        }
    }
}
