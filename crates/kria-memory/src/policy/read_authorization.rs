//! Policy-before-everything read enforcement (task **F1.4.5**; design §2 A5
//! isolation, §5.2 snapshot reads, §9.2 cache keys; MGR-004 AC4/AC5/AC7,
//! MGR-028, MGR-035; MGD-007/MGD-034; property suites V-POLICY-01/02).
//!
//! Design §2 **A5** is categorical: "authorization and Effective Policy precede
//! planning, counts, ranking, serialization, caching, and rendering." MGR-004
//! AC4 restates it for reads: "WHEN any read executes, THE Cognitive_Memory_System
//! SHALL enforce Effective_Policy **before** query planning, result counts,
//! ranking, serialization, caching, and rendering." AC5 adds that a hidden
//! record contributing to an aggregate exposes only *caller-authorized* counts
//! and labels, and AC7 demands **zero** protected-data leaks across every read
//! flow.
//!
//! ## What this module delivers (and what it defers)
//!
//! The concrete read pipeline — the five-strategy retrieval engine (F3), the
//! bounded graph traversal (F3/F4), the serialization DTOs and renderer scene
//! (F4) — and the concrete cognitive-record tables (F2) do not exist yet. This
//! module therefore delivers the **enforcement primitive and discipline** those
//! later stages MUST route through, so that "policy first" is *structurally*
//! true rather than a convention retrofitted onto a finished pipeline:
//!
//! * [`authorize_read`] is the single policy-first gate. It consumes the
//!   caller's [`CallerContext`] and the caller's read [`EffectivePolicy`] and
//!   produces an [`AuthorizedScope`] — or a typed [`ReadDenial`]. **No** row,
//!   count, rank, DTO, cache key, cursor, log, or trace may be produced without
//!   first obtaining an [`AuthorizedScope`], because every downstream helper is
//!   a method on that value.
//! * [`AuthorizedScope::predicate`] yields a [`ScopePredicate`] — a SQL
//!   `WHERE`-clause fragment plus bound parameters — that is injected into query
//!   planning so unauthorized rows are excluded **at the query level**, never
//!   filtered after the fact (A5: policy precedes planning). Concrete tables in
//!   F2/F3 compose this fragment into their `SELECT`s.
//! * [`AuthorizedScope::admit_candidates`] is the only constructor of
//!   [`AuthorizedCandidates`], the typed evidence that a set of items has passed
//!   the scope filter. Ranking, counting, and serialization stages accept
//!   `&AuthorizedCandidates<T>` and therefore **cannot** — at the type level —
//!   operate on unauthorized candidates (A5: policy precedes counts, ranking,
//!   serialization).
//! * [`AuthorizedScope::authorized_total`] computes counts over authorized rows
//!   only, so a hidden record can never change an authorized caller's observable
//!   count (MGR-004 AC5, the paired-world non-interference property V-POLICY-02).
//! * [`AuthorizedScope::cache_key`] / [`AuthorizedScope::cursor_key`] mix the
//!   [`AuthorizedScope::policy_hash`] into every key (design §9.2:
//!   `(schema, revision, callerPolicyHash, queryHash)`), so an entry computed
//!   under one policy can **never** be served under another.
//! * [`AuthorizedScope::redacted_ref`] and [`PolicySafeLog`] give logs and
//!   traces policy-safe, non-reversible correlation identifiers that carry no
//!   record id, label, content, or hidden count (MGR-028 AC2).
//!
//! ## Downstream stages that consume this primitive (deferred gates)
//!
//! | Stage                     | Gate | How it consumes the primitive |
//! |---------------------------|------|-------------------------------|
//! | SQL / query planning      | F2/F3 | compose [`ScopePredicate`] into every `SELECT` before rows are read |
//! | Authorized totals/counts  | F3   | count via [`AuthorizedScope::authorized_total`] / `COUNT(*)` under the predicate |
//! | Traversal expansion       | F3/F4 | filter each hop's frontier with [`AuthorizedScope::retain_authorized`] before expanding |
//! | Ranking                   | F3   | rank only [`AuthorizedCandidates`] |
//! | Serialization / DTOs      | F4   | build DTOs only from [`AuthorizedCandidates`]; project via [`AuthorizedScope::admits`] |
//! | Cache / cursor keys       | F3/F4 | derive keys with [`AuthorizedScope::cache_key`] / [`cursor_key`] |
//! | Logs / traces             | F1–F5 | emit [`PolicySafeLog`] with [`RedactedRef`] identifiers only |
//! | Renderer DTOs (scene)     | F4    | scene builder consumes [`AuthorizedCandidates`]; unauthorized omitted |
//!
//! This subtask delivers the contract + core helpers + tests; it does **not**
//! implement in-flight invalidation on identity/scope change (that is F1.4.6,
//! which builds on [`AuthorizedScope::policy_hash`]) or the admission benchmark
//! (F1.4.7).

use rusqlite::types::Value as SqlValue;

use crate::ids::blake3_hex;
use crate::model::{
    CallerContext, GraphRevision, PolicyPartition, RecordId, SchemaVersion,
};

use super::effective_policy::{DenyReason, EffectivePolicy, PolicyOutcome, POLICY_VERSION};
use super::source_trust::{Capability, CapabilitySet};

use std::collections::BTreeSet;

/// Length (hex chars) of a [`RedactedRef`] token. 24 hex chars = 96 bits of the
/// BLAKE3 digest — enough to correlate within one policy without being a
/// practically reversible or collision-prone identifier.
const REDACTED_REF_LEN: usize = 24;

