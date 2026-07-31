//! The core cognitive [`Record`] and its [`RecordKind`] (design §4.2, task
//! F2.1.1).
//!
//! A `Record` is the Rust counterpart of a `records` row. `record_kind` is a
//! **closed** set (the schema enforces `CHECK(record_kind IN
//! (memory/summary/skill/rule))`), so it is a plain enum whose [`FromStr`]
//! rejects unknown values — an unknown kind can never reach the write path.
//! The payload is modeled as [`RecordPayload`] so the schema's "exactly one of
//! {content, content_cipher}" invariant holds by construction in Rust too.
//!
//! This task defines the type + its enums and basic validation; the exhaustive
//! SQL↔Rust↔API round-trip properties are task 2.1.5.
//!
//! ## Derived / lifecycle field logic (task F2.1.3)
//!
//! The `content_hash`, `estimated_tokens`, `truth_state`, `staleness_class`,
//! `valid_interval`, `superseded_by`, `episode_id`, and `goal_context_id`
//! fields are wired to real compute/validation logic here (previously bare
//! placeholders):
//!
//!   * **Canonical content hash** — [`Record::canonical_content_hash`] hashes
//!     the record's *canonical plaintext content* (NFC-normalized, trimmed,
//!     whitespace-collapsed, lowercased BLAKE3 hex via
//!     [`crate::memory::ids::normalized_content_hash`]). Identical content →
//!     identical hash, which feeds dedup, outbox supersede-by-newer, and
//!     rebuild membership (MGR-042 derived convergence). For a
//!     [`RecordPayload::Ciphertext`] record the hash is computed over the
//!     *plaintext captured before encryption* at creation time — so encrypted
//!     records with equal plaintext still converge, and the same helper is
//!     reused so v2 records and the legacy `Memory` row hash identically.
//!   * **Estimated tokens** — [`Record::estimate_tokens`] reuses the shared
//!     `~4 chars/token` heuristic ([`crate::memory::governance::estimate_tokens`]),
//!     so it is deterministic and non-negative (the schema `CHECK
//!     estimated_tokens >= 0` holds by construction of a `u32`).
//!   * **Truth / staleness / lifecycle** — typed via [`TruthState`] and
//!     [`StalenessClass`]; [`TruthState::initial`] gives the coherent initial
//!     disposition for a freshly-stored observation and
//!     [`TruthState::is_default_read_visible`] encodes that superseded/
//!     forgotten/deleted records are excluded from default reads. Staleness
//!     governs *re-verification, never deletion* (design §22.4).
//!   * **Valid interval / supersession / episode / goal-context** — carried as
//!     validated value objects ([`ValidInterval`] is already non-inverted);
//!     [`Record::is_superseded`] / [`Record::is_default_read_visible`] express
//!     the field-level meaning.
//!
//! The *transition* logic (lifecycle FSM commands, supersede command, active
//! predicate) is F1.7 / 2.4 and the governed write path is F1.5 — this task
//! only wires the field-level representation + compute/validation.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::{
    encoding_err, EpisodeId, EventId, GoalId, PolicyPartition, RecordId, SchemaVersion, SourceId,
    UtcTimestamp, ValidInterval,
};
use crate::memory::error::MemoryResult;
use crate::memory::governance::estimate_tokens as estimate_tokens_heuristic;
use crate::memory::ids::normalized_content_hash;
use crate::memory::model::truth::TruthState;
use crate::memory::types::StalenessClass;

/// The kind of cognitive record (design §4.2 `record_kind`
/// `CHECK(memory/summary/skill/rule)`). A **closed** set: the schema `CHECK`
/// rejects anything else, so [`FromStr`] errors on unknown input rather than
/// preserving it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    /// A raw/derived memory (compression level 0–1).
    Memory,
    /// A consolidated summary over episodes.
    Summary,
    /// A reusable procedural skill.
    Skill,
    /// A terminal governing rule.
    Rule,
}

impl RecordKind {
    /// The canonical text form stored in `record_kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            RecordKind::Memory => "memory",
            RecordKind::Summary => "summary",
            RecordKind::Skill => "skill",
            RecordKind::Rule => "rule",
        }
    }

    /// All known variants.
    pub fn all() -> &'static [RecordKind] {
        &[
            RecordKind::Memory,
            RecordKind::Summary,
            RecordKind::Skill,
            RecordKind::Rule,
        ]
    }
}

impl FromStr for RecordKind {
    type Err = crate::memory::error::MemoryError;
    fn from_str(s: &str) -> MemoryResult<Self> {
        Ok(match s {
            "memory" => RecordKind::Memory,
            "summary" => RecordKind::Summary,
            "skill" => RecordKind::Skill,
            "rule" => RecordKind::Rule,
            other => return Err(encoding_err(format!("unknown record_kind {other:?}"))),
        })
    }
}

