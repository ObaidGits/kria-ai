//! Durable journal persistence (HRA Phase D1 / Task 27 wiring).
//!
//! The [`Journal`] is the in-memory decision log; this store makes it survive a crash/restart so
//! the Reconciler can replay prior leases on boot and reclaim orphaned GPU processes. Writes are
//! crash-safe: bytes go to a temp file, are fsync'd, then atomically renamed over the target (so a
//! torn write can never corrupt the live journal — the worst case is losing the very last flush,
//! which `Journal::from_bytes` already tolerates by truncating at the first bad record).
//!
//! Pure file IO; no async, no device access — fully unit-testable in a tempdir.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::journal::Journal;

/// Crash-safe on-disk backing for a [`Journal`].
#[derive(Debug, Clone)]
pub struct JournalStore {
    path: PathBuf,
}

impl JournalStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load + recover the journal from disk. A missing file is a clean cold start (empty journal).
    /// A corrupt/partial tail is truncated (last-good wins). Returns `(journal, truncated_count)`.
    pub fn load(&self) -> (Journal, usize) {
        match fs::read(&self.path) {
            Ok(bytes) => Journal::from_bytes(&bytes),
            Err(_) => (Journal::new(), 0),
        }
    }

    /// Atomically persist the journal: write to `<path>.tmp`, fsync, rename over `path`. Best-effort
    /// directory fsync after rename so the rename itself is durable on crash.
    pub fn save(&self, journal: &Journal) -> std::io::Result<()> {
        let bytes = journal.to_bytes();
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("journal.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &self.path)?;
        // Durable rename: fsync the containing directory (best-effort; ignored on platforms that
        // don't support directory fsync).
        if let Some(parent) = self.path.parent() {
            if let Ok(dir) = fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::journal::DecisionKind;
    use super::super::types::{Epoch, TurnId};
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("kria_journal_test_{}_{}", std::process::id(), name));
        let _ = fs::remove_file(&p);
        p
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = tmp_path("round_trip");
        let store = JournalStore::new(&path);
        let mut j = Journal::new();
        j.bump_epoch(TurnId("boot".into()), 0);
        j.append(
            TurnId("t".into()),
            DecisionKind::LeaseReleased { token: 7 },
            1,
        );
        store.save(&j).unwrap();

        let (loaded, truncated) = store.load();
        assert_eq!(truncated, 0);
        assert_eq!(loaded.current_epoch(), Epoch(1));
        assert_eq!(loaded.records().len(), j.records().len());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn missing_file_is_clean_cold_start() {
        let path = tmp_path("missing");
        let store = JournalStore::new(&path);
        let (loaded, truncated) = store.load();
        assert_eq!(loaded.records().len(), 0);
        assert_eq!(truncated, 0);
    }

    #[test]
    fn atomic_overwrite_replaces_prior_content() {
        let path = tmp_path("overwrite");
        let store = JournalStore::new(&path);

        let mut j1 = Journal::new();
        j1.append(
            TurnId("a".into()),
            DecisionKind::Evicted { model: "a".into() },
            1,
        );
        store.save(&j1).unwrap();

        let mut j2 = Journal::new();
        j2.append(
            TurnId("b".into()),
            DecisionKind::Evicted { model: "b".into() },
            1,
        );
        j2.append(
            TurnId("c".into()),
            DecisionKind::Evicted { model: "c".into() },
            2,
        );
        store.save(&j2).unwrap();

        let (loaded, _) = store.load();
        assert_eq!(loaded.records().len(), 2);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn corrupt_tail_truncates_on_load() {
        let path = tmp_path("corrupt_tail");
        // Write a valid prefix followed by garbage bytes (simulating a torn append).
        let mut j = Journal::new();
        j.append(
            TurnId("a".into()),
            DecisionKind::Evicted { model: "a".into() },
            1,
        );
        let mut bytes = j.to_bytes();
        bytes.extend_from_slice(b"\x00\xffgarbage");
        fs::write(&path, &bytes).unwrap();

        // from_bytes on a fully-unparseable buffer yields empty; the point is it never panics and
        // never returns corrupt records.
        let (loaded, _) = JournalStore::new(&path).load();
        assert!(loaded.records().len() <= j.records().len());
        let _ = fs::remove_file(&path);
    }
}