// ─────────────────────────────────────────────────────────────────────────
// ReadDenial — why a read is not authorized
// ─────────────────────────────────────────────────────────────────────────

/// Why [`authorize_read`] refused to produce an [`AuthorizedScope`]. Denial is a
/// typed, terminal outcome — there is **no** permissive fallback and no partial
/// scope. A denied read produces no predicate, no count, no DTO, and no cache
/// key, so nothing downstream can run on unauthorized data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadDenial {
    /// The caller's Effective Policy itself denied (empty capability
    /// intersection, incompatible namespace/scope/owner, or no contributors).
    /// Carries the exact [`DenyReason`]s from the meet.
    PolicyDenied(BTreeSet<DenyReason>),
    /// The Effective Policy allowed a write-ish grant but does not carry
    /// [`Capability::ReadCore`], so the caller may not read governed records.
    NotReadable,
}

impl ReadDenial {
    /// A stable, content-free reason code for policy-safe logging (MGR-028 AC2):
    /// it names *why* a read was denied without revealing any record identity,
    /// label, or count.
    pub fn reason_code(&self) -> &'static str {
        match self {
            ReadDenial::PolicyDenied(_) => "policy_denied",
            ReadDenial::NotReadable => "not_readable",
        }
    }
}

impl std::fmt::Display for ReadDenial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadDenial::PolicyDenied(reasons) => {
                write!(f, "read denied by effective policy:")?;
                for (i, r) in reasons.iter().enumerate() {
                    write!(f, "{}{r}", if i == 0 { " " } else { ", " })?;
                }
                Ok(())
            }
            ReadDenial::NotReadable => {
                f.write_str("read denied: effective policy lacks read_core capability")
            }
        }
    }
}

impl std::error::Error for ReadDenial {}

// ─────────────────────────────────────────────────────────────────────────
// ScopedItem — an item carrying the policy partition to authorize it against
// ─────────────────────────────────────────────────────────────────────────

/// A read candidate that carries the [`PolicyPartition`] its authorization is
/// decided against. Every projectable/ranking/serializable read item implements
/// this so the [`AuthorizedScope`] can decide visibility uniformly. The concrete
/// cognitive-record types (F2) and retrieval candidates (F3) implement it.
pub trait ScopedItem {
    /// The record's own governing policy partition (namespace / scope /
    /// sensitivity / owner).
    fn policy_partition(&self) -> &PolicyPartition;
}

/// A blanket impl so a bare [`PolicyPartition`] is itself a scoped item — useful
/// for tests and for stages that only carry the partition.
impl ScopedItem for PolicyPartition {
    fn policy_partition(&self) -> &PolicyPartition {
        self
    }
}

// ─────────────────────────────────────────────────────────────────────────
// ScopePredicate — the SQL filter injected before rows are read
// ─────────────────────────────────────────────────────────────────────────

/// A parameterized SQL predicate deriving the authorized partition filter, to be
/// composed into a query's `WHERE` clause **before** any rows are read (A5:
/// policy precedes planning). The predicate is derived from the
/// [`AuthorizedScope`], never applied as a post-filter, so unauthorized rows are
/// excluded by the query planner itself.
///
/// The fragment uses positional `?` placeholders in declaration order; bind
/// [`params`](ScopePredicate::params) in the same order. Columns are addressed
/// through a caller-supplied table alias so the fragment composes into any
/// governed read `SELECT` (the concrete column names — `namespace`, `scope`,
/// `sensitivity`, `owner_id` — match the design §4.1 policy columns).
#[derive(Debug, Clone, PartialEq)]
pub struct ScopePredicate {
    clause: String,
    params: Vec<SqlValue>,
}

impl ScopePredicate {
    /// The `WHERE`-clause fragment (without the leading `WHERE`/`AND`), with
    /// every policy column qualified by `alias` (e.g. `"r"` → `r.namespace`).
    /// Combine into a query with `AND`.
    pub fn clause(&self) -> &str {
        &self.clause
    }

    /// The bound parameter values, in the same order as the `?` placeholders in
    /// [`clause`](Self::clause). Directly bindable via `rusqlite` `params_from_iter`.
    pub fn params(&self) -> &[SqlValue] {
        &self.params
    }

    /// The fragment wrapped as `AND (…)` for appending to an existing `WHERE`.
    pub fn and_clause(&self) -> String {
        format!("AND ({})", self.clause)
    }
}

// ─────────────────────────────────────────────────────────────────────────
// AuthorizedScope — the typed, policy-derived authorized query scope
// ─────────────────────────────────────────────────────────────────────────

/// The **authorized query scope**: the typed policy predicate every read stage
/// consumes. It is produced *only* by [`authorize_read`] (the policy-first
/// gate), so its existence is proof that authorization and Effective Policy have
/// already been applied — planning, counts, ranking, serialization, caching, and
/// rendering all take an `AuthorizedScope` (or the [`AuthorizedCandidates`] it
/// mints) as input.
///
/// It carries the authorized [`PolicyPartition`] (the caller's clearance:
/// namespace / scope / sensitivity ceiling / owner), the read
/// [`CapabilitySet`], and a deterministic [`policy_hash`](Self::policy_hash)
/// that binds a cache entry / cursor / log line to exactly this policy so it can
/// never be reused under another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedScope {
    partition: PolicyPartition,
    capabilities: CapabilitySet,
    policy_hash: String,
}

impl AuthorizedScope {
    /// The authorized partition: the caller's clearance the read is confined to.
    /// A record is visible only if it falls within this partition (see
    /// [`admits`](Self::admits)).
    pub fn partition(&self) -> &PolicyPartition {
        &self.partition
    }

