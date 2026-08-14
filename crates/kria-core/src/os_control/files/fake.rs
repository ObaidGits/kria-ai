//! Deny-live fake [`TrashTransport`] (OSC-010, OSC-011, OSC-033), Task 3.1.
//!
//! Compiled only under `os-control-test`. Nothing here opens a bus, spawns a
//! process, or touches a raw device, so
//! [`crate::os_control::access::deny_live_transport`] is unreachable from this
//! file and the process-wide deny-live sentinel never trips.
//!
//! # Why this fake wraps a real transport
//!
//! Trash is a **filesystem layout**, not a protocol: the freedesktop
//! `files/` + `info/*.trashinfo` pair either exists on disk or it does not.
//! A fake that returned canned booleans instead would certify nothing, so this
//! fake models a genuine trash directory **under a caller-supplied root**
//! ([`RealTrashTransport`] over that root) and adds what a scripted double is
//! actually for: call recording, scriptable faults, and a permanent-delete
//! model.
//!
//! # It can never reach the user's real Trash
//!
//! [`FakeTrashTransport::new`] **panics** if the root is (or is inside) the
//! real Trash — `$XDG_DATA_HOME/Trash`, `$HOME/.local/share/Trash`, or a
//! per-filesystem `.Trash-<uid>` directory. Tests pass a `tempfile`-backed
//! root ([`crate::os_control::testing::temp_dir`]), per OSC-010.7.
//!
//! # Facts this fake keeps honest
//!
//! * **An occupied original path is a conflict, never a silent overwrite.**
//!   The real transport already reports
//!   [`occupied_restore_target_error`] when the path is genuinely occupied;
//!   [`FakeTrashTransport::restore_conflict`] scripts that same conflict so
//!   the blocked branch is testable without staging an occupant, and it is
//!   refused **before** any move.
//! * **Permanent deletion is irreversible.** [`FakeTrashTransport::purge_now`]
//!   models `delete_permanently`: the entry and its metadata are gone, so
//!   [`TrashTransport::item_present`] reports `false` and
//!   [`TrashTransport::restore_item`] refuses. There is deliberately **no**
//!   un-purge/undo method on this type — a fake that offered one would let a
//!   test certify a recovery the OS cannot perform.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;

use crate::os_control::context::AdmittedMutationContext;
use crate::os_control::contract::{ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::ApplyOutcome;
use crate::os_control::testing::CallRecorder;

use super::trash::{
    occupied_restore_target_error, unknown_trash_item_error, RealTrashTransport, TrashItem,
    TrashItemId, TrashMoveOutcome, TrashTransport,
};

/// Provider identity reported by the fake transport.
pub const FAKE_TRASH_PROVIDER_ID: &str = "fake-trash-freedesktop";

/// Whether `root` is (or is inside) a real user Trash directory.
///
/// Deliberately dependency-free and env-driven: this type never calls
/// `dirs::data_dir()`, it only refuses to *be pointed at* the real thing.
fn is_real_trash_root(root: &Path) -> bool {
    // A per-filesystem `$topdir/.Trash-<uid>` (or `$topdir/.Trash`) is the
    // other real Trash location, wherever it lives.
    let named_trash = root.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|name| name == ".Trash" || name.starts_with(".Trash-"))
    });
    if named_trash {
        return true;
    }

    let mut real_roots: Vec<PathBuf> = Vec::new();
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            real_roots.push(PathBuf::from(xdg).join("Trash"));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        if !home.is_empty() {
            real_roots.push(PathBuf::from(home).join(".local").join("share").join("Trash"));
        }
    }
    real_roots.iter().any(|real| root.starts_with(real))
}

/// A recording, scriptable Trash transport over an injected trash root.
pub struct FakeTrashTransport {
    /// The genuine freedesktop layout under the caller-supplied root.
    inner: RealTrashTransport,
    root: PathBuf,
    /// Every transport call, in order (`"trash_path"`, `"path_present"`, …).
    recorder: CallRecorder,
    /// Mutating dispatch attempts, including pre-effect refusals.
    dispatches: Mutex<usize>,
    /// Items whose restore must report the occupied-target conflict.
    restore_conflicts: Mutex<Vec<TrashItemId>>,
    /// Items permanently deleted. Irreversible: there is no way back out of
    /// this list.
    purged: Mutex<Vec<TrashItemId>>,
    /// When set, presence reads fail instead of reporting a presence fact.
    read_failure: Mutex<Option<String>>,
}

