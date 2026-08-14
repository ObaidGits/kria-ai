//! Forward-compatibility policy for the v2 cognitive model (design §8/§13/§40,
//! R25; MGR-034 AC5; task F2.1.4).
//!
//! Design §40/R25 states the coherent rule the whole model surface obeys:
//!
//!   * **READ / interchange** — an older binary reading a value written by a
//!     newer one MUST preserve it verbatim for read diagnostics/interchange
//!     rather than failing. This covers two distinct kinds of "newer" data:
//!       1. an unknown *value* in a **forward-compatible free-text** column
//!          (`truth_state`, `staleness_class`, `entity_type`, `alias_type`,
//!          roles, …) — already preserved by [`TruthState::Other`] and the
//!          [`string_enum!`](crate::types) `Other(String)` fallback; and
//!       2. an unknown *optional field* that a newer schema added to a record —
//!          which a naive derived `Deserialize` silently **drops**. This module
//!          adds the missing mechanism ([`ForwardCompatible`]) that captures
//!          those unrecognized fields into an [`UnknownFields`] map so an
//!          export→import round-trip through an older reader loses no data.
//!
//!   * **WRITE / command** — a command that depends on an unknown **required**
//!     semantic value MUST be rejected (you cannot act on a meaning you do not
//!     understand). This is *already* enforced, and this module only documents
//!     the coherent boundary rather than re-implementing it:
//!       * a **closed-set** required field (`record_kind`, goal `status`,
//!         evidence `polarity`, consolidation `level`) rejects an unrecognized
//!         value at construction via its `FromStr`/`Deserialize`, which errors
//!         with [`StorageError::Encoding`](crate::error::StorageError)
//!         (see [`RecordKind`](crate::model::RecordKind) et al.); and
//!       * an unknown/unsupported required `schema_version` on a command is
//!         denied by the write-boundary schema check with
//!         `UnsupportedSchema` (design §8; authority `validation`).
//!
//! ## The read↔write asymmetry, made explicit
//!
//! | Position                                   | Read / interchange            | Write / command                     |
//! |--------------------------------------------|-------------------------------|-------------------------------------|
//! | Unknown value in **free-text** column      | preserved (`Other`)           | preserved (free text, no closed set)|
//! | Unknown **optional** field                 | preserved ([`ForwardCompatible`]) | n/a (not a required semantic)   |
//! | Unknown value in **closed/required** field | denied at typed projection    | **denied** (`Encoding`)             |
//! | Unknown/unsupported required schema version | preserved only in raw diag.  | **denied** (`UnsupportedSchema`)    |
//!
//! ## Two read representations
//!
//! There are deliberately two ways to read newer-written data:
//!
//!   * **Typed projection** — [`ForwardCompatible<T>`] over a typed DTO (e.g.
//!     `ForwardCompatible<Record>`). It preserves unknown *optional* fields but
//!     still refuses an unknown value in a *closed/required* position, because
//!     the typed `T` cannot be constructed from a meaning it does not
//!     understand. This is the interchange projection used for records that a
//!     reader *does* understand structurally.
//!   * **Raw diagnostic** — parsing the row into an untyped
//!     [`serde_json::Value`] / [`UnknownFields`] preserves *everything*,
//!     including an unknown required enum/version, for read diagnostics only. It
//!     is never a write source. Task 2.7 builds the interchange package on this
//!     split (a raw-preserving envelope plus a typed projection per record kind).
//!
//! The authority **command envelope** stays strict on purpose: it is the write
//! boundary, so it must never silently absorb unknown fields via
//! [`ForwardCompatible`]. That wrapper is for read/interchange DTOs only.

use serde::{Deserialize, Serialize};

/// Unrecognized JSON fields captured verbatim from a newer writer, keyed by
/// field name. Ordinary `serde_json::Map` preserves values losslessly (numbers,
/// strings, nested objects/arrays), so a round-trip re-emits them unchanged.
pub type UnknownFields = serde_json::Map<String, serde_json::Value>;

