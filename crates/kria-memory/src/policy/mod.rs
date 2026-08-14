//! The v2 Effective-Policy / Write-Policy engine (design §3 planned
//! `policy/{write_policy,effective_policy,modes,source_trust}.rs`; §4.1 policy
//! columns; §7.3/§7.4 source trust; task **F1.4**).
//!
//! This module is the redesign's mandatory admission boundary. It is distinct
//! from the legacy [`crate::write_policy`] module (the v1 write gate),
//! which F1.5 hard-cuts once every durable writer is routed through this engine.
//! Rather than redefine provenance primitives, this module **reuses** the
//! validated value objects already anchored at the authority boundary
//! ([`crate::authority::command`]'s
//! [`SourceKind`](crate::authority::SourceKind) /
//! [`SourceTrust`](crate::authority::SourceTrust) /
//! [`SourceContext`](crate::authority::SourceContext)) and the caller
//! model ([`crate::model::CallerOrigin`]).
//!
//! ## Module map (F1.4)
//!
//! * [`source_trust`] — **F1.4.1 (this task)**: the source identity / trust /
//!   capability *context* every contributing origin carries, its default trust
//!   tier, its permitted-capability set, and its consent requirement. This is
//!   the input the Effective-Policy meet (F1.4.2) intersects.
//! * [`effective_policy`] — **F1.4.2**: the most-restrictive meet over
//!   contributing source policies with a policy-version / provenance hash,
//!   denying on empty capability or incompatible namespace/scope/owner
//!   intersection.
//! * [`declassification`] — **F1.4.3**: an authorized declassification recorded
//!   as a **new** governed, audited provenance record (MGR-004 AC3) — never a
//!   mutation of the contributing source policy — routed through the
//!   authority transaction so it is auditable and reversible.
//! * [`crate::modes`] — **F1.4.4**: the five canonical `Memory_Mode`
//!   classes (Permanent / Temporary / Session_Only / Read_Only / Disabled) with
//!   typed admission/read errors and the session-purge ledger. It is the
//!   redesign's deterministic mode gate (shared with the historical F1.3.2 write
//!   gate), so it lives alongside the other write-boundary primitives in
//!   `memory/modes.rs` rather than being forked into a parallel module here.
//! * [`read_authorization`] — **F1.4.5**: the policy-before-everything read
//!   enforcement primitive. [`authorize_read`](read_authorization::authorize_read)
//!   is the single policy-first gate that turns a caller + Effective Policy into
//!   a typed [`AuthorizedScope`](read_authorization::AuthorizedScope); every
//!   downstream read stage (SQL/query planning, authorized totals, traversal
//!   expansion, ranking, serialization, cache/cursor keys, logs, traces,
//!   renderer DTOs) consumes that scope (or the
//!   [`AuthorizedCandidates`](read_authorization::AuthorizedCandidates) it mints)
//!   so "policy first" is structurally true. F2/F3/F4 wire the concrete tables,
//!   retrieval, traversal, and DTOs through this contract.
//! * [`invalidation`] — **F1.4.6**: the in-flight policy-change invalidation
//!   primitive. It builds on [`AuthorizedScope::policy_hash`](read_authorization::AuthorizedScope::policy_hash)
//!   to bind every in-flight response, pending write, trace, cursor, and cache
//!   entry to a [`PolicyEpoch`](invalidation::PolicyEpoch); when the caller's
//!   identity / scope / capability / effective policy changes (the policy hash
//!   differs), the response/trace is discarded, the pending write is rolled
//!   back, the cursor is rejected with a typed bounded-refetch instruction, and
//!   superseded cache entries are evicted. F3/F4 wire the concrete request
//!   pipeline, cursors, caches, and UI session through this contract.
//! * `write_policy` — F1.5: the policy-before-everything durable-write admission
//!   gate (not built here).

pub mod declassification;
pub mod effective_policy;
pub mod invalidation;
pub mod read_authorization;
pub mod source_trust;
