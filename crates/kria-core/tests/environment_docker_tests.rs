use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use bollard::Docker;
use kria_core::infra::environment::{
    CommandExecutor, CommandRequest, DockerEnvironment, DockerEnvironmentConfig,
    EnvironmentError, EnvironmentLifecycle, EnvironmentResetKind, FileSystemOps, ReadFileRequest,
    ShellState, WriteFileRequest,
};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root from CARGO_MANIFEST_DIR")
        .to_path_buf()
}

async fn ensure_docker_running() {
    let docker = Docker::connect_with_local_defaults().expect("connect to Docker daemon");
    docker.ping().await.expect("Docker daemon ping failed");
}

fn make_config(memory_mb: i64, pids_limit: i64) -> DockerEnvironmentConfig {
    let root = workspace_root();
    let mut cfg = DockerEnvironmentConfig::from_env();
    cfg.image = DockerEnvironmentConfig::default().image;
    cfg.network_mode = "bridge".to_string();
    cfg.workspace_tmpfs_size_mb = 128;
    cfg.memory_mb = memory_mb;
    cfg.cpus = 1.0;
    cfg.pids_limit = pids_limit;
    cfg.seccomp_profile = root.join("config/seccomp/kria-seccomp.json");
    cfg.network_policy_check_script = root.join("scripts/setup-kria-net.sh");
    cfg.container_name_prefix = "kria-env-test".to_string();
    cfg.inject_host_uid_gid = true;
    cfg
}

async fn setup_docker_provider(memory_mb: i64, pids_limit: i64) -> DockerEnvironment {
    // SAFETY: This test binary is run with --test-threads=1, so process-wide env mutation is scoped.
    unsafe {
        std::env::set_var("KRIA_EXEC_DOCKER_NETWORK_NAME", "bridge");
        std::env::set_var("KRIA_EXEC_DOCKER_NETWORK_POLICY_MODE", "internal");
        std::env::set_var("KRIA_EXEC_DOCKER_READONLY_ROOTFS", "false");
    }

    ensure_docker_running().await;

    let env = DockerEnvironment::new(make_config(memory_mb, pids_limit))
        .expect("create DockerEnvironment test instance");
    env.ensure_ready()
        .await
        .expect("DockerEnvironment ensure_ready failed");
    env
}

fn default_shell_state() -> ShellState {
    ShellState {
        cwd: PathBuf::from("."),
        env_vars: HashMap::new(),
        generation: 0,
    }
}

fn shell_state_with_env(extra: &[(&str, &str)]) -> ShellState {
    let mut shell_state = default_shell_state();
    for (key, value) in extra {
        shell_state
            .env_vars
            .insert((*key).to_string(), (*value).to_string());
    }
    shell_state
}

fn request(
    program: &str,
    args: &[&str],
    timeout_ms: u64,
    max_bytes: usize,
    max_lines: usize,
) -> CommandRequest {
    CommandRequest {
        program: program.to_string(),
        args: args.iter().map(|arg| (*arg).to_string()).collect(),
        timeout_ms,
        max_bytes,
        max_lines,
    }
}

#[tokio::test]
async fn docker_basic_execution_test() {
    let env = setup_docker_provider(512, 128).await;

    let result = env
        .execute_command(
            request("/bin/sh", &["-lc", "echo -n basic-ok"], 10_000, 16 * 1024, 1_024),
            default_shell_state(),
        )
        .await
        .expect("basic command execution should succeed");

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "basic-ok");
    assert!(result.stderr.is_empty());

    env.shutdown().await.expect("shutdown after basic test");
}

#[tokio::test]
async fn docker_env_var_injection_test() {
    let env = setup_docker_provider(512, 128).await;

    let result = env
        .execute_command(
            request(
                "/bin/sh",
                &["-lc", "printf '%s' \"$KRIA_PHASE5_TEST_ENV\""],
                10_000,
                16 * 1024,
                1_024,
            ),
            shell_state_with_env(&[("KRIA_PHASE5_TEST_ENV", "env-injected")]),
        )
        .await
        .expect("env var injection command should succeed");

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "env-injected");

    env.shutdown().await.expect("shutdown after env var test");
}

#[tokio::test]
async fn docker_archive_io_tmpfs_test() {
    let env = setup_docker_provider(512, 128).await;

    let rel_path = PathBuf::from("tmpfs-note.txt");
    let payload = b"archive-roundtrip-ok".to_vec();

    let write_result = env
        .write_file(WriteFileRequest {
            path: rel_path.clone(),
            contents: payload.clone(),
            create_parent: false,
        })
        .await
        .expect("write_file should succeed with archive upload API");

    assert_eq!(write_result.bytes_written, payload.len());

    let read_result = env
        .read_file(ReadFileRequest {
            path: rel_path.clone(),
        })
        .await
        .expect("read_file should succeed with archive download API");

    assert_eq!(read_result.contents, payload);

    env.shutdown().await.expect("shutdown after archive IO test");
}