/// A forward-compatible read/interchange wrapper around a typed DTO `T` that
/// preserves any **unknown optional fields** a newer writer added, so an
/// export→import round-trip through an older reader loses no data (design
/// §40/R25; MGR-034 AC5).
///
/// `T`'s recognized fields deserialize into the typed value as usual; every
/// other top-level field is captured into [`UnknownFields`] and re-emitted
/// verbatim on serialize. Because `T` is still fully typed, an unknown value in
/// a **closed/required** position (e.g. an unrecognized `record_kind`) is still
/// rejected during deserialization — this wrapper only tolerates unknown
/// *optional fields*, never unknown *required semantics*.
///
/// `T` must serialize as a JSON object/map (a struct); wrapping a primitive or
/// sequence is a programmer error and will fail to (de)serialize.
///
/// This is the mechanism task 2.7 (interchange format) builds its
/// extension-preservation on; it is intentionally standalone here (no store or
/// package wiring yet).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForwardCompatible<T> {
    /// The recognized, typed payload.
    #[serde(flatten)]
    value: T,
    /// Unrecognized ("unknown optional") fields, preserved verbatim.
    #[serde(flatten)]
    unknown: UnknownFields,
}

impl<T> ForwardCompatible<T> {
    /// Wrap a typed value with no unknown fields (the common "we wrote this"
    /// case).
    pub fn new(value: T) -> Self {
        Self {
            value,
            unknown: UnknownFields::new(),
        }
    }

    /// Wrap a typed value together with preserved unknown fields.
    pub fn with_unknown(value: T, unknown: UnknownFields) -> Self {
        Self { value, unknown }
    }

    /// The recognized typed payload.
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Mutable access to the recognized typed payload.
    pub fn value_mut(&mut self) -> &mut T {
        &mut self.value
    }

    /// Consume the wrapper, returning just the typed payload (dropping preserved
    /// unknown fields — callers that must not lose them use [`into_parts`]).
    ///
    /// [`into_parts`]: ForwardCompatible::into_parts
    pub fn into_value(self) -> T {
        self.value
    }

    /// Consume the wrapper, returning the typed payload and its preserved
    /// unknown fields.
    pub fn into_parts(self) -> (T, UnknownFields) {
        (self.value, self.unknown)
    }

    /// The preserved unknown fields.
    pub fn unknown_fields(&self) -> &UnknownFields {
        &self.unknown
    }

    /// Whether any unknown (newer-writer) fields were preserved.
    pub fn has_unknown_fields(&self) -> bool {
        !self.unknown.is_empty()
    }

    /// The names of the preserved unknown fields (useful for diagnostics).
    pub fn unknown_field_names(&self) -> impl Iterator<Item = &str> {
        self.unknown.keys().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::record::{Record, RecordKind, RecordPayload};
    use crate::model::truth::TruthState;
    use crate::model::{
        EventId, PolicyPartition, RecordId, SchemaVersion, SourceId, UtcTimestamp, ValidInterval,
    };
    use crate::types::StalenessClass;

    fn sample_record() -> Record {
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

    // ── READ direction: preserve unknown OPTIONAL fields ─────────────────

    #[test]
    fn preserves_unknown_optional_fields_on_roundtrip() {
        // A record written by a *newer* build carries three fields this build
        // does not know: a string, an integer, and a nested object.
        let record = sample_record();
        let mut obj = match serde_json::to_value(&record).unwrap() {
            serde_json::Value::Object(m) => m,
            other => panic!("record did not serialize to an object: {other:?}"),
        };
        obj.insert("future_note".into(), serde_json::json!("from v-next"));
        obj.insert("future_priority".into(), serde_json::json!(42));
        obj.insert(
            "future_meta".into(),
            serde_json::json!({"nested": [1, 2, 3], "flag": true}),
        );
        let newer_json = serde_json::to_string(&serde_json::Value::Object(obj)).unwrap();

        // An older reader projects it through the forward-compatible wrapper.
        let fc: ForwardCompatible<Record> = serde_json::from_str(&newer_json).unwrap();

        // The recognized payload is intact…
        assert_eq!(fc.value(), &record);
        // …and the unknown optional fields are preserved verbatim.
        assert!(fc.has_unknown_fields());
        let mut names: Vec<&str> = fc.unknown_field_names().collect();
        names.sort_unstable();
        assert_eq!(names, ["future_meta", "future_note", "future_priority"]);
        assert_eq!(
            fc.unknown_fields().get("future_priority"),
            Some(&serde_json::json!(42)),
            "integers preserved as integers (no float coercion)"
        );
        assert_eq!(
            fc.unknown_fields().get("future_meta"),
            Some(&serde_json::json!({"nested": [1, 2, 3], "flag": true})),
            "nested structure preserved verbatim"
        );

        // Re-export loses nothing: the unknown fields survive re-serialization.
        let reexported: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&fc).unwrap()).unwrap();
        let original: serde_json::Value = serde_json::from_str(&newer_json).unwrap();
        assert_eq!(reexported, original, "export→import→export is lossless");
    }

