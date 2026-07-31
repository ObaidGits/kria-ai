//! Interchange v1 canonical manifest — design §4.4, MGR-032, design §A4.
//!
//! An interchange package is a self-describing export artifact. The manifest
//! carries every version, scope, ordering, and checksum field needed to validate
//! and import the package without any external context (MGR-032: "self-describing
//! with schema/ontology versions").
//!
//! ## Key behavioural rules
//!
//! 1. **Self-describing** — the manifest carries all versions; no external
//!    context is needed.
//! 2. **Secret exclusion** — [`SecretExclusionRules::default_safe`] excludes
//!    max-sensitivity records, detected secrets, and shred keys by default.
//! 3. **Extension preservation** — unknown `extensions` fields round-trip
//!    unchanged; v1 parsers must not reject manifests that carry extension data.
//! 4. **Version compatibility** — the major version must match; a difference in
//!    the minor version is backward-compatible.
//! 5. **Canonical ordering** — the ordering enum determines the deterministic
//!    sort of content files in the package.

use serde::{Deserialize, Serialize};

// ── InterchangeVersion ────────────────────────────────────────────────────

/// The version of the Interchange format.
///
/// `major` is incremented on breaking changes; `minor` is incremented for
/// backward-compatible additions. Two versions are compatible when they share
/// the same major number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterchangeVersion {
    /// Major version (breaking changes increment this).
    pub major: u32,
    /// Minor version (backward-compatible additions increment this).
    pub minor: u32,
    /// The canonical string representation (e.g. `"1.0"`).
    pub as_string: String,
}

impl InterchangeVersion {
    /// The current Interchange version — v1 (major=1, minor=0).
    pub fn v1() -> Self {
        InterchangeVersion {
            major: 1,
            minor: 0,
            as_string: "1.0".to_string(),
        }
    }

    /// Whether `self` is compatible with `other`.
    ///
    /// Compatibility requires the same major version; the minor version
    /// difference is backward-compatible in either direction.
    pub fn is_compatible_with(&self, other: &Self) -> bool {
        self.major == other.major
    }
}

// ── AlgorithmVersionRef ───────────────────────────────────────────────────

/// A reference to a named algorithm and the specific version used during an
/// export or analytics run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlgorithmVersionRef {
    /// The algorithm identifier (e.g. `"rrf-general-v1"`).
    pub algorithm_name: String,
    /// The version string of that algorithm.
    pub version: String,
}

// ── InterchangeSchemaVersions ─────────────────────────────────────────────

/// All schema/ontology/model versions captured at export time.
///
/// Carries everything needed to interpret the package content without
/// consulting the importing system's own version metadata (MGR-032).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterchangeSchemaVersions {
    /// The Interchange format version string (e.g. `"1.0"`).
    pub format_version: String,
    /// The authority schema migration number (must be > 0).
    pub schema_version: u32,
    /// The relation ontology version (e.g. `"ontology-v1"`).
    pub ontology_version: String,
    /// The embedding model identity used at export time, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model_id: Option<String>,
    /// The embedding model version used at export time, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model_version: Option<String>,
    /// The algorithm/profile versions used in analytics included in this export.
    #[serde(default)]
    pub algorithm_versions: Vec<AlgorithmVersionRef>,
}

// ── InterchangeScope ──────────────────────────────────────────────────────

/// What is included in an interchange export.
///
/// Filters are applied at export time; an importing system uses this to
/// understand what subset of the originating authority is present and to
/// reproduce the same filter when re-importing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterchangeScope {
    /// Record kinds included (empty = all kinds).
    #[serde(default)]
    pub record_kinds: Vec<String>,
    /// Policy namespace filter (`None` = all namespaces).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace_filter: Option<String>,
    /// Policy scope filter (`None` = all scopes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_filter: Option<String>,
    /// Maximum sensitivity level to include (`0..=3`).
    pub max_sensitivity: u8,
    /// Whether to include event history in the export.
    pub include_events: bool,
    /// Whether to include retrieval traces in the export.
    pub include_traces: bool,
    /// Whether to include source metadata in the export.
    pub include_sources: bool,
}

