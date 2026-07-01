//! Daily planner (Phase 2 intelligence upgrade).
//!
//! Pure, testable greedy time-blocking: fit prioritised tasks into free slots.
//! The tool composes this with Phase 1.2 calendar availability + the task store.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use super::store::Task;

/// A scheduled block in the day plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedBlock {
    pub task_id: i64,
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub minutes: i64,
}

/// Greedily place tasks (already priority-ordered) into free `slots`.
///
/// Each task gets `default_minutes` unless its notes contain an explicit
/// estimate like `~45m`. Tasks that don't fit any remaining slot are returned
/// as `unscheduled` (by id).
pub fn plan_day(
    slots: &[(DateTime<Utc>, DateTime<Utc>)],
    tasks: &[Task],
    default_minutes: i64,
) -> (Vec<PlannedBlock>, Vec<i64>) {
    // Mutable remaining cursors per slot.
    let mut cursors: Vec<(DateTime<Utc>, DateTime<Utc>)> =
        slots.iter().filter(|(s, e)| e > s).cloned().collect();

    let mut blocks = Vec::new();
    let mut unscheduled = Vec::new();

    for task in tasks {
        let minutes = estimate_minutes(task, default_minutes);
        let need = Duration::minutes(minutes);
        let mut placed = false;
        for slot in cursors.iter_mut() {
            if slot.1 - slot.0 >= need {
                let start = slot.0;
                let end = start + need;
                blocks.push(PlannedBlock {
                    task_id: task.id,
                    title: task.title.clone(),
                    start,
                    end,
                    minutes,
                });
                slot.0 = end; // consume the slot front
                placed = true;
                break;
            }
        }
        if !placed {
            unscheduled.push(task.id);
        }
    }

    (blocks, unscheduled)
}

/// Estimate a task's duration. Honours a `~Nm` / `~Nh` hint in notes.
fn estimate_minutes(task: &Task, default_minutes: i64) -> i64 {
    if let Some(notes) = &task.notes {
        if let Some(m) = parse_estimate(notes) {
            return m;
        }
    }
    default_minutes.max(5)
}

fn parse_estimate(text: &str) -> Option<i64> {
    // look for "~45m" or "~2h"
    let lower = text.to_ascii_lowercase();
    let idx = lower.find('~')?;
    let rest: String = lower[idx + 1..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == 'h' || *c == 'm')
        .collect();
    if let Some(h) = rest.strip_suffix('h') {
        return h.parse::<i64>().ok().map(|n| n * 60);
    }
    rest.trim_end_matches('m')
        .parse::<i64>()
        .ok()
        .filter(|n| *n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn dt(h: i64, m: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 18, 0, 0, 0).unwrap()
            + Duration::hours(h)
            + Duration::minutes(m)
    }

    fn task(id: i64, title: &str, notes: Option<&str>) -> Task {
        Task {
            id,
            title: title.into(),
            notes: notes.map(|s| s.to_string()),
            source: "manual".into(),
            status: "open".into(),
            priority_bucket: "normal".into(),
            priority_score: 200,
            due_at: None,
            external_ref: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn fits_tasks_into_slots_in_order() {
        let slots = vec![(dt(9, 0), dt(10, 0)), (dt(14, 0), dt(16, 0))];
        let tasks = vec![task(1, "a", None), task(2, "b", None), task(3, "c", None)];
        let (blocks, unscheduled) = plan_day(&slots, &tasks, 30);
        // slot1 (60m) holds a,b; slot2 holds c
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].task_id, 1);
        assert_eq!((blocks[0].start, blocks[0].end), (dt(9, 0), dt(9, 30)));
        assert_eq!((blocks[1].start, blocks[1].end), (dt(9, 30), dt(10, 0)));
        assert_eq!(blocks[2].start, dt(14, 0));
        assert!(unscheduled.is_empty());
    }

    #[test]
    fn overflow_goes_unscheduled() {
        let slots = vec![(dt(9, 0), dt(9, 30))];
        let tasks = vec![task(1, "a", None), task(2, "b", None)];
        let (blocks, unscheduled) = plan_day(&slots, &tasks, 30);
        assert_eq!(blocks.len(), 1);
        assert_eq!(unscheduled, vec![2]);
    }

    #[test]
    fn honours_notes_estimate() {
        let slots = vec![(dt(9, 0), dt(12, 0))];
        let tasks = vec![task(1, "deep work", Some("focus block ~2h"))];
        let (blocks, _) = plan_day(&slots, &tasks, 30);
        assert_eq!(blocks[0].minutes, 120);
        assert_eq!(blocks[0].end, dt(11, 0));
    }

    #[test]
    fn empty_slots_all_unscheduled() {
        let (blocks, unscheduled) = plan_day(&[], &[task(1, "a", None)], 30);
        assert!(blocks.is_empty());
        assert_eq!(unscheduled, vec![1]);
    }
}
