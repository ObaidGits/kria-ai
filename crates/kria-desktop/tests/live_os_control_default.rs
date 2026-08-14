//! Guards that LIVE OS control is compiled into the default desktop build.
//!
//! # Why this test exists
//!
//! The owner's requirement is that running the app needs **no extra flag** for a
//! prompt to reach the laptop. That is expressed by one line in `Cargo.toml`
//! (`default = ["os-control-live"]`), and a single careless edit could remove it.
//! The failure would be silent and very confusing: the app would build, start,
//! accept commands, and answer "not available" to every one of them, with nothing
//! in the logs pointing at a missing feature.
//!
//! So the requirement is asserted here instead of trusted.

/// The live composition must be compiled into the default build.
#[test]
fn live_os_control_is_enabled_by_default() {
    assert!(
        cfg!(feature = "os-control-live"),
        "LIVE OS control is not compiled into the default desktop build. Restore \
         `default = [\"os-control-live\"]` in crates/kria-desktop/Cargo.toml — without it every OS \
         command answers `Unavailable` even though the app starts normally."
    );
}

/// The deny-live test composition must never be linked into the desktop app.
///
/// `os-control-test` arms a sentinel that panics on any real host access. If it
/// ever reached the shipped app, the first OS command would abort the process
/// rather than run.
#[test]
fn the_deny_live_test_composition_is_not_linked() {
    assert!(
        !cfg!(feature = "os-control-test"),
        "the deny-live test composition is linked into the desktop app; its sentinel would panic \
         on the first real OS action"
    );
}
