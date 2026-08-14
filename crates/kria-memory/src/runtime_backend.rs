//! Authority-DB runtime backend (memory-upgrade cutover).
//!
//! `KriaMemoryRuntime` is the production implementation of the desktop
//! `MemoryManager`/`MemoryReader` surface (and the RAG chunk store) over the
//! single authority [`Database`]. It replaces the legacy `MemoryStore` SQLite
//! engine: conversations/sessions/preferences/media are served through the
//! shared [`ConversationStore`], while facts, snippets, and document chunks are
//! served directly against the authority tables added in migration 0005.
//!
//! One authority DB means chat, derived facts, snippets, and RAG chunks all
//! live together (unified backup / encryption / L14), eliminating the legacy
//! chat-vs-memory data split.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use rusqlite::params;

use crate::conversation::ConversationStore;
use crate::db::Database;
use crate::error::StorageError;
use crate::manager::{MemoryManager, MemoryReader, MemoryTurnWrite};
use crate::runtime_types::{
    ChatMediaRecord, ConversationTurn, DocumentChunk, MemoryFact, MemoryFetchRequest,
    PreferenceRecord,
};

/// Runtime memory backend over the authority database.
#[derive(Clone)]
pub struct KriaMemoryRuntime {
    db: Arc<Database>,
    conversation: ConversationStore,
}

impl KriaMemoryRuntime {
    /// Open (creating if needed) an authority DB at `path` and build the backend.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let db = Arc::new(Database::open(path)?);
        Ok(Self::from_db(db))
    }

    /// Build the backend over an already-open authority database. The same
    /// `Arc<Database>` should be shared with any directly-constructed
    /// [`ConversationStore`] so all consumers hit one DB.
    pub fn from_db(db: Arc<Database>) -> Self {
        let conversation = ConversationStore::new(db.clone());
        Self { db, conversation }
    }

    /// The shared authority database handle.
    pub fn database(&self) -> Arc<Database> {
        self.db.clone()
    }

    /// A clone of the conversation store built over the same authority DB.
    pub fn conversation(&self) -> ConversationStore {
        self.conversation.clone()
    }

    // ── RAG chunk reads (not part of the runtime trait) ──────────────

    /// Full-text search over document chunks (RAG keyword floor).
    pub fn search_chunks(&self, query: &str, limit: usize) -> anyhow::Result<Vec<DocumentChunk>> {
        let Some(match_expr) = crate::stores::fts5_query(query) else {
            return Ok(Vec::new());
        };
        let out = self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT c.id, c.doc_id, c.doc_name, c.doc_type, c.chunk_index, c.content, \
                     c.char_offset, c.created_at FROM document_chunks c \
                     JOIN chunks_fts f ON c.id = f.rowid \
                     WHERE chunks_fts MATCH ?1 ORDER BY rank LIMIT ?2",
                )
                .map_err(StorageError::Sqlite)?;
            let chunks = stmt
                .query_map(params![match_expr, limit as i64], row_to_chunk)
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(chunks)
        })?;
        Ok(out)
    }

    /// Fetch a single chunk by id.
    pub fn get_chunk(&self, id: i64) -> anyhow::Result<Option<DocumentChunk>> {
        let out = self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, doc_id, doc_name, doc_type, chunk_index, content, char_offset, \
                     created_at FROM document_chunks WHERE id = ?1",
                )
                .map_err(StorageError::Sqlite)?;
            let mut rows = stmt
                .query_map(params![id], row_to_chunk)
                .map_err(StorageError::Sqlite)?;
            Ok(rows.next().transpose().map_err(StorageError::Sqlite)?)
        })?;
        Ok(out)
    }

    /// All chunks for a document id, in chunk order.
    pub fn get_chunks_by_doc(&self, doc_id: &str) -> anyhow::Result<Vec<DocumentChunk>> {
        let out = self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, doc_id, doc_name, doc_type, chunk_index, content, char_offset, \
                     created_at FROM document_chunks WHERE doc_id = ?1 ORDER BY chunk_index",
                )
                .map_err(StorageError::Sqlite)?;
            let chunks = stmt
                .query_map(params![doc_id], row_to_chunk)
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(chunks)
        })?;
        Ok(out)
    }
}

fn row_to_chunk(row: &rusqlite::Row<'_>) -> rusqlite::Result<DocumentChunk> {
    Ok(DocumentChunk {
        id: Some(row.get(0)?),
        doc_id: row.get(1)?,
        doc_name: row.get(2)?,
        doc_type: row.get(3)?,
        chunk_index: row.get(4)?,
        content: row.get(5)?,
        char_offset: row.get(6)?,
        created_at: parse_dt(row.get::<_, String>(7)?),
    })
}