// ── SecretExclusionRules ──────────────────────────────────────────────────

/// Rules for excluding secrets from an interchange export.
///
/// Secrets that match any rule are excluded from the export. The design
/// invariant (§A4) is that no hidden ID, name, or topology is exposed; these
/// rules reinforce that by stripping secrets before serialisation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretExclusionRules {
    /// Whether to exclude all high-sensitivity (`sensitivity >= 3`) records.
    pub exclude_max_sensitivity: bool,
    /// Whether to exclude records that match API key / private key patterns.
    pub exclude_detected_secrets: bool,
    /// Specific record IDs to exclude unconditionally.
    #[serde(default)]
    pub excluded_record_ids: Vec<String>,
    /// Whether to exclude encryption key references and shred-key rows.
    pub exclude_shred_keys: bool,
}

impl SecretExclusionRules {
    /// The conservative default rules (suitable for any export):
    ///
    /// - `exclude_max_sensitivity = true`  — drops sensitivity-3 records
    /// - `exclude_detected_secrets = true` — drops API/private-key pattern matches
    /// - `exclude_shred_keys = true`       — drops shred-key catalog references
    /// - `excluded_record_ids = []`        — no extra ad-hoc exclusions
    pub fn default_safe() -> Self {
        SecretExclusionRules {
            exclude_max_sensitivity: true,
            exclude_detected_secrets: true,
            excluded_record_ids: Vec::new(),
            exclude_shred_keys: true,
        }
    }
}

// ── InterchangeOrdering ───────────────────────────────────────────────────

/// The canonical ordering of records in an interchange export.
///
/// The chosen variant is stored in the manifest so that an importer can
/// reproduce the same sort or detect mismatches without inspecting the payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterchangeOrdering {
    /// Records ordered by authority revision (chronological).
    ByRevision,
    /// Records ordered by creation timestamp.
    ByCreatedAt,
    /// Records ordered by record kind then ID (deterministic for diffs).
    ByKindThenId,
}

// ── InterchangeManifest ───────────────────────────────────────────────────

/// The canonical Interchange v1 manifest.
///
/// Self-describing: carries all version, scope, ordering, and checksum
/// information needed to validate and import the package without external
/// context (MGR-032 / design §4.4).
///
/// ### Extension preservation
///
/// Unknown `extensions` fields round-trip unchanged. v1 parsers **must not**
/// reject manifests that carry `extensions` — this allows future minor-version
/// additions to be transparently forwarded through v1 tooling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterchangeManifest {
    /// The Interchange format version string (e.g. `"1.0"`).
    pub format_version: String,
    /// RFC 3339 UTC timestamp when this manifest was created.
    pub created_at: String,
    /// Schema/model versions at export time.
    pub schema_versions: InterchangeSchemaVersions,
    /// What was included in this export.
    pub scope: InterchangeScope,
    /// SHA-256 hex digest of the entire package content in canonical order.
    pub package_checksum: String,
    /// The canonical ordering used for content files in this package.
    pub content_ordering: InterchangeOrdering,
    /// Count of records included.
    pub record_count: u64,
    /// Count of events included.
    pub event_count: u64,
    /// Count of relationships/links included.
    pub link_count: u64,
    /// Whether any extension fields are present in this manifest.
    pub has_extensions: bool,
    /// Extension fields unknown to v1 parsers; preserved unchanged for
    /// round-trip compatibility. v1 parsers must not fail on this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<serde_json::Value>,
}

// ── InterchangeValidationError ────────────────────────────────────────────

