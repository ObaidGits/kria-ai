//! Bounded-chunk streaming, hash computation, and semantic candidate types for
//! consent-gated source ingestion (design §5.4, task F2.6.3 / MGR-046).
//!
//! ## Key behavioral rules (MGR-046)
//!
//! 1. **Bounded chunks**: `content.len() <= MAX_CHUNK_BYTES` (1 MiB). Larger
//!    content fails [`IngestionChunk::validate`].
//! 2. **Hash integrity**: `content_hash` is the SHA-256 hex of `content`.
//!    Mismatches fail [`IngestionChunk::validate`].
//! 3. **Complete semantic units**: [`SemanticCandidate`] represents a complete
//!    semantic unit. Partial records MUST NOT be submitted to WritePolicyEngine
//!    (design §5.4: "interruption commits no partial semantic record").
//! 4. **Deduplication**: [`ItemHash::identity_hash`] = SHA-256(content_hash +
//!    version_hash). When a candidate's identity hash matches a stored hash,
//!    `is_duplicate = true`.
//! 5. **Policy inheritance**: [`SemanticCandidate`] inherits policy from the
//!    source (namespace / scope / sensitivity).
//! 6. **SHA-256**: uses the `sha2` crate (workspace dependency).

use sha2::{Digest, Sha256};

// ── ChunkBound constant ────────────────────────────────────────────────────

/// Maximum chunk size in bytes (1 MiB).
///
/// Chunks larger than this value are rejected by [`IngestionChunk::validate`].
pub const MAX_CHUNK_BYTES: usize = 1024 * 1024;

// ── ChunkValidationError ───────────────────────────────────────────────────

/// Errors produced by [`IngestionChunk::validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkValidationError {
    /// The chunk content exceeds [`MAX_CHUNK_BYTES`].
    ExceedsMaxSize {
        /// The actual byte count.
        got: usize,
        /// The maximum allowed byte count.
        max: usize,
    },
    /// The stored `content_hash` does not match the hash of `content`.
    HashMismatch {
        /// The hash stored in the chunk.
        stored: String,
        /// The hash actually computed from `content`.
        computed: String,
    },
}

impl std::fmt::Display for ChunkValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChunkValidationError::ExceedsMaxSize { got, max } => {
                write!(
                    f,
                    "chunk exceeds maximum size: {got} bytes (max {max} bytes)"
                )
            }
            ChunkValidationError::HashMismatch { stored, computed } => {
                write!(
                    f,
                    "chunk hash mismatch: stored={stored}, computed={computed}"
                )
            }
        }
    }
}

impl std::error::Error for ChunkValidationError {}

// ── IngestionChunk ─────────────────────────────────────────────────────────

/// One bounded chunk of source content ready for processing.
///
/// Each chunk is ≤ [`MAX_CHUNK_BYTES`] bytes. Larger content must be split
/// across multiple chunks. Each chunk carries a content hash computed over its
/// bytes.
#[derive(Debug, Clone)]
pub struct IngestionChunk {
    /// The source ID this chunk belongs to.
    pub source_id: String,
    /// Chunk sequence number (0-indexed).
    pub sequence: u64,
    /// The raw bytes of this chunk (≤ [`MAX_CHUNK_BYTES`]).
    pub content: Vec<u8>,
    /// SHA-256 hex hash of `content`.
    pub content_hash: String,
    /// Byte offset of this chunk in the source.
    pub byte_offset: u64,
    /// Whether this is the final chunk.
    pub is_final: bool,
    /// Structured locator (policy-safe JSON) identifying where in the source
    /// this chunk is from.
    pub locator_json: Option<String>,
}

impl IngestionChunk {
    /// Validate that the chunk satisfies both invariants:
    /// 1. `content.len() <= MAX_CHUNK_BYTES`
    /// 2. `content_hash` matches the SHA-256 hex of `content`
    pub fn validate(&self) -> Result<(), ChunkValidationError> {
        // Invariant 1: size bound.
        if self.content.len() > MAX_CHUNK_BYTES {
            return Err(ChunkValidationError::ExceedsMaxSize {
                got: self.content.len(),
                max: MAX_CHUNK_BYTES,
            });
        }

        // Invariant 2: hash integrity.
        let computed = Self::compute_hash(&self.content);
        if computed != self.content_hash {
            return Err(ChunkValidationError::HashMismatch {
                stored: self.content_hash.clone(),
                computed,
            });
        }

        Ok(())
    }