impl FakeTrashTransport {
    /// Compose over an explicit trash `root`, creating its `files/`/`info/`
    /// subdirectories.
    ///
    /// # Panics
    /// If `root` is (or is inside) a real user Trash directory, or if the
    /// layout cannot be created.
    #[must_use]
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        assert!(
            !is_real_trash_root(&root),
            "FakeTrashTransport refuses a real Trash root; inject a temp dir instead"
        );
        let inner = RealTrashTransport::new(&root)
            .expect("fake trash root must be creatable (inject a temp dir)");
        Self {
            inner,
            root,
            recorder: CallRecorder::new(),
            dispatches: Mutex::new(0),
            restore_conflicts: Mutex::new(Vec::new()),
            purged: Mutex::new(Vec::new()),
            read_failure: Mutex::new(None),
        }
    }

    /// Builder: script `item_id`'s restore as blocked by an occupied original
    /// path, refused before any move (OSC-011.4).
    #[must_use]
    pub fn restore_conflict(self, item_id: &TrashItemId) -> Self {
        self.restore_conflicts
            .lock()
            .expect("restore_conflicts mutex")
            .push(item_id.clone());
        self
    }

    /// Builder: make presence reads fail, proving an unreadable Trash never
    /// becomes a fabricated "absent".
    #[must_use]
    pub fn read_failure(self, reason: impl Into<String>) -> Self {
        *self.read_failure.lock().expect("read_failure mutex") = Some(reason.into());
        self
    }

    /// The injected trash root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every transport call label, in order.
    #[must_use]
    pub fn labels(&self) -> Vec<String> {
        self.recorder.labels()
    }

    /// How many mutating dispatches were attempted (a pre-effect refusal still
    /// counts as an attempt).
    #[must_use]
    pub fn dispatch_count(&self) -> usize {
        *self.dispatches.lock().expect("dispatches mutex")
    }

    /// Fixture setup: move `path` into the injected Trash *outside* the
    /// governed lifecycle, returning the assigned item id. Deliberately
    /// bypasses `AdmittedMutationContext` — this seeds an "already trashed"
    /// state, it is not the behaviour under test.
    ///
    /// # Panics
    /// If the move fails, or `path` does not exist.
    pub fn seed_trashed(&self, path: &Path) -> TrashItemId {
        match self.inner.trash_now(path).expect("fixture trash move") {
            TrashMoveOutcome::Done(item) => item.item_id,
            TrashMoveOutcome::PartialResidue { item, .. } => item.item_id,
        }
    }

    /// Model `delete_permanently`: erase the trashed payload and its metadata.
    ///
    /// **Irreversible.** Returns whether an entry was erased. There is no
    /// inverse method on this type, because the OS has no inverse either.
    pub fn purge_now(&self, item_id: &TrashItemId) -> bool {
        let payload = self.root.join("files").join(item_id.as_str());
        let info = self
            .root
            .join("info")
            .join(format!("{}.trashinfo", item_id.as_str()));
        let existed = payload.exists() || payload.symlink_metadata().is_ok() || info.exists();
        if payload.is_dir() {
            let _ = std::fs::remove_dir_all(&payload);
        } else {
            let _ = std::fs::remove_file(&payload);
        }
        let _ = std::fs::remove_file(&info);
        self.purged
            .lock()
            .expect("purged mutex")
            .push(item_id.clone());
        self.recorder.record("purge_now");
        existed
    }

    /// Whether `item_id` was permanently deleted through this fake.
    #[must_use]
    pub fn permanently_deleted(&self, item_id: &TrashItemId) -> bool {
        self.purged
            .lock()
            .expect("purged mutex")
            .iter()
            .any(|purged| purged == item_id)
    }

    /// The pre-mutation refusal for `item_id`, if any: a permanently deleted
    /// item can never be restored, and a scripted conflict is reported with the
    /// same error shape the real transport uses for a genuinely occupied path.
    fn restore_refusal(&self, item_id: &TrashItemId) -> Option<OsControlError> {
        if self.permanently_deleted(item_id) {
            self.recorder.record("restore_refused_permanently_deleted");
            return Some(unknown_trash_item_error());
        }
        let conflict = self
            .restore_conflicts
            .lock()
            .expect("restore_conflicts mutex")
            .iter()
            .any(|scripted| scripted == item_id);
        if conflict {
            self.recorder.record("restore_refused_target_occupied");
            return Some(occupied_restore_target_error());
        }
        None
    }

    fn read_fault(&self) -> Option<OsControlError> {
        self.read_failure
            .lock()
            .expect("read_failure mutex")
            .clone()
            .map(|reason| OsControlError::Unavailable {
                provider: Some(ProviderId::new(FAKE_TRASH_PROVIDER_ID)),
                reason: SafeText::new(format!("trash presence undetermined: {reason}")),
                retryable: true,
            })
    }
}

