//! Conversation history store (memory-upgrade Step-1 cutover).
//!
//! Chat/session replay is a distinct concern from cognitive memory: it stores
//! raw turns for UI restoration and session listing, not derived knowledge.
//! `ConversationStore` is the production replacement for the legacy
//! `MemoryStore` conversation/session/preference surface, backed by the same
//! authority `Database` (unified backup/encryption). API is intentionally
//! faithful to the legacy one so consumers swap with a one-line change.
//!
//! **Pending F1.5/F2 governed-writer cutover.** [`CommandCandidate::conversation_turn`](
//! crate::authority::CommandCandidate::conversation_turn) is the typed
//! command-candidate scaffolding (task F1.5.1) this store's turn writes will
//! route through once a concrete `TxSemanticStore` builder persists a
//! conversation-turn semantic row (F2). Until that builder exists, cutting
//! [`ConversationStore::store_turn`] over to the
//! [`AuthorityCommandBus`](crate::authority::AuthorityCommandBus) would
//! silently stop persisting turn content — the bus's only available semantic
//! store today (`DeferredSemanticStore`) writes no concrete row. This module
//! therefore remains the live, real persistence path for chat history until
//! F2 lands; see the ledger in [`crate::model::legacy_mapping`].

use std::sync::Arc;

use chrono::{DateTime, Utc};
use rusqlite::params;

use crate::db::Database;
use crate::error::{MemoryResult, StorageError};
// Single source of truth for these records (no duplicate definitions).
pub use crate::runtime_types::{ChatMediaRecord, ConversationTurn};

/// A session summary: `(session_id, turn_count, last_active)`.
pub type SessionSummary = (String, i64, String);

/// Conversation history store over the shared authority database.
#[derive(Clone)]
pub struct ConversationStore {
    db: Arc<Database>,
}