impl std::fmt::Display for RecordKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RecordKind {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// The record body: exactly one of a plaintext string or an encrypted cipher
/// blob (design §4.2 payload exclusivity `CHECK`). Modeling it as an enum makes
/// the "exactly one" invariant unrepresentable-to-violate in Rust.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordPayload {
    /// Plaintext content (`records.content`).
    Plaintext(String),
    /// Crypto-shreddable ciphertext (`records.content_cipher`).
    Ciphertext(Vec<u8>),
}

impl RecordPayload {
    /// Whether this payload is stored as plaintext.
    pub fn is_plaintext(&self) -> bool {
        matches!(self, RecordPayload::Plaintext(_))
    }

    /// The plaintext content, if this is a plaintext payload.
    pub fn as_plaintext(&self) -> Option<&str> {
        match self {
            RecordPayload::Plaintext(s) => Some(s),
            RecordPayload::Ciphertext(_) => None,
        }
    }
}

/// A cognitive record (`records` row — design §4.2).
///
/// Provenance encoding (2.1.2) and the full hash/token/staleness/lifecycle
/// wiring (2.1.3) refine this type; here it captures every column with a
/// validated value object so nothing is a raw unchecked string.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// Stable identity (`records.id`).
    pub id: RecordId,
    /// The record kind (closed set).
    pub record_kind: RecordKind,
    /// Content/schema version.
    pub schema_version: SchemaVersion,
    /// Exactly-one payload representation.
    pub payload: RecordPayload,
    /// Canonical content hash (BLAKE3 hex); wired in 2.1.3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Truth/lifecycle disposition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truth_state: Option<TruthState>,
    /// Re-verification class (governs re-verification, not deletion).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staleness_class: Option<StalenessClass>,
    /// Half-open valid interval `[valid_from, valid_until)`.
    #[serde(default = "ValidInterval::open")]
    pub valid_interval: ValidInterval,
    /// Namespace/owner/scope/sensitivity policy partition.
    pub policy: PolicyPartition,
    /// Contributing source id (policy provenance).
    pub source_id: SourceId,
    /// Effective policy version tag.
    pub policy_version: String,
    /// The creating authority event.
    pub created_event_id: EventId,
    /// Transaction-time creation instant.
    pub created_at: UtcTimestamp,
    /// The record that supersedes this one, if any (self-reference).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<RecordId>,
    /// Owning episode, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_id: Option<EpisodeId>,
    /// Owning goal context, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_context_id: Option<GoalId>,
    /// Estimated token cost (non-negative); wired in 2.1.3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_tokens: Option<u32>,
    /// Shred-key catalog reference for Hard Delete pending cryptographic erasure
    /// (MGR-041 / design §5.4).  When set, identifies the subject whose
    /// `shred_keys` row controls deletion lifecycle.  Currently a status-flag
    /// reference only: no ciphertext payloads exist and no encryption is
    /// implemented.  Populated regardless of whether real key material exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shred_key_id: Option<String>,
    /// Shred-key version (catalog reference, not secret material).
    /// Reserved for when payload encryption is implemented (MGR-041).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_version: Option<u32>,
}

impl Record {
    /// The canonical **content hash** (BLAKE3 hex) for a piece of plaintext
    /// content.
    ///
    /// The input is canonicalized before hashing (NFC Unicode normalization +
    /// trim + internal-whitespace collapse + lowercase, via
    /// [`normalized_content_hash`]) so semantically-equal content converges to
    /// one hash regardless of Unicode form or incidental whitespace/case. This
    /// is what dedup, outbox supersede-by-newer, and rebuild membership key on
    /// (MGR-042 derived convergence).
    ///
    /// It is a pure function of the content: identical content always yields
    /// the identical hash, and (barring a BLAKE3 collision) different canonical
    /// content yields a different hash. For a [`RecordPayload::Ciphertext`]
    /// record, pass the *plaintext captured before encryption* — the hash must
    /// be derived from plaintext at creation, never from ciphertext (which
    /// varies per key/nonce and would defeat convergence).
    pub fn canonical_content_hash(content: &str) -> String {
        normalized_content_hash(content)
    }

    /// The deterministic, non-negative **estimated token cost** of a piece of
    /// plaintext content, using the shared `~4 chars/token` heuristic
    /// ([`crate::memory::governance::estimate_tokens`]). Reused verbatim so v2
    /// records and the legacy `Memory` row estimate identically; the `u32`
    /// return makes the schema `CHECK (estimated_tokens >= 0)` hold by
    /// construction.
    pub fn estimate_tokens(content: &str) -> u32 {
        estimate_tokens_heuristic(content)
    }

