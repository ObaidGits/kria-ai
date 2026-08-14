//! Live files-domain composition helpers (design §3, §9.1).
//!
//! linux-os-control-production **Task 3.1** (OSC-010, OSC-011).
//!
//! Unlike the D-Bus/subprocess/device adapters elsewhere under
//! `linux::providers::*`, Trash and archive operations are plain `std::fs`
//! against an injectable root/path — never a live bus connection, child
//! process, or raw device handle (see `os_control::files` module docs for the
//! full rationale). So [`RealTrashTransport`]/[`RealArchiveTransport`] need
//! no [`LiveHostAccessToken`]-gated wrapper here; a live composition root
//! constructs them directly with [`live_trash_root`].
//!
//! Ownership changes are the one operation in this domain that reaches a
//! privileged live boundary — the Polkit broker — so [`LiveFileOwnershipBroker`]
//! *is* gated behind a [`LiveHostAccessToken`]: it composes
//! [`RealOwnershipTransport`] over the already deny-live-gated
//! [`crate::os_control::broker::LiveBrokerTransport`].

use std::path::PathBuf;

use crate::os_control::access::LiveHostAccessToken;
use crate::os_control::broker::LiveBrokerTransport;
use crate::os_control::files::RealOwnershipTransport;

/// Resolve the real freedesktop.org Trash root for the current user
/// (`$XDG_DATA_HOME/Trash`, defaulting to `~/.local/share/Trash`). Live
/// composition roots pass this to [`crate::os_control::files::RealTrashTransport::new`];
/// tests must never call this function and must always inject a
/// `tempfile::TempDir` path instead (OSC-010.7).
#[must_use]
pub fn live_trash_root() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("Trash")
}

/// The live ownership transport, composed over the live Polkit broker
/// transport. Constructible only in a live composition root; a value cannot
/// exist under `os-control-test` because [`LiveBrokerTransport::new`]
/// requires a [`LiveHostAccessToken`].
pub type LiveFileOwnershipBroker = RealOwnershipTransport<LiveBrokerTransport>;

/// Construct the live ownership transport. Requires a [`LiveHostAccessToken`],
/// so no completion test can build one.
#[must_use]
pub fn live_file_ownership_broker(token: &LiveHostAccessToken) -> LiveFileOwnershipBroker {
    RealOwnershipTransport::new(LiveBrokerTransport::new(token))
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn live_trash_root_ends_with_trash_segment() {
        let root = live_trash_root();
        assert_eq!(root.file_name().and_then(|n| n.to_str()), Some("Trash"));
    }
}