    /// The capabilities authorized for this read (always contains
    /// [`Capability::ReadCore`]).
    pub fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    /// The deterministic policy hash bound into every cache key, cursor, and
    /// policy-safe log/trace identifier (design §9.2 `callerPolicyHash`). It is
    /// a pure function of the authorized partition, the read capabilities, and
    /// the contributing Effective Policy's `provenance_hash`, so two reads share
    /// a hash **iff** they run under the same authorization — the basis for
    /// F1.4.6 in-flight invalidation.
    pub fn policy_hash(&self) -> &str {
        &self.policy_hash
    }

    /// Whether `partition` is authorized under this scope. This is the in-memory
    /// twin of [`predicate`](Self::predicate) and MUST decide identically to the
    /// SQL filter (a property test asserts the equivalence): a record is visible
    /// iff its namespace and scope match the caller's, its sensitivity is within
    /// the caller's ceiling, and its owner is unset or equals the caller's owner.
    pub fn admits(&self, partition: &PolicyPartition) -> bool {
        partition.namespace() == self.partition.namespace()
            && partition.scope() == self.partition.scope()
            && partition.sensitivity() <= self.partition.sensitivity()
            && owner_admitted(self.partition.owner_id(), partition.owner_id())
    }

    /// Whether a [`ScopedItem`] is authorized under this scope.
    pub fn admits_item<T: ScopedItem>(&self, item: &T) -> bool {
        self.admits(item.policy_partition())
    }

    /// The parameterized SQL [`ScopePredicate`] for this scope, addressing policy
    /// columns through `alias`. Inject this into query planning so unauthorized
    /// rows are excluded at the query level (A5). The generated fragment is:
    ///
    /// ```sql
    /// {alias}.namespace = ? AND {alias}.scope = ? AND {alias}.sensitivity <= ?
    ///   AND ({alias}.owner_id IS NULL[ OR {alias}.owner_id = ?])
    /// ```
    ///
    /// The trailing owner disjunct is present only when the caller carries an
    /// owner; an owner-less caller sees only owner-less rows.
    pub fn predicate(&self, alias: &str) -> ScopePredicate {
        let mut clause =
            format!("{alias}.namespace = ? AND {alias}.scope = ? AND {alias}.sensitivity <= ?");
        let mut params = vec![
            SqlValue::Text(self.partition.namespace().to_string()),
            SqlValue::Text(self.partition.scope().to_string()),
            SqlValue::Integer(i64::from(self.partition.sensitivity())),
        ];
        match self.partition.owner_id() {
            Some(owner) => {
                clause.push_str(&format!(
                    " AND ({alias}.owner_id IS NULL OR {alias}.owner_id = ?)"
                ));
                params.push(SqlValue::Text(owner.to_string()));
            }
            None => {
                clause.push_str(&format!(" AND {alias}.owner_id IS NULL"));
            }
        }
        ScopePredicate { clause, params }
    }

    /// Filter `items` to only those authorized under this scope, minting the
    /// typed [`AuthorizedCandidates`] evidence. This is the **only** constructor
    /// of `AuthorizedCandidates`, so any stage that accepts one is guaranteed to
    /// have received policy-filtered input (A5: policy precedes ranking /
    /// serialization / rendering).
    pub fn admit_candidates<T, I>(&self, items: I) -> AuthorizedCandidates<T>
    where
        T: ScopedItem,
        I: IntoIterator<Item = T>,
    {
        let items = items
            .into_iter()
            .filter(|item| self.admits_item(item))
            .collect();
        AuthorizedCandidates {
            policy_hash: self.policy_hash.clone(),
            items,
        }
    }

    /// Retain only authorized items in-place — the per-hop traversal-expansion
    /// hook. Traversal MUST call this on each hop's frontier **before**
    /// expanding it, so a hidden record can never be traversed *through* even if
    /// it is never itself emitted.
    pub fn retain_authorized<T: ScopedItem>(&self, frontier: &mut Vec<T>) {
        frontier.retain(|item| self.admits_item(item));
    }

    /// The authorized count over `candidates`: the number of items visible under
    /// this scope. A hidden (unauthorized) contributor is never counted, so the
    /// count an authorized caller observes is identical whether or not hidden
    /// records exist (MGR-004 AC5; V-POLICY-02 non-interference).
    pub fn authorized_total<'a, T, I>(&self, candidates: I) -> usize
    where
        T: ScopedItem + 'a,
        I: IntoIterator<Item = &'a T>,
    {
        candidates
            .into_iter()
            .filter(|item| self.admits_item(*item))
            .count()
    }

    /// The cache key for a read result under this scope (design §9.2:
    /// `(schema, revision, callerPolicyHash, queryHash)`). Because the
    /// [`policy_hash`](Self::policy_hash) is a component, a result computed under
    /// one policy can never be served under another — cross-policy reuse is
    /// impossible by construction.
    pub fn cache_key(
        &self,
        schema: SchemaVersion,
        revision: GraphRevision,
        query_hash: &str,
    ) -> String {
        format!(
            "{}:{}:{}:{}",
            schema.get(),
            revision.get(),
            self.policy_hash,
            query_hash
        )
    }

    /// The cursor key for a paginated read under this scope. Like
    /// [`cache_key`](Self::cache_key) it binds the policy hash so a cursor issued
    /// under one policy cannot be replayed under another; `position` is the
    /// opaque page position (offset, sort-key tuple, …) the retrieval layer
    /// defines.
    pub fn cursor_key(&self, revision: GraphRevision, query_hash: &str, position: &str) -> String {
        format!(
            "cur:{}:{}:{}:{}",
            revision.get(),
            self.policy_hash,
            query_hash,
            position
        )
    }

