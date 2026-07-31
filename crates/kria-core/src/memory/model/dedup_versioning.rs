//! Deterministic duplicate reuse/versioning and idempotent source event
//! identities for consent-gated source ingestion (design §4.1, task F2.6.4 /
//! MGR-046).
//!
//! ## Key behavioral rules (MGR-046)
//!
//! 1. **Deterministic key**: Same `source_id + sequence + item_identity_hash`
//!    → same `idempotency_key`. Retried ingestion operations produce the same
//!    event identity, enabling safe retries against the `idempotency_results`
//!    table (PK: `(caller_partition, idempotency_key)`).
//!
//! 2. **Duplicate detection** via [`DuplicateEvaluator::evaluate`]:
//!    - No stored item → `New` → `Skip` (just ingest; no duplicate action)
//!    - Same content AND version hash → `Unchanged` → `Skip`
//!    - Same content hash, different version hash → `VersionBump` → `Reuse`
//!    - Different content hash → `Changed` → `Version`
//!
//! 3. **Lifecycle events are idempotent**: Same `source_id + event_kind +
//!    timestamp_epoch_secs` → same idempotency key (truncated to seconds so
//!    sub-second retries within the same second collide intentionally).
//!
//! 4. **SHA-256 hash**: uses the `sha2` crate (workspace dependency), same
//!    as [`super::ingestion_chunk`].

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::ingestion_chunk::{ItemHash, SemanticCandidate};

// ── Internal helper ────────────────────────────────────────────────────────

/// SHA-256 hex of arbitrary bytes.
fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

// ── SourceEventKey ─────────────────────────────────────────────────────────

/// A deterministic idempotency key for a source ingestion event.
///
/// Given the same inputs, the same key is always produced.
/// This ensures retried ingestion operations produce the same event identity,
/// mapping safely to the `idempotency_results` table PK
/// `(caller_partition, idempotency_key)` (design §4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEventKey {
    /// The deterministic key string for the authority `idempotency_results`
    /// table.
    pub idempotency_key: String,
    /// The command hash (deterministic from the command inputs).
    pub command_hash: String,
}

impl SourceEventKey {
    /// Compute a deterministic idempotency key from source, chunk, and content
    /// hash.
    ///
    /// - `idempotency_key` = SHA-256(`"{source_id}:{sequence}:{item_identity_hash}"`)
    /// - `command_hash`    = SHA-256(`"{source_id}:{record_kind}:{content_hash}"`)
    ///
    /// The same input triple always produces the same key, so retried
    /// operations are collapsed at the `idempotency_results` boundary.
    pub fn compute(
        source_id: &str,
        sequence: u64,
        item_identity_hash: &str,
        record_kind: &str,
        content_hash: &str,
    ) -> Self {
        let idempotency_key = sha256_hex(&format!("{source_id}:{sequence}:{item_identity_hash}"));
        let command_hash = sha256_hex(&format!("{source_id}:{record_kind}:{content_hash}"));
        Self {
            idempotency_key,
            command_hash,
        }
    }
}

// ── DuplicateDecision ──────────────────────────────────────────────────────

/// The decision made for a duplicate ingestion item.
///
/// Returned alongside a [`VersionedItemState`] by
/// [`DuplicateEvaluator::evaluate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateDecision {
    /// Reuse the existing record without creating a new one.
    ///
    /// Applies when the content is unchanged but version metadata has bumped —
    /// the stored record is correct; only its version metadata needs updating.
    Reuse,
    /// Create a new versioned record (content changed since last ingestion).
    Version,
    /// Skip entirely (no action needed).
    ///
    /// Used for both new items (just ingest normally) and exact duplicates
    /// (no change at all).
    Skip,
}

// ── VersionedItemState ─────────────────────────────────────────────────────

