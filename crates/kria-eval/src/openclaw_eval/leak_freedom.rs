//! Property 12 — leak-freedom (ICP spec `openclaw-icp`, tasks.md task 10.6).
//!
//! **Property 12: Leak-freedom** — after completed / failed / cancelled runs,
//! rig-owned container and pool-lease counts return to their pre-run baseline
//! (no leaked containers, no leaked leases). **Validates: Requirements 4.1.**
//!
//! Real-code grounding (verified by reading `runtime/docker.rs`, not assumed):
//! `DockerRuntime::execute` runs the frozen lifecycle
//! `admit → checkout/create → exec → call → (cancel|complete|fail) → cleanup`
//! and the cleanup stage **ALWAYS runs** — `destroy` for bespoke containers or
//! `checkin` for warm-pool containers, and the HRA `_lease` drops at scope end —
//! on the completed, failed, AND cancelled paths (the `tokio::select!` cancel
//! arm falls through to the same cleanup block). This module proves that frozen
//! guarantee end-to-end against REAL Docker for all three terminal outcomes,
//! using the existing `TestRig` + frozen `leak_detector`. CIL never touches
//! containers; the frozen runtime does — so this test drives the real
//! `DockerRuntime` (via `build_runtime_registry`), exactly the engine's path.
//!
//! GATING: this test REQUIRES real Docker and is therefore `#[ignore]`d so it
//! COMPILES in CI but never runs there. It executes in the nightly/live gate
//! (see `.github/workflows/n8n-live-nightly.yml` / `.devin/workflows/run-live-stress.md`)
//! via `cargo test -p kria-eval -- --ignored`. It also skips honestly (never a
//! fabricated pass) if Docker is unreachable when it IS run, mirroring the rest
//! of the harness (R1.3, R15 honesty invariant).
//!
//! Property-based coverage note: a full proptest generator over arbitrary
//! skills is impractical here — each case spins a real container, so the input
//! space is deliberately the three *terminal outcomes* of the frozen lifecycle
//! (the axis Property 12 actually quantifies over). The test is table-driven
//! over `{Completed, Failed, Cancelled}`, asserting baseline restoration after
//! each, which is the essence of Property 12.

/// The three terminal run outcomes Property 12 quantifies over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// A matched, valid skill invocation that runs to success.
    Completed,
    /// An invocation that reaches the container but fails at the app level
    /// (unknown tool) — exercises the `Stage::Failed` cleanup path.
    Failed,
    /// An invocation cancelled in-flight — exercises the `Stage::Cancelled`
    /// cleanup path (cancellation propagation preserved).
    Cancelled,
}

