//! Exact, policy-prefiltered 384-d cosine vector store backed by SQLite.
//!
//! ## Architecture (F3.1 / MGR-032 / MGD-024)
//!
//! * [`PartitionId`] — typed newtype that gates every read/write path; obtained
//!   only through [`ensure_partition`].
//! * [`ensure_partition`] — validates an [`EmbeddingPartitionManifest`], upserts
//!   the `embedding_partitions` row, and returns a `PartitionId`.
//! * [`SqliteVectorStore`] — implements [`VectorStore`] over `mem_vectors_v2`.
//!   The legacy `mem_vectors` table (migration 0002) is untouched here; the
//!   F3.1 write-path cutover replaces callers.
//!
//! ## Encoding invariants
//! * Vectors are exactly 1536 bytes of little-endian f32 (`dimension × 4`).
//! * The `sensitivity` column stores `0-3` integers mapping onto
//!   [`SensitivityLevel`]: 0=public, 1=internal, 2=private, 3=secret.
//! * Policy filtering (namespace, scope, sensitivity, truth_state) runs
//!   entirely in SQL before any BLOB is decoded.

use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::types::{
    ModelVersion, Scope, ScopeFilter, Sensitivity, VectorHit, VectorPayload,
};

use super::manifest::{EmbeddingPartitionManifest, ManifestError};
use super::ports::VectorStore;

// ─── PartitionId ─────────────────────────────────────────────────────────────

/// A typed, validated partition identifier.
///
/// Only obtainable through [`ensure_partition`], which validates the manifest
/// and guarantees the `embedding_partitions` row exists.  This type-state
/// prevents any code from issuing vector reads/writes against an unregistered
/// or mis-matched partition.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PartitionId(String);

impl PartitionId {
    /// The raw string value (for SQL parameters).
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Construct a `PartitionId` directly from a raw string value.
    ///
    /// This bypasses [`ensure_partition`] validation and should only be used
    /// when the caller has already verified the partition exists (e.g. when
    /// reading a stored `model_partition` value from `derived_outbox` or
    /// `rebuild_cursor`).
    pub fn from_raw(id: String) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for PartitionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ─── PartitionError ───────────────────────────────────────────────────────────

/// Errors raised by [`ensure_partition`].
#[derive(Debug, thiserror::Error)]
pub enum PartitionError {
    /// The manifest failed internal validation.
    #[error("invalid manifest: {0}")]
    InvalidManifest(#[from] ManifestError),

    /// The partition row already exists but its schema fields differ from the
    /// supplied manifest.  The conflicting field is named.
    #[error(
        "partition conflict on field \"{field}\": \
         stored={stored:?}, requested={requested:?}"
    )]
    SchemaMismatch {
        field: &'static str,
        stored: String,
        requested: String,
    },

    /// A SQLite error during the upsert.
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
}

impl From<rusqlite::Error> for PartitionError {
    fn from(e: rusqlite::Error) -> Self {
        PartitionError::Storage(StorageError::Sqlite(e))
    }
}

// ─── Manifest canonical checksum ─────────────────────────────────────────────

/// Compute the SHA-256 of the canonical JSON representation of `manifest`.
///
/// The canonical form is produced by [`serde_json::to_string`] which uses
/// camelCase field names (matching the `#[serde(rename_all = "camelCase")]`
/// on the struct).  The same deterministic serialisation is used for every
/// `ensure_partition` call so the stored `manifest_checksum` is stable.
fn manifest_checksum(manifest: &EmbeddingPartitionManifest) -> String {
    let json = serde_json::to_string(manifest).expect("manifest serialisation is infallible");
    let digest = Sha256::digest(json.as_bytes());
    hex::encode(digest)
}

/// Derive a deterministic `partition_id` from `(model_id, source_revision)`.
///
/// Format: `"{model_id}:{source_revision}"` — stable, human-readable, and
/// unique within the registry constraint `(model_id, source_revision)`.
fn derive_partition_id(manifest: &EmbeddingPartitionManifest) -> String {
    format!("{}:{}", manifest.model_id, manifest.source_revision)
}

// ─── ensure_partition ────────────────────────────────────────────────────────

/// Validate `manifest` and upsert the `embedding_partitions` row.
///
/// ### Behaviour
/// 1. Calls `manifest.validate()` — returns [`PartitionError::InvalidManifest`]
///    on any encoding/version invariant violation.
/// 2. Looks up any existing row for `(model_id, source_revision)`.
/// 3. If a row exists and all schema fields match, returns the existing
///    [`PartitionId`] unchanged (idempotent).
/// 4. If a row exists with DIFFERENT schema fields, returns
///    [`PartitionError::SchemaMismatch`] — callers must resolve the conflict
///    before vectors can be written.
/// 5. If no row exists, inserts it with `status='active'` and the current
///    UTC timestamp as `build_time`.
///
/// The function does NOT lock the write connection for the full call; it
/// executes a single INSERT OR IGNORE + SELECT cycle that is safe under SQLite's
/// serialized-writer model.
pub fn ensure_partition(
    conn: &Connection,
    manifest: &EmbeddingPartitionManifest,
) -> Result<PartitionId, PartitionError> {
    // 1. Validate the manifest contract (encoding / version invariants).
    manifest.validate()?;

    let partition_id = derive_partition_id(manifest);
    let checksum = manifest_checksum(manifest);

    // 2. Check for an existing row.
    let existing: Option<(
        String, // model_id
        String, // model_source_revision
        i64,    // dimension
        String, // dtype
        i64,    // normalized
        i64,    // max_tokens
        String, // pooling
        i64,    // vector_byte_length
    )> = conn
        .query_row(
            "SELECT model_id, model_source_revision, dimension, dtype, \
                    normalized, max_tokens, pooling, vector_byte_length \
             FROM embedding_partitions WHERE partition_id = ?1",
            params![partition_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            },
        )
        .optional()?;

    if let Some((
        stored_model_id,
        stored_revision,
        stored_dim,
        stored_dtype,
        stored_normalized,
        stored_max_tokens,
        stored_pooling,
        stored_vbl,
    )) = existing
    {
        // 3/4: Row exists — verify every schema field matches.
        macro_rules! check_field {
            ($field:literal, $stored:expr, $req:expr) => {
                let stored_str = $stored.to_string();
                let req_str = $req.to_string();
                if stored_str != req_str {
                    return Err(PartitionError::SchemaMismatch {
                        field: $field,
                        stored: stored_str,
                        requested: req_str,
                    });
                }
            };
        }
        check_field!("model_id", stored_model_id, manifest.model_id);
        check_field!(
            "model_source_revision",
            stored_revision,
            manifest.source_revision
        );
        check_field!("dimension", stored_dim, manifest.dimension as i64);
        check_field!("dtype", stored_dtype, manifest.dtype);
        check_field!("normalized", stored_normalized, 1_i64); // l2 normalization → 1
        check_field!("max_tokens", stored_max_tokens, manifest.max_tokens as i64);
        check_field!("pooling", stored_pooling, manifest.pooling);
        check_field!(
            "vector_byte_length",
            stored_vbl,
            manifest.vector_byte_length as i64
        );

        return Ok(PartitionId(partition_id));
    }

    // 5. Insert the new partition row.
    let build_time = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO embedding_partitions (
             partition_id, model_id, model_source_revision,
             onnx_sha256, tokenizer_sha256,
             license_spdx, license_disposition_id,
             ort_version, fastembed_version,
             dimension, dtype, normalized, max_tokens, pooling, vector_byte_length,
             status, build_time, manifest_checksum
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
             ?10, ?11, ?12, ?13, ?14, ?15,
             'active', ?16, ?17
         )",
        params![
            partition_id,
            manifest.model_id,
            manifest.source_revision,
            manifest.onnx_sha256,
            manifest.tokenizer_sha256,
            manifest.license_spdx,
            manifest.license_disposition_id,
            manifest.ort_version,
            manifest.fastembed_version,
            manifest.dimension as i64,
            manifest.dtype,
            1_i64, // l2 normalized = true
            manifest.max_tokens as i64,
            manifest.pooling,
            manifest.vector_byte_length as i64,
            build_time,
            checksum,
        ],
    )?;

    Ok(PartitionId(partition_id))
}

// ─── VectorDecodeError ───────────────────────────────────────────────────────

/// Errors produced by vector blob decoding and validation (task 3.1.3).
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum VectorDecodeError {
    /// The byte slice is not the expected 1536 bytes.
    #[error("wrong byte length: expected {expected}, got {actual}")]
    WrongByteLength { expected: usize, actual: usize },

    /// A decoded f32 element is NaN.
    #[error("NaN value at index {0}")]
    NaNAtIndex(usize),

    /// A decoded f32 element is +Inf or −Inf.
    #[error("Inf value at index {0}")]
    InfAtIndex(usize),

    /// All elements are zero (L2 norm is zero).
    #[error("zero-norm vector (all-zero is not a valid embedding)")]
    ZeroNorm,

    /// The caller supplied a vector whose dimension does not match the manifest.
    #[error("dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: u32, actual: usize },
}

// ─── Vector encoding helpers ─────────────────────────────────────────────────