/// An error produced by [`InterchangeManifestValidator::validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterchangeValidationError {
    /// The manifest's format version string is not recognised by this
    /// implementation.
    UnknownFormatVersion { version: String },
    /// The manifest was produced by an incompatible (different-major) format
    /// version.
    IncompatibleFormatVersion { manifest: String, supported: String },
    /// The embedded `schema_version` is zero or otherwise invalid.
    InvalidSchemaVersion { got: u32 },
    /// The `package_checksum` field is empty.
    EmptyPackageChecksum,
    /// The `max_sensitivity` value in [`InterchangeScope`] is outside `0..=3`.
    InvalidSensitivity { got: u8 },
    /// A required field is absent or empty.
    MissingRequiredField { field: String },
}

impl std::fmt::Display for InterchangeValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownFormatVersion { version } => {
                write!(f, "unknown interchange format version: {version:?}")
            }
            Self::IncompatibleFormatVersion {
                manifest,
                supported,
            } => write!(
                f,
                "incompatible interchange format: manifest is {manifest:?}, \
                 this implementation supports {supported:?}"
            ),
            Self::InvalidSchemaVersion { got } => {
                write!(f, "schema_version must be > 0, got {got}")
            }
            Self::EmptyPackageChecksum => write!(f, "package_checksum must not be empty"),
            Self::InvalidSensitivity { got } => {
                write!(f, "scope.max_sensitivity {got} is out of range 0..=3")
            }
            Self::MissingRequiredField { field } => {
                write!(f, "required field {field:?} is missing or empty")
            }
        }
    }
}

impl std::error::Error for InterchangeValidationError {}

// ── InterchangeManifestValidator ──────────────────────────────────────────

/// Validates an [`InterchangeManifest`] for self-consistency.
pub struct InterchangeManifestValidator;

impl InterchangeManifestValidator {
    /// The format version this implementation supports (`"1.0"`).
    pub const SUPPORTED_VERSION: &'static str = "1.0";

    /// The major version this implementation supports.
    pub const SUPPORTED_MAJOR: u32 = 1;

    /// Validate a manifest is self-consistent.
    ///
    /// Checks (in order):
    /// 1. `format_version` is a known, compatible version string.
    /// 2. `schema_versions.schema_version` is > 0.
    /// 3. `package_checksum` is non-empty.
    /// 4. `scope.max_sensitivity` is in `0..=3`.
    /// 5. `created_at` is non-empty (presence only; full RFC 3339 validation
    ///    is deferred to the import boundary).
    ///
    /// Returns `Ok(())` or the first [`InterchangeValidationError`] found.
    pub fn validate(manifest: &InterchangeManifest) -> Result<(), InterchangeValidationError> {
        // 1. Version compatibility
        if !Self::is_version_compatible(&manifest.format_version) {
            // Distinguish "completely unknown" from "wrong major"
            let parsed = Self::parse_version(&manifest.format_version);
            return Err(match parsed {
                Some((major, _)) if major != Self::SUPPORTED_MAJOR => {
                    InterchangeValidationError::IncompatibleFormatVersion {
                        manifest: manifest.format_version.clone(),
                        supported: Self::SUPPORTED_VERSION.to_string(),
                    }
                }
                _ => InterchangeValidationError::UnknownFormatVersion {
                    version: manifest.format_version.clone(),
                },
            });
        }

        // 2. Schema version must be > 0
        if manifest.schema_versions.schema_version == 0 {
            return Err(InterchangeValidationError::InvalidSchemaVersion {
                got: manifest.schema_versions.schema_version,
            });
        }

        // 3. Package checksum must be non-empty
        if manifest.package_checksum.trim().is_empty() {
            return Err(InterchangeValidationError::EmptyPackageChecksum);
        }

        // 4. Sensitivity in 0..=3
        if manifest.scope.max_sensitivity > 3 {
            return Err(InterchangeValidationError::InvalidSensitivity {
                got: manifest.scope.max_sensitivity,
            });
        }

        // 5. created_at must be present
        if manifest.created_at.trim().is_empty() {
            return Err(InterchangeValidationError::MissingRequiredField {
                field: "created_at".to_string(),
            });
        }

        Ok(())
    }

