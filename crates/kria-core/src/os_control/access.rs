//! Live host-access seam and the process-wide deny-live transport sentinel.
//!
//! linux-os-control-production **Task 0.4** — "Establish code-test safety rules"
//! (OSC-033, OSC-034), design §2 invariant 14 and §18.
//!
//! # Why this module exists
//!
//! Completion tests for this spec must be able to run inside the owner's live
//! desktop session **without any observable OS mutation**. Two independent code
//! seams enforce that, and both live here so they compile in every feature
//! configuration (they are referenced by future live transports in
//! `os_control::linux::*`, which are compiled regardless of the test feature):
//!
//! 1. [`LiveHostAccessToken`] — an unforgeable capability token that live
//!    provider/transport construction will require to borrow. It can be minted
//!    **only** by the desktop/server startup composition roots, which enable the
//!    `os-control-live` feature. Under `os-control-test` the minting constructor
//!    does not exist and the token's only field is private, so an integration
//!    test (whose `kria-core` dependency is *not* compiled with `cfg(test)`)
//!    cannot construct or inject live host access. This satisfies OSC-033.8.
//!
//! 2. The **deny-live sentinel** — a process-wide tripwire that every raw
//!    process / bus / session / Polkit / secret / device transport constructor
//!    must call via [`deny_live_transport`]. When the sentinel is *armed* (the
//!    default under `os-control-test`) the call panics, proving that a
//!    fake-backed suite launched no child process and opened no live transport.
//!
//! Live providers/transports do not exist yet (they arrive with Tasks 1.x). This
//! module establishes the seam so those constructors have exactly one gate to
//! call and exactly one token type to require.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Unforgeable proof that construction is happening inside a live composition
/// root (desktop/server startup), not a test.
///
/// Live OS provider/transport constructors (Tasks 1.x) will take
/// `&LiveHostAccessToken` by borrow. The type is deliberately:
///
/// * **non-literal-constructible outside this crate** — its only field is
///   private, so no downstream crate or test can write `LiveHostAccessToken {}`;
/// * **mintable only under `os-control-live`** — [`LiveHostAccessToken::mint`]
///   exists solely when that feature is enabled, and that feature is mutually
///   exclusive with `os-control-test` (enforced by a crate-level
///   `compile_error!`). Therefore no completion-test binary can obtain one.
#[derive(Debug)]
pub struct LiveHostAccessToken {
    /// Private unit seal: prevents struct-literal construction from any other
    /// crate or module (including test crates), so the only way to obtain a
    /// token is [`LiveHostAccessToken::mint`], which is `os-control-live`-only.
    _seal: (),
}

#[cfg(feature = "os-control-live")]
impl LiveHostAccessToken {
    /// Mint the live host-access token.
    ///
    /// Only compiled under the `os-control-live` feature, which is owned by the
    /// desktop/server startup composition roots. Because `os-control-live` and
    /// `os-control-test` are mutually exclusive, this function is guaranteed
    /// absent from every completion-test build.
    #[must_use]
    pub fn mint() -> Self {
        note_live_composition();
        Self { _seal: () }
    }
}

/// Kinds of raw host transport whose constructors must call
/// [`deny_live_transport`] before touching the live system. Naming the kind
/// keeps the sentinel panic message specific and auditable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RawTransportKind {
    /// A child process launch (`std::process::Command`, `ExecWrapper`, …).
    Process,
    /// A live D-Bus system-bus connection.
    SystemBus,
    /// A live D-Bus session-bus connection.
    SessionBus,
    /// A `logind`/session-manager transport.
    Session,
    /// A Polkit authority transport.
    Polkit,
    /// A Secret Service transport.
    Secret,
    /// A raw device handle (input, display, audio, storage, …).
    Device,
    /// A native process-signal syscall (`kill(2)`, `setpriority(2)`) against
    /// an existing process — distinct from [`Self::Process`] (which launches
    /// a *new* child process).
    ProcessSignal,
}

impl RawTransportKind {
    /// Stable human label used in the sentinel panic message.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            RawTransportKind::Process => "child process",
            RawTransportKind::SystemBus => "system D-Bus",
            RawTransportKind::SessionBus => "session D-Bus",
            RawTransportKind::Session => "logind session",
            RawTransportKind::Polkit => "Polkit authority",
            RawTransportKind::Secret => "Secret Service",
            RawTransportKind::Device => "raw device",
            RawTransportKind::ProcessSignal => "process signal syscall",
        }
    }
}