/// The state of an ingestion candidate relative to the stored item, used to
/// drive the [`DuplicateDecision`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionedItemState {
    /// New item never seen before (no stored record exists).
    New,
    /// Same content hash and version hash as stored — no change at all.
    Unchanged,
    /// Content hash differs from stored — a new version is needed.
    Changed,
    /// Content hash matches stored, but the version hash differs — only the
    /// version metadata changed.
    VersionBump,
}

// ── DuplicateEvaluator ─────────────────────────────────────────────────────

/// Stateless evaluator that determines the deduplication decision for an
/// ingestion candidate against its previously stored [`ItemHash`].
pub struct DuplicateEvaluator;

impl DuplicateEvaluator {
    /// Evaluate a candidate against its stored item hash.
    ///
    /// Returns `(VersionedItemState, DuplicateDecision)` according to the
    /// rules in MGR-046:
    ///
    /// | `stored`     | content_hash | version_hash | State         | Decision  |
    /// |---|---|---|---|---|
    /// | `None`       | —            | —            | `New`         | `Skip`    |
    /// | matches both | same         | same         | `Unchanged`   | `Skip`    |
    /// | content same | same         | differs      | `VersionBump` | `Reuse`   |
    /// | differs      | differs      | —            | `Changed`     | `Version` |
    pub fn evaluate(
        candidate_hash: &ItemHash,
        stored: Option<&ItemHash>,
    ) -> (VersionedItemState, DuplicateDecision) {
        match stored {
            // No stored record — brand new item.
            None => (VersionedItemState::New, DuplicateDecision::Skip),

            Some(stored_hash) => {
                if stored_hash.content_hash == candidate_hash.content_hash {
                    if stored_hash.version_hash == candidate_hash.version_hash {
                        // Exact duplicate — content and version are identical.
                        (VersionedItemState::Unchanged, DuplicateDecision::Skip)
                    } else {
                        // Same content, different version metadata.
                        (VersionedItemState::VersionBump, DuplicateDecision::Reuse)
                    }
                } else {
                    // Content changed — a new version must be created.
                    (VersionedItemState::Changed, DuplicateDecision::Version)
                }
            }
        }
    }
}

// ── IdempotencyKeyBuilder ──────────────────────────────────────────────────

/// Builder that produces [`SourceEventKey`] values for different event kinds.
pub struct IdempotencyKeyBuilder;

impl IdempotencyKeyBuilder {
    /// Build a [`SourceEventKey`] for a semantic candidate.
    ///
    /// Delegates to [`SourceEventKey::compute`] using the candidate's
    /// `source_id`, `chunk_sequence`, `item_hash.identity_hash`,
    /// `record_kind`, and `item_hash.content_hash`.
    pub fn for_candidate(candidate: &SemanticCandidate) -> SourceEventKey {
        SourceEventKey::compute(
            &candidate.source_id,
            candidate.chunk_sequence,
            &candidate.item_hash.identity_hash,
            &candidate.record_kind,
            &candidate.item_hash.content_hash,
        )
    }

    /// Build a [`SourceEventKey`] for a source lifecycle event.
    ///
    /// - `idempotency_key` = SHA-256(`"{source_id}:{event_kind}:{timestamp_epoch_secs}"`)
    /// - `command_hash`    = SHA-256(`"{source_id}:{event_kind}:{timestamp_epoch_secs}"`)
    ///   (same input as idempotency key; command hash is over the same
    ///   parameters since lifecycle events are their own command)
    ///
    /// `timestamp_epoch_secs` is truncated to whole seconds so sub-second
    /// retries within the same second always yield the same key.
    pub fn for_lifecycle_event(
        source_id: &str,
        event_kind: &str,
        timestamp_epoch_secs: u64,
    ) -> SourceEventKey {
        let payload = format!("{source_id}:{event_kind}:{timestamp_epoch_secs}");
        let key = sha256_hex(&payload);
        SourceEventKey {
            idempotency_key: key.clone(),
            command_hash: key,
        }
    }
}