    /// Populate the derived content fields (`content_hash`, `estimated_tokens`)
    /// from the record's *plaintext content* and return the updated record.
    ///
    /// For ciphertext payloads the caller passes the pre-encryption plaintext
    /// (the ciphertext itself must never be hashed — see
    /// [`Record::canonical_content_hash`]). This is the seam that keeps
    /// `content_hash`/`estimated_tokens` from being raw, unchecked
    /// caller-supplied optionals.
    #[must_use]
    pub fn with_derived_content_fields(mut self, plaintext: &str) -> Self {
        self.content_hash = Some(Self::canonical_content_hash(plaintext));
        self.estimated_tokens = Some(Self::estimate_tokens(plaintext));
        self
    }

    /// Derive the content fields directly from a [`RecordPayload::Plaintext`]
    /// payload. Returns `self` unchanged for a ciphertext payload, whose
    /// plaintext is not available at rest and must instead be supplied via
    /// [`Record::with_derived_content_fields`] at creation time.
    #[must_use]
    pub fn derive_content_fields_from_payload(self) -> Self {
        match self.payload.as_plaintext() {
            Some(content) => {
                let content = content.to_owned();
                self.with_derived_content_fields(&content)
            }
            None => self,
        }
    }

    /// The record's truth disposition, defaulting to [`TruthState::initial`]
    /// (`Unverified`) when unset — a freshly-stored observation that has not
    /// yet been verified against a source.
    pub fn truth_state_or_initial(&self) -> TruthState {
        self.truth_state.clone().unwrap_or_else(TruthState::initial)
    }

    /// The record's re-verification class, defaulting to the conservative
    /// [`StalenessClass::VolatileVerifiable`] when unclassified so an
    /// unclassified record is eligible for re-verification (never deletion —
    /// design §22.4).
    pub fn staleness_class_or_default(&self) -> StalenessClass {
        self.staleness_class
            .clone()
            .unwrap_or(StalenessClass::VolatileVerifiable)
    }

    /// Whether this record has been superseded by a successor record. A
    /// superseded record points to its successor via `superseded_by` and is
    /// moved to history (never destroyed); default reads exclude it. The
    /// supersession *command* is task 2.4.4 — this only reports the field.
    pub fn is_superseded(&self) -> bool {
        self.superseded_by.is_some()
    }

