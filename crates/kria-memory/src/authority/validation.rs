//! Pre-transaction command validation (task **F1.3.2**, design §5.1 command
//! state machine `Validate --> {Rejected, Replay, Previewed, TxOpen}`).
//!
//! This is the **"Validate before BEGIN"** stage the authority runs *before* it
//! opens any SQL transaction. It consumes an already-constructed
//! [`CommandEnvelope`] (F1.3.1) and produces a typed [`ValidationOutcome`] the
//! transaction stage (F1.3.3+) consumes:
//!
//! * [`ValidationOutcome::Proceed`] — all checks passed; open the transaction.
//! * [`ValidationOutcome::Replay`] — a matching idempotency result already
//!   exists (same partition/key/hash); return the stored result and do **not**
//!   re-execute (MGR-005 AC3, MGR-033 AC6).
//! * [`ValidationOutcome::Rejected`] — one or more deterministic checks failed;
//!   the carried [`RejectionReason`] codes are recorded verbatim as the
//!   `audit_records.reason_codes_json` of the rejected command (F1.3.4).
//!
//! ## Deterministic validation order (design §5.1, MGR-035 AC2)
//!
//! The stage evaluates checks in a fixed, deterministic order so the same
//! envelope always yields the same outcome and reason ordering:
//!
//! 1. **schema** — the command's declared content-schema version is supported
//!    (unknown/too-new is denied, MGR-034 AC5).
//! 2. **caller capability** — the caller origin is permitted to issue this
//!    [`CommandKind`] (design §8 capability table).
//! 3. **mode** — [`MemoryMode`] admission via the existing deterministic mode
//!    gate ([`crate::modes::evaluate`], MGR-035 AC4–AC7).
//! 4. **identity** — caller identity/partition is well-formed and its asserted
//!    provenance trust is consistent with the caller origin (design §19.8: a
//!    remote caller can never assert local `System` trust).
//! 5. **limits** — payload byte size and execution deadline are within bounded
//!    ceilings (A6 boundedness).
//! 6. **policy inputs** — the policy-relevant inputs the F1.4 Effective-Policy
//!    meet will consume are present and well-shaped. This is **presence/shape
//!    only**; the restrictive-meet lattice itself is F1.4.
//! 7. **idempotency replay / hash conflict** — look up
//!    `idempotency_results (caller_partition, idempotency_key)`; a matching
//!    `command_hash` replays the stored result, a *different* hash is an
//!    [`RejectionCode::IdempotencyConflict`] (MGR-005 AC3, design §4.1).
//! 8. **destructive-preview freshness** — a command that
//!    [`requires_preview`](CommandEnvelope::requires_preview) confirms against a
//!    base revision; if the authority has advanced past that base revision the
//!    preview is stale ([`RejectionCode::RevisionConflict`], design §5.1
//!    `Previewed --> Validate: confirm with base revision`, §8.4 stale-preview).
//!
//! Checks 1–6 are pure (no I/O) and are all evaluated so every applicable
//! reason is reported together. Checks 7–8 require read-only lookups
//! ([`ValidationReads`]) and only run once the pure checks pass — the reads
//! never synchronously write access counters (parent-task invariant).

use serde::Serialize;

use crate::error::MemoryResult;
use crate::model::{CallerOrigin, EventId, GraphRevision, IdempotencyKey};
use crate::modes::{self, ModeWriteContext, ModeWriteDecision};

use super::command::{CommandEnvelope, SourceTrust};
use super::CommandKind;

/// The command content-schema version this authority build supports. The v2
/// authority epoch is `2` (see `authority_meta.schema_epoch`); a command that
/// declares a newer content schema is denied rather than misinterpreted
/// (MGR-034 AC5).
pub const SUPPORTED_COMMAND_SCHEMA: u32 = 2;

/// Default maximum canonical payload size, in bytes (A6 boundedness). Bounded so
/// a hostile/buggy caller cannot submit an unbounded command body.
pub const DEFAULT_MAX_PAYLOAD_BYTES: usize = 256 * 1024;

