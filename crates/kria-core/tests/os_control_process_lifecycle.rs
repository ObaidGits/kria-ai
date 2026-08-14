//! Task 2.5 — process domain slice ("Migrate files, processes, applications,
//! packages, scheduler, disk, clipboard and notifications", OSC-013).
//!
//! # What this binary proves
//!
//! [`os_control::processes`] already unit-tests its pieces in isolation
//! (digest binding, desired-state mapping, graceful-vs-forced distinction).
//! This is the **deny-live, in-process** harness that drives the *real*
//! [`ProcessControl`]`<`[`FakeProcessTransport`]`>` provider through
//! [`OsControlRuntime::run_mutation`] end to end, over the same governed
//! audit-admission + resource-lease + grant chain the other domain lifecycle
//! harnesses use, so the full observe → idempotency → seal → apply → verify
//! lifecycle is exercised for `kill_process` and `set_process_priority`:
//!
//! * `kill_process` is `Unchanged` (zero dispatch) when the target process is
//!   already dead;
//! * `kill_process` with `force=true` dispatches exactly one `SIGKILL`
//!   (never `SIGTERM`) and reaches `Verified` once the process is confirmed
//!   dead;
//! * `kill_process` with `force=false` (graceful) dispatches exactly one
//!   `SIGTERM` — proving the graceful/forced split is real, not merely
//!   documented;
//! * `set_process_priority` is `Unchanged` when already at the desired
//!   niceness, and dispatches+verifies a real change otherwise;
//! * neither operation ever claims rollback availability (`rollbackClaim:
//!   None` in the frozen manifest);
//! * a missing scripted transport response reports the frozen `Unavailable`
//!   envelope — never a fabricated liveness/priority state;
//! * the whole run never trips the process-wide deny-live sentinel and no
//!   process handler in this module ever sends a real signal — the only
//!   evidence is the fake transport's recorded [`SignalCall`]s.
//!
//! Run ONLY this binary, serially:
//! `cargo test -p kria-core --no-default-features --features os-control-test \
//!    --test os_control_process_lifecycle -- --test-threads=1`

#![cfg(feature = "os-control-test")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serial_test::serial;
use tokio_util::sync::CancellationToken;

use kria_core::agent::execution_gate::OsActionGrant;
use kria_core::agent::resource_lease::{ResourceLeaseManager, ResourceRequirement};
use kria_core::agent::turn_memory::ExecutionTarget;
use kria_core::safety::RiskLevel;

use kria_core::os_control::processes::fake::FakeProcessTransport;
use kria_core::os_control::processes::{
    CommandMetadataState, ProcessControl, ProcessFilter, ProcessIdentity, ProcessLifecycleState,
    ProcessObservation, ProcessOp, ProcessRequest,
};
use kria_core::os_control::resource::os_write_requirements;
use kria_core::os_control::runtime::OsControlRuntime;
use kria_core::os_control::runtime::SealBinding;
use kria_core::os_control::{
    sentinel_is_armed, sentinel_trip_count, ActionId, ActionLifecycle, AdmissionRequest,
    AppliedDispatch, AuditAdmissionToken, BoundedVec, ComparatorKind, CorrelationId,
    DesiredStateControl, Digest, HostExecutionContext, MutationPlan, OsAuditStore, OsLeaseContext,
    OsResourceCoordinator, ProviderId, RedactionPolicy, RequestSensitivity, RollbackPlan,
    SessionContext, SessionId, SnapshotRevision,
};

const SESSION: &str = "sess-process-1";

struct Chain {
    audit: OsAuditStore,
    grant: OsActionGrant,
    host_ctx: HostExecutionContext,
    lease_set: kria_core::os_control::AcquiredResourceLeaseSet,
    token: AuditAdmissionToken,
    reqs: Vec<ResourceRequirement>,
    params: serde_json::Value,
    tool: String,
}

