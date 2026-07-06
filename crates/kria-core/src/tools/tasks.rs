//! Unified task + durable reminder tools (Phase 2).
//!
//! Handlers open a shared [`TaskStore`] against `kria.db`. Registered
//! unconditionally; if the DB can't open, registration is skipped gracefully.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::infra::ToolResult;
use crate::safety::RiskLevel;
use crate::tasks::store::{NewTask, TaskFilter, TaskStore};
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

const VALID_STATUSES: &[&str] = &[
    "open",
    "in_progress",
    "blocked",
    "waiting",
    "done",
    "cancelled",
];

fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

#[derive(Clone)]
struct Store(Arc<TaskStore>);

struct TaskAdd(Store);
#[async_trait]
impl ToolHandler for TaskAdd {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let title = params["title"].as_str().unwrap_or("").trim().to_string();
        if title.is_empty() {
            return ToolResult::err("title is required");
        }
        let notes = params["notes"].as_str().map(|s| s.to_string());
        let source = params["source"].as_str().unwrap_or("manual").to_string();
        let due_at = params["due_at"]
            .as_str()
            .and_then(|s| crate::tasks::nl_time::parse(s, chrono::Utc::now()));

        match self.0 .0.add_task(NewTask {
            title,
            notes,
            source,
            due_at,
            external_ref: params["external_ref"].as_str().map(|s| s.to_string()),
        }) {
            Ok(task) => ToolResult::ok(serde_json::to_value(task).unwrap_or_default()),
            Err(e) => ToolResult::err(format!("failed to add task: {e}")),
        }
    }
}

struct TaskList(Store);
#[async_trait]
impl ToolHandler for TaskList {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let filter = TaskFilter {
            status: params["status"].as_str().map(|s| s.to_string()),
            bucket: params["bucket"].as_str().map(|s| s.to_string()),
            active_only: params["active_only"].as_bool().unwrap_or(false),
        };
        match self.0 .0.list_tasks(&filter) {
            Ok(tasks) => ToolResult::ok(serde_json::json!({
                "count": tasks.len(),
                "tasks": tasks,
            })),
            Err(e) => ToolResult::err(format!("failed to list tasks: {e}")),
        }
    }
}

struct TaskUpdateStatus(Store);
#[async_trait]
impl ToolHandler for TaskUpdateStatus {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let id = match params["id"].as_i64() {
            Some(v) => v,
            None => return ToolResult::err("id (integer) is required"),
        };
        let status = params["status"].as_str().unwrap_or("").trim().to_string();
        if !VALID_STATUSES.contains(&status.as_str()) {
            return ToolResult::err(format!(
                "invalid status '{status}'. Allowed: {}",
                VALID_STATUSES.join(", ")
            ));
        }
        match self.0 .0.update_status(id, &status) {
            Ok(Some(task)) => ToolResult::ok(serde_json::to_value(task).unwrap_or_default()),
            Ok(None) => ToolResult::err(format!("task {id} not found")),
            Err(e) => ToolResult::err(format!("failed to update task: {e}")),
        }
    }
}

struct TaskNext(Store);
#[async_trait]
impl ToolHandler for TaskNext {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        match self.0 .0.next_task() {
            Ok(Some(task)) => ToolResult::ok(serde_json::to_value(task).unwrap_or_default()),
            Ok(None) => ToolResult::ok(serde_json::json!({
                "message": "No actionable tasks. You're all caught up.",
            })),
            Err(e) => ToolResult::err(format!("failed to fetch next task: {e}")),
        }
    }
}

struct TaskStats(Store);
#[async_trait]
impl ToolHandler for TaskStats {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        match self.0 .0.productivity_stats() {
            Ok(stats) => ToolResult::ok(serde_json::to_value(stats).unwrap_or_default()),
            Err(e) => ToolResult::err(format!("failed to compute stats: {e}")),
        }
    }
}

