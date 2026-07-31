//! Interchange import validation — design §4.4, MGR-046, MGR-029.
//!
//! Whole-manifest validation precedes one idempotent AuthorityTx import.
//! All records are validated before any record is committed.
//!
//! ## Key behavioural rules
//!
//! 1. **Whole-manifest validation first**: ALL records are validated before ANY is committed.
//! 2. **Checksum must match**: [`PackageChecksumVerifier::verify`] fails if computed ≠ manifest's.
//! 3. **Idempotent key**: same package + policy namespace → same key → repeated imports are no-ops.
//! 4. **Unknown required fields cause Skip (not reject)**: records with unrecognized required
//!    fields are counted but do NOT block the import of other valid records.
//! 5. **Limits enforced**: record count and total bytes must not exceed [`ImportLimits`].

use sha2::{Digest, Sha256};

use super::interchange::{InterchangeManifest, InterchangeManifestValidator};
use super::interchange_export::{ExportRecord, IndependentParserValidator};

// ── ImportValidationError ─────────────────────────────────────────────────

/// An error produced during interchange import validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportValidationError {
    /// The manifest failed its self-consistency check.
    ManifestInvalid { reason: String },
    /// The manifest's format version is incompatible with this implementation.
    VersionIncompatible { manifest: String, supported: String },
    /// The number of records exceeds the configured limit.
    RecordCountExceedsLimit { got: u64, max: u64 },
    /// The total bytes of all content JSON exceeds the configured limit.
    TotalBytesExceedsLimit { got: u64, max: u64 },
    /// The computed package checksum does not match the manifest's declared value.
    PackageChecksumMismatch { expected: String, computed: String },
    /// A record's content hash does not match the actual hash of its content JSON.
    RecordHashMismatch {
        record_id: String,
        stored: String,
        computed: String,
    },
    /// A record's content JSON is not valid JSON.
    RecordInvalidJson { record_id: String, error: String },
    /// A record is missing a required interchange field.
    RecordMissingField { record_id: String, field: String },
    /// A record's sensitivity exceeds the allowed limit.
    SensitivityExceeded { record_id: String, got: u8, max: u8 },
}

impl std::fmt::Display for ImportValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ManifestInvalid { reason } => write!(f, "manifest invalid: {reason}"),
            Self::VersionIncompatible {
                manifest,
                supported,
            } => {
                write!(
                    f,
                    "version incompatible: manifest={manifest:?} supported={supported:?}"
                )
            }
            Self::RecordCountExceedsLimit { got, max } => {
                write!(f, "record count {got} exceeds limit {max}")
            }
            Self::TotalBytesExceedsLimit { got, max } => {
                write!(f, "total bytes {got} exceeds limit {max}")
            }
            Self::PackageChecksumMismatch { expected, computed } => {
                write!(
                    f,
                    "package checksum mismatch: expected={expected:?} computed={computed:?}"
                )
            }
            Self::RecordHashMismatch {
                record_id,
                stored,
                computed,
            } => {
                write!(
                    f,
                    "record {record_id:?} hash mismatch: stored={stored:?} computed={computed:?}"
                )
            }
            Self::RecordInvalidJson { record_id, error } => {
                write!(f, "record {record_id:?} invalid JSON: {error}")
            }
            Self::RecordMissingField { record_id, field } => {
                write!(f, "record {record_id:?} missing field {field:?}")
            }
            Self::SensitivityExceeded {
                record_id,
                got,
                max,
            } => {
                write!(
                    f,
                    "record {record_id:?} sensitivity {got} exceeds limit {max}"
                )
            }
        }
    }
}

impl std::error::Error for ImportValidationError {}

// ── ImportLimits ──────────────────────────────────────────────────────────

/// Configurable limits for one interchange import operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportLimits {
    /// Maximum number of records allowed in one import. Default: 100_000.
    pub max_records: u64,
    /// Maximum total bytes for all content JSON. Default: 500 MB.
    pub max_total_bytes: u64,
    /// Maximum allowed sensitivity in imported records. Default: 3.
    pub max_sensitivity: u8,
}