/// The single caller-capability decision (design §8 capability table): is
/// `origin` permitted to issue `kind`? A locally-trusted desktop caller may
/// issue every kind; a transport-authenticated remote caller may only
/// `Observe` by default — destructive/corrective kinds are disabled until an
/// explicit operation grant, which the F1.4 Effective-Policy layer introduces.
///
/// This is the *one* place the origin→kind lattice is decided. It backs
/// [`CommandValidator::check_capability`] for commands that already reached
/// the governed [`super::AuthorityCommandBus`], **and** it is public so an
/// adapter boundary that has not yet been cut over to the command bus (task
/// F1.5.3's Axum routes, still calling the pre-authority [`WritePolicy`]
/// engine because the F2 semantic builders do not exist yet) can reject an
/// unsupported remote mutation *before* touching any store — matching design
/// §8.3's capability matrix ("disabled by default; explicit operation
/// grants") without duplicating the decision or waiting for F2.
///
/// [`WritePolicy`]: crate::write_policy::WritePolicy
pub fn is_command_capability_permitted(origin: CallerOrigin, kind: CommandKind) -> bool {
    match origin {
        CallerOrigin::LocalDesktop => true,
        CallerOrigin::AuthenticatedRemote => matches!(kind, CommandKind::Observe),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Rejection reason codes
// ─────────────────────────────────────────────────────────────────────────

/// A stable rejection reason code. These mirror the design's
/// `MemoryApiErrorCodeV2` (§8.2) so the pre-transaction stage, the audit
/// `reason_codes_json`, and the adapter error surface all speak one vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionCode {
    /// The command declared a content-schema version this build cannot honor.
    UnsupportedSchema,
    /// The caller origin is not permitted to issue this command kind.
    UnsupportedCapability,
    /// The active [`MemoryMode`] forbids this durable write.
    ModeRejected,
    /// The caller identity/partition/provenance is malformed or inconsistent.
    InvalidIdentity,
    /// A bounded limit (payload size, deadline) was exceeded.
    LimitExceeded,
    /// A required policy input for the command was missing or malformed.
    InvalidPolicyInput,
    /// The idempotency key was reused with a *different* command hash.
    IdempotencyConflict,
    /// A destructive/corrective command's preview is stale (the authority has
    /// advanced past its base revision).
    RevisionConflict,
}

impl RejectionCode {
    /// The canonical snake_case text (stable for `reason_codes_json` and logs).
    pub fn as_str(self) -> &'static str {
        match self {
            RejectionCode::UnsupportedSchema => "unsupported_schema",
            RejectionCode::UnsupportedCapability => "unsupported_capability",
            RejectionCode::ModeRejected => "mode_rejected",
            RejectionCode::InvalidIdentity => "invalid_identity",
            RejectionCode::LimitExceeded => "limit_exceeded",
            RejectionCode::InvalidPolicyInput => "invalid_policy_input",
            RejectionCode::IdempotencyConflict => "idempotency_conflict",
            RejectionCode::RevisionConflict => "revision_conflict",
        }
    }
}

impl std::fmt::Display for RejectionCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single rejection reason: a stable [`RejectionCode`] plus a bounded,
/// human-readable detail. Serialized into `audit_records.reason_codes_json`
/// (F1.3.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RejectionReason {
    /// The stable reason code.
    pub code: RejectionCode,
    /// A short explanation (safe to log; never carries secret content).
    pub detail: String,
}

impl RejectionReason {
    /// Build a rejection reason.
    fn new(code: RejectionCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Stored idempotency result + read-only lookups
// ─────────────────────────────────────────────────────────────────────────

/// The stored outcome of a previously committed command, read back from
/// `idempotency_results` (design §4.1). On a matching replay the transaction
/// stage returns this verbatim instead of re-executing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredIdempotencyResult {
    /// The canonical command hash stored for the original command (raw hex text
    /// as persisted; compared against the envelope's current hash).
    pub command_hash: String,
    /// The serialized original result (`idempotency_results.result_json`).
    pub result_json: String,
    /// The revision the original command committed at, if it was graph-visible.
    pub committed_revision: Option<GraphRevision>,
    /// The completion event id of the original command, if one was appended.
    pub event_id: Option<EventId>,
}

/// The read-only lookups the pre-transaction validation stage needs.
///
/// Implementations MUST be side-effect free: they read the WAL snapshot and
/// never synchronously write (no access counters, no touch timestamps) — the
/// parent-task invariant "reads never synchronously write access counters".
pub trait ValidationReads {
    /// Look up an existing idempotency result by its
    /// `(caller_partition, idempotency_key)` composite key, or `None` if the
    /// caller has never committed under that key.
    fn lookup_idempotency(
        &self,
        caller_partition: &str,
        idempotency_key: &IdempotencyKey,
    ) -> MemoryResult<Option<StoredIdempotencyResult>>;