struct ReminderSet(Store);
#[async_trait]
impl ToolHandler for ReminderSet {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let message = params["message"].as_str().unwrap_or("").trim().to_string();
        if message.is_empty() {
            return ToolResult::err("message is required");
        }
        // Accept: `when` (natural "tomorrow 5pm" / ISO), or `fire_at` (ISO),
        // or `fire_in_minutes`.
        let now = Utc::now();
        let fire_at = if let Some(s) = params["when"]
            .as_str()
            .or_else(|| params["fire_at"].as_str())
        {
            match crate::tasks::nl_time::parse(s, now) {
                Some(dt) => dt,
                None => {
                    return ToolResult::err(format!(
                    "couldn't understand time '{s}'. Try ISO 8601, or English like 'tomorrow 5pm'."
                ))
                }
            }
        } else {
            let minutes = params["fire_in_minutes"].as_f64().unwrap_or(5.0).max(0.0);
            now + chrono::Duration::milliseconds((minutes * 60_000.0) as i64)
        };
        let task_id = params["task_id"].as_i64();
        let recurrence = params["recurrence"]
            .as_str()
            .filter(|s| !s.trim().is_empty());

        match self
            .0
             .0
            .add_reminder(&message, fire_at, task_id, recurrence)
        {
            Ok(reminder) => ToolResult::ok(serde_json::json!({
                "scheduled": true,
                "durable": true,
                "reminder": reminder,
            })),
            Err(e) => ToolResult::err(format!("failed to set reminder: {e}")),
        }
    }
}

struct TaskEdit(Store);
#[async_trait]
impl ToolHandler for TaskEdit {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let id = match params["id"].as_i64() {
            Some(v) => v,
            None => return ToolResult::err("id (integer) is required"),
        };
        let title = params["title"].as_str();
        let notes = params["notes"].as_str();
        let clear_due = params["clear_due"].as_bool().unwrap_or(false);
        let due_at = params["due_at"]
            .as_str()
            .and_then(|s| crate::tasks::nl_time::parse(s, chrono::Utc::now()));
        match self.0 .0.update_task(id, title, notes, due_at, clear_due) {
            Ok(Some(task)) => ToolResult::ok(serde_json::to_value(task).unwrap_or_default()),
            Ok(None) => ToolResult::err(format!("task {id} not found")),
            Err(e) => ToolResult::err(format!("failed to edit task: {e}")),
        }
    }
}

struct TaskComplete(Store);
#[async_trait]
impl ToolHandler for TaskComplete {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let query = params["text"].as_str().unwrap_or("").trim().to_string();
        if query.is_empty() {
            return ToolResult::err("text is required (e.g. 'report ho gaya')");
        }
        match self.0 .0.complete_by_text(&query) {
            Ok(Some(task)) => ToolResult::ok(serde_json::json!({
                "completed": true,
                "task": task,
            })),
            Ok(None) => ToolResult::ok(serde_json::json!({
                "completed": false,
                "message": "No matching active task found.",
            })),
            Err(e) => ToolResult::err(format!("failed to complete task: {e}")),
        }
    }
}

struct ReminderSnooze(Store);
#[async_trait]
impl ToolHandler for ReminderSnooze {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let id = match params["id"].as_i64() {
            Some(v) => v,
            None => return ToolResult::err("id (integer) is required"),
        };
        let minutes = params["minutes"].as_i64().unwrap_or(10);
        match self.0 .0.snooze_reminder(id, minutes) {
            Ok(true) => ToolResult::ok(serde_json::json!({ "snoozed": true, "minutes": minutes })),
            Ok(false) => ToolResult::err(format!("reminder {id} not found")),
            Err(e) => ToolResult::err(format!("failed to snooze: {e}")),
        }
    }
}

struct ReminderCancel(Store);
#[async_trait]
impl ToolHandler for ReminderCancel {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let id = match params["id"].as_i64() {
            Some(v) => v,
            None => return ToolResult::err("id (integer) is required"),
        };
        match self.0 .0.cancel_reminder(id) {
            Ok(true) => ToolResult::ok(serde_json::json!({ "cancelled": true })),
            Ok(false) => ToolResult::err(format!("reminder {id} not found")),
            Err(e) => ToolResult::err(format!("failed to cancel: {e}")),
        }
    }
}

struct PlanMyDay(Store);
#[async_trait]
impl ToolHandler for PlanMyDay {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        use crate::tools::availability::{free_slots, BusyInterval};

