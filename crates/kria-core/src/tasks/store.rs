//! SQLite-backed Task + Reminder store (Phase 2.1/2.3).
//!
//! Mirrors `memory::store::MemoryStore`: a single `Mutex<Connection>` over the
//! shared `kria.db` (WAL), `CREATE TABLE IF NOT EXISTS` migrations, RFC3339 TEXT
//! timestamps. Backs the unified task engine, durable reminders, and
//! productivity analytics.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

use super::priority::{self, PriorityBucket};

/// A user task in the unified queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub notes: Option<String>,
    /// Origin: manual | gmail | calendar | github.
    pub source: String,
    /// open | in_progress | blocked | waiting | done | cancelled.
    pub status: String,
    pub priority_bucket: String,
    pub priority_score: i64,
    pub due_at: Option<DateTime<Utc>>,
    pub external_ref: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A durable reminder (survives restart).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reminder {
    pub id: i64,
    pub message: String,
    pub fire_at: DateTime<Utc>,
    pub fired: bool,
    pub task_id: Option<i64>,
    /// Recurrence rule in storage form (e.g. "daily", "weekly:fri"); None = one-shot.
    #[serde(default)]
    pub recurrence: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Filter for listing tasks.
#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    pub status: Option<String>,
    pub bucket: Option<String>,
    /// Exclude done/cancelled when true.
    pub active_only: bool,
}

/// Productivity snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProductivityStats {
    pub total: i64,
    pub open: i64,
    pub in_progress: i64,
    pub blocked: i64,
    pub waiting: i64,
    pub done: i64,
    pub overdue: i64,
    pub done_today: i64,
    pub urgent: i64,
    pub important: i64,
}

/// New-task input.
#[derive(Debug, Clone)]
pub struct NewTask {
    pub title: String,
    pub notes: Option<String>,
    pub source: String,
    pub due_at: Option<DateTime<Utc>>,
    pub external_ref: Option<String>,
}

pub struct TaskStore {
    conn: Mutex<Connection>,
}

