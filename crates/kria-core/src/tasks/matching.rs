//! Natural completion matching (Phase 2 intelligence upgrade).
//!
//! Tiny, dependency-free token-overlap scorer to map a free-text phrase
//! ("report ho gaya", "finished the deploy") to the most likely task by title.
//! No fuzzy crate, no embeddings — good enough for short titles + Hinglish.

use super::store::Task;

fn tokens(s: &str) -> Vec<String> {
    s.to_ascii_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 2) // drop tiny stopword-ish tokens
        .map(|t| t.to_string())
        .collect()
}

/// Score in [0,1]: fraction of the task's title tokens present in the query.
pub fn score(query: &str, title: &str) -> f32 {
    let q: Vec<String> = tokens(query);
    let t: Vec<String> = tokens(title);
    if t.is_empty() || q.is_empty() {
        return 0.0;
    }
    let hits = t.iter().filter(|tok| q.contains(tok)).count();
    hits as f32 / t.len() as f32
}

/// Return the id of the best-matching task above `threshold`, if any.
/// Ties broken by higher priority_score.
pub fn best_match(query: &str, tasks: &[Task], threshold: f32) -> Option<i64> {
    let mut best: Option<(i64, f32, i64)> = None; // (id, score, priority)
    for task in tasks {
        let s = score(query, &task.title);
        if s < threshold {
            continue;
        }
        let better = match best {
            None => true,
            Some((_, bs, bp)) => s > bs || (s == bs && task.priority_score > bp),
        };
        if better {
            best = Some((task.id, s, task.priority_score));
        }
    }
    best.map(|(id, _, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn task(id: i64, title: &str) -> Task {
        Task {
            id,
            title: title.into(),
            notes: None,
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
    fn matches_paraphrase() {
        let tasks = vec![
            task(1, "Send quarterly report"),
            task(2, "Book dentist appointment"),
        ];
        // "report ho gaya" → token "report" matches task 1
        assert_eq!(best_match("report ho gaya", &tasks, 0.3), Some(1));
    }

    #[test]
    fn no_match_below_threshold() {
        let tasks = vec![task(1, "Send quarterly report")];
        assert_eq!(best_match("buy groceries", &tasks, 0.3), None);
    }

    #[test]
    fn picks_best_overlap() {
        let tasks = vec![
            task(1, "deploy staging server"),
            task(2, "deploy production server now"),
        ];
        // "finished deploy production" overlaps task 2 more
        assert_eq!(best_match("finished deploy production", &tasks, 0.3), Some(2));
    }
}