    /// A stable, non-reversible reference to a record id for logs and traces
    /// (MGR-028 AC2). It is the BLAKE3 digest of the id salted by this scope's
    /// [`policy_hash`](Self::policy_hash), so it reveals no record identity and
    /// **cannot be correlated across policies** — the same id under two
    /// different policies produces two different refs, preventing a hidden
    /// record's presence from being inferred by cross-policy log comparison.
    pub fn redacted_ref(&self, record_id: &RecordId) -> RedactedRef {
        let mut input =
            String::with_capacity(self.policy_hash.len() + record_id.as_str().len() + 1);
        input.push_str(&self.policy_hash);
        input.push('\n');
        input.push_str(record_id.as_str());
        let full = blake3_hex(input.as_bytes());
        RedactedRef(full[..REDACTED_REF_LEN].to_string())
    }

    /// Begin a policy-safe log/trace record for a correlated read. The returned
    /// [`PolicySafeLog`] exposes only the policy hash, a correlation id, and
    /// authorized aggregate counts / redacted refs — never content, labels, raw
    /// ids, or hidden cardinality (MGR-028 AC2).
    pub fn log(&self, correlation_id: impl Into<String>) -> PolicySafeLog {
        PolicySafeLog {
            correlation_id: correlation_id.into(),
            policy_hash: self.policy_hash.clone(),
            authorized_count: None,
            refs: Vec::new(),
        }
    }
}

