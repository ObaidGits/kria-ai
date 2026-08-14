//! Interchange streaming export — design §4.4, MGR-046, MGR-029.
//!
//! Policy-selected streaming export with deterministic order and independent
//! parser validation. This module is the domain model for the export side of
//! the interchange package: it tracks state, filters records, sorts them, and
//! validates their content — all without performing I/O.
//!
//! ## Key behavioural rules
//!
//! 1. **Deterministic order** — [`ExportOrderComparator`] sorts records by the
//!    [`InterchangeOrdering`] stored in the manifest before export.
//! 2. **Policy-first** — [`PolicyExportFilter::passes_filter`] applies
//!    sensitivity, namespace, scope, and kind filters before anything else.
//! 3. **Secret exclusion** — records matching exclusion rules are filtered out
//!    (MGR-046: "export excludes unauthorized secrets").
//! 4. **Package checksum** — [`ExportStream::finalize`] produces SHA-256 of all
//!    `content_hash` values concatenated in order (MGR-029: "deterministic order
//!    for reproducibility").
//! 5. **Independent parser** — [`IndependentParserValidator`] validates records
//!    using only JSON parsing, without KRIA's internal Rust types.

use sha2::{Digest, Sha256};

use super::interchange::{
    InterchangeManifest, InterchangeOrdering, InterchangeScope, SecretExclusionRules,
};

// ── ExportValidationError ─────────────────────────────────────────────────

/// An error produced during export record validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportValidationError {
    /// The content is not valid JSON.
    InvalidJson { error: String },
    /// A required field is missing from the JSON.
    MissingField { field: String },
    /// The content hash does not match the actual hash.
    HashMismatch { stored: String, computed: String },
    /// The record sensitivity exceeds the allowed max.
    SensitivityExceeded { got: u8, max: u8 },
}

impl std::fmt::Display for ExportValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson { error } => write!(f, "invalid JSON: {error}"),
            Self::MissingField { field } => write!(f, "required field {field:?} is missing"),
            Self::HashMismatch { stored, computed } => write!(
                f,
                "content hash mismatch: stored={stored:?} computed={computed:?}"
            ),
            Self::SensitivityExceeded { got, max } => {
                write!(f, "sensitivity {got} exceeds max {max}")
            }
        }
    }
}

impl std::error::Error for ExportValidationError {}

// ── ExportRecord ──────────────────────────────────────────────────────────

/// A single record serialized for the interchange export stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRecord {
    /// The record kind (e.g. `"memory"`, `"entity"`, `"relationship"`).
    pub record_kind: String,
    /// The stable record ID.
    pub record_id: String,
    /// The serialized content (canonical JSON).
    pub content_json: String,
    /// The SHA-256 hex hash of `content_json`.
    pub content_hash: String,
    /// The graph revision at which this record was exported.
    pub revision: u64,
    /// The policy namespace.
    pub policy_namespace: String,
    /// The policy scope.
    pub policy_scope: String,
    /// The sensitivity level (`0..=3`).
    pub sensitivity: u8,
}

impl ExportRecord {
    /// Compute and validate the content hash.
    ///
    /// Returns `Ok(())` when the stored [`ExportRecord::content_hash`] matches
    /// the SHA-256 hex digest of [`ExportRecord::content_json`]. Returns
    /// [`ExportValidationError::HashMismatch`] otherwise.
    pub fn verify_hash(&self) -> Result<(), ExportValidationError> {
        let computed = sha256_hex(self.content_json.as_bytes());
        if computed != self.content_hash {
            return Err(ExportValidationError::HashMismatch {
                stored: self.content_hash.clone(),
                computed,
            });
        }
        Ok(())
    }
}

/// Compute the SHA-256 hex digest of `data`.
fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

// ── ExportStream ──────────────────────────────────────────────────────────

