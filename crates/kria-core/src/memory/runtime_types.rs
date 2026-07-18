//! Shared runtime memory data types (single source of truth).
//!
//! These plain data records are the stable contract between the runtime memory
//! backend ([`crate::memory::runtime_backend::KriaMemoryRuntime`]), the
//! conversation store, the `MemoryManager`/`MemoryReader` traits, and every
//! consumer (desktop, server, tools, telegram). They intentionally live in one
//! module so there is exactly one definition of each type — no duplication
//! between the conversation store and the runtime trait surface.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single conversation turn (user or assistant), as persisted for replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationTurn {
    pub id: Option<i64>,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub tool_name: Option<String>,
    pub tool_result: Option<String>,
    pub tokens_used: Option<i64>,
    pub timestamp: DateTime<Utc>,
}

/// Typed read request for fetching recent memories from a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryFetchRequest {
    pub session_id: String,
    pub limit: usize,
}

/// Typed preference update payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreferenceRecord {
    pub key: String,
    pub value: String,
}

/// A persisted image or uploaded file associated with a chat session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMediaRecord {
    pub session_id: String,
    /// "generated" | "uploaded"
    pub media_type: String,
    pub file_path: String,
    pub sha256: Option<String>,
    /// The prompt used to generate this image (generated only).
    pub prompt: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub style: Option<String>,
    pub provenance: Option<String>,
}

/// A durable derived fact with decay/access bookkeeping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryFact {
    pub id: Option<i64>,
    pub text: String,
    pub category: String,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,
    pub access_count: i32,
    pub decay_score: f64,
}

/// A chunk of an ingested document for RAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentChunk {
    pub id: Option<i64>,
    pub doc_id: String,
    pub doc_name: String,
    pub doc_type: String,
    pub chunk_index: i32,
    pub content: String,
    pub char_offset: i64,
    pub created_at: DateTime<Utc>,
}