/// Encode a `f32` slice as little-endian bytes.
///
/// The caller is responsible for validating the slice before calling
/// (via [`validate_raw_vector`] or [`validate_and_decode_vector_blob`]).
pub(crate) fn encode_vector(v: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for f in v {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    bytes
}

/// Validate a `&[f32]` slice for storage invariants.
///
/// Checks performed (in order):
/// 1. Length must equal `EmbeddingPartitionManifest::REQUIRED_DIMENSION` (384).
/// 2. No element may be NaN.
/// 3. No element may be +Inf or −Inf.
/// 4. The L2 norm (computed with f64 accumulation) must be non-zero.
///
/// This is the primary validation gate called by [`SqliteVectorStore::upsert_v2`]
/// before the slice is encoded and written to `mem_vectors_v2`.
pub fn validate_raw_vector(v: &[f32]) -> Result<(), VectorDecodeError> {
    use crate::memory::stores::manifest::REQUIRED_DIMENSION;

    let expected = REQUIRED_DIMENSION as usize;
    if v.len() != expected {
        return Err(VectorDecodeError::DimensionMismatch {
            expected: REQUIRED_DIMENSION,
            actual: v.len(),
        });
    }

    let mut norm_sq = 0.0f64;
    for (i, &x) in v.iter().enumerate() {
        if x.is_nan() {
            return Err(VectorDecodeError::NaNAtIndex(i));
        }
        if x.is_infinite() {
            return Err(VectorDecodeError::InfAtIndex(i));
        }
        norm_sq += (x as f64) * (x as f64);
    }
    if norm_sq == 0.0 {
        return Err(VectorDecodeError::ZeroNorm);
    }
    Ok(())
}

/// Decode and validate a raw byte blob, cross-checking against a manifest.
///
/// Steps:
/// 1. Checks blob length == `manifest.vector_byte_length` (== 1536).
/// 2. Decodes 384 little-endian `f32` values.
/// 3. Rejects NaN at any index.
/// 4. Rejects Inf at any index.
/// 5. Rejects zero-norm (f64 accumulation).
///
/// Returns the decoded `Vec<f32>` on success, or a [`VectorDecodeError`].
pub fn validate_and_decode_vector_blob(
    bytes: &[u8],
    manifest: &super::manifest::EmbeddingPartitionManifest,
) -> Result<Vec<f32>, VectorDecodeError> {
    let expected = manifest.vector_byte_length as usize;
    if bytes.len() != expected {
        return Err(VectorDecodeError::WrongByteLength {
            expected,
            actual: bytes.len(),
        });
    }

    let v = decode_vector_raw(bytes);

    let mut norm_sq = 0.0f64;
    for (i, &x) in v.iter().enumerate() {
        if x.is_nan() {
            return Err(VectorDecodeError::NaNAtIndex(i));
        }
        if x.is_infinite() {
            return Err(VectorDecodeError::InfAtIndex(i));
        }
        norm_sq += (x as f64) * (x as f64);
    }
    if norm_sq == 0.0 {
        return Err(VectorDecodeError::ZeroNorm);
    }
    Ok(v)
}

/// Decode little-endian bytes into a `f32` vec (raw, no validation).
///
/// Internal helper; external code should use [`validate_and_decode_vector_blob`]
/// or work with pre-validated `&[f32]` slices.
#[inline]
pub(crate) fn decode_vector_raw(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Decode little-endian bytes into a validated `f32` vec.
///
/// Returns an error if the blob has the wrong length, contains NaN/Inf, or
/// has a zero L2 norm.  Uses the canonical manifest's `vector_byte_length` (1536).
pub fn decode_vector(bytes: &[u8]) -> Result<Vec<f32>, VectorDecodeError> {
    use crate::memory::stores::manifest::REQUIRED_VECTOR_BYTE_LENGTH;

    let expected = REQUIRED_VECTOR_BYTE_LENGTH as usize;
    if bytes.len() != expected {
        return Err(VectorDecodeError::WrongByteLength {
            expected,
            actual: bytes.len(),
        });
    }
    let v = decode_vector_raw(bytes);
    let mut norm_sq = 0.0f64;
    for (i, &x) in v.iter().enumerate() {
        if x.is_nan() {
            return Err(VectorDecodeError::NaNAtIndex(i));
        }
        if x.is_infinite() {
            return Err(VectorDecodeError::InfAtIndex(i));
        }
        norm_sq += (x as f64) * (x as f64);
    }
    if norm_sq == 0.0 {
        return Err(VectorDecodeError::ZeroNorm);
    }
    Ok(v)
}

/// Cosine similarity with f64 accumulation (F3.1 invariant: no f32 rounding
/// in the dot-product accumulation path).  Returns `0.0` for degenerate inputs
/// (zero-norm or mismatched length).
pub(crate) fn cosine_f64(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for i in 0..a.len() {
        let ai = a[i] as f64;
        let bi = b[i] as f64;
        dot += ai * bi;
        na += ai * ai;
        nb += bi * bi;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        (dot / denom) as f32
    }
}

/// Legacy cosine kept for backward compat with the old `mem_vectors` tests.
pub(crate) fn cosine(a: &[f32], b: &[f32]) -> f32 {
    cosine_f64(a, b)
}

// ─── Sensitivity integer mapping ─────────────────────────────────────────────

/// Map the string `Sensitivity` enum to an integer stored in `mem_vectors_v2`.
///
/// `sensitivity` is a policy column that participates in the composite index
/// `(partition_id, namespace, scope, sensitivity, truth_state)`. Storing it as
/// an integer keeps that index compact and makes range comparisons (`sensitivity
/// <= ?`) fast.
///
/// Mapping:
/// * 0 = public
/// * 1 = internal (Other("internal") / future)
/// * 2 = private
/// * 3 = secret
fn sensitivity_to_int(s: &Sensitivity) -> i64 {
    match s {
        Sensitivity::Public => 0,
        Sensitivity::Private => 2,
        Sensitivity::Secret => 3,
        Sensitivity::Other(_) => 1, // treat unknown as "internal"
    }
}

#[allow(dead_code)] // used by task 3.1.5 rebuild/read path
fn int_to_sensitivity(i: i64) -> Sensitivity {
    match i {
        0 => Sensitivity::Public,
        2 => Sensitivity::Private,
        3 => Sensitivity::Secret,
        _ => Sensitivity::Private, // conservative default for unknown values
    }
}

// ─── SqliteVectorStore ────────────────────────────────────────────────────────

/// Exact, policy-prefiltered vector store backed by `mem_vectors_v2`.
///
/// This is the F3.1 production store.  It requires every write to supply a
/// [`PartitionId`] obtained from [`ensure_partition`], guaranteeing all vectors
/// are tied to a validated `embedding_partitions` row.
pub struct SqliteVectorStore {
    db: Arc<Database>,
}

impl SqliteVectorStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

/// Extended payload for the v2 `mem_vectors_v2` table.
///
/// Unlike the legacy [`VectorPayload`], this carries the partition id, owner,
/// truth state, and revision — all required columns in `mem_vectors_v2`.
#[derive(Clone, Debug)]
pub struct VectorPayloadV2 {
    pub partition_id: PartitionId,
    pub content_hash: String,
    pub namespace: String,
    pub owner_id: String,
    pub scope: Scope,
    pub sensitivity: Sensitivity,
    pub truth_state: String,
    pub revision: i64,
}

// ─── ExactVectorSearchRequest ─────────────────────────────────────────────────

/// Parameters for an exact, policy-prefiltered cosine search over `mem_vectors_v2`.
///
/// All policy dimensions (`namespace`, `scope`, `max_sensitivity`,
/// `allowed_truth_states`) are pushed down into the SQL WHERE clause and
/// resolved against the composite index `ix_mv2_policy` before any BLOB is
/// decoded.  No post-filter Rust passes are required — policy filtering is
/// complete in SQL.
///
/// ## F3.1 invariants
/// * `query` must be exactly 384 f32 elements, L2-normalised, finite, non-zero.
///   Callers are responsible for pre-validating with [`validate_raw_vector`].
/// * Cosine is computed with f64 accumulation (no f32 rounding in the dot
///   product path) via [`cosine_f64`].
/// * Results are stable-sorted: score descending, then `record_id` ascending
///   for ties (deterministic across runs).
#[derive(Clone, Debug)]
pub struct ExactVectorSearchRequest {
    /// Partition to search (obtained from [`ensure_partition`]).
    pub partition_id: PartitionId,
    /// The query vector — must be 384-dim, pre-validated (finite, non-zero norm).
    pub query: Vec<f32>,
    /// Exact namespace filter — only rows with `namespace = ?` are considered.
    pub namespace: String,
    /// Exact scope filter — only rows with `scope = ?` are considered.
    pub scope: String,
    /// Maximum sensitivity level (inclusive): rows with `sensitivity > max_sensitivity`
    /// are excluded.  Maps `Public=0, Internal=1, Private=2, Secret=3`.
    pub max_sensitivity: i64,
    /// Truth states to include, e.g. `["Current", "Stale"]`.
    pub allowed_truth_states: Vec<String>,
    /// Maximum number of hits to return (top-k).
    pub k: usize,
}

impl SqliteVectorStore {
    /// Upsert a vector row into `mem_vectors_v2`.
    ///
    /// `record_id` must be the string representation of the record's UUID.  On
    /// conflict the entire row (vector + all policy/truth/revision columns) is
    /// replaced — `mem_vectors_v2` is a derived projection so overwriting is safe.
    ///
    /// Returns an error if the vector fails F3.1 validation (wrong dimension,
    /// NaN, Inf, or zero norm).  See [`validate_raw_vector`].
    pub async fn upsert_v2(
        &self,
        record_id: Uuid,
        vector: &[f32],
        payload: &VectorPayloadV2,
    ) -> MemoryResult<()> {
        // Validate before encoding — rejects wrong dimension, NaN, Inf, zero norm.
        validate_raw_vector(vector)
            .map_err(|e| StorageError::Serde(format!("vector validation failed: {e}")))?;

        let blob = encode_vector(vector);
        let partition_id = payload.partition_id.0.clone();
        let content_hash = payload.content_hash.clone();
        let namespace = payload.namespace.clone();
        let owner_id = payload.owner_id.clone();
        let scope = payload.scope.as_str().to_string();
        let sensitivity_int = sensitivity_to_int(&payload.sensitivity);
        let truth_state = payload.truth_state.clone();
        let revision = payload.revision;
        let record_id_str = record_id.to_string();

        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "INSERT INTO mem_vectors_v2 (
                     partition_id, record_id, vector, content_hash,
                     namespace, owner_id, scope, sensitivity, truth_state, revision
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                 ON CONFLICT(partition_id, record_id) DO UPDATE SET
                     vector        = excluded.vector,
                     content_hash  = excluded.content_hash,
                     namespace     = excluded.namespace,
                     owner_id      = excluded.owner_id,
                     scope         = excluded.scope,
                     sensitivity   = excluded.sensitivity,
                     truth_state   = excluded.truth_state,
                     revision      = excluded.revision",
                params![
                    partition_id,
                    record_id_str,
                    blob,
                    content_hash,
                    namespace,
                    owner_id,
                    scope,
                    sensitivity_int,
                    truth_state,
                    revision,
                ],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    /// Delete vector rows from `mem_vectors_v2` for a specific partition.
    pub async fn delete_v2(&self, partition_id: &PartitionId, ids: &[Uuid]) -> MemoryResult<()> {
        let tx = self.db.begin()?;
        for id in ids {
            tx.conn()
                .execute(
                    "DELETE FROM mem_vectors_v2 WHERE partition_id = ?1 AND record_id = ?2",
                    params![partition_id.as_str(), id.to_string()],
                )
                .map_err(StorageError::Sqlite)?;
        }
        tx.commit()
    }

    /// All record ids present in `mem_vectors_v2` for a given partition.
    pub async fn all_ids_v2(&self, partition_id: &PartitionId) -> MemoryResult<Vec<Uuid>> {
        let pid = partition_id.0.clone();
        self.db.with_read(move |conn: &Connection| {
            let mut stmt = conn
                .prepare("SELECT record_id FROM mem_vectors_v2 WHERE partition_id = ?1")
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![pid], |r| r.get::<_, String>(0))
                .map_err(StorageError::Sqlite)?;
            let mut out = Vec::new();
            for r in rows {
                let s = r.map_err(StorageError::Sqlite)?;
                out.push(
                    Uuid::parse_str(&s)
                        .map_err(|e| StorageError::Serde(format!("bad uuid: {e}")))?,
                );
            }
            Ok(out)
        })
    }

    /// Policy-prefiltered exact cosine search over `mem_vectors_v2`.
    ///
    /// The SQL query filters by `(partition_id, namespace, scope, sensitivity,
    /// truth_state)` using the composite index `ix_mv2_policy` before any BLOB
    /// is decoded.  All policy filtering is complete in SQL — no Rust
    /// post-processing pass is needed.
    ///
    /// Cosine similarity is computed with f64 accumulation ([`cosine_f64`]).
    /// Results are stable-sorted: score descending, then `record_id` ascending
    /// for ties (deterministic).
    ///
    /// The SQLite read executes on a blocking worker thread via
    /// [`tokio::task::spawn_blocking`] to avoid occupying an async executor
    /// thread for the potentially multi-millisecond scan + decode loop.
    pub async fn search_v2(&self, req: ExactVectorSearchRequest) -> MemoryResult<Vec<VectorHit>> {
        let db = self.db.clone();

        tokio::task::spawn_blocking(move || {
            db.with_read(move |conn: &Connection| {
                let ExactVectorSearchRequest {
                    partition_id,
                    query,
                    namespace,
                    scope,
                    max_sensitivity,
                    allowed_truth_states,
                    k,
                } = req;

                // Build a parameterized query.  Truth state list is small (≤5)
                // so we expand it inline rather than using a temp table.
                //
                // The WHERE clause order matches the composite index key:
                //   ix_mv2_policy(partition_id, namespace, scope, sensitivity, truth_state)
                // so SQLite can resolve policy rows without a full table scan.
                //
                // valid-time gate insertion point:
                //   When `mem_vectors_v2` gains `valid_from` / `valid_until`
                //   columns, add the following after the truth_state IN clause:
                //     AND (valid_from IS NULL OR valid_from <= ?<n>)
                //     AND (valid_until IS NULL OR valid_until > ?<n+1>)
                //   and bind the two instant parameters (e.g. UTC epoch seconds)
                //   after the truth_state list placeholders.
                let placeholders: String = (1..=allowed_truth_states.len())
                    .map(|i| format!("?{}", i + 4))
                    .collect::<Vec<_>>()
                    .join(",");
                let truth_clause = if allowed_truth_states.is_empty() {
                    "'Current'".to_string() // safe default
                } else {
                    placeholders
                };
                let sql = format!(
                    "SELECT record_id, vector \
                     FROM mem_vectors_v2 \
                     WHERE partition_id = ?1 \
                       AND namespace     = ?2 \
                       AND scope         = ?3 \
                       AND sensitivity   <= ?4 \
                       AND truth_state   IN ({truth_clause})",
                );

                let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;

                // Bind params: ?1=partition_id, ?2=namespace, ?3=scope,
                // ?4=max_sensitivity, ?5…=truth_states.
                let mut bound_params: Vec<Box<dyn rusqlite::ToSql>> = vec![
                    Box::new(partition_id.0.clone()),
                    Box::new(namespace.clone()),
                    Box::new(scope.clone()),
                    Box::new(max_sensitivity),
                ];
                for ts in &allowed_truth_states {
                    bound_params.push(Box::new(ts.clone()));
                }

                let params_ref: Vec<&dyn rusqlite::ToSql> =
                    bound_params.iter().map(|b| b.as_ref()).collect();

                let rows = stmt
                    .query_map(params_ref.as_slice(), |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
                    })
                    .map_err(StorageError::Sqlite)?;

                let mut scored: Vec<VectorHit> = Vec::new();
                for row in rows {
                    let (record_id, blob) = row.map_err(StorageError::Sqlite)?;
                    // Decode without re-validation — the schema CHECK + upsert
                    // validation guarantee the stored blob is a valid 1536-byte,
                    // finite, non-zero-norm vector.
                    let v = decode_vector_raw(&blob);
                    // F3.1 invariant: cosine computed with f64 accumulation
                    // (cosine_f64), no f32 rounding in the dot-product path.
                    let score = cosine_f64(&query, &v);
                    let uuid = Uuid::parse_str(&record_id)
                        .map_err(|e| StorageError::Serde(format!("bad uuid: {e}")))?;
                    scored.push(VectorHit { id: uuid, score });
                }

                // Stable sort: score descending, then record_id ascending for
                // ties — deterministic across identical-score vectors.
                // `.sort_by` is stable in Rust, so equal elements keep their
                // original insertion order within the same score bucket; the
                // `then_with` UUID tiebreak makes the final order deterministic
                // regardless of the storage scan order.
                scored.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| a.id.cmp(&b.id))
                });
                scored.truncate(k);
                Ok(scored)
            })
        })
        .await
        .map_err(|e| StorageError::Serde(format!("spawn_blocking join error: {e}")))?
    }
}

