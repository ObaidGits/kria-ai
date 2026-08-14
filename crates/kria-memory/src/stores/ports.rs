//! Storage-port traits — the backend-swap seam (memory-upgrade design §16, L2/L4).
//!
//! **Hybrid sync/async split (engineering decision):** the three ports that
//! participate in the authority transaction (`EventStore`, `RelationalStore`,
//! `GraphStore`) are **synchronous** — they wrap in-process SQLite behind one
//! serialized write connection, where async would only add a `MutexGuard`
//! held across `.await`. The I/O-bound ports (`VectorStore`, `SearchStore`,
//! `Embedder`, `LlmClient`) are **async** because they do real I/O and may be
//! remote/heavy. Both keep the seam that lets backends swap without touching
//! callers (ADR-002/003/004).

use async_trait::async_trait;
use uuid::Uuid;

use crate::db::AuthorityTx;
use crate::error::MemoryResult;
use crate::ids::Hlc;
use crate::types::{
    AuditRecord, Availability, Entity, Event, IndexTarget, Memory, MemoryState, ModelVersion,
    OutboxEntry, OutboxStatus, ScopeFilter, SearchHit, VectorHit, VectorPayload,
};

/// Immutable append-only event log (L1). Writes join the authority transaction;
/// reads use the WAL read pool.
pub trait EventStore: Send + Sync {
    /// Append an immutable event. Idempotent by event id (Issue 28).
    fn append(&self, tx: &mut AuthorityTx<'_>, event: &Event) -> MemoryResult<()>;
    /// Fetch a single event by id.
    fn get(&self, id: Uuid) -> MemoryResult<Option<Event>>;
    /// Read events with `hlc > from_hlc`, ascending, up to `limit`. Forensic /
    /// consumer pull only — never used to regenerate memory content (L4).
    fn read_range(&self, from_hlc: &Hlc, limit: usize) -> MemoryResult<Vec<Event>>;
    /// Read a durable consumer cursor (defaults to `Hlc::ZERO`).
    fn cursor(&self, consumer: &str) -> MemoryResult<Hlc>;
    /// Advance a durable consumer cursor within the authority transaction.
    fn advance_cursor(
        &self,
        tx: &mut AuthorityTx<'_>,
        consumer: &str,
        hlc: &Hlc,
    ) -> MemoryResult<()>;
    /// Number of events still past a consumer's cursor — the durable
    /// enrichment-backlog depth (R2 telemetry gauge).
    fn pending_count(&self, consumer: &str) -> MemoryResult<u64>;
}

/// Derived memories, goals, preferences, outbox, and audit (L4). Writes join the
/// authority transaction; reads use the WAL read pool.
pub trait RelationalStore: Send + Sync {
    fn upsert_memory(&self, tx: &mut AuthorityTx<'_>, memory: &Memory) -> MemoryResult<()>;
    fn get_memory(&self, id: Uuid) -> MemoryResult<Option<Memory>>;
    fn set_memory_state(
        &self,
        tx: &mut AuthorityTx<'_>,
        id: Uuid,
        state: MemoryState,
    ) -> MemoryResult<()>;
    /// Find an active memory by dedup key `(namespace, memory_type, content_hash)`.
    fn find_by_content_hash(
        &self,
        namespace: &str,
        memory_type: &str,
        content_hash: &str,
    ) -> MemoryResult<Option<Memory>>;

    fn enqueue_outbox(&self, tx: &mut AuthorityTx<'_>, entry: &OutboxEntry) -> MemoryResult<()>;
    /// Pending entries for `target` whose backoff window has elapsed (i.e.
    /// `next_attempt_at IS NULL OR next_attempt_at <= now`), ordered oldest
    /// first, capped at `limit`.  This is the enhanced relay fetch (task 1.8.4).
    fn pending_outbox(&self, target: IndexTarget, limit: usize) -> MemoryResult<Vec<OutboxEntry>>;
    /// Mark an outbox entry's lifecycle state.
    ///
    /// - `status`          — new lifecycle value (`Done`, `Pending`, `DeadLetter`)
    /// - `attempts`        — updated attempt count
    /// - `next_attempt_at` — when `Some`, set the backoff gate (task 1.8.4);
    ///                       when `None`, clear it (entry is immediately eligible)
    /// - `error_code`      — when `Some`, record the failure reason; when `None`,
    ///                       clear any previously recorded code
    fn mark_outbox(
        &self,
        tx: &mut AuthorityTx<'_>,
        id: i64,
        status: OutboxStatus,
        attempts: u32,
        next_attempt_at: Option<chrono::DateTime<chrono::Utc>>,
        error_code: Option<&str>,
    ) -> MemoryResult<()>;

    /// Record a Write Policy decision to the memory-audit log (design §28).
    fn record_audit(&self, tx: &mut AuthorityTx<'_>, record: &AuditRecord) -> MemoryResult<()>;
}

/// Graph entities with cycle-safe, depth-capped traversal
/// (ADR-004). Entity writes join the authority transaction; reads use the pool.
///
/// The legacy `add_relationship` / `relationships_for` methods were deleted in
/// task F2.2.7. Relationship writes now go through the v2 governed path
/// (`RelationshipCommandBus`). Graph traversal over `relationships_v2` is
/// implemented in F3.3.
pub trait GraphStore: Send + Sync {
    fn add_entity(&self, tx: &mut AuthorityTx<'_>, entity: &Entity) -> MemoryResult<()>;
    /// Cycle-safe, visited-set, depth-capped (`max_hops <= 3`) traversal
    /// over `relationships_v2` (entity endpoints only). Returns `(entity_id, distance)` pairs.
    fn neighbors(&self, root: Uuid, max_hops: u8) -> MemoryResult<Vec<(Uuid, u8)>>;
    fn search_entities(&self, query: &str) -> MemoryResult<Vec<Entity>>;
}

/// Vector index. `SqliteVectorStore` is the **durable authority** for vectors —
/// an exact cosine search over `mem_vectors_v2` with f64 accumulation and
/// SQL-prefiltered partitions. The trait keeps backends caller-transparent so a
/// future scale-out backend can be swapped in without touching callers. No
/// ANN/LanceDB/Qdrant/HNSW backend is part of the current release. Async — real I/O.
#[async_trait]
pub trait VectorStore: Send + Sync {
    async fn create_partition(&self, model: &ModelVersion, dim: usize) -> MemoryResult<()>;
    async fn upsert(
        &self,
        model: &ModelVersion,
        id: Uuid,
        vector: &[f32],
        payload: &VectorPayload,
    ) -> MemoryResult<()>;
    async fn search(
        &self,
        model: &ModelVersion,
        query: &[f32],
        k: usize,
        filter: &ScopeFilter,
    ) -> MemoryResult<Vec<VectorHit>>;
    async fn delete(&self, model: &ModelVersion, ids: &[Uuid]) -> MemoryResult<()>;
    /// All ids present in a partition — for reconciliation set-difference (D-16).
    async fn all_ids(&self, model: &ModelVersion) -> MemoryResult<Vec<Uuid>>;
}

/// Full-text index (SQLite FTS5 v1; Tantivy P2). Async for a uniform seam with
/// the future out-of-process Tantivy backend.
#[async_trait]
pub trait SearchStore: Send + Sync {
    async fn index(&self, id: Uuid, content: &str, namespace: &str) -> MemoryResult<()>;
    async fn query(
        &self,
        query: &str,
        k: usize,
        filter: &ScopeFilter,
    ) -> MemoryResult<Vec<SearchHit>>;
    async fn delete(&self, ids: &[Uuid]) -> MemoryResult<()>;
    async fn all_ids(&self) -> MemoryResult<Vec<Uuid>>;
}

/// Text embedder (ONNX). Optional — memory works when it returns `Unavailable`
/// (L8). Async so heavier models don't block the executor.
#[async_trait]
pub trait Embedder: Send + Sync {
    fn model_version(&self) -> ModelVersion;
    fn dim(&self) -> usize;
    async fn embed(&self, texts: &[String]) -> MemoryResult<Vec<Vec<f32>>>;
    async fn health(&self) -> Availability;
}

/// LLM access for ambiguous classification / synthesis. Optional (L8).
#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn classify(&self, prompt: &str) -> MemoryResult<String>;
    async fn health(&self) -> Availability;
}
