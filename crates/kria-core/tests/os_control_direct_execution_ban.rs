//! Task 2.6 — "Delete superseded direct execution paths" (OSC-001, OSC-002,
//! OSC-035).
//!
//! A permanent, automated source-level policy test proving that the OS tool
//! facades migrated by Tasks 2.1–2.5 (`tools/system_config.rs`,
//! `tools/power.rs`, and the migrated portions of `tools/process.rs`,
//! `tools/app_lifecycle.rs`, `tools/interaction.rs`, `tools/communication.rs`,
//! and `tools/scheduler.rs`) never regress back to a direct-execution path:
//! `sh -c`, a raw `tokio::process::Command`/`std::process::Command`,
//! `ExecWrapper`, or a local VM-dispatch call. This is the "Code-level
//! validation" required by Task 2.6's completion proof — a static scan, not a
//! runtime behavior test, so (unlike the deny-live suites) it is **not**
//! gated behind `os-control-test` and runs under plain `cargo test -p
//! kria-core`.
//!
//! # Scope and exclusions
//!
//! Each target file may contain a small, explicit, documented set of tools
//! that are legitimately **not** part of this spec's migrated OS-control
//! surface — either because they are non-OS-control tools outside the
//! frozen manifest (`screenshot`, `type_text`, `focus_window`'s
//! wmctrl-based app-focus logic, `hash_text`, the pre-migration
//! `open_application`/`open_url`/`browser_search`/`send_message` family that
//! is still owned by `IntentDispatcher` and explicitly deferred to Task
//! 3.3 — see `os_control::applications` module docs), or because the
//! manifest itself explicitly defers them to a later task
//! (`create_scheduled_task`/`delete_scheduled_task` → Task 4.5;
//! `get_active_connections` → out of scope per the legacy-difference
//! report). Each exclusion below is a narrow named span (one struct + its
//! impl block), never a blanket file exemption, and the exact source marker
//! it anchors to is documented inline.
//!
//! Every *other* line of these files — including every migrated handler's
//! `execute`/`execute_with_context` body — is scanned and must be free of the
//! banned patterns.