fn row_to_fact(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryFact> {
    Ok(MemoryFact {
        id: Some(row.get(0)?),
        text: row.get(1)?,
        category: row.get(2)?,
        source: row.get(3)?,
        created_at: parse_dt(row.get::<_, String>(4)?),
        last_accessed: parse_dt(row.get::<_, String>(5)?),
        access_count: row.get(6)?,
        decay_score: row.get(7)?,
    })
}

fn parse_dt(s: String) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

impl MemoryManager for KriaMemoryRuntime {
    fn store_turn(&self, turn: &MemoryTurnWrite) -> anyhow::Result<i64> {
        let mut last_id = 0_i64;

        if !turn.user_prompt.trim().is_empty() {
            last_id = self.conversation.store_turn(&ConversationTurn {
                id: None,
                session_id: turn.session_id.clone(),
                role: "user".to_string(),
                content: turn.user_prompt.clone(),
                tool_name: None,
                tool_result: None,
                tokens_used: None,
                timestamp: turn.timestamp,
            })?;
        }

        let has_assistant_payload = !turn.assistant_response.trim().is_empty()
            || turn
                .tool_name
                .as_deref()
                .is_some_and(|name| !name.trim().is_empty())
            || turn
                .tool_result
                .as_deref()
                .is_some_and(|result| !result.trim().is_empty())
            || turn.tokens_used.is_some();

        if has_assistant_payload {
            last_id = self.conversation.store_turn(&ConversationTurn {
                id: None,
                session_id: turn.session_id.clone(),
                role: "assistant".to_string(),
                content: turn.assistant_response.clone(),
                tool_name: turn.tool_name.clone(),
                tool_result: turn.tool_result.clone(),
                tokens_used: turn.tokens_used.map(|v| v as i64),
                timestamp: turn.timestamp,
            })?;
        }

        if let Some(extraction) = turn.extraction.as_ref() {
            let mut seen = HashSet::new();

            for text in extraction
                .extracted_facts
                .iter()
                .map(|fact| fact.trim())
                .filter(|fact| !fact.is_empty())
            {
                if !seen.insert(text.to_string()) {
                    continue;
                }
                self.store_fact(&MemoryFact {
                    id: None,
                    text: text.to_string(),
                    category: "semantic_fact".to_string(),
                    source: "semantic_parser".to_string(),
                    created_at: turn.timestamp,
                    last_accessed: turn.timestamp,
                    access_count: 0,
                    decay_score: 1.0,
                })?;
            }

            let inferred_context = extraction.inferred_context.trim();
            if !inferred_context.is_empty() && seen.insert(inferred_context.to_string()) {
                self.store_fact(&MemoryFact {
                    id: None,
                    text: inferred_context.to_string(),
                    category: "inferred_context".to_string(),
                    source: "semantic_parser".to_string(),
                    created_at: turn.timestamp,
                    last_accessed: turn.timestamp,
                    access_count: 0,
                    decay_score: 1.0,
                })?;
            }

            for (key, value) in &extraction.user_preferences {
                let key = key.trim();
                let value = value.trim();
                if key.is_empty() || value.is_empty() {
                    continue;
                }
                self.conversation.set_preference(key, value)?;
            }
        }

        Ok(last_id)
    }

    fn delete_session(&self, session_id: &str) -> anyhow::Result<usize> {
        Ok(self.conversation.delete_session(session_id)?)
    }

    fn delete_session_preferences(&self, session_id: &str) -> anyhow::Result<usize> {
        Ok(self.conversation.delete_session_preferences(session_id)?)
    }

    fn store_fact(&self, fact: &MemoryFact) -> anyhow::Result<i64> {
        let tx = self.db.begin()?;
        tx.conn().execute(
            "INSERT INTO memory_facts (text, category, source, created_at, last_accessed, \
             access_count, decay_score) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                fact.text,
                fact.category,
                fact.source,
                fact.created_at.to_rfc3339(),
                fact.last_accessed.to_rfc3339(),
                fact.access_count,
                fact.decay_score,
            ],
        )?;
        let id = tx.conn().last_insert_rowid();
        tx.conn().execute(
            "INSERT INTO facts_fts(rowid, text) VALUES (?1, ?2)",
            params![id, fact.text],
        )?;
        tx.commit()?;
        Ok(id)
    }

    fn update_fact_access(&self, id: i64) -> anyhow::Result<()> {
        let tx = self.db.begin()?;
        tx.conn().execute(
            "UPDATE memory_facts SET access_count = access_count + 1, \
             last_accessed = datetime('now') WHERE id = ?1",
            params![id],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn update_fact_decay(&self, id: i64, new_score: f64) -> anyhow::Result<()> {
        let tx = self.db.begin()?;
        tx.conn().execute(
            "UPDATE memory_facts SET decay_score = ?2 WHERE id = ?1",
            params![id, new_score],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn delete_fact(&self, id: i64) -> anyhow::Result<()> {
        let tx = self.db.begin()?;
        tx.conn()
            .execute("DELETE FROM memory_facts WHERE id = ?1", params![id])?;
        tx.conn()
            .execute("DELETE FROM facts_fts WHERE rowid = ?1", params![id])?;
        tx.commit()?;
        Ok(())
    }

    fn store_media(&self, media: &ChatMediaRecord) -> anyhow::Result<i64> {
        Ok(self.conversation.store_chat_media(media)?)
    }

    fn store_snippet(
        &self,
        name: &str,
        content: &str,
        language: &str,
        tags: &[String],
    ) -> anyhow::Result<()> {
        let tags_json = serde_json::to_string(tags)?;
        let tx = self.db.begin()?;
        tx.conn().execute(
            "INSERT OR REPLACE INTO snippets (name, content, language, tags) \
             VALUES (?1, ?2, ?3, ?4)",
            params![name, content, language, tags_json],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn store_document_chunk(&self, chunk: &DocumentChunk) -> anyhow::Result<i64> {
        let tx = self.db.begin()?;
        tx.conn().execute(
            "INSERT INTO document_chunks (doc_id, doc_name, doc_type, chunk_index, content, \
             char_offset, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                chunk.doc_id,
                chunk.doc_name,
                chunk.doc_type,
                chunk.chunk_index,
                chunk.content,
                chunk.char_offset,
                chunk.created_at.to_rfc3339(),
            ],
        )?;
        let id = tx.conn().last_insert_rowid();
        tx.conn().execute(
            "INSERT INTO chunks_fts(rowid, content) VALUES (?1, ?2)",
            params![id, chunk.content],
        )?;
        tx.commit()?;
        Ok(id)
    }

    fn delete_document_chunks(&self, doc_id: &str) -> anyhow::Result<usize> {
        let tx = self.db.begin()?;
        // Drop matching FTS rows first (explicit-rowid FTS5), then the chunks.
        tx.conn().execute(
            "DELETE FROM chunks_fts WHERE rowid IN \
             (SELECT id FROM document_chunks WHERE doc_id = ?1)",
            params![doc_id],
        )?;
        let n = tx.conn().execute(
            "DELETE FROM document_chunks WHERE doc_id = ?1",
            params![doc_id],
        )?;
        tx.commit()?;
        Ok(n)
    }

    fn set_preference(&self, preference: &PreferenceRecord) -> anyhow::Result<()> {
        Ok(self
            .conversation
            .set_preference(&preference.key, &preference.value)?)
    }
}

impl MemoryReader for KriaMemoryRuntime {
    fn fetch_memories(&self, query: &MemoryFetchRequest) -> anyhow::Result<Vec<ConversationTurn>> {
        self.get_recent_turns(&query.session_id, query.limit)
    }

    fn get_recent_turns(
        &self,
        session_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<ConversationTurn>> {
        Ok(self.conversation.get_recent_turns(session_id, limit)?)
    }

    fn search_conversations(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<ConversationTurn>> {
        Ok(self.conversation.search_conversations(query, limit)?)
    }

    fn search_facts(&self, query: &str, limit: usize) -> anyhow::Result<Vec<MemoryFact>> {
        let Some(match_expr) = crate::stores::fts5_query(query) else {
            return Ok(Vec::new());
        };
        let out = self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT f.id, f.text, f.category, f.source, f.created_at, f.last_accessed, \
                     f.access_count, f.decay_score FROM memory_facts f \
                     JOIN facts_fts fts ON f.id = fts.rowid \
                     WHERE facts_fts MATCH ?1 ORDER BY rank LIMIT ?2",
                )
                .map_err(StorageError::Sqlite)?;
            let facts = stmt
                .query_map(params![match_expr, limit as i64], row_to_fact)
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(facts)
        })?;
        Ok(out)
    }

    fn all_facts_with_decay(&self, min_score: f64) -> anyhow::Result<Vec<MemoryFact>> {
        let out = self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, text, category, source, created_at, last_accessed, access_count, \
                     decay_score FROM memory_facts WHERE decay_score >= ?1 ORDER BY decay_score DESC",
                )
                .map_err(StorageError::Sqlite)?;
            let facts = stmt
                .query_map(params![min_score], row_to_fact)
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(facts)
        })?;
        Ok(out)
    }

    fn get_preference(&self, key: &str) -> anyhow::Result<Option<String>> {
        Ok(self.conversation.get_preference(key)?)
    }

    fn list_sessions(&self) -> anyhow::Result<Vec<(String, i64, String)>> {
        Ok(self.conversation.list_sessions()?)
    }

    fn list_documents(&self) -> anyhow::Result<Vec<(String, String, String, i64)>> {
        let out = self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT doc_id, doc_name, doc_type, COUNT(*) as chunks \
                     FROM document_chunks GROUP BY doc_id ORDER BY doc_name",
                )
                .map_err(StorageError::Sqlite)?;
            let docs = stmt
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(docs)
        })?;
        Ok(out)
    }

    fn get_session_media(&self, session_id: &str) -> anyhow::Result<Vec<ChatMediaRecord>> {
        Ok(self.conversation.get_session_media(session_id)?)
    }

    fn get_snippet(&self, name: &str) -> anyhow::Result<Option<(String, String, String)>> {
        let out = self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare("SELECT content, language, tags FROM snippets WHERE name = ?1")
                .map_err(StorageError::Sqlite)?;
            let mut rows = stmt
                .query_map(params![name], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
                .map_err(StorageError::Sqlite)?;
            Ok(rows.next().transpose().map_err(StorageError::Sqlite)?)
        })?;
        Ok(out)
    }

    fn list_snippets(&self, tag: Option<&str>) -> anyhow::Result<Vec<String>> {
        let out = self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare("SELECT name, tags FROM snippets ORDER BY name")
                .map_err(StorageError::Sqlite)?;
            let all: Vec<(String, String)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(all)
        })?;
        let filtered = out
            .into_iter()
            .filter(|(_, tags_json)| match tag {
                Some(t) => tags_json.contains(t),
                None => true,
            })
            .map(|(name, _)| name)
            .collect();
        Ok(filtered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> KriaMemoryRuntime {
        let db = Arc::new(Database::open_in_memory().unwrap());
        KriaMemoryRuntime::from_db(db)
    }

    #[test]
    fn turns_and_sessions_roundtrip() {
        let rt = backend();
        rt.store_turn(&MemoryTurnWrite {
            session_id: "s1".into(),
            user_prompt: "hello".into(),
            assistant_response: "hi".into(),
            tool_name: None,
            tool_result: None,
            tokens_used: Some(5),
            timestamp: Utc::now(),
            extraction: None,
        })
        .unwrap();
        let recent = rt.get_recent_turns("s1", 10).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].content, "hello");
        assert_eq!(rt.list_sessions().unwrap().len(), 1);
    }

    #[test]
    fn facts_store_search_decay_delete() {
        let rt = backend();
        let id = rt
            .store_fact(&MemoryFact {
                id: None,
                text: "the sky is blue".into(),
                category: "general".into(),
                source: "test".into(),
                created_at: Utc::now(),
                last_accessed: Utc::now(),
                access_count: 0,
                decay_score: 1.0,
            })
            .unwrap();
        assert!(id > 0);
        let hits = rt.search_facts("sky", 10).unwrap();
        assert_eq!(hits.len(), 1);
        rt.update_fact_decay(id, 0.4).unwrap();
        assert!(rt.all_facts_with_decay(0.5).unwrap().is_empty());
        assert_eq!(rt.all_facts_with_decay(0.3).unwrap().len(), 1);
        rt.delete_fact(id).unwrap();
        assert!(rt.search_facts("sky", 10).unwrap().is_empty());
    }

    #[test]
    fn snippets_and_chunks() {
        let rt = backend();
        rt.store_snippet("greet", "print('hi')", "python", &["demo".into()])
            .unwrap();
        assert_eq!(
            rt.get_snippet("greet").unwrap().map(|(c, _, _)| c),
            Some("print('hi')".to_string())
        );
        assert_eq!(rt.list_snippets(Some("demo")).unwrap(), vec!["greet"]);

        let cid = rt
            .store_document_chunk(&DocumentChunk {
                id: None,
                doc_id: "d1".into(),
                doc_name: "Doc".into(),
                doc_type: "text".into(),
                chunk_index: 0,
                content: "retrieval augmented generation".into(),
                char_offset: 0,
                created_at: Utc::now(),
            })
            .unwrap();
        assert!(cid > 0);
        assert_eq!(rt.search_chunks("retrieval", 10).unwrap().len(), 1);
        assert_eq!(rt.get_chunks_by_doc("d1").unwrap().len(), 1);
        assert_eq!(rt.list_documents().unwrap().len(), 1);
        assert_eq!(rt.delete_document_chunks("d1").unwrap(), 1);
        assert!(rt.search_chunks("retrieval", 10).unwrap().is_empty());
    }

    #[test]
    fn preferences_via_backend() {
        let rt = backend();
        rt.set_preference(&PreferenceRecord {
            key: "theme".into(),
            value: "dark".into(),
        })
        .unwrap();
        assert_eq!(rt.get_preference("theme").unwrap().as_deref(), Some("dark"));
    }
}
