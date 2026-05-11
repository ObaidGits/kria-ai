use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kria_core::infra::environment::remote_qemu::{
    ControlPlaneTransport, FileCommitPolicy, GuestFilesystemPolicy, GuestOsFamily,
    HelperProvisioning, HostArtifactGcConfig, HostPlatform, InfrastructureRuntimeConfig,
    PrivilegedCommitMode, QemuSshEnvironment, RemoteConfig, ReplayCachePolicy, ResetPolicy,
    SshMultiplexingConfig, SshPoolConfig, SshTransportBackend, StagedArtifactLeaseMetadata,
    TargetKind,
};
use kria_core::infra::environment::{
    CommandExecutor, CommandRequest, EnvironmentError, EnvironmentLifecycle, ResetReason,
    ShellState,
};
use kria_core::infra::pool::{
    InventoryState, SelectionWeights, TargetHealthTelemetry, TargetId, TargetPool, TargetPoolConfig,
};
use kria_core::infra::qos::AdaptiveQosScheduler;
use kria_core::infra::snapshot::{
    SnapshotCreateRequest, SnapshotDriftTolerance, SnapshotRestoreRequest, VmSnapshotProvider,
};
use tokio::runtime::Handle;
use uuid::Uuid;

fn test_root(label: &str) -> PathBuf {
    let mut root = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    root.push(format!("kria_remote_qemu_chaos_{label}_{nanos}"));
    fs::create_dir_all(&root).expect("create chaos test root");
    root
}

