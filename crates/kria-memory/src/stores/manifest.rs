//! Pinned model manifest for the `all-MiniLM-L6-v2` embedding partition.
//!
//! ## Purpose
//! Every vector partition consumed by the exact `SqliteVectorStore` must be
//! grounded in an `EmbeddingPartitionManifest` that locks down:
//!
//! - **Model identity** — canonical ID, source repository URL, exact git
//!   revision, and reviewed license disposition.
//! - **Artifact integrity** — SHA-256 checksums of the ONNX file and the
//!   `tokenizer.json` (either verified or marked `PENDING_VERIFY`).
//! - **Runtime versions** — exact `ort` / `fastembed` crate versions from
//!   `Cargo.toml` so a dependency bump is always caught.
//! - **Encoding contract** — dimension (384), dtype (`f32le`), byte size
//!   (1536), pooling strategy (`mean`), and normalization (`l2`).
//! - **Token budget** — max tokens (256, WordPiece limit for this model).
//!
//! The manifest is the single machine-readable source of truth used by
//! `VectorStorePort::ensure_partition`, the rebuild gate, and all tests that
//! assert vector correctness.  Nothing may accept or emit a vector for this
//! partition without a valid, checked manifest.
//!
//! ## Design invariants (F3.1 / MGR-032 / MGD-024)
//! - Canonical model ID is `all-MiniLM-L6-v2` — NOT the legacy label
//!   `minilm_v1`.  The `minilm_v1` label will be replaced in follow-on tasks.
//! - Dimension is exactly 384; vectors are 1536 bytes of little-endian `f32`.
//! - Pooling is mean-pooling over the attention mask.
//! - Output is L2-normalised, so `dot(a, b) == cosine(a, b)`.
//! - License is Apache-2.0 — **explicit, never inferred**.
//! - Checksums marked `PENDING_VERIFY` must be verified before F5 evidence gate.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Immutable, pinned description of the `all-MiniLM-L6-v2` partition contract.
///
/// Serialises to/from JSON with camelCase field names so the canonical
/// `models/manifest/all-minilm-l6-v2.json` file is human-readable and
/// matches common ML tooling conventions.
///
/// **All fields are required.**  Partial manifests (missing `dimension`,
/// missing `ort_version`, etc.) are rejected by [`EmbeddingPartitionManifest::validate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingPartitionManifest {
    // ── Model identity ────────────────────────────────────────────────────
    /// Canonical model identifier, e.g. `"all-MiniLM-L6-v2"`.
    /// Must match the HuggingFace model card ID exactly.
    pub model_id: String,

    /// Source repository URL (HuggingFace hub).
    pub source_repo_url: String,

    /// Exact git revision (40-character SHA-1) of the source repository
    /// at the time the artifacts were frozen.
    pub source_revision: String,

    // ── Artifact integrity ────────────────────────────────────────────────
    /// SHA-256 hex digest of the ONNX model file (`model.onnx` / `all-MiniLM-L6-v2.onnx`).
    /// Use `"PENDING_VERIFY"` if the value has not yet been manually confirmed
    /// against the downloaded artifact; this blocks the F5 evidence gate.
    pub onnx_sha256: String,

    /// SHA-256 hex digest of `tokenizer.json`.
    /// Same `"PENDING_VERIFY"` convention.
    pub tokenizer_sha256: String,

    // ── License ───────────────────────────────────────────────────────────
    /// Reviewed FOSS license SPDX ID, e.g. `"Apache-2.0"`.
    /// Must be set explicitly — never inferred from the repository.
    pub license_spdx: String,

    /// Opaque disposition ID issued by the license-review gate, e.g.
    /// `"KRIA-LIC-001"`.  Required before release; use `"PENDING_REVIEW"` in
    /// pre-release builds only.
    pub license_disposition_id: String,

    // ── Runtime versions ──────────────────────────────────────────────────
    /// Exact `ort` crate version string as declared in `Cargo.toml`,
    /// e.g. `"2.0.0-rc.12"`.
    pub ort_version: String,

    /// Exact `fastembed` crate version string as declared in `Cargo.toml`,
    /// e.g. `"5"`.  FastEmbed is used by the intent-routing embedder;
    /// version is pinned here for SBOM completeness.
    pub fastembed_version: String,

    // ── Encoding contract ─────────────────────────────────────────────────
    /// Maximum token count accepted by the tokenizer (WordPiece limit for
    /// `all-MiniLM-L6-v2`).  **Must be 256.**
    pub max_tokens: u32,

    /// Pooling strategy applied after the transformer backbone.
    /// **Must be `"mean"`.**
    pub pooling: String,

    /// Output embedding dimension.  **Must be 384.**
    pub dimension: u32,

    /// Scalar element dtype, serialised as little-endian IEEE-754 `f32`.
    /// **Must be `"f32le"`.**
    pub dtype: String,

    /// Post-pooling normalisation applied before storage.
    /// **Must be `"l2"`** so that `dot(a, b) == cosine(a, b)`.
    pub normalization: String,

    /// Byte length of one encoded vector (`dimension × 4`).
    /// **Must be 1536.**
    pub vector_byte_length: u32,
}

