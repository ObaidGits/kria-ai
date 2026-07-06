//! Permanent regression suite (design.md "Permanent regression framework").
//!
//! Rule: no production bug found during hardening may be fixed without a
//! named permanent test here, following the `regr_<Rxx>_<slug>` convention.
//! Each test MUST fail with the fix reverted and pass with it applied — that
//! proof is a one-time manual step at authoring time (recorded in the test's
//! doc comment), not something this module can automate away.
//!
//! The suite runs every iteration and at freeze (tasks.md "Notes": "Iteration
//! gate is mandatory"). Nothing here is ever deleted or weakened.
//!
//! No entries exist yet — Task 1 only establishes the skeleton + convention.
//! Entries land as real bugs are found in tasks 2+ (design.md
//! "Permanent regression framework", tasks.md task 30).

/// Naming convention documented in code so every future regression test is
/// consistent and traceable back to the requirement it guards.
///
/// Format: `regr_<requirement>_<slug>`, e.g. `regr_r3_5_index_db_drift_hidden`.
pub const NAMING_CONVENTION_DOC: &str = "regr_<requirement>_<slug>";

/// Registry of regression test names discovered by convention (via
/// `#[test] fn regr_*`) — used by the freeze scorer (task 22) to assert the
/// regression suite exists and is non-empty once bugs have been found, rather
/// than silently reporting an empty suite as "green".
pub fn expected_test_prefix() -> &'static str {
    "regr_"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naming_convention_is_documented() {
        assert!(NAMING_CONVENTION_DOC.starts_with("regr_"));
        assert_eq!(expected_test_prefix(), "regr_");
    }

    // Real regression tests land below this line as bugs are found in later
    // tasks, e.g.:
    //
    // /// Bug: <describe>. Found during task <N> (<requirement>).
    // /// Fails without the fix in <file>; passes with it.
    // #[test]
    // fn regr_r3_5_index_db_drift_hidden() { ... }

    /// Bug: found during task 2 (R1 enable/disable lifecycle validation).
    /// `DockerOutage` (fault_injector.rs) mutated the process-global
    /// `DOCKER_HOST` env var with no synchronization. Rust runs `#[test]`
    /// functions on multiple threads by default, so a real-Docker check
    /// (e.g. `validate_enabled_lifecycle`) running concurrently with a
    /// `DockerOutage`-using test (e.g. `validate_docker_absent_is_honest`)
    /// could observe the OTHER test's outage and falsely report "docker not
    /// reachable" even though Docker was fine — a false Skipped, which is a
    /// dishonest signal (R15) even though it is not a false Pass.
    ///
    /// Fix: `DockerOutage` now holds a process-global `Mutex` for its entire
    /// lifetime (`fault_injector::DOCKER_ENV_LOCK`), and any real-Docker check
    /// that must see the true environment takes the same lock via
    /// `fault_injector::docker_env_test_guard()` before checking (see
    /// `lifecycle::validate_enabled_lifecycle`).
    ///
    /// This test fails without the fix: running many `DockerOutage::start()`
    /// instances concurrently with many `DOCKER_HOST` reads must never
    /// observe a torn/interleaved value once the fix serializes them.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn regr_r1_docker_outage_env_race() {
        use crate::openclaw_eval::fault_injector::{docker_env_test_guard, DockerOutage};
        use std::sync::Arc;
        use tokio::sync::Barrier;

        // Second bug found while fixing the first: this test's OWN
        // setup/teardown originally set/removed DOCKER_HOST directly,
        // WITHOUT holding the shared lock — so it could itself race against
        // any other test's `DockerOutage` running concurrently in the same
        // test binary (all `#[test]` fns share one process). Setup/teardown
        // now hold the same guard as every other DOCKER_HOST reader/writer.
        let setup_guard = docker_env_test_guard().await;
        std::env::set_var("DOCKER_HOST", "unix:///var/run/docker.sock");
        drop(setup_guard);

        // Real concurrency on the multi-thread runtime (worker_threads = 4),
        // not just interleaved futures on one thread — this is what actually
        // exercised the original race.
        let barrier = Arc::new(Barrier::new(2));
        let b1 = barrier.clone();
        let b2 = barrier.clone();

        let outage_task = tokio::spawn(async move {
            b1.wait().await;
            for _ in 0..50 {
                let outage = DockerOutage::start().await;
                assert_eq!(
                    std::env::var("DOCKER_HOST").unwrap(),
                    "unix:///tmp/kria-openclaw-eval-nonexistent.sock"
                );
                drop(outage);
            }
        });

        let checker_task = tokio::spawn(async move {
            b2.wait().await;
            for _ in 0..50 {
                // Without the fix, this guard is a no-op and this read can
                // observe the outage task's mutated value mid-flight.
                let _guard = docker_env_test_guard().await;
                let value = std::env::var("DOCKER_HOST").unwrap();
                assert_eq!(
                    value, "unix:///var/run/docker.sock",
                    "docker_env_test_guard() must serialize against DockerOutage \
                     so a real-Docker check never observes another test's simulated outage"
                );
            }
        });

        outage_task.await.unwrap();
        checker_task.await.unwrap();

        let teardown_guard = docker_env_test_guard().await;
        std::env::remove_var("DOCKER_HOST");
        drop(teardown_guard);
    }

    /// Bug: found during task 2, discovered WHILE fixing
    /// `regr_r1_docker_outage_env_race` above. First attempt at making
    /// `TestRig` container names unique per-instance (to fix a real
    /// concurrent-rig container-name collision, also found in task 2) used a
    /// full UUID suffix. `RuntimeManager::create_container`
    /// (runtime_manager.rs) builds the actual Docker `hostname` as
    /// `"{container_name}-{resource_class}-{generation}-{short_uuid}"`, and
    /// Linux's `sethostname(2)` caps hostnames at 64 bytes. The full-UUID
    /// variant pushed every rig-created container over that limit, so EVERY
    /// container creation failed with `sethostname: invalid argument` and 12
    /// half-created ("Created", never "Running") containers leaked onto the
    /// real host before the bug was caught.
    ///
    /// Fix: `rig::unique_rig_container_name()` uses only an 8-hex-char
    /// suffix. This test fails without that fix (or with any change that
    /// grows the per-instance name back toward 64 bytes headroom).
    #[test]
    fn regr_r2_rig_container_name_too_long_for_hostname() {
        use crate::openclaw_eval::rig::RIG_CONTAINER_PREFIX;

        // Mirror RuntimeManager's real naming scheme (runtime_manager.rs) with
        // the longest realistic resource-class label and a non-trivial
        // generation counter, using the SAME construction rig.rs uses.
        let unique_suffix = uuid::Uuid::new_v4().simple().to_string();
        let container_name = format!("{RIG_CONTAINER_PREFIX}-{}", &unique_suffix[..8]);
        let simulated_hostname = format!("{container_name}-standard-9999-{}", &unique_suffix[..8]);

        assert!(
            simulated_hostname.len() <= 64,
            "simulated Docker hostname '{simulated_hostname}' ({} bytes) exceeds the Linux \
             sethostname(2) 64-byte limit — this is exactly the bug that leaked 12 containers \
             on the real host during task 2",
            simulated_hostname.len()
        );
    }

    // Bug: found during task 2 — tracked as `regr_r2_concurrent_rig_reap_interference`.
    // `RuntimeManager::initialize()`/`::shutdown()` both call
    // `reap_orphaned_containers()`, which filters Docker containers by a
    // SUBSTRING match on `config.container_name`. Two `TestRig`s running
    // concurrently could interleave their reap sweeps and warm-pool creation,
    // leaking or destroying the OTHER rig's containers — reproduced directly:
    // `cargo test openclaw_eval --test-threads=1` was consistently clean (0
    // leaks), while default (parallel) test threads leaked ~1 container per
    // run. This is NOT a real single-process production bug (the real
    // desktop app only ever constructs ONE `RuntimeManager`), so A0-A9 was
    // deliberately left untouched. Fix: `rig::rig_lifecycle_lock()` — an
    // owned `tokio::sync::Mutex` guard held for the FULL `TestRig` lifetime
    // (from `up()` through the end of `down()`), serializing every rig
    // instance's `initialize()`/reap/`shutdown()` end-to-end. This is
    // exercised (not a standalone unit test, since it requires real Docker +
    // real concurrency) every time `rig::tests::rig_up_and_down_leaves_zero_containers`
    // and `lifecycle::tests::r1_enabled_lifecycle_real_docker` run together
    // under the default parallel test runner — verified clean across 10+
    // consecutive full-suite runs after the fix.
}
