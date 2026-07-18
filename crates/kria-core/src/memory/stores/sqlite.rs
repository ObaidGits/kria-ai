//! SQLite-backed authority stores (memory-upgrade design §14/§16).
//!
//! Implements the synchronous authority ports over the shared [`Database`].
//! Writes take `&mut AuthorityTx`; reads use the WAL read pool.

use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::memory::db::{AuthorityTx, Database};
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::ids::Hlc;
use crate::memory::types::{parse_source_tag, Event, EventType};

use super::ports::EventStore;

/// Parse a hyphenated UUID string from a row, mapping errors to a storage error.
fn parse_uuid(s: &str) -> Result<Uuid, StorageError> {
    Uuid::parse_str(s).map_err(|e| StorageError::Serde(format!("bad uuid {s:?}: {e}")))
}

fn parse_uuid_opt(s: Option<String>) -> Result<Option<Uuid>, StorageError> {
    match s {
        Some(s) => Ok(Some(parse_uuid(&s)?)),
        None => Ok(None),
    }
}

/// The raw column tuple for an `events` row (extracted inside the rusqlite
/// closure, then converted to [`Event`] where richer error handling is allowed).
struct RawEvent {
    id: String,
    hlc: String,
    ts_utc: String,
    tz_offset_min: i64,
    event_type: String,
    source: String,
    session_id: Option<String>,
    parent_event_id: Option<String>,
    shred_key_id: Option<String>,
    payload: String,
    encrypted: i64,
    checksum: String,
}

impl RawEvent {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            hlc: row.get(1)?,
            ts_utc: row.get(2)?,
            tz_offset_min: row.get(3)?,
            event_type: row.get(4)?,
            source: row.get(5)?,
            session_id: row.get(6)?,
            parent_event_id: row.get(7)?,
            shred_key_id: row.get(8)?,
            payload: row.get(9)?,
            encrypted: row.get(10)?,
            checksum: row.get(11)?,
        })
    }

    fn into_event(self) -> Result<Event, StorageError> {
        let ts_utc = chrono::DateTime::parse_from_rfc3339(&self.ts_utc)
            .map_err(|e| StorageError::Serde(format!("bad ts_utc: {e}")))?
            .with_timezone(&chrono::Utc);
        let hlc =
            Hlc::decode(&self.hlc).ok_or_else(|| StorageError::Serde("bad hlc encoding".into()))?;
        let payload: serde_json::Value = serde_json::from_str(&self.payload)
            .map_err(|e| StorageError::Serde(format!("bad payload json: {e}")))?;
        Ok(Event {
            id: parse_uuid(&self.id)?,
            hlc,
            ts_utc,
            tz_offset_min: self.tz_offset_min as i16,
            event_type: self.event_type.parse::<EventType>().unwrap(), // infallible
            source: parse_source_tag(&self.source),
            session_id: parse_uuid_opt(self.session_id)?,
            parent_event_id: parse_uuid_opt(self.parent_event_id)?,
            shred_key_id: self.shred_key_id,
            payload,
            encrypted: self.encrypted != 0,
            checksum: self.checksum,
        })
    }
}

const EVENT_COLS: &str = "id, hlc, ts_utc, tz_offset_min, event_type, source, \
     session_id, parent_event_id, shred_key_id, payload, encrypted, checksum";

/// SQLite implementation of [`EventStore`].
pub struct SqliteEventStore {
    db: Arc<Database>,
}

impl SqliteEventStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

