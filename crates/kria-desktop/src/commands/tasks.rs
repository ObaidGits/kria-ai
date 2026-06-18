//! Tauri commands for the unified Task engine + durable reminders (Phase 2).
//!
//! These let the frontend Task board read/write tasks directly (the agent uses
//! the equivalent tools). Each opens the shared `kria.db` TaskStore.

use chrono::{DateTime, Duration, Utc};
use kria_core::tasks::{NewTask, ProductivityStats, Reminder, Task, TaskFilter, TaskStore};

fn open() -> Result<TaskStore, String> {
    let paths = kria_core::platform::paths::KriaPaths::resolve();
    TaskStore::open(&paths.db_path).map_err(|e| format!("failed to open task store: {e}"))
}

fn parse_due(s: Option<String>) -> Option<DateTime<Utc>> {
    s.and_then(|v| DateTime::parse_from_rfc3339(v.trim()).ok())
        .map(|d| d.with_timezone(&Utc))
}

#[tauri::command]
pub async fn task_list(
    status: Option<String>,
    bucket: Option<String>,
    active_only: Option<bool>,
) -> Result<Vec<Task>, String> {
    let store = open()?;
    store
        .list_tasks(&TaskFilter {
            status,
            bucket,
            active_only: active_only.unwrap_or(false),
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn task_add(
    title: String,
    notes: Option<String>,
    due_at: Option<String>,
    source: Option<String>,
) -> Result<Task, String> {
    let store = open()?;
    store
        .add_task(NewTask {
            title,
            notes,
            source: source.unwrap_or_else(|| "manual".into()),
            due_at: parse_due(due_at),
            external_ref: None,
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn task_update_status(id: i64, status: String) -> Result<Option<Task>, String> {
    open()?.update_status(id, &status).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn task_delete(id: i64) -> Result<bool, String> {
    open()?.delete_task(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn task_stats() -> Result<ProductivityStats, String> {
    open()?.productivity_stats().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reminder_list(include_fired: Option<bool>) -> Result<Vec<Reminder>, String> {
    open()?
        .list_reminders(include_fired.unwrap_or(false))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reminder_set(
    message: String,
    when: Option<String>,
    fire_in_minutes: Option<f64>,
    fire_at: Option<String>,
    recurrence: Option<String>,
) -> Result<Reminder, String> {
    let store = open()?;
    let now = Utc::now();
    let fire = if let Some(s) = when.or(fire_at) {
        kria_core::tasks::nl_time::parse(&s, now)
            .ok_or_else(|| format!("couldn't understand time '{s}'"))?
    } else {
        let minutes = fire_in_minutes.unwrap_or(5.0).max(0.0);
        now + Duration::milliseconds((minutes * 60_000.0) as i64)
    };
    store
        .add_reminder(&message, fire, None, recurrence.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn task_edit(
    id: i64,
    title: Option<String>,
    notes: Option<String>,
    due_at: Option<String>,
    clear_due: Option<bool>,
) -> Result<Option<Task>, String> {
    let due = due_at.and_then(|s| kria_core::tasks::nl_time::parse(&s, Utc::now()));
    open()?
        .update_task(
            id,
            title.as_deref(),
            notes.as_deref(),
            due,
            clear_due.unwrap_or(false),
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn task_complete(text: String) -> Result<Option<Task>, String> {
    open()?.complete_by_text(&text).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reminder_snooze(id: i64, minutes: Option<i64>) -> Result<bool, String> {
    open()?
        .snooze_reminder(id, minutes.unwrap_or(10))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reminder_cancel(id: i64) -> Result<bool, String> {
    open()?.cancel_reminder(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plan_my_day(
    work_start: Option<String>,
    work_end: Option<String>,
    slot_minutes: Option<i64>,
) -> Result<serde_json::Value, String> {
    use kria_core::tasks::TaskFilter;
    use kria_core::tools::availability::free_slots;

    let store = open()?;
    let now = Utc::now();
    let day = now.date_naive();
    let slot = slot_minutes.unwrap_or(30).max(5);

    let parse_hm = |hm: &str, fb_h: u32| -> DateTime<Utc> {
        let mut it = hm.split(':');
        let h = it.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(fb_h);
        let m = it.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
        day.and_hms_opt(h.min(23), m.min(59), 0)
            .map(|nd| DateTime::<Utc>::from_naive_utc_and_offset(nd, Utc))
            .unwrap_or(now)
    };
    let win_start = parse_hm(work_start.as_deref().unwrap_or("09:00"), 9).max(now);
    let win_end = parse_hm(work_end.as_deref().unwrap_or("18:00"), 18);

    let slots: Vec<(DateTime<Utc>, DateTime<Utc>)> = free_slots(win_start, win_end, &[], slot)
        .into_iter()
        .map(|fs| (fs.start, fs.end))
        .collect();
    let tasks = store
        .list_tasks(&TaskFilter {
            active_only: true,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
    let (blocks, unscheduled) = kria_core::tasks::planner::plan_day(&slots, &tasks, slot);
    Ok(serde_json::json!({
        "window": { "start": win_start.to_rfc3339(), "end": win_end.to_rfc3339() },
        "planned": blocks,
        "unscheduled_task_ids": unscheduled,
    }))
}
