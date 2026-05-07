use std::collections::HashSet;
use std::sync::atomic::Ordering;

use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{
    AdmissionBarrierOutcome, ControlPlaneTransport, EvidenceSource, ExecutionEnvelope,
    FileCommitPolicy, GuestFilesystemPolicy, GuestOsFamily, HelperExecutionEvidence,
    HelperProvisioning, HostArtifactGcConfig, HostGarbageCollector, HostPlatform,
    InflightCommandHandle, InfrastructureRuntimeConfig, JournalTerminalFooter, LastGaspPacket,
    ParentIdentity, PrivilegedCommitMode, QemuSshEnvironment, RemoteConfig, ReplayCachePolicy,
    ResetPolicy, ResetPriorityQuota, SshMultiplexingConfig, SshPoolConfig, SshTransportBackend,
    StagedArtifactLeaseMetadata, TargetKind,
};

fn test_root(name: &str) -> std::path::PathBuf {
    let root =
        std::env::temp_dir().join(format!("kria-remote-qemu-test-{}-{}", name, Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("create test root");
    root
}

fn test_remote_config(root: &std::path::Path) -> RemoteConfig {
    let workspace_root = root.join("workspace");
    let staging_root = root.join("staging");
    let control_root = root.join("control");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::create_dir_all(&staging_root).expect("create staging root");
    std::fs::create_dir_all(&control_root).expect("create control root");

    RemoteConfig {
        host_platform: HostPlatform::Linux,
        host: "127.0.0.1".to_string(),
        port: 22,
        username: "tester".to_string(),
        ssh_key_path: root.join("id_ed25519"),
        guest_os_family: GuestOsFamily::Posix,
        target_kind: TargetKind::PhysicalRemoteHost,
        qemu_boot_cmd: None,
        qemu_pid_state_file: root.join("qemu.pid"),
        instance_id: format!("test-instance-{}", Uuid::new_v4()),
        remote_control_dir: control_root,
        transport_backend: SshTransportBackend::OpenSshControlMaster,
        ssh_multiplexing: SshMultiplexingConfig {
            enable_control_master: false,
            control_path_cmd: root.join("cmd.sock"),
            control_path_bulk: root.join("bulk.sock"),
            control_persist_secs: 30,
            establish_timeout_ms: 500,
            control_check_timeout_ms: 500,
            allow_no_mux_for_test: true,
            rust_ssh_max_parallel_channels: 8,
        },
        helper_provisioning: HelperProvisioning {
            required_helper_version: "test".to_string(),
            helper_manifest_path: root.join("helper.manifest"),
            helper_manifest_sig_path: root.join("helper.manifest.sig"),
            helper_public_key_path: root.join("helper.pub"),
            host_helper_cache_dir: root.join("helper_cache"),
            remote_helper_dir: root.join("remote_helper"),
            remote_helper_lock_dir: root.join("remote_helper_lock"),
            helper_lock_timeout_ms: 100,
            helper_lock_claim_retry_ms: 10,
            supervisor_heartbeat_interval_ms: 50,
            supervisor_heartbeat_timeout_ms: 200,
            worker_journal_silence_timeout_ms: 200,
            emergency_status_buffer_bytes: 512 * 1024,
            last_gasp_packet_timeout_ms: 100,
            max_helper_rss_bytes: 64 * 1024 * 1024,
        },
        control_transport: ControlPlaneTransport::EphemeralSftpFile,
        envelope_ttl_ms: 1_000,
        max_command_payload_bytes: 4096,
        file_commit_policy: FileCommitPolicy {
            remote_staging_dir: staging_root,
            privileged_commit_mode: PrivilegedCommitMode::Disabled,
            privileged_commit_helper_path: None,
            staging_sweep_ttl_secs: 1,
            staging_lease_heartbeat_timeout_ms: 1,
            staging_sweep_batch_limit: 32,
            enforce_linux_openat2: true,
            privileged_probe_timeout_ms: 200,
            privileged_commit_timeout_ms: 200,
            disable_privileged_on_probe_failure: true,
        },
        guest_filesystem_policy: GuestFilesystemPolicy {
            require_control_dir_writable: true,
            require_staging_dir_writable: true,
            require_non_readonly_mount: true,
            min_free_bytes_floor: 64 * 1024 * 1024,
        },
        reset_policy: ResetPolicy {
            admission_freeze_timeout_ms: 1,
            zombie_reap_timeout_ms: 1,
            lock_acquire_timeout_ms: 50,
            network_call_timeout_ms: 250,
            total_reset_deadline_ms: 5_000,
        },
        replay_cache_policy: ReplayCachePolicy {
            retained_epoch_buckets: 2,
            max_nonces_per_epoch: 128,
        },
        ssh_pool: SshPoolConfig {
            max_active_targets_hard_cap: 8,
            idle_ttl_secs: 30,
            sweep_interval_secs: 30,
            fd_soft_limit: 4096,
            fd_reserve: 64,
            fd_per_command_budget: 4,
            fd_telemetry_sample_ms: 100,
        },
        host_artifact_gc: HostArtifactGcConfig {
            enable_gc: true,
            gc_ttl_secs: 60,
            state_root_dir: root.join("state"),
            host_binary_sha256_or_build_id: "test-binary".to_string(),
        },
        infrastructure_runtime: InfrastructureRuntimeConfig {
            infra_worker_threads: 2,
            high_priority_queue_capacity: 16,
            medium_priority_queue_capacity: 16,
            low_priority_queue_capacity: 16,
            infra_spawn_timeout_ms: 500,
        },
        ssh_connect_timeout_ms: 500,
        command_timeout_ms: 500,
        boot_wait_timeout_ms: 500,
        poll_interval_ms: 10,
        shutdown_timeout_ms: 500,
        soft_reset_grace_ms: 50,
        soft_reset_kill_timeout_ms: 50,
        max_soft_reset_attempts: 2,
        inflight_drain_timeout_ms: 100,
        local_cancel_kill_timeout_ms: 100,
        max_stdout_bytes: 1024 * 1024,
        max_stderr_bytes: 1024 * 1024,
        max_read_file_bytes: 1024 * 1024,
        command_timeout_requires_reset: true,
        known_hosts_path: None,
        strict_host_key_checking: false,
        pinned_host_key_sha256: None,
        remote_workspace_root: Some(workspace_root),
    }
}

fn staged_metadata(now_unix_ms: u64) -> StagedArtifactLeaseMetadata {
    StagedArtifactLeaseMetadata {
        owner_instance_id: "instance-a".to_string(),
        owner_pid: None,
        owner_pid_start_time_ticks: None,
        owner_binary_sha256_or_build_id: None,
        generation: 1,
        epoch_uuid: Uuid::new_v4(),
        artifact_nonce: "artifact-nonce".to_string(),
        created_unix_ms: now_unix_ms,
        lease_heartbeat_unix_ms: now_unix_ms,
        expected_sha256: "abc123".to_string(),
        bytes: 16,
    }
}

fn parent_identity(pid: u32, start_time_ticks: u64, nonce: &str) -> ParentIdentity {
    ParentIdentity {
        pid,
        start_time_ticks,
        session_nonce: nonce.to_string(),
    }
}

fn evidence_envelope(command_id: &str) -> ExecutionEnvelope {
    ExecutionEnvelope {
        command_id: command_id.to_string(),
        generation: 7,
        epoch_uuid: Uuid::new_v4().into_bytes(),
        transport_generation_id: 13,
        instance_id: "instance-test".to_string(),
        issued_at_host_unix_ms_info_only: 1,
        ttl_ms_from_receipt: 500,
        nonce: "nonce-1".to_string(),
        parent_session_nonce: "nonce-parent".to_string(),
        parent_ssh_session_pid: Some(1234),
        parent_ssh_session_start_time_ticks: Some(5678),
        cwd: "/tmp".to_string(),
        env: Default::default(),
        program: "echo".to_string(),
        args: vec!["hello".to_string()],
        command_sha256: "sha256-test".to_string(),
        stdin_mode: "none".to_string(),
    }
}

#[test]
fn triple_check_identity_accepts_exact_match() {
    let expected = parent_identity(1234, 5678, "nonce-a");
    let observed = parent_identity(1234, 5678, "nonce-a");
    assert!(QemuSshEnvironment::validate_parent_identity_triple(
        &expected, &observed
    ));
}

#[test]
fn triple_check_identity_rejects_pid_reuse() {
    let expected = parent_identity(1234, 5678, "nonce-a");
    let observed = parent_identity(4321, 5678, "nonce-a");
    assert!(!QemuSshEnvironment::validate_parent_identity_triple(
        &expected, &observed
    ));
}

#[test]
fn triple_check_identity_rejects_start_time_mismatch() {
    let expected = parent_identity(1234, 5678, "nonce-a");
    let observed = parent_identity(1234, 9999, "nonce-a");
    assert!(!QemuSshEnvironment::validate_parent_identity_triple(
        &expected, &observed
    ));
}

#[test]
fn triple_check_identity_rejects_session_nonce_mismatch() {
    let expected = parent_identity(1234, 5678, "nonce-a");
    let observed = parent_identity(1234, 5678, "nonce-b");
    assert!(!QemuSshEnvironment::validate_parent_identity_triple(
        &expected, &observed
    ));
}

#[test]
fn host_gc_binary_fingerprint_triple_check_rejects_mismatch() {
    assert!(
        HostGarbageCollector::validate_owner_triple_with_fingerprint(
            Some(100),
            Some(200),
            Some("build-a"),
            100,
            200,
            "build-a",
        )
    );

    assert!(
        !HostGarbageCollector::validate_owner_triple_with_fingerprint(
            Some(100),
            Some(200),
            Some("build-a"),
            100,
            200,
            "build-b",
        )
    );
}

#[test]
fn high_to_medium_priority_fairness_every_third_high() {
    let mut quota = ResetPriorityQuota::default();
    let observed = (0..7)
        .map(|_| quota.on_high_recovery_step())
        .collect::<Vec<_>>();

    assert_eq!(
        observed,
        vec![false, false, true, false, false, true, false]
    );
}

#[tokio::test]
async fn reserved_reset_slots_preserve_high_priority_capacity() {
    let root = test_root("reserved-reset-slots");
    let mut config = test_remote_config(&root);
    config.infrastructure_runtime.high_priority_queue_capacity = 10;
    let handle = Handle::current();
    let env = QemuSshEnvironment::new(config, handle.clone(), handle)
        .expect("construct qemu ssh environment");

    let mut guards = Vec::new();
    for _ in 0..7 {
        guards.push(
            env.acquire_infra_slot("cancel_inflight::flood")
                .expect("high-non-reset slot should be available"),
        );
    }

    assert!(env.acquire_infra_slot("cancel_inflight::flood").is_err());
    assert!(env
        .acquire_infra_slot("reset_environment::admission_barrier")
        .is_ok());

    drop(guards);
}

#[test]
fn journal_footer_is_authoritative_over_last_gasp() {
    let envelope = evidence_envelope("cmd-journal-authority");
    let last_gasp = serde_json::to_string(&LastGaspPacket {
        command_id: envelope.command_id.clone(),
        generation: envelope.generation,
        epoch_uuid: envelope.epoch_uuid,
        nonce: envelope.nonce.clone(),
        terminal_state: "Exited".to_string(),
        exit_code_or_signal: 0,
        last_error: String::new(),
        stdout: "last-gasp-stdout".to_string(),
        stderr: String::new(),
    })
    .expect("serialize last-gasp packet");

    let resolved = QemuSshEnvironment::resolve_terminal_evidence(
        &envelope,
        HelperExecutionEvidence {
            journal_footer: Some(JournalTerminalFooter {
                command_id: envelope.command_id.clone(),
                generation: envelope.generation,
                epoch_uuid: envelope.epoch_uuid,
                nonce: envelope.nonce.clone(),
                exit_code: 42,
                stdout: "journal-stdout".to_string(),
                stderr: "journal-stderr".to_string(),
                journal_complete: true,
            }),
            last_gasp_packet_raw: Some(last_gasp),
        },
    )
    .expect("resolve evidence");

    assert_eq!(resolved.source, EvidenceSource::Journal);
    assert_eq!(resolved.exit_code, 42);
    assert_eq!(resolved.stdout, "journal-stdout");
}

#[test]
fn incomplete_journal_falls_back_to_last_gasp() {
    let envelope = evidence_envelope("cmd-last-gasp-fallback");
    let last_gasp = serde_json::to_string(&LastGaspPacket {
        command_id: envelope.command_id.clone(),
        generation: envelope.generation,
        epoch_uuid: envelope.epoch_uuid,
        nonce: envelope.nonce.clone(),
        terminal_state: "Exited".to_string(),
        exit_code_or_signal: 3,
        last_error: "fallback".to_string(),
        stdout: "last-gasp-stdout".to_string(),
        stderr: "last-gasp-stderr".to_string(),
    })
    .expect("serialize last-gasp packet");

    let resolved = QemuSshEnvironment::resolve_terminal_evidence(
        &envelope,
        HelperExecutionEvidence {
            journal_footer: Some(JournalTerminalFooter {
                command_id: envelope.command_id.clone(),
                generation: envelope.generation,
                epoch_uuid: envelope.epoch_uuid,
                nonce: envelope.nonce.clone(),
                exit_code: 0,
                stdout: "journal-stdout".to_string(),
                stderr: String::new(),
                journal_complete: false,
            }),
            last_gasp_packet_raw: Some(last_gasp),
        },
    )
    .expect("resolve evidence");

    assert_eq!(resolved.source, EvidenceSource::LastGasp);
    assert_eq!(resolved.exit_code, 3);
    assert_eq!(resolved.stderr, "last-gasp-stderr");
}

#[tokio::test]
async fn zombie_reaping_orphans_handles_after_barrier_timeout() {
    let root = test_root("zombie-reaping");
    let config = test_remote_config(&root);
    let handle = Handle::current();
    let env = QemuSshEnvironment::new(config, handle.clone(), handle)
        .expect("construct qemu ssh environment");

    env.admission_inflight.store(1, Ordering::Release);
    env.inflight_registry.write().await.insert(
        "cmd-zombie".to_string(),
        InflightCommandHandle {
            command_id: "cmd-zombie".to_string(),
            generation: 0,
            epoch_uuid: Uuid::new_v4(),
            transport_generation_id: 0,
            cancel_token: CancellationToken::new(),
            local_process_ids: Vec::new(),
            remote_status_path: root.join("cmd-zombie.status"),
            remote_tmp_paths: HashSet::new(),
            parent_identity: None,
            helper_supervisor_pid: None,
            helper_worker_pid: None,
            helper_worker_start_time_ticks: None,
        },
    );

    let outcome = env
        .wait_for_admission_barrier_or_zombie_reap()
        .await
        .expect("zombie reaping outcome");

    match outcome {
        AdmissionBarrierOutcome::ZombieReaping { orphaned_handles } => {
            assert_eq!(orphaned_handles, 1);
        }
        AdmissionBarrierOutcome::BarrierReached => {
            panic!("expected ZombieReaping outcome");
        }
    }

    assert!(env.inflight_registry.read().await.is_empty());
    assert!(env.zombie_commands.read().await.contains("cmd-zombie"));
    assert_eq!(env.admission_inflight.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn barrier_requires_admission_and_registry_empty() {
    let root = test_root("dual-barrier");
    let config = test_remote_config(&root);
    let handle = Handle::current();
    let env = QemuSshEnvironment::new(config, handle.clone(), handle)
        .expect("construct qemu ssh environment");

    env.admission_inflight.store(0, Ordering::Release);
    env.inflight_registry.write().await.insert(
        "cmd-dual-barrier".to_string(),
        InflightCommandHandle {
            command_id: "cmd-dual-barrier".to_string(),
            generation: 0,
            epoch_uuid: Uuid::new_v4(),
            transport_generation_id: 0,
            cancel_token: CancellationToken::new(),
            local_process_ids: Vec::new(),
            remote_status_path: root.join("cmd-dual-barrier.status"),
            remote_tmp_paths: HashSet::new(),
            parent_identity: None,
            helper_supervisor_pid: None,
            helper_worker_pid: None,
            helper_worker_start_time_ticks: None,
        },
    );

    let outcome = env
        .wait_for_admission_barrier_or_zombie_reap()
        .await
        .expect("dual barrier outcome");

    match outcome {
        AdmissionBarrierOutcome::ZombieReaping { orphaned_handles } => {
            assert_eq!(orphaned_handles, 1);
        }
        AdmissionBarrierOutcome::BarrierReached => {
            panic!("expected ZombieReaping outcome");
        }
    }
}

#[tokio::test]
async fn stale_epoch_or_generation_fence_is_rejected() {
    let root = test_root("stale-fence");
    let config = test_remote_config(&root);
    let handle = Handle::current();
    let env = QemuSshEnvironment::new(config, handle.clone(), handle)
        .expect("construct qemu ssh environment");

    let generation = env.generation.load(Ordering::Acquire);
    let epoch_uuid = env.current_epoch_uuid();
    assert!(env.is_command_fence_current(generation, epoch_uuid));

    env.generation.fetch_add(1, Ordering::AcqRel);
    assert!(!env.is_command_fence_current(generation, epoch_uuid));
}

#[test]
fn global_sweep_predicate_requires_all_three_conditions() {
    let now = 20_000;
    let mut metadata = staged_metadata(1_000);
    metadata.lease_heartbeat_unix_ms = 1_000;

    let ttl_ms = 5_000;
    let heartbeat_timeout_ms = 500;

    assert!(QemuSshEnvironment::should_delete_staged_artifact(
        &metadata,
        now,
        ttl_ms,
        heartbeat_timeout_ms,
        true,
    ));

    assert!(!QemuSshEnvironment::should_delete_staged_artifact(
        &metadata,
        4_000,
        ttl_ms,
        heartbeat_timeout_ms,
        true,
    ));

    assert!(!QemuSshEnvironment::should_delete_staged_artifact(
        &metadata, now, ttl_ms, 25_000, true,
    ));

    assert!(!QemuSshEnvironment::should_delete_staged_artifact(
        &metadata,
        now,
        ttl_ms,
        heartbeat_timeout_ms,
        false,
    ));
}