/// The streaming export session for an interchange package.
///
/// Does NOT do actual I/O — this is the domain model tracking state.
/// Call [`ExportStream::record_emitted`] for each emitted record, then
/// [`ExportStream::finalize`] to obtain the final package checksum.
pub struct ExportStream {
    /// The manifest being built.
    pub manifest: InterchangeManifest,
    /// Records emitted so far (in order).
    pub emitted_count: u64,
    /// Running SHA-256 input: all `content_hash` values concatenated in order.
    pub running_checksum_input: String,
    /// Whether the export is complete.
    pub is_complete: bool,
}

impl ExportStream {
    /// Create a new export stream for `manifest`.
    pub fn new(manifest: InterchangeManifest) -> Self {
        ExportStream {
            manifest,
            emitted_count: 0,
            running_checksum_input: String::new(),
            is_complete: false,
        }
    }

    /// Record that one [`ExportRecord`] was emitted.
    ///
    /// Increments [`ExportStream::emitted_count`] and appends the record's
    /// `content_hash` to [`ExportStream::running_checksum_input`] so the
    /// final checksum reflects emission order.
    pub fn record_emitted(&mut self, record: &ExportRecord) {
        self.emitted_count += 1;
        self.running_checksum_input.push_str(&record.content_hash);
    }

    /// Finalize the export and produce the package checksum.
    ///
    /// The checksum is the SHA-256 hex digest of all `content_hash` values
    /// concatenated in emission order (deterministic given deterministic
    /// ordering — MGR-029). Marks the stream as complete and returns the
    /// checksum string.
    pub fn finalize(&mut self) -> String {
        self.is_complete = true;
        sha256_hex(self.running_checksum_input.as_bytes())
    }
}

// ── ExportOrderComparator ─────────────────────────────────────────────────

/// Computes the sort key for an export record based on the manifest's
/// `content_ordering`.
pub struct ExportOrderComparator;

impl ExportOrderComparator {
    /// Compute a deterministic sort key for `record` given `ordering`.
    ///
    /// | Ordering        | Sort key format                                    |
    /// |-----------------|----------------------------------------------------|
    /// | `ByRevision`    | `"{revision:020}:{record_id}"`                     |
    /// | `ByCreatedAt`   | `"{record_id}"` (created_at not in `ExportRecord`) |
    /// | `ByKindThenId`  | `"{record_kind}:{record_id}"`                      |
    ///
    /// `ByCreatedAt` falls back to `record_id` because `ExportRecord` does not
    /// carry a `created_at` field; the caller should supply a created_at-aware
    /// sort if that ordering is required (the manifest documents the chosen
    /// ordering so importers can detect mismatches).
    pub fn sort_key(record: &ExportRecord, ordering: &InterchangeOrdering) -> String {
        match ordering {
            InterchangeOrdering::ByRevision => {
                format!("{:020}:{}", record.revision, record.record_id)
            }
            InterchangeOrdering::ByCreatedAt => {
                // ExportRecord has no created_at; fall back to record_id for
                // a stable, deterministic key.
                record.record_id.clone()
            }
            InterchangeOrdering::ByKindThenId => {
                format!("{}:{}", record.record_kind, record.record_id)
            }
        }
    }

    /// Compare two records for ordering.
    pub fn compare(
        a: &ExportRecord,
        b: &ExportRecord,
        ordering: &InterchangeOrdering,
    ) -> std::cmp::Ordering {
        Self::sort_key(a, ordering).cmp(&Self::sort_key(b, ordering))
    }
}

// ── PolicyExportFilter ────────────────────────────────────────────────────

/// Filters export records based on scope and secret exclusion rules.
pub struct PolicyExportFilter;

