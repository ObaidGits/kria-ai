//! Files domain: Trash lifecycle, archives, and bound-path ownership changes
//! (design §3, §9.1, §10.1).
//!
//! linux-os-control-production **Task 3.1** — "Complete files, Trash,
//! restore, permanent delete and archives" (OSC-010, OSC-011).
//!
//! # Scope
//!
//! `tools/file_ops.rs`'s existing `read_file`/`write_file`/`list_directory`/
//! `copy_file`/`rename_file`/`move_file` already call plain `std::fs` — no
//! shell, no subprocess — so OSC-002 was already satisfied there (Task 2.5
//! migrated the *process/clipboard/notification* subprocess call sites, not
//! these). This module closes the remaining OSC-010/OSC-011 gap: there was
//! previously **no Trash tool, no permanent-delete-as-distinct-action, and no
//! archive support at all**. This module adds:
//!
//! * [`trash`] — the freedesktop.org Trash specification (`files/` +
//!   `info/*.trashinfo`) behind [`trash::TrashControl`], so `trash_file`
//!   (the new default-delete path) and `restore_from_trash` never call
//!   `std::fs::remove_file`/`remove_dir_all` directly. `delete_file`/
//!   `delete_permanently` remain the distinct, explicitly-worded RED
//!   permanent path (never routed to by default).
//! * [`archive`] — bounded zip create/list/extract behind
//!   [`archive::ArchiveControl`], with entry-count/expanded-byte/
//!   compression-ratio/traversal limits and staged-then-verified extraction
//!   (OSC-011.5, OSC-011.6).
//! * [`ownership`] — `set_file_ownership` behind
//!   [`ownership::OwnershipControl`], which dispatches **exclusively**
//!   through the existing typed `BrokerOperation::SetBoundPathOwnership`
//!   (Task 1.5) — never a raw `chown` subprocess.
//!
//! # Why no deny-live transport gating here
//!
//! Unlike the D-Bus/subprocess/device domains under `os_control::linux::*`,
//! Trash and archive operations are **plain `std::fs`** against an
//! *injectable* root (the Trash directory, or the archive
//! source/destination/staging paths) — never a live bus connection, child
//! process, or raw device handle. There is nothing here for
//! [`crate::os_control::access::deny_live_transport`] to guard: a completion
//! test gets host safety by injecting a `tempfile::TempDir` as the Trash root
//! (never `dirs::data_dir()`) and by only ever naming paths under that
//! tempdir or another tempdir, exactly as `OSC-010.7` ("provider tests SHALL
//! use temporary directories only") requires. This mirrors the task's own
//! test-strategy note: "file_ops-style domains don't need a fake-transport
//! abstraction the way D-Bus/subprocess domains do... real std::fs against a
//! tempdir is the correct test strategy here." The one operation in this
//! module that *does* reach a live system boundary — [`ownership`]'s Polkit
//! broker dispatch — reuses the already deny-live-gated
//! [`crate::os_control::broker`] transport seam rather than inventing a new
//! one.

/// Direct file mutations: permissions, append, permanent delete.
pub mod attributes;
pub mod archive;
pub mod ownership;
pub mod trash;

/// Deny-live fake Trash transport (Task 0.4 / OSC-033); test composition only.
#[cfg(feature = "os-control-test")]
pub mod fake;

use crate::os_control::contract::Digest;

/// Compute the canonical path-identity digest used to bind grants/resources/
/// receipts to an exact path (design §10.4 `canonical-path-identity`).
/// Deliberately simple and dependency-free: a digest of the path's lossily
/// UTF-8-rendered string. This binds *identity*, not filesystem content, so
/// unlike a content hash it stays cheap to compute on every observation.
#[must_use]
pub fn canonical_path_identity(path: &std::path::Path) -> Digest {
    Digest::of_str(&path.to_string_lossy())
}

pub use archive::{
    create_archive_result, extract_archive_result, list_archive_result, validate_entry_bounds,
    validate_entry_path, ArchiveBoundsViolation, ArchiveControl, ArchiveControlPort, ArchiveEntry,
    ArchiveEntryPage, ArchiveFocus, ArchiveFormat, ArchiveMutationResult, ArchiveOp,
    ArchiveRequest, ArchiveState, ArchiveTransport, RealArchiveTransport, ARCHIVE_PROVIDER_ID,
    MAX_ARCHIVE_ENTRIES, MAX_ARCHIVE_EXPANDED_BYTES, MAX_ARCHIVE_INPUT_ENTRIES,
    MAX_COMPRESSION_RATIO, MAX_ENTRY_EXPANDED_BYTES,
};
pub use ownership::{
    set_file_ownership_result, OwnershipControl, OwnershipControlPort, OwnershipRequest,
    OwnershipState, RealOwnershipTransport, OWNERSHIP_PROVIDER_ID,
};
pub use trash::{
    occupied_restore_target_error, restore_trash_item_result, trash_file_result,
    unknown_trash_item_error, RealTrashTransport, RestoreMoveOutcome, RestoreResolution,
    TrashControl, TrashControlPort, TrashItem, TrashItemId, TrashMoveOutcome, TrashOp,
    TrashRequest, TrashState, TrashTransport, TRASH_PROVIDER_ID,
};



#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn canonical_path_identity_is_stable_and_distinguishes_paths() {
        let a = canonical_path_identity(Path::new("/tmp/a"));
        let b = canonical_path_identity(Path::new("/tmp/a"));
        let c = canonical_path_identity(Path::new("/tmp/b"));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