// ─── Legacy VectorStore impl (kept for backward compatibility) ────────────────
// The old `VectorStore` trait methods target the legacy `mem_vectors` table from
// migration 0002.  They are retained unchanged until the F3.1 write-path cutover
// removes all callers.

#[async_trait]
impl VectorStore for SqliteVectorStore {
    async fn create_partition(&self, _model: &ModelVersion, _dim: usize) -> MemoryResult<()> {
        // Partitioning is by the `model_version` column; the table already
        // exists (migration 0002). No-op for the brute-force legacy backend.
        Ok(())
    }

    async fn upsert(
        &self,
        model: &ModelVersion,
        id: Uuid,
        vector: &[f32],
        payload: &VectorPayload,
    ) -> MemoryResult<()> {
        let blob = encode_vector(vector);
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "INSERT INTO mem_vectors(model_version, id, vector, namespace, scope, \
                 sensitivity, memory_type, content_hash, created_at) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) \
                 ON CONFLICT(model_version, id) DO UPDATE SET vector=excluded.vector, \
                 namespace=excluded.namespace, scope=excluded.scope, \
                 sensitivity=excluded.sensitivity, memory_type=excluded.memory_type, \
                 content_hash=excluded.content_hash",
                params![
                    model.as_str(),
                    id.to_string(),
                    blob,
                    payload.namespace,
                    payload.scope.as_str(),
                    payload.sensitivity.as_str(),
                    payload.memory_type.as_str(),
                    payload.content_hash,
                    payload.created_at.to_rfc3339(),
                ],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    async fn search(
        &self,
        model: &ModelVersion,
        query: &[f32],
        k: usize,
        filter: &ScopeFilter,
    ) -> MemoryResult<Vec<VectorHit>> {
        let model = model.clone();
        let query = query.to_vec();
        let filter = filter.clone();
        self.db.with_read(move |conn: &Connection| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, vector, namespace, scope, sensitivity FROM mem_vectors \
                     WHERE model_version = ?1",
                )
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![model.as_str()], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })
                .map_err(StorageError::Sqlite)?;