/// Read a `kria-core` source file by its path relative to the crate root.
fn read_src(rel: &str) -> String {
    let path = format!("{}/{}", env!("CARGO_MANIFEST_DIR"), rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// Remove the source span starting at `start_marker` (inclusive) up to, but
/// not including, the nearest of `stop_markers` that appears after it. Used
/// to carve out a narrow, documented, named exclusion (one struct + its impl
/// block) that is explicitly not part of this task's migrated OS-control
/// surface. Panics if `start_marker` cannot be found, so a future edit that
/// renames/removes the excluded tool forces this allowlist to be revisited
/// rather than silently widening its own exemption.
fn remove_span(src: &str, start_marker: &str, stop_markers: &[&str]) -> String {
    let Some(start) = src.find(start_marker) else {
        panic!(
            "exclusion marker `{start_marker}` not found — the source changed; \
             update this allowlist to match"
        );
    };
    let after_start = start + start_marker.len();
    let Some(stop_offset) = stop_markers
        .iter()
        .filter_map(|m| src[after_start..].find(m))
        .min()
    else {
        panic!(
            "none of the stop markers {stop_markers:?} were found after `{start_marker}` — \
             update this allowlist to match"
        );
    };
    let stop = after_start + stop_offset;
    format!("{}{}", &src[..start], &src[stop..])
}

/// Drop every line whose trimmed content starts with a `//` comment marker
/// (covers `//`, `///`, and `//!`). This codebase uses only line comments in
/// these tool files (no block comments), and several migrated handlers'
/// doc comments *narrate* the removal of `sh -c`/`ExecWrapper`/direct
/// `Command`/`vm_dispatch` (e.g. "no longer build a `sh -c` string") —
/// scanning those literal prose mentions as if they were code would produce
/// false positives, so comment lines are excluded from the pattern scan.
fn strip_comment_lines(src: &str) -> String {
    src.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

const BANNED_PATTERNS: &[&str] = &[
    "tokio::process::Command::new",
    "std::process::Command::new",
    "ExecWrapper",
    "vm_dispatch",
];

/// Assert that `code_only` (already comment-stripped, already scrubbed of
/// documented exclusions) contains none of the banned direct-execution
/// patterns.
fn assert_no_direct_execution(file_label: &str, code_only: &str) {
    for pattern in BANNED_PATTERNS {
        assert!(
            !code_only.contains(pattern),
            "{file_label}: found banned direct-execution pattern `{pattern}` in migrated \
             OS-control tool code — host effects must reach through the injected \
             OsControlRuntime, never a direct subprocess/ExecWrapper/VM-dispatch call \
             (Task 2.6 completion proof)"
        );
    }
    // A raw shell-interpreter invocation (`sh -c "…"` / `bash -c "…"`) is
    // banned even if it is not built through `Command::new` directly (e.g. a
    // format!()-constructed command string handed to some other exec path).
    for shell_literal in ["sh -c", "bash -c"] {
        assert!(
            !code_only.contains(shell_literal),
            "{file_label}: found shell-interpreter literal `{shell_literal}` in migrated \
             OS-control tool code (Task 2.6 completion proof)"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 2.1/2.2/2.3 — audio, display, connectivity, power-profile.
// No exclusions: every handler in this file (including the retained
// `get/set/list_environment_variable` tools, OSC-035.4 out-of-scope but
// process-free) is already free of direct-execution patterns.
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn system_config_facade_has_no_direct_execution_path() {
    let src = read_src("src/tools/system_config.rs");
    let code_only = strip_comment_lines(&src);
    assert_no_direct_execution("tools/system_config.rs", &code_only);
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 2.4 — lock, suspend, hibernate, shutdown, reboot.
// No exclusions: the whole file is migrated and contains no process-spawning
// code at all (Task 2.4's own completion proof: "power.rs contains no Linux
// shell command strings").
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn power_facade_has_no_direct_execution_path() {
    let src = read_src("src/tools/power.rs");
    let code_only = strip_comment_lines(&src);
    assert_no_direct_execution("tools/power.rs", &code_only);
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 2.5 — processes: `set_process_priority` is migrated and scanned.
// Excluded: `GetActiveConnections` (`get_active_connections`) — a read-only
// network-connections diagnostic view, a distinct subsystem from the
// canonical `ConnectivityControl` DTOs, explicitly recorded as out of scope
// for the v1 OS-control manifest in the legacy-difference report (§4).
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn process_facade_has_no_direct_execution_path_outside_documented_exclusions() {
    let src = read_src("src/tools/process.rs");
    let scrubbed = remove_span(&src, "struct GetActiveConnections", &["pub fn register("]);
    let code_only = strip_comment_lines(&scrubbed);
    assert_no_direct_execution("tools/process.rs", &code_only);
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 2.5 — applications: `graceful_close_application` (`CloseApplication`)
// and `kill_process` (`KillProcess`) are migrated and scanned.
// Excluded (all pre-migration, owned by the existing `IntentDispatcher` /
// `InstalledAppRegistry` composition and explicitly deferred to Task 3.3 per
// `os_control::applications` module docs — never claimed as migrated by
// Task 2.5):
//   * `OpenApplication` — its best-effort "focus existing window" fallback
//     uses `wmctrl`/`xdotool` directly.
//   * `LegacyOpenApplicationWithFile` / `LegacyOpenApplication` — stateless
//     fallback handlers used only when `register_with_dispatcher` is called
//     with `dispatcher: None` (early startup / tests), documented in the
//     file's own "Legacy stubs (no dispatcher)" section.
//   * `FocusWindow` (`focus_window`) — non-canonical, non-OS-control window
//     tool that shells out to `wmctrl` directly.
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn app_lifecycle_facade_has_no_direct_execution_path_outside_documented_exclusions() {
    let src = read_src("src/tools/app_lifecycle.rs");
    let scrubbed = remove_span(
        &src,
        "struct OpenApplication {",
        &["struct OpenApplicationWithFile {"],
    );
    let scrubbed = remove_span(
        &scrubbed,
        "struct LegacyOpenApplicationWithFile;",
        &["struct OpenUrl {"],
    );
    let scrubbed = remove_span(
        &scrubbed,
        "struct LegacyOpenApplication;",
        &["struct LegacyOpenUrl;"],
    );
    let scrubbed = remove_span(
        &scrubbed,
        "struct FocusWindow;",
        &["fn os_process_unavailable("],
    );
    let code_only = strip_comment_lines(&scrubbed);
    assert_no_direct_execution("tools/app_lifecycle.rs", &code_only);
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 2.5 — interaction: `get_clipboard`/`set_clipboard`/
// `transform_clipboard` are migrated and scanned.
// Excluded (non-OS-control tools outside the frozen manifest scope, per Task
// 2.6's own instructions):
//   * `Screenshot` (`screenshot`) — shells out to `maim`/`scrot`/
//     `gnome-screenshot`/`import` directly.
//   * `TypeText` (`type_text`) — shells out to `xdotool` directly.
// `HashText`/`TransformText` are also out of scope but contain no
// direct-execution pattern, so no exclusion span is needed for them.
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn interaction_facade_has_no_direct_execution_path_outside_documented_exclusions() {
    let src = read_src("src/tools/interaction.rs");
    let scrubbed = remove_span(&src, "struct Screenshot;", &["struct TypeText;"]);
    let scrubbed = remove_span(&scrubbed, "struct TypeText;", &["#[cfg(test)]"]);
    let code_only = strip_comment_lines(&scrubbed);
    assert_no_direct_execution("tools/interaction.rs", &code_only);
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 2.5 — communication: `send_notification` is migrated and scanned.
// No exclusions: `compose_email` (opens a `mailto:` link via `open::that`)
// and `schedule_reminder` (an in-process timer that delivers through the
// same governed `send_notification` route) contain no direct-execution
// pattern either.
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn communication_facade_has_no_direct_execution_path() {
    let src = read_src("src/tools/communication.rs");
    let code_only = strip_comment_lines(&src);
    assert_no_direct_execution("tools/communication.rs", &code_only);
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 2.5 — scheduler: `list_scheduled_tasks` is migrated and scanned.
//
// The former exclusions are GONE, and deliberately so. `CreateScheduledTask` and
// `DeleteScheduledTask` used to write a `crontab` pipe directly, with no policy,
// grant, lease, audit or verification. They were deleted rather than migrated,
// because a scheduled task that can run an arbitrary command later is a
// persistent arbitrary-execution hole that outlives the session. Task 4.5 owns
// the typed replacement.
//
// So this file is now scanned WHOLE with no scrubbing — a stricter check than the
// one it replaces. If a direct-execution path ever reappears here, this fails.
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn scheduler_facade_has_no_direct_execution_path_outside_documented_exclusions() {
    let src = read_src("src/tools/scheduler.rs");
    let code_only = strip_comment_lines(&src);
    assert_no_direct_execution("tools/scheduler.rs", &code_only);
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 3.4 — packages: `search_package`/`get_package_info`/
// `list_installed_packages`/`plan_package_changes`/`install_package`/
// `uninstall_package`/`check_system_updates`/`get_reboot_required` are
// migrated and scanned. No exclusions: the previous ~1700-line direct-
// execution implementation (apt/dnf/pacman/zypper/brew/winget/choco/snap/
// flatpak subprocess calls plus the ad-hoc `PrivEsc`/`pkexec`/`sudo`
// privilege-escalation machinery) is deleted outright, not excluded — every
// handler in this file reaches host effects only through the injected
// `OsControlRuntime`.
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn packages_facade_has_no_direct_execution_path() {
    let src = read_src("src/tools/packages.rs");
    let code_only = strip_comment_lines(&src);
    assert_no_direct_execution("tools/packages.rs", &code_only);
    // The ad-hoc privilege-escalation machinery this task deletes must not
    // reappear either.
    for pattern in ["pkexec", "PrivEsc", "\"sudo\""] {
        assert!(
            !code_only.contains(pattern),
            "tools/packages.rs: found deleted privilege-escalation pattern `{pattern}` — \
             privileged package mutation must dispatch exclusively through \
             `BrokerOperation::ApplyPackagePlan` (Task 3.4 completion proof)"
        );
    }
}

/// Sanity check on the test itself: every exclusion marker above must still
/// resolve to a struct that actually contains a banned pattern in the *full*
/// (non-scrubbed) source. If a future migration removes the direct-execution
/// code from one of these deferred tools, this test would start passing for
/// the wrong reason (an exclusion that no longer excludes anything) — this
/// guards that the allowlist stays meaningful rather than accumulating stale
/// entries.
#[test]
fn documented_exclusions_still_correspond_to_real_direct_execution_code() {
    let cases: &[(&str, &[&str])] = &[
        ("src/tools/process.rs", &["tokio::process::Command::new"]),
        (
            "src/tools/app_lifecycle.rs",
            &["tokio::process::Command::new"],
        ),
        (
            "src/tools/interaction.rs",
            &["tokio::process::Command::new"],
        ),
        // `src/tools/scheduler.rs` is deliberately ABSENT: its two direct-crontab
        // handlers were deleted, so it now has no exclusion to keep honest.
    ];
    for (rel, patterns) in cases {
        let src = read_src(rel);
        let code_only = strip_comment_lines(&src);
        for pattern in *patterns {
            assert!(
                code_only.contains(pattern),
                "{rel}: expected the documented exclusions to still cover a real \
                 `{pattern}` occurrence — if this file was cleaned up, narrow or remove \
                 the corresponding exclusion in os_control_direct_execution_ban.rs"
            );
        }
    }
}