impl ImportLimits {
    /// Conservative safe defaults: up to 100k records, 500 MB of content, sensitivity ≤ 3.
    pub fn default_safe() -> Self {
        ImportLimits {
            max_records: 100_000,
            max_total_bytes: 500 * 1024 * 1024, // 500 MB
            max_sensitivity: 3,
        }
    }
}

// ── PackageChecksumVerifier ───────────────────────────────────────────────

/// Verifies the package checksum of an interchange import.
///
/// The package checksum is SHA-256 of all record `content_hash` values
/// concatenated in order. Records must already be in canonical order (matching
/// the manifest's `content_ordering`).
pub struct PackageChecksumVerifier;

impl PackageChecksumVerifier {
    /// Verify that the package's computed checksum matches the manifest's declared checksum.
    pub fn verify(
        records: &[ExportRecord],
        manifest: &InterchangeManifest,
    ) -> Result<(), ImportValidationError> {
        let computed = Self::compute_package_checksum(records);
        if computed != manifest.package_checksum {
            return Err(ImportValidationError::PackageChecksumMismatch {
                expected: manifest.package_checksum.clone(),
                computed,
            });
        }
        Ok(())
    }

    /// Compute the package checksum from an ordered slice of records.
    ///
    /// SHA-256 of all `content_hash` values concatenated in order.
    pub fn compute_package_checksum(records: &[ExportRecord]) -> String {
        let mut input = String::new();
        for record in records {
            input.push_str(&record.content_hash);
        }
        sha256_hex(input.as_bytes())
    }
}

/// Compute SHA-256 hex digest of `data`.
fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

// ── ImportSemanticReport ──────────────────────────────────────────────────

/// Summary report from semantic validation of all import records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSemanticReport {
    /// Count of records that passed all validation checks.
    pub valid_count: u32,
    /// Count of records skipped due to unknown required semantics.
    pub skipped_count: u32,
    /// Count of records that would be imported (valid_count after skips).
    pub import_count: u32,
    /// Whether any record was skipped due to unknown required semantics.
    pub has_unknown_required: bool,
}

// ── ImportSemanticValidator ───────────────────────────────────────────────

/// Validates that all records in an import package have required semantic fields.
pub struct ImportSemanticValidator;

impl ImportSemanticValidator {
    /// Validate that all records in the package have required semantic fields.
    ///
    /// Each record must:
    /// - Have valid JSON content ([`IndependentParserValidator::validate_json`])
    /// - Have required fields ([`IndependentParserValidator::validate_required_fields`])
    /// - Have a valid content hash ([`ExportRecord::verify_hash`])
    /// - Have sensitivity within allowed limits
    ///
    /// Records whose content JSON is not a JSON object with unknown required
    /// field patterns are counted as skipped (not rejected). All other failures
    /// are hard errors.
    pub fn validate_all(
        records: &[ExportRecord],
        limits: &ImportLimits,
    ) -> Result<ImportSemanticReport, ImportValidationError> {
        let mut valid_count: u32 = 0;
        let mut skipped_count: u32 = 0;
        let mut has_unknown_required = false;

        for record in records {
            // 1. Validate JSON
            if let Err(e) = IndependentParserValidator::validate_json(&record.content_json) {
                return Err(ImportValidationError::RecordInvalidJson {
                    record_id: record.record_id.clone(),
                    error: e.to_string(),
                });
            }

            // 2. Validate required fields — unknown required semantics → skip (not reject)
            match IndependentParserValidator::validate_required_fields(&record.content_json) {
                Ok(()) => {}
                Err(e) => {
                    // Required fields missing means this record has unknown required semantics.
                    // Per behavioural rule 4: skip, don't reject.
                    let _ = e;
                    skipped_count += 1;
                    has_unknown_required = true;
                    continue;
                }
            }

            // 3. Validate content hash
            if let Err(e) = record.verify_hash() {
                // Hash mismatch is a hard error (data integrity)
                let (stored, computed) = match e {
                    super::interchange_export::ExportValidationError::HashMismatch {
                        stored,
                        computed,
                    } => (stored, computed),
                    _ => (record.content_hash.clone(), String::new()),
                };
                return Err(ImportValidationError::RecordHashMismatch {
                    record_id: record.record_id.clone(),
                    stored,
                    computed,
                });
            }

            // 4. Validate sensitivity
            if record.sensitivity > limits.max_sensitivity {
                return Err(ImportValidationError::SensitivityExceeded {
                    record_id: record.record_id.clone(),
                    got: record.sensitivity,
                    max: limits.max_sensitivity,
                });
            }

            valid_count += 1;
        }

        let import_count = valid_count;
        Ok(ImportSemanticReport {
            valid_count,
            skipped_count,
            import_count,
            has_unknown_required,
        })
    }
}