    #[test]
    fn no_unknown_fields_when_everything_is_recognized() {
        let record = sample_record();
        let json = serde_json::to_string(&ForwardCompatible::new(record.clone())).unwrap();
        let fc: ForwardCompatible<Record> = serde_json::from_str(&json).unwrap();
        assert!(!fc.has_unknown_fields());
        assert_eq!(fc.into_value(), record);
    }

    #[test]
    fn preserves_unknown_free_text_enum_value_on_read() {
        // A newer writer used a `truth_state` value this build does not know.
        // The free-text column is forward-compatible: it round-trips as
        // `TruthState::Other` rather than failing (design §40).
        let mut obj = match serde_json::to_value(sample_record()).unwrap() {
            serde_json::Value::Object(m) => m,
            _ => unreachable!(),
        };
        obj.insert(
            "truth_state".into(),
            serde_json::json!("provisionally_true"),
        );
        let json = serde_json::to_string(&serde_json::Value::Object(obj)).unwrap();

        let fc: ForwardCompatible<Record> = serde_json::from_str(&json).unwrap();
        assert_eq!(
            fc.value().truth_state,
            Some(TruthState::Other("provisionally_true".into()))
        );
        // A free-text unknown value is not an "unknown field" — it deserialized
        // into the recognized `truth_state` field.
        assert!(!fc.has_unknown_fields());
    }

    // ── WRITE direction: deny unknown REQUIRED semantics ─────────────────

    #[test]
    fn rejects_unknown_value_in_closed_required_field() {
        // `record_kind` is a closed set (schema CHECK). An unknown value in that
        // required position is denied even through the read/interchange
        // projection — a typed `Record` cannot be built from a kind whose
        // meaning is unknown (you cannot act on it).
        let mut obj = match serde_json::to_value(sample_record()).unwrap() {
            serde_json::Value::Object(m) => m,
            _ => unreachable!(),
        };
        obj.insert("record_kind".into(), serde_json::json!("telepathy"));
        let json = serde_json::to_string(&serde_json::Value::Object(obj)).unwrap();

        assert!(
            serde_json::from_str::<ForwardCompatible<Record>>(&json).is_err(),
            "unknown value in a closed/required field must be rejected, not preserved"
        );

        // The raw diagnostic representation, by contrast, preserves everything
        // (including the unknown required value) for read diagnostics only — it
        // is never a write source (design §13). Task 2.7 uses this split.
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            raw.get("record_kind"),
            Some(&serde_json::json!("telepathy"))
        );
    }

    #[test]
    fn rejects_missing_required_field_rather_than_defaulting() {
        // A required field that is absent must fail, not be silently defaulted.
        let mut obj = match serde_json::to_value(sample_record()).unwrap() {
            serde_json::Value::Object(m) => m,
            _ => unreachable!(),
        };
        obj.remove("record_kind");
        let json = serde_json::to_string(&serde_json::Value::Object(obj)).unwrap();
        assert!(serde_json::from_str::<ForwardCompatible<Record>>(&json).is_err());
    }
}
