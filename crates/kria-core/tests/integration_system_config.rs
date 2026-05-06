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

#[tokio::test]
#[serial]
async fn set_volume_idempotency_skips_apply_when_already_set() {
    if !cfg!(unix) {
        return;
    }

    let sandbox = tempdir().expect("failed to create tempdir");
    let wpctl_set_log = sandbox.path().join("wpctl_set.log");
    let pactl_log = sandbox.path().join("pactl.log");
    let amixer_log = sandbox.path().join("amixer.log");

    let wpctl_script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"get-volume\" ]; then\n  echo \"Volume: 0.60\"\n  exit 0\nfi\nif [ \"$1\" = \"set-volume\" ]; then\n  echo \"wpctl set invoked\" >> \"{}\"\n  exit 0\nfi\necho \"unexpected args: $*\" >&2\nexit 1\n",
        wpctl_set_log.to_string_lossy()
    );
    write_script(sandbox.path(), "wpctl", &wpctl_script);

    let pactl_script = format!(
        "#!/bin/sh\necho \"pactl invoked\" >> \"{}\"\nexit 1\n",
        pactl_log.to_string_lossy()
    );
    write_script(sandbox.path(), "pactl", &pactl_script);

    let amixer_script = format!(
        "#!/bin/sh\necho \"amixer invoked\" >> \"{}\"\nexit 1\n",
        amixer_log.to_string_lossy()
    );
    write_script(sandbox.path(), "amixer", &amixer_script);

    let _path_guard = PathOverrideGuard::prepend(sandbox.path());

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("set_volume")
        .expect("set_volume handler missing");

    let result = handler
        .execute(serde_json::json!({
            "level": 60
        }))
        .await;

    assert!(result.success, "expected idempotent success: {result:?}");
    assert_eq!(result.data["changed"].as_bool(), Some(false));
    assert_eq!(
        result.data["already_in_desired_state"].as_bool(),
        Some(true)
    );
    assert!(
        !wpctl_set_log.exists(),
        "set-volume apply must not run when pre-flight volume already matches"
    );
    assert!(
        !pactl_log.exists(),
        "fallback apply must not run when idempotency short-circuits"
    );
    assert!(
        !amixer_log.exists(),
        "fallback apply must not run when idempotency short-circuits"
    );
}

#[tokio::test]
#[serial]
async fn set_volume_surfaces_stderr_when_all_backends_fail() {
    if !cfg!(unix) {
        return;
    }

    let sandbox = tempdir().expect("failed to create tempdir");

    write_script(
        sandbox.path(),
        "wpctl",
        "#!/bin/sh\nif [ \"$1\" = \"get-volume\" ]; then\n  echo \"Volume: 0.10\"\n  exit 0\nfi\necho \"wpctl backend hard failure\" >&2\nexit 1\n",
    );

    write_script(
        sandbox.path(),
        "pactl",
        "#!/bin/sh\necho \"pactl fallback failure\" >&2\nexit 1\n",
    );

    write_script(
        sandbox.path(),
        "amixer",
        "#!/bin/sh\necho \"amixer fallback failure\" >&2\nexit 1\n",
    );

    let _path_guard = PathOverrideGuard::prepend(sandbox.path());

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("set_volume")
        .expect("set_volume handler missing");

    let result = handler
        .execute(serde_json::json!({
            "level": 80
        }))
        .await;

    assert!(
        !result.success,
        "expected failure when all volume backends fail: {result:?}"
    );

    let error = result.error.unwrap_or_default();
    assert!(
        error.contains("wpctl backend hard failure"),
        "stderr from primary backend should bubble up, got: {error}"
    );
    assert!(
        error.contains("pactl fallback failure"),
        "stderr from fallback backend should be present, got: {error}"
    );
}