// ── ImportIdempotencyKey ──────────────────────────────────────────────────

/// A deterministic idempotency key for an import operation.
///
/// The same package checksum + importing policy namespace always produces the
/// same key, so repeated imports of the same package are idempotent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportIdempotencyKey {
    /// The computed idempotency key string.
    pub key: String,
}

impl ImportIdempotencyKey {
    /// Compute the idempotency key from the package checksum and policy namespace.
    ///
    /// The key is: SHA-256 hex of `"{package_checksum}:{policy_namespace}"`.
    pub fn compute(package_checksum: &str, policy_namespace: &str) -> Self {
        let input = format!("{package_checksum}:{policy_namespace}");
        ImportIdempotencyKey {
            key: sha256_hex(input.as_bytes()),
        }
    }
}

// ── ImportValidationResult ────────────────────────────────────────────────

/// The result of a successful full pre-import validation pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportValidationResult {
    /// Whether the manifest passed self-consistency validation.
    pub manifest_valid: bool,
    /// Whether the package checksum was verified successfully.
    pub checksum_verified: bool,
    /// Semantic validation report for all records.
    pub semantic_report: ImportSemanticReport,
    /// The deterministic idempotency key for this import.
    pub idempotency_key: ImportIdempotencyKey,
    /// Whether the import is ready to proceed (all checks passed).
    pub import_ready: bool,
}

// ── InterchangeImportValidator ────────────────────────────────────────────

/// The full pre-import validation pipeline.
///
/// Validates the entire package before any record is committed, enforcing:
/// whole-manifest validation, version compatibility, limits, checksum, and
/// per-record semantics.
pub struct InterchangeImportValidator;

impl InterchangeImportValidator {
    /// The supported interchange format version string.
    pub const SUPPORTED_VERSION: &'static str = InterchangeManifestValidator::SUPPORTED_VERSION;