impl EventStore for SqliteEventStore {
    fn append(&self, tx: &mut AuthorityTx<'_>, event: &Event) -> MemoryResult<()> {
        let payload = serde_json::to_string(&event.payload)
            .map_err(|e| StorageError::Serde(e.to_string()))?;
        // Idempotent by id (Issue 28): re-appending the same event is a no-op.
        tx.conn()
            .execute(
                "INSERT OR IGNORE INTO events(id, hlc, ts_utc, tz_offset_min, event_type, \
                 source, session_id, parent_event_id, shred_key_id, payload, encrypted, checksum) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    event.id.to_string(),
                    event.hlc.encode(),
                    event.ts_utc.to_rfc3339(),
                    event.tz_offset_min as i64,
                    event.event_type.as_str(),
                    event.source.tag(),
                    event.session_id.map(|u| u.to_string()),
                    event.parent_event_id.map(|u| u.to_string()),
                    event.shred_key_id.as_deref(),
                    payload,
                    event.encrypted as i64,
                    event.checksum,
                ],
            )
            .map_err(StorageError::Sqlite)?;
        Ok(())
    }

    fn get(&self, id: Uuid) -> MemoryResult<Option<Event>> {
        self.db.with_read(|conn: &Connection| {
            let raw = conn
                .query_row(
                    &format!("SELECT {EVENT_COLS} FROM events WHERE id = ?1"),
                    params![id.to_string()],
                    RawEvent::from_row,
                )
                .optional()
                .map_err(StorageError::Sqlite)?;
            match raw {
                Some(r) => Ok(Some(r.into_event()?)),
                None => Ok(None),
            }
        })
    }

    fn read_range(&self, from_hlc: &Hlc, limit: usize) -> MemoryResult<Vec<Event>> {
        self.db.with_read(|conn: &Connection| {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {EVENT_COLS} FROM events WHERE hlc > ?1 ORDER BY hlc ASC LIMIT ?2"
                ))
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![from_hlc.encode(), limit as i64], RawEvent::from_row)
                .map_err(StorageError::Sqlite)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.map_err(StorageError::Sqlite)?.into_event()?);
            }
            Ok(out)
        })
    }

    fn cursor(&self, consumer: &str) -> MemoryResult<Hlc> {
        self.db.with_read(|conn: &Connection| {
            let enc: Option<String> = conn
                .query_row(
                    "SELECT last_hlc FROM event_consumer_cursor WHERE consumer = ?1",
                    params![consumer],
                    |r| r.get(0),
                )
                .optional()
                .map_err(StorageError::Sqlite)?;
            Ok(enc
                .filter(|s| !s.is_empty())
                .and_then(|s| Hlc::decode(&s))
                .unwrap_or(Hlc::ZERO))
        })
    }

    fn advance_cursor(
        &self,
        tx: &mut AuthorityTx<'_>,
        consumer: &str,
        hlc: &Hlc,
    ) -> MemoryResult<()> {
        tx.conn()
            .execute(
                "INSERT INTO event_consumer_cursor(consumer, last_hlc) VALUES(?1, ?2) \
                 ON CONFLICT(consumer) DO UPDATE SET last_hlc = excluded.last_hlc",
                params![consumer, hlc.encode()],
            )
            .map_err(StorageError::Sqlite)?;
        Ok(())
    }

    fn pending_count(&self, consumer: &str) -> MemoryResult<u64> {
        let cursor = self.cursor(consumer)?;
        self.db.with_read(move |conn: &Connection| {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM events WHERE hlc > ?1",
                    params![cursor.encode()],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            Ok(n.max(0) as u64)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::ids::{new_id, HlcGenerator};
    use crate::memory::types::Source;

    fn sample_event(hlc: Hlc) -> Event {
        Event {
            id: new_id(),
            hlc,
            ts_utc: chrono::Utc::now(),
            tz_offset_min: 0,
            event_type: EventType::Observation,
            source: Source::User,
            session_id: Some(new_id()),
            parent_event_id: None,
            shred_key_id: None,
            payload: serde_json::json!({"text": "hello"}),
            encrypted: false,
            checksum: "abc".into(),
        }
    }

    #[test]
    fn append_get_roundtrip_and_idempotent() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let store = SqliteEventStore::new(db.clone());
        let gen = HlcGenerator::new();
        let ev = sample_event(gen.now());

        let mut tx = db.begin().unwrap();
        store.append(&mut tx, &ev).unwrap();
        store.append(&mut tx, &ev).unwrap(); // idempotent: no error, no dup
        tx.commit().unwrap();

        let got = store.get(ev.id).unwrap().expect("event present");
        assert_eq!(got.id, ev.id);
        assert_eq!(got.event_type, EventType::Observation);
        assert_eq!(got.source, Source::User);
        assert_eq!(got.payload["text"], "hello");

        // Exactly one row despite double append.
        let range = store.read_range(&Hlc::ZERO, 100).unwrap();
        assert_eq!(range.len(), 1);
    }

    #[test]
    fn read_range_is_ordered_and_bounded() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let store = SqliteEventStore::new(db.clone());
        let gen = HlcGenerator::new();
        let mut hlcs = Vec::new();
        {
            let mut tx = db.begin().unwrap();
            for _ in 0..5 {
                let ev = sample_event(gen.now());
                hlcs.push(ev.hlc);
                store.append(&mut tx, &ev).unwrap();
            }
            tx.commit().unwrap();
        }
        let first_two = store.read_range(&Hlc::ZERO, 2).unwrap();
        assert_eq!(first_two.len(), 2);
        assert!(first_two[0].hlc < first_two[1].hlc);

        // Range strictly after the 3rd hlc returns the last 2.
        let after = store.read_range(&hlcs[2], 100).unwrap();
        assert_eq!(after.len(), 2);
    }

    #[test]
    fn cursor_defaults_zero_and_advances() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let store = SqliteEventStore::new(db.clone());
        assert_eq!(store.cursor("slow_path").unwrap(), Hlc::ZERO);

        let gen = HlcGenerator::new();
        let h = gen.now();
        let mut tx = db.begin().unwrap();
        store.advance_cursor(&mut tx, "slow_path", &h).unwrap();
        tx.commit().unwrap();
        assert_eq!(store.cursor("slow_path").unwrap(), h);
    }

    #[test]
    fn rollback_discards_writes() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let store = SqliteEventStore::new(db.clone());
        let gen = HlcGenerator::new();
        let ev = sample_event(gen.now());
        {
            let mut tx = db.begin().unwrap();
            store.append(&mut tx, &ev).unwrap();
            // drop without commit → rollback
        }
        assert!(store.get(ev.id).unwrap().is_none());
    }
}
