//! kria-test-app — disposable GUI stand-in for KRIA's safe E2E automation
//! tests. Defined in `docs/GUI_INTELLIGENCE_REVIEW.md` Appendix D.2.
//!
//! # Hard rules
//!
//! 1. This binary MUST refuse to run against the user's real desktop session.
//!    It only runs when `DISPLAY` points at an Xvfb/Xephyr instance (`:99`,
//!    `:100`, …), or when `KRIA_TEST_APP_FORCE=1` is explicitly set for
//!    developers debugging via Xephyr.
//! 2. All file writes go to `$KRIA_TEST_DUMP_PATH` (must be set, must be
//!    under `$TMPDIR`).
//! 3. No network. No `sudo`. No persistent state.
//!
//! This skeleton currently only enforces those guards and exits 0. The real
//! GUI (a `eframe`/`egui` window with text entry, Save button, dialog
//! spawner, OCR-injection mode, and hidden-focus mode) lands in phases
//! P5/P6 when the safe E2E harness is wired up.

use anyhow::{bail, Result};
use std::env;

fn main() -> Result<()> {
    enforce_sandbox_guards()?;
    eprintln!(
        "kria-test-app: skeleton stub ok. \
         Real GUI lands with P5/P6 of docs/GUI_INTELLIGENCE_REVIEW.md."
    );
    Ok(())
}

/// Refuse to run unless we are clearly in a sandboxed virtual display, or
/// the operator explicitly opts in via `KRIA_TEST_APP_FORCE=1`.
fn enforce_sandbox_guards() -> Result<()> {
    if env::var("KRIA_TEST_APP_FORCE").as_deref() == Ok("1") {
        eprintln!("kria-test-app: KRIA_TEST_APP_FORCE=1 set, skipping DISPLAY check");
        return validate_dump_path();
    }

    let display = env::var("DISPLAY").unwrap_or_default();
    if display.is_empty() {
        bail!(
            "kria-test-app refuses to start: no DISPLAY set. Run under Xvfb \
             with DISPLAY=:99 or set KRIA_TEST_APP_FORCE=1 for nested Xephyr."
        );
    }

    // Allow only :99..:199 (Xvfb/Xephyr conventional range) — explicitly
    // forbid :0/:1 (real desktop sessions).
    let trimmed = display.trim_start_matches(':');
    let display_num: u32 = trimmed
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if display_num < 99 {
        bail!(
            "kria-test-app refuses to start: DISPLAY='{}' looks like a real \
             desktop session. Use Xvfb/Xephyr at :99 or higher.",
            display
        );
    }

    validate_dump_path()
}

fn validate_dump_path() -> Result<()> {
    let dump = env::var("KRIA_TEST_DUMP_PATH").unwrap_or_default();
    if dump.is_empty() {
        bail!("kria-test-app refuses to start: KRIA_TEST_DUMP_PATH must be set");
    }
    let tmpdir = env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    if !dump.starts_with(&tmpdir) && !dump.starts_with("/tmp/") {
        bail!(
            "kria-test-app refuses to start: KRIA_TEST_DUMP_PATH='{}' must \
             live under TMPDIR ('{}') or /tmp",
            dump,
            tmpdir
        );
    }
    Ok(())
}