// ── Unit tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::model::ingestion_chunk::{ItemHash, SemanticCandidate};

    // ── SourceEventKey::compute — determinism ───────────────────────────

    #[test]
    fn source_event_key_compute_is_deterministic() {
        let k1 = SourceEventKey::compute("src-001", 3, "idhash_abc", "memory", "contenthash_xyz");
        let k2 = SourceEventKey::compute("src-001", 3, "idhash_abc", "memory", "contenthash_xyz");
        assert_eq!(k1.idempotency_key, k2.idempotency_key);
        assert_eq!(k1.command_hash, k2.command_hash);
    }

    #[test]
    fn source_event_key_compute_is_64_hex_chars() {
        let k = SourceEventKey::compute("src-001", 0, "id_hash", "memory", "c_hash");
        assert_eq!(
            k.idempotency_key.len(),
            64,
            "idempotency_key must be 64 hex chars (SHA-256)"
        );
        assert_eq!(
            k.command_hash.len(),
            64,
            "command_hash must be 64 hex chars (SHA-256)"
        );
        assert!(k.idempotency_key.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(k.command_hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ── SourceEventKey::compute — different inputs produce different keys

    #[test]
    fn source_event_key_different_source_id_produces_different_key() {
        let k1 = SourceEventKey::compute("src-001", 0, "hash", "memory", "ch");
        let k2 = SourceEventKey::compute("src-002", 0, "hash", "memory", "ch");
        assert_ne!(k1.idempotency_key, k2.idempotency_key);
    }

    #[test]
    fn source_event_key_different_sequence_produces_different_key() {
        let k1 = SourceEventKey::compute("src-001", 0, "hash", "memory", "ch");
        let k2 = SourceEventKey::compute("src-001", 1, "hash", "memory", "ch");
        assert_ne!(k1.idempotency_key, k2.idempotency_key);
    }

    #[test]
    fn source_event_key_different_identity_hash_produces_different_key() {
        let k1 = SourceEventKey::compute("src-001", 0, "hash_a", "memory", "ch");
        let k2 = SourceEventKey::compute("src-001", 0, "hash_b", "memory", "ch");
        assert_ne!(k1.idempotency_key, k2.idempotency_key);
    }

    #[test]
    fn source_event_key_different_record_kind_produces_different_command_hash() {
        let k1 = SourceEventKey::compute("src-001", 0, "hash", "memory", "ch");
        let k2 = SourceEventKey::compute("src-001", 0, "hash", "entity", "ch");
        // record_kind is part of command_hash, not idempotency_key
        assert_ne!(k1.command_hash, k2.command_hash);
    }

    #[test]
    fn source_event_key_different_content_hash_produces_different_command_hash() {
        let k1 = SourceEventKey::compute("src-001", 0, "identity", "memory", "content_a");
        let k2 = SourceEventKey::compute("src-001", 0, "identity", "memory", "content_b");
        assert_ne!(k1.command_hash, k2.command_hash);
    }

    // ── DuplicateEvaluator::evaluate — New when no stored ──────────────

    #[test]
    fn evaluate_new_when_no_stored() {
        let candidate = ItemHash::compute(b"some content", "v1");
        let (state, decision) = DuplicateEvaluator::evaluate(&candidate, None);
        assert_eq!(state, VersionedItemState::New);
        assert_eq!(decision, DuplicateDecision::Skip);
    }

    // ── DuplicateEvaluator::evaluate — Unchanged when same content+version

    #[test]
    fn evaluate_unchanged_when_same_content_and_version() {
        let candidate = ItemHash::compute(b"content", "v1");
        let stored = ItemHash::compute(b"content", "v1");
        let (state, decision) = DuplicateEvaluator::evaluate(&candidate, Some(&stored));
        assert_eq!(state, VersionedItemState::Unchanged);
        assert_eq!(decision, DuplicateDecision::Skip);
    }

    // ── DuplicateEvaluator::evaluate — VersionBump when same content, different version

    #[test]
    fn evaluate_version_bump_when_same_content_different_version() {
        let candidate = ItemHash::compute(b"same content", "v2");
        let stored = ItemHash::compute(b"same content", "v1");
        // Both have the same content_hash but different version_hash.
        assert_eq!(candidate.content_hash, stored.content_hash);
        assert_ne!(candidate.version_hash, stored.version_hash);

        let (state, decision) = DuplicateEvaluator::evaluate(&candidate, Some(&stored));
        assert_eq!(state, VersionedItemState::VersionBump);
        assert_eq!(decision, DuplicateDecision::Reuse);
    }

    // ── DuplicateEvaluator::evaluate — Changed when different content ───

    #[test]
    fn evaluate_changed_when_different_content() {
        let candidate = ItemHash::compute(b"new content", "v1");
        let stored = ItemHash::compute(b"old content", "v1");
        assert_ne!(candidate.content_hash, stored.content_hash);

        let (state, decision) = DuplicateEvaluator::evaluate(&candidate, Some(&stored));
        assert_eq!(state, VersionedItemState::Changed);
        assert_eq!(decision, DuplicateDecision::Version);
    }

    #[test]
    fn evaluate_changed_when_different_content_and_different_version() {
        let candidate = ItemHash::compute(b"content b", "v2");
        let stored = ItemHash::compute(b"content a", "v1");
        let (state, decision) = DuplicateEvaluator::evaluate(&candidate, Some(&stored));
        // Content changed takes precedence over version change.
        assert_eq!(state, VersionedItemState::Changed);
        assert_eq!(decision, DuplicateDecision::Version);
    }

    // ── IdempotencyKeyBuilder::for_candidate — deterministic ───────────

    fn make_candidate(
        source_id: &str,
        sequence: u64,
        content: &[u8],
        version: &str,
    ) -> SemanticCandidate {
        let item_hash = ItemHash::compute(content, version);
        SemanticCandidate {
            source_id: source_id.to_owned(),
            chunk_sequence: sequence,
            item_hash,
            record_kind: "memory".to_owned(),
            content: String::from_utf8_lossy(content).into_owned(),
            locator_json: None,
            policy_namespace: "user".to_owned(),
            policy_scope: "personal".to_owned(),
            policy_sensitivity: 0,
            is_duplicate: false,
        }
    }

    #[test]
    fn for_candidate_is_deterministic() {
        let c = make_candidate("src-001", 5, b"record content", "v1");
        let k1 = IdempotencyKeyBuilder::for_candidate(&c);
        let k2 = IdempotencyKeyBuilder::for_candidate(&c);
        assert_eq!(k1.idempotency_key, k2.idempotency_key);
        assert_eq!(k1.command_hash, k2.command_hash);
    }

    #[test]
    fn for_candidate_different_source_produces_different_key() {
        let c1 = make_candidate("src-001", 5, b"content", "v1");
        let c2 = make_candidate("src-002", 5, b"content", "v1");
        let k1 = IdempotencyKeyBuilder::for_candidate(&c1);
        let k2 = IdempotencyKeyBuilder::for_candidate(&c2);
        assert_ne!(k1.idempotency_key, k2.idempotency_key);
    }

    #[test]
    fn for_candidate_different_sequence_produces_different_key() {
        let c1 = make_candidate("src-001", 0, b"content", "v1");
        let c2 = make_candidate("src-001", 1, b"content", "v1");
        let k1 = IdempotencyKeyBuilder::for_candidate(&c1);
        let k2 = IdempotencyKeyBuilder::for_candidate(&c2);
        assert_ne!(k1.idempotency_key, k2.idempotency_key);
    }

    // ── IdempotencyKeyBuilder::for_lifecycle_event — idempotent within same second

    #[test]
    fn for_lifecycle_event_same_second_produces_same_key() {
        let k1 =
            IdempotencyKeyBuilder::for_lifecycle_event("src-001", "consent_granted", 1_700_000_000);
        let k2 =
            IdempotencyKeyBuilder::for_lifecycle_event("src-001", "consent_granted", 1_700_000_000);
        assert_eq!(k1.idempotency_key, k2.idempotency_key);
        assert_eq!(k1.command_hash, k2.command_hash);
    }

    #[test]
    fn for_lifecycle_event_different_second_produces_different_key() {
        let k1 =
            IdempotencyKeyBuilder::for_lifecycle_event("src-001", "consent_granted", 1_700_000_000);
        let k2 =
            IdempotencyKeyBuilder::for_lifecycle_event("src-001", "consent_granted", 1_700_000_001);
        assert_ne!(k1.idempotency_key, k2.idempotency_key);
    }

    #[test]
    fn for_lifecycle_event_different_event_kind_produces_different_key() {
        let k1 =
            IdempotencyKeyBuilder::for_lifecycle_event("src-001", "consent_granted", 1_700_000_000);
        let k2 =
            IdempotencyKeyBuilder::for_lifecycle_event("src-001", "consent_revoked", 1_700_000_000);
        assert_ne!(k1.idempotency_key, k2.idempotency_key);
    }

    #[test]
    fn for_lifecycle_event_different_source_produces_different_key() {
        let k1 = IdempotencyKeyBuilder::for_lifecycle_event(
            "src-001",
            "ingestion_started",
            1_700_000_000,
        );
        let k2 = IdempotencyKeyBuilder::for_lifecycle_event(
            "src-002",
            "ingestion_started",
            1_700_000_000,
        );
        assert_ne!(k1.idempotency_key, k2.idempotency_key);
    }

    #[test]
    fn for_lifecycle_event_key_is_64_hex_chars() {
        let k = IdempotencyKeyBuilder::for_lifecycle_event("src-001", "started", 0);
        assert_eq!(k.idempotency_key.len(), 64);
        assert!(k.idempotency_key.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn for_lifecycle_event_idempotency_key_equals_command_hash() {
        // Lifecycle events use the same payload for both fields.
        let k = IdempotencyKeyBuilder::for_lifecycle_event("src-001", "consent_granted", 42);
        assert_eq!(k.idempotency_key, k.command_hash);
    }

    // ── Serde round-trips ────────────────────────────────────────────────

    #[test]
    fn duplicate_decision_serde_roundtrip() {
        for decision in [
            DuplicateDecision::Reuse,
            DuplicateDecision::Version,
            DuplicateDecision::Skip,
        ] {
            let json = serde_json::to_string(&decision).unwrap();
            let back: DuplicateDecision = serde_json::from_str(&json).unwrap();
            assert_eq!(back, decision);
        }
    }

    #[test]
    fn versioned_item_state_serde_roundtrip() {
        for state in [
            VersionedItemState::New,
            VersionedItemState::Unchanged,
            VersionedItemState::Changed,
            VersionedItemState::VersionBump,
        ] {
            let json = serde_json::to_string(&state).unwrap();
            let back: VersionedItemState = serde_json::from_str(&json).unwrap();
            assert_eq!(back, state);
        }
    }

    #[test]
    fn duplicate_decision_serde_uses_snake_case() {
        assert_eq!(
            serde_json::to_string(&DuplicateDecision::Reuse).unwrap(),
            "\"reuse\""
        );
        assert_eq!(
            serde_json::to_string(&DuplicateDecision::Version).unwrap(),
            "\"version\""
        );
        assert_eq!(
            serde_json::to_string(&DuplicateDecision::Skip).unwrap(),
            "\"skip\""
        );
    }

    #[test]
    fn versioned_item_state_serde_uses_snake_case() {
        assert_eq!(
            serde_json::to_string(&VersionedItemState::New).unwrap(),
            "\"new\""
        );
        assert_eq!(
            serde_json::to_string(&VersionedItemState::Unchanged).unwrap(),
            "\"unchanged\""
        );
        assert_eq!(
            serde_json::to_string(&VersionedItemState::Changed).unwrap(),
            "\"changed\""
        );
        assert_eq!(
            serde_json::to_string(&VersionedItemState::VersionBump).unwrap(),
            "\"version_bump\""
        );
    }
}