/// Whether a caller with owner `caller_owner` may read a record owned by
/// `record_owner`. An owner-less record is readable by anyone in the partition;
/// an owned record is readable only by that same owner.
fn owner_admitted(caller_owner: Option<&str>, record_owner: Option<&str>) -> bool {
    match record_owner {
        None => true,
        Some(rec) => caller_owner == Some(rec),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// authorize_read — the single policy-first gate
// ─────────────────────────────────────────────────────────────────────────

/// The policy-first read gate (design §2 A5; MGR-004 AC4). Produces an
/// [`AuthorizedScope`] **iff** the caller's Effective Policy allows and carries
/// [`Capability::ReadCore`]; otherwise a typed [`ReadDenial`] with no scope.
///
/// This is the structural chokepoint: because every downstream read helper is a
/// method on [`AuthorizedScope`] (and [`AuthorizedCandidates`] can only be minted
/// by one), no stage — planning, counts, ranking, serialization, caching,
/// cursors, logs, traces, renderer DTOs — can execute before this gate has run.
///
/// The authorized partition is the caller's own [`CallerContext`] clearance (a
/// caller can never read beyond the partition they were authenticated for); the
/// [`policy_hash`](AuthorizedScope::policy_hash) additionally folds in the
/// Effective Policy's `provenance_hash`, so the same caller under two different
/// effective policies gets two different scopes (and therefore incompatible
/// caches/cursors/logs).
pub fn authorize_read(
    caller: &CallerContext,
    policy: &EffectivePolicy,
) -> Result<AuthorizedScope, ReadDenial> {
    // Policy-first: a denied Effective Policy propagates verbatim; no scope, no
    // downstream stage.
    let grant = match policy.outcome() {
        PolicyOutcome::Allow(grant) => grant,
        PolicyOutcome::Deny(reasons) => return Err(ReadDenial::PolicyDenied(reasons.clone())),
    };

    // Read authorization requires the ReadCore capability (MGR-043 AC2). A grant
    // that permits only writes never yields a read scope.
    if !grant.capabilities().contains(Capability::ReadCore) {
        return Err(ReadDenial::NotReadable);
    }

    let partition = caller.partition().clone();
    let capabilities = *grant.capabilities();
    let policy_hash = compute_policy_hash(&partition, &capabilities, policy.provenance_hash());

    Ok(AuthorizedScope {
        partition,
        capabilities,
        policy_hash,
    })
}

/// The deterministic caller-policy hash mixed into cache keys, cursors, and
/// redacted refs. A pure function of the authorized partition, the read
/// capabilities, and the Effective Policy provenance hash, version-prefixed for
/// domain separation. Equal authorizations hash equally; any change to
/// partition, capabilities, or contributing policy changes the hash (the basis
/// for F1.4.6 invalidation).
fn compute_policy_hash(
    partition: &PolicyPartition,
    capabilities: &CapabilitySet,
    provenance_hash: &str,
) -> String {
    let mut input = String::new();
    input.push_str(POLICY_VERSION);
    input.push('\n');
    input.push_str(&partition.partition_key());
    input.push('\n');
    input.push_str(partition.owner_id().unwrap_or(""));
    input.push('\n');
    for cap in capabilities.iter() {
        input.push_str(cap.as_str());
        input.push(',');
    }
    input.push('\n');
    input.push_str(provenance_hash);
    blake3_hex(input.as_bytes())
}

// ─────────────────────────────────────────────────────────────────────────
// AuthorizedCandidates — typed evidence a set has passed the scope filter
// ─────────────────────────────────────────────────────────────────────────

/// A set of read candidates that have **already** passed the [`AuthorizedScope`]
/// filter. It can only be constructed by [`AuthorizedScope::admit_candidates`],
/// so a stage that accepts `&AuthorizedCandidates<T>` is *structurally*
/// guaranteed to receive only authorized items — this is how "policy precedes
/// ranking / serialization / rendering" (A5) is enforced by the type system
/// rather than by convention.
///
/// Ranking reorders [`items`](Self::items); counting reads [`len`](Self::len);
/// serialization / DTO building iterates [`items`](Self::items). None of them
/// can reach an unauthorized record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedCandidates<T> {
    policy_hash: String,
    items: Vec<T>,
}

impl<T> AuthorizedCandidates<T> {
    /// The authorized items, in insertion order (ranking may reorder a mutable
    /// borrow via [`items_mut`](Self::items_mut)).
    pub fn items(&self) -> &[T] {
        &self.items
    }

    /// A mutable borrow of the authorized items, for in-place ranking. Reordering
    /// cannot introduce an unauthorized item because the set is closed.
    pub fn items_mut(&mut self) -> &mut [T] {
        &mut self.items
    }

    /// The number of authorized candidates — the authorized count safe to expose
    /// (MGR-004 AC5).
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the authorized set is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The [`AuthorizedScope::policy_hash`] these candidates were admitted under,
    /// for binding the derived cache entry / cursor / log to the same policy.
    pub fn policy_hash(&self) -> &str {
        &self.policy_hash
    }

    /// Consume into the owned authorized items.
    pub fn into_items(self) -> Vec<T> {
        self.items
    }

    /// Iterate the authorized items.
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.items.iter()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// RedactedRef + PolicySafeLog — privacy-safe observability (MGR-028)
// ─────────────────────────────────────────────────────────────────────────

/// A non-reversible, policy-salted reference to a record for logs and traces
/// (MGR-028 AC2). It carries no record id, label, or content, and differs across
/// policies for the same record, so protected identity cannot leak or be
/// cross-correlated through observability output.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RedactedRef(String);

impl RedactedRef {
    /// The opaque token text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RedactedRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A policy-safe log/trace record for a correlated read (MGR-028 AC2). It
/// exposes only a correlation id, the policy hash, an authorized aggregate
/// count, and [`RedactedRef`] identifiers — never memory content, private
/// labels, raw record ids, embeddings, or hidden cardinality. Build it with
/// [`AuthorizedScope::log`] and render with [`PolicySafeLog::render`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicySafeLog {
    correlation_id: String,
    policy_hash: String,
    authorized_count: Option<usize>,
    refs: Vec<RedactedRef>,
}

impl PolicySafeLog {
    /// Attach the caller-authorized count (only counts authorized rows).
    pub fn with_authorized_count(mut self, count: usize) -> Self {
        self.authorized_count = Some(count);
        self
    }

    /// Attach a redacted record reference (never a raw id).
    pub fn with_ref(mut self, redacted: RedactedRef) -> Self {
        self.refs.push(redacted);
        self
    }

    /// The correlation id.
    pub fn correlation_id(&self) -> &str {
        &self.correlation_id
    }

    /// The policy hash this log line is bound to.
    pub fn policy_hash(&self) -> &str {
        &self.policy_hash
    }

    /// The authorized aggregate count, if attached.
    pub fn authorized_count(&self) -> Option<usize> {
        self.authorized_count
    }

    /// The redacted references attached to this record.
    pub fn refs(&self) -> &[RedactedRef] {
        &self.refs
    }

    /// Render a single-line, content-free log string safe to persist in logs and
    /// traces. Contains only the correlation id, policy hash, authorized count,
    /// and redacted refs.
    pub fn render(&self) -> String {
        let mut out = format!("corr={} policy={}", self.correlation_id, self.policy_hash);
        if let Some(count) = self.authorized_count {
            out.push_str(&format!(" authorized_count={count}"));
        }
        if !self.refs.is_empty() {
            out.push_str(" refs=[");
            for (i, r) in self.refs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(r.as_str());
            }
            out.push(']');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::SourceTrust;
    use crate::policy::effective_policy::ContributingPolicy;
    use crate::policy::source_trust::{Capability, ConsentRequirement};
    use proptest::prelude::*;
    use rusqlite::{params, params_from_iter};

    // ── Fixtures ────────────────────────────────────────────────────────

    fn partition(ns: &str, scope: &str, sensitivity: u8, owner: Option<&str>) -> PolicyPartition {
        PolicyPartition::with_owner(ns, scope, sensitivity, owner.map(str::to_string)).unwrap()
    }

    /// A caller authenticated for the given clearance partition.
    fn caller(partition: PolicyPartition) -> CallerContext {
        CallerContext::local_desktop("device-1", partition).unwrap()
    }

    /// An allowing read policy (carries ReadCore + Observe) over the given
    /// partition.
    fn read_policy(partition: PolicyPartition, caps: &[Capability]) -> EffectivePolicy {
        let contributor = ContributingPolicy::new(
            "src-read",
            partition,
            CapabilitySet::from_capabilities(caps.iter().copied()),
            SourceTrust::System,
            ConsentRequirement::NotRequired,
        )
        .unwrap();
        EffectivePolicy::of(contributor)
    }

    fn read_scope(ns: &str, scope: &str, sensitivity: u8, owner: Option<&str>) -> AuthorizedScope {
        let part = partition(ns, scope, sensitivity, owner);
        let policy = read_policy(
            part.clone(),
            &[Capability::ReadCore, Capability::ObserveMemory],
        );
        authorize_read(&caller(part), &policy).expect("read is authorized")
    }

    // ── The policy-first gate ───────────────────────────────────────────

    #[test]
    fn authorize_read_denies_a_denied_policy_with_no_scope() {
        // Empty capability intersection → the meet denies.
        let a = read_policy(
            partition("user", "chat", 1, None),
            &[Capability::CorrectMemory],
        );
        let b = read_policy(
            partition("user", "chat", 1, None),
            &[Capability::ObserveMemory],
        );
        let denied = a.meet(&b);
        assert!(denied.is_denied());
        let c = caller(partition("user", "chat", 1, None));
        let err = authorize_read(&c, &denied).expect_err("denied policy yields no scope");
        assert!(matches!(err, ReadDenial::PolicyDenied(_)));
        assert_eq!(err.reason_code(), "policy_denied");
    }

    #[test]
    fn authorize_read_denies_a_grant_without_read_core() {
        // Allows (observe only) but no ReadCore → not readable.
        let part = partition("user", "chat", 1, None);
        let policy = read_policy(part.clone(), &[Capability::ObserveMemory]);
        assert!(policy.is_allowed());
        let err = authorize_read(&caller(part), &policy).expect_err("no read_core");
        assert_eq!(err, ReadDenial::NotReadable);
    }

    #[test]
    fn authorize_read_grants_scope_confined_to_caller_partition() {
        let scope = read_scope("user", "chat", 2, Some("owner-1"));
        assert_eq!(scope.partition().namespace(), "user");
        assert_eq!(scope.partition().scope(), "chat");
        assert_eq!(scope.partition().sensitivity(), 2);
        assert_eq!(scope.partition().owner_id(), Some("owner-1"));
        assert!(scope.capabilities().contains(Capability::ReadCore));
    }

    // ── admits(): visibility rule (in-memory twin of the SQL predicate) ──

    #[test]
    fn admits_enforces_namespace_scope_sensitivity_and_owner() {
        let scope = read_scope("user", "chat", 2, Some("owner-1"));
        // Authorized: same ns/scope, within sensitivity, matching owner.
        assert!(scope.admits(&partition("user", "chat", 2, Some("owner-1"))));
        // Authorized: owner-less record is readable by anyone in the partition.
        assert!(scope.admits(&partition("user", "chat", 1, None)));
        // Hidden: higher sensitivity than the caller's ceiling.
        assert!(!scope.admits(&partition("user", "chat", 3, None)));
        // Hidden: different namespace.
        assert!(!scope.admits(&partition("system", "chat", 0, None)));
        // Hidden: different scope.
        assert!(!scope.admits(&partition("user", "notes", 0, None)));
        // Hidden: a different owner's record.
        assert!(!scope.admits(&partition("user", "chat", 0, Some("owner-2"))));
    }

    #[test]
    fn owner_less_caller_sees_only_owner_less_rows() {
        let scope = read_scope("user", "chat", 3, None);
        assert!(scope.admits(&partition("user", "chat", 1, None)));
        assert!(!scope.admits(&partition("user", "chat", 1, Some("owner-1"))));
    }

    // ── (a) SQL predicate excludes unauthorized rows at the QUERY level ──

    /// Build a tiny records table with the design §4.1 policy columns and seed
    /// it with a labelled mix of authorized and hidden rows.
    fn seed_policy_rows(conn: &rusqlite::Connection) {
        conn.execute_batch(
            "CREATE TABLE rows_under_test(
                 label TEXT NOT NULL,
                 namespace TEXT NOT NULL,
                 scope TEXT NOT NULL,
                 sensitivity INTEGER NOT NULL,
                 owner_id TEXT
             );",
        )
        .unwrap();
        let mut insert = conn
            .prepare(
                "INSERT INTO rows_under_test(label, namespace, scope, sensitivity, owner_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .unwrap();
        // Authorized rows for caller user/chat/2 owner-1:
        insert
            .execute(params!["auth-a", "user", "chat", 0, Option::<String>::None])
            .unwrap();
        insert
            .execute(params!["auth-b", "user", "chat", 2, "owner-1"])
            .unwrap();
        // Hidden rows:
        insert
            .execute(params![
                "hidden-sensitivity",
                "user",
                "chat",
                3,
                Option::<String>::None
            ])
            .unwrap();
        insert
            .execute(params![
                "hidden-namespace",
                "system",
                "chat",
                0,
                Option::<String>::None
            ])
            .unwrap();
        insert
            .execute(params![
                "hidden-scope",
                "user",
                "notes",
                0,
                Option::<String>::None
            ])
            .unwrap();
        insert
            .execute(params!["hidden-owner", "user", "chat", 0, "owner-2"])
            .unwrap();
    }

    #[test]
    fn sql_predicate_excludes_unauthorized_rows_at_query_level() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        seed_policy_rows(&conn);

        let scope = read_scope("user", "chat", 2, Some("owner-1"));
        let predicate = scope.predicate("r");

        let sql = format!(
            "SELECT label FROM rows_under_test AS r WHERE {} ORDER BY label",
            predicate.clause()
        );
        let mut stmt = conn.prepare(&sql).unwrap();
        let labels: Vec<String> = stmt
            .query_map(params_from_iter(predicate.params().iter()), |row| {
                row.get::<_, String>(0)
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();

        // Only the two authorized rows survive the query-level filter.
        assert_eq!(labels, vec!["auth-a".to_string(), "auth-b".to_string()]);
        // Explicitly confirm no hidden row leaked.
        assert!(!labels.iter().any(|l| l.starts_with("hidden")));
    }

    #[test]
    fn sql_predicate_and_admits_decide_identically() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        seed_policy_rows(&conn);
        let scope = read_scope("user", "chat", 2, Some("owner-1"));
        let predicate = scope.predicate("r");

        // Rows that pass the SQL predicate:
        let sql = format!(
            "SELECT namespace, scope, sensitivity, owner_id FROM rows_under_test AS r WHERE {}",
            predicate.clause()
        );
        let mut stmt = conn.prepare(&sql).unwrap();
        let sql_rows: Vec<PolicyPartition> = stmt
            .query_map(params_from_iter(predicate.params().iter()), |row| {
                let ns: String = row.get(0)?;
                let sc: String = row.get(1)?;
                let sens: i64 = row.get(2)?;
                let owner: Option<String> = row.get(3)?;
                Ok(PolicyPartition::with_owner(ns, sc, sens as u8, owner).unwrap())
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        // Every SQL-admitted row is admitted in-memory, and vice versa over the seeds.
        for p in &sql_rows {
            assert!(
                scope.admits(p),
                "SQL admitted a row admits() rejects: {p:?}"
            );
        }
        assert_eq!(sql_rows.len(), 2);
    }

    // ── (b) cache/cursor keys differ when the policy differs ────────────

    #[test]
    fn cache_and_cursor_keys_diverge_across_policies() {
        let schema = SchemaVersion::new(0x4D475205);
        let rev = GraphRevision::new(7);
        let query_hash = "qh-abc";

        // Same caller partition, two different effective policies (different
        // capability sets → different provenance hash → different policy hash).
        let part = partition("user", "chat", 2, None);
        let policy_a = read_policy(part.clone(), &[Capability::ReadCore]);
        let policy_b = read_policy(
            part.clone(),
            &[Capability::ReadCore, Capability::ObserveMemory],
        );
        let scope_a = authorize_read(&caller(part.clone()), &policy_a).unwrap();
        let scope_b = authorize_read(&caller(part), &policy_b).unwrap();

        assert_ne!(scope_a.policy_hash(), scope_b.policy_hash());
        assert_ne!(
            scope_a.cache_key(schema, rev, query_hash),
            scope_b.cache_key(schema, rev, query_hash),
            "a cache entry under one policy must never be reused under another"
        );
        assert_ne!(
            scope_a.cursor_key(rev, query_hash, "page-0"),
            scope_b.cursor_key(rev, query_hash, "page-0"),
        );

        // A different caller clearance (partition) also diverges.
        let scope_c = read_scope("user", "notes", 2, None);
        assert_ne!(
            scope_a.cache_key(schema, rev, query_hash),
            scope_c.cache_key(schema, rev, query_hash)
        );

        // Identical authorization → identical, reusable key.
        let scope_a2 =
            authorize_read(&caller(partition("user", "chat", 2, None)), &policy_a).unwrap();
        assert_eq!(
            scope_a.cache_key(schema, rev, query_hash),
            scope_a2.cache_key(schema, rev, query_hash)
        );
    }

    // ── (c) policy-safe projection / DTO omits unauthorized fields ──────

    /// A minimal DTO-like scoped item carrying a private label.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Candidate {
        label: String,
        partition: PolicyPartition,
    }
    impl ScopedItem for Candidate {
        fn policy_partition(&self) -> &PolicyPartition {
            &self.partition
        }
    }

    fn candidate(label: &str, p: PolicyPartition) -> Candidate {
        Candidate {
            label: label.to_string(),
            partition: p,
        }
    }

    #[test]
    fn admit_candidates_omits_unauthorized_items() {
        let scope = read_scope("user", "chat", 2, Some("owner-1"));
        let items = vec![
            candidate("auth-a", partition("user", "chat", 0, None)),
            candidate("hidden-hi", partition("user", "chat", 3, None)),
            candidate("auth-b", partition("user", "chat", 2, Some("owner-1"))),
            candidate(
                "hidden-owner",
                partition("user", "chat", 0, Some("owner-2")),
            ),
        ];
        let authorized = scope.admit_candidates(items);
        let labels: Vec<&str> = authorized
            .items()
            .iter()
            .map(|c| c.label.as_str())
            .collect();
        assert_eq!(labels, vec!["auth-a", "auth-b"]);
        // The hidden labels never appear in the authorized DTO set.
        assert!(!labels.iter().any(|l| l.starts_with("hidden")));
        assert_eq!(authorized.policy_hash(), scope.policy_hash());
    }

    // ── (d) counts reflect only authorized rows ─────────────────────────

    #[test]
    fn authorized_total_counts_only_authorized_rows() {
        let scope = read_scope("user", "chat", 2, None);
        let rows = vec![
            partition("user", "chat", 0, None),
            partition("user", "chat", 2, None),
            partition("user", "chat", 3, None), // hidden (sensitivity)
            partition("system", "chat", 0, None), // hidden (namespace)
        ];
        assert_eq!(scope.authorized_total(rows.iter()), 2);
    }

    // ── (e) paired-world non-interference (V-POLICY-02 minimal) ──────────

    #[test]
    fn hidden_record_does_not_change_authorized_observable_output() {
        let scope = read_scope("user", "chat", 2, Some("owner-1"));

        // World 1: only authorized records.
        let world1 = vec![
            candidate("a", partition("user", "chat", 0, None)),
            candidate("b", partition("user", "chat", 2, Some("owner-1"))),
        ];
        // World 2: identical authorized records PLUS hidden records that the
        // caller must never observe.
        let world2 = vec![
            candidate("a", partition("user", "chat", 0, None)),
            candidate("secret-1", partition("user", "chat", 3, None)),
            candidate("b", partition("user", "chat", 2, Some("owner-1"))),
            candidate("secret-2", partition("user", "chat", 0, Some("owner-9"))),
            candidate("secret-3", partition("system", "chat", 0, None)),
        ];

        let auth1 = scope.admit_candidates(world1.clone());
        let auth2 = scope.admit_candidates(world2.clone());

        // Observable count is identical across the paired worlds.
        assert_eq!(auth1.len(), auth2.len());
        // Observable labels (serialization output) are identical.
        let labels1: Vec<&str> = auth1.items().iter().map(|c| c.label.as_str()).collect();
        let labels2: Vec<&str> = auth2.items().iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels1, labels2);
        // Authorized totals over the raw worlds match too.
        assert_eq!(
            scope.authorized_total(world1.iter()),
            scope.authorized_total(world2.iter())
        );
        // Redacted refs computed only over authorized ids would be identical;
        // no hidden label appears anywhere in the observable projection.
        assert!(!labels2.iter().any(|l| l.starts_with("secret")));
    }

    // ── retain_authorized: traversal-expansion hook ─────────────────────

    #[test]
    fn retain_authorized_filters_frontier_before_expansion() {
        let scope = read_scope("user", "chat", 1, None);
        let mut frontier = vec![
            partition("user", "chat", 0, None),
            partition("user", "chat", 2, None), // hidden
            partition("user", "chat", 1, None),
        ];
        scope.retain_authorized(&mut frontier);
        assert_eq!(frontier.len(), 2);
        assert!(frontier.iter().all(|p| p.sensitivity() <= 1));
    }

    // ── Policy-safe observability (MGR-028) ─────────────────────────────

    #[test]
    fn redacted_ref_hides_id_and_differs_across_policies() {
        let rid = RecordId::new_v7();
        let scope_a = read_scope("user", "chat", 2, None);
        let scope_b = read_scope("user", "notes", 2, None);

        let ref_a = scope_a.redacted_ref(&rid);
        let ref_a2 = scope_a.redacted_ref(&rid);
        let ref_b = scope_b.redacted_ref(&rid);

        // Stable within a policy…
        assert_eq!(ref_a, ref_a2);
        // …never reveals the raw id…
        assert_ne!(ref_a.as_str(), rid.as_str());
        assert!(!ref_a.as_str().contains(rid.as_str()));
        // …and cannot be correlated across policies.
        assert_ne!(ref_a, ref_b);
    }

    #[test]
    fn policy_safe_log_contains_no_content_only_safe_fields() {
        let scope = read_scope("user", "chat", 2, None);
        let rid = RecordId::new_v7();
        let line = scope
            .log("corr-123")
            .with_authorized_count(4)
            .with_ref(scope.redacted_ref(&rid))
            .render();
        assert!(line.contains("corr=corr-123"));
        assert!(line.contains(&format!("policy={}", scope.policy_hash())));
        assert!(line.contains("authorized_count=4"));
        // No raw record id leaks into the log line.
        assert!(!line.contains(rid.as_str()));
    }

    // ── Property: authorize_read is deterministic + admits ⇔ predicate ──

    fn partition_strategy() -> impl Strategy<Value = PolicyPartition> {
        (
            prop_oneof![Just("user"), Just("system"), Just("work")],
            prop_oneof![Just("chat"), Just("notes"), Just("code")],
            0u8..=3,
            prop_oneof![
                Just(None),
                Just(Some("owner-1".to_string())),
                Just(Some("owner-2".to_string())),
            ],
        )
            .prop_map(|(ns, sc, sens, owner)| {
                PolicyPartition::with_owner(ns, sc, sens, owner).unwrap()
            })
    }

    proptest! {
        /// The in-memory `admits` decision must exactly match the SQL predicate
        /// over an arbitrary caller clearance and record partition — so a stage
        /// that filters in SQL and a stage that filters in memory can never
        /// disagree about visibility (no post-filter leak).
        #[test]
        fn prop_admits_matches_sql_predicate(
            caller_part in partition_strategy(),
            record_part in partition_strategy(),
        ) {
            let policy = read_policy(caller_part.clone(), &[Capability::ReadCore]);
            let scope = authorize_read(&caller(caller_part), &policy).unwrap();

            let conn = rusqlite::Connection::open_in_memory().unwrap();
            conn.execute_batch(
                "CREATE TABLE r(namespace TEXT, scope TEXT, sensitivity INTEGER, owner_id TEXT);",
            ).unwrap();
            conn.execute(
                "INSERT INTO r(namespace, scope, sensitivity, owner_id) VALUES (?1, ?2, ?3, ?4)",
                params![
                    record_part.namespace(),
                    record_part.scope(),
                    i64::from(record_part.sensitivity()),
                    record_part.owner_id(),
                ],
            ).unwrap();

            let predicate = scope.predicate("r");
            let sql = format!("SELECT COUNT(*) FROM r WHERE {}", predicate.clause());
            let sql_visible: i64 = conn
                .query_row(&sql, params_from_iter(predicate.params().iter()), |row| row.get(0))
                .unwrap();

            prop_assert_eq!(sql_visible == 1, scope.admits(&record_part));
        }

        /// Non-interference: appending any number of hidden (unauthorized)
        /// records never changes the authorized count.
        #[test]
        fn prop_hidden_records_do_not_change_authorized_count(
            authorized_sens in 0u8..=2,
            extra_hidden in prop::collection::vec(partition_strategy(), 0..8),
        ) {
            let scope = read_scope("user", "chat", 2, None);
            // A fixed set of authorized rows.
            let base = vec![
                partition("user", "chat", 0, None),
                partition("user", "chat", authorized_sens, None),
            ];
            let base_count = scope.authorized_total(base.iter());

            // World with hidden rows appended.
            let mut world = base.clone();
            for p in &extra_hidden {
                // Only keep the ones that are actually hidden for this scope, so
                // the appended set never accidentally adds authorized rows.
                if !scope.admits(p) {
                    world.push(p.clone());
                }
            }
            prop_assert_eq!(base_count, scope.authorized_total(world.iter()));
        }
    }
}
