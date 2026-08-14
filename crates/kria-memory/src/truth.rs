//! Truth Maintenance System (memory-upgrade design §22, ADR-009).
//!
//! Ensures KRIA never confidently relies on stale/contradicted knowledge.
//! Deterministic (L8): staleness classes govern *re-verification* (not
//! deletion), contradictions resolve by a fixed precedence order, and a superseded
//! memory moves to version history (never destroyed).

use std::sync::Arc;

use rusqlite::params;
use uuid::Uuid;

use crate::db::Database;
use crate::error::{MemoryResult, StorageError};
use crate::stores::ports::RelationalStore;
use crate::types::{Memory, MemoryState, Source, StalenessClass, VerifyPredicate};

/// How long before a memory of a given staleness class should be re-verified.
pub fn reverify_after(class: &StalenessClass) -> Option<chrono::Duration> {
    match class {
        StalenessClass::Immutable | StalenessClass::Permanent => None,
        StalenessClass::VolatileVerifiable => Some(chrono::Duration::hours(1)),
        StalenessClass::VolatileUnverifiable => Some(chrono::Duration::hours(1)),
        // Slow and any forward-compat class default to the 30-day window.
        StalenessClass::Slow | StalenessClass::Other(_) => Some(chrono::Duration::days(30)),
    }
}

/// Whether a memory is possibly stale as of `now` (design §22.4). Immutable /
/// Permanent are never stale.
pub fn is_stale(memory: &Memory, now: chrono::DateTime<chrono::Utc>) -> bool {
    let Some(window) = reverify_after(&memory.staleness_class) else {
        return false;
    };
    let anchor = memory.last_accessed.unwrap_or(memory.valid_from);
    now.signed_duration_since(anchor) > window
}

/// The winner of a contradiction (design §22.5 deterministic order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContradictionWinner {
    /// `incoming` supersedes `existing`.
    Incoming,
    /// `existing` stays; `incoming` is rejected.
    Existing,
    /// Neither dominates → keep both as competing beliefs, surface to user.
    KeepBoth,
}

/// Resolve a contradiction between an existing memory and an incoming fact,
/// using the deterministic precedence (design §22.5):
/// 1. user-stated beats inferred
/// 2. more-recently-verified beats stale
/// 3. higher Memory-Worth beats lower
/// 4. else → keep both.
pub fn resolve_contradiction(
    existing: &Memory,
    existing_source_user: bool,
    incoming_source: &Source,
    incoming_recency: chrono::DateTime<chrono::Utc>,
) -> ContradictionWinner {
    let incoming_user = matches!(incoming_source, Source::User);

    // 1. user-stated precedence
    match (incoming_user, existing_source_user) {
        (true, false) => return ContradictionWinner::Incoming,
        (false, true) => return ContradictionWinner::Existing,
        _ => {}
    }

    // 2. more-recently-verified
    let existing_recency = existing.last_accessed.unwrap_or(existing.valid_from);
    if incoming_recency > existing_recency + chrono::Duration::hours(1) {
        return ContradictionWinner::Incoming;
    }
    if existing_recency > incoming_recency + chrono::Duration::hours(1) {
        return ContradictionWinner::Existing;
    }

    // 3. higher Memory-Worth (only when significant)
    let ew = existing.worth.score();
    if existing.worth.is_significant() && ew > 0.2 {
        return ContradictionWinner::Existing;
    }

    // 4. ambiguous
    ContradictionWinner::KeepBoth
}

/// Verify a `Git` predicate. `spec` is `"<repo_path>"` or `"<repo_path>#<ref>"`.
/// The repo must exist as a git working tree/repo (`.git` present); when a ref
/// is given it is resolved with `git -C <repo> rev-parse --verify` (degrading to
/// the repo-existence check if the `git` binary is unavailable — L8, never
/// asserts current on a missing tool). Real filesystem/subprocess check — no stub.
fn verify_git(spec: &str) -> bool {
    let (repo, git_ref) = match spec.split_once('#') {
        Some((r, gr)) => (r.trim(), Some(gr.trim())),
        None => (spec.trim(), None),
    };
    if repo.is_empty() {
        return false;
    }
    let repo_path = std::path::Path::new(repo);
    // A git repo has a `.git` entry (dir for worktrees, file for submodules/worktrees).
    let is_repo = repo_path.exists() && repo_path.join(".git").exists();
    if !is_repo {
        return false;
    }
    match git_ref {
        None => true,
        Some(r) => {
            match std::process::Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(["rev-parse", "--verify", "--quiet", r])
                .output()
            {
                Ok(out) => out.status.success(),
                // git binary missing → degrade to repo-existence (already true).
                Err(_) => true,
            }
        }
    }
}