            let mut scored: Vec<VectorHit> = Vec::new();
            for row in rows {
                let (id, blob, ns, scope, sens) = row.map_err(StorageError::Sqlite)?;
                let scope: Scope = scope.parse().unwrap();
                let sens: Sensitivity = sens.parse().unwrap();
                if !filter.allows(&ns, &scope, &sens) {
                    continue;
                }
                let v = decode_vector_raw(&blob);
                let score = cosine(&query, &v);
                let uuid = Uuid::parse_str(&id)
                    .map_err(|e| StorageError::Serde(format!("bad uuid: {e}")))?;
                scored.push(VectorHit { id: uuid, score });
            }
            scored.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            scored.truncate(k);
            Ok(scored)
        })
    }

    async fn delete(&self, model: &ModelVersion, ids: &[Uuid]) -> MemoryResult<()> {
        let tx = self.db.begin()?;
        for id in ids {
            tx.conn()
                .execute(
                    "DELETE FROM mem_vectors WHERE model_version = ?1 AND id = ?2",
                    params![model.as_str(), id.to_string()],
                )
                .map_err(StorageError::Sqlite)?;
        }
        tx.commit()
    }

    async fn all_ids(&self, model: &ModelVersion) -> MemoryResult<Vec<Uuid>> {
        let model = model.clone();
        self.db.with_read(move |conn: &Connection| {
            let mut stmt = conn
                .prepare("SELECT id FROM mem_vectors WHERE model_version = ?1")
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![model.as_str()], |r| r.get::<_, String>(0))
                .map_err(StorageError::Sqlite)?;
            let mut out = Vec::new();
            for r in rows {
                let s = r.map_err(StorageError::Sqlite)?;
                out.push(
                    Uuid::parse_str(&s)
                        .map_err(|e| StorageError::Serde(format!("bad uuid: {e}")))?,
                );
            }
            Ok(out)
        })
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::Database;
    use crate::memory::stores::manifest::EmbeddingPartitionManifest;

    fn canonical() -> EmbeddingPartitionManifest {
        EmbeddingPartitionManifest::canonical()
    }

    fn open_db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().unwrap())
    }

    // ── ensure_partition: happy path ─────────────────────────────────────────

    /// A valid canonical manifest produces a PartitionId without error.
    #[test]
    fn ensure_partition_canonical_succeeds() {
        let db = open_db();
        let conn = db.write();
        let pid = ensure_partition(&conn, &canonical()).expect("canonical manifest must succeed");
        assert!(!pid.as_str().is_empty());
        assert!(pid.as_str().contains("all-MiniLM-L6-v2"));
    }

    /// Calling ensure_partition twice with the same manifest is idempotent.
    #[test]
    fn ensure_partition_is_idempotent() {
        let db = open_db();
        let conn = db.write();
        let m = canonical();
        let pid1 = ensure_partition(&conn, &m).unwrap();
        let pid2 = ensure_partition(&conn, &m).unwrap();
        assert_eq!(pid1, pid2, "repeated call must return the same PartitionId");
    }

    /// The partition row is actually stored in embedding_partitions.
    #[test]
    fn ensure_partition_row_is_persisted() {
        let db = open_db();
        let conn = db.write();
        let m = canonical();
        ensure_partition(&conn, &m).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM embedding_partitions WHERE model_id = ?1",
                params![m.model_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "exactly one partition row must be stored");
    }

    /// Stored dimension must equal 384 (enforced by both schema CHECK and test).
    #[test]
    fn ensure_partition_dimension_stored_as_384() {
        let db = open_db();
        let conn = db.write();
        ensure_partition(&conn, &canonical()).unwrap();

        let dim: i64 = conn
            .query_row(
                "SELECT dimension FROM embedding_partitions WHERE model_id = ?1",
                params!["all-MiniLM-L6-v2"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dim, 384);
    }

    /// Stored vector_byte_length must equal 1536.
    #[test]
    fn ensure_partition_vector_byte_length_1536() {
        let db = open_db();
        let conn = db.write();
        ensure_partition(&conn, &canonical()).unwrap();

        let vbl: i64 = conn
            .query_row(
                "SELECT vector_byte_length FROM embedding_partitions WHERE model_id = ?1",
                params!["all-MiniLM-L6-v2"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(vbl, 1536);
    }

    // ── ensure_partition: manifest validation rejections ─────────────────────

    /// Wrong dimension is rejected at the manifest validation layer.
    #[test]
    fn ensure_partition_rejects_wrong_dimension() {
        let db = open_db();
        let conn = db.write();
        let mut m = canonical();
        m.dimension = 768;
        m.vector_byte_length = 768 * 4;
        let err = ensure_partition(&conn, &m).expect_err("wrong dimension must be rejected");
        assert!(
            matches!(err, PartitionError::InvalidManifest(_)),
            "expected InvalidManifest, got: {err:?}"
        );
    }

    /// Wrong dtype is rejected.
    #[test]
    fn ensure_partition_rejects_wrong_dtype() {
        let db = open_db();
        let conn = db.write();
        let mut m = canonical();
        m.dtype = "f16le".to_string();
        let err = ensure_partition(&conn, &m).expect_err("wrong dtype must be rejected");
        assert!(matches!(err, PartitionError::InvalidManifest(_)));
    }

    /// Wrong pooling is rejected.
    #[test]
    fn ensure_partition_rejects_wrong_pooling() {
        let db = open_db();
        let conn = db.write();
        let mut m = canonical();
        m.pooling = "cls".to_string();
        let err = ensure_partition(&conn, &m).expect_err("wrong pooling must be rejected");
        assert!(matches!(err, PartitionError::InvalidManifest(_)));
    }

    /// Wrong normalization is rejected.
    #[test]
    fn ensure_partition_rejects_wrong_normalization() {
        let db = open_db();
        let conn = db.write();
        let mut m = canonical();
        m.normalization = "none".to_string();
        let err = ensure_partition(&conn, &m).expect_err("wrong normalization must be rejected");
        assert!(matches!(err, PartitionError::InvalidManifest(_)));
    }

    /// Wrong model ID is rejected (the old "minilm_v1" legacy label).
    #[test]
    fn ensure_partition_rejects_legacy_model_id() {
        let db = open_db();
        let conn = db.write();
        let mut m = canonical();
        m.model_id = "minilm_v1".to_string();
        let err = ensure_partition(&conn, &m).expect_err("legacy model ID must be rejected");
        assert!(matches!(err, PartitionError::InvalidManifest(_)));
    }

    /// An invalid source revision (too short) is rejected.
    #[test]
    fn ensure_partition_rejects_invalid_revision() {
        let db = open_db();
        let conn = db.write();
        let mut m = canonical();
        m.source_revision = "abc123".to_string(); // not 40 chars
        let err = ensure_partition(&conn, &m).expect_err("invalid revision must be rejected");
        assert!(matches!(err, PartitionError::InvalidManifest(_)));
    }

    // ── ensure_partition: schema mismatch ────────────────────────────────────

    /// If a row already exists with a different schema field, SchemaMismatch is returned.
    ///
    /// This test bypasses the Rust ensure_partition path by inserting a row
    /// directly via SQL with `PRAGMA ignore_check_constraints = ON` to simulate
    /// a DB that was manipulated outside the normal write path.  If the PRAGMA
    /// is not supported (older SQLite), we skip the test gracefully.
    #[test]
    fn ensure_partition_detects_schema_mismatch_via_direct_insert() {
        let db = open_db();
        let conn = db.write();
        let m = canonical();
        let partition_id = derive_partition_id(&m);
        let checksum = manifest_checksum(&m);
        let build_time = "2025-01-01T00:00:00Z";

        // Attempt to disable CHECK constraints (supported since SQLite 3.41.0).
        // If not supported the PRAGMA is silently ignored.
        let _ = conn.execute_batch("PRAGMA ignore_check_constraints = ON;");

        let insert_result = conn.execute(
            "INSERT OR IGNORE INTO embedding_partitions (
                 partition_id, model_id, model_source_revision,
                 onnx_sha256, tokenizer_sha256, license_spdx, license_disposition_id,
                 ort_version, fastembed_version,
                 dimension, dtype, normalized, max_tokens, pooling, vector_byte_length,
                 status, build_time, manifest_checksum
             ) VALUES (?1, ?2, ?3, 'x', 'x', 'Apache-2.0', 'PENDING_REVIEW',
                       '2.0.0-rc.12', '5', 384, 'f32le', 1, 256, 'cls', 1536,
                       'active', ?4, ?5)",
            params![
                partition_id,
                m.model_id,
                m.source_revision,
                build_time,
                checksum
            ],
        );

        // Re-enable CHECK constraints.
        let _ = conn.execute_batch("PRAGMA ignore_check_constraints = OFF;");

        match insert_result {
            Ok(1) => {
                // Row was inserted with pooling='cls'. ensure_partition must detect mismatch.
                let err = ensure_partition(&conn, &m)
                    .expect_err("pooling mismatch must return SchemaMismatch");
                assert!(
                    matches!(
                        err,
                        PartitionError::SchemaMismatch {
                            field: "pooling",
                            ..
                        }
                    ),
                    "expected SchemaMismatch on pooling, got: {err:?}"
                );
            }
            _ => {
                // Either PRAGMA not supported or CHECK constraint fired anyway.
                // Verify that the normal path (no row exists) works correctly.
                let pid = ensure_partition(&conn, &m)
                    .expect("canonical manifest must succeed when no corrupt row exists");
                assert_eq!(pid.as_str(), partition_id);
            }
        }
    }

    // ── embedding_partitions schema constraints ───────────────────────────────

    /// The SQLite CHECK constraint on `dimension` must reject non-384 values.
    #[test]
    fn schema_rejects_wrong_dimension_in_table() {
        let db = open_db();
        let conn = db.write();
        let err = conn.execute(
            "INSERT INTO embedding_partitions (
                 partition_id, model_id, model_source_revision,
                 onnx_sha256, tokenizer_sha256, license_spdx, license_disposition_id,
                 ort_version, fastembed_version,
                 dimension, dtype, normalized, max_tokens, pooling, vector_byte_length,
                 status, build_time, manifest_checksum
             ) VALUES ('test', 'x', 'a', 'b', 'c', 'd', 'e', 'f', 'g',
                       768, 'f32le', 1, 256, 'mean', 1536,
                       'active', '2025-01-01T00:00:00Z', 'hash')",
            [],
        );
        assert!(err.is_err(), "dimension != 384 must be rejected by CHECK");
    }

    /// The SQLite CHECK constraint on `dtype` must reject non-'f32le' values.
    #[test]
    fn schema_rejects_wrong_dtype_in_table() {
        let db = open_db();
        let conn = db.write();
        let err = conn.execute(
            "INSERT INTO embedding_partitions (
                 partition_id, model_id, model_source_revision,
                 onnx_sha256, tokenizer_sha256, license_spdx, license_disposition_id,
                 ort_version, fastembed_version,
                 dimension, dtype, normalized, max_tokens, pooling, vector_byte_length,
                 status, build_time, manifest_checksum
             ) VALUES ('test2', 'x', 'a', 'b', 'c', 'd', 'e', 'f', 'g',
                       384, 'f16le', 1, 256, 'mean', 1536,
                       'active', '2025-01-01T00:00:00Z', 'hash')",
            [],
        );
        assert!(err.is_err(), "dtype != 'f32le' must be rejected by CHECK");
    }

    /// The SQLite CHECK constraint on `status` must reject invalid values.
    #[test]
    fn schema_rejects_invalid_status() {
        let db = open_db();
        let conn = db.write();
        let err = conn.execute(
            "INSERT INTO embedding_partitions (
                 partition_id, model_id, model_source_revision,
                 onnx_sha256, tokenizer_sha256, license_spdx, license_disposition_id,
                 ort_version, fastembed_version,
                 dimension, dtype, normalized, max_tokens, pooling, vector_byte_length,
                 status, build_time, manifest_checksum
             ) VALUES ('test3', 'x', 'a', 'b', 'c', 'd', 'e', 'f', 'g',
                       384, 'f32le', 1, 256, 'mean', 1536,
                       'unknown', '2025-01-01T00:00:00Z', 'hash')",
            [],
        );
        assert!(err.is_err(), "invalid status must be rejected by CHECK");
    }

    // ── mem_vectors_v2 schema constraints ────────────────────────────────────

    fn insert_valid_partition(conn: &Connection) -> String {
        let m = EmbeddingPartitionManifest::canonical();
        let pid = ensure_partition(conn, &m).unwrap();
        pid.0
    }

    /// A vector with the correct 1536-byte length is accepted.
    #[test]
    fn mem_vectors_v2_accepts_correct_vector_length() {
        let db = open_db();
        let conn = db.write();
        let pid = insert_valid_partition(&conn);
        let good_vec = vec![0u8; 1536];
        let res = conn.execute(
            "INSERT INTO mem_vectors_v2 (partition_id, record_id, vector, content_hash,
                 namespace, owner_id, scope, sensitivity, truth_state, revision)
             VALUES (?1, ?2, ?3, 'h', 'ns', 'o', 'private', 2, 'Current', 0)",
            params![pid, "r1", good_vec],
        );
        assert!(res.is_ok(), "correct vector length must be accepted");
    }

    /// A vector with the wrong length is rejected by the CHECK constraint.
    #[test]
    fn mem_vectors_v2_rejects_wrong_vector_length() {
        let db = open_db();
        let conn = db.write();
        let pid = insert_valid_partition(&conn);
        let bad_vec = vec![0u8; 768]; // 768 instead of 1536
        let res = conn.execute(
            "INSERT INTO mem_vectors_v2 (partition_id, record_id, vector, content_hash,
                 namespace, owner_id, scope, sensitivity, truth_state, revision)
             VALUES (?1, ?2, ?3, 'h', 'ns', 'o', 'private', 2, 'Current', 0)",
            params![pid, "r2", bad_vec],
        );
        assert!(
            res.is_err(),
            "wrong vector length must be rejected by CHECK"
        );
    }

    /// Sensitivity > 3 is rejected.
    #[test]
    fn mem_vectors_v2_rejects_invalid_sensitivity() {
        let db = open_db();
        let conn = db.write();
        let pid = insert_valid_partition(&conn);
        let good_vec = vec![0u8; 1536];
        let res = conn.execute(
            "INSERT INTO mem_vectors_v2 (partition_id, record_id, vector, content_hash,
                 namespace, owner_id, scope, sensitivity, truth_state, revision)
             VALUES (?1, ?2, ?3, 'h', 'ns', 'o', 'private', 4, 'Current', 0)",
            params![pid, "r3", good_vec],
        );
        assert!(res.is_err(), "sensitivity > 3 must be rejected by CHECK");
    }

    /// Negative sensitivity is rejected.
    #[test]
    fn mem_vectors_v2_rejects_negative_sensitivity() {
        let db = open_db();
        let conn = db.write();
        let pid = insert_valid_partition(&conn);
        let good_vec = vec![0u8; 1536];
        let res = conn.execute(
            "INSERT INTO mem_vectors_v2 (partition_id, record_id, vector, content_hash,
                 namespace, owner_id, scope, sensitivity, truth_state, revision)
             VALUES (?1, ?2, ?3, 'h', 'ns', 'o', 'private', -1, 'Current', 0)",
            params![pid, "r4", good_vec],
        );
        assert!(
            res.is_err(),
            "negative sensitivity must be rejected by CHECK"
        );
    }

    /// Negative revision is rejected.
    #[test]
    fn mem_vectors_v2_rejects_negative_revision() {
        let db = open_db();
        let conn = db.write();
        let pid = insert_valid_partition(&conn);
        let good_vec = vec![0u8; 1536];
        let res = conn.execute(
            "INSERT INTO mem_vectors_v2 (partition_id, record_id, vector, content_hash,
                 namespace, owner_id, scope, sensitivity, truth_state, revision)
             VALUES (?1, ?2, ?3, 'h', 'ns', 'o', 'private', 2, 'Current', -1)",
            params![pid, "r5", good_vec],
        );
        assert!(res.is_err(), "negative revision must be rejected by CHECK");
    }

    /// An orphan vector referencing a non-existent partition_id is rejected.
    #[test]
    fn mem_vectors_v2_rejects_orphan_partition_ref() {
        let db = open_db();
        let conn = db.write();
        let good_vec = vec![0u8; 1536];
        let res = conn.execute(
            "INSERT INTO mem_vectors_v2 (partition_id, record_id, vector, content_hash,
                 namespace, owner_id, scope, sensitivity, truth_state, revision)
             VALUES ('nonexistent', 'r6', ?1, 'h', 'ns', 'o', 'private', 2, 'Current', 0)",
            params![good_vec],
        );
        assert!(
            res.is_err(),
            "orphan partition reference must be rejected by FK"
        );
    }

    // ── v2 store methods ─────────────────────────────────────────────────────

    fn make_payload_v2(pid: &PartitionId) -> VectorPayloadV2 {
        VectorPayloadV2 {
            partition_id: pid.clone(),
            content_hash: "testhash".to_string(),
            namespace: "core".to_string(),
            owner_id: "owner1".to_string(),
            scope: Scope::Global,
            sensitivity: Sensitivity::Private,
            truth_state: "Current".to_string(),
            revision: 0,
        }
    }

    /// upsert_v2 / all_ids_v2 round-trip.
    #[tokio::test]
    async fn upsert_and_all_ids_v2() {
        let db = open_db();
        let pid = {
            let conn = db.write();
            ensure_partition(&conn, &canonical()).unwrap()
        };
        let vs = SqliteVectorStore::new(db.clone());
        let id_a = Uuid::now_v7();
        let id_b = Uuid::now_v7();
        let vec_a = vec![1.0f32; 384];
        // Use a valid non-zero vector for vec_b (unit vector along dim 1)
        let mut vec_b = vec![0.0f32; 384];
        vec_b[1] = 1.0;
        vs.upsert_v2(id_a, &vec_a, &make_payload_v2(&pid))
            .await
            .unwrap();
        vs.upsert_v2(id_b, &vec_b, &make_payload_v2(&pid))
            .await
            .unwrap();

        let mut ids = vs.all_ids_v2(&pid).await.unwrap();
        ids.sort();
        let mut expected = vec![id_a, id_b];
        expected.sort();
        assert_eq!(ids, expected);
    }

    /// delete_v2 removes the row.
    #[tokio::test]
    async fn delete_v2_removes_row() {
        let db = open_db();
        let pid = {
            let conn = db.write();
            ensure_partition(&conn, &canonical()).unwrap()
        };
        let vs = SqliteVectorStore::new(db.clone());
        let id = Uuid::now_v7();
        vs.upsert_v2(id, &vec![1.0f32; 384], &make_payload_v2(&pid))
            .await
            .unwrap();
        assert_eq!(vs.all_ids_v2(&pid).await.unwrap().len(), 1);
        vs.delete_v2(&pid, &[id]).await.unwrap();
        assert!(vs.all_ids_v2(&pid).await.unwrap().is_empty());
    }

    // ── Vector validation (task 3.1.3) ───────────────────────────────────────

    /// Helper: build a valid normalised 384-dim f32 vector.
    fn valid_384_vector() -> Vec<f32> {
        // Unit vector along dimension 0: norm = 1.0, all finite.
        let mut v = vec![0.0f32; 384];
        v[0] = 1.0;
        v
    }

    /// Helper: encode a f32 slice as little-endian bytes (no validation).
    fn raw_encode(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    // ── validate_raw_vector ──────────────────────────────────────────────────

    /// A correct 384-element normalised vector passes.
    #[test]
    fn validate_raw_vector_valid_passes() {
        let v = valid_384_vector();
        assert!(validate_raw_vector(&v).is_ok());
    }

    /// Wrong dimension (768) is rejected with DimensionMismatch.
    #[test]
    fn validate_raw_vector_rejects_768_dim() {
        let v = vec![0.5f32; 768];
        let err = validate_raw_vector(&v).expect_err("768-dim must be rejected");
        assert!(
            matches!(
                err,
                VectorDecodeError::DimensionMismatch {
                    expected: 384,
                    actual: 768
                }
            ),
            "got: {err:?}"
        );
    }

    /// Empty slice is rejected with DimensionMismatch.
    #[test]
    fn validate_raw_vector_rejects_empty() {
        let v: Vec<f32> = vec![];
        let err = validate_raw_vector(&v).expect_err("empty vector must be rejected");
        assert!(matches!(
            err,
            VectorDecodeError::DimensionMismatch {
                expected: 384,
                actual: 0
            }
        ));
    }

    /// NaN at index 0 is rejected.
    #[test]
    fn validate_raw_vector_rejects_nan_at_index_0() {
        let mut v = valid_384_vector();
        v[0] = f32::NAN;
        let err = validate_raw_vector(&v).expect_err("NaN must be rejected");
        assert!(
            matches!(err, VectorDecodeError::NaNAtIndex(0)),
            "got: {err:?}"
        );
    }

    /// NaN at a non-zero index is rejected with the correct index.
    #[test]
    fn validate_raw_vector_rejects_nan_at_index_42() {
        let mut v = valid_384_vector();
        v[42] = f32::NAN;
        let err = validate_raw_vector(&v).expect_err("NaN must be rejected");
        assert!(
            matches!(err, VectorDecodeError::NaNAtIndex(42)),
            "got: {err:?}"
        );
    }

    /// +Infinity is rejected.
    #[test]
    fn validate_raw_vector_rejects_pos_inf() {
        let mut v = valid_384_vector();
        v[10] = f32::INFINITY;
        let err = validate_raw_vector(&v).expect_err("+Inf must be rejected");
        assert!(
            matches!(err, VectorDecodeError::InfAtIndex(10)),
            "got: {err:?}"
        );
    }

    /// -Infinity is rejected.
    #[test]
    fn validate_raw_vector_rejects_neg_inf() {
        let mut v = valid_384_vector();
        v[200] = f32::NEG_INFINITY;
        let err = validate_raw_vector(&v).expect_err("-Inf must be rejected");
        assert!(
            matches!(err, VectorDecodeError::InfAtIndex(200)),
            "got: {err:?}"
        );
    }

    /// All-zero vector (zero norm) is rejected.
    #[test]
    fn validate_raw_vector_rejects_all_zero() {
        let v = vec![0.0f32; 384];
        let err = validate_raw_vector(&v).expect_err("zero-norm must be rejected");
        assert!(matches!(err, VectorDecodeError::ZeroNorm), "got: {err:?}");
    }

    /// A near-zero-but-not-zero vector (single epsilon element) passes.
    #[test]
    fn validate_raw_vector_accepts_near_zero_nonzero() {
        let mut v = vec![0.0f32; 384];
        v[0] = f32::EPSILON;
        // norm_sq = (f32::EPSILON as f64)^2 > 0.0 — should pass
        validate_raw_vector(&v).expect("epsilon-norm vector must be accepted");
    }

    /// A uniform 384-dim vector (e.g., all 1/sqrt(384)) passes.
    #[test]
    fn validate_raw_vector_accepts_uniform_normalised() {
        let inv_norm = (384.0f32).sqrt().recip();
        let v = vec![inv_norm; 384];
        validate_raw_vector(&v).expect("normalised uniform vector must be accepted");
    }

    // ── validate_and_decode_vector_blob ─────────────────────────────────────

    /// A valid 1536-byte blob decodes to the original f32 slice.
    #[test]
    fn validate_blob_round_trips_correctly() {
        let m = canonical();
        let original = valid_384_vector();
        let blob = raw_encode(&original);
        let decoded = validate_and_decode_vector_blob(&blob, &m).expect("valid blob must decode");
        assert_eq!(decoded.len(), 384);
        // Values must match exactly (round-trip through LE bytes).
        for (i, (&orig, &dec)) in original.iter().zip(decoded.iter()).enumerate() {
            assert_eq!(orig, dec, "mismatch at index {i}");
        }
    }

    /// Blob of 768 bytes (half-length) is rejected.
    #[test]
    fn validate_blob_rejects_768_bytes() {
        let m = canonical();
        let blob = vec![0u8; 768];
        let err =
            validate_and_decode_vector_blob(&blob, &m).expect_err("768-byte blob must be rejected");
        assert!(
            matches!(
                err,
                VectorDecodeError::WrongByteLength {
                    expected: 1536,
                    actual: 768
                }
            ),
            "got: {err:?}"
        );
    }

    /// Blob of 1535 bytes is rejected.
    #[test]
    fn validate_blob_rejects_1535_bytes() {
        let m = canonical();
        let blob = vec![0u8; 1535];
        let err = validate_and_decode_vector_blob(&blob, &m)
            .expect_err("1535-byte blob must be rejected");
        assert!(
            matches!(
                err,
                VectorDecodeError::WrongByteLength {
                    expected: 1536,
                    actual: 1535
                }
            ),
            "got: {err:?}"
        );
    }

    /// Blob of 1537 bytes is rejected.
    #[test]
    fn validate_blob_rejects_1537_bytes() {
        let m = canonical();
        // Create a 1537-byte blob that would decode as all zeros plus one extra byte
        let mut blob = vec![0u8; 1537];
        // Set first element to 1.0 so norm isn't zero if length check passes
        blob[0..4].copy_from_slice(&1.0f32.to_le_bytes());
        let err = validate_and_decode_vector_blob(&blob, &m)
            .expect_err("1537-byte blob must be rejected");
        assert!(
            matches!(
                err,
                VectorDecodeError::WrongByteLength {
                    expected: 1536,
                    actual: 1537
                }
            ),
            "got: {err:?}"
        );
    }

    /// Blob of 0 bytes is rejected.
    #[test]
    fn validate_blob_rejects_zero_bytes() {
        let m = canonical();
        let blob = vec![];
        let err =
            validate_and_decode_vector_blob(&blob, &m).expect_err("empty blob must be rejected");
        assert!(
            matches!(
                err,
                VectorDecodeError::WrongByteLength {
                    expected: 1536,
                    actual: 0
                }
            ),
            "got: {err:?}"
        );
    }

    /// Blob with NaN at index 0 is rejected.
    #[test]
    fn validate_blob_rejects_nan_at_index_0() {
        let m = canonical();
        let mut v = valid_384_vector();
        v[0] = f32::NAN;
        let blob = raw_encode(&v);
        let err =
            validate_and_decode_vector_blob(&blob, &m).expect_err("NaN blob must be rejected");
        assert!(
            matches!(err, VectorDecodeError::NaNAtIndex(0)),
            "got: {err:?}"
        );
    }

    /// Blob with +Inf is rejected.
    #[test]
    fn validate_blob_rejects_pos_inf() {
        let m = canonical();
        let mut v = valid_384_vector();
        v[5] = f32::INFINITY;
        let blob = raw_encode(&v);
        let err =
            validate_and_decode_vector_blob(&blob, &m).expect_err("+Inf blob must be rejected");
        assert!(
            matches!(err, VectorDecodeError::InfAtIndex(5)),
            "got: {err:?}"
        );
    }

    /// Blob with -Inf is rejected.
    #[test]
    fn validate_blob_rejects_neg_inf() {
        let m = canonical();
        let mut v = valid_384_vector();
        v[383] = f32::NEG_INFINITY;
        let blob = raw_encode(&v);
        let err =
            validate_and_decode_vector_blob(&blob, &m).expect_err("-Inf blob must be rejected");
        assert!(
            matches!(err, VectorDecodeError::InfAtIndex(383)),
            "got: {err:?}"
        );
    }

    /// All-zero 1536-byte blob (zero norm) is rejected.
    #[test]
    fn validate_blob_rejects_all_zero() {
        let m = canonical();
        let blob = vec![0u8; 1536];
        let err = validate_and_decode_vector_blob(&blob, &m)
            .expect_err("zero-norm blob must be rejected");
        assert!(matches!(err, VectorDecodeError::ZeroNorm), "got: {err:?}");
    }

    /// Decoded values match independently computed expected byte values.
    #[test]
    fn validate_blob_decoded_values_match_independent_computation() {
        let m = canonical();
        // Build a known vector: [0.5, 1.5, 2.5, 0.0, 0.0, ...]
        let mut v = vec![0.0f32; 384];
        v[0] = 0.5;
        v[1] = 1.5;
        v[2] = 2.5;
        let blob = raw_encode(&v);
        let decoded =
            validate_and_decode_vector_blob(&blob, &m).expect("known-value blob must decode");
        // Independently verify byte encoding of each value.
        assert_eq!(decoded[0], 0.5f32);
        assert_eq!(decoded[1], 1.5f32);
        assert_eq!(decoded[2], 2.5f32);
        for i in 3..384 {
            assert_eq!(decoded[i], 0.0f32, "element {i} should be 0.0");
        }
        // Also verify the raw bytes match independently computed LE representation.
        let expected_bytes_0: [u8; 4] = 0.5f32.to_le_bytes();
        assert_eq!(&blob[0..4], &expected_bytes_0);
        let expected_bytes_1: [u8; 4] = 1.5f32.to_le_bytes();
        assert_eq!(&blob[4..8], &expected_bytes_1);
        let expected_bytes_2: [u8; 4] = 2.5f32.to_le_bytes();
        assert_eq!(&blob[8..12], &expected_bytes_2);
    }

    // ── decode_vector (public, validated) ────────────────────────────────────

    /// decode_vector on a valid 1536-byte blob succeeds.
    #[test]
    fn decode_vector_valid_blob_succeeds() {
        let v = valid_384_vector();
        let blob = raw_encode(&v);
        let decoded = decode_vector(&blob).expect("valid blob must decode");
        assert_eq!(decoded.len(), 384);
        assert_eq!(decoded[0], 1.0f32);
    }

    /// decode_vector on a 768-byte blob returns WrongByteLength.
    #[test]
    fn decode_vector_wrong_length_returns_error() {
        let blob = vec![0u8; 768];
        let err = decode_vector(&blob).expect_err("wrong length must fail");
        assert!(matches!(
            err,
            VectorDecodeError::WrongByteLength {
                expected: 1536,
                actual: 768
            }
        ));
    }

    // ── upsert_v2 validation integration ─────────────────────────────────────

    /// upsert_v2 rejects a NaN-containing vector.
    #[tokio::test]
    async fn upsert_v2_rejects_nan_vector() {
        let db = open_db();
        let pid = {
            let conn = db.write();
            ensure_partition(&conn, &canonical()).unwrap()
        };
        let vs = SqliteVectorStore::new(db.clone());
        let mut v = valid_384_vector();
        v[7] = f32::NAN;
        let err = vs
            .upsert_v2(Uuid::now_v7(), &v, &make_payload_v2(&pid))
            .await
            .expect_err("NaN vector must be rejected by upsert_v2");
        assert!(
            format!("{err:?}").contains("NaN") || format!("{err}").contains("NaN"),
            "error must mention NaN: {err}"
        );
    }

    /// upsert_v2 rejects an all-zero vector.
    #[tokio::test]
    async fn upsert_v2_rejects_zero_norm_vector() {
        let db = open_db();
        let pid = {
            let conn = db.write();
            ensure_partition(&conn, &canonical()).unwrap()
        };
        let vs = SqliteVectorStore::new(db.clone());
        let v = vec![0.0f32; 384];
        let err = vs
            .upsert_v2(Uuid::now_v7(), &v, &make_payload_v2(&pid))
            .await
            .expect_err("zero-norm vector must be rejected by upsert_v2");
        assert!(
            format!("{err:?}").contains("zero") || format!("{err}").contains("zero"),
            "error must mention zero norm: {err}"
        );
    }

    /// upsert_v2 rejects a vector with wrong dimension.
    #[tokio::test]
    async fn upsert_v2_rejects_wrong_dimension() {
        let db = open_db();
        let pid = {
            let conn = db.write();
            ensure_partition(&conn, &canonical()).unwrap()
        };
        let vs = SqliteVectorStore::new(db.clone());
        let v = vec![1.0f32; 768]; // wrong dimension
        let err = vs
            .upsert_v2(Uuid::now_v7(), &v, &make_payload_v2(&pid))
            .await
            .expect_err("wrong-dim vector must be rejected by upsert_v2");
        assert!(
            format!("{err:?}").contains("mismatch") || format!("{err}").contains("mismatch"),
            "error must mention dimension mismatch: {err}"
        );
    }

    /// upsert_v2 accepts a valid 384-dim finite non-zero vector.
    #[tokio::test]
    async fn upsert_v2_accepts_valid_vector() {
        let db = open_db();
        let pid = {
            let conn = db.write();
            ensure_partition(&conn, &canonical()).unwrap()
        };
        let vs = SqliteVectorStore::new(db.clone());
        let v = valid_384_vector();
        vs.upsert_v2(Uuid::now_v7(), &v, &make_payload_v2(&pid))
            .await
            .expect("valid 384-dim vector must be accepted");
    }

    fn legacy_payload() -> VectorPayload {
        VectorPayload {
            namespace: "core".into(),
            scope: Scope::Global,
            sensitivity: Sensitivity::Private,
            memory_type: crate::memory::types::MemoryType::Semantic,
            content_hash: "h".into(),
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn upsert_search_ranks_by_cosine() {
        let db = open_db();
        let vs = SqliteVectorStore::new(db.clone());
        let model = ModelVersion("minilm_v1".into());
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        vs.upsert(&model, a, &[1.0, 0.0, 0.0], &legacy_payload())
            .await
            .unwrap();
        vs.upsert(&model, b, &[0.0, 1.0, 0.0], &legacy_payload())
            .await
            .unwrap();

        let hits = vs
            .search(&model, &[1.0, 0.0, 0.0], 10, &ScopeFilter::default())
            .await
            .unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, a);
        assert!(hits[0].score > hits[1].score);
    }

    // ── search_v2: task 3.1.4 ────────────────────────────────────────────────

    /// Build a minimal valid [`ExactVectorSearchRequest`] for the canonical partition.
    fn default_search_req(pid: &PartitionId, query: Vec<f32>) -> ExactVectorSearchRequest {
        ExactVectorSearchRequest {
            partition_id: pid.clone(),
            query,
            namespace: "core".to_string(),
            scope: "global".to_string(),
            max_sensitivity: 2, // up to Private
            allowed_truth_states: vec!["Current".to_string()],
            k: 10,
        }
    }

    /// Upsert a vector with specific namespace/scope/sensitivity/truth_state.
    async fn upsert_v2_custom(
        vs: &SqliteVectorStore,
        id: Uuid,
        vec: &[f32],
        pid: &PartitionId,
        namespace: &str,
        scope: &str,
        sensitivity: Sensitivity,
        truth_state: &str,
    ) {
        let payload = VectorPayloadV2 {
            partition_id: pid.clone(),
            content_hash: "h".to_string(),
            namespace: namespace.to_string(),
            owner_id: "owner1".to_string(),
            scope: scope.parse().unwrap(),
            sensitivity,
            truth_state: truth_state.to_string(),
            revision: 0,
        };
        vs.upsert_v2(id, vec, &payload).await.unwrap();
    }

    /// Helper: unit vector along dimension `dim` (length 384).
    fn unit_vec(dim: usize) -> Vec<f32> {
        let mut v = vec![0.0f32; 384];
        v[dim] = 1.0;
        v
    }

    /// search_v2 returns top-k results ranked by cosine similarity.
    #[tokio::test]
    async fn search_v2_ranks_by_cosine() {
        let db = open_db();
        let pid = {
            let conn = db.write();
            ensure_partition(&conn, &canonical()).unwrap()
        };
        let vs = SqliteVectorStore::new(db.clone());
        let id_a = Uuid::now_v7();
        let id_b = Uuid::now_v7();
        // id_a is aligned with dim 0; id_b is orthogonal (dim 1).
        upsert_v2_custom(
            &vs,
            id_a,
            &unit_vec(0),
            &pid,
            "core",
            "global",
            Sensitivity::Public,
            "Current",
        )
        .await;
        upsert_v2_custom(
            &vs,
            id_b,
            &unit_vec(1),
            &pid,
            "core",
            "global",
            Sensitivity::Public,
            "Current",
        )
        .await;

        // Query aligned with dim 0 → id_a should score higher.
        let req = default_search_req(&pid, unit_vec(0));
        let hits = vs.search_v2(req).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, id_a, "best match must be id_a");
        assert!(hits[0].score > hits[1].score, "scores must be descending");
    }

    /// Stable sort tiebreak: two vectors with identical scores (both orthogonal
    /// to the query) must return the lexicographically smaller UUID first.
    #[tokio::test]
    async fn search_v2_stable_sort_tiebreak_by_uuid() {
        let db = open_db();
        let pid = {
            let conn = db.write();
            ensure_partition(&conn, &canonical()).unwrap()
        };
        let vs = SqliteVectorStore::new(db.clone());

        // Use fixed UUIDs so we know their ordering deterministically.
        let id_small = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let id_large = Uuid::parse_str("ffffffff-ffff-ffff-ffff-ffffffffffff").unwrap();

        // Both vectors are orthogonal to the query (dim 0) → cosine = 0.0 for both.
        upsert_v2_custom(
            &vs,
            id_small,
            &unit_vec(1),
            &pid,
            "core",
            "global",
            Sensitivity::Public,
            "Current",
        )
        .await;
        upsert_v2_custom(
            &vs,
            id_large,
            &unit_vec(2),
            &pid,
            "core",
            "global",
            Sensitivity::Public,
            "Current",
        )
        .await;

        let req = default_search_req(&pid, unit_vec(0));
        let hits = vs.search_v2(req).await.unwrap();
        assert_eq!(hits.len(), 2);
        // Both have score 0.0; tiebreak must put id_small (lex smaller) first.
        assert_eq!(
            hits[0].id, id_small,
            "tiebreak: smaller UUID must come first; got {:?}",
            hits[0].id
        );
        assert_eq!(hits[1].id, id_large);
    }

    /// Top-k truncation: only the `k` best results are returned.
    #[tokio::test]
    async fn search_v2_top_k_truncation() {
        let db = open_db();
        let pid = {
            let conn = db.write();
            ensure_partition(&conn, &canonical()).unwrap()
        };
        let vs = SqliteVectorStore::new(db.clone());

        // Insert 5 vectors across dims 0..4.
        for dim in 0..5usize {
            let id = Uuid::now_v7();
            upsert_v2_custom(
                &vs,
                id,
                &unit_vec(dim),
                &pid,
                "core",
                "global",
                Sensitivity::Public,
                "Current",
            )
            .await;
        }

        // Request only top-2.
        let mut req = default_search_req(&pid, unit_vec(0));
        req.k = 2;
        let hits = vs.search_v2(req).await.unwrap();
        assert_eq!(hits.len(), 2, "must return exactly k=2 results");
        // Best hit must be the dim-0 vector (cosine = 1.0).
        assert!(
            (hits[0].score - 1.0).abs() < 1e-5,
            "best score must be ~1.0"
        );
    }

    /// Policy filter: a row with wrong namespace is excluded from results.
    #[tokio::test]
    async fn search_v2_excludes_wrong_namespace() {
        let db = open_db();
        let pid = {
            let conn = db.write();
            ensure_partition(&conn, &canonical()).unwrap()
        };
        let vs = SqliteVectorStore::new(db.clone());

        let id_correct_ns = Uuid::now_v7();
        let id_wrong_ns = Uuid::now_v7();
        // Both perfectly aligned with query (dim 0) — score is identical — but
        // one has a different namespace.
        upsert_v2_custom(
            &vs,
            id_correct_ns,
            &unit_vec(0),
            &pid,
            "core",
            "global",
            Sensitivity::Public,
            "Current",
        )
        .await;
        upsert_v2_custom(
            &vs,
            id_wrong_ns,
            &unit_vec(0),
            &pid,
            "other",
            "global",
            Sensitivity::Public,
            "Current",
        )
        .await;

        let req = default_search_req(&pid, unit_vec(0));
        let hits = vs.search_v2(req).await.unwrap();
        // Only the "core" namespace row should appear.
        assert_eq!(hits.len(), 1, "wrong-namespace row must be excluded");
        assert_eq!(hits[0].id, id_correct_ns);
    }

    /// Policy filter: a row with wrong scope is excluded from results.
    #[tokio::test]
    async fn search_v2_excludes_wrong_scope() {
        let db = open_db();
        let pid = {
            let conn = db.write();
            ensure_partition(&conn, &canonical()).unwrap()
        };
        let vs = SqliteVectorStore::new(db.clone());

        let id_correct_scope = Uuid::now_v7();
        let id_wrong_scope = Uuid::now_v7();
        upsert_v2_custom(
            &vs,
            id_correct_scope,
            &unit_vec(0),
            &pid,
            "core",
            "global",
            Sensitivity::Public,
            "Current",
        )
        .await;
        upsert_v2_custom(
            &vs,
            id_wrong_scope,
            &unit_vec(0),
            &pid,
            "core",
            "session",
            Sensitivity::Public,
            "Current",
        )
        .await;

        // Request scope="global" only.
        let req = default_search_req(&pid, unit_vec(0));
        let hits = vs.search_v2(req).await.unwrap();
        assert_eq!(hits.len(), 1, "wrong-scope row must be excluded");
        assert_eq!(hits[0].id, id_correct_scope);
    }

    /// Policy filter: secret (sensitivity=3) row is excluded when max_sensitivity=2.
    #[tokio::test]
    async fn search_v2_excludes_secret_when_max_sensitivity_is_2() {
        let db = open_db();
        let pid = {
            let conn = db.write();
            ensure_partition(&conn, &canonical()).unwrap()
        };
        let vs = SqliteVectorStore::new(db.clone());

        let id_private = Uuid::now_v7();
        let id_secret = Uuid::now_v7();
        upsert_v2_custom(
            &vs,
            id_private,
            &unit_vec(0),
            &pid,
            "core",
            "global",
            Sensitivity::Private,
            "Current",
        )
        .await;
        upsert_v2_custom(
            &vs,
            id_secret,
            &unit_vec(0),
            &pid,
            "core",
            "global",
            Sensitivity::Secret,
            "Current",
        )
        .await;

        // max_sensitivity = 2 (Private) → secret row (3) must be excluded.
        let req = default_search_req(&pid, unit_vec(0)); // max_sensitivity=2 in default_search_req
        let hits = vs.search_v2(req).await.unwrap();
        assert_eq!(
            hits.len(),
            1,
            "secret row must be excluded with max_sensitivity=2"
        );
        assert_eq!(hits[0].id, id_private);
    }

    /// Policy filter: a row with a disallowed truth_state is excluded.
    #[tokio::test]
    async fn search_v2_excludes_wrong_truth_state() {
        let db = open_db();
        let pid = {
            let conn = db.write();
            ensure_partition(&conn, &canonical()).unwrap()
        };
        let vs = SqliteVectorStore::new(db.clone());

        let id_current = Uuid::now_v7();
        let id_superseded = Uuid::now_v7();
        upsert_v2_custom(
            &vs,
            id_current,
            &unit_vec(0),
            &pid,
            "core",
            "global",
            Sensitivity::Public,
            "Current",
        )
        .await;
        upsert_v2_custom(
            &vs,
            id_superseded,
            &unit_vec(0),
            &pid,
            "core",
            "global",
            Sensitivity::Public,
            "Superseded",
        )
        .await;

        // Only "Current" is in allowed_truth_states.
        let req = default_search_req(&pid, unit_vec(0));
        let hits = vs.search_v2(req).await.unwrap();
        assert_eq!(hits.len(), 1, "Superseded row must be excluded");
        assert_eq!(hits[0].id, id_current);
    }

    /// Empty result when no rows match the policy filter.
    #[tokio::test]
    async fn search_v2_returns_empty_when_no_match() {
        let db = open_db();
        let pid = {
            let conn = db.write();
            ensure_partition(&conn, &canonical()).unwrap()
        };
        let vs = SqliteVectorStore::new(db.clone());

        // Insert a row in a different namespace.
        let id = Uuid::now_v7();
        upsert_v2_custom(
            &vs,
            id,
            &unit_vec(0),
            &pid,
            "other_ns",
            "global",
            Sensitivity::Public,
            "Current",
        )
        .await;

        let req = default_search_req(&pid, unit_vec(0)); // searches namespace="core"
        let hits = vs.search_v2(req).await.unwrap();
        assert!(
            hits.is_empty(),
            "no rows matching namespace must yield empty result"
        );
    }

    #[tokio::test]
    async fn secret_is_filtered_and_delete_works() {
        let db = open_db();
        let vs = SqliteVectorStore::new(db.clone());
        let model = ModelVersion("minilm_v1".into());
        let secret_id = Uuid::now_v7();
        let mut p = legacy_payload();
        p.sensitivity = Sensitivity::Secret;
        vs.upsert(&model, secret_id, &[1.0, 0.0], &p).await.unwrap();

        let hits = vs
            .search(&model, &[1.0, 0.0], 10, &ScopeFilter::default())
            .await
            .unwrap();
        assert!(hits.is_empty());

        assert_eq!(vs.all_ids(&model).await.unwrap().len(), 1);
        vs.delete(&model, &[secret_id]).await.unwrap();
        assert!(vs.all_ids(&model).await.unwrap().is_empty());
    }
}