impl RunOutcome {
    /// Every terminal outcome must restore the pre-run baseline.
    pub const ALL: [RunOutcome; 3] = [
        RunOutcome::Completed,
        RunOutcome::Failed,
        RunOutcome::Cancelled,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw_eval::leak_detector;
    use crate::openclaw_eval::rig::{verify_docker_reachable, TestRig};
    use kria_core::openclaw::runtime::build_runtime_registry;
    use kria_core::openclaw::runtime::{LaunchSpec, RuntimeContext, RuntimeKind};
    use kria_core::openclaw::types::ResourceClass;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    /// Build the `LaunchSpec` for a given outcome. Uses the bundled
    /// `oc_calculator` skill for the reachable paths (Completed/Cancelled) and
    /// a deliberately-unknown skill id for the Failed path (guaranteed
    /// app-level failure that still goes through the full checkout→exec→cleanup
    /// lifecycle, so a leak would still show up).
    fn spec_for(outcome: RunOutcome, correlation_id: &str) -> LaunchSpec {
        let (skill_id, params) = match outcome {
            RunOutcome::Completed | RunOutcome::Cancelled => (
                "oc_calculator".to_string(),
                serde_json::json!({ "expression": "5 * 5" }),
            ),
            RunOutcome::Failed => (
                // No such tool is advertised by the substrate → the MCP
                // `tools/call` returns an error → `ToolResult::err` →
                // `Stage::Failed`, but the container is still checked back in.
                "oc_no_such_skill_leak_probe".to_string(),
                serde_json::json!({}),
            ),
        };
        LaunchSpec {
            skill_id,
            params,
            resource_class: ResourceClass::Light,
            timeout: Duration::from_secs(30),
            correlation_id: correlation_id.to_string(),
            grants: Vec::new(),
            mounted_skill_dir: None,
        }
    }

    /// Property 12 (real Docker): for each of completed / failed / cancelled,
    /// the rig container count and active leases return to baseline after the
    /// frozen `DockerRuntime` finishes the run.
    ///
    /// `#[ignore]`d: requires real Docker. Runs in the nightly/live gate via
    /// `cargo test -p kria-eval -- --ignored`. **Validates: Requirements 4.1.**
    #[tokio::test]
    #[ignore = "requires real Docker; runs in the nightly/live gate (cargo test -- --ignored)"]
    async fn prop12_leak_freedom_completed_failed_cancelled_return_to_baseline() {
        if verify_docker_reachable().await.is_err() {
            eprintln!(
                "SKIPPED (Outcome::Skipped, not Pass): docker not reachable in this environment"
            );
            return;
        }

        let rig = TestRig::up()
            .await
            .expect("rig must come up against real Docker");
        let runtimes = build_runtime_registry(rig.pool.clone());
        let runtime = runtimes
            .get(RuntimeKind::Docker)
            .expect("docker runtime must be registered");

        for outcome in RunOutcome::ALL {
            // Capture the pre-run baseline (rig containers + active leases +
            // warm count) via the frozen leak detector.
            let baseline = leak_detector::baseline(&rig.pool)
                .await
                .expect("baseline snapshot must succeed");

            let spec = spec_for(outcome, &format!("prop12-{outcome:?}"));

            // For the cancelled path, hand the runtime a pre-cancelled token:
            // the `biased` `tokio::select!` in `DockerRuntime::execute` takes
            // the cancellation arm deterministically, driving the run to
            // `Stage::Cancelled` while STILL running the shared cleanup block.
            let ctx = match outcome {
                RunOutcome::Cancelled => {
                    let token = CancellationToken::new();
                    token.cancel();
                    RuntimeContext {
                        cancellation: token,
                    }
                }
                _ => RuntimeContext::detached(),
            };

            let result = runtime.execute(spec, ctx).await;

            match outcome {
                RunOutcome::Completed => assert!(
                    result.success,
                    "Completed run must succeed (bundled oc_calculator): {result:?}"
                ),
                RunOutcome::Failed | RunOutcome::Cancelled => assert!(
                    !result.success,
                    "{outcome:?} run must report a non-success ToolResult: {result:?}"
                ),
            }

            // Property 12: every terminal outcome returns to baseline — 0
            // leaked containers, 0 leaked leases.
            leak_detector::assert_returned_to(&rig.pool, baseline)
                .await
                .unwrap_or_else(|e| {
                    panic!("Property 12 leak-freedom violated after {outcome:?} run: {e}")
                });
        }

        // Teardown itself asserts 0 rig containers remain (frozen invariant).
        rig.down()
            .await
            .expect("rig teardown must leave 0 leaked containers");
    }

    /// CI-safe (no Docker): the outcome table Property 12 quantifies over
    /// covers exactly the three terminal lifecycle stages, so the property is
    /// not accidentally narrowed to a single happy path.
    #[test]
    fn prop12_covers_all_three_terminal_outcomes() {
        assert_eq!(RunOutcome::ALL.len(), 3);
        assert!(RunOutcome::ALL.contains(&RunOutcome::Completed));
        assert!(RunOutcome::ALL.contains(&RunOutcome::Failed));
        assert!(RunOutcome::ALL.contains(&RunOutcome::Cancelled));
    }
}