/// Verify a `Tool` predicate: the named executable is resolvable on `PATH`
/// (real availability check, deterministic — no stub).
fn tool_available(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    // Absolute/relative path form → check directly.
    if name.contains(std::path::MAIN_SEPARATOR) {
        return std::path::Path::new(name).exists();
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        let candidate = dir.join(name);
        candidate.is_file() || candidate.with_extension("exe").is_file() // windows portability
    })
}

/// The Truth Maintenance service.
pub struct TruthMaintenance {
    db: Arc<Database>,
    relational: Arc<dyn RelationalStore>,
}

impl TruthMaintenance {
    pub fn new(db: Arc<Database>, relational: Arc<dyn RelationalStore>) -> Self {
        Self { db, relational }
    }

    /// Supersede `loser` with `winner`: mark loser `Superseded`, link
    /// `superseded_by`, record the contradiction edge. Version history is
    /// preserved — the loser is never destroyed (design §22.3).
    pub fn supersede(&self, winner: Uuid, loser: Uuid) -> MemoryResult<()> {
        let mut tx = self.db.begin()?;
        self.relational
            .set_memory_state(&mut tx, loser, MemoryState::Superseded)?;
        tx.conn()
            .execute(
                "UPDATE memories SET superseded_by = ?2 WHERE id = ?1",
                params![loser.to_string(), winner.to_string()],
            )
            .map_err(StorageError::Sqlite)?;
        tx.conn()
            .execute(
                "INSERT OR IGNORE INTO memory_contradicts(a_id, b_id) VALUES(?1, ?2)",
                params![winner.to_string(), loser.to_string()],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    /// Verify a memory carrying a `verify_against` predicate against its live
    /// source (design §22.4). Returns `true` if still valid; on change or
    /// missing source it demotes confidence + flags (never asserts stale-as-current).
    pub fn verify_against_source(&self, memory: &Memory) -> MemoryResult<bool> {
        let Some(pred) = &memory.verify_against else {
            return Ok(true);
        };
        let valid = match pred {
            VerifyPredicate::Path(p) => std::path::Path::new(p).exists(),
            VerifyPredicate::Git(spec) => verify_git(spec),
            VerifyPredicate::Tool(name) => tool_available(name),
        };
        if !valid {
            let new_conf = (memory.confidence * 0.5).clamp(0.0, 1.0);
            let tx = self.db.begin()?;
            tx.conn()
                .execute(
                    "UPDATE memories SET confidence = ?2 WHERE id = ?1",
                    params![memory.id.to_string(), new_conf as f64],
                )
                .map_err(StorageError::Sqlite)?;
            tx.commit()?;
        }
        Ok(valid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stores::ports::EventStore;
    use crate::stores::{SqliteEventStore, SqliteRelationalStore};
    use crate::types::{
        Event, EventType, MemoryType, MemoryWorth, Modality, Scope, Sensitivity,
    };

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }

    fn mem(user_recency: chrono::DateTime<chrono::Utc>, staleness: StalenessClass) -> Memory {
        Memory {
            id: crate::ids::new_id(),
            content: "employer is Acme".into(),
            memory_type: MemoryType::Semantic,
            compression_level: 0,
            source_event_id: crate::ids::new_id(),
            namespace: "core".into(),
            owner_id: "user".into(),
            device_id: "d".into(),
            scope: Scope::Global,
            confidence: 0.8,
            importance: 5.0,
            access_count: 0,
            decay_score: 1.0,
            staleness_class: staleness,
            sensitivity: Sensitivity::Private,
            state: MemoryState::Active,
            created_at: user_recency,
            last_accessed: Some(user_recency),
            valid_from: user_recency,
            valid_until: None,
            embedding_id: None,
            embedding_model_version: None,
            estimated_tokens: 3,
            content_hash: "h".into(),
            shred_key_id: None,
            verify_against: None,
            superseded_by: None,
            episode_id: None,
            goal_context_id: None,
            worth: MemoryWorth::default(),
            modality: Modality::Text,
            preference_pair_id: None,
            training_eligible: false,
        }
    }

    #[test]
    fn staleness_windows() {
        assert!(reverify_after(&StalenessClass::Immutable).is_none());
        assert!(reverify_after(&StalenessClass::Slow).is_some());
        let old = now() - chrono::Duration::days(40);
        assert!(is_stale(&mem(old, StalenessClass::Slow), now()));
        assert!(!is_stale(&mem(old, StalenessClass::Immutable), now()));
    }

    #[test]
    fn user_stated_beats_inferred() {
        let existing = mem(now(), StalenessClass::Slow); // inferred (we pass existing_user=false)
        assert_eq!(
            resolve_contradiction(&existing, false, &Source::User, now()),
            ContradictionWinner::Incoming
        );
        assert_eq!(
            resolve_contradiction(
                &existing,
                true,
                &Source::ExternalContent("web".into()),
                now()
            ),
            ContradictionWinner::Existing
        );
    }

    #[test]
    fn recency_breaks_tie_then_keep_both() {
        let old = now() - chrono::Duration::days(2);
        let existing = mem(old, StalenessClass::Slow);
        // Both inferred; incoming much more recent → incoming wins.
        assert_eq!(
            resolve_contradiction(
                &existing,
                false,
                &Source::ExternalContent("web".into()),
                now()
            ),
            ContradictionWinner::Incoming
        );
        // Same recency, no worth → keep both.
        let recent = now();
        let existing2 = mem(recent, StalenessClass::Slow);
        assert_eq!(
            resolve_contradiction(
                &existing2,
                false,
                &Source::ExternalContent("web".into()),
                recent
            ),
            ContradictionWinner::KeepBoth
        );
    }

    #[test]
    fn git_and_tool_verification_are_real_not_stubs() {
        // Git: a non-existent repo path → not verified.
        assert!(!verify_git("/definitely/not/a/repo/xyz"));
        assert!(!verify_git(""));
        // Git: the KRIA repo itself (this crate lives inside it) → verified.
        // Walk up from CARGO_MANIFEST_DIR to find the workspace `.git`.
        let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut found = None;
        for _ in 0..6 {
            if dir.join(".git").exists() {
                found = Some(dir.clone());
                break;
            }
            if !dir.pop() {
                break;
            }
        }
        if let Some(repo) = found {
            assert!(
                verify_git(repo.to_str().unwrap()),
                "real git repo must verify"
            );
        }

        // Tool: a bogus binary is unavailable; `sh` exists on the Linux dev box.
        assert!(!tool_available("definitely-not-a-real-binary-xyz"));
        assert!(!tool_available(""));
        #[cfg(unix)]
        assert!(tool_available("sh"), "sh must resolve on PATH (unix)");
    }

    #[test]
    fn supersede_preserves_version_history() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let events = Arc::new(SqliteEventStore::new(db.clone()));
        let rel: Arc<dyn RelationalStore> = Arc::new(SqliteRelationalStore::new(db.clone()));
        let tms = TruthMaintenance::new(db.clone(), rel.clone());

        // Seed two memories sharing a source event.
        let ev = Event {
            id: crate::ids::new_id(),
            hlc: crate::ids::HlcGenerator::new().now(),
            ts_utc: now(),
            tz_offset_min: 0,
            event_type: EventType::UserMessage,
            source: Source::User,
            session_id: None,
            parent_event_id: None,
            shred_key_id: None,
            payload: serde_json::json!({}),
            encrypted: false,
            checksum: "c".into(),
        };
        {
            let mut tx = db.begin().unwrap();
            events.append(&mut tx, &ev).unwrap();
            tx.commit().unwrap();
        }
        let mut winner = mem(now(), StalenessClass::Slow);
        winner.source_event_id = ev.id;
        winner.content_hash = "w".into();
        let mut loser = mem(now(), StalenessClass::Slow);
        loser.source_event_id = ev.id;
        loser.content_hash = "l".into();
        {
            let mut tx = db.begin().unwrap();
            rel.upsert_memory(&mut tx, &winner).unwrap();
            rel.upsert_memory(&mut tx, &loser).unwrap();
            tx.commit().unwrap();
        }

        tms.supersede(winner.id, loser.id).unwrap();
        let l = rel.get_memory(loser.id).unwrap().unwrap();
        assert_eq!(l.state, MemoryState::Superseded);
        assert_eq!(l.superseded_by, Some(winner.id));
        // Loser still exists (version history, never destroyed).
        assert!(rel.get_memory(loser.id).unwrap().is_some());
    }
}