    /// Whether the given `format_version` string is compatible with this
    /// implementation.
    ///
    /// A version is compatible when it parses as `"<major>.<minor>"` and the
    /// major component equals [`Self::SUPPORTED_MAJOR`]. Minor-version
    /// differences are backward-compatible.
    pub fn is_version_compatible(format_version: &str) -> bool {
        match Self::parse_version(format_version) {
            Some((major, _minor)) => major == Self::SUPPORTED_MAJOR,
            None => false,
        }
    }

    /// Parse `"<major>.<minor>"` into `(major, minor)`, returning `None` on
    /// any other shape.
    fn parse_version(s: &str) -> Option<(u32, u32)> {
        let (major_s, minor_s) = s.split_once('.')?;
        let major = major_s.parse::<u32>().ok()?;
        let minor = minor_s.parse::<u32>().ok()?;
        Some((major, minor))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── helpers ─────────────────────────────────────────────────────────

    fn valid_manifest() -> InterchangeManifest {
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
                max_sensitivity: 2,
                include_events: false,
                include_traces: false,
                include_sources: true,
            },
            package_checksum: "abc123def456".to_string(),
            content_ordering: InterchangeOrdering::ByRevision,
            record_count: 10,
            event_count: 5,
            link_count: 3,
            has_extensions: false,
            extensions: None,
        }
    }

    // ── InterchangeVersion ───────────────────────────────────────────────

    #[test]
    fn interchange_version_v1_is_major_1() {
        let v = InterchangeVersion::v1();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 0);
        assert_eq!(v.as_string, "1.0");
    }

    #[test]
    fn is_compatible_with_same_major() {
        let v1 = InterchangeVersion::v1();
        let v1_minor = InterchangeVersion {
            major: 1,
            minor: 5,
            as_string: "1.5".to_string(),
        };
        assert!(v1.is_compatible_with(&v1_minor));
        assert!(v1_minor.is_compatible_with(&v1));
    }

    #[test]
    fn is_not_compatible_with_different_major() {
        let v1 = InterchangeVersion::v1();
        let v2 = InterchangeVersion {
            major: 2,
            minor: 0,
            as_string: "2.0".to_string(),
        };
        assert!(!v1.is_compatible_with(&v2));
        assert!(!v2.is_compatible_with(&v1));
    }

    // ── SecretExclusionRules ─────────────────────────────────────────────

    #[test]
    fn default_safe_has_expected_flags() {
        let rules = SecretExclusionRules::default_safe();
        assert!(rules.exclude_max_sensitivity);
        assert!(rules.exclude_detected_secrets);
        assert!(rules.exclude_shred_keys);
        assert!(rules.excluded_record_ids.is_empty());
    }

    // ── InterchangeManifestValidator::validate ───────────────────────────

    #[test]
    fn validate_ok_for_valid_manifest() {
        let m = valid_manifest();
        assert!(InterchangeManifestValidator::validate(&m).is_ok());
    }

    #[test]
    fn validate_err_for_unknown_version() {
        let mut m = valid_manifest();
        m.format_version = "99.0".to_string();
        let err = InterchangeManifestValidator::validate(&m).unwrap_err();
        assert!(
            matches!(
                err,
                InterchangeValidationError::IncompatibleFormatVersion { .. }
            ),
            "expected IncompatibleFormatVersion, got {err:?}"
        );
    }

    #[test]
    fn validate_err_for_unparseable_version() {
        let mut m = valid_manifest();
        m.format_version = "not-a-version".to_string();
        let err = InterchangeManifestValidator::validate(&m).unwrap_err();
        assert!(
            matches!(err, InterchangeValidationError::UnknownFormatVersion { .. }),
            "expected UnknownFormatVersion, got {err:?}"
        );
    }

    #[test]
    fn validate_err_for_empty_checksum() {
        let mut m = valid_manifest();
        m.package_checksum = "".to_string();
        let err = InterchangeManifestValidator::validate(&m).unwrap_err();
        assert!(
            matches!(err, InterchangeValidationError::EmptyPackageChecksum),
            "expected EmptyPackageChecksum, got {err:?}"
        );
    }

    #[test]
    fn validate_err_for_whitespace_checksum() {
        let mut m = valid_manifest();
        m.package_checksum = "   ".to_string();
        let err = InterchangeManifestValidator::validate(&m).unwrap_err();
        assert!(matches!(
            err,
            InterchangeValidationError::EmptyPackageChecksum
        ));
    }

    #[test]
    fn validate_err_for_invalid_sensitivity() {
        let mut m = valid_manifest();
        m.scope.max_sensitivity = 4; // out of range 0..=3
        let err = InterchangeManifestValidator::validate(&m).unwrap_err();
        assert!(
            matches!(
                err,
                InterchangeValidationError::InvalidSensitivity { got: 4 }
            ),
            "expected InvalidSensitivity(4), got {err:?}"
        );
    }

    #[test]
    fn validate_err_for_zero_schema_version() {
        let mut m = valid_manifest();
        m.schema_versions.schema_version = 0;
        let err = InterchangeManifestValidator::validate(&m).unwrap_err();
        assert!(
            matches!(
                err,
                InterchangeValidationError::InvalidSchemaVersion { got: 0 }
            ),
            "expected InvalidSchemaVersion(0), got {err:?}"
        );
    }

    // ── is_version_compatible ────────────────────────────────────────────

    #[test]
    fn version_compatible_1_0_is_true() {
        assert!(InterchangeManifestValidator::is_version_compatible("1.0"));
    }

    #[test]
    fn version_compatible_1_x_is_true() {
        assert!(InterchangeManifestValidator::is_version_compatible("1.3"));
        assert!(InterchangeManifestValidator::is_version_compatible("1.99"));
    }

    #[test]
    fn version_compatible_2_0_is_false() {
        assert!(!InterchangeManifestValidator::is_version_compatible("2.0"));
    }

    #[test]
    fn version_compatible_0_0_is_false() {
        assert!(!InterchangeManifestValidator::is_version_compatible("0.0"));
    }

    #[test]
    fn version_compatible_garbage_is_false() {
        assert!(!InterchangeManifestValidator::is_version_compatible("abc"));
        assert!(!InterchangeManifestValidator::is_version_compatible(""));
        assert!(!InterchangeManifestValidator::is_version_compatible("1"));
        assert!(!InterchangeManifestValidator::is_version_compatible(
            "1.0.0"
        ));
    }

    // ── InterchangeOrdering serde round-trip ─────────────────────────────

    #[test]
    fn interchange_ordering_serde_roundtrip() {
        let cases = [
            (InterchangeOrdering::ByRevision, "\"by_revision\""),
            (InterchangeOrdering::ByCreatedAt, "\"by_created_at\""),
            (InterchangeOrdering::ByKindThenId, "\"by_kind_then_id\""),
        ];
        for (variant, expected_json) in &cases {
            let serialized = serde_json::to_string(variant).unwrap();
            assert_eq!(
                &serialized, expected_json,
                "serialization mismatch for {variant:?}"
            );
            let deserialized: InterchangeOrdering = serde_json::from_str(&serialized).unwrap();
            assert_eq!(
                &deserialized, variant,
                "deserialization mismatch for {variant:?}"
            );
        }
    }

    // ── Extension round-trip ─────────────────────────────────────────────

    #[test]
    fn manifest_extensions_round_trip_unchanged() {
        let mut m = valid_manifest();
        m.has_extensions = true;
        m.extensions = Some(serde_json::json!({
            "future_field": "some_value",
            "another_field": 42
        }));
        let json = serde_json::to_string(&m).unwrap();
        let back: InterchangeManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.extensions, m.extensions);
        assert!(back.has_extensions);
    }

    #[test]
    fn manifest_without_extensions_is_accepted() {
        let m = valid_manifest();
        assert!(m.extensions.is_none());
        let json = serde_json::to_string(&m).unwrap();
        let back: InterchangeManifest = serde_json::from_str(&json).unwrap();
        assert!(back.extensions.is_none());
        assert!(!back.has_extensions);
    }
}