/// Process-wide "armed" flag for the deny-live sentinel.
///
/// Armed by default whenever the crate is built with `os-control-test`, so the
/// mere act of running the test suite guards every raw transport. In non-test
/// compositions it defaults disarmed (live code is allowed to open transports).
static SENTINEL_ARMED: AtomicBool = AtomicBool::new(cfg!(feature = "os-control-test"));

/// Count of raw-transport construction attempts observed while armed. A healthy
/// deny-live suite finishes with this at zero.
static SENTINEL_TRIPS: AtomicUsize = AtomicUsize::new(0);

/// Count of live composition roots that minted a [`LiveHostAccessToken`]. Used
/// by live builds for observability; always zero under `os-control-test`.
static LIVE_COMPOSITIONS: AtomicUsize = AtomicUsize::new(0);

/// Gate that every raw host-transport constructor MUST call before opening a
/// live handle.
///
/// * When the sentinel is armed (default under `os-control-test`): records the
///   violation and **panics**, aborting the offending test with a message that
///   names the transport kind. This is what proves a fake-backed suite performs
///   no live child/bus/session/device access.
/// * When disarmed (live composition): a cheap no-op that returns immediately.
pub fn deny_live_transport(kind: RawTransportKind) {
    if SENTINEL_ARMED.load(Ordering::SeqCst) {
        SENTINEL_TRIPS.fetch_add(1, Ordering::SeqCst);
        panic!(
            "deny-live sentinel tripped: attempted to open a live {} transport under a \
             deny-live (os-control-test) composition; completion tests must use fakes/fixtures \
             and never touch the live system",
            kind.label()
        );
    }
}

/// Record that a live composition root minted a host-access token. Only invoked
/// by [`LiveHostAccessToken::mint`], which exists solely under `os-control-live`.
#[cfg_attr(not(feature = "os-control-live"), allow(dead_code))]
fn note_live_composition() {
    LIVE_COMPOSITIONS.fetch_add(1, Ordering::SeqCst);
}

/// Whether the deny-live sentinel is currently armed.
#[must_use]
pub fn sentinel_is_armed() -> bool {
    SENTINEL_ARMED.load(Ordering::SeqCst)
}

/// How many raw-transport constructions were observed while armed (violations).
#[must_use]
pub fn sentinel_trip_count() -> usize {
    SENTINEL_TRIPS.load(Ordering::SeqCst)
}

/// Number of live compositions that minted a host-access token (0 under test).
#[must_use]
pub fn live_composition_count() -> usize {
    LIVE_COMPOSITIONS.load(Ordering::SeqCst)
}

// ── Test-only controls ──────────────────────────────────────────────────────
// Exposed only under `os-control-test` so the sentinel's own behaviour can be
// exercised deterministically without leaking arm/disarm controls into any live
// build. Callers must serialize (`serial_test`) because the state is global.
#[cfg(feature = "os-control-test")]
mod test_controls {
    use super::{Ordering, SENTINEL_ARMED, SENTINEL_TRIPS};

    /// Disarm the sentinel and return a guard that re-arms + resets the trip
    /// counter on drop. Lets a test call [`super::deny_live_transport`] to prove
    /// the tripwire behaviour without polluting other tests' trip count.
    #[must_use]
    pub fn scoped_disarm() -> SentinelDisarmGuard {
        SENTINEL_ARMED.store(false, Ordering::SeqCst);
        SentinelDisarmGuard { _private: () }
    }

    /// RAII guard that restores the armed state and clears the trip counter.
    pub struct SentinelDisarmGuard {
        _private: (),
    }

    impl Drop for SentinelDisarmGuard {
        fn drop(&mut self) {
            SENTINEL_TRIPS.store(0, Ordering::SeqCst);
            SENTINEL_ARMED.store(true, Ordering::SeqCst);
        }
    }

    /// Reset the trip counter to zero (armed state left unchanged).
    pub fn reset_trip_count() {
        SENTINEL_TRIPS.store(0, Ordering::SeqCst);
    }
}

#[cfg(feature = "os-control-test")]
pub use test_controls::{reset_trip_count, scoped_disarm, SentinelDisarmGuard};

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn sentinel_is_armed_by_default_under_test_feature() {
        assert!(
            sentinel_is_armed(),
            "os-control-test builds must arm the deny-live sentinel by default"
        );
    }

    #[test]
    fn raw_transport_labels_are_stable() {
        assert_eq!(RawTransportKind::Process.label(), "child process");
        assert_eq!(RawTransportKind::SystemBus.label(), "system D-Bus");
        assert_eq!(RawTransportKind::Device.label(), "raw device");
    }
}