        let now = chrono::Utc::now();
        let work_start = params["work_start"].as_str().unwrap_or("09:00");
        let work_end = params["work_end"].as_str().unwrap_or("18:00");
        let slot_minutes = params["slot_minutes"].as_i64().unwrap_or(30).max(5);

        let day = now.date_naive();
        let parse_hm = |hm: &str, fallback_h: u32| {
            let mut it = hm.split(':');
            let h = it
                .next()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(fallback_h);
            let m = it.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
            day.and_hms_opt(h.min(23), m.min(59), 0).map(|nd| {
                chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(nd, chrono::Utc)
            })
        };
        let win_start = match parse_hm(work_start, 9) {
            Some(d) => d.max(now),
            None => now,
        };
        let win_end = match parse_hm(work_end, 18) {
            Some(d) => d,
            None => now + chrono::Duration::hours(8),
        };

        // Optional busy intervals (e.g. from gw_calendar_availability).
        let mut busy = Vec::new();
        if let Some(arr) = params["busy"].as_array() {
            for b in arr {
                if let (Some(s), Some(e)) = (
                    b["start"].as_str().and_then(parse_rfc3339),
                    b["end"].as_str().and_then(parse_rfc3339),
                ) {
                    busy.push(BusyInterval {
                        start: s,
                        end: e,
                        title: None,
                    });
                }
            }
        }

        let slots: Vec<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> =
            free_slots(win_start, win_end, &busy, slot_minutes)
                .into_iter()
                .map(|fs| (fs.start, fs.end))
                .collect();

        let tasks = match self.0 .0.list_tasks(&TaskFilter {
            active_only: true,
            ..Default::default()
        }) {
            Ok(t) => t,
            Err(e) => return ToolResult::err(format!("failed to load tasks: {e}")),
        };

        let (blocks, unscheduled) = crate::tasks::planner::plan_day(&slots, &tasks, slot_minutes);
        ToolResult::ok(serde_json::json!({
            "window": { "start": win_start.to_rfc3339(), "end": win_end.to_rfc3339() },
            "planned": blocks,
            "unscheduled_task_ids": unscheduled,
        }))
    }
}

struct ReminderList(Store);
#[async_trait]
impl ToolHandler for ReminderList {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let include_fired = params["include_fired"].as_bool().unwrap_or(false);
        match self.0 .0.list_reminders(include_fired) {
            Ok(reminders) => ToolResult::ok(serde_json::json!({
                "count": reminders.len(),
                "reminders": reminders,
            })),
            Err(e) => ToolResult::err(format!("failed to list reminders: {e}")),
        }
    }
}

