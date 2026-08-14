//! Task 0.4 — "Establish code-test safety rules" (OSC-033, OSC-034).
//!
//! Integration-level, host-safe proofs that the deny-live testing composition is
//! sound. This binary's `kria-core` dependency is compiled **without**
//! `cfg(test)`, so it exercises exactly the surface an ordinary integration test
//! sees — which is the case OSC-033.8 targets.
//!
//! Proven here:
//!   * no live constructor / `LiveHostAccessToken` minting symbol is reachable
//!     under `os-control-test` (deny-live composition, OSC-033.8);
//!   * the process-wide deny-live sentinel is armed and stays untouched while
//!     fake-backed suites run (no child process / live bus / session / device);
//!   * a mis-wired raw-transport attempt is caught by the sentinel;
//!   * a missing fake returns `Unavailable` instead of a live fallthrough;
//!   * the focused test-command manifest linter accepts the mandated invocations
//!     and rejects unsafe Cargo invocations.
//!
//! Gated behind `os-control-test` so it only compiles under the mandated
//! `--no-default-features --features os-control-test` invocation.
#![cfg(feature = "os-control-test")]

use std::path::PathBuf;

use kria_core::os_control::access::{self, RawTransportKind};
use kria_core::os_control::testing::{
    lint_test_command, ScriptedFake, TestCommandManifest, TestCommandViolation, TestingError,
    FAKE_RECEIPT_TAG,
};

// Compile-time proof that the live composition feature is OFF in this deny-live
// test binary. `LiveHostAccessToken::mint` only exists under `os-control-live`,
// so this guarantees no live token constructor is reachable from a
// completion-test binary (OSC-033.8). If both features were ever enabled the
// crate would already fail its own `compile_error!` guard.
const _: () = assert!(
    !cfg!(feature = "os-control-live"),
    "os-control-live must never be enabled alongside os-control-test"
);

#[test]
fn no_live_composition_or_token_minting_is_reachable() {
    // No live composition root ran, so no host-access token was ever minted.
    assert_eq!(
        access::live_composition_count(),
        0,
        "a completion-test build must never mint a LiveHostAccessToken"
    );
}

#[test]
fn deny_live_sentinel_is_armed_under_test_composition() {
    assert!(
        access::sentinel_is_armed(),
        "the deny-live sentinel must be armed under os-control-test"
    );
}

#[test]
#[serial_test::serial(os_control_sentinel)]
fn fake_backed_flow_records_calls_without_tripping_the_sentinel() {
    access::reset_trip_count();
    let baseline = access::sentinel_trip_count();

    // Drive a representative fake-backed observe → apply → verify flow.
    let fake: ScriptedFake<&str> = ScriptedFake::new();
    fake.push("observe_before", "unchanged");
    fake.push("apply_once", "applied");
    fake.push("observe_after", "changed");

    let before = fake
        .next("observe_before")
        .expect("scripted observe_before");
    assert!(before.is_fake());
    assert_eq!(before.tag, FAKE_RECEIPT_TAG);
    let applied = fake.next("apply_once").expect("scripted apply_once");
    assert_eq!(applied.payload, "applied");
    let after = fake.next("observe_after").expect("scripted observe_after");
    assert_eq!(after.payload, "changed");

    assert_eq!(
        fake.recorder().labels(),
        vec![
            "observe_before".to_string(),
            "apply_once".to_string(),
            "observe_after".to_string(),
        ]
    );

    // The fake path launched no child process and opened no live transport.
    assert_eq!(
        access::sentinel_trip_count(),
        baseline,
        "fake-backed flow must not trip the deny-live sentinel"
    );
}

#[test]
fn missing_fake_returns_unavailable_not_a_live_path() {
    let fake: ScriptedFake<u32> = ScriptedFake::new();
    let err = fake.next("apply_once").unwrap_err();
    assert_eq!(
        err,
        TestingError::Unavailable {
            operation: "apply_once".to_string()
        }
    );
}

#[test]
#[serial_test::serial(os_control_sentinel)]
fn sentinel_traps_a_raw_transport_attempt() {
    // Proves the tripwire actually fires: an armed sentinel must panic on any
    // raw-transport construction attempt. The panic message is suppressed to
    // keep test output clean, then we restore a zero trip count for any
    // subsequently ordered sentinel test.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(|| {
        access::deny_live_transport(RawTransportKind::Process);
    });
    std::panic::set_hook(prev_hook);

    assert!(
        result.is_err(),
        "an armed deny-live sentinel must panic on a raw transport attempt"
    );
    assert!(
        access::sentinel_trip_count() >= 1,
        "the sentinel must record the raw-transport violation"
    );
    access::reset_trip_count();
}

fn manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/os_control/test-commands.toml")
}

#[test]
fn focused_test_command_manifest_is_consistent_with_linter() {
    let text = std::fs::read_to_string(manifest_path()).expect("read test-command manifest");
    let manifest = TestCommandManifest::from_toml(&text).expect("parse test-command manifest");
    assert!(
        !manifest.allowed.is_empty(),
        "manifest must list at least one allowed invocation"
    );
    assert!(
        !manifest.rejected.is_empty(),
        "manifest must list at least one rejected invocation"
    );
    let problems = manifest.verify();
    assert!(
        problems.is_empty(),
        "test-command manifest disagrees with the linter: {problems:#?}"
    );
}

#[test]
fn linter_rejects_unsafe_cargo_invocations() {
    // Explicit spot checks independent of the manifest file.
    assert_eq!(
        lint_test_command("cargo test -p kria-core"),
        Err(TestCommandViolation::MissingTestFeature)
    );
    assert_eq!(
        lint_test_command(
            "cargo test -p kria-core --no-default-features --features os-control-test,os-control-live"
        ),
        Err(TestCommandViolation::DualComposition)
    );
    assert!(lint_test_command(
        "cargo test -p kria-core --no-default-features --features os-control-test"
    )
    .is_ok());
}