    /// The current authority revision (`authority_meta.graph_revision`), used to
    /// judge destructive-preview freshness.
    fn current_revision(&self) -> MemoryResult<GraphRevision>;
}

// ─────────────────────────────────────────────────────────────────────────
// Validation configuration
// ─────────────────────────────────────────────────────────────────────────

/// Bounded limits/ceilings the validation stage enforces (A6). Constructed once
/// at the composition root and shared; every field has a safe default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidationConfig {
    /// Highest command content-schema version accepted.
    pub max_supported_schema: u32,
    /// Maximum canonical payload size, in bytes.
    pub max_payload_bytes: usize,
    /// Maximum execution deadline budget, in milliseconds.
    pub max_deadline_ms: u64,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            max_supported_schema: SUPPORTED_COMMAND_SCHEMA,
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
            max_deadline_ms: super::command::Deadline::MAX_MS,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Validation outcome
// ─────────────────────────────────────────────────────────────────────────

/// The typed result of the pre-transaction validation stage. The transaction
/// stage (F1.3.3+) matches on this to decide whether to open a transaction,
/// return a stored result, or record a rejection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationOutcome {
    /// All checks passed; the caller may open the authority transaction.
    Proceed,
    /// A matching idempotency result already exists; return it without
    /// re-executing (MGR-005 AC3, MGR-033 AC6).
    Replay(StoredIdempotencyResult),
    /// The command was rejected; the reasons are recorded verbatim in audit.
    /// Never empty.
    Rejected(Vec<RejectionReason>),
}

impl ValidationOutcome {
    /// Whether the outcome permits opening the transaction.
    pub fn is_proceed(&self) -> bool {
        matches!(self, ValidationOutcome::Proceed)
    }

    /// The rejection reasons, if this outcome is a rejection.
    pub fn rejection_reasons(&self) -> Option<&[RejectionReason]> {
        match self {
            ValidationOutcome::Rejected(reasons) => Some(reasons),
            _ => None,
        }
    }

    /// Whether the rejection set contains a given code (convenience for tests
    /// and adapters mapping to error responses).
    pub fn has_code(&self, code: RejectionCode) -> bool {
        self.rejection_reasons()
            .is_some_and(|rs| rs.iter().any(|r| r.code == code))
    }
}

// ─────────────────────────────────────────────────────────────────────────
// The validator
// ─────────────────────────────────────────────────────────────────────────

/// The pre-transaction command validator. Holds the bounded [`ValidationConfig`]
/// and borrows the read-only [`ValidationReads`] surface; it owns no SQL.
pub struct CommandValidator<'r, R: ValidationReads + ?Sized> {
    config: ValidationConfig,
    reads: &'r R,
}

impl<'r, R: ValidationReads + ?Sized> CommandValidator<'r, R> {
    /// Build a validator with the given config over a read surface.
    pub fn new(config: ValidationConfig, reads: &'r R) -> Self {
        Self { config, reads }
    }

    /// Build a validator with default limits.
    pub fn with_defaults(reads: &'r R) -> Self {
        Self::new(ValidationConfig::default(), reads)
    }