fn test_remote_config(root: &Path) -> RemoteConfig {
    let workspace_root = root.join("workspace");
    let staging_root = root.join("staging");
    let control_root = root.join("control");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::create_dir_all(&staging_root).expect("create staging root");
    fs::create_dir_all(&control_root).expect("create control root");

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
        instance_id: format!("chaos-instance-{}", Uuid::new_v4()),
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
            supervisor_heartbeat_timeout_ms: 500,
            worker_journal_silence_timeout_ms: 500,
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
            staging_sweep_batch_limit: 64,
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
            admission_freeze_timeout_ms: 250,
            zombie_reap_timeout_ms: 250,
            lock_acquire_timeout_ms: 100,
            network_call_timeout_ms: 500,
            total_reset_deadline_ms: 12_000,
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
            host_binary_sha256_or_build_id: "active-kria-binary".to_string(),
        },
        infrastructure_runtime: InfrastructureRuntimeConfig {
            infra_worker_threads: 2,
            high_priority_queue_capacity: 10,
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
        inflight_drain_timeout_ms: 500,
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

fn write_executable(path: &Path, script: &str) {
    fs::write(path, script.as_bytes()).expect("write executable test helper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)
            .expect("read executable metadata")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod executable helper");
    }
}

fn shell_state_with_path(bin_dir: &Path) -> ShellState {
    let mut env_vars = HashMap::new();
    let inherited_path = std::env::var("PATH").unwrap_or_default();
    let merged_path = if inherited_path.is_empty() {
        bin_dir.display().to_string()
    } else {
        format!("{}:{}", bin_dir.display(), inherited_path)
    };
    env_vars.insert("PATH".to_string(), merged_path);

    ShellState {
        cwd: PathBuf::new(),
        env_vars,
        generation: 0,
    }
}

fn staged_sidecar_path(staged_path: &Path) -> PathBuf {
    let mut sidecar = staged_path.as_os_str().to_os_string();
    sidecar.push(".lease.json");
    PathBuf::from(sidecar)
}

#[cfg(target_os = "linux")]
fn current_process_start_time_ticks() -> u64 {
    let stat = fs::read_to_string("/proc/self/stat").expect("read /proc/self/stat");
    let close_paren = stat
        .rfind(')')
        .expect("expected process name terminator in stat");
    let after_comm = stat
        .get(close_paren + 2..)
        .expect("expected stat fields after process name");
    let mut fields = after_comm.split_whitespace();
    for _ in 0..19 {
        let _ = fields
            .next()
            .expect("unexpected /proc/self/stat field count");
    }

    fields
        .next()
        .expect("missing process start-time ticks")
        .parse::<u64>()
        .expect("parse process start-time ticks")
}

#[tokio::test]
async fn chaos_enospc_uses_last_gasp_terminal_packet_recovery() {
    let root = test_root("enospc-last-gasp");
    let config = test_remote_config(&root);
    let handle = Handle::current();
    let env = QemuSshEnvironment::new(config, handle.clone(), handle)
        .expect("construct qemu ssh environment");
    env.helper_worker_stdout_stderr_local_logs
        .store(true, Ordering::Release);
    env.activate_verified_lease(Uuid::new_v4(), Duration::from_secs(30))
        .await
        .expect("activate lease for command execution");

    let bin_dir = root.join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let command_name = "__simulate_incomplete_journal__";
    write_executable(
        &bin_dir.join(command_name),
        "#!/usr/bin/env bash\necho 'simulated ENOSPC while flushing journal' >&2\nexit 28\n",
    );

    let result = env
        .execute_command(
            CommandRequest {
                program: command_name.to_string(),
                args: Vec::new(),
                timeout_ms: 1_000,
                max_bytes: 128 * 1024,
                max_lines: 2_048,
            },
            shell_state_with_path(&bin_dir),
        )
        .await;

    match result {
        Err(EnvironmentError::CommandFailed { exit_code, stderr }) => {
            assert_eq!(exit_code, 28, "expected ENOSPC-style exit code");
            assert!(
                stderr.contains("ENOSPC"),
                "expected stderr from last-gasp packet, got: {stderr}"
            );
        }
        other => panic!("expected recovered command failure from last-gasp packet, got {other:?}"),
    }
}

#[tokio::test]
async fn chaos_cancellation_flood_still_allows_reserved_reset_slot() {
    let root = test_root("cancel-flood-reserved-reset-slot");
    let mut config = test_remote_config(&root);
    config.infrastructure_runtime.high_priority_queue_capacity = 10;
    config.reset_policy.total_reset_deadline_ms = 15_000;

    let handle = Handle::current();
    let env = Arc::new(
        QemuSshEnvironment::new(config, handle.clone(), handle)
            .expect("construct qemu ssh environment"),
    );
    env.helper_worker_stdout_stderr_local_logs
        .store(true, Ordering::Release);
    env.activate_verified_lease(Uuid::new_v4(), Duration::from_secs(30))
        .await
        .expect("activate lease for command execution");

    let bin_dir = root.join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let command_name = "flood_sleeper";
    write_executable(
        &bin_dir.join(command_name),
        "#!/usr/bin/env bash\nsleep 0.3\nexit 0\n",
    );

    let shell_state = shell_state_with_path(&bin_dir);
    let mut flood = Vec::new();
    for _ in 0..1_000 {
        let env = Arc::clone(&env);
        let shell_state = shell_state.clone();
        flood.push(tokio::spawn(async move {
            env.execute_command(
                CommandRequest {
                    program: command_name.to_string(),
                    args: Vec::new(),
                    timeout_ms: 2_000,
                    max_bytes: 8 * 1024,
                    max_lines: 256,
                },
                shell_state,
            )
            .await
        }));
    }

    tokio::time::sleep(Duration::from_millis(40)).await;

    env.reset_environment(ResetReason::ResourceExhaustion)
        .await
        .expect("reset should still acquire reserved high-priority slots under flood");

    let mut saturation_hits = 0usize;
    for task in flood {
        let outcome = task.await.expect("join flood task");
        if let Err(EnvironmentError::EnvironmentResetRequired { reason }) = outcome {
            if reason.contains("infra priority queue saturated") {
                saturation_hits += 1;
            }
        }
    }

    assert!(
        saturation_hits > 0,
        "expected at least one saturation signal during 1,000-call flood"
    );
}

#[tokio::test]
async fn chaos_pid_collision_forgery_rejected_by_binary_fingerprint() {
    #[cfg(not(target_os = "linux"))]
    {
        return;
    }

    let root = test_root("pid-collision-fingerprint");
    let mut config = test_remote_config(&root);
    config.host_artifact_gc.host_binary_sha256_or_build_id = "active-kria-binary".to_string();
    config.file_commit_policy.staging_sweep_ttl_secs = 0;
    config.file_commit_policy.staging_lease_heartbeat_timeout_ms = 0;

    let handle = Handle::current();
    let env = QemuSshEnvironment::new(config, handle.clone(), handle)
        .expect("construct qemu ssh environment");

    let staged_path = root.join("staging").join("forged_owner.upload.active");
    fs::write(&staged_path, b"forged").expect("create staged artifact");
    let sidecar_path = staged_sidecar_path(&staged_path);
    fs::write(&sidecar_path, b"{}").expect("create sidecar placeholder");

    let metadata = StagedArtifactLeaseMetadata {
        owner_instance_id: "forged-owner".to_string(),
        owner_pid: Some(std::process::id()),
        owner_pid_start_time_ticks: Some(current_process_start_time_ticks()),
        owner_binary_sha256_or_build_id: Some("forged-other-binary".to_string()),
        generation: 1,
        epoch_uuid: Uuid::new_v4(),
        artifact_nonce: "forged-nonce".to_string(),
        created_unix_ms: 0,
        lease_heartbeat_unix_ms: 0,
        expected_sha256: "deadbeef".to_string(),
        bytes: 6,
    };

    env.staged_artifact_index
        .write()
        .await
        .entry("forged-command".to_string())
        .or_default()
        .insert(staged_path.clone(), metadata);

    env.reset_environment(ResetReason::Policy)
        .await
        .expect("reset sweep should process forged artifact metadata");

    assert!(
        !staged_path.exists(),
        "forged artifact should be deleted when fingerprint triple-check fails"
    );
    assert!(
        !sidecar_path.exists(),
        "forged artifact sidecar should be deleted during sweep"
    );

    let index = env.staged_artifact_index.read().await;
    let still_indexed = index
        .get("forged-command")
        .and_then(|entries| entries.get(&staged_path))
        .is_some();
    assert!(
        !still_indexed,
        "forged artifact should be removed from index"
    );
}

#[tokio::test]
async fn lease_expiry_tainting_marks_inventory_tainted() {
    let root = test_root("lease-expiry-tainting");
    let config = test_remote_config(&root);

    let handle = Handle::current();
    let env = Arc::new(
        QemuSshEnvironment::new(config, handle.clone(), handle)
            .expect("construct qemu ssh environment"),
    );

    let pool = TargetPool::with_config(
        TargetPoolConfig {
            lease_ttl_ms: 15,
            heartbeat_grace_ms: 5,
            quarantine_cooldown_ms: 25,
        },
        SelectionWeights::default(),
        Arc::new(AdaptiveQosScheduler::with_config(Default::default())),
    );

    let target_id = TargetId::new();
    pool.add_target(
        target_id.clone(),
        Arc::clone(&env),
        TargetHealthTelemetry::default(),
    )
    .await;

    let lease = pool.acquire_lease().await.expect("acquire lease");
    tokio::time::sleep(Duration::from_millis(30)).await;

    let heartbeat = pool.heartbeat(&lease.lease_id).await;
    assert!(
        matches!(
            heartbeat,
            Err(EnvironmentError::EnvironmentResetRequired { .. })
        ),
        "expired lease heartbeat should fail closed"
    );

    let inventory_state = pool
        .inventory_state(&target_id)
        .await
        .expect("target state should exist");
    assert!(matches!(inventory_state, InventoryState::Tainted { .. }));
    assert!(
        env.tainted.load(Ordering::Acquire),
        "target environment should be tainted after lease expiry"
    );
}

#[tokio::test]
async fn snapshot_integrity_failure_taints_and_rejects_restore() {
    let root = test_root("snapshot-integrity-failure");
    let config = test_remote_config(&root);

    let handle = Handle::current();
    let env = QemuSshEnvironment::new(config, handle.clone(), handle)
        .expect("construct qemu ssh environment");

    let metadata = env
        .create_snapshot(SnapshotCreateRequest {
            label: "integrity-check".to_string(),
        })
        .await
        .expect("create snapshot");

    let payload_path = root
        .join("control")
        .join("vm_snapshots")
        .join(format!("{}.payload.json", metadata.snapshot_id.0));
    let payload = fs::read(&payload_path).expect("read snapshot payload");
    let mut payload_json: serde_json::Value =
        serde_json::from_slice(&payload).expect("deserialize payload");
    payload_json["toolchain_fingerprint"] =
        serde_json::Value::String("tampered-toolchain".to_string());
    fs::write(
        &payload_path,
        serde_json::to_vec_pretty(&payload_json).expect("serialize tampered payload"),
    )
    .expect("write tampered payload");

    let restore = env
        .restore_snapshot(SnapshotRestoreRequest {
            snapshot_id: metadata.snapshot_id,
            drift_tolerance: SnapshotDriftTolerance::default(),
        })
        .await;

    match restore {
        Err(EnvironmentError::EnvironmentResetFailed { reason, .. }) => {
            assert_eq!(reason, "snapshot_integrity_mismatch");
        }
        other => panic!("expected snapshot integrity mismatch, got {other:?}"),
    }

    assert!(
        env.tainted.load(Ordering::Acquire),
        "environment should be tainted after integrity failure"
    );
}
