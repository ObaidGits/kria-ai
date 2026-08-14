//! linux-os-control-production **Task 2.1** — "Migrate audio volume and add
//! getters/mute" migrated these tests off the direct-subprocess `set_volume`
//! handler. **Task 2.2** — "Migrate brightness and prepare display provider
//! seam" did the same for `set_brightness`/`get_display_state`.
//!
//! `SetVolume`/`SetAudioMute`/`GetAudioState`/`SetBrightness`/
//! `GetDisplayState` no longer call `wpctl`/`pactl`/`amixer`/`gdbus`/
//! `brightnessctl`/`xrandr` directly (or through `ExecWrapper`); they reach
//! host effects only through the injected `OsControlRuntime` +
//! `os_control::audio::AudioControl` / `os_control::display::DisplayControl`
//! provider (see `tools/system_config.rs::os_audio_unavailable` /
//! `os_display_unavailable`). Until a live provider is composed into the
//! runtime by a desktop/server startup root, the handlers fail closed with the
//! frozen `Unavailable` envelope and **never** fall back to an ungoverned
//! subprocess — so a `PATH`-injected fake `wpctl`/`pactl`/`amixer`/`gdbus`/
//! `brightnessctl`/`xrandr` script (the pre-migration test technique below) is
//! never invoked. That is exactly the Task 2.1/2.2 completion proof: "No
//! audio/display handler directly invokes a process."
//!
//! The governed `AudioControl`/`DisplayControl` lifecycle itself (idempotency,
//! dispatch, verification, rollback) is covered end-to-end against a fake
//! transport in `tests/os_control_audio_lifecycle.rs` /
//! `tests/os_control_display_lifecycle.rs`.

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
async fn set_volume_never_invokes_a_process_without_a_composed_provider() {
    if !cfg!(unix) {
        return;
    }

    let sandbox = tempdir().expect("failed to create tempdir");
    let wpctl_set_log = sandbox.path().join("wpctl_set.log");
    let pactl_log = sandbox.path().join("pactl.log");
    let amixer_log = sandbox.path().join("amixer.log");

    // Every backend is scripted to succeed loudly (logging invocation) so this
    // test would fail if the handler ever launched a subprocess directly.
    let wpctl_script = format!(
        "#!/bin/sh\necho \"wpctl invoked: $*\" >> \"{}\"\necho \"Volume: 0.60\"\nexit 0\n",
        wpctl_set_log.to_string_lossy()
    );
    write_script(sandbox.path(), "wpctl", &wpctl_script);

    let pactl_script = format!(
        "#!/bin/sh\necho \"pactl invoked: $*\" >> \"{}\"\nexit 0\n",
        pactl_log.to_string_lossy()
    );
    write_script(sandbox.path(), "pactl", &pactl_script);

    let amixer_script = format!(
        "#!/bin/sh\necho \"amixer invoked: $*\" >> \"{}\"\nexit 0\n",
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

    // No provider is composed in this build: the handler fails closed with the
    // frozen `Unavailable` envelope rather than any ungoverned subprocess.
    assert!(
        !result.success,
        "set_volume without a composed provider must report Unavailable, got: {result:?}"
    );
    assert_eq!(
        result.error.as_deref(),
        Some("os_control.unavailable"),
        "unexpected error code: {result:?}"
    );
    assert!(
        !wpctl_set_log.exists(),
        "set_volume must never launch wpctl directly (Task 2.1 completion proof)"
    );
    assert!(
        !pactl_log.exists(),
        "set_volume must never launch pactl directly (Task 2.1 completion proof)"
    );
    assert!(
        !amixer_log.exists(),
        "set_volume must never launch amixer directly (Task 2.1 completion proof)"
    );
}

#[tokio::test]
#[serial]
async fn set_audio_mute_never_invokes_a_process_without_a_composed_provider() {
    if !cfg!(unix) {
        return;
    }

    let sandbox = tempdir().expect("failed to create tempdir");
    let wpctl_log = sandbox.path().join("wpctl.log");

    let wpctl_script = format!(
        "#!/bin/sh\necho \"wpctl invoked: $*\" >> \"{}\"\nexit 0\n",
        wpctl_log.to_string_lossy()
    );
    write_script(sandbox.path(), "wpctl", &wpctl_script);

    let _path_guard = PathOverrideGuard::prepend(sandbox.path());

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("set_audio_mute")
        .expect("set_audio_mute handler missing");

    let result = handler
        .execute(serde_json::json!({ "muted": true }))
        .await;

    assert!(
        !result.success,
        "set_audio_mute without a composed provider must report Unavailable, got: {result:?}"
    );
    assert!(
        !wpctl_log.exists(),
        "set_audio_mute must never launch wpctl directly (Task 2.1 completion proof)"
    );
}

#[tokio::test]
#[serial]
async fn get_audio_state_never_invokes_a_process_without_a_composed_provider() {
    if !cfg!(unix) {
        return;
    }

    let sandbox = tempdir().expect("failed to create tempdir");
    let wpctl_log = sandbox.path().join("wpctl.log");

    let wpctl_script = format!(
        "#!/bin/sh\necho \"wpctl invoked: $*\" >> \"{}\"\necho \"Volume: 0.10\"\nexit 0\n",
        wpctl_log.to_string_lossy()
    );
    write_script(sandbox.path(), "wpctl", &wpctl_script);

    let _path_guard = PathOverrideGuard::prepend(sandbox.path());

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("get_audio_state")
        .expect("get_audio_state handler missing");

    let result = handler.execute(serde_json::json!({})).await;

    assert!(
        !result.success,
        "get_audio_state without a composed provider must report Unavailable, got: {result:?}"
    );
    assert!(
        !wpctl_log.exists(),
        "get_audio_state must never launch wpctl directly (Task 2.1 completion proof)"
    );
}

#[tokio::test]
#[serial]
async fn set_brightness_never_invokes_a_process_without_a_composed_provider() {
    if !cfg!(unix) {
        return;
    }

    let sandbox = tempdir().expect("failed to create tempdir");
    let gdbus_log = sandbox.path().join("gdbus.log");
    let brightnessctl_log = sandbox.path().join("brightnessctl.log");
    let xrandr_log = sandbox.path().join("xrandr.log");

    // Every backend is scripted to succeed loudly (logging invocation) so this
    // test would fail if the handler ever launched a subprocess directly.
    let gdbus_script = format!(
        "#!/bin/sh\necho \"gdbus invoked: $*\" >> \"{}\"\necho \"(<int32 60>,)\"\nexit 0\n",
        gdbus_log.to_string_lossy()
    );
    write_script(sandbox.path(), "gdbus", &gdbus_script);

    let brightnessctl_script = format!(
        "#!/bin/sh\necho \"brightnessctl invoked: $*\" >> \"{}\"\necho 150\nexit 0\n",
        brightnessctl_log.to_string_lossy()
    );
    write_script(sandbox.path(), "brightnessctl", &brightnessctl_script);

    let xrandr_script = format!(
        "#!/bin/sh\necho \"xrandr invoked: $*\" >> \"{}\"\nexit 0\n",
        xrandr_log.to_string_lossy()
    );
    write_script(sandbox.path(), "xrandr", &xrandr_script);

    let _path_guard = PathOverrideGuard::prepend(sandbox.path());

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("set_brightness")
        .expect("set_brightness handler missing");

    let result = handler
        .execute(serde_json::json!({
            "level": 60
        }))
        .await;

    // No provider is composed in this build: the handler fails closed with the
    // frozen `Unavailable` envelope rather than any ungoverned subprocess.
    assert!(
        !result.success,
        "set_brightness without a composed provider must report Unavailable, got: {result:?}"
    );
    assert_eq!(
        result.error.as_deref(),
        Some("os_control.unavailable"),
        "unexpected error code: {result:?}"
    );
    assert!(
        !gdbus_log.exists(),
        "set_brightness must never launch gdbus directly (Task 2.2 completion proof)"
    );
    assert!(
        !brightnessctl_log.exists(),
        "set_brightness must never launch brightnessctl directly (Task 2.2 completion proof)"
    );
    assert!(
        !xrandr_log.exists(),
        "set_brightness must never launch xrandr directly (Task 2.2 completion proof)"
    );
}

#[tokio::test]
#[serial]
async fn get_display_state_never_invokes_a_process_without_a_composed_provider() {
    if !cfg!(unix) {
        return;
    }

    let sandbox = tempdir().expect("failed to create tempdir");
    let gdbus_log = sandbox.path().join("gdbus.log");

    let gdbus_script = format!(
        "#!/bin/sh\necho \"gdbus invoked: $*\" >> \"{}\"\necho \"(<int32 10>,)\"\nexit 0\n",
        gdbus_log.to_string_lossy()
    );
    write_script(sandbox.path(), "gdbus", &gdbus_script);

    let _path_guard = PathOverrideGuard::prepend(sandbox.path());

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("get_display_state")
        .expect("get_display_state handler missing");

    let result = handler.execute(serde_json::json!({})).await;

    assert!(
        !result.success,
        "get_display_state without a composed provider must report Unavailable, got: {result:?}"
    );
    assert!(
        !gdbus_log.exists(),
        "get_display_state must never launch gdbus directly (Task 2.2 completion proof)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 2.3 — "Migrate Wi-Fi and power-profile controls" migrated these tests
// off the direct-subprocess `toggle_wifi`/`connect_wifi`/`get_wifi_networks`/
// `set_power_plan` handlers.
//
// `ToggleWifi`/`ConnectWifi`/`GetWifiNetworks`/`SetPowerPlan`/`GetPowerPlan` no
// longer call `nmcli`/`powerprofilesctl` directly (or through `ExecWrapper`);
// they reach host effects only through the injected `OsControlRuntime` +
// `os_control::connectivity::ConnectivityControl` /
// `os_control::power::PowerControl` provider (see
// `tools/system_config.rs::os_connectivity_unavailable` /
// `os_power_unavailable`). Until a live provider is composed into the runtime
// by a desktop/server startup root, the handlers fail closed with the frozen
// `Unavailable` envelope and **never** fall back to an ungoverned subprocess —
// so a `PATH`-injected fake `nmcli`/`powerprofilesctl` script (the
// pre-migration test technique above) is never invoked. That is exactly the
// Task 2.3 completion proof: "No connectivity/power handler directly invokes a
// process."
//
// The governed `ConnectivityControl`/`PowerControl` lifecycle itself
// (idempotency, dispatch, verification, rollback, duplicate-SSID
// clarification, secret non-leakage) is covered end-to-end against a fake
// transport in `tests/os_control_connectivity_lifecycle.rs` /
// `tests/os_control_power_lifecycle.rs`.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn toggle_wifi_never_invokes_a_process_without_a_composed_provider() {
    if !cfg!(unix) {
        return;
    }

    let sandbox = tempdir().expect("failed to create tempdir");
    let nmcli_log = sandbox.path().join("nmcli.log");

    let nmcli_script = format!(
        "#!/bin/sh\necho \"nmcli invoked: $*\" >> \"{}\"\necho \"enabled\"\nexit 0\n",
        nmcli_log.to_string_lossy()
    );
    write_script(sandbox.path(), "nmcli", &nmcli_script);

    let _path_guard = PathOverrideGuard::prepend(sandbox.path());

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("toggle_wifi")
        .expect("toggle_wifi handler missing");

    let result = handler
        .execute(serde_json::json!({ "enable": true }))
        .await;

    assert!(
        !result.success,
        "toggle_wifi without a composed provider must report Unavailable, got: {result:?}"
    );
    assert!(
        !nmcli_log.exists(),
        "toggle_wifi must never launch nmcli directly (Task 2.3 completion proof)"
    );
}

#[tokio::test]
#[serial]
async fn connect_wifi_never_invokes_a_process_without_a_composed_provider() {
    if !cfg!(unix) {
        return;
    }

    let sandbox = tempdir().expect("failed to create tempdir");
    let nmcli_log = sandbox.path().join("nmcli.log");

    let nmcli_script = format!(
        "#!/bin/sh\necho \"nmcli invoked: $*\" >> \"{}\"\nexit 0\n",
        nmcli_log.to_string_lossy()
    );
    write_script(sandbox.path(), "nmcli", &nmcli_script);

    let _path_guard = PathOverrideGuard::prepend(sandbox.path());

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("connect_wifi")
        .expect("connect_wifi handler missing");

    let result = handler
        .execute(serde_json::json!({ "ssid": "HomeNet", "password": "hunter2" }))
        .await;

    assert!(
        !result.success,
        "connect_wifi without a composed provider must report Unavailable, got: {result:?}"
    );
    assert!(
        !nmcli_log.exists(),
        "connect_wifi must never launch nmcli directly (Task 2.3 completion proof)"
    );
    // The password must never leak into the (non-existent) result either.
    let serialized = serde_json::to_string(&result).expect("result serializes");
    assert!(!serialized.contains("hunter2"), "password leaked into tool result");
}

#[tokio::test]
#[serial]
async fn get_wifi_networks_never_invokes_a_process_without_a_composed_provider() {
    if !cfg!(unix) {
        return;
    }

    let sandbox = tempdir().expect("failed to create tempdir");
    let nmcli_log = sandbox.path().join("nmcli.log");

    let nmcli_script = format!(
        "#!/bin/sh\necho \"nmcli invoked: $*\" >> \"{}\"\necho \"HomeNet:AA\\:BB\\:CC\\:DD\\:EE\\:01:80:WPA2\"\nexit 0\n",
        nmcli_log.to_string_lossy()
    );
    write_script(sandbox.path(), "nmcli", &nmcli_script);

    let _path_guard = PathOverrideGuard::prepend(sandbox.path());

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("get_wifi_networks")
        .expect("get_wifi_networks handler missing");

    let result = handler.execute(serde_json::json!({})).await;

    assert!(
        !result.success,
        "get_wifi_networks without a composed provider must report Unavailable, got: {result:?}"
    );
    assert!(
        !nmcli_log.exists(),
        "get_wifi_networks must never launch nmcli directly (Task 2.3 completion proof)"
    );
}

#[tokio::test]
#[serial]
async fn set_power_plan_never_invokes_a_process_without_a_composed_provider() {
    if !cfg!(unix) {
        return;
    }

    let sandbox = tempdir().expect("failed to create tempdir");
    let powerprofilesctl_log = sandbox.path().join("powerprofilesctl.log");

    let script = format!(
        "#!/bin/sh\necho \"powerprofilesctl invoked: $*\" >> \"{}\"\necho \"balanced\"\nexit 0\n",
        powerprofilesctl_log.to_string_lossy()
    );
    write_script(sandbox.path(), "powerprofilesctl", &script);

    let _path_guard = PathOverrideGuard::prepend(sandbox.path());

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("set_power_plan")
        .expect("set_power_plan handler missing");

    let result = handler
        .execute(serde_json::json!({ "plan": "performance" }))
        .await;

    assert!(
        !result.success,
        "set_power_plan without a composed provider must report Unavailable, got: {result:?}"
    );
    assert!(
        !powerprofilesctl_log.exists(),
        "set_power_plan must never launch powerprofilesctl directly (Task 2.3 completion proof)"
    );
}

#[tokio::test]
#[serial]
async fn get_power_plan_never_invokes_a_process_without_a_composed_provider() {
    if !cfg!(unix) {
        return;
    }

    let sandbox = tempdir().expect("failed to create tempdir");
    let powerprofilesctl_log = sandbox.path().join("powerprofilesctl.log");

    let script = format!(
        "#!/bin/sh\necho \"powerprofilesctl invoked: $*\" >> \"{}\"\necho \"balanced\"\nexit 0\n",
        powerprofilesctl_log.to_string_lossy()
    );
    write_script(sandbox.path(), "powerprofilesctl", &script);

    let _path_guard = PathOverrideGuard::prepend(sandbox.path());

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("get_power_plan")
        .expect("get_power_plan handler missing");

    let result = handler.execute(serde_json::json!({})).await;

    assert!(
        !result.success,
        "get_power_plan without a composed provider must report Unavailable, got: {result:?}"
    );
    assert!(
        !powerprofilesctl_log.exists(),
        "get_power_plan must never launch powerprofilesctl directly (Task 2.3 completion proof)"
    );
}