    /// Run the full deterministic validation stage over `env`.
    ///
    /// Returns `Ok(ValidationOutcome)` for every *normal* outcome — including a
    /// rejection, which is not an error. `Err` is reserved for genuine storage
    /// failures encountered during the idempotency/revision reads.
    pub fn validate(&self, env: &CommandEnvelope) -> MemoryResult<ValidationOutcome> {
        // ── Pure checks 1–6 (no I/O): collect every applicable reason. ──────
        let mut reasons = Vec::new();
        self.check_schema(env, &mut reasons);
        self.check_capability(env, &mut reasons);
        self.check_mode(env, &mut reasons);
        self.check_identity(env, &mut reasons);
        self.check_limits(env, &mut reasons);
        self.check_policy_inputs(env, &mut reasons);
        if !reasons.is_empty() {
            return Ok(ValidationOutcome::Rejected(reasons));
        }

        // ── Check 7 (read): idempotency replay / hash conflict. ─────────────
        let caller_partition = env.caller().partition_key();
        if let Some(stored) = self
            .reads
            .lookup_idempotency(&caller_partition, env.idempotency_key())?
        {
            if stored.command_hash == env.command_hash().as_str() {
                // Same key + same hash → legitimate replay of the same intent.
                return Ok(ValidationOutcome::Replay(stored));
            }
            // Same key + different hash → conflict (MGR-005 AC3).
            return Ok(ValidationOutcome::Rejected(vec![RejectionReason::new(
                RejectionCode::IdempotencyConflict,
                format!(
                    "idempotency key {:?} was already used with a different command",
                    env.idempotency_key().as_str()
                ),
            )]));
        }

        // ── Check 8 (read): destructive-preview freshness. ──────────────────
        if env.requires_preview() {
            let current = self.reads.current_revision()?;
            let base = env.base_revision();
            if current != base {
                return Ok(ValidationOutcome::Rejected(vec![RejectionReason::new(
                    RejectionCode::RevisionConflict,
                    format!(
                        "preview is stale: computed against revision {base} but authority is at revision {current}"
                    ),
                )]));
            }
        }

        Ok(ValidationOutcome::Proceed)
    }

    // ── 1. schema ───────────────────────────────────────────────────────
    /// The command's declared content-schema version (an optional
    /// `schema_version` field on the payload object) must be a positive integer
    /// no greater than [`ValidationConfig::max_supported_schema`]. A missing
    /// field defaults to the current schema (accepted); a non-integer or
    /// out-of-range value is denied (MGR-034 AC5).
    fn check_schema(&self, env: &CommandEnvelope, reasons: &mut Vec<RejectionReason>) {
        let Some(version) = env.payload().get("schema_version") else {
            return; // absent → current schema
        };
        match version.as_u64() {
            Some(v) if v >= 1 && v <= self.config.max_supported_schema as u64 => {}
            Some(v) => reasons.push(RejectionReason::new(
                RejectionCode::UnsupportedSchema,
                format!(
                    "command schema_version {v} is not supported (max {})",
                    self.config.max_supported_schema
                ),
            )),
            None => reasons.push(RejectionReason::new(
                RejectionCode::UnsupportedSchema,
                "command schema_version must be a positive integer",
            )),
        }
    }

    // ── 2. caller capability ──────────────────────────────────────────────
    /// The caller origin must be permitted to issue this command kind (design
    /// §8 capability table). Delegates to the shared, adapter-reusable
    /// [`is_command_capability_permitted`] so no boundary (Tauri, Axum, or the
    /// command bus itself) can drift from a second copy of this decision
    /// (F1.5.3: the server Axum routes gate their pre-authority legacy writes
    /// with the exact same function).
    fn check_capability(&self, env: &CommandEnvelope, reasons: &mut Vec<RejectionReason>) {
        if !is_command_capability_permitted(env.caller().origin(), env.kind()) {
            reasons.push(RejectionReason::new(
                RejectionCode::UnsupportedCapability,
                format!(
                    "caller origin {} may not issue command kind {:?} (grant required)",
                    env.caller().origin(),
                    env.kind()
                ),
            ));
        }
    }

    // ── 3. mode ───────────────────────────────────────────────────────────
    /// Delegate to the deterministic mode gate ([`crate::modes`]). Every
    /// command reaching this stage is a durable write, so a `Reject` decision
    /// blocks it (MGR-035 AC4–AC7). `Allow`/`AllowSessionScoped` both pass here;
    /// session-scoping is applied inside the transaction.
    fn check_mode(&self, env: &CommandEnvelope, reasons: &mut Vec<RejectionReason>) {
        let ctx = ModeWriteContext {
            is_personal_scope: env.caller().partition().scope() == "personal",
            is_library_ingest: env.source().source_kind() == super::SourceKind::Library,
        };
        if let ModeWriteDecision::Reject(reason) = modes::evaluate(env.mode(), &ctx) {
            reasons.push(RejectionReason::new(
                RejectionCode::ModeRejected,
                format!("mode {} forbids this write: {reason:?}", env.mode()),
            ));
        }
    }

