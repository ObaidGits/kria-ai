//! Storage ports (traits) + their concrete backends (memory-upgrade design §16).
//!
//! `ports` defines the backend-swap seam; `sqlite` implements the synchronous
//! authority-transaction ports over the shared [`crate::memory::db::Database`].
//! Vector (exact SQLite cosine) and full-text (FTS5) backends are implemented in
//! `sqlite_vectors` and `sqlite_search` respectively.
//!
//! `manifest` contains the pinned `EmbeddingPartitionManifest` for the
//! `all-MiniLM-L6-v2` partition (F3.1 / task 3.1.1).

pub mod fts5_query;
pub mod manifest;
pub mod ports;
pub mod sqlite;
pub mod sqlite_fts_rebuild;
pub mod sqlite_graph;
pub mod sqlite_memory;
pub mod sqlite_search;
pub mod sqlite_search_documents;
pub mod sqlite_vector_rebuild;
pub mod sqlite_vectors;

pub use fts5_query::{
    compile_fts5_query, validate_filter_clause_count, CompiledFts5Query, QueryCompileError,
    MAX_FILTER_CLAUSES, MAX_QUERY_CHARS,
};
pub use manifest::{EmbeddingPartitionManifest, ManifestError};
pub use ports::{
    Embedder, EventStore, GraphStore, LlmClient, RelationalStore, SearchStore, VectorStore,
};
pub use sqlite::SqliteEventStore;
pub use sqlite_fts_rebuild::{
    compute_fts_membership_hash, rebuild_fts_for_kind, rebuild_fts_from_stream,
    reconcile_fts_index, FtsRebuildOutcome, FtsRebuildRecord, ReconciliationReport,
};
pub use sqlite_graph::SqliteGraphStore;
pub use sqlite_memory::SqliteRelationalStore;
pub use sqlite_search::{delete_fts_in_tx, fts5_query, index_fts_in_tx, SqliteSearchStore};
pub use sqlite_search_documents::{
    count_search_documents, delete_search_document, search_documents_fts_query,
    upsert_search_document, Fts5SearchQuery, FtsSearchResult, PolicySummary, SearchDocument,
    SearchDocumentHit, SqliteSearchDocumentStore, TotalSemantics,
};
pub use sqlite_vector_rebuild::{
    compute_membership_hash, enqueue_vector_outbox, rebuild_partition, write_derived_manifest,
    DerivedManifestRow, ProcessBatchResult, RebuildCursor, RebuildOutcome, RebuildRecord,
    RebuildStatus, VectorOutboxEntry, VectorOutboxProcessor,
};
pub use sqlite_vectors::{
    decode_vector, ensure_partition, validate_and_decode_vector_blob, validate_raw_vector,
    PartitionError, PartitionId, SqliteVectorStore, VectorDecodeError, VectorPayloadV2,
};