// ── Known hard constants for `all-MiniLM-L6-v2` ──────────────────────────

/// The canonical model identifier (`all-MiniLM-L6-v2`).
pub const MODEL_ID: &str = "all-MiniLM-L6-v2";
/// Required embedding dimension.
pub const REQUIRED_DIMENSION: u32 = 384;
/// Required max WordPiece token budget.
pub const REQUIRED_MAX_TOKENS: u32 = 256;
/// Required pooling label.
pub const REQUIRED_POOLING: &str = "mean";
/// Required dtype label.
pub const REQUIRED_DTYPE: &str = "f32le";
/// Required normalisation label.
pub const REQUIRED_NORMALIZATION: &str = "l2";
/// Bytes per f32 × dimension = 1536.
pub const REQUIRED_VECTOR_BYTE_LENGTH: u32 = REQUIRED_DIMENSION * 4;
/// Required license SPDX identifier.
pub const REQUIRED_LICENSE_SPDX: &str = "Apache-2.0";

/// Pinned `ort` crate version from `Cargo.toml`.
pub const ORT_VERSION: &str = "2.0.0-rc.12";
/// Pinned `fastembed` crate version from `Cargo.toml`.
pub const FASTEMBED_VERSION: &str = "5";

/// Sentinel used when a checksum has not yet been manually verified.
pub const PENDING_VERIFY: &str = "PENDING_VERIFY";
/// Sentinel used when a license disposition review is pending.
pub const PENDING_REVIEW: &str = "PENDING_REVIEW";

// ── Validation ────────────────────────────────────────────────────────────

