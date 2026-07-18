//! Storage ports (traits) + their concrete backends (memory-upgrade design §16).
//!
//! `ports` defines the backend-swap seam; `sqlite` implements the synchronous
//! authority-transaction ports over the shared [`crate::memory::db::Database`].
//! Vector (LanceDB) and full-text (FTS5/Tantivy) backends land in later tasks.

pub mod ann_vectors;
pub mod ports;
pub mod sqlite;
pub mod sqlite_graph;
pub mod sqlite_memory;
pub mod sqlite_search;
pub mod sqlite_vectors;

pub use ann_vectors::AnnVectorStore;
pub use ports::{
    Embedder, EventStore, GraphStore, LlmClient, RelationalStore, SearchStore, VectorStore,
};
pub use sqlite::SqliteEventStore;
pub use sqlite_graph::SqliteGraphStore;
pub use sqlite_memory::SqliteRelationalStore;
pub use sqlite_search::{delete_fts_in_tx, fts5_query, index_fts_in_tx, SqliteSearchStore};
pub use sqlite_vectors::SqliteVectorStore;