    /// Compute the SHA-256 hex hash of the given bytes (without storing them).
    pub fn compute_hash(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }
}

// ── ItemHash ───────────────────────────────────────────────────────────────

/// The hash identity of a semantic ingestion item.
///
/// Used for deduplication: an item with the same content hash and version hash
/// as a stored item may be reused without re-ingesting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemHash {
    /// SHA-256 hex hash of the item's canonical content.
    pub content_hash: String,
    /// SHA-256 hex hash of the item's version metadata (source version, schema).
    pub version_hash: String,
    /// The combined identity hash (SHA-256 of `content_hash` concatenated with
    /// `version_hash`).
    pub identity_hash: String,
}

impl ItemHash {
    /// Compute an [`ItemHash`] from content bytes and a version string.
    ///
    /// - `content_hash`  = SHA-256(content_bytes)
    /// - `version_hash`  = SHA-256(version.as_bytes())
    /// - `identity_hash` = SHA-256(content_hash_hex + version_hash_hex)
    pub fn compute(content_bytes: &[u8], version: &str) -> Self {
        let content_hash = sha256_hex(content_bytes);
        let version_hash = sha256_hex(version.as_bytes());
        // Identity = SHA-256 over the concatenation of the two hex strings.
        let identity_input = format!("{content_hash}{version_hash}");
        let identity_hash = sha256_hex(identity_input.as_bytes());

        Self {
            content_hash,
            version_hash,
            identity_hash,
        }
    }

    /// Whether this hash is identical to another (same content + version).
    ///
    /// Equality is determined by the `identity_hash` field alone, which
    /// encodes both content and version.
    pub fn matches(&self, other: &ItemHash) -> bool {
        self.identity_hash == other.identity_hash
    }
}

/// Internal helper: SHA-256 hex of arbitrary bytes.
fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

// ── SemanticCandidate ──────────────────────────────────────────────────────

/// A complete semantic candidate extracted from a source chunk.
///
/// A semantic candidate represents one coherent piece of knowledge extracted
/// from source content. It must be **complete** (not spanning chunks) before
/// being submitted to the WritePolicyEngine.
///
/// Design §5.4: *"each semantic write governed; interruption commits no partial
/// semantic record."*
#[derive(Debug, Clone)]
pub struct SemanticCandidate {
    /// The source ID this candidate came from.
    pub source_id: String,
    /// The chunk sequence number this candidate was extracted from.
    pub chunk_sequence: u64,
    /// The item identity hash.
    pub item_hash: ItemHash,
    /// The record kind (e.g. `"memory"`, `"entity"`, `"relationship"`).
    pub record_kind: String,
    /// The canonical content (policy-safe, to be written to authority).
    pub content: String,
    /// The structured locator JSON (where in the source this came from).
    pub locator_json: Option<String>,
    /// The source's policy namespace to inherit.
    pub policy_namespace: String,
    /// The source's policy scope to inherit.
    pub policy_scope: String,
    /// The source's policy sensitivity to inherit (`0..=3`).
    pub policy_sensitivity: u8,
    /// Whether this candidate was already present (dedup hit).
    pub is_duplicate: bool,
}

// ── IngestionBoundary ──────────────────────────────────────────────────────

/// Tracks whether a semantic candidate spans chunk boundaries (not permitted).
///
/// A semantic candidate that spans two chunks cannot be committed atomically.
/// The ingestion worker must buffer content across chunks to find complete
/// semantic units before creating a [`SemanticCandidate`].
#[derive(Debug, Clone)]
pub struct IngestionBoundary {
    /// The chunk sequence number where this boundary started.
    pub started_at_sequence: u64,
    /// Whether the candidate is complete (fully within one or more chunks
    /// ending at a semantic boundary, not a byte boundary).
    pub is_complete: bool,
    /// The byte offset in the current chunk where the boundary was detected.
    pub boundary_offset: usize,
}

