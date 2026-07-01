//! Durable reminder scheduler (Phase 2.3).
//!
//! A polling loop over the persistent reminder table: every `poll_interval`, it
//! fires all due-and-unfired reminders via a callback and marks them fired.
//! Because the DB is the source of truth, reminders **survive restart** — any
//! overdue reminder fires on the first poll after boot, and reminders added at
//! runtime are picked up on the next poll without coupling to the writer.

use std::sync::Arc;
use std::time::Duration;

use super::store::{Reminder, TaskStore};

/// Spawn the reminder polling loop. `fire` is invoked once per due reminder.
///
/// Returns immediately; the loop runs until the process exits.
pub fn spawn<F>(store: Arc<TaskStore>, fire: F, poll_interval: Duration)
where
    F: Fn(&Reminder) + Send + Sync + 'static,
{
    tokio::spawn(async move {
        tracing::info!(
            poll_secs = poll_interval.as_secs(),
            "[reminders] durable scheduler started"
        );
        loop {
            match store.due_reminders(chrono::Utc::now()) {
                Ok(due) => {
                    for reminder in due {
                        fire(&reminder);
                        if let Err(e) = store.mark_reminder_fired(reminder.id) {
                            tracing::warn!(
                                reminder_id = reminder.id,
                                error = %e,
                                "[reminders] failed to mark fired"
                            );
                        }
                        // Recurring → schedule the next occurrence.
                        if reminder.recurrence.is_some() {
                            if let Err(e) = store.reschedule_recurring(&reminder) {
                                tracing::warn!(
                                    reminder_id = reminder.id,
                                    error = %e,
                                    "[reminders] failed to reschedule recurring"
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "[reminders] due query failed");
                }
            }
            tokio::time::sleep(poll_interval).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration as ChronoDuration, Utc};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn fires_overdue_then_marks() {
        let store = TaskStore::open_in_memory().unwrap();
        store
            .add_reminder("ping", Utc::now() - ChronoDuration::minutes(1), None, None)
            .unwrap();

        let count = Arc::new(AtomicUsize::new(0));
        let count2 = count.clone();
        let store = Arc::new(store);
        spawn(
            store.clone(),
            move |_r| {
                count2.fetch_add(1, Ordering::SeqCst);
            },
            Duration::from_millis(50),
        );

        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "overdue reminder fires once"
        );
        assert!(store.due_reminders(Utc::now()).unwrap().is_empty());
    }
}
