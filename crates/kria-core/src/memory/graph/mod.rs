//! Graph module — policy-safe typed graph projection, query, and traversal.
//!
//! This module is the planned target for MGR-002 (Canonical Mixed Graph
//! Projection) and MGR-007 (Versioned Bounded Graph API), wired as F2.3 in the
//! implementation plan.
//!
//! Sub-modules:
//! - [`projection`] — Canonical DTOs for the typed policy-safe graph contract
//!   (task 2.3.1).  Every projected item carries stable ID, node kind, authority
//!   class, Graph_Revision, Effective_Policy, Truth_State, Valid_Time,
//!   Provenance summary, and typed metadata.
//! - [`query`] — Entity-primary query request/response DTOs and the pure
//!   stateless [`query::GraphQueryProjector`] that enforces the entity-primary
//!   contract, generates labeled navigation facets, handles hidden endpoints,
//!   and enforces hard node/edge limits (task 2.3.2).
//! - [`traversal`] — Cycle-safe ≤3-hop BFS and shortest-path traversal engine
//!   with per-hop/total caps, hidden-intermediary policy, and deterministic
//!   ordering (task 2.3.3).
//! - [`policy_filter`] — Stateless policy filter gate applied before every
//!   seed/expansion/count/frontier operation; omits any path with a hidden
//!   intermediary and exposes no hidden stable identifier (task 2.3.4).
//!   Implements design §A5, §6.5, and MGR-004 AC 4/5.
//! - [`frontier`] — Opaque frontier resumption tokens ([`frontier::FrontierTokenBuilder`]),
//!   endpoint-complete edge assembly ([`frontier::EdgeAssembler`]), and UUID-as-label
//!   guard ([`frontier::LabelGuard`]) (task 2.3.5). Implements MGR-002 AC 4/6,
//!   MGR-001 AC 4, and design §A4.
//! - [`analytics`] — Honest analytics vocabulary for connected-component,
//!   community, and centrality outputs (task 2.4.6). Implements MGR-011 AC 1–4
//!   and design §A4.

pub mod analytics;
pub mod frontier;
pub mod policy_filter;
pub mod projection;
pub mod query;
pub mod traversal;

#[cfg(test)]
mod tests;