// ── ChunkProcessor ─────────────────────────────────────────────────────────

/// Stateless helper that processes chunks and produces hash / dedup verdicts.
pub struct ChunkProcessor;

impl ChunkProcessor {
    /// Validate a chunk: checks the size bound and hash integrity.
    ///
    /// Delegates to [`IngestionChunk::validate`].
    pub fn validate_chunk(chunk: &IngestionChunk) -> Result<(), ChunkValidationError> {
        chunk.validate()
    }

    /// Compute a chunk's SHA-256 hex hash from its content bytes.
    ///
    /// Delegates to [`IngestionChunk::compute_hash`].
    pub fn compute_chunk_hash(content: &[u8]) -> String {
        IngestionChunk::compute_hash(content)
    }

    /// Check whether a candidate is a duplicate of an existing stored item.
    ///
    /// Returns `true` when `candidate_hash.identity_hash` matches the
    /// `identity_hash` of any element in `stored_hashes`.
    pub fn is_duplicate(candidate_hash: &ItemHash, stored_hashes: &[ItemHash]) -> bool {
        stored_hashes
            .iter()
            .any(|h| h.identity_hash == candidate_hash.identity_hash)
    }
}

// ── Unit tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helper: build a valid IngestionChunk ────────────────────────────

    fn make_chunk(content: Vec<u8>) -> IngestionChunk {
        let hash = IngestionChunk::compute_hash(&content);
        IngestionChunk {
            source_id: "source-001".to_owned(),
            sequence: 0,
            content,
            content_hash: hash,
            byte_offset: 0,
            is_final: false,
            locator_json: None,
        }
    }

    // ── MAX_CHUNK_BYTES constant ────────────────────────────────────────

    #[test]
    fn max_chunk_bytes_is_one_mib() {
        assert_eq!(MAX_CHUNK_BYTES, 1024 * 1024);
    }

    // ── IngestionChunk::validate — size bound ───────────────────────────

    #[test]
    fn validate_ok_for_empty_chunk() {
        let chunk = make_chunk(vec![]);
        assert!(chunk.validate().is_ok());
    }

    #[test]
    fn validate_ok_for_exactly_one_mib() {
        let chunk = make_chunk(vec![0u8; MAX_CHUNK_BYTES]);
        assert!(chunk.validate().is_ok());
    }

    #[test]
    fn validate_err_for_one_byte_over_limit() {
        let chunk = make_chunk(vec![0u8; MAX_CHUNK_BYTES + 1]);
        let err = chunk.validate().unwrap_err();
        assert!(
            matches!(
                err,
                ChunkValidationError::ExceedsMaxSize {
                    got,
                    max
                } if got == MAX_CHUNK_BYTES + 1 && max == MAX_CHUNK_BYTES
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_err_for_clearly_oversized_chunk() {
        let chunk = make_chunk(vec![0u8; MAX_CHUNK_BYTES * 2]);
        assert!(matches!(
            chunk.validate().unwrap_err(),
            ChunkValidationError::ExceedsMaxSize { .. }
        ));
    }

    // ── IngestionChunk::validate — hash integrity ───────────────────────

    #[test]
    fn validate_err_on_hash_mismatch() {
        let mut chunk = make_chunk(b"hello world".to_vec());
        chunk.content_hash = "deadbeef".to_owned();

        let err = chunk.validate().unwrap_err();
        assert!(
            matches!(err, ChunkValidationError::HashMismatch { .. }),
            "expected HashMismatch, got: {err}"
        );
    }

    #[test]
    fn validate_err_displays_stored_and_computed_hashes() {
        let mut chunk = make_chunk(b"test".to_vec());
        chunk.content_hash = "bad_hash".to_owned();
        let err = chunk.validate().unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("bad_hash"), "missing stored hash in: {msg}");
        assert!(msg.contains("mismatch"), "missing 'mismatch' in: {msg}");
    }

    // ── IngestionChunk::compute_hash — determinism ──────────────────────

    #[test]
    fn compute_hash_is_deterministic() {
        let bytes = b"deterministic content for hashing";
        let h1 = IngestionChunk::compute_hash(bytes);
        let h2 = IngestionChunk::compute_hash(bytes);
        assert_eq!(h1, h2, "hash must be deterministic");
    }

    #[test]
    fn compute_hash_differs_for_different_inputs() {
        let h1 = IngestionChunk::compute_hash(b"content a");
        let h2 = IngestionChunk::compute_hash(b"content b");
        assert_ne!(h1, h2);
    }

    #[test]
    fn compute_hash_is_64_hex_chars() {
        // SHA-256 produces 32 bytes = 64 hex characters.
        let h = IngestionChunk::compute_hash(b"test");
        assert_eq!(h.len(), 64, "expected 64-char hex: {h}");
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()), "not hex: {h}");
    }

    #[test]
    fn compute_hash_known_value() {
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let h = IngestionChunk::compute_hash(b"");
        assert_eq!(
            h,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    // ── ItemHash::compute — determinism ─────────────────────────────────

    #[test]
    fn item_hash_compute_is_deterministic() {
        let h1 = ItemHash::compute(b"my content", "v1.0");
        let h2 = ItemHash::compute(b"my content", "v1.0");
        assert_eq!(h1, h2);
    }

    #[test]
    fn item_hash_identity_is_64_hex_chars() {
        let h = ItemHash::compute(b"content", "version");
        assert_eq!(h.identity_hash.len(), 64);
        assert_eq!(h.content_hash.len(), 64);
        assert_eq!(h.version_hash.len(), 64);
    }

    #[test]
    fn item_hash_different_content_produces_different_hash() {
        let h1 = ItemHash::compute(b"content a", "v1");
        let h2 = ItemHash::compute(b"content b", "v1");
        assert_ne!(h1.content_hash, h2.content_hash);
        assert_ne!(h1.identity_hash, h2.identity_hash);
    }

    #[test]
    fn item_hash_different_version_produces_different_hash() {
        let h1 = ItemHash::compute(b"same content", "v1");
        let h2 = ItemHash::compute(b"same content", "v2");
        assert_eq!(
            h1.content_hash, h2.content_hash,
            "content_hash should be same"
        );
        assert_ne!(h1.version_hash, h2.version_hash);
        assert_ne!(h1.identity_hash, h2.identity_hash);
    }

    // ── ItemHash::matches ────────────────────────────────────────────────

    #[test]
    fn item_hash_matches_true_for_same_content_and_version() {
        let h1 = ItemHash::compute(b"data", "1.0");
        let h2 = ItemHash::compute(b"data", "1.0");
        assert!(h1.matches(&h2));
        assert!(h2.matches(&h1));
    }

    #[test]
    fn item_hash_matches_false_for_different_content() {
        let h1 = ItemHash::compute(b"data a", "1.0");
        let h2 = ItemHash::compute(b"data b", "1.0");
        assert!(!h1.matches(&h2));
    }

    #[test]
    fn item_hash_matches_false_for_different_version() {
        let h1 = ItemHash::compute(b"data", "1.0");
        let h2 = ItemHash::compute(b"data", "2.0");
        assert!(!h1.matches(&h2));
    }

    // ── ChunkProcessor::is_duplicate ────────────────────────────────────

    #[test]
    fn is_duplicate_true_when_hash_in_stored_list() {
        let candidate = ItemHash::compute(b"some content", "v1");
        let stored = vec![
            ItemHash::compute(b"other content", "v1"),
            ItemHash::compute(b"some content", "v1"), // exact match
            ItemHash::compute(b"yet another", "v1"),
        ];
        assert!(ChunkProcessor::is_duplicate(&candidate, &stored));
    }

    #[test]
    fn is_duplicate_false_when_hash_not_in_stored_list() {
        let candidate = ItemHash::compute(b"new content", "v1");
        let stored = vec![
            ItemHash::compute(b"content a", "v1"),
            ItemHash::compute(b"content b", "v1"),
        ];
        assert!(!ChunkProcessor::is_duplicate(&candidate, &stored));
    }

    #[test]
    fn is_duplicate_false_for_empty_stored_list() {
        let candidate = ItemHash::compute(b"anything", "v1");
        assert!(!ChunkProcessor::is_duplicate(&candidate, &[]));
    }

    #[test]
    fn is_duplicate_false_when_content_same_but_version_differs() {
        let candidate = ItemHash::compute(b"content", "v2");
        let stored = vec![ItemHash::compute(b"content", "v1")];
        assert!(!ChunkProcessor::is_duplicate(&candidate, &stored));
    }

    // ── SemanticCandidate — is_duplicate flag ───────────────────────────

    #[test]
    fn semantic_candidate_is_duplicate_flag_set_correctly() {
        let hash = ItemHash::compute(b"record content", "v1");
        let stored = vec![ItemHash::compute(b"record content", "v1")];
        let is_dup = ChunkProcessor::is_duplicate(&hash, &stored);

        let candidate = SemanticCandidate {
            source_id: "src-001".to_owned(),
            chunk_sequence: 0,
            item_hash: hash,
            record_kind: "memory".to_owned(),
            content: "record content".to_owned(),
            locator_json: None,
            policy_namespace: "user".to_owned(),
            policy_scope: "personal".to_owned(),
            policy_sensitivity: 0,
            is_duplicate: is_dup,
        };

        assert!(candidate.is_duplicate);
    }

    #[test]
    fn semantic_candidate_not_duplicate_for_new_content() {
        let hash = ItemHash::compute(b"brand new content", "v1");
        let stored: Vec<ItemHash> = vec![];
        let is_dup = ChunkProcessor::is_duplicate(&hash, &stored);

        let candidate = SemanticCandidate {
            source_id: "src-002".to_owned(),
            chunk_sequence: 1,
            item_hash: hash,
            record_kind: "entity".to_owned(),
            content: "brand new content".to_owned(),
            locator_json: Some(r#"{"path": "/doc/section/1"}"#.to_owned()),
            policy_namespace: "user".to_owned(),
            policy_scope: "work".to_owned(),
            policy_sensitivity: 1,
            is_duplicate: is_dup,
        };

        assert!(!candidate.is_duplicate);
    }

    // ── ChunkProcessor::validate_chunk ──────────────────────────────────

    #[test]
    fn chunk_processor_validate_delegates_to_chunk() {
        let chunk = make_chunk(b"valid content".to_vec());
        assert!(ChunkProcessor::validate_chunk(&chunk).is_ok());

        let oversized = make_chunk(vec![0u8; MAX_CHUNK_BYTES + 1]);
        assert!(ChunkProcessor::validate_chunk(&oversized).is_err());
    }

    // ── ChunkProcessor::compute_chunk_hash ──────────────────────────────

    #[test]
    fn chunk_processor_compute_hash_matches_chunk_compute_hash() {
        let content = b"processor hash test";
        let via_processor = ChunkProcessor::compute_chunk_hash(content);
        let via_chunk = IngestionChunk::compute_hash(content);
        assert_eq!(via_processor, via_chunk);
    }

    // ── IngestionBoundary ────────────────────────────────────────────────

    #[test]
    fn ingestion_boundary_fields_accessible() {
        let b = IngestionBoundary {
            started_at_sequence: 3,
            is_complete: true,
            boundary_offset: 512,
        };
        assert_eq!(b.started_at_sequence, 3);
        assert!(b.is_complete);
        assert_eq!(b.boundary_offset, 512);
    }

    // ── Round-trip: compute hash then validate ───────────────────────────

    #[test]
    fn chunk_created_with_correct_hash_validates() {
        let content = b"round-trip validation test".to_vec();
        let hash = IngestionChunk::compute_hash(&content);
        let chunk = IngestionChunk {
            source_id: "src".to_owned(),
            sequence: 7,
            content,
            content_hash: hash,
            byte_offset: 4096,
            is_final: true,
            locator_json: Some(r#"{"line": 42}"#.to_owned()),
        };
        assert!(chunk.validate().is_ok());
    }
}
