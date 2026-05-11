# KRIA Memory System

> **Last Updated:** 2026-05-11

---

## Overview

KRIA uses SQLite-backed storage for persistent memory, facts, and knowledge. All writes flow through `MemoryManager`, ensuring consistent audit trails and access tracking.

---

## Memory Types

| Type | Purpose | Retention |
|------|---------|-----------|
| **Episodic** | Conversation turns | 90 days |
| **Semantic** | Facts and knowledge | Permanent (with decay) |
| **Working** | Current session context | Session lifetime |

---

## Storage Schema

### Facts Table

```sql
CREATE TABLE facts (
    id INTEGER PRIMARY KEY,
    key TEXT UNIQUE,
    value TEXT,
    category TEXT,
    confidence REAL,
    access_count INTEGER,
    last_accessed TEXT,
    created_at TEXT,
    decay_factor REAL
);
```

### Turns Table

```sql
CREATE TABLE turns (
    id INTEGER PRIMARY KEY,
    session_id TEXT,
    role TEXT,
    content TEXT,
    timestamp TEXT,
    metadata TEXT
);
```

---

## Memory Manager

```rust
pub trait MemoryManager: Send + Sync {
    fn store_turn(&self, turn: ConversationTurn) -> Result<TurnId>;
    fn store_fact(&self, fact: FactWrite) -> Result<FactId>;
    fn store_media(&self, media: MediaWrite) -> Result<MediaId>;
    fn store_snippet(&self, snippet: SnippetWrite) -> Result<()>;
    fn store_document_chunks(&self, doc: DocumentIngestWrite) -> Result<DocumentId>;
    fn set_preference(&self, pref: PreferenceWrite) -> Result<()>;
}
```

---

## RAG Integration

- Document chunking via Python sidecar
- Embeddings via `sentence-transformers`
- Vector search via SQLite FTS5 + semantic reranking
- Chunk size: 512 tokens, overlap: 50 tokens

---

## Decay Model

Facts decay based on access frequency:

```rust
fn calculate_relevance(fact: &Fact) -> f64 {
    let age_days = (now - fact.last_accessed).days();
    let decay = fact.decay_factor * age_days;
    fact.confidence * decay / (1.0 + fact.access_count as f64 * 0.1)
}
```

---

## Configuration

```toml
[memory]
max_facts = 10000
decay_factor = 0.95
min_confidence = 0.5
retention_days = 90
```