#[tokio::test]
async fn docker_output_flood_control_test() {
    let env = setup_docker_provider(512, 128).await;

    let err = env
        .execute_command(
            request(
                "/bin/sh",
                &[
                    "-lc",
                    "i=0; while [ \"$i\" -lt 200 ]; do echo flood-line-$i; i=$((i+1)); done",
                ],
                10_000,
                512,
                20,
            ),
            default_shell_state(),
        )
        .await
        .expect_err("output flood should hit configured output limits");

    match err {
        EnvironmentError::OutputLimitExceeded {
            max_bytes,
            max_lines,
            observed_bytes,
            observed_lines,
        } => {
            assert_eq!(max_bytes, 512);
            assert_eq!(max_lines, 20);
            assert!(
                observed_bytes > max_bytes || observed_lines > max_lines,
                "expected observed output to exceed at least one configured limit"
            );
        }
        other => panic!("expected OutputLimitExceeded, got: {other:?}"),
    }

    env.shutdown()
        .await
        .expect("shutdown after output flood control test");
}

#[tokio::test]
async fn docker_memory_limit_oom_test() {
    let env = setup_docker_provider(64, 128).await;
    let mut resets = env.subscribe_environment_resets();

    // SIGKILL produces OOM-style exit code 137, which should trigger the OOM fallback path.
    let err = env
        .execute_command(
            request("/bin/sh", &["-lc", "kill -9 $$"], 10_000, 16 * 1024, 1_024),
            default_shell_state(),
        )
        .await
        .expect_err("OOM-style command should trigger ContainerOomKilled fallback");

    match err {
        EnvironmentError::ContainerOomKilled { exit_code } => {
            assert_eq!(exit_code, Some(137));
        }
        other => panic!("expected ContainerOomKilled fallback, got: {other:?}"),
    }

    let reset_event = tokio::time::timeout(Duration::from_secs(3), resets.recv())
        .await
        .expect("timed out waiting for OOM reset event")
        .expect("failed to receive OOM reset event");
    assert_eq!(reset_event.kind, EnvironmentResetKind::Oom);

    let recovered = env
        .execute_command(
            request(
                "/bin/sh",
                &["-lc", "echo -n post-oom-recovered"],
                10_000,
                16 * 1024,
                1_024,
            ),
            default_shell_state(),
        )
        .await
        .expect("environment should recover after OOM fallback recycle");
    assert_eq!(recovered.stdout, "post-oom-recovered");

    env.shutdown().await.expect("shutdown after OOM test");
}

#[tokio::test]
async fn docker_pid_limit_exhaustion_test() {
    let env = setup_docker_provider(512, 8).await;
    let mut resets = env.subscribe_environment_resets();

    let initial = env
        .execute_command(
            request("/bin/true", &[], 10_000, 8 * 1024, 256),
            default_shell_state(),
        )
        .await;

    let pid_err = match initial {
        Err(err @ EnvironmentError::PidLimitExceeded { .. }) => err,
        Ok(_) => {
            env.execute_command(
                request(
                    "/bin/sh",
                    &[
                        "-lc",
                        "set -e; i=0; while [ \"$i\" -lt 512 ]; do sleep 30 & i=$((i+1)); done",
                    ],
                    10_000,
                    16 * 1024,
                    2_048,
                ),
                default_shell_state(),
            )
            .await
            .expect_err("expected PID exhaustion fallback from background spawn loop")
        }
        Err(other) => panic!("expected PID limit fallback, got: {other:?}"),
    };

    match pid_err {
        EnvironmentError::PidLimitExceeded { limit } => assert_eq!(limit, 8),
        other => panic!("expected PidLimitExceeded, got: {other:?}"),
    }

    let reset_event = tokio::time::timeout(Duration::from_secs(3), resets.recv())
        .await
        .expect("timed out waiting for PID-limit reset event")
        .expect("failed to receive PID-limit reset event");
    assert_eq!(reset_event.kind, EnvironmentResetKind::PidLimit);

    let recovered = env
        .execute_command(
            request(
                "/bin/sh",
                &["-lc", "echo -n post-pid-recovered"],
                10_000,
                16 * 1024,
                1_024,
            ),
            default_shell_state(),
        )
        .await
        .expect("environment should recover after PID limit fallback recycle");
    assert_eq!(recovered.stdout, "post-pid-recovered");

    env.shutdown().await.expect("shutdown after PID limit test");
}