impl ConversationStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Persist a turn + its FTS row. Returns the new row id.
    pub fn store_turn(&self, turn: &ConversationTurn) -> MemoryResult<i64> {
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "INSERT INTO conversations (session_id, role, content, tool_name, tool_result, \
                 tokens_used, timestamp) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    turn.session_id,
                    turn.role,
                    turn.content,
                    turn.tool_name,
                    turn.tool_result,
                    turn.tokens_used,
                    turn.timestamp.to_rfc3339(),
                ],
            )
            .map_err(StorageError::Sqlite)?;
        let id = tx.conn().last_insert_rowid();
        tx.conn()
            .execute(
                "INSERT INTO conversations_fts(rowid, content) VALUES (?1, ?2)",
                params![id, turn.content],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()?;
        Ok(id)
    }

    /// Most recent `limit` turns for a session, in chronological order.
    pub fn get_recent_turns(
        &self,
        session_id: &str,
        limit: usize,
    ) -> MemoryResult<Vec<ConversationTurn>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, role, content, tool_name, tool_result, tokens_used, \
                     timestamp FROM conversations WHERE session_id = ?1 ORDER BY id DESC LIMIT ?2",
                )
                .map_err(StorageError::Sqlite)?;
            let turns = stmt
                .query_map(params![session_id, limit as i64], |row| {
                    Ok(ConversationTurn {
                        id: Some(row.get(0)?),
                        session_id: row.get(1)?,
                        role: row.get(2)?,
                        content: row.get(3)?,
                        tool_name: row.get(4)?,
                        tool_result: row.get(5)?,
                        tokens_used: row.get(6)?,
                        timestamp: row.get::<_, String>(7).map(|s| {
                            DateTime::parse_from_rfc3339(&s)
                                .map(|d| d.with_timezone(&Utc))
                                .unwrap_or_else(|_| Utc::now())
                        })?,
                    })
                })
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(turns.into_iter().rev().collect())
        })
    }

    /// Every turn in a session, oldest first.
    ///
    /// Separate from [`Self::get_recent_turns`] because that one takes a `limit` and
    /// its callers pass 100 — correct for building a prompt context window, silently
    /// wrong for an export, where a longer conversation would lose its beginning with
    /// nothing to indicate the file is incomplete.
    ///
    /// There is no limit parameter here on purpose: a caller that wants the whole
    /// conversation should not have to guess a number large enough.
    pub fn get_all_turns(&self, session_id: &str) -> MemoryResult<Vec<ConversationTurn>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, role, content, tool_name, tool_result, tokens_used, \
                     timestamp FROM conversations WHERE session_id = ?1 ORDER BY id ASC",
                )
                .map_err(StorageError::Sqlite)?;
            let turns = stmt
                .query_map(params![session_id], |row| {
                    Ok(ConversationTurn {
                        id: Some(row.get(0)?),
                        session_id: row.get(1)?,
                        role: row.get(2)?,
                        content: row.get(3)?,
                        tool_name: row.get(4)?,
                        tool_result: row.get(5)?,
                        tokens_used: row.get(6)?,
                        timestamp: row.get::<_, String>(7).map(|s| {
                            DateTime::parse_from_rfc3339(&s)
                                .map(|d| d.with_timezone(&Utc))
                                .unwrap_or_else(|_| Utc::now())
                        })?,
                    })
                })
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(turns)
        })
    }

    /// Full-text search across conversation turns (parity with the legacy
    /// `MemoryReader::search_conversations`). Uses the FTS5 index, newest match
    /// first. Empty/operator-only queries return no rows.
    pub fn search_conversations(
        &self,
        query: &str,
        limit: usize,
    ) -> MemoryResult<Vec<ConversationTurn>> {
        let Some(match_expr) = crate::stores::fts5_query(query) else {
            return Ok(Vec::new());
        };
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT c.id, c.session_id, c.role, c.content, c.tool_name, c.tool_result, \
                     c.tokens_used, c.timestamp FROM conversations c \
                     JOIN conversations_fts f ON c.id = f.rowid \
                     WHERE conversations_fts MATCH ?1 ORDER BY c.id DESC LIMIT ?2",
                )
                .map_err(StorageError::Sqlite)?;
            let turns = stmt
                .query_map(params![match_expr, limit as i64], |row| {
                    Ok(ConversationTurn {
                        id: Some(row.get(0)?),
                        session_id: row.get(1)?,
                        role: row.get(2)?,
                        content: row.get(3)?,
                        tool_name: row.get(4)?,
                        tool_result: row.get(5)?,
                        tokens_used: row.get(6)?,
                        timestamp: row.get::<_, String>(7).map(|s| {
                            DateTime::parse_from_rfc3339(&s)
                                .map(|d| d.with_timezone(&Utc))
                                .unwrap_or_else(|_| Utc::now())
                        })?,
                    })
                })
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(turns)
        })
    }

    /// List sessions with turn counts + last-active, newest first.
    pub fn list_sessions(&self) -> MemoryResult<Vec<SessionSummary>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT session_id, COUNT(*) as turns, MAX(timestamp) as last_active \
                     FROM conversations GROUP BY session_id ORDER BY last_active DESC",
                )
                .map_err(StorageError::Sqlite)?;
            let sessions = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(sessions)
        })
    }

    /// Delete a session's turns (and FTS rows). Returns rows removed.
    pub fn delete_session(&self, session_id: &str) -> MemoryResult<usize> {
        let tx = self.db.begin()?;
        // Remove FTS rows for the session's turns first.
        tx.conn()
            .execute(
                "INSERT INTO conversations_fts(conversations_fts, rowid, content) \
                 SELECT 'delete', id, content FROM conversations WHERE session_id = ?1",
                params![session_id],
            )
            .ok(); // best-effort FTS cleanup
        tx.conn()
            .execute(
                "DELETE FROM chat_media WHERE session_id = ?1",
                params![session_id],
            )
            .map_err(StorageError::Sqlite)?;
        let n = tx
            .conn()
            .execute(
                "DELETE FROM conversations WHERE session_id = ?1",
                params![session_id],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()?;
        Ok(n)
    }

    /// Persist a chat-media record. Returns the row id.
    pub fn store_chat_media(&self, record: &ChatMediaRecord) -> MemoryResult<i64> {
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "INSERT INTO chat_media (session_id, media_type, file_path, sha256, prompt, \
                 width, height, style, provenance) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    record.session_id,
                    record.media_type,
                    record.file_path,
                    record.sha256,
                    record.prompt,
                    record.width.map(|v| v as i64),
                    record.height.map(|v| v as i64),
                    record.style,
                    record.provenance,
                ],
            )
            .map_err(StorageError::Sqlite)?;
        let id = tx.conn().last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// All media for a session, oldest first.
    pub fn get_session_media(&self, session_id: &str) -> MemoryResult<Vec<ChatMediaRecord>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT session_id, media_type, file_path, sha256, prompt, width, height, \
                     style, provenance FROM chat_media WHERE session_id = ?1 ORDER BY created_at ASC",
                )
                .map_err(StorageError::Sqlite)?;
            let records = stmt
                .query_map(params![session_id], |row| {
                    Ok(ChatMediaRecord {
                        session_id: row.get(0)?,
                        media_type: row.get(1)?,
                        file_path: row.get(2)?,
                        sha256: row.get(3)?,
                        prompt: row.get(4)?,
                        width: row.get::<_, Option<i64>>(5)?.map(|v| v as u32),
                        height: row.get::<_, Option<i64>>(6)?.map(|v| v as u32),
                        style: row.get(7)?,
                        provenance: row.get(8)?,
                    })
                })
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(records)
        })
    }

    // ── Preferences (reuse the authority `preferences` table) ──

    pub fn set_preference(&self, key: &str, value: &str) -> MemoryResult<()> {
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "INSERT INTO preferences(key, value, vector_clock, updated_at, device_id) \
                 VALUES(?1,?2,'',?3,'local') \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![key, value, Utc::now().to_rfc3339()],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    pub fn get_preference(&self, key: &str) -> MemoryResult<Option<String>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare("SELECT value FROM preferences WHERE key = ?1")
                .map_err(StorageError::Sqlite)?;
            let mut rows = stmt.query(params![key]).map_err(StorageError::Sqlite)?;
            if let Some(row) = rows.next().map_err(StorageError::Sqlite)? {
                Ok(Some(row.get(0).map_err(StorageError::Sqlite)?))
            } else {
                Ok(None)
            }
        })
    }

    pub fn delete_preference(&self, key: &str) -> MemoryResult<usize> {
        let tx = self.db.begin()?;
        let n = tx
            .conn()
            .execute("DELETE FROM preferences WHERE key = ?1", params![key])
            .map_err(StorageError::Sqlite)?;
        tx.commit()?;
        Ok(n)
    }

    /// Delete all preferences whose key is scoped to a session (keys containing
    /// `:{session_id}`), mirroring the legacy cleanup semantics.
    pub fn delete_session_preferences(&self, session_id: &str) -> MemoryResult<usize> {
        let tx = self.db.begin()?;
        let pattern = format!("%:{session_id}");
        let n = tx
            .conn()
            .execute(
                "DELETE FROM preferences WHERE key LIKE ?1",
                params![pattern],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(session: &str, role: &str, content: &str) -> ConversationTurn {
        ConversationTurn {
            id: None,
            session_id: session.into(),
            role: role.into(),
            content: content.into(),
            tool_name: None,
            tool_result: None,
            tokens_used: Some(10),
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn turns_persist_and_return_in_order() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let cs = ConversationStore::new(db);
        cs.store_turn(&turn("s1", "user", "hello")).unwrap();
        cs.store_turn(&turn("s1", "assistant", "hi there")).unwrap();
        cs.store_turn(&turn("s2", "user", "other session")).unwrap();

        let recent = cs.get_recent_turns("s1", 10).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].content, "hello"); // chronological
        assert_eq!(recent[1].content, "hi there");
    }

    #[test]
    fn search_conversations_finds_by_content() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let cs = ConversationStore::new(db);
        cs.store_turn(&turn("s1", "user", "how do I configure the memory system"))
            .unwrap();
        cs.store_turn(&turn("s1", "assistant", "use the write policy"))
            .unwrap();
        let hits = cs.search_conversations("memory", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("memory system"));
        assert!(cs.search_conversations("", 10).unwrap().is_empty());
    }

    #[test]
    fn sessions_listed_and_deletable() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let cs = ConversationStore::new(db);
        cs.store_turn(&turn("s1", "user", "a")).unwrap();
        cs.store_turn(&turn("s2", "user", "b")).unwrap();
        assert_eq!(cs.list_sessions().unwrap().len(), 2);

        let removed = cs.delete_session("s1").unwrap();
        assert_eq!(removed, 1);
        assert_eq!(cs.list_sessions().unwrap().len(), 1);
        assert!(cs.get_recent_turns("s1", 10).unwrap().is_empty());
    }

    #[test]
    fn preferences_crud() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let cs = ConversationStore::new(db);
        cs.set_preference("theme", "dark").unwrap();
        assert_eq!(cs.get_preference("theme").unwrap().as_deref(), Some("dark"));
        cs.set_preference("theme", "light").unwrap(); // overwrite
        assert_eq!(
            cs.get_preference("theme").unwrap().as_deref(),
            Some("light")
        );
        assert_eq!(cs.delete_preference("theme").unwrap(), 1);
        assert!(cs.get_preference("theme").unwrap().is_none());
        assert!(cs.get_preference("missing").unwrap().is_none());
    }

    #[test]
    fn session_scoped_preference_cleanup() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let cs = ConversationStore::new(db);
        cs.set_preference("session_title:abc", "My chat").unwrap();
        cs.set_preference("session_title:xyz", "Other").unwrap();
        cs.set_preference("global_key", "keep").unwrap();
        let removed = cs.delete_session_preferences("abc").unwrap();
        assert_eq!(removed, 1);
        assert!(cs.get_preference("session_title:abc").unwrap().is_none());
        assert_eq!(
            cs.get_preference("global_key").unwrap().as_deref(),
            Some("keep")
        );
    }

    #[test]
    fn chat_media_roundtrip() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let cs = ConversationStore::new(db);
        cs.store_chat_media(&ChatMediaRecord {
            session_id: "s1".into(),
            media_type: "image".into(),
            file_path: "/tmp/a.png".into(),
            sha256: Some("abc".into()),
            prompt: Some("a cat".into()),
            width: Some(512),
            height: Some(512),
            style: None,
            provenance: Some("generated".into()),
        })
        .unwrap();
        let media = cs.get_session_media("s1").unwrap();
        assert_eq!(media.len(), 1);
        assert_eq!(media[0].width, Some(512));
    }
}
