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
async fn install_package_idempotency_noop_when_already_installed() {
    if !cfg!(unix) {
        return;
    }

    let sandbox = tempdir().expect("failed to create tempdir");
    let apt_get_log = sandbox.path().join("apt_get.log");
    let pkexec_log = sandbox.path().join("pkexec.log");

    write_script(
        sandbox.path(),
        "dpkg-query",
        "#!/bin/sh\necho \"install ok installed 1.2.3\"\nexit 0\n",
    );

    let apt_get_script = format!(
        "#!/bin/sh\necho \"apt-get invoked\" >> \"{}\"\nexit 0\n",
        apt_get_log.to_string_lossy()
    );
    write_script(sandbox.path(), "apt-get", &apt_get_script);

    let pkexec_script = format!(
        "#!/bin/sh\necho \"pkexec invoked\" >> \"{}\"\nexit 1\n",
        pkexec_log.to_string_lossy()
    );
    write_script(sandbox.path(), "pkexec", &pkexec_script);

    let _path_guard = PathOverrideGuard::prepend(sandbox.path());

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("install_package")
        .expect("install_package handler missing");

    let result = handler
        .execute(serde_json::json!({
            "name": "demo-pkg",
            "source": "apt"
        }))
        .await;

    assert!(result.success, "expected idempotent success: {result:?}");
    assert_eq!(result.data["changed"].as_bool(), Some(false));
    assert_eq!(
        result.data["already_in_desired_state"].as_bool(),
        Some(true)
    );
    assert!(
        !apt_get_log.exists(),
        "install apply command should not run when package is already installed"
    );
    assert!(
        !pkexec_log.exists(),
        "privilege escalation should not run when pre-flight idempotency short-circuits"
    );
}

#[tokio::test]
#[serial]
async fn install_package_surfaces_stderr_on_apply_failure() {
    if !cfg!(unix) {
        return;
    }

    let sandbox = tempdir().expect("failed to create tempdir");
    let apt_get_log = sandbox.path().join("apt_get.log");

    write_script(
        sandbox.path(),
        "dpkg-query",
        "#!/bin/sh\necho \"not installed\" >&2\nexit 1\n",
    );

    let apt_get_script = format!(
        "#!/bin/sh\necho \"apt-get install attempted\" >> \"{}\"\necho \"E: Unable to locate package demo-pkg\" >&2\nexit 1\n",
        apt_get_log.to_string_lossy()
    );
    write_script(sandbox.path(), "apt-get", &apt_get_script);

    write_script(
        sandbox.path(),
        "pkexec",
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo \"pkexec 1.0\"\n  exit 0\nfi\nexec \"$@\"\n",
    );

    let _path_guard = PathOverrideGuard::prepend(sandbox.path());

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("install_package")
        .expect("install_package handler missing");

    let result = handler
        .execute(serde_json::json!({
            "name": "demo-pkg",
            "source": "apt"
        }))
        .await;

    assert!(
        !result.success,
        "expected failure when apt-get returns non-zero: {result:?}"
    );

    let error = result.error.unwrap_or_default();
    assert!(
        error.contains("E: Unable to locate package demo-pkg"),
        "stderr should be surfaced in ToolResult::err, got: {error}"
    );

    assert!(
        apt_get_log.exists(),
        "apply command should be attempted when pre-flight reports package absent"
    );
}