impl PolicyExportFilter {
    /// Check whether `record` passes the export policy filter.
    ///
    /// Returns `true` when **all** of the following hold:
    ///
    /// 1. `record.sensitivity <= scope.max_sensitivity`
    /// 2. `record.policy_namespace` matches `scope.namespace_filter` (when set)
    /// 3. `record.policy_scope` matches `scope.scope_filter` (when set)
    /// 4. `record.record_kind` is in `scope.record_kinds` (when non-empty)
    /// 5. `record` does NOT match any secret exclusion rule
    pub fn passes_filter(
        record: &ExportRecord,
        scope: &InterchangeScope,
        exclusion_rules: &SecretExclusionRules,
    ) -> bool {
        // 1. Sensitivity cap
        if record.sensitivity > scope.max_sensitivity {
            return false;
        }

        // 2. Namespace filter
        if let Some(ns_filter) = &scope.namespace_filter {
            if &record.policy_namespace != ns_filter {
                return false;
            }
        }

        // 3. Scope filter
        if let Some(scope_filter) = &scope.scope_filter {
            if &record.policy_scope != scope_filter {
                return false;
            }
        }

        // 4. Record kind filter (empty = all kinds pass)
        if !scope.record_kinds.is_empty() && !scope.record_kinds.contains(&record.record_kind) {
            return false;
        }

        // 5. Secret exclusion rules
        if Self::is_explicitly_excluded(&record.record_id, exclusion_rules) {
            return false;
        }
        if exclusion_rules.exclude_max_sensitivity && record.sensitivity >= 3 {
            return false;
        }

        true
    }

    /// Check whether `record_id` is in the exclusion list.
    pub fn is_explicitly_excluded(record_id: &str, exclusion_rules: &SecretExclusionRules) -> bool {
        exclusion_rules
            .excluded_record_ids
            .iter()
            .any(|id| id == record_id)
    }
}

// ── IndependentParserValidator ────────────────────────────────────────────

/// Validates that an export record's content can be independently parsed.
///
/// An "independent parser" means: a parser that has no knowledge of KRIA's
/// internal types can still parse the content as valid JSON and find the
/// required fields. This satisfies the design §4.4 requirement for
/// "independent parser validation".
pub struct IndependentParserValidator;

impl IndependentParserValidator {
    /// Validate that `content_json` is well-formed JSON.
    pub fn validate_json(content_json: &str) -> Result<(), ExportValidationError> {
        serde_json::from_str::<serde_json::Value>(content_json).map_err(|e| {
            ExportValidationError::InvalidJson {
                error: e.to_string(),
            }
        })?;
        Ok(())
    }

    /// Validate that `content_json` contains the required interchange fields.
    ///
    /// Required: one of `"id"` or `"record_id"`, and one of `"kind"` or
    /// `"record_kind"`. These are the minimum fields an independent (KRIA-agnostic)
    /// parser needs to identify and route a record.
    pub fn validate_required_fields(content_json: &str) -> Result<(), ExportValidationError> {
        let value: serde_json::Value =
            serde_json::from_str(content_json).map_err(|e| ExportValidationError::InvalidJson {
                error: e.to_string(),
            })?;

        let obj = match value.as_object() {
            Some(o) => o,
            None => {
                return Err(ExportValidationError::MissingField {
                    field: "id or record_id".to_string(),
                })
            }
        };

        // Must have "id" or "record_id"
        let has_id = obj.contains_key("id") || obj.contains_key("record_id");
        if !has_id {
            return Err(ExportValidationError::MissingField {
                field: "id or record_id".to_string(),
            });
        }

        // Must have "kind" or "record_kind"
        let has_kind = obj.contains_key("kind") || obj.contains_key("record_kind");
        if !has_kind {
            return Err(ExportValidationError::MissingField {
                field: "kind or record_kind".to_string(),
            });
        }

        Ok(())
    }