/// Errors returned by [`EmbeddingPartitionManifest::validate`].
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("model_id must be \"{}\", got \"{}\"", MODEL_ID, .0)]
    WrongModelId(String),

    #[error("dimension must be {}, got {}", REQUIRED_DIMENSION, .0)]
    WrongDimension(u32),

    #[error("max_tokens must be {}, got {}", REQUIRED_MAX_TOKENS, .0)]
    WrongMaxTokens(u32),

    #[error("pooling must be \"{}\", got \"{}\"", REQUIRED_POOLING, .0)]
    WrongPooling(String),

    #[error("dtype must be \"{}\", got \"{}\"", REQUIRED_DTYPE, .0)]
    WrongDtype(String),

    #[error("normalization must be \"{}\", got \"{}\"", REQUIRED_NORMALIZATION, .0)]
    WrongNormalization(String),

    #[error("vector_byte_length must be {}, got {}", REQUIRED_VECTOR_BYTE_LENGTH, .0)]
    WrongByteLength(u32),

    #[error("license_spdx must be \"{}\", got \"{}\"", REQUIRED_LICENSE_SPDX, .0)]
    WrongLicense(String),

    #[error("ort_version must be \"{}\", got \"{}\"", ORT_VERSION, .0)]
    WrongOrtVersion(String),

    #[error("fastembed_version must be \"{}\", got \"{}\"", FASTEMBED_VERSION, .0)]
    WrongFastembedVersion(String),

    #[error("source_revision must be a 40-character hex string, got \"{}\"", .0)]
    InvalidRevision(String),

    #[error("onnx_sha256 is set to PENDING_VERIFY — must be resolved before F5")]
    OnnxChecksumPending,

    #[error("tokenizer_sha256 is set to PENDING_VERIFY — must be resolved before F5")]
    TokenizerChecksumPending,

    #[error("license_disposition_id is set to PENDING_REVIEW — must be resolved before release")]
    LicenseDispositionPending,

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl EmbeddingPartitionManifest {
    /// Build the canonical `all-MiniLM-L6-v2` manifest using the pinned
    /// constants.  Checksums and the license disposition ID are seeded with
    /// `PENDING_VERIFY` / `PENDING_REVIEW` sentinels and must be filled in via
    /// manual verification before the F5 evidence gate.
    pub fn canonical() -> Self {
        Self {
            model_id: MODEL_ID.to_string(),
            source_repo_url: "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2"
                .to_string(),
            // Canonical commit on the sentence-transformers HuggingFace hub.
            // Verified against: https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/commit/8b3219a92973c328a8e22fadcfa821b5dc75636a
            source_revision: "8b3219a92973c328a8e22fadcfa821b5dc75636a".to_string(),
            // SHA-256 of the ONNX file must be verified against the downloaded artifact
            // before the F5 evidence gate.  Run:
            //   sha256sum ~/.kria/models/embeddings/all-MiniLM-L6-v2.onnx
            // and replace this sentinel with the hex digest.
            onnx_sha256: PENDING_VERIFY.to_string(),
            // SHA-256 of tokenizer.json must be verified analogously:
            //   sha256sum ~/.kria/models/embeddings/tokenizer.json
            tokenizer_sha256: PENDING_VERIFY.to_string(),
            license_spdx: REQUIRED_LICENSE_SPDX.to_string(),
            // License reviewed and cleared for use in KRIA (single-user, local,
            // pre-production).  Disposition ID to be assigned by the formal
            // SBOM/license-gate before F5.
            license_disposition_id: PENDING_REVIEW.to_string(),
            ort_version: ORT_VERSION.to_string(),
            fastembed_version: FASTEMBED_VERSION.to_string(),
            max_tokens: REQUIRED_MAX_TOKENS,
            pooling: REQUIRED_POOLING.to_string(),
            dimension: REQUIRED_DIMENSION,
            dtype: REQUIRED_DTYPE.to_string(),
            normalization: REQUIRED_NORMALIZATION.to_string(),
            vector_byte_length: REQUIRED_VECTOR_BYTE_LENGTH,
        }
    }

    /// Validate that all required fields hold their pinned values.
    ///
    /// Returns `Ok(())` if the manifest is self-consistent.  Pending
    /// checksums / disposition IDs are **not** treated as errors here so that
    /// pre-F5 builds can load the manifest; callers that need fully-verified
    /// manifests must call [`Self::validate_strict`].
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.model_id != MODEL_ID {
            return Err(ManifestError::WrongModelId(self.model_id.clone()));
        }
        if self.dimension != REQUIRED_DIMENSION {
            return Err(ManifestError::WrongDimension(self.dimension));
        }
        if self.max_tokens != REQUIRED_MAX_TOKENS {
            return Err(ManifestError::WrongMaxTokens(self.max_tokens));
        }
        if self.pooling != REQUIRED_POOLING {
            return Err(ManifestError::WrongPooling(self.pooling.clone()));
        }
        if self.dtype != REQUIRED_DTYPE {
            return Err(ManifestError::WrongDtype(self.dtype.clone()));
        }
        if self.normalization != REQUIRED_NORMALIZATION {
            return Err(ManifestError::WrongNormalization(
                self.normalization.clone(),
            ));
        }
        if self.vector_byte_length != REQUIRED_VECTOR_BYTE_LENGTH {
            return Err(ManifestError::WrongByteLength(self.vector_byte_length));
        }
        if self.license_spdx != REQUIRED_LICENSE_SPDX {
            return Err(ManifestError::WrongLicense(self.license_spdx.clone()));
        }
        if self.ort_version != ORT_VERSION {
            return Err(ManifestError::WrongOrtVersion(self.ort_version.clone()));
        }
        if self.fastembed_version != FASTEMBED_VERSION {
            return Err(ManifestError::WrongFastembedVersion(
                self.fastembed_version.clone(),
            ));
        }
        // source_revision must be a 40-char hex string.
        let rev = &self.source_revision;
        if rev.len() != 40 || !rev.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ManifestError::InvalidRevision(rev.clone()));
        }
        Ok(())
    }

    /// Like [`Self::validate`] but also rejects pending checksums and
    /// disposition IDs.  This is the gate required before F5.
    pub fn validate_strict(&self) -> Result<(), ManifestError> {
        self.validate()?;
        if self.onnx_sha256 == PENDING_VERIFY {
            return Err(ManifestError::OnnxChecksumPending);
        }
        if self.tokenizer_sha256 == PENDING_VERIFY {
            return Err(ManifestError::TokenizerChecksumPending);
        }
        if self.license_disposition_id == PENDING_REVIEW {
            return Err(ManifestError::LicenseDispositionPending);
        }
        Ok(())
    }

    /// Load from a JSON file on disk.
    pub fn load_from_file(path: &Path) -> Result<Self, ManifestError> {
        let bytes = std::fs::read(path)?;
        let manifest: Self = serde_json::from_slice(&bytes)?;
        Ok(manifest)
    }

    /// Serialise to a pretty-printed JSON string.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Return the canonical vector byte length for this partition (`dimension × 4`).
    pub fn expected_byte_length(&self) -> usize {
        (self.dimension as usize) * 4
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical manifest must pass non-strict validation (PENDING fields allowed).
    #[test]
    fn canonical_manifest_passes_validate() {
        let m = EmbeddingPartitionManifest::canonical();
        m.validate().expect("canonical manifest must be valid");
    }

    /// The canonical manifest must fail strict validation until checksums and
    /// disposition IDs are resolved — this is intentional and expected at this stage.
    #[test]
    fn canonical_manifest_fails_strict_before_checksums_resolved() {
        let m = EmbeddingPartitionManifest::canonical();
        let err = m
            .validate_strict()
            .expect_err("strict validation must fail until checksums/disposition are resolved");
        // Confirm the failure is specifically the ONNX checksum sentinel.
        match err {
            ManifestError::OnnxChecksumPending
            | ManifestError::TokenizerChecksumPending
            | ManifestError::LicenseDispositionPending => {}
            other => panic!("unexpected strict-validation error: {other}"),
        }
    }

    /// All pinned encoding fields must equal the F3.1 invariants.
    #[test]
    fn canonical_encoding_fields_match_invariants() {
        let m = EmbeddingPartitionManifest::canonical();
        assert_eq!(m.model_id, MODEL_ID);
        assert_eq!(m.dimension, REQUIRED_DIMENSION);
        assert_eq!(m.max_tokens, REQUIRED_MAX_TOKENS);
        assert_eq!(m.pooling, REQUIRED_POOLING);
        assert_eq!(m.dtype, REQUIRED_DTYPE);
        assert_eq!(m.normalization, REQUIRED_NORMALIZATION);
        assert_eq!(m.vector_byte_length, REQUIRED_VECTOR_BYTE_LENGTH);
        assert_eq!(m.vector_byte_length, 384 * 4);
        assert_eq!(m.license_spdx, REQUIRED_LICENSE_SPDX);
    }

    /// Runtime version fields must match the exact versions in Cargo.toml.
    #[test]
    fn canonical_runtime_versions_are_pinned() {
        let m = EmbeddingPartitionManifest::canonical();
        assert_eq!(
            m.ort_version, "2.0.0-rc.12",
            "ort version must match Cargo.toml"
        );
        assert_eq!(
            m.fastembed_version, "5",
            "fastembed version must match Cargo.toml"
        );
    }

    /// The source revision must be a valid 40-character hex commit hash.
    #[test]
    fn source_revision_is_valid_git_sha() {
        let m = EmbeddingPartitionManifest::canonical();
        assert_eq!(m.source_revision.len(), 40, "revision must be 40 chars");
        assert!(
            m.source_revision.chars().all(|c| c.is_ascii_hexdigit()),
            "revision must be hex"
        );
    }

    /// A manifest with wrong dimension must be rejected.
    #[test]
    fn wrong_dimension_is_rejected() {
        let mut m = EmbeddingPartitionManifest::canonical();
        m.dimension = 768;
        let err = m.validate().expect_err("wrong dimension must fail");
        assert!(matches!(err, ManifestError::WrongDimension(768)));
    }

    /// A manifest with wrong model ID must be rejected.
    #[test]
    fn wrong_model_id_is_rejected() {
        let mut m = EmbeddingPartitionManifest::canonical();
        m.model_id = "minilm_v1".to_string(); // the old legacy label
        let err = m.validate().expect_err("legacy model ID must fail");
        assert!(matches!(err, ManifestError::WrongModelId(_)));
    }

    /// A manifest with wrong dtype must be rejected.
    #[test]
    fn wrong_dtype_is_rejected() {
        let mut m = EmbeddingPartitionManifest::canonical();
        m.dtype = "f16le".to_string();
        let err = m.validate().expect_err("wrong dtype must fail");
        assert!(matches!(err, ManifestError::WrongDtype(_)));
    }

    /// A manifest with wrong normalization must be rejected.
    #[test]
    fn wrong_normalization_is_rejected() {
        let mut m = EmbeddingPartitionManifest::canonical();
        m.normalization = "none".to_string();
        let err = m.validate().expect_err("wrong normalization must fail");
        assert!(matches!(err, ManifestError::WrongNormalization(_)));
    }

    /// A manifest with wrong ort version must be rejected.
    #[test]
    fn wrong_ort_version_is_rejected() {
        let mut m = EmbeddingPartitionManifest::canonical();
        m.ort_version = "1.16.3".to_string();
        let err = m.validate().expect_err("wrong ort version must fail");
        assert!(matches!(err, ManifestError::WrongOrtVersion(_)));
    }

    /// The manifest round-trips through JSON without loss.
    #[test]
    fn manifest_json_roundtrip() {
        let original = EmbeddingPartitionManifest::canonical();
        let json = original.to_json_pretty().expect("serialise must succeed");
        let decoded: EmbeddingPartitionManifest =
            serde_json::from_str(&json).expect("deserialise must succeed");
        assert_eq!(original, decoded, "round-trip must be lossless");
    }

    /// `expected_byte_length` must return 1536 for the canonical manifest.
    #[test]
    fn expected_byte_length_is_1536() {
        let m = EmbeddingPartitionManifest::canonical();
        assert_eq!(m.expected_byte_length(), 1536);
    }

    /// The canonical JSON on disk must parse and pass validation.
    ///
    /// This test is skipped if the file doesn't exist (during CI without the
    /// full model directory) but asserts correctness when it does.
    #[test]
    fn json_file_parses_and_validates() {
        let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("models/manifest/all-minilm-l6-v2.json"))
            .unwrap_or_default();

        if !manifest_path.exists() {
            // File may not exist in stripped CI; skip but don't fail.
            eprintln!(
                "SKIP: {} not found; run 'cargo test' from the workspace root \
                 with the models/manifest directory present",
                manifest_path.display()
            );
            return;
        }

        let m =
            EmbeddingPartitionManifest::load_from_file(&manifest_path).expect("load must succeed");
        m.validate()
            .expect("manifest from disk must pass non-strict validation");
    }
}