/// Register task + reminder tools. Opens the shared `kria.db` TaskStore; on
/// failure, logs and skips (tools simply won't be present).
pub fn register(reg: &ToolRegistry) {
    let paths = crate::platform::paths::KriaPaths::resolve();
    let store = match TaskStore::open(&paths.db_path) {
        Ok(s) => Store(Arc::new(s)),
        Err(e) => {
            tracing::warn!(error = %e, "[tasks] could not open task store — task tools disabled");
            return;
        }
    };

    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "task_add".into(),
                description: "Add a task to the unified task queue. Auto-prioritises by due date and keywords.".into(),
                category: "tasks".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("title", "string", "Task title", true),
                    param("notes", "string", "Optional details", false),
                    param("due_at", "string", "Due date/time (ISO 8601)", false),
                    param("source", "string", "Origin: manual|gmail|calendar|github (default manual)", false),
                    param("external_ref", "string", "External id (e.g. email/issue id)", false),
                ],
            },
            Arc::new(TaskAdd(store.clone())),
        ),
        (
            ToolDef {
                name: "task_list".into(),
                description: "List tasks ordered by priority. Optionally filter by status or priority bucket.".into(),
                category: "tasks".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("status", "string", "Filter: open|in_progress|blocked|waiting|done|cancelled", false),
                    param("bucket", "string", "Filter: urgent|important|blocked|waiting|normal", false),
                    param("active_only", "boolean", "Exclude done/cancelled (default false)", false),
                ],
            },
            Arc::new(TaskList(store.clone())),
        ),
        (
            ToolDef {
                name: "task_update_status".into(),
                description: "Update a task's status (open, in_progress, blocked, waiting, done, cancelled).".into(),
                category: "tasks".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("id", "integer", "Task id", true),
                    param("status", "string", "New status", true),
                ],
            },
            Arc::new(TaskUpdateStatus(store.clone())),
        ),
        (
            ToolDef {
                name: "task_next".into(),
                description: "Get the single highest-priority actionable task — 'what should I work on next'.".into(),
                category: "tasks".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(TaskNext(store.clone())),
        ),
        (
            ToolDef {
                name: "task_stats".into(),
                description: "Productivity stats: open/in-progress/blocked/done counts, overdue, done today, urgent/important.".into(),
                category: "tasks".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(TaskStats(store.clone())),
        ),
        (
            ToolDef {
                name: "reminder_set".into(),
                description: "Set a DURABLE reminder that survives app restart. Time via 'when' (natural: 'tomorrow 5pm') or fire_in_minutes. Recurrence: daily | weekly:fri | monthly:15 | every:30m.".into(),
                category: "tasks".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("message", "string", "Reminder message", true),
                    param("when", "string", "Natural/ISO time ('tomorrow 5pm', '2026-06-19T10:00:00Z')", false),
                    param("fire_in_minutes", "number", "Minutes from now (if no 'when')", false),
                    param("recurrence", "string", "Repeat: daily | weekly:<day> | monthly:<n> | every:<n>m", false),
                    param("task_id", "integer", "Optional linked task id", false),
                ],
            },
            Arc::new(ReminderSet(store.clone())),
        ),
        (
            ToolDef {
                name: "reminder_list".into(),
                description: "List durable reminders (pending by default; pass include_fired=true for all).".into(),
                category: "tasks".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param(
                    "include_fired",
                    "boolean",
                    "Include already-fired reminders (default false)",
                    false,
                )],
            },
            Arc::new(ReminderList(store.clone())),
        ),
        (
            ToolDef {
                name: "task_edit".into(),
                description: "Edit a task's title, notes, or due date (natural language ok). Re-prioritises automatically.".into(),
                category: "tasks".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("id", "integer", "Task id", true),
                    param("title", "string", "New title", false),
                    param("notes", "string", "New notes", false),
                    param("due_at", "string", "New due date (ISO or 'tomorrow 5pm')", false),
                    param("clear_due", "boolean", "Remove the due date", false),
                ],
            },
            Arc::new(TaskEdit(store.clone())),
        ),
        (
            ToolDef {
                name: "task_complete".into(),
                description: "Mark a task done from a natural phrase (e.g. 'report ho gaya') — fuzzy-matches the active task.".into(),
                category: "tasks".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param("text", "string", "What you finished", true)],
            },
            Arc::new(TaskComplete(store.clone())),
        ),
        (
            ToolDef {
                name: "reminder_snooze".into(),
                description: "Snooze a reminder by N minutes from now (default 10).".into(),
                category: "tasks".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("id", "integer", "Reminder id", true),
                    param("minutes", "integer", "Snooze minutes (default 10)", false),
                ],
            },
            Arc::new(ReminderSnooze(store.clone())),
        ),
        (
            ToolDef {
                name: "reminder_cancel".into(),
                description: "Cancel (delete) a reminder by id.".into(),
                category: "tasks".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param("id", "integer", "Reminder id", true)],
            },
            Arc::new(ReminderCancel(store.clone())),
        ),
        (
            ToolDef {
                name: "plan_my_day".into(),
                description: "Build a time-blocked plan: fit active tasks into today's free slots. Pass optional 'busy' intervals (from gw_calendar_availability) for calendar-aware planning.".into(),
                category: "tasks".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("work_start", "string", "Work day start HH:MM (default 09:00)", false),
                    param("work_end", "string", "Work day end HH:MM (default 18:00)", false),
                    param("slot_minutes", "integer", "Default minutes per task (default 30)", false),
                    param("busy", "array", "Busy intervals [{start,end} ISO] to avoid", false),
                ],
            },
            Arc::new(PlanMyDay(store.clone())),
        ),
    ];

    for (def, handler) in tools {
        reg.register(def, handler);
    }
    tracing::info!("[tasks] registered 7 task/reminder tools");
}