impl Chain {
    async fn build(tool: &str, params: serde_json::Value) -> Self {
        let audit = OsAuditStore::open_in_memory();

        let token = audit
            .admit_action(&AdmissionRequest {
                session_id: SessionId::new(SESSION),
                correlation_id: CorrelationId::new("corr-1"),
                action_id: ActionId::new("act-1"),
                tool_name: tool.to_string(),
                params: params.clone(),
                target_hash: Digest::of_str(ExecutionTarget::Host.as_str()),
                capability_snapshot_revision: SnapshotRevision(1),
                risk: RiskLevel::Red,
                decision_id: None,
                sensitivity: RequestSensitivity::Mutation,
            })
            .expect("audit admission must succeed on a healthy store");

        let reqs = os_write_requirements(tool, &params);
        let coordinator = OsResourceCoordinator::new(ResourceLeaseManager::new());
        let lease_set = coordinator
            .acquire_write_leases(
                &OsLeaseContext {
                    workflow_id: SESSION.to_string(),
                    stage_id: None,
                    action_hash: Digest::of_str(tool).as_hex().to_string(),
                },
                tool,
                &params,
            )
            .await
            .expect("write leases acquire in canonical order");

        let grant = OsActionGrant::for_test(
            SESSION,
            tool,
            &params,
            ExecutionTarget::Host,
            &reqs,
            RiskLevel::Red,
        );

        let host_ctx = HostExecutionContext::for_test(
            CorrelationId::new("corr-1"),
            ActionId::new("act-1"),
            token.observation_authority(),
            Arc::new(SessionContext::new(SessionId::new(SESSION))),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(30),
            RedactionPolicy::default(),
        );

        Self {
            audit,
            grant,
            host_ctx,
            lease_set,
            token,
            reqs,
            params,
            tool: tool.to_string(),
        }
    }

    fn binding(&self) -> SealBinding<'_> {
        SealBinding {
            session_id: SESSION,
            action: &self.tool,
            params: &self.params,
            target: ExecutionTarget::Host,
            resource_requirements: &self.reqs,
            capability_snapshot_revision: SnapshotRevision(1),
        }
    }

    fn admission_count(&self) -> usize {
        self.audit.verify_chain().expect("audit hash chain intact");
        self.audit.admission_count(self.token.admission_id())
    }
}

fn kill_request(pid: u32, force: bool) -> ProcessRequest {
    ProcessRequest {
        action: "kill_process".to_string(),
        params: serde_json::json!({ "pid": pid, "signal": if force { "kill" } else { "term" } }),
        op: ProcessOp::Terminate {
            identity: ProcessIdentity::new(pid, 0),
            force,
        },
    }
}

fn priority_request(pid: u32, nice: i32) -> ProcessRequest {
    ProcessRequest {
        action: "set_process_priority".to_string(),
        params: serde_json::json!({ "pid": pid, "priority": nice }),
        op: ProcessOp::SetPriority {
            identity: ProcessIdentity::new(pid, 0),
            nice,
        },
    }
}

fn plan(rollback: RollbackPlan) -> MutationPlan {
    MutationPlan {
        receipt_id: kria_core::os_control::ReceiptId::new("r-process-1"),
        provider: ProviderId::new("process-native-syscall"),
        comparator: ComparatorKind::Exact,
        tolerance: None,
        deadline_ms: 500,
        rollback,
        latency_ms: 5,
    }
}

fn recorded() -> kria_core::os_control::AuditCompletionState {
    kria_core::os_control::AuditCompletionState::Recorded {
        record_id: kria_core::os_control::AuditRecordId::new("rec-process"),
    }
}

fn applied() -> kria_core::os_control::ApplyOutcome {
    kria_core::os_control::ApplyOutcome::Applied(AppliedDispatch::new(None, BoundedVec::new()))
}

