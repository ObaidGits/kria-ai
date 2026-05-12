//! RFC 008: Global Safety Halt — Master Kill Switch for GUI Automation
//!
//! A process-wide atomic flag that prevents the HTN executor from issuing
//! any further tool calls. When set:
//!  - `execute_workflow` exits its loop on the next iteration with an abort error
//!  - All GUI tool calls return `GuiError::IpcError("HALT")`
//!  - The uinput daemon socket connection is intentionally closed (handled
//!    by orchestrator) which triggers the daemon's dead-man's switch.
//!
//! This flag is the single source of truth for "is automation allowed?".
//! Set by:
//!  - User toggling the GUI Automation switch in the UI
//!  - Orchestrator detecting a sidecar crash
//!  - Emergency shutdown handlers
//!
//! Cleared by:
//!  - User toggling the GUI Automation switch back on (after services are
//!    confirmed healthy by orchestrator)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

/// Global halt flag. Defaults to `false` (automation allowed).
static GLOBAL_HALT: AtomicBool = AtomicBool::new(false);

/// Most recent halt reason. Used to surface a meaningful explanation
/// in workflow error messages instead of just "GLOBAL_SAFETY_HALT".
/// `None` means the halt has never been engaged (or has been released).
static HALT_REASON: RwLock<Option<String>> = RwLock::new(None);

/// Returns `true` if GUI automation is currently halted.
///
/// All input-issuing code paths (`execute_workflow`, tool handlers for
/// `type_text`/`click_mouse`/`press_shortcut`) must check this before
/// dispatching to the OS.
#[inline]
pub fn is_halted() -> bool {
    GLOBAL_HALT.load(Ordering::SeqCst)
}

/// Returns the most recent halt reason, if any.
///
/// Useful for surfacing actionable diagnostics in error messages
/// (e.g. "vision sidecar still starting" vs "user disabled automation").
pub fn halt_reason() -> Option<String> {
    HALT_REASON.read().ok().and_then(|guard| guard.clone())
}

/// Set the halt flag. Returns the previous value.
fn set_halt(value: bool) -> bool {
    GLOBAL_HALT.swap(value, Ordering::SeqCst)
}

/// Engage the global halt — block all further automation tool calls.
///
/// Idempotent. Safe to call from any thread, signal handler, or Drop impl.
/// The most recent reason is always recorded (overwriting any prior reason)
/// so error messages reflect the latest cause.
pub fn engage_halt(reason: &str) {
    let was_halted = set_halt(true);
    if let Ok(mut guard) = HALT_REASON.write() {
        *guard = Some(reason.to_string());
    }
    if !was_halted {
        tracing::error!(
            target: "global_halt",
            reason = %reason,
            "🛑 GLOBAL SAFETY HALT ENGAGED — all GUI automation blocked"
        );
    } else {
        tracing::debug!(
            target: "global_halt",
            reason = %reason,
            "halt reason updated (already engaged)"
        );
    }
}

/// Release the global halt — allow automation tool calls again.
///
/// Should only be called after the orchestrator has verified both the
/// vision sidecar and uinput daemon are healthy.
pub fn release_halt(reason: &str) {
    let was_halted = set_halt(false);
    if let Ok(mut guard) = HALT_REASON.write() {
        *guard = None;
    }
    if was_halted {
        tracing::info!(
            target: "global_halt",
            reason = %reason,
            "✅ Global safety halt released — automation re-enabled"
        );
    }
}

/// Convenience: returns a `Result::Err` if halted, `Ok(())` otherwise.
pub fn check_or_halt() -> Result<(), &'static str> {
    if is_halted() {
        Err("GLOBAL_SAFETY_HALT")
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn test_default_not_halted() {
        // Reset state in case other tests left it set
        release_halt("test reset");
        assert!(!is_halted());
        assert!(check_or_halt().is_ok());
    }

    #[test]
    #[serial]
    fn test_engage_and_release() {
        release_halt("test reset");
        assert!(!is_halted());

        engage_halt("unit test");
        assert!(is_halted());
        assert!(check_or_halt().is_err());

        release_halt("unit test cleanup");
        assert!(!is_halted());
    }

    #[test]
    #[serial]
    fn test_idempotent() {
        release_halt("reset");
        engage_halt("first");
        engage_halt("second"); // should not panic
        assert!(is_halted());
        release_halt("cleanup");
    }
}