    // ── 4. identity ───────────────────────────────────────────────────────
    /// The caller identity/partition must be well-formed (guaranteed by the
    /// value objects) and its asserted provenance trust must be consistent with
    /// the caller origin: only a locally-trusted [`CallerOrigin::LocalDesktop`]
    /// caller may assert [`SourceTrust::System`] provenance. A remote caller
    /// claiming `System` trust would be escalating scope, which the authority
    /// forbids (design §19.8).
    fn check_identity(&self, env: &CommandEnvelope, reasons: &mut Vec<RejectionReason>) {
        // Defensive: the partition key must be non-empty (guaranteed by the
        // PolicyPartition value object, re-asserted here at the boundary).
        if env.caller().partition_key().trim().is_empty() {
            reasons.push(RejectionReason::new(
                RejectionCode::InvalidIdentity,
                "caller partition key must not be empty",
            ));
        }
        if env.source().trust() == SourceTrust::System
            && env.caller().origin() != CallerOrigin::LocalDesktop
        {
            reasons.push(RejectionReason::new(
                RejectionCode::InvalidIdentity,
                format!(
                    "source asserts System trust but caller origin is {} (only local callers may assert System trust)",
                    env.caller().origin()
                ),
            ));
        }
    }

    // ── 5. limits ───────────────────────────────────────────────────────
    /// Payload byte size and execution deadline must be within bounded ceilings
    /// (A6). The deadline is already capped by the [`Deadline`](super::command::Deadline)
    /// value object; this re-asserts a (possibly stricter) config ceiling.
    fn check_limits(&self, env: &CommandEnvelope, reasons: &mut Vec<RejectionReason>) {
        let payload_bytes = serde_json::to_vec(env.payload())
            .map(|b| b.len())
            .unwrap_or(usize::MAX);
        if payload_bytes > self.config.max_payload_bytes {
            reasons.push(RejectionReason::new(
                RejectionCode::LimitExceeded,
                format!(
                    "payload size {payload_bytes} bytes exceeds max {}",
                    self.config.max_payload_bytes
                ),
            ));
        }
        if env.deadline().budget_ms() > self.config.max_deadline_ms {
            reasons.push(RejectionReason::new(
                RejectionCode::LimitExceeded,
                format!(
                    "deadline {}ms exceeds max {}ms",
                    env.deadline().budget_ms(),
                    self.config.max_deadline_ms
                ),
            ));
        }
    }

    // ── 6. policy inputs ──────────────────────────────────────────────────
    /// The policy-relevant inputs the F1.4 Effective-Policy meet will consume
    /// must be present and well-shaped. This is **presence/shape only**: the
    /// payload must be a JSON object (so per-kind policy fields can be read from
    /// it downstream) and the caller partition must be present. The restrictive
    /// source-trust/capability lattice is deferred to F1.4.
    fn check_policy_inputs(&self, env: &CommandEnvelope, reasons: &mut Vec<RejectionReason>) {
        if !env.payload().is_object() {
            reasons.push(RejectionReason::new(
                RejectionCode::InvalidPolicyInput,
                "command payload must be a JSON object carrying policy inputs",
            ));
        }
    }
}

