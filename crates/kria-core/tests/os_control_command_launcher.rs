//! Code-level proof of the governed command launcher (design §4/§8, OSC-005,
//! OSC-007, OSC-016).
//!
//! These tests exercise the launcher's own contract — the before/after-spawn
//! split, output bounding, the hermetic environment, deadline and cancellation
//! handling — using **hermetic throwaway children** (`/bin/echo`, `/bin/false`,
//! `/bin/sh -c` loops, `/bin/sleep`). Nothing here changes a single setting on the
//! host: no brightness, no volume, no network, no packages. The subject under
//! test is process handling, not OS state.
//!
//! The deny-live sentinel is armed under `os-control-test`, so each test takes
//! `scoped_disarm()` and is `#[serial]` — the sentinel is global state, and the
//! guard re-arms it and clears the trip counter on drop.

use std::time::{Duration, Instant};

use serial_test::serial;
use tokio_util::sync::CancellationToken;

use kria_core::agent::execution_gate::OsActionGrant;
use kria_core::agent::resource_lease::ResourceLeaseManager;
use kria_core::agent::turn_memory::ExecutionTarget;
use kria_core::os_control::access::scoped_disarm;
use kria_core::os_control::context::RedactionPolicy;
use kria_core::os_control::contract::{ActionId, CapabilityId, CorrelationId, Digest, SnapshotRevision};
use kria_core::os_control::governed::{OsCallRequest, OsGovernedCall};
use kria_core::os_control::linux::structured_command::{
    CommandPlan, CommandPolicy, SecretStdin, StructuredCommandRequest, TrustedExecutable,
};
use kria_core::os_control::receipt::{ApplyOutcome, UncertainEffectCause};
use kria_core::os_control::resource::{os_write_requirements, OsResourceCoordinator};
use kria_core::os_control::runtime::OsControlRuntime;
use kria_core::os_control::OsAuditStore;
use kria_core::safety::RiskLevel;

const SESSION: &str = "launcher-session";

/// Everything a sealed request needs, kept alive together — the sealed context
/// borrows from the call, the grant and the runtime.
struct Harness {
    audit: OsAuditStore,
    coordinator: OsResourceCoordinator,
    runtime: OsControlRuntime,
    params: serde_json::Value,
    cancellation: CancellationToken,
}

impl Harness {
    fn new() -> Self {
        Self {
            audit: OsAuditStore::open_in_memory(),
            coordinator: OsResourceCoordinator::new(ResourceLeaseManager::new()),
            runtime: OsControlRuntime::detached(),
            params: serde_json::json!({ "probe": true }),
            cancellation: CancellationToken::new(),
        }
    }

    async fn call(&self, deadline: Instant) -> OsGovernedCall {
        let tool = "set_brightness";
        let request = OsCallRequest {
            session_id: SESSION,
            correlation_id: CorrelationId::new("corr-launch"),
            action_id: ActionId::new("act-launch"),
            action: tool,
            params: &self.params,
            target: ExecutionTarget::Host,
            risk: RiskLevel::Yellow,
            requirements: os_write_requirements(tool, &self.params),
            snapshot_revision: SnapshotRevision(1),
            cancellation: self.cancellation.clone(),
            deadline,
            redaction: RedactionPolicy::default(),
            snapshot: None,
        };
        let grant = OsActionGrant::for_test(
            SESSION,
            tool,
            &self.params,
            ExecutionTarget::Host,
            &os_write_requirements(tool, &self.params),
            RiskLevel::Yellow,
        );
        OsGovernedCall::admit(&self.audit, &self.coordinator, grant, request)
            .await
            .expect("admission succeeds")
    }
}

/// Build a sealed request for `program` with `args`.
fn request_for<'a>(
    runtime: &'a OsControlRuntime,
    call: &'a OsGovernedCall,
    program: &str,
    args: &[&str],
) -> StructuredCommandRequest {
    // All five authorities come from the same admission; the runtime refuses if
    // any of them disagree.
    let binding = call.binding();
    let ctx = runtime
        .seal_mutation_context(
            call.observation(),
            call.grant().expect("a mutation call carries a grant"),
            call.leases().expect("a mutation call holds leases"),
            call.admission(),
            &binding,
        )
        .expect("the sealed authorities agree");
    let executable = TrustedExecutable::new(program, Digest::from_hex(&"ab".repeat(32)))
        .expect("an absolute path is a valid trusted executable");
    let plan = CommandPlan::new(
        CapabilityId::new("set_brightness"),
        "set_brightness",
        serde_json::json!({ "probe": true }),
        executable,
        args.iter().map(|a| (*a).to_string()).collect(),
    );
    StructuredCommandRequest::from_admitted(&ctx, plan, &CommandPolicy::new())
        .expect("the plan validates against the grant")
}

