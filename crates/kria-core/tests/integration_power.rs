//! linux-os-control-production **Task 2.4** — "Migrate lock, suspend,
//! hibernate, shutdown and reboot" migrated these tests off the
//! direct-subprocess `lock_screen`/`sleep`/`hibernate`/`shutdown_system`/
//! `reboot_system` handlers.
//!
//! `LockScreen`/`Sleep`/`Hibernate`/`ShutdownSystem`/`RebootSystem` no longer
//! call `loginctl`/`systemctl`/`shutdown`/`reboot` directly (or through
//! `sh -c`/`tokio::process::Command`/`vm_dispatch_command_with_sudo`); they
//! reach host effects only through the injected `OsControlRuntime` +
//! `os_control::power::session::PowerSessionControl` provider (see
//! `tools/power.rs::os_power_session_unavailable`). Until a live provider is
//! composed into the runtime by a desktop/server startup root, the handlers
//! fail closed with the frozen `Unavailable` envelope and **never** fall back
//! to an ungoverned subprocess or a `sudo` privilege-escalation path — so a
//! `PATH`-injected fake `loginctl`/`systemctl`/`shutdown`/`reboot` script (the
//! pre-migration test technique below) is never invoked. That is exactly the
//! Task 2.4 completion proof: "`power.rs` contains no Linux shell command
//! strings."
//!
//! The governed `PowerSessionControl` lifecycle itself (lock verification via
//! `LockedHint`, accepted semantics for suspend/hibernate/shutdown/reboot,
//! hibernate-availability probing, no-rollback-claim) is covered end-to-end
//! against a fake transport in `tests/os_control_session_lifecycle.rs`.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use serial_test::serial;
use tempfile::tempdir;

struct PathOverrideGuard {
    original: Option<OsString>,
}

impl PathOverrideGuard {
    fn prepend(path: &Path) -> Self {
        let original = env::var_os("PATH");
        let mut entries = vec![path.to_path_buf()];
        if let Some(ref value) = original {
            entries.extend(env::split_paths(value));
        }

        let joined = env::join_paths(entries).expect("failed to join PATH entries");
        // SAFETY: tests are serialized, so process-wide PATH mutation is scoped safely.
        unsafe { env::set_var("PATH", joined) };

        Self { original }
    }
}

impl Drop for PathOverrideGuard {
    fn drop(&mut self) {
        match self.original.take() {
            Some(value) => {
                // SAFETY: restores process PATH at the end of serialized test scope.
                unsafe { env::set_var("PATH", value) }
            }
            None => {
                // SAFETY: restores process PATH at the end of serialized test scope.
                unsafe { env::remove_var("PATH") }
            }
        }
    }
}

#[cfg(unix)]
fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join(name);
    fs::write(&path, body).expect("failed to write script");

    let mut perms = fs::metadata(&path)
        .expect("failed to stat script")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).expect("failed to chmod script");

    path
}

/// Install fake `loginctl`, `systemctl`, `shutdown`, and `reboot` scripts on
/// `PATH` that log their invocation loudly. If any migrated power handler
/// ever launched a subprocess directly (or through a shell), one of these
/// logs would appear.
fn install_fake_power_binaries(sandbox: &Path) -> Vec<PathBuf> {
    let mut logs = Vec::new();
    for name in ["loginctl", "systemctl", "shutdown", "reboot"] {
        let log = sandbox.join(format!("{name}.log"));
        let script = format!(
            "#!/bin/sh\necho \"{name} invoked: $*\" >> \"{}\"\nexit 0\n",
            log.to_string_lossy()
        );
        write_script(sandbox, name, &script);
        logs.push(log);
    }
    logs
}

async fn assert_never_invokes_a_process(tool: &str, params: serde_json::Value) {
    if !cfg!(unix) {
        return;
    }

    let sandbox = tempdir().expect("failed to create tempdir");
    let logs = install_fake_power_binaries(sandbox.path());
    let _path_guard = PathOverrideGuard::prepend(sandbox.path());

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg.get_handler(tool).unwrap_or_else(|| panic!("{tool} handler missing"));

    let result = handler.execute(params).await;

    assert!(
        !result.success,
        "{tool} without a composed provider must report Unavailable, got: {result:?}"
    );
    for log in &logs {
        assert!(
            !log.exists(),
            "{tool} must never launch a subprocess directly (Task 2.4 completion proof); \
             found log: {}",
            log.display()
        );
    }
}

#[tokio::test]
#[serial]
async fn lock_screen_never_invokes_a_process_without_a_composed_provider() {
    assert_never_invokes_a_process("lock_screen", serde_json::json!({})).await;
}

#[tokio::test]
#[serial]
async fn sleep_never_invokes_a_process_without_a_composed_provider() {
    assert_never_invokes_a_process("sleep", serde_json::json!({})).await;
}

#[tokio::test]
#[serial]
async fn hibernate_never_invokes_a_process_without_a_composed_provider() {
    assert_never_invokes_a_process("hibernate", serde_json::json!({})).await;
}

#[tokio::test]
#[serial]
async fn shutdown_system_never_invokes_a_process_without_a_composed_provider() {
    assert_never_invokes_a_process(
        "shutdown_system",
        serde_json::json!({ "delay_minutes": 5 }),
    )
    .await;
}

#[tokio::test]
#[serial]
async fn reboot_system_never_invokes_a_process_without_a_composed_provider() {
    assert_never_invokes_a_process("reboot_system", serde_json::json!({})).await;
}