/// Free-function convenience: validate `env` with default limits over `reads`.
pub fn validate_command<R: ValidationReads + ?Sized>(
    env: &CommandEnvelope,
    reads: &R,
) -> MemoryResult<ValidationOutcome> {
    CommandValidator::with_defaults(reads).validate(env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::command::{Deadline, PreviewToken, SourceContext, SourceKind};
    use crate::model::{CallerContext, InvocationId, PolicyPartition};
    use crate::types::MemoryMode;
    use std::cell::Cell;
    use std::collections::HashMap;

    // ── A controllable in-memory read surface ────────────────────────────
    struct FakeReads {
        idempotency: HashMap<(String, String), StoredIdempotencyResult>,
        current: GraphRevision,
        read_calls: Cell<u32>,
    }

    impl FakeReads {
        fn with_revision(current: GraphRevision) -> Self {
            Self {
                idempotency: HashMap::new(),
                current,
                read_calls: Cell::new(0),
            }
        }

        fn insert(&mut self, partition: &str, key: &str, stored: StoredIdempotencyResult) {
            self.idempotency
                .insert((partition.to_string(), key.to_string()), stored);
        }
    }

    impl ValidationReads for FakeReads {
        fn lookup_idempotency(
            &self,
            caller_partition: &str,
            idempotency_key: &IdempotencyKey,
        ) -> MemoryResult<Option<StoredIdempotencyResult>> {
            self.read_calls.set(self.read_calls.get() + 1);
            Ok(self
                .idempotency
                .get(&(
                    caller_partition.to_string(),
                    idempotency_key.as_str().to_string(),
                ))
                .cloned())
        }

        fn current_revision(&self) -> MemoryResult<GraphRevision> {
            Ok(self.current)
        }
    }

    fn partition() -> PolicyPartition {
        PolicyPartition::new("user", "chat", 0).unwrap()
    }

    fn local_caller() -> CallerContext {
        CallerContext::local_desktop("local-desktop", partition()).unwrap()
    }

    fn remote_caller() -> CallerContext {
        CallerContext::authenticated_remote("actor-1", "device-1", partition()).unwrap()
    }

    fn source(trust: SourceTrust) -> SourceContext {
        SourceContext::new(
            InvocationId::new_v7(),
            SourceKind::Native,
            "core:cognition",
            trust,
        )
        .unwrap()
    }

    fn observe(caller: CallerContext, payload: serde_json::Value) -> CommandEnvelope {
        CommandEnvelope::new(
            caller,
            CommandKind::Observe,
            IdempotencyKey::new("cmd-1").unwrap(),
            GraphRevision::base(),
            source(SourceTrust::System),
            MemoryMode::Permanent,
            Deadline::default_write(),
            payload,
            None,
        )
        .unwrap()
    }

    fn forget(
        caller: CallerContext,
        base_revision: GraphRevision,
        mode: MemoryMode,
    ) -> CommandEnvelope {
        CommandEnvelope::new(
            caller,
            CommandKind::Forget,
            IdempotencyKey::new("cmd-forget").unwrap(),
            base_revision,
            source(SourceTrust::System),
            mode,
            Deadline::default_write(),
            serde_json::json!({"target": "rec-1"}),
            Some(PreviewToken::new("tok-1").unwrap()),
        )
        .unwrap()
    }

    // ── Happy path ───────────────────────────────────────────────────────
    #[test]
    fn observe_proceeds_when_all_checks_pass() {
        let reads = FakeReads::with_revision(GraphRevision::base());
        let env = observe(local_caller(), serde_json::json!({"content": "hello"}));
        let outcome = validate_command(&env, &reads).unwrap();
        assert_eq!(outcome, ValidationOutcome::Proceed);
    }

    // ── 7. idempotency replay (same key + hash) ──────────────────────────
    #[test]
    fn replay_returns_stored_result_for_same_key_and_hash() {
        let env = observe(local_caller(), serde_json::json!({"content": "hello"}));
        let mut reads = FakeReads::with_revision(GraphRevision::base());
        let stored = StoredIdempotencyResult {
            command_hash: env.command_hash().as_str().to_string(),
            result_json: r#"{"status":"committed"}"#.to_string(),
            committed_revision: Some(GraphRevision::new(1)),
            event_id: Some(EventId::new_v7()),
        };
        reads.insert(&env.caller().partition_key(), "cmd-1", stored.clone());

        let outcome = validate_command(&env, &reads).unwrap();
        assert_eq!(outcome, ValidationOutcome::Replay(stored));
    }

    // ── 7. idempotency conflict (same key, different hash) ────────────────
    #[test]
    fn conflict_when_same_key_different_hash() {
        let env = observe(local_caller(), serde_json::json!({"content": "hello"}));
        let mut reads = FakeReads::with_revision(GraphRevision::base());
        reads.insert(
            &env.caller().partition_key(),
            "cmd-1",
            StoredIdempotencyResult {
                command_hash: "a-totally-different-hash".to_string(),
                result_json: "{}".to_string(),
                committed_revision: Some(GraphRevision::new(1)),
                event_id: None,
            },
        );

        let outcome = validate_command(&env, &reads).unwrap();
        assert!(outcome.has_code(RejectionCode::IdempotencyConflict));
    }

    // ── 1. schema rejection ──────────────────────────────────────────────
    #[test]
    fn rejects_unsupported_schema_version() {
        let reads = FakeReads::with_revision(GraphRevision::base());
        let env = observe(
            local_caller(),
            serde_json::json!({"content": "x", "schema_version": 9999}),
        );
        let outcome = validate_command(&env, &reads).unwrap();
        assert!(outcome.has_code(RejectionCode::UnsupportedSchema));
    }

    #[test]
    fn accepts_supported_schema_version() {
        let reads = FakeReads::with_revision(GraphRevision::base());
        let env = observe(
            local_caller(),
            serde_json::json!({"content": "x", "schema_version": SUPPORTED_COMMAND_SCHEMA}),
        );
        assert_eq!(
            validate_command(&env, &reads).unwrap(),
            ValidationOutcome::Proceed
        );
    }

    // ── 2. capability rejection ──────────────────────────────────────────
    #[test]
    fn rejects_remote_caller_destructive_capability() {
        // Remote callers may not issue Forget without a grant. Use a remote
        // caller with non-System trust to isolate the capability failure.
        let env = CommandEnvelope::new(
            remote_caller(),
            CommandKind::Forget,
            IdempotencyKey::new("cmd-forget").unwrap(),
            GraphRevision::base(),
            source(SourceTrust::Trusted),
            MemoryMode::Permanent,
            Deadline::default_write(),
            serde_json::json!({"target": "rec-1"}),
            Some(PreviewToken::new("tok-1").unwrap()),
        )
        .unwrap();
        let reads = FakeReads::with_revision(GraphRevision::base());
        let outcome = validate_command(&env, &reads).unwrap();
        assert!(outcome.has_code(RejectionCode::UnsupportedCapability));
    }

    #[test]
    fn remote_caller_may_observe() {
        let reads = FakeReads::with_revision(GraphRevision::base());
        // Remote + Observe + non-System trust (System trust would need a local
        // caller — see identity check).
        let env = CommandEnvelope::new(
            remote_caller(),
            CommandKind::Observe,
            IdempotencyKey::new("cmd-obs").unwrap(),
            GraphRevision::base(),
            source(SourceTrust::Trusted),
            MemoryMode::Permanent,
            Deadline::default_write(),
            serde_json::json!({"content": "hi"}),
            None,
        )
        .unwrap();
        assert_eq!(
            validate_command(&env, &reads).unwrap(),
            ValidationOutcome::Proceed
        );
    }

    // ── 3. mode rejection ────────────────────────────────────────────────
    #[test]
    fn rejects_read_only_mode() {
        let reads = FakeReads::with_revision(GraphRevision::base());
        let env = CommandEnvelope::new(
            local_caller(),
            CommandKind::Observe,
            IdempotencyKey::new("cmd-1").unwrap(),
            GraphRevision::base(),
            source(SourceTrust::System),
            MemoryMode::ReadOnly,
            Deadline::default_write(),
            serde_json::json!({"content": "hi"}),
            None,
        )
        .unwrap();
        let outcome = validate_command(&env, &reads).unwrap();
        assert!(outcome.has_code(RejectionCode::ModeRejected));
    }

    #[test]
    fn rejects_incognito_mode() {
        let reads = FakeReads::with_revision(GraphRevision::base());
        let env = CommandEnvelope::new(
            local_caller(),
            CommandKind::Observe,
            IdempotencyKey::new("cmd-1").unwrap(),
            GraphRevision::base(),
            source(SourceTrust::System),
            MemoryMode::Incognito,
            Deadline::default_write(),
            serde_json::json!({"content": "hi"}),
            None,
        )
        .unwrap();
        assert!(validate_command(&env, &reads)
            .unwrap()
            .has_code(RejectionCode::ModeRejected));
    }

    // ── 4. identity rejection ────────────────────────────────────────────
    #[test]
    fn rejects_remote_caller_asserting_system_trust() {
        // Remote + Observe (capability ok) but System trust → identity conflict.
        let env = CommandEnvelope::new(
            remote_caller(),
            CommandKind::Observe,
            IdempotencyKey::new("cmd-1").unwrap(),
            GraphRevision::base(),
            source(SourceTrust::System),
            MemoryMode::Permanent,
            Deadline::default_write(),
            serde_json::json!({"content": "hi"}),
            None,
        )
        .unwrap();
        let reads = FakeReads::with_revision(GraphRevision::base());
        assert!(validate_command(&env, &reads)
            .unwrap()
            .has_code(RejectionCode::InvalidIdentity));
    }

    // ── 5. limits rejection ──────────────────────────────────────────────
    #[test]
    fn rejects_oversized_payload() {
        let reads = FakeReads::with_revision(GraphRevision::base());
        let big = "x".repeat(1024);
        let env = observe(local_caller(), serde_json::json!({"content": big}));
        let config = ValidationConfig {
            max_payload_bytes: 64,
            ..ValidationConfig::default()
        };
        let outcome = CommandValidator::new(config, &reads)
            .validate(&env)
            .unwrap();
        assert!(outcome.has_code(RejectionCode::LimitExceeded));
    }

    // ── 6. policy-input rejection ────────────────────────────────────────
    #[test]
    fn rejects_non_object_payload() {
        let reads = FakeReads::with_revision(GraphRevision::base());
        let env = observe(local_caller(), serde_json::json!(["not", "an", "object"]));
        let outcome = validate_command(&env, &reads).unwrap();
        assert!(outcome.has_code(RejectionCode::InvalidPolicyInput));
    }

    // ── 8. destructive-preview freshness ─────────────────────────────────
    #[test]
    fn fresh_preview_proceeds() {
        // Forget confirming against base revision 5, authority also at 5 → fresh.
        let reads = FakeReads::with_revision(GraphRevision::new(5));
        let env = forget(local_caller(), GraphRevision::new(5), MemoryMode::Permanent);
        assert_eq!(
            validate_command(&env, &reads).unwrap(),
            ValidationOutcome::Proceed
        );
    }

    #[test]
    fn stale_preview_is_rejected() {
        // Preview computed at revision 5, but authority has advanced to 6.
        let reads = FakeReads::with_revision(GraphRevision::new(6));
        let env = forget(local_caller(), GraphRevision::new(5), MemoryMode::Permanent);
        let outcome = validate_command(&env, &reads).unwrap();
        assert!(outcome.has_code(RejectionCode::RevisionConflict));
    }

    // ── Ordering / aggregation ───────────────────────────────────────────
    #[test]
    fn pure_checks_aggregate_and_precede_reads() {
        // Read-only mode + oversized payload: both pure reasons reported, and no
        // idempotency read is performed (pure checks short-circuit before I/O).
        let reads = FakeReads::with_revision(GraphRevision::base());
        let big = "y".repeat(200);
        let env = CommandEnvelope::new(
            local_caller(),
            CommandKind::Observe,
            IdempotencyKey::new("cmd-1").unwrap(),
            GraphRevision::base(),
            source(SourceTrust::System),
            MemoryMode::ReadOnly,
            Deadline::default_write(),
            serde_json::json!({"content": big}),
            None,
        )
        .unwrap();
        let config = ValidationConfig {
            max_payload_bytes: 32,
            ..ValidationConfig::default()
        };
        let outcome = CommandValidator::new(config, &reads)
            .validate(&env)
            .unwrap();
        assert!(outcome.has_code(RejectionCode::ModeRejected));
        assert!(outcome.has_code(RejectionCode::LimitExceeded));
        assert_eq!(
            reads.read_calls.get(),
            0,
            "pure-check rejection must not touch the read surface"
        );
    }

    #[test]
    fn deadline_over_config_ceiling_is_rejected() {
        let reads = FakeReads::with_revision(GraphRevision::base());
        let env = CommandEnvelope::new(
            local_caller(),
            CommandKind::Observe,
            IdempotencyKey::new("cmd-1").unwrap(),
            GraphRevision::base(),
            source(SourceTrust::System),
            MemoryMode::Permanent,
            Deadline::from_millis(10_000).unwrap(),
            serde_json::json!({"content": "hi"}),
            None,
        )
        .unwrap();
        let config = ValidationConfig {
            max_deadline_ms: 2_000,
            ..ValidationConfig::default()
        };
        let outcome = CommandValidator::new(config, &reads)
            .validate(&env)
            .unwrap();
        assert!(outcome.has_code(RejectionCode::LimitExceeded));
    }
}