fn far_deadline() -> Instant {
    Instant::now() + Duration::from_secs(20)
}

/// A command that exits 0 is Applied — the verifier then confirms the effect.
#[serial]
#[tokio::test]
async fn a_successful_command_is_applied() {
    let _disarm = scoped_disarm();
    let harness = Harness::new();
    let call = harness.call(far_deadline()).await;
    let request = request_for(&harness.runtime, &call, "/bin/echo", &["ok"]);

    let outcome = request.dispatch().await.expect("dispatch completes");
    assert!(
        matches!(outcome, ApplyOutcome::Applied(_)),
        "exit 0 must be Applied, got {outcome:?}"
    );
}

/// A non-zero exit is **Uncertain**, never an error: the mutator ran, so "no
/// effect" is no longer provable and only a fresh observation may conclude.
#[serial]
#[tokio::test]
async fn a_failing_command_is_uncertain_not_an_error() {
    let _disarm = scoped_disarm();
    let harness = Harness::new();
    let call = harness.call(far_deadline()).await;
    let request = request_for(&harness.runtime, &call, "/bin/false", &[]);

    let outcome = request
        .dispatch()
        .await
        .expect("a failing command still yields an outcome, not an Err");
    match outcome {
        ApplyOutcome::Uncertain(u) => assert_eq!(
            u.cause(),
            UncertainEffectCause::ProviderReportedFailureAfterDispatch
        ),
        other => panic!("a non-zero exit must be Uncertain, got {other:?}"),
    }
}

/// A missing executable never starts, so it stays a *pre*-mutation error — the
/// one case where "no effect" is provable.
#[serial]
#[tokio::test]
async fn a_missing_executable_is_a_pre_mutation_error() {
    let _disarm = scoped_disarm();
    let harness = Harness::new();
    let call = harness.call(far_deadline()).await;
    let request = request_for(
        &harness.runtime,
        &call,
        "/nonexistent/kria-launcher-probe",
        &[],
    );

    let error = request
        .dispatch()
        .await
        .expect_err("a command that never started must be an error");
    assert_eq!(
        error.code(),
        "os_control.protocol_before_mutation",
        "the child never ran, so no effect is provable"
    );
}

/// An already-elapsed deadline refuses **before** spawning: nothing runs.
#[serial]
#[tokio::test]
async fn an_elapsed_deadline_refuses_before_spawning() {
    let _disarm = scoped_disarm();
    let harness = Harness::new();
    // Admit with a live deadline, then dispatch against an expired one.
    let call = harness.call(Instant::now() + Duration::from_millis(1)).await;
    let request = request_for(&harness.runtime, &call, "/bin/echo", &["never"]);
    tokio::time::sleep(Duration::from_millis(30)).await;

    let error = request
        .dispatch()
        .await
        .expect_err("an elapsed deadline must refuse");
    assert_eq!(error.code(), "os_control.timed_out_before_mutation");
}

/// Cancellation before dispatch refuses before spawning.
#[serial]
#[tokio::test]
async fn cancellation_before_dispatch_refuses_before_spawning() {
    let _disarm = scoped_disarm();
    let harness = Harness::new();
    let call = harness.call(far_deadline()).await;
    let request = request_for(&harness.runtime, &call, "/bin/echo", &["never"]);
    harness.cancellation.cancel();

    let error = request
        .dispatch()
        .await
        .expect_err("a cancelled call must refuse");
    assert_eq!(error.code(), "os_control.cancelled_before_mutation");
}

/// A child that outlives its deadline yields **Uncertain**, not an error, and is
/// killed rather than left running.
#[serial]
#[tokio::test]
async fn a_child_that_outlives_its_deadline_is_uncertain() {
    let _disarm = scoped_disarm();
    let harness = Harness::new();
    let call = harness.call(Instant::now() + Duration::from_millis(120)).await;
    let request = request_for(&harness.runtime, &call, "/bin/sleep", &["30"]);

    let started = Instant::now();
    let outcome = request
        .dispatch()
        .await
        .expect("a timeout after spawn is an outcome, never an Err");
    match outcome {
        ApplyOutcome::Uncertain(u) => {
            assert_eq!(u.cause(), UncertainEffectCause::TimedOutAfterDispatch)
        }
        other => panic!("a post-spawn timeout must be Uncertain, got {other:?}"),
    }
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the launcher must not wait for the child's natural end"
    );
}