    /// Validate both JSON well-formedness and required fields.
    pub fn validate_record(record: &ExportRecord) -> Result<(), ExportValidationError> {
        Self::validate_json(&record.content_json)?;
        Self::validate_required_fields(&record.content_json)?;
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::interchange::{
        InterchangeManifest, InterchangeOrdering, InterchangeSchemaVersions, InterchangeScope,
        SecretExclusionRules,
    };

    // ── helpers ──────────────────────────────────────────────────────────

    fn make_record(
        record_kind: &str,
        record_id: &str,
        content_json: &str,
        revision: u64,
        sensitivity: u8,
    ) -> ExportRecord {
        let content_hash = sha256_hex(content_json.as_bytes());
        ExportRecord {
            record_kind: record_kind.to_string(),
            record_id: record_id.to_string(),
            content_json: content_json.to_string(),
            content_hash,
            revision,
            policy_namespace: "default".to_string(),
            policy_scope: "personal".to_string(),
            sensitivity,
        }
    }

    fn default_scope() -> InterchangeScope {
        InterchangeScope {
            record_kinds: vec![],
            namespace_filter: None,
            scope_filter: None,
            max_sensitivity: 2,
            include_events: false,
            include_traces: false,
            include_sources: true,
        }
    }

    fn make_manifest(ordering: InterchangeOrdering) -> InterchangeManifest {
        InterchangeManifest {
            format_version: "1.0".to_string(),
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
            schema_versions: InterchangeSchemaVersions {
                format_version: "1.0".to_string(),
                schema_version: 1,
                ontology_version: "ontology-v1".to_string(),
                embedding_model_id: None,
                embedding_model_version: None,
                algorithm_versions: vec![],
            },
            scope: default_scope(),
            package_checksum: "placeholder".to_string(),
            content_ordering: ordering,
            record_count: 0,
            event_count: 0,
            link_count: 0,
            has_extensions: false,
            extensions: None,
        }
    }

    // ── ExportRecord::verify_hash ─────────────────────────────────────────

    #[test]
    fn verify_hash_ok_for_correct_hash() {
        let content = r#"{"id":"abc","kind":"memory"}"#;
        let record = make_record("memory", "rec-1", content, 1, 0);
        assert!(record.verify_hash().is_ok());
    }

    #[test]
    fn verify_hash_err_for_wrong_hash() {
        let content = r#"{"id":"abc","kind":"memory"}"#;
        let mut record = make_record("memory", "rec-1", content, 1, 0);
        record.content_hash = "deadbeef".to_string(); // corrupt it
        let err = record.verify_hash().unwrap_err();
        assert!(
            matches!(err, ExportValidationError::HashMismatch { .. }),
            "expected HashMismatch, got {err:?}"
        );
    }

    // ── PolicyExportFilter::passes_filter ─────────────────────────────────

    #[test]
    fn passes_filter_ok_when_all_conditions_met() {
        let record = make_record("memory", "rec-1", r#"{"id":"r1","kind":"memory"}"#, 1, 1);
        let scope = default_scope(); // max_sensitivity=2
        let rules = SecretExclusionRules {
            exclude_max_sensitivity: true,
            exclude_detected_secrets: false,
            excluded_record_ids: vec![],
            exclude_shred_keys: false,
        };
        assert!(PolicyExportFilter::passes_filter(&record, &scope, &rules));
    }

    #[test]
    fn passes_filter_fails_for_sensitivity_exceeds_max() {
        let mut record = make_record("memory", "rec-2", r#"{"id":"r2","kind":"memory"}"#, 1, 3);
        record.sensitivity = 3; // max is 2
        let scope = default_scope(); // max_sensitivity=2
        let rules = SecretExclusionRules::default_safe();
        assert!(!PolicyExportFilter::passes_filter(&record, &scope, &rules));
    }

    #[test]
    fn passes_filter_fails_for_wrong_namespace() {
        let mut record = make_record("memory", "rec-3", r#"{"id":"r3","kind":"memory"}"#, 1, 1);
        record.policy_namespace = "other-ns".to_string();
        let mut scope = default_scope();
        scope.namespace_filter = Some("default".to_string());
        let rules = SecretExclusionRules::default_safe();
        assert!(!PolicyExportFilter::passes_filter(&record, &scope, &rules));
    }

    #[test]
    fn passes_filter_fails_for_excluded_record_id() {
        let record = make_record(
            "memory",
            "secret-id",
            r#"{"id":"s1","kind":"memory"}"#,
            1,
            1,
        );
        let scope = default_scope();
        let rules = SecretExclusionRules {
            exclude_max_sensitivity: true,
            exclude_detected_secrets: false,
            excluded_record_ids: vec!["secret-id".to_string()],
            exclude_shred_keys: false,
        };
        assert!(!PolicyExportFilter::passes_filter(&record, &scope, &rules));
    }

    #[test]
    fn passes_filter_fails_for_wrong_scope_filter() {
        let mut record = make_record("memory", "rec-4", r#"{"id":"r4","kind":"memory"}"#, 1, 1);
        record.policy_scope = "work".to_string();
        let mut scope = default_scope();
        scope.scope_filter = Some("personal".to_string());
        let rules = SecretExclusionRules::default_safe();
        assert!(!PolicyExportFilter::passes_filter(&record, &scope, &rules));
    }

    #[test]
    fn passes_filter_fails_for_record_kind_not_in_allowed_kinds() {
        let record = make_record("entity", "rec-5", r#"{"id":"r5","kind":"entity"}"#, 1, 1);
        let mut scope = default_scope();
        scope.record_kinds = vec!["memory".to_string()];
        let rules = SecretExclusionRules::default_safe();
        assert!(!PolicyExportFilter::passes_filter(&record, &scope, &rules));
    }

    // ── ExportOrderComparator::sort_key ───────────────────────────────────

    #[test]
    fn sort_key_by_revision_includes_padded_revision_and_id() {
        let record = make_record("memory", "zzz", r#"{"id":"z","kind":"memory"}"#, 42, 0);
        let key = ExportOrderComparator::sort_key(&record, &InterchangeOrdering::ByRevision);
        assert_eq!(key, "00000000000000000042:zzz");
    }

    #[test]
    fn sort_key_by_kind_then_id_includes_kind_and_id() {
        let record = make_record("entity", "abc", r#"{"id":"abc","kind":"entity"}"#, 1, 0);
        let key = ExportOrderComparator::sort_key(&record, &InterchangeOrdering::ByKindThenId);
        assert_eq!(key, "entity:abc");
    }

    #[test]
    fn sort_key_by_created_at_falls_back_to_record_id() {
        let record = make_record("memory", "xyz", r#"{"id":"xyz","kind":"memory"}"#, 5, 0);
        let key = ExportOrderComparator::sort_key(&record, &InterchangeOrdering::ByCreatedAt);
        assert_eq!(key, "xyz");
    }

    #[test]
    fn compare_by_revision_orders_lower_revision_first() {
        let a = make_record("memory", "a", r#"{"id":"a","kind":"memory"}"#, 1, 0);
        let b = make_record("memory", "b", r#"{"id":"b","kind":"memory"}"#, 2, 0);
        assert_eq!(
            ExportOrderComparator::compare(&a, &b, &InterchangeOrdering::ByRevision),
            std::cmp::Ordering::Less
        );
    }

    // ── ExportStream::record_emitted and finalize ─────────────────────────

    #[test]
    fn export_stream_finalize_is_deterministic() {
        let manifest = make_manifest(InterchangeOrdering::ByRevision);
        let r1 = make_record("memory", "r1", r#"{"id":"r1","kind":"memory"}"#, 1, 0);
        let r2 = make_record("entity", "r2", r#"{"id":"r2","kind":"entity"}"#, 2, 0);

        let checksum_a = {
            let mut stream = ExportStream::new(manifest.clone());
            stream.record_emitted(&r1);
            stream.record_emitted(&r2);
            stream.finalize()
        };

        let checksum_b = {
            let mut stream = ExportStream::new(manifest.clone());
            stream.record_emitted(&r1);
            stream.record_emitted(&r2);
            stream.finalize()
        };

        assert_eq!(checksum_a, checksum_b, "checksum must be deterministic");
    }

    #[test]
    fn export_stream_checksum_differs_with_different_order() {
        let manifest = make_manifest(InterchangeOrdering::ByRevision);
        let r1 = make_record("memory", "r1", r#"{"id":"r1","kind":"memory"}"#, 1, 0);
        let r2 = make_record("entity", "r2", r#"{"id":"r2","kind":"entity"}"#, 2, 0);

        let checksum_ab = {
            let mut stream = ExportStream::new(manifest.clone());
            stream.record_emitted(&r1);
            stream.record_emitted(&r2);
            stream.finalize()
        };

        let checksum_ba = {
            let mut stream = ExportStream::new(manifest.clone());
            stream.record_emitted(&r2);
            stream.record_emitted(&r1);
            stream.finalize()
        };

        assert_ne!(
            checksum_ab, checksum_ba,
            "different emission order must produce different checksum"
        );
    }

    #[test]
    fn export_stream_emitted_count_increments() {
        let manifest = make_manifest(InterchangeOrdering::ByRevision);
        let mut stream = ExportStream::new(manifest);
        assert_eq!(stream.emitted_count, 0);
        let r = make_record("memory", "r1", r#"{"id":"r1","kind":"memory"}"#, 1, 0);
        stream.record_emitted(&r);
        assert_eq!(stream.emitted_count, 1);
        stream.record_emitted(&r);
        assert_eq!(stream.emitted_count, 2);
    }

    #[test]
    fn export_stream_is_complete_after_finalize() {
        let manifest = make_manifest(InterchangeOrdering::ByRevision);
        let mut stream = ExportStream::new(manifest);
        assert!(!stream.is_complete);
        stream.finalize();
        assert!(stream.is_complete);
    }

    // ── IndependentParserValidator::validate_json ─────────────────────────

    #[test]
    fn validate_json_ok_for_valid_json() {
        assert!(IndependentParserValidator::validate_json(r#"{"id":"x","kind":"memory"}"#).is_ok());
    }

    #[test]
    fn validate_json_err_for_invalid_json() {
        let err = IndependentParserValidator::validate_json("not-json").unwrap_err();
        assert!(
            matches!(err, ExportValidationError::InvalidJson { .. }),
            "expected InvalidJson, got {err:?}"
        );
    }

    // ── IndependentParserValidator::validate_required_fields ─────────────

    #[test]
    fn validate_required_fields_ok_when_id_and_kind_present() {
        assert!(IndependentParserValidator::validate_required_fields(
            r#"{"id":"abc","kind":"memory"}"#
        )
        .is_ok());
    }

    #[test]
    fn validate_required_fields_ok_with_record_id_and_record_kind() {
        assert!(IndependentParserValidator::validate_required_fields(
            r#"{"record_id":"abc","record_kind":"entity"}"#
        )
        .is_ok());
    }

    #[test]
    fn validate_required_fields_err_when_id_missing() {
        let err =
            IndependentParserValidator::validate_required_fields(r#"{"kind":"memory","value":1}"#)
                .unwrap_err();
        assert!(
            matches!(err, ExportValidationError::MissingField { .. }),
            "expected MissingField, got {err:?}"
        );
    }

    #[test]
    fn validate_required_fields_err_when_kind_missing() {
        let err = IndependentParserValidator::validate_required_fields(r#"{"id":"abc","value":1}"#)
            .unwrap_err();
        assert!(
            matches!(err, ExportValidationError::MissingField { .. }),
            "expected MissingField, got {err:?}"
        );
    }

    #[test]
    fn validate_record_ok_for_valid_record() {
        let record = make_record(
            "memory",
            "rec-ok",
            r#"{"id":"rec-ok","kind":"memory","content":"hello"}"#,
            1,
            0,
        );
        assert!(IndependentParserValidator::validate_record(&record).is_ok());
    }

    #[test]
    fn validate_record_err_for_invalid_json_content() {
        let mut record = make_record(
            "memory",
            "rec-bad",
            r#"{"id":"rec-bad","kind":"memory"}"#,
            1,
            0,
        );
        record.content_json = "this is not json".to_string();
        let err = IndependentParserValidator::validate_record(&record).unwrap_err();
        assert!(matches!(err, ExportValidationError::InvalidJson { .. }));
    }
}