// ─────────────────────────────────────────────────────────────────────────────
// A) kill_process idempotency: already dead → Unchanged, zero dispatch
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn kill_process_already_dead_is_unchanged_with_zero_dispatch() {
    let baseline = sentinel_trip_count();
    let params = serde_json::json!({ "pid": 4242, "signal": "kill" });
    let chain = Chain::build("kill_process", params).await;
    assert_eq!(chain.admission_count(), 1);

    let transport = FakeProcessTransport::new().alive_ok(false);
    let provider = ProcessControl::new(transport);
    let request = kill_request(4242, true);
    let desired = request.desired_state();

    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("unchanged receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert!(!receipt.changed());
    assert_eq!(
        provider.transport().dispatch_count(),
        0,
        "already-dead process must not dispatch a signal"
    );
    assert_eq!(sentinel_trip_count(), baseline, "no live transport touched");
}

// ─────────────────────────────────────────────────────────────────────────────
// B) kill_process forced: dispatches exactly one SIGKILL (force=true), Verified
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn kill_process_forced_dispatches_sigkill_and_reaches_verified() {
    let params = serde_json::json!({ "pid": 100, "signal": "kill" });
    let chain = Chain::build("kill_process", params).await;

    let transport = FakeProcessTransport::new()
        .alive_ok(true) // 1: pre-observation
        .alive_ok(true) // 2: under-lease re-observation
        .alive_ok(false) // 3: post-apply re-observation
        .alive_ok(false) // 4: verify independent read
        .dispatch_outcome(applied());
    let provider = ProcessControl::new(transport);
    let request = kill_request(100, true);
    let desired = request.desired_state();

    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("verified receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Verified);
    let calls = provider.transport().signal_calls();
    assert_eq!(calls.len(), 1, "apply exactly once");
    assert!(calls[0].force, "force=true must send SIGKILL, not SIGTERM");
    assert_eq!(calls[0].identity.pid, 100);
}

// ─────────────────────────────────────────────────────────────────────────────
// C) kill_process graceful: dispatches exactly one SIGTERM (force=false) —
//    the graceful/forced split is real, not merely documented.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn kill_process_graceful_dispatches_sigterm_not_sigkill() {
    let params = serde_json::json!({ "pid": 200, "signal": "term" });
    let chain = Chain::build("kill_process", params).await;

    let transport = FakeProcessTransport::new()
        .alive_ok(true)
        .alive_ok(true)
        .alive_ok(false)
        .alive_ok(false)
        .dispatch_outcome(applied());
    let provider = ProcessControl::new(transport);
    let request = kill_request(200, false);
    let desired = request.desired_state();

    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("verified receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Verified);
    let calls = provider.transport().signal_calls();
    assert_eq!(calls.len(), 1);
    assert!(
        !calls[0].force,
        "force=false must send SIGTERM, not SIGKILL"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// D) set_process_priority: idempotent at desired niceness → Unchanged
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn set_process_priority_already_at_desired_is_unchanged() {
    let params = serde_json::json!({ "pid": 300, "priority": 10 });
    let chain = Chain::build("set_process_priority", params).await;

    let transport = FakeProcessTransport::new().priority_ok(10);
    let provider = ProcessControl::new(transport);
    let request = priority_request(300, 10);
    let desired = request.desired_state();

    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("unchanged receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Unchanged);
    assert_eq!(provider.transport().dispatch_count(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// E) set_process_priority: real change dispatches+verifies
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn set_process_priority_changes_and_reaches_verified() {
    let params = serde_json::json!({ "pid": 400, "priority": -5 });
    let chain = Chain::build("set_process_priority", params).await;

    let transport = FakeProcessTransport::new()
        .priority_ok(0) // 1: pre-observation
        .priority_ok(0) // 2: under-lease re-observation
        .priority_ok(0) // 3: apply()'s pre-apply rollback-capture read
        .priority_ok(-5) // 4: post-apply re-observation
        .priority_ok(-5) // 5: verify independent read
        .dispatch_outcome(applied());
    let provider = ProcessControl::new(transport);
    let request = priority_request(400, -5);
    let desired = request.desired_state();

    let receipt = OsControlRuntime::detached()
        .run_mutation(
            &provider,
            &chain.host_ctx,
            &chain.grant,
            &chain.lease_set,
            &chain.token,
            &chain.binding(),
            &request,
            &desired,
            &plan(RollbackPlan::Unavailable),
            recorded(),
        )
        .await
        .expect("verified receipt");

    assert_eq!(receipt.lifecycle(), ActionLifecycle::Verified);
    let calls = provider.transport().priority_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, -5);
}

// ─────────────────────────────────────────────────────────────────────────────
// F) Missing scripted read reports Unavailable — never a fabricated state
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn missing_scripted_liveness_reports_unavailable_not_a_fabricated_state() {
    let chain = Chain::build(
        "kill_process",
        serde_json::json!({ "pid": 1, "signal": "kill" }),
    )
    .await;
    let transport = FakeProcessTransport::new();
    let provider = ProcessControl::new(transport);
    let request = kill_request(1, true);

    let err = provider
        .observe(&chain.host_ctx, &request)
        .await
        .expect_err("missing scripted read must report Unavailable");

    assert!(matches!(
        err,
        kria_core::os_control::OsControlError::Unavailable { .. }
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// G) The runtime's processes() port seam resolves through a composed
//    HostOsControl aggregate and falls back to Unavailable when none is
//    composed.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn runtime_processes_port_is_unavailable_when_no_provider_is_composed() {
    assert!(sentinel_is_armed());
    let rt = OsControlRuntime::detached();
    let result = rt.processes("kill_process");
    assert!(matches!(
        result,
        Err(kria_core::os_control::OsControlError::Unavailable { .. })
    ));
}

#[test]
fn runtime_processes_port_resolves_through_composed_fake_host() {
    use kria_core::os_control::testing::FakeHostOsControl;

    let transport = FakeProcessTransport::new().alive_ok(true);
    let process_provider: Arc<dyn kria_core::os_control::ProcessControlPort> =
        Arc::new(ProcessControl::new(transport));

    let fake_host = FakeHostOsControl::new("process-aggregate").with_processes(process_provider);
    let rt = OsControlRuntime::with_host(Arc::new(fake_host));

    let _ = rt.processes("kill_process").expect("process port composed");
    assert_eq!(rt.provider_id().unwrap().as_str(), "process-aggregate");
}

// ─────────────────────────────────────────────────────────────────────────────
// H) list_processes / get_process_info: content-free schema (OSC-013.4/.6)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn list_processes_returns_content_free_observations_with_not_requested_metadata() {
    let chain = Chain::build("list_processes", serde_json::json!({})).await;

    let obs_a = ProcessObservation::new(
        ProcessIdentity::new(100, 111_000),
        "gedit",
        Digest::of_str("/usr/bin/gedit"),
        "1000",
        ProcessLifecycleState::Running,
        5,
        1024 * 1024,
    );
    let obs_b = ProcessObservation::new(
        ProcessIdentity::new(200, 222_000),
        "firefox",
        Digest::of_str("/usr/bin/firefox"),
        "1000",
        ProcessLifecycleState::Sleeping,
        1,
        512 * 1024 * 1024,
    );
    let transport = FakeProcessTransport::new()
        .with_process(obs_a.clone())
        .with_process(obs_b.clone());
    let provider = ProcessControl::new(transport);

    let page = provider
        .list_observations(&chain.host_ctx, &ProcessFilter::default(), 0, 50)
        .await
        .expect("list succeeds");

    assert_eq!(page.items.len(), 2);
    for item in &page.items {
        // Content-free by default: every observation starts NotRequested —
        // never a fabricated argv/environment/cwd field (there is no such
        // field to fabricate).
        assert_eq!(item.command_metadata, CommandMetadataState::NotRequested);
    }
}

#[tokio::test]
#[serial]
async fn get_process_info_reports_pid_reuse_as_absent_never_conflated() {
    let chain = Chain::build(
        "get_process_info",
        serde_json::json!({ "process": { "pid": 100, "start_time": 111_000 } }),
    )
    .await;

    // The process table has a DIFFERENT process now living at pid 100 (a
    // reused PID with a different start_time) — the original identity must
    // report absent (unknown_process_identity_error), never the unrelated
    // reused-PID process's data.
    let reused = ProcessObservation::new(
        ProcessIdentity::new(100, 999_999), // different start_time: PID reuse
        "unrelated-process",
        Digest::of_str("/usr/bin/unrelated"),
        "1000",
        ProcessLifecycleState::Running,
        2,
        2048,
    );
    let transport = FakeProcessTransport::new().with_process(reused);
    let provider = ProcessControl::new(transport);

    let original_identity = ProcessIdentity::new(100, 111_000);
    let err = provider
        .read_observation(&chain.host_ctx, original_identity)
        .await
        .expect_err("original identity must be reported absent after PID reuse");

    assert!(matches!(
        err,
        kria_core::os_control::OsControlError::InvalidRequest { .. }
    ));
}

#[tokio::test]
#[serial]
async fn get_process_info_returns_exact_match_for_correct_identity() {
    let chain = Chain::build(
        "get_process_info",
        serde_json::json!({ "process": { "pid": 100, "start_time": 111_000 } }),
    )
    .await;

    let identity = ProcessIdentity::new(100, 111_000);
    let obs = ProcessObservation::new(
        identity,
        "gedit",
        Digest::of_str("/usr/bin/gedit"),
        "1000",
        ProcessLifecycleState::Running,
        5,
        1024,
    );
    let transport = FakeProcessTransport::new().with_process(obs);
    let provider = ProcessControl::new(transport);

    let result = provider
        .read_observation(&chain.host_ctx, identity)
        .await
        .expect("exact identity match succeeds");

    assert_eq!(result.identity, identity);
    assert_eq!(result.command_metadata, CommandMetadataState::NotRequested);
}

#[tokio::test]
#[serial]
async fn list_processes_exact_app_id_match_excludes_ambiguous_similar_names() {
    // Exact-name matching (never substring): filtering by app_id "code" must
    // not also match "vscode" or "code-helper".
    let chain = Chain::build("list_processes", serde_json::json!({})).await;

    let exact = ProcessObservation::new(
        ProcessIdentity::new(10, 1),
        "code",
        Digest::of_str("/usr/bin/code"),
        "code",
        ProcessLifecycleState::Running,
        1,
        1024,
    );
    let similar = ProcessObservation::new(
        ProcessIdentity::new(20, 2),
        "vscode",
        Digest::of_str("/usr/bin/vscode"),
        "vscode",
        ProcessLifecycleState::Running,
        1,
        1024,
    );
    let transport = FakeProcessTransport::new()
        .with_process(exact)
        .with_process(similar);
    let provider = ProcessControl::new(transport);

    let filter = ProcessFilter {
        app_id: Some("code".to_string()),
        ..Default::default()
    };
    let page = provider
        .list_observations(&chain.host_ctx, &filter, 0, 50)
        .await
        .expect("list succeeds");

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].identity.pid, 10);
}

// ─────────────────────────────────────────────────────────────────────────────
// I) get_process_command_metadata: mandatory approval, argv bounds/truncation
//    (OSC-013.5/.6) — RED, ephemeral, content-free reporting elsewhere
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn get_process_command_metadata_returns_bounded_argv_only() {
    let identity = ProcessIdentity::new(42, 555);
    let chain = Chain::build(
        "get_process_command_metadata",
        serde_json::json!({ "process": { "pid": 42, "start_time": 555 }, "purpose": "debugging a hang" }),
    )
    .await;

    let raw_argv = vec!["gedit".to_string(), "--new-window".to_string(), "a.txt".to_string()];
    let metadata = kria_core::os_control::processes::BoundedCommandMetadata::from_raw_argv(
        Digest::of_str("/usr/bin/gedit"),
        &raw_argv,
    );
    let transport = FakeProcessTransport::new().with_command_metadata(identity, metadata);
    let provider = ProcessControl::new(transport);

    let result = provider
        .read_command_metadata(&chain.host_ctx, identity, "debugging a hang")
        .await
        .expect("command metadata resolves");

    assert_eq!(result.argument_count(), 3);
    assert!(!result.truncated());
    assert_eq!(result.argv()[0].expose_argument(), "gedit");
}

#[tokio::test]
#[serial]
async fn get_process_command_metadata_denies_without_mandatory_approval() {
    // Mandatory-approval simulation: the transport is configured to deny
    // every command-metadata request regardless of process table contents —
    // representing the fail-closed path when the RED tool's admission is
    // rejected before the provider is ever asked for real argv.
    let identity = ProcessIdentity::new(42, 555);
    let chain = Chain::build(
        "get_process_command_metadata",
        serde_json::json!({ "process": { "pid": 42, "start_time": 555 }, "purpose": "debugging a hang" }),
    )
    .await;

    let transport = FakeProcessTransport::new().deny_all_command_metadata();
    let provider = ProcessControl::new(transport);

    let err = provider
        .read_command_metadata(&chain.host_ctx, identity, "debugging a hang")
        .await
        .expect_err("denied command metadata must fail closed");

    assert!(matches!(
        err,
        kria_core::os_control::OsControlError::PermissionDenied { .. }
    ));
}

#[tokio::test]
#[serial]
async fn get_process_command_metadata_truncates_oversized_argv() {
    let identity = ProcessIdentity::new(42, 555);
    let chain = Chain::build(
        "get_process_command_metadata",
        serde_json::json!({ "process": { "pid": 42, "start_time": 555 }, "purpose": "audit" }),
    )
    .await;

    let raw_argv: Vec<String> = (0..100).map(|i| format!("arg{i}")).collect();
    let metadata = kria_core::os_control::processes::BoundedCommandMetadata::from_raw_argv(
        Digest::of_str("/bin/x"),
        &raw_argv,
    );
    let transport = FakeProcessTransport::new().with_command_metadata(identity, metadata);
    let provider = ProcessControl::new(transport);

    let result = provider
        .read_command_metadata(&chain.host_ctx, identity, "audit")
        .await
        .expect("command metadata resolves");

    assert!(result.truncated());
    assert!(result.argument_count() <= 64);
}

#[tokio::test]
#[serial]
async fn get_process_command_metadata_unknown_identity_reports_unknown_not_fabricated() {
    let identity = ProcessIdentity::new(999, 1);
    let chain = Chain::build(
        "get_process_command_metadata",
        serde_json::json!({ "process": { "pid": 999, "start_time": 1 }, "purpose": "audit" }),
    )
    .await;

    let transport = FakeProcessTransport::new();
    let provider = ProcessControl::new(transport);

    let err = provider
        .read_command_metadata(&chain.host_ctx, identity, "audit")
        .await
        .expect_err("unknown identity must fail closed, never fabricate argv");

    assert!(matches!(
        err,
        kria_core::os_control::OsControlError::InvalidRequest { .. }
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// J) set_process_priority rollback: captures prior niceness and restores it
//    (OSC-013.8, frozen manifest `rollbackClaim: UserRequestable`)
// ─────────────────────────────────────────────────────────────────────────────

fn rollback_token(action_hash: Digest) -> kria_core::os_control::RollbackToken {
    use kria_core::os_control::{GrantNonce, ReceiptId};
    kria_core::os_control::RollbackToken::new(
        Digest::of_str("token-1"),
        SessionId::new(SESSION),
        action_hash,
        ProviderId::new("process-native-syscall"),
        ReceiptId::new("r-process-1"),
        GrantNonce::new("nonce-1"),
        std::time::SystemTime::now() + Duration::from_secs(60),
    )
}

#[tokio::test]
#[serial]
async fn set_process_priority_rollback_restores_captured_prior_niceness() {
    let params = serde_json::json!({ "pid": 500, "priority": -5 });
    let chain = Chain::build("set_process_priority", params).await;

    let transport = FakeProcessTransport::new()
        .priority_ok(10) // apply()'s pre-apply capture read
        .dispatch_outcome(applied()) // apply() dispatch
        .dispatch_outcome(applied()); // rollback() dispatch
    let provider = ProcessControl::new(transport);
    let request = priority_request(500, -5);

    let permit = kria_core::os_control::context::MutationPermit::for_test(
        &chain.lease_set,
        &chain.token,
        Digest::of_str(chain.grant.resource_set_digest()),
    );
    let mutation_ctx = kria_core::os_control::context::AdmittedMutationContext::for_test(
        &chain.host_ctx,
        &chain.grant,
        permit,
    );

    // Drive apply() directly to populate the rollback snapshot for this
    // session.
    let _ = provider
        .apply(&mutation_ctx, &request, &request.desired_state())
        .await
        .expect("apply succeeds");

    let token = rollback_token(Digest::of_str("set_process_priority"));

    let outcome = provider
        .rollback(&mutation_ctx, &token)
        .await
        .expect("rollback succeeds");

    // The rollback dispatched a set_priority call restoring the captured
    // prior niceness (10), not the newly-applied one (-5).
    let calls = provider.transport().priority_calls();
    assert!(
        calls.iter().any(|(_, nice)| *nice == 10),
        "rollback must restore the captured prior niceness (10), got: {calls:?}"
    );
    assert!(matches!(
        outcome,
        kria_core::os_control::ApplyOutcome::Applied(_)
    ));
}

#[tokio::test]
#[serial]
async fn set_process_priority_rollback_without_snapshot_reports_uncertain_not_fabricated() {
    let chain = Chain::build(
        "set_process_priority",
        serde_json::json!({ "pid": 600, "priority": 3 }),
    )
    .await;

    // No apply() call happened for this session, so no snapshot exists.
    let transport = FakeProcessTransport::new();
    let provider = ProcessControl::new(transport);

    let permit = kria_core::os_control::context::MutationPermit::for_test(
        &chain.lease_set,
        &chain.token,
        Digest::of_str(chain.grant.resource_set_digest()),
    );
    let mutation_ctx = kria_core::os_control::context::AdmittedMutationContext::for_test(
        &chain.host_ctx,
        &chain.grant,
        permit,
    );

    let token = rollback_token(Digest::of_str("set_process_priority"));

    let outcome = provider
        .rollback(&mutation_ctx, &token)
        .await
        .expect("rollback call itself does not error");

    assert!(matches!(
        outcome,
        kria_core::os_control::ApplyOutcome::Uncertain(_)
    ));
    assert_eq!(
        provider.transport().priority_calls().len(),
        0,
        "no snapshot means no fabricated restore dispatch"
    );
}