#[async_trait]
impl TrashTransport for FakeTrashTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(FAKE_TRASH_PROVIDER_ID)
    }

    async fn path_present(&self, path: &Path) -> Result<bool, OsControlError> {
        self.recorder.record("path_present");
        if let Some(fault) = self.read_fault() {
            return Err(fault);
        }
        self.inner.path_present(path).await
    }

    async fn item_present(&self, item_id: &TrashItemId) -> Result<bool, OsControlError> {
        self.recorder.record("item_present");
        if let Some(fault) = self.read_fault() {
            return Err(fault);
        }
        // A permanently deleted item is genuinely absent — that is a fact the
        // filesystem agrees with, since `purge_now` erased the metadata.
        self.inner.item_present(item_id).await
    }

    async fn find_latest_item_for_path(
        &self,
        original_path: &Path,
    ) -> Result<Option<TrashItem>, OsControlError> {
        self.recorder.record("find_latest_item_for_path");
        if let Some(fault) = self.read_fault() {
            return Err(fault);
        }
        self.inner.find_latest_item_for_path(original_path).await
    }

    async fn trash_path(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        path: &Path,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.recorder.record("trash_path");
        *self.dispatches.lock().expect("dispatches mutex") += 1;
        self.inner.trash_path(ctx, path).await
    }

    async fn restore_item(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        item_id: &TrashItemId,
        resolution: super::trash::RestoreResolution,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.recorder.record("restore_item");
        *self.dispatches.lock().expect("dispatches mutex") += 1;
        if let Some(refusal) = self.restore_refusal(item_id) {
            return Err(refusal);
        }
        self.inner.restore_item(ctx, item_id, resolution).await
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;
    use crate::os_control::testing::temp_dir;

    #[test]
    fn a_real_trash_root_is_refused() {
        assert!(is_real_trash_root(Path::new(
            "/mnt/data/.Trash-1000/files"
        )));
        assert!(is_real_trash_root(Path::new("/mnt/data/.Trash")));
        if let Some(home) = std::env::var_os("HOME") {
            let real = PathBuf::from(home).join(".local").join("share").join("Trash");
            assert!(
                is_real_trash_root(&real),
                "the user's real Trash must never be accepted as a fake root"
            );
        }
        let fixture = temp_dir();
        assert!(
            !is_real_trash_root(fixture.path()),
            "an injected temp root must be accepted"
        );
    }

    #[tokio::test]
    async fn trashing_records_the_dispatch_and_keeps_the_payload_under_the_injected_root() {
        let workspace = temp_dir();
        let trash_root = temp_dir();
        let target = workspace.path().join("doc.txt");
        std::fs::write(&target, b"hello").expect("fixture write");

        let fake = FakeTrashTransport::new(trash_root.path());
        let item_id = fake.seed_trashed(&target);

        assert!(!target.exists(), "the original path is emptied");
        assert!(trash_root
            .path()
            .join("files")
            .join(item_id.as_str())
            .exists());
        assert!(fake.item_present(&item_id).await.expect("ledger read"));
        assert_eq!(fake.root(), trash_root.path());
    }

    #[tokio::test]
    async fn permanent_deletion_is_irreversible_and_offers_no_restore() {
        let workspace = temp_dir();
        let trash_root = temp_dir();
        let target = workspace.path().join("secret-notes.txt");
        std::fs::write(&target, b"PLACEHOLDER-NOT-A-REAL-SECRET").expect("fixture write");

        let fake = FakeTrashTransport::new(trash_root.path());
        let item_id = fake.seed_trashed(&target);
        assert!(fake.item_present(&item_id).await.expect("present"));

        assert!(fake.purge_now(&item_id), "the entry existed before the purge");

        assert!(fake.permanently_deleted(&item_id));
        assert!(
            !fake.item_present(&item_id).await.expect("ledger read"),
            "a permanently deleted item is genuinely absent"
        );
        // The only restore path this fake has refuses, with the canonical
        // unknown-item error shape. There is no un-purge method to call.
        let refusal = fake
            .restore_refusal(&item_id)
            .expect("a purged item can never be restored");
        assert_eq!(refusal, unknown_trash_item_error());
    }

    #[test]
    fn a_scripted_occupied_target_is_a_conflict_not_an_overwrite() {
        let trash_root = temp_dir();
        let item_id = TrashItemId::new("report.txt");
        let fake = FakeTrashTransport::new(trash_root.path()).restore_conflict(&item_id);

        let refusal = fake
            .restore_refusal(&item_id)
            .expect("an occupied original path blocks the restore");
        assert_eq!(refusal, occupied_restore_target_error());
        assert!(matches!(refusal, OsControlError::InvalidRequest { .. }));
        assert_eq!(
            fake.dispatch_count(),
            0,
            "the refusal happens before any dispatch effect"
        );

        // An unrelated item is unaffected by the scripted conflict.
        assert!(fake
            .restore_refusal(&TrashItemId::new("other.txt"))
            .is_none());
    }

    #[tokio::test]
    async fn an_unreadable_trash_reports_a_fault_instead_of_absent() {
        let trash_root = temp_dir();
        let fake = FakeTrashTransport::new(trash_root.path()).read_failure("info dir unreadable");
        let err = fake
            .item_present(&TrashItemId::new("x"))
            .await
            .expect_err("an unreadable ledger must not read as absent");
        assert!(matches!(
            err,
            OsControlError::Unavailable { retryable: true, .. }
        ));
    }
}
