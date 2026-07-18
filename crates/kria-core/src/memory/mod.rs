// ── Cognitive Memory System (memory-upgrade spec) ──────────────────────────
// New foundation modules. Built alongside the legacy system below; the legacy
// modules are deleted at task 19 (hard cutover). See
// `.kiro/specs/memory-upgrade/`.
pub mod api;
pub mod causal;
pub mod cognition;
pub mod cold_start;
pub mod cold_start_scan;
pub mod contract;
pub mod conversation;
pub mod db;
pub mod dreaming;
pub mod embedding;
pub mod entity_resolution;
pub mod error;
pub mod extraction;
pub mod feedback;
pub mod goals;
pub mod governance;
pub mod graph_intel;
pub mod ids;
pub mod integration;
pub mod jobs;
pub mod knowledge_gap;
pub mod library;
pub mod lifecycle;
pub mod maintenance;
pub mod merge;
pub mod modes;
pub mod observability;
pub mod planning;
pub mod reasoning;
pub mod research;
pub mod retrieval_opt;
pub mod retriever;
pub mod runtime_backend;
pub mod runtime_types;
pub mod salience;
pub mod scheduler;
pub mod self_improvement;
pub mod sensitivity;
pub mod stores;
pub mod truth;
pub mod types;
pub mod write_policy;

pub mod active_learning;

// ── Shared infrastructure (embeddings, vector index, LLM extraction) ───────
// These are part of the unified architecture, not legacy storage.
pub mod embeddings;
pub mod manager;
pub mod semantic_parser;

pub use manager::MemoryTurnWrite;
pub use manager::{MemoryManager, MemoryReader, MemoryRuntime};
pub use runtime_backend::KriaMemoryRuntime;
pub use runtime_types::{
    ChatMediaRecord, ConversationTurn, DocumentChunk, MemoryFact, MemoryFetchRequest,
    PreferenceRecord,
};
pub use semantic_parser::{MemoryExtraction, SemanticMemoryParser};