fn parse_dt(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn parse_dt_opt(s: Option<String>) -> Option<DateTime<Utc>> {
    s.and_then(|v| DateTime::parse_from_rfc3339(&v).ok())
        .map(|d| d.with_timezone(&Utc))
}

impl TaskStore {
    /// Open (or create) the store against the shared SQLite DB and migrate.
    pub fn open(db_path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Open an ephemeral in-memory store (tests / ephemeral hosts).
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS tasks (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                title           TEXT NOT NULL,
                notes           TEXT,
                source          TEXT NOT NULL DEFAULT 'manual',
                status          TEXT NOT NULL DEFAULT 'open',
                priority_bucket TEXT NOT NULL DEFAULT 'normal',
                priority_score  INTEGER NOT NULL DEFAULT 0,
                due_at          TEXT,
                external_ref    TEXT,
                created_at      TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
            CREATE INDEX IF NOT EXISTS idx_tasks_score ON tasks(priority_score);

            CREATE TABLE IF NOT EXISTS reminders (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                message     TEXT NOT NULL,
                fire_at     TEXT NOT NULL,
                fired       INTEGER NOT NULL DEFAULT 0,
                task_id     INTEGER,
                recurrence  TEXT,
                created_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_reminders_due ON reminders(fired, fire_at);
            ",
        )?;
        // Add recurrence column for DBs created before the upgrade (ignored if present).
        let _ = conn.execute("ALTER TABLE reminders ADD COLUMN recurrence TEXT", []);
        Ok(())
    }

    // ── Tasks ───────────────────────────────────────────────────────

    pub fn add_task(&self, new: NewTask) -> anyhow::Result<Task> {
        let now = Utc::now();
        let status = "open".to_string();
        let text = format!("{} {}", new.title, new.notes.clone().unwrap_or_default());
        let (bucket, score) = priority::classify(&status, new.due_at, &text, now);

        let id = {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO tasks (title, notes, source, status, priority_bucket, priority_score, due_at, external_ref, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                params![
                    new.title,
                    new.notes,
                    new.source,
                    status,
                    bucket.as_str(),
                    score,
                    new.due_at.map(|d| d.to_rfc3339()),
                    new.external_ref,
                    now.to_rfc3339(),
                ],
            )?;
            conn.last_insert_rowid()
        };
        self.get_task(id)?
            .ok_or_else(|| anyhow::anyhow!("task vanished after insert"))
    }

    pub fn get_task(&self, id: i64) -> anyhow::Result<Option<Task>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, notes, source, status, priority_bucket, priority_score, due_at, external_ref, created_at, updated_at
             FROM tasks WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], row_to_task)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    pub fn list_tasks(&self, filter: &TaskFilter) -> anyhow::Result<Vec<Task>> {
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from(
            "SELECT id, title, notes, source, status, priority_bucket, priority_score, due_at, external_ref, created_at, updated_at
             FROM tasks WHERE 1=1",
        );
        if let Some(ref s) = filter.status {
            sql.push_str(&format!(" AND status = '{}'", s.replace('\'', "")));
        }
        if let Some(ref b) = filter.bucket {
            sql.push_str(&format!(" AND priority_bucket = '{}'", b.replace('\'', "")));
        }
        if filter.active_only {
            sql.push_str(" AND status NOT IN ('done','cancelled')");
        }
        sql.push_str(" ORDER BY priority_score DESC, COALESCE(due_at, '9999') ASC, id ASC");

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_task)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn update_status(&self, id: i64, status: &str) -> anyhow::Result<Option<Task>> {
        {
            let conn = self.conn.lock().unwrap();
            let updated = conn.execute(
                "UPDATE tasks SET status = ?1, updated_at = ?2 WHERE id = ?3",
                params![status, Utc::now().to_rfc3339(), id],
            )?;
            if updated == 0 {
                return Ok(None);
            }
        }
        // Recompute priority since status influences the bucket.
        self.recompute_priority(id)?;
        self.get_task(id)
    }

    /// Recompute the stored priority bucket/score from current fields.
    pub fn recompute_priority(&self, id: i64) -> anyhow::Result<()> {
        let task = match self.get_task(id)? {
            Some(t) => t,
            None => return Ok(()),
        };
        let text = format!("{} {}", task.title, task.notes.unwrap_or_default());
        let (bucket, score) = priority::classify(&task.status, task.due_at, &text, Utc::now());
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE tasks SET priority_bucket = ?1, priority_score = ?2 WHERE id = ?3",
            params![bucket.as_str(), score, id],
        )?;
        Ok(())
    }

    pub fn delete_task(&self, id: i64) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    /// Edit a task's title / notes / due date. `None` fields are left unchanged.
    /// Re-prioritises afterwards. `clear_due = true` removes the due date.
    pub fn update_task(
        &self,
        id: i64,
        title: Option<&str>,
        notes: Option<&str>,
        due_at: Option<DateTime<Utc>>,
        clear_due: bool,
    ) -> anyhow::Result<Option<Task>> {
        {
            let conn = self.conn.lock().unwrap();
            if let Some(t) = title {
                conn.execute(
                    "UPDATE tasks SET title = ?1, updated_at = ?2 WHERE id = ?3",
                    params![t, Utc::now().to_rfc3339(), id],
                )?;
            }
            if let Some(n) = notes {
                conn.execute("UPDATE tasks SET notes = ?1 WHERE id = ?2", params![n, id])?;
            }
            if clear_due {
                conn.execute("UPDATE tasks SET due_at = NULL WHERE id = ?1", params![id])?;
            } else if let Some(d) = due_at {
                conn.execute(
                    "UPDATE tasks SET due_at = ?1 WHERE id = ?2",
                    params![d.to_rfc3339(), id],
                )?;
            }
        }
        self.recompute_priority(id)?;
        self.get_task(id)
    }

    /// Mark the best fuzzy-matching active task as done from a free-text phrase.
    /// Returns the completed task, or None if nothing matched.
    pub fn complete_by_text(&self, query: &str) -> anyhow::Result<Option<Task>> {
        let active = self.list_tasks(&TaskFilter {
            active_only: true,
            ..Default::default()
        })?;
        match super::matching::best_match(query, &active, 0.3) {
            Some(id) => self.update_status(id, "done"),
            None => Ok(None),
        }
    }

    /// Highest-priority actionable task (open/in_progress), or None.
    pub fn next_task(&self) -> anyhow::Result<Option<Task>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, notes, source, status, priority_bucket, priority_score, due_at, external_ref, created_at, updated_at
             FROM tasks WHERE status IN ('open','in_progress')
             ORDER BY priority_score DESC, COALESCE(due_at, '9999') ASC, id ASC LIMIT 1",
        )?;
        let mut rows = stmt.query_map([], row_to_task)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    pub fn productivity_stats(&self) -> anyhow::Result<ProductivityStats> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        let today = Utc::now().date_naive().to_string(); // YYYY-MM-DD prefix
        let mut s = ProductivityStats::default();
        s.total = count(&conn, "SELECT COUNT(*) FROM tasks", params![])?;
        s.open = count(&conn, "SELECT COUNT(*) FROM tasks WHERE status='open'", params![])?;
        s.in_progress = count(&conn, "SELECT COUNT(*) FROM tasks WHERE status='in_progress'", params![])?;
        s.blocked = count(&conn, "SELECT COUNT(*) FROM tasks WHERE status='blocked'", params![])?;
        s.waiting = count(&conn, "SELECT COUNT(*) FROM tasks WHERE status='waiting'", params![])?;
        s.done = count(&conn, "SELECT COUNT(*) FROM tasks WHERE status='done'", params![])?;
        s.overdue = count(
            &conn,
            "SELECT COUNT(*) FROM tasks WHERE status NOT IN ('done','cancelled') AND due_at IS NOT NULL AND due_at < ?1",
            params![now],
        )?;
        s.done_today = count(
            &conn,
            "SELECT COUNT(*) FROM tasks WHERE status='done' AND substr(updated_at,1,10) = ?1",
            params![today],
        )?;
        s.urgent = count(&conn, "SELECT COUNT(*) FROM tasks WHERE priority_bucket='urgent' AND status NOT IN ('done','cancelled')", params![])?;
        s.important = count(&conn, "SELECT COUNT(*) FROM tasks WHERE priority_bucket='important' AND status NOT IN ('done','cancelled')", params![])?;
        Ok(s)
    }

    // ── Reminders ───────────────────────────────────────────────────

    pub fn add_reminder(
        &self,
        message: &str,
        fire_at: DateTime<Utc>,
        task_id: Option<i64>,
        recurrence: Option<&str>,
    ) -> anyhow::Result<Reminder> {
        let now = Utc::now();
        let id = {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO reminders (message, fire_at, fired, task_id, recurrence, created_at)
                 VALUES (?1, ?2, 0, ?3, ?4, ?5)",
                params![
                    message,
                    fire_at.to_rfc3339(),
                    task_id,
                    recurrence,
                    now.to_rfc3339()
                ],
            )?;
            conn.last_insert_rowid()
        };
        Ok(Reminder {
            id,
            message: message.to_string(),
            fire_at,
            fired: false,
            task_id,
            recurrence: recurrence.map(|s| s.to_string()),
            created_at: now,
        })
    }

    /// Snooze a reminder by `minutes` from now and un-fire it.
    pub fn snooze_reminder(&self, id: i64, minutes: i64) -> anyhow::Result<bool> {
        let new_fire = (Utc::now() + chrono::Duration::minutes(minutes.max(1))).to_rfc3339();
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "UPDATE reminders SET fire_at = ?1, fired = 0 WHERE id = ?2",
            params![new_fire, id],
        )?;
        Ok(n > 0)
    }

    /// Cancel (delete) a reminder.
    pub fn cancel_reminder(&self, id: i64) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("DELETE FROM reminders WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    /// If `reminder` recurs, create its next occurrence and return it.
    pub fn reschedule_recurring(&self, reminder: &Reminder) -> anyhow::Result<Option<Reminder>> {
        let rec = super::recurrence::Recurrence::parse(reminder.recurrence.as_deref());
        match rec.next_after(reminder.fire_at) {
            Some(next) => Ok(Some(self.add_reminder(
                &reminder.message,
                next,
                reminder.task_id,
                reminder.recurrence.as_deref(),
            )?)),
            None => Ok(None),
        }
    }

    /// Get a single reminder by id.
    pub fn get_reminder(&self, id: i64) -> anyhow::Result<Option<Reminder>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, message, fire_at, fired, task_id, recurrence, created_at FROM reminders WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], row_to_reminder)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Reminders that are due (fire_at <= now) and not yet fired.
    pub fn due_reminders(&self, now: DateTime<Utc>) -> anyhow::Result<Vec<Reminder>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, message, fire_at, fired, task_id, recurrence, created_at
             FROM reminders WHERE fired = 0 AND fire_at <= ?1 ORDER BY fire_at ASC",
        )?;
        let rows = stmt.query_map(params![now.to_rfc3339()], row_to_reminder)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn mark_reminder_fired(&self, id: i64) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE reminders SET fired = 1 WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn list_reminders(&self, include_fired: bool) -> anyhow::Result<Vec<Reminder>> {
        let conn = self.conn.lock().unwrap();
        let sql = if include_fired {
            "SELECT id, message, fire_at, fired, task_id, recurrence, created_at FROM reminders ORDER BY fire_at ASC"
        } else {
            "SELECT id, message, fire_at, fired, task_id, recurrence, created_at FROM reminders WHERE fired = 0 ORDER BY fire_at ASC"
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], row_to_reminder)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