/// A child cancelled after spawning yields Uncertain — the effect may already
/// have landed, so the launcher must not claim otherwise.
#[serial]
#[tokio::test]
async fn cancellation_after_spawn_is_uncertain() {
    let _disarm = scoped_disarm();
    let harness = Harness::new();
    let call = harness.call(far_deadline()).await;
    let request = request_for(&harness.runtime, &call, "/bin/sleep", &["30"]);

    let token = harness.cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(80)).await;
        token.cancel();
    });

    let outcome = request.dispatch().await.expect("an outcome, not an Err");
    match outcome {
        ApplyOutcome::Uncertain(u) => {
            assert_eq!(u.cause(), UncertainEffectCause::CancelledAfterDispatch)
        }
        other => panic!("post-spawn cancellation must be Uncertain, got {other:?}"),
    }
}

/// A child writing far more than the bound is capped, completes, and reports the
/// truncation — a runaway process cannot exhaust memory.
#[serial]
#[tokio::test]
async fn runaway_output_is_bounded_and_reported() {
    let _disarm = scoped_disarm();
    let harness = Harness::new();
    let call = harness.call(far_deadline()).await;
    // ~1.3 MB, far above the 64 KB default bound. No pipe: a `yes | head`
    // pipeline would die of SIGPIPE and exit non-zero, which is a *different*
    // behaviour (correctly reported as Uncertain) than the bounding under test.
    let request = request_for(
        &harness.runtime,
        &call,
        "/bin/sh",
        &["-c", "seq 1 200000"],
    );

    let outcome = request
        .dispatch()
        .await
        .expect("a noisy child still completes");
    let warnings = match &outcome {
        ApplyOutcome::Applied(a) => a.warnings(),
        other => panic!("expected Applied, got {other:?}"),
    };
    assert!(
        warnings.iter().any(|w| w.code.to_string() == "output_truncated"),
        "truncation must be surfaced so bounded output is never mistaken for complete output"
    );
}

/// The child's environment is hermetic: only the allowlisted keys survive, so an
/// inherited `LD_PRELOAD` cannot redirect a trusted executable.
#[serial]
#[tokio::test]
async fn the_child_environment_is_hermetic() {
    let _disarm = scoped_disarm();
    // Set a hostile variable in the PARENT; the child must not see it.
    std::env::set_var("LD_PRELOAD", "/tmp/kria-should-not-be-inherited.so");
    std::env::set_var("KRIA_LAUNCHER_LEAK_PROBE", "leaked");

    let harness = Harness::new();
    let call = harness.call(far_deadline()).await;
    let request = request_for(&harness.runtime, &call, "/bin/sh", &["-c", "env"]);

    // The request's own env map is the whole contract: it is what the launcher
    // installs after clearing, so asserting on it proves the child's view.
    let keys: Vec<&str> = request.env().keys().map(String::as_str).collect();
    assert!(
        !keys.contains(&"LD_PRELOAD"),
        "LD_PRELOAD must never reach the child: {keys:?}"
    );
    assert!(
        !keys.contains(&"KRIA_LAUNCHER_LEAK_PROBE"),
        "no ambient parent variable may be inherited: {keys:?}"
    );
    assert_eq!(request.locale(), "C", "the locale is pinned so output parses");

    let outcome = request.dispatch().await.expect("dispatch completes");
    assert!(matches!(outcome, ApplyOutcome::Applied(_)));

    std::env::remove_var("LD_PRELOAD");
    std::env::remove_var("KRIA_LAUNCHER_LEAK_PROBE");
}

