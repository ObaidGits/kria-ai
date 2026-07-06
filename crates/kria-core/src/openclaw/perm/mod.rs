//! Permission engine module (OpenClaw ICP §8.7).
//!
//! This module hosts the tiered, metadata-driven permission redesign that
//! **extends** the frozen `ApprovalCache` — it introduces no second permission
//! store and no second source of truth. The frozen `ApprovalCache` remains the
//! in-process hash/widening primitive and `capability::classify_risk` remains
//! the risk primitive; this module only adds durable, scoped, revocable
//! persistence on top of them.
//!
//! # Module map
//!
//! | Item | Purpose | Frozen owner extended |
//! |------|---------|-----------------------|
//! | [`GrantStore`] | Persistent per-scope grants over `capability_grants_scoped` | new table in `skills.db` via additive `MIGRATIONS` (migration 5) |
//!
//! `PermissionEngine::authorize` and the `evaluate → authorize` swap / revocation
//! wiring land in later tasks (11.2 / 11.4); this file scaffolds the module and
//! re-exports the persistence layer only.
//!
//! # Runtime authority invariants (preserved)
//!
//! Deny-by-default: an expired or revoked grant is **never** returned as active,
//! so a caller that reuses a grant can only ever reuse a live, un-revoked one.
//! The registry stays the single source of truth; `capability_grants_scoped` is
//! additive persistence keyed by `skill_id`, rebuildable/droppable with the flag
//! off.

pub mod engine;
pub mod grant_store;

pub use engine::{
    AuthorizeRequest, DefaultPermissionEngine, PermissionDecision, PermissionEngine,
    PermissionTier, PromptSpec, RiskEscalation,
};
pub use grant_store::{GrantDecision, GrantStore, ScopeKind, ScopedGrant};
