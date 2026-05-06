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
async fn maximize_window_idempotency_skips_mutation_when_already_maximized() {
    if !cfg!(unix) {
        return;
    }

    let sandbox = tempdir().expect("failed to create tempdir");
    let wmctrl_mutate_log = sandbox.path().join("wmctrl_mutate.log");

    let wmctrl_script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"-l\" ]; then\n  echo \"0x01200007  0 host Demo Window\"\n  exit 0\nfi\nif [ \"$1\" = \"-r\" ]; then\n  echo \"wmctrl mutation invoked\" >> \"{}\"\n  exit 0\nfi\necho \"unexpected wmctrl args: $*\" >&2\nexit 1\n",
        wmctrl_mutate_log.to_string_lossy()
    );
    write_script(sandbox.path(), "wmctrl", &wmctrl_script);

    write_script(
        sandbox.path(),
        "xprop",
        "#!/bin/sh\nif [ \"$1\" = \"-id\" ] && [ \"$3\" = \"_NET_WM_STATE\" ]; then\n  echo \"_NET_WM_STATE(ATOM) = _NET_WM_STATE_MAXIMIZED_VERT, _NET_WM_STATE_MAXIMIZED_HORZ\"\n  exit 0\nfi\necho \"unexpected xprop args: $*\" >&2\nexit 1\n",
    );

    let _path_guard = PathOverrideGuard::prepend(sandbox.path());

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("maximize_window")
        .expect("maximize_window handler missing");

    let result = handler
        .execute(serde_json::json!({
            "title": "Demo Window"
        }))
        .await;

    assert!(result.success, "expected idempotent success: {result:?}");
    assert_eq!(result.data["changed"].as_bool(), Some(false));
    assert_eq!(
        result.data["already_in_desired_state"].as_bool(),
        Some(true)
    );
    assert!(
        !wmctrl_mutate_log.exists(),
        "wmctrl mutation must not run when maximize pre-flight shows already maximized"
    );
}

#[tokio::test]
#[serial]
async fn move_window_surfaces_wmctrl_stderr_for_missing_window() {
    if !cfg!(unix) {
        return;
    }

    let sandbox = tempdir().expect("failed to create tempdir");

    write_script(
        sandbox.path(),
        "wmctrl",
        "#!/bin/sh\nif [ \"$1\" = \"-lG\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"-r\" ]; then\n  echo \"Cannot find window: $2\" >&2\n  exit 1\nfi\necho \"unexpected wmctrl args: $*\" >&2\nexit 1\n",
    );

    let _path_guard = PathOverrideGuard::prepend(sandbox.path());

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("move_window")
        .expect("move_window handler missing");

    let result = handler
        .execute(serde_json::json!({
            "title": "Missing Window",
            "x": 100,
            "y": 200
        }))
        .await;

    assert!(
        !result.success,
        "expected failure for missing window: {result:?}"
    );

    let error = result.error.unwrap_or_default();
    assert!(
        error.contains("Cannot find window: Missing Window"),
        "stderr from wmctrl should bubble up, got: {error}"
    );
}