fn count(conn: &Connection, sql: &str, p: impl rusqlite::Params) -> anyhow::Result<i64> {
    Ok(conn.query_row(sql, p, |r| r.get::<_, i64>(0))?)
}

fn row_to_task(row: &rusqlite::Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get(0)?,
        title: row.get(1)?,
        notes: row.get(2)?,
        source: row.get(3)?,
        status: row.get(4)?,
        priority_bucket: row.get(5)?,
        priority_score: row.get(6)?,
        due_at: parse_dt_opt(row.get::<_, Option<String>>(7)?),
        external_ref: row.get(8)?,
        created_at: parse_dt(&row.get::<_, String>(9)?),
        updated_at: parse_dt(&row.get::<_, String>(10)?),
    })
}

fn row_to_reminder(row: &rusqlite::Row) -> rusqlite::Result<Reminder> {
    Ok(Reminder {
        id: row.get(0)?,
        message: row.get(1)?,
        fire_at: parse_dt(&row.get::<_, String>(2)?),
        fired: row.get::<_, i64>(3)? != 0,
        task_id: row.get(4)?,
        recurrence: row.get(5)?,
        created_at: parse_dt(&row.get::<_, String>(6)?),
    })
}

/// Convenience: the priority bucket of a task as an enum.
pub fn bucket_of(task: &Task) -> PriorityBucket {
    PriorityBucket::from_str(&task.priority_bucket)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn store() -> TaskStore {
        // In-memory DB per test.
        TaskStore::open_in_memory().unwrap()
    }

    fn new_task(title: &str, due: Option<DateTime<Utc>>) -> NewTask {
        NewTask {
            title: title.to_string(),
            notes: None,
            source: "manual".into(),
            due_at: due,
            external_ref: None,
        }
    }

    #[test]
    fn add_get_list_roundtrip() {
        let s = store();
        let t = s.add_task(new_task("write report", None)).unwrap();
        assert!(t.id > 0);
        assert_eq!(t.status, "open");
        let got = s.get_task(t.id).unwrap().unwrap();
        assert_eq!(got.title, "write report");
        assert_eq!(s.list_tasks(&TaskFilter::default()).unwrap().len(), 1);
    }

    #[test]
    fn list_orders_by_priority() {
        let s = store();
        s.add_task(new_task("someday", None)).unwrap(); // normal
        s.add_task(new_task("due soon", Some(Utc::now() + Duration::hours(2))))
            .unwrap(); // urgent
        let list = s.list_tasks(&TaskFilter::default()).unwrap();
        assert_eq!(list[0].title, "due soon");
        assert_eq!(list[0].priority_bucket, "urgent");
    }

    #[test]
    fn update_status_recomputes_priority_and_next_skips_done() {
        let s = store();
        let t = s
            .add_task(new_task("ship", Some(Utc::now() + Duration::hours(1))))
            .unwrap();
        assert_eq!(s.next_task().unwrap().unwrap().id, t.id);
        s.update_status(t.id, "done").unwrap();
        let done = s.get_task(t.id).unwrap().unwrap();
        assert_eq!(done.status, "done");
        assert_eq!(done.priority_score, 0);
        assert!(s.next_task().unwrap().is_none());
    }

    #[test]
    fn active_only_filter_excludes_done() {
        let s = store();
        let a = s.add_task(new_task("a", None)).unwrap();
        let b = s.add_task(new_task("b", None)).unwrap();
        s.update_status(b.id, "done").unwrap();
        let active = s
            .list_tasks(&TaskFilter {
                active_only: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, a.id);
    }

    #[test]
    fn productivity_stats_counts() {
        let s = store();
        s.add_task(new_task("overdue", Some(Utc::now() - Duration::hours(1))))
            .unwrap();
        let done = s.add_task(new_task("finished", None)).unwrap();
        s.update_status(done.id, "done").unwrap();
        let stats = s.productivity_stats().unwrap();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.done, 1);
        assert_eq!(stats.done_today, 1);
        assert_eq!(stats.overdue, 1);
        assert_eq!(stats.urgent, 1);
    }

    #[test]
    fn edit_task_updates_and_reprioritises() {
        let s = store();
        let t = s.add_task(new_task("draft", None)).unwrap();
        assert_eq!(t.priority_bucket, "normal");
        let edited = s
            .update_task(t.id, Some("draft URGENT report"), None, None, false)
            .unwrap()
            .unwrap();
        assert_eq!(edited.title, "draft URGENT report");
        assert_eq!(edited.priority_bucket, "important"); // keyword bumped it
    }

    #[test]
    fn complete_by_text_marks_done() {
        let s = store();
        s.add_task(new_task("Send quarterly report", None)).unwrap();
        s.add_task(new_task("Book dentist", None)).unwrap();
        let done = s.complete_by_text("report ho gaya").unwrap().unwrap();
        assert_eq!(done.status, "done");
        assert!(done.title.contains("report"));
    }

    #[test]
    fn recurring_reminder_reschedules() {
        let s = store();
        let r = s
            .add_reminder("standup", Utc::now() - Duration::minutes(1), None, Some("daily"))
            .unwrap();
        let next = s.reschedule_recurring(&r).unwrap().unwrap();
        assert_eq!(next.message, "standup");
        assert!(next.fire_at > r.fire_at);
        assert_eq!(next.recurrence.as_deref(), Some("daily"));
    }

    #[test]
    fn snooze_and_cancel_reminder() {
        let s = store();
        let r = s
            .add_reminder("x", Utc::now() - Duration::minutes(5), None, None)
            .unwrap();
        assert!(s.snooze_reminder(r.id, 10).unwrap());
        assert!(s.due_reminders(Utc::now()).unwrap().is_empty()); // pushed to future
        assert!(s.cancel_reminder(r.id).unwrap());
        assert!(s.get_reminder(r.id).unwrap().is_none());
    }
}