    /// Whether this record is visible to *default* reads, from the record's own
    /// fields alone: it must not be superseded and its truth disposition must
    /// be default-read-visible (not superseded/forgotten/deleted).
    ///
    /// This is a field-level convenience only. The authoritative active
    /// predicate — which additionally intersects the [`ValidInterval`] against
    /// the requested transaction snapshot and applies Effective Policy — is
    /// task 2.4.1.
    pub fn is_default_read_visible(&self) -> bool {
        !self.is_superseded() && self.truth_state_or_initial().is_default_read_visible()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Record {
        Record {
            id: RecordId::new_v7(),
            record_kind: RecordKind::Memory,
            schema_version: SchemaVersion::new(1),
            payload: RecordPayload::Plaintext("hello".into()),
            content_hash: None,
            truth_state: Some(TruthState::Current),
            staleness_class: Some(StalenessClass::Permanent),
            valid_interval: ValidInterval::open(),
            policy: PolicyPartition::new("user", "chat", 1).unwrap(),
            source_id: SourceId::new_v7(),
            policy_version: "p1".into(),
            created_event_id: EventId::new_v7(),
            created_at: UtcTimestamp::now(),
            superseded_by: None,
            episode_id: None,
            goal_context_id: None,
            estimated_tokens: Some(3),
            shred_key_id: None,
            key_version: None,
        }
    }

    #[test]
    fn record_kind_roundtrip_and_rejects_unknown() {
        for k in RecordKind::all() {
            assert_eq!(RecordKind::from_str(k.as_str()).unwrap(), *k);
        }
        assert!(RecordKind::from_str("fact").is_err());
        assert!(serde_json::from_str::<RecordKind>("\"fact\"").is_err());
    }

    #[test]
    fn record_serde_roundtrips() {
        let r = sample();
        let json = serde_json::to_string(&r).unwrap();
        let back: Record = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn payload_is_mutually_exclusive_by_construction() {
        let p = RecordPayload::Plaintext("x".into());
        assert!(p.is_plaintext());
        assert_eq!(p.as_plaintext(), Some("x"));
        let c = RecordPayload::Ciphertext(vec![1, 2, 3]);
        assert!(!c.is_plaintext());
        assert_eq!(c.as_plaintext(), None);
    }

    // ── 2.1.3 derived / lifecycle field logic ───────────────────────────

    #[test]
    fn content_hash_is_deterministic_and_content_addressed() {
        let h1 = Record::canonical_content_hash("The quick brown fox");
        let h2 = Record::canonical_content_hash("The quick brown fox");
        // Same content → same hash (feeds dedup / rebuild membership).
        assert_eq!(h1, h2);
        // Canonicalized: case / surrounding + internal whitespace collapse.
        assert_eq!(
            h1,
            Record::canonical_content_hash("  the   QUICK  brown fox  ")
        );
        // Different content → different hash.
        assert_ne!(h1, Record::canonical_content_hash("a different memory"));
        // BLAKE3 hex is 64 chars.
        assert_eq!(h1.len(), 64);
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn token_estimate_is_deterministic_and_non_negative() {
        assert_eq!(Record::estimate_tokens(""), 0);
        let a = Record::estimate_tokens("hello world");
        let b = Record::estimate_tokens("hello world");
        assert_eq!(a, b);
        // ~4 chars/token, rounded up: "hello world" is 11 bytes → ceil(11/4)=3.
        assert_eq!(a, 3);
        // Longer content estimates more tokens.
        assert!(Record::estimate_tokens(&"x".repeat(400)) > a);
    }

    #[test]
    fn derives_content_fields_from_plaintext_payload() {
        let r = Record {
            content_hash: None,
            estimated_tokens: None,
            ..sample()
        }
        .derive_content_fields_from_payload();
        assert_eq!(
            r.content_hash.as_deref(),
            Some(Record::canonical_content_hash("hello").as_str())
        );
        assert_eq!(r.estimated_tokens, Some(Record::estimate_tokens("hello")));
    }

    #[test]
    fn ciphertext_derives_from_supplied_preencryption_plaintext() {
        let plaintext = "secret note";
        let cipher = Record {
            payload: RecordPayload::Ciphertext(vec![9, 9, 9]),
            content_hash: None,
            estimated_tokens: None,
            ..sample()
        };
        // Payload-based derivation is a no-op for ciphertext (no plaintext at rest).
        assert!(cipher
            .clone()
            .derive_content_fields_from_payload()
            .content_hash
            .is_none());
        // Explicitly supplying the pre-encryption plaintext derives the fields,
        // and the hash matches an equal-plaintext plaintext record (convergence).
        let derived = cipher.with_derived_content_fields(plaintext);
        assert_eq!(
            derived.content_hash,
            Some(Record::canonical_content_hash(plaintext))
        );
        assert_eq!(
            derived.estimated_tokens,
            Some(Record::estimate_tokens(plaintext))
        );
    }

    #[test]
    fn truth_and_staleness_defaults_are_coherent() {
        let r = Record {
            truth_state: None,
            staleness_class: None,
            ..sample()
        };
        assert_eq!(r.truth_state_or_initial(), TruthState::Unverified);
        assert_eq!(
            r.staleness_class_or_default(),
            StalenessClass::VolatileVerifiable
        );
        // An explicit disposition is preserved (not overridden by the default).
        let r2 = Record {
            truth_state: Some(TruthState::Confirmed),
            ..sample()
        };
        assert_eq!(r2.truth_state_or_initial(), TruthState::Confirmed);
    }

    #[test]
    fn valid_interval_containment_is_half_open() {
        let from = UtcTimestamp::from_rfc3339_utc("2026-01-01T00:00:00Z").unwrap();
        let until = UtcTimestamp::from_rfc3339_utc("2026-06-01T00:00:00Z").unwrap();
        let mid = UtcTimestamp::from_rfc3339_utc("2026-03-01T00:00:00Z").unwrap();
        let r = Record {
            valid_interval: ValidInterval::new(Some(from), Some(until)).unwrap(),
            ..sample()
        };
        assert!(r.valid_interval.contains(from)); // inclusive lower
        assert!(r.valid_interval.contains(mid));
        assert!(!r.valid_interval.contains(until)); // exclusive upper
    }

    #[test]
    fn superseded_record_representation() {
        let active = sample();
        assert!(!active.is_superseded());
        assert!(active.is_default_read_visible());

        let successor = RecordId::new_v7();
        let superseded = Record {
            superseded_by: Some(successor.clone()),
            ..sample()
        };
        assert!(superseded.is_superseded());
        assert_eq!(superseded.superseded_by, Some(successor));
        // A superseded record is excluded from default reads.
        assert!(!superseded.is_default_read_visible());

        // A Deleted/Forgotten truth disposition also hides the record.
        let deleted = Record {
            truth_state: Some(TruthState::Deleted),
            ..sample()
        };
        assert!(!deleted.is_default_read_visible());
    }
}