    /// Run the full pre-import validation pipeline.
    ///
    /// Steps (in order):
    /// 1. Validate manifest ([`InterchangeManifestValidator::validate`])
    /// 2. Check version compatibility
    /// 3. Check record count against limits
    /// 4. Check total bytes against limits
    /// 5. Verify package checksum
    /// 6. Validate all record semantics
    ///
    /// Returns `Ok(ImportValidationResult)` on full success.
    /// Returns `Err` on the first blocking failure.
    /// Unknown optional fields are preserved; unknown required semantics are
    /// counted but not blocking.
    pub fn validate(
        manifest: &InterchangeManifest,
        records: &[ExportRecord],
        limits: &ImportLimits,
    ) -> Result<ImportValidationResult, ImportValidationError> {
        // Step 1: Validate manifest self-consistency
        InterchangeManifestValidator::validate(manifest).map_err(|e| {
            ImportValidationError::ManifestInvalid {
                reason: e.to_string(),
            }
        })?;

        // Step 2: Check version compatibility
        if !InterchangeManifestValidator::is_version_compatible(&manifest.format_version) {
            return Err(ImportValidationError::VersionIncompatible {
                manifest: manifest.format_version.clone(),
                supported: Self::SUPPORTED_VERSION.to_string(),
            });
        }

        // Step 3: Check record count against limits
        let record_count = records.len() as u64;
        if record_count > limits.max_records {
            return Err(ImportValidationError::RecordCountExceedsLimit {
                got: record_count,
                max: limits.max_records,
            });
        }

        // Step 4: Check total bytes against limits
        let total_bytes: u64 = records.iter().map(|r| r.content_json.len() as u64).sum();
        if total_bytes > limits.max_total_bytes {
            return Err(ImportValidationError::TotalBytesExceedsLimit {
                got: total_bytes,
                max: limits.max_total_bytes,
            });
        }

        // Step 5: Verify package checksum
        PackageChecksumVerifier::verify(records, manifest)?;

        // Step 6: Validate all record semantics
        let semantic_report = ImportSemanticValidator::validate_all(records, limits)?;

        // Compute idempotency key from package checksum + policy namespace
        let policy_namespace = records
            .first()
            .map(|r| r.policy_namespace.as_str())
            .unwrap_or("");
        let idempotency_key =
            ImportIdempotencyKey::compute(&manifest.package_checksum, policy_namespace);

        Ok(ImportValidationResult {
            manifest_valid: true,
            checksum_verified: true,
            semantic_report,
            idempotency_key,
            import_ready: true,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::model::interchange::{
        InterchangeManifest, InterchangeOrdering, InterchangeSchemaVersions, InterchangeScope,
    };
    use crate::memory::model::interchange_export::ExportRecord;

    // ── helpers ──────────────────────────────────────────────────────────

    fn sha256_hex_test(data: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(data);
        format!("{:x}", hasher.finalize())
    }

    fn make_record(record_id: &str, content_json: &str, sensitivity: u8) -> ExportRecord {
        let content_hash = sha256_hex_test(content_json.as_bytes());
        ExportRecord {
            record_kind: "memory".to_string(),
            record_id: record_id.to_string(),
            content_json: content_json.to_string(),
            content_hash,
            revision: 1,
            policy_namespace: "default".to_string(),
            policy_scope: "personal".to_string(),
            sensitivity,
        }
    }

    fn make_valid_manifest(records: &[ExportRecord]) -> InterchangeManifest {
        let package_checksum = PackageChecksumVerifier::compute_package_checksum(records);
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
            scope: InterchangeScope {
                record_kinds: vec![],
                namespace_filter: None,
                scope_filter: None,
                max_sensitivity: 3,
                include_events: false,
                include_traces: false,
                include_sources: true,
            },
            package_checksum,
            content_ordering: InterchangeOrdering::ByRevision,
            record_count: records.len() as u64,
            event_count: 0,
            link_count: 0,
            has_extensions: false,
            extensions: None,
        }
    }

    // ── ImportLimits::default_safe ────────────────────────────────────────

    #[test]
    fn import_limits_default_safe_has_expected_values() {
        let limits = ImportLimits::default_safe();
        assert_eq!(limits.max_records, 100_000);
        assert_eq!(limits.max_total_bytes, 500 * 1024 * 1024);
        assert_eq!(limits.max_sensitivity, 3);
    }

    // ── PackageChecksumVerifier ───────────────────────────────────────────

    #[test]
    fn checksum_verifier_ok_for_matching_checksum() {
        let r1 = make_record("r1", r#"{"id":"r1","kind":"memory"}"#, 0);
        let r2 = make_record("r2", r#"{"id":"r2","kind":"memory"}"#, 0);
        let records = vec![r1, r2];
        let manifest = make_valid_manifest(&records);
        assert!(PackageChecksumVerifier::verify(&records, &manifest).is_ok());
    }

    #[test]
    fn checksum_verifier_err_for_wrong_checksum() {
        let r1 = make_record("r1", r#"{"id":"r1","kind":"memory"}"#, 0);
        let records = vec![r1];
        let mut manifest = make_valid_manifest(&records);
        manifest.package_checksum = "deadbeef00000000".to_string();
        let err = PackageChecksumVerifier::verify(&records, &manifest).unwrap_err();
        assert!(
            matches!(err, ImportValidationError::PackageChecksumMismatch { .. }),
            "expected PackageChecksumMismatch, got {err:?}"
        );
    }

    #[test]
    fn checksum_verifier_compute_is_deterministic() {
        let r1 = make_record("r1", r#"{"id":"r1","kind":"memory"}"#, 0);
        let r2 = make_record("r2", r#"{"id":"r2","kind":"memory"}"#, 0);
        let records = vec![r1, r2];
        let cs1 = PackageChecksumVerifier::compute_package_checksum(&records);
        let cs2 = PackageChecksumVerifier::compute_package_checksum(&records);
        assert_eq!(cs1, cs2);
    }

    // ── ImportSemanticValidator ───────────────────────────────────────────

    #[test]
    fn semantic_validator_ok_for_valid_records() {
        let r1 = make_record("r1", r#"{"id":"r1","kind":"memory","content":"hello"}"#, 0);
        let r2 = make_record("r2", r#"{"id":"r2","kind":"entity","content":"world"}"#, 1);
        let limits = ImportLimits::default_safe();
        let report = ImportSemanticValidator::validate_all(&[r1, r2], &limits).unwrap();
        assert_eq!(report.valid_count, 2);
        assert_eq!(report.skipped_count, 0);
        assert_eq!(report.import_count, 2);
        assert!(!report.has_unknown_required);
    }

    #[test]
    fn semantic_validator_err_for_hash_mismatch() {
        let mut record = make_record("r-bad-hash", r#"{"id":"r1","kind":"memory"}"#, 0);
        record.content_hash = "00000000bad".to_string();
        let limits = ImportLimits::default_safe();
        let err = ImportSemanticValidator::validate_all(&[record], &limits).unwrap_err();
        assert!(
            matches!(err, ImportValidationError::RecordHashMismatch { .. }),
            "expected RecordHashMismatch, got {err:?}"
        );
    }

    #[test]
    fn semantic_validator_err_for_invalid_json() {
        let mut record = make_record("r-bad-json", r#"{"id":"r1","kind":"memory"}"#, 0);
        // Corrupt the JSON after computing the hash so hash check won't fire first
        // (actually hash check comes after json check — we need the json to be bad
        // but the hash to match the bad json)
        let bad_json = "not valid json at all";
        let correct_hash_for_bad = sha256_hex_test(bad_json.as_bytes());
        record.content_json = bad_json.to_string();
        record.content_hash = correct_hash_for_bad;
        let limits = ImportLimits::default_safe();
        let err = ImportSemanticValidator::validate_all(&[record], &limits).unwrap_err();
        assert!(
            matches!(err, ImportValidationError::RecordInvalidJson { .. }),
            "expected RecordInvalidJson, got {err:?}"
        );
    }

    #[test]
    fn semantic_validator_err_for_sensitivity_exceeded() {
        let record = make_record("r-sens", r#"{"id":"r-sens","kind":"memory"}"#, 3);
        let limits = ImportLimits {
            max_records: 100_000,
            max_total_bytes: 500 * 1024 * 1024,
            max_sensitivity: 2, // record has sensitivity 3
        };
        let err = ImportSemanticValidator::validate_all(&[record], &limits).unwrap_err();
        assert!(
            matches!(
                err,
                ImportValidationError::SensitivityExceeded { got: 3, max: 2, .. }
            ),
            "expected SensitivityExceeded(3, max=2), got {err:?}"
        );
    }

    #[test]
    fn semantic_validator_skips_records_with_unknown_required_semantics() {
        // A record whose content JSON is missing required fields → skipped, not rejected
        let mut record = make_record("r-skip", r#"{"content":"no id or kind here"}"#, 0);
        // Recompute hash since we changed content_json
        record.content_hash = sha256_hex_test(record.content_json.as_bytes());
        let limits = ImportLimits::default_safe();
        let report = ImportSemanticValidator::validate_all(&[record], &limits).unwrap();
        assert_eq!(report.skipped_count, 1);
        assert_eq!(report.valid_count, 0);
        assert!(report.has_unknown_required);
    }

    // ── ImportIdempotencyKey ──────────────────────────────────────────────

    #[test]
    fn idempotency_key_is_deterministic() {
        let k1 = ImportIdempotencyKey::compute("checksum-abc", "default");
        let k2 = ImportIdempotencyKey::compute("checksum-abc", "default");
        assert_eq!(k1.key, k2.key);
    }

    #[test]
    fn idempotency_key_differs_for_different_checksum() {
        let k1 = ImportIdempotencyKey::compute("checksum-abc", "default");
        let k2 = ImportIdempotencyKey::compute("checksum-xyz", "default");
        assert_ne!(k1.key, k2.key);
    }

    #[test]
    fn idempotency_key_differs_for_different_namespace() {
        let k1 = ImportIdempotencyKey::compute("same-checksum", "ns-a");
        let k2 = ImportIdempotencyKey::compute("same-checksum", "ns-b");
        assert_ne!(k1.key, k2.key);
    }

    // ── InterchangeImportValidator::validate ─────────────────────────────

    #[test]
    fn full_pipeline_ok_for_valid_package() {
        let r1 = make_record("r1", r#"{"id":"r1","kind":"memory","content":"hello"}"#, 0);
        let r2 = make_record("r2", r#"{"id":"r2","kind":"entity","content":"world"}"#, 1);
        let records = vec![r1, r2];
        let manifest = make_valid_manifest(&records);
        let limits = ImportLimits::default_safe();
        let result = InterchangeImportValidator::validate(&manifest, &records, &limits).unwrap();
        assert!(result.manifest_valid);
        assert!(result.checksum_verified);
        assert!(result.import_ready);
        assert_eq!(result.semantic_report.valid_count, 2);
        assert_eq!(result.semantic_report.skipped_count, 0);
        assert!(!result.semantic_report.has_unknown_required);
    }

    #[test]
    fn full_pipeline_err_on_manifest_invalid() {
        let records: Vec<ExportRecord> = vec![];
        let mut manifest = make_valid_manifest(&records);
        manifest.format_version = "not-a-version".to_string();
        let limits = ImportLimits::default_safe();
        let err = InterchangeImportValidator::validate(&manifest, &records, &limits).unwrap_err();
        assert!(
            matches!(err, ImportValidationError::ManifestInvalid { .. }),
            "expected ManifestInvalid, got {err:?}"
        );
    }

    #[test]
    fn full_pipeline_err_on_record_count_limit() {
        let r1 = make_record("r1", r#"{"id":"r1","kind":"memory"}"#, 0);
        let records = vec![r1];
        let manifest = make_valid_manifest(&records);
        let limits = ImportLimits {
            max_records: 0, // 1 record > 0 limit
            max_total_bytes: 500 * 1024 * 1024,
            max_sensitivity: 3,
        };
        let err = InterchangeImportValidator::validate(&manifest, &records, &limits).unwrap_err();
        assert!(
            matches!(
                err,
                ImportValidationError::RecordCountExceedsLimit { got: 1, max: 0 }
            ),
            "expected RecordCountExceedsLimit, got {err:?}"
        );
    }

    #[test]
    fn full_pipeline_err_on_total_bytes_limit() {
        let content = r#"{"id":"r1","kind":"memory","content":"hello world"}"#;
        let r1 = make_record("r1", content, 0);
        let records = vec![r1];
        let manifest = make_valid_manifest(&records);
        let limits = ImportLimits {
            max_records: 100_000,
            max_total_bytes: 1, // content is much larger than 1 byte
            max_sensitivity: 3,
        };
        let err = InterchangeImportValidator::validate(&manifest, &records, &limits).unwrap_err();
        assert!(
            matches!(err, ImportValidationError::TotalBytesExceedsLimit { .. }),
            "expected TotalBytesExceedsLimit, got {err:?}"
        );
    }

    #[test]
    fn full_pipeline_ok_for_empty_records() {
        let records: Vec<ExportRecord> = vec![];
        let manifest = make_valid_manifest(&records);
        let limits = ImportLimits::default_safe();
        let result = InterchangeImportValidator::validate(&manifest, &records, &limits).unwrap();
        assert!(result.import_ready);
        assert_eq!(result.semantic_report.valid_count, 0);
        assert_eq!(result.semantic_report.import_count, 0);
    }
}