/// The launcher runs the exact sealed argv — the digest computed at seal time
/// still matches at dispatch time, so nothing was rewritten in between.
#[serial]
#[tokio::test]
async fn the_dispatched_argv_is_exactly_the_sealed_argv() {
    let _disarm = scoped_disarm();
    let harness = Harness::new();
    let call = harness.call(far_deadline()).await;
    let request = request_for(&harness.runtime, &call, "/bin/echo", &["--level", "42"]);

    assert_eq!(request.args(), ["--level", "42"]);
    let sealed_digest = request.argv_digest().clone();
    let recomputed = kria_core::os_control::linux::structured_command::compute_argv_digest(
        request.executable().path(),
        request.args(),
    );
    assert_eq!(
        sealed_digest, recomputed,
        "the argv digest must still describe the argv that is about to run"
    );

    let _ = request.dispatch().await.expect("dispatch completes");
}

// ── the governed secret stdin channel ───────────────────────────────────────

/// Build a sealed request that delivers `payload` on stdin.
fn request_with_stdin<'a>(
    runtime: &'a OsControlRuntime,
    call: &'a OsGovernedCall,
    program: &str,
    args: &[&str],
    payload: &str,
) -> StructuredCommandRequest {
    let binding = call.binding();
    let ctx = runtime
        .seal_mutation_context(
            call.observation(),
            call.grant().expect("a mutation call carries a grant"),
            call.leases().expect("a mutation call holds leases"),
            call.admission(),
            &binding,
        )
        .expect("the sealed authorities agree");
    let executable = TrustedExecutable::new(program, Digest::from_hex(&"ab".repeat(32)))
        .expect("an absolute path is a valid trusted executable");
    // The action/params must match the grant the harness minted; the stdin
    // channel is orthogonal to which action carries it.
    let plan = CommandPlan::new(
        CapabilityId::new("set_brightness"),
        "set_brightness",
        serde_json::json!({ "probe": true }),
        executable,
        args.iter().map(|a| (*a).to_string()).collect(),
    )
    .with_secret_stdin(SecretStdin::new(payload.as_bytes().to_vec()));
    StructuredCommandRequest::from_admitted(&ctx, plan, &CommandPolicy::new())
        .expect("the plan validates against the grant")
}

/// The payload actually reaches the child on stdin. The child exits 0 only if it
/// read the exact bytes, so Applied proves delivery.
#[serial]
#[tokio::test]
async fn a_stdin_payload_reaches_the_child() {
    let _disarm = scoped_disarm();
    let harness = Harness::new();
    let call = harness.call(far_deadline()).await;
    let request = request_with_stdin(
        &harness.runtime,
        &call,
        "/bin/sh",
        &["-c", "read line; [ \"$line\" = \"kria-secret-payload\" ]"],
        "kria-secret-payload\n",
    );

    let outcome = request.dispatch().await.expect("dispatch completes");
    assert!(
        matches!(outcome, ApplyOutcome::Applied(_)),
        "the child only exits 0 if it read the exact payload, got {outcome:?}"
    );
}

/// The payload never appears in argv, in the argv digest, or in the audit-facing
/// projection. `/proc/<pid>/cmdline` is world-readable, so an argv transfer would
/// publish a password the user had just copied.
#[serial]
#[tokio::test]
async fn a_stdin_payload_never_appears_in_argv_or_any_digest() {
    let _disarm = scoped_disarm();
    let harness = Harness::new();
    let call = harness.call(far_deadline()).await;
    let secret = "correct-horse-battery-staple";
    let request = request_with_stdin(
        &harness.runtime,
        &call,
        "/bin/cat",
        &[],
        secret,
    );

    for arg in request.args() {
        assert!(!arg.contains(secret), "the payload leaked into argv: {arg}");
    }

    // The digest must describe the argv only. If the payload were folded in, two
    // different clipboard contents would produce different digests for the same
    // command — itself an information leak.
    let recomputed = kria_core::os_control::linux::structured_command::compute_argv_digest(
        request.executable().path(),
        request.args(),
    );
    assert_eq!(
        request.argv_digest(),
        &recomputed,
        "the argv digest must cover argv only, never the stdin payload"
    );

    // Only the length is disclosed.
    assert_eq!(request.stdin_len(), Some(secret.len()));

    let summary = serde_json::to_string(&request.safe_summary()).expect("summary serializes");
    assert!(
        !summary.contains(secret),
        "the payload leaked into the audit-facing summary"
    );

    // Debug output is a common accidental leak path (tracing, panics).
    let rendered = format!("{:?}", SecretStdin::new(secret.as_bytes().to_vec()));
    assert!(
        !rendered.contains(secret),
        "Debug must never render the payload: {rendered}"
    );
    assert!(rendered.contains(&secret.len().to_string()));
}
