//! GUI Cognition cooperative cancellation + runtime pre-action guard (Task 1.2).
//!
//! This module adds the two runaway-control primitives that bound a GUI
//! Cognition turn *before each action* (Requirement 21):
//!
//! - [`GuiCancelToken`] — a cooperative, cloneable cancel handle wrapping
//!   [`tokio_util::sync::CancellationToken`] (the same idiom the service
//!   orchestrator uses) plus a sanitized human-readable reason. The workflow
//!   loop checks it *before each action*; a cancellation therefore halts the
//!   turn before the next action executes (Requirement 21.1).
//! - [`GuiCancelRegistry`] — a process-local registry keyed by an opaque turn
//!   key (the desktop layer uses the `session_id`) so the UI / a Tauri command
//!   can request cancellation of the *active* turn without threading a handle
//!   through every call site. A process-global instance is exposed via
//!   [`gui_cancel_registry`], mirroring the [`crate::safety::global_halt`]
//!   master kill-switch pattern.
//!
//! The pre-action guard ([`evaluate_pre_action_guard`]) folds together the two
//! stop signals the runtime honors before each action:
//!   1. [`crate::safety::is_halted`] — the existing GlobalSafetyHalt master
//!      kill-switch (Requirement 21.2). It always wins over a per-turn cancel.
//!   2. the per-turn [`GuiCancelToken`] (Requirement 21.1).
//!
//! Enforcement of these loop-level checks is gated behind the
//! `gui_cog_runtime_guards` flag ([`GuiRuntimeGuardConfig::is_enforced`]); when
//! the flag is OFF (the default) the guard reports [`PreActionGuard::Proceed`]
//! and existing Step 1–12 behavior is preserved. Note the GlobalSafetyHalt is
//! *also* enforced at the GUI action backend regardless of this flag (it returns
//! `HALT`), so turning the flag off never removes the master kill-switch — it
//! only changes whether the loop stops *early* with a clear, sanitized reason.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, RwLock};

use tokio_util::sync::CancellationToken;

use super::perception::sanitize_gui_text;
use super::turn_budget::GuiRuntimeGuardConfig;

/// Max length of a sanitized cancel/halt reason surfaced in events.
const REASON_LIMIT: usize = 200;

/// Default reason used when a cancel is requested without an explicit message.
pub const DEFAULT_CANCEL_REASON: &str = "turn cancelled by user";
/// Default reason used when GlobalSafetyHalt is engaged without a recorded reason.
pub const DEFAULT_HALT_REASON: &str = "global safety halt engaged";

/// A cooperative cancellation handle for a single GUI Cognition turn.
///
/// Cloneable: all clones share the same underlying cancellation state and
/// reason, so the desktop layer can hand one clone to the runtime and keep
/// another in the [`GuiCancelRegistry`] for the cancel API.
#[derive(Debug, Clone)]
pub struct GuiCancelToken {
    token: CancellationToken,
    reason: Arc<RwLock<Option<String>>>,
}

impl GuiCancelToken {
    /// Create a fresh, un-cancelled token.
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
            reason: Arc::new(RwLock::new(None)),
        }
    }

    /// Request cancellation, recording a sanitized reason.
    ///
    /// Idempotent: the first recorded reason is preserved so the original cause
    /// is not overwritten by a later redundant cancel. The raw reason is
    /// sanitized so no untrusted prompt/screen text leaks into events.
    pub fn cancel(&self, reason: &str) {
        let cleaned = sanitize_gui_text(reason, REASON_LIMIT).text;
        let cleaned = if cleaned.trim().is_empty() {
            DEFAULT_CANCEL_REASON.to_string()
        } else {
            cleaned
        };
        if let Ok(mut guard) = self.reason.write() {
            if guard.is_none() {
                *guard = Some(cleaned);
            }
        }
        self.token.cancel();
    }

    /// Whether cancellation has been requested.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// The sanitized cancellation reason, if cancelled with one.
    pub fn reason(&self) -> Option<String> {
        self.reason.read().ok().and_then(|guard| guard.clone())
    }

    /// Borrow the underlying [`CancellationToken`] for `select!`-style awaiting.
    #[inline]
    pub fn raw(&self) -> &CancellationToken {
        &self.token
    }
}

impl Default for GuiCancelToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Process-local registry of active turn cancel tokens, keyed by an opaque turn
/// key (the desktop layer uses the `session_id`).
///
/// The registry never blocks the runtime: a poisoned lock simply degrades to a
/// no-op (the cooperative cancel is best-effort, the master kill-switch remains
/// the hard guarantee).
#[derive(Debug, Default)]
pub struct GuiCancelRegistry {
    tokens: Mutex<HashMap<String, GuiCancelToken>>,
}

impl GuiCancelRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a fresh cancel token for `key`, replacing any prior token.
    /// Returns the token clone to hand to the runtime for this turn.
    pub fn register(&self, key: &str) -> GuiCancelToken {
        let token = GuiCancelToken::new();
        self.register_token(key, token.clone());
        token
    }

    /// Register an existing token under `key` (replacing any prior token).
    pub fn register_token(&self, key: &str, token: GuiCancelToken) {
        if let Ok(mut map) = self.tokens.lock() {
            map.insert(key.to_string(), token);
        }
    }

    /// Request cancellation of the active turn registered under `key`.
    ///
    /// Returns `true` if a token was found and cancelled, `false` if no active
    /// turn is registered for that key.
    pub fn request_cancel(&self, key: &str, reason: &str) -> bool {
        if let Ok(map) = self.tokens.lock() {
            if let Some(token) = map.get(key) {
                token.cancel(reason);
                return true;
            }
        }
        false
    }

    /// Whether an active (registered) turn exists for `key`.
    pub fn is_active(&self, key: &str) -> bool {
        self.tokens
            .lock()
            .map(|map| map.contains_key(key))
            .unwrap_or(false)
    }

    /// Remove the token for `key` (call when the turn finishes).
    pub fn unregister(&self, key: &str) {
        if let Ok(mut map) = self.tokens.lock() {
            map.remove(key);
        }
    }
}

/// Process-global cancel registry, mirroring the GlobalSafetyHalt pattern so the
/// desktop cancel command can reach the active turn without extra plumbing.
pub fn gui_cancel_registry() -> &'static GuiCancelRegistry {
    static REGISTRY: LazyLock<GuiCancelRegistry> = LazyLock::new(GuiCancelRegistry::new);
    &REGISTRY
}

/// Outcome of the pre-action guard evaluated *before each action* in the loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreActionGuard {
    /// No stop signal: the loop may execute the next action.
    Proceed,
    /// GlobalSafetyHalt is engaged (master kill-switch). No further action runs.
    Halted {
        /// Sanitized halt reason for the outcome/events.
        reason: String,
    },
    /// The per-turn cancel token was tripped. No further action runs.
    Cancelled {
        /// Sanitized cancel reason for the outcome/events.
        reason: String,
    },
}

impl PreActionGuard {
    /// Whether the loop must stop before the next action.
    pub fn should_abort(&self) -> bool {
        !matches!(self, PreActionGuard::Proceed)
    }

    /// A short, stable cause tag for events (`"global_safety_halt"`/`"cancelled"`).
    pub fn cause(&self) -> &'static str {
        match self {
            PreActionGuard::Proceed => "proceed",
            PreActionGuard::Halted { .. } => "global_safety_halt",
            PreActionGuard::Cancelled { .. } => "cancelled",
        }
    }

    /// The sanitized reason string, if the guard aborts.
    pub fn reason(&self) -> Option<&str> {
        match self {
            PreActionGuard::Proceed => None,
            PreActionGuard::Halted { reason } | PreActionGuard::Cancelled { reason } => {
                Some(reason.as_str())
            }
        }
    }
}

/// Evaluate the pre-action guard for the current turn.
///
/// When `gui_cog_runtime_guards` is OFF this always returns
/// [`PreActionGuard::Proceed`] (existing behavior preserved). When ON it checks,
/// in priority order: the GlobalSafetyHalt master kill-switch (Requirement
/// 21.2), then the cooperative per-turn cancel token (Requirement 21.1).
pub fn evaluate_pre_action_guard(
    guards: &GuiRuntimeGuardConfig,
    cancel: Option<&GuiCancelToken>,
) -> PreActionGuard {
    evaluate_pre_action_guard_with(guards, cancel, crate::safety::is_halted, || {
        crate::safety::halt_reason()
    })
}

/// Testable core of [`evaluate_pre_action_guard`] with injectable halt probes,
/// so the priority/flag logic can be unit-tested without touching the global
/// halt flag.
pub fn evaluate_pre_action_guard_with<H, R>(
    guards: &GuiRuntimeGuardConfig,
    cancel: Option<&GuiCancelToken>,
    is_halted: H,
    halt_reason: R,
) -> PreActionGuard
where
    H: Fn() -> bool,
    R: Fn() -> Option<String>,
{
    if !guards.is_enforced() {
        return PreActionGuard::Proceed;
    }
    // GlobalSafetyHalt is the master kill-switch: it always wins.
    if is_halted() {
        let reason = halt_reason()
            .map(|raw| sanitize_gui_text(&raw, REASON_LIMIT).text)
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_HALT_REASON.to_string());
        return PreActionGuard::Halted { reason };
    }
    if let Some(token) = cancel {
        if token.is_cancelled() {
            let reason = token.reason().unwrap_or_else(|| DEFAULT_CANCEL_REASON.to_string());
            return PreActionGuard::Cancelled { reason };
        }
    }
    PreActionGuard::Proceed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::gui_cognition::turn_budget::TurnBudget;

    fn guards_on() -> GuiRuntimeGuardConfig {
        GuiRuntimeGuardConfig::enabled(TurnBudget::default())
    }

    fn guards_off() -> GuiRuntimeGuardConfig {
        GuiRuntimeGuardConfig::default()
    }

    #[test]
    fn token_starts_uncancelled() {
        let token = GuiCancelToken::new();
        assert!(!token.is_cancelled());
        assert_eq!(token.reason(), None);
    }

    #[test]
    fn cancel_records_first_reason_only() {
        let token = GuiCancelToken::new();
        token.cancel("user pressed stop");
        assert!(token.is_cancelled());
        assert_eq!(token.reason().as_deref(), Some("user pressed stop"));
        // Second cancel does not overwrite the original cause.
        token.cancel("something else");
        assert_eq!(token.reason().as_deref(), Some("user pressed stop"));
    }

    #[test]
    fn cancel_with_empty_reason_falls_back_to_default() {
        let token = GuiCancelToken::new();
        token.cancel("   ");
        assert_eq!(token.reason().as_deref(), Some(DEFAULT_CANCEL_REASON));
    }

    #[test]
    fn registry_register_then_cancel_active_turn() {
        let registry = GuiCancelRegistry::new();
        let token = registry.register("session-xyz");
        assert!(registry.is_active("session-xyz"));
        assert!(!token.is_cancelled());

        assert!(registry.request_cancel("session-xyz", "stop button"));
        assert!(token.is_cancelled());
        assert_eq!(token.reason().as_deref(), Some("stop button"));
    }

    #[test]
    fn registry_cancel_unknown_key_is_false() {
        let registry = GuiCancelRegistry::new();
        assert!(!registry.request_cancel("missing", "stop"));
    }

    #[test]
    fn registry_unregister_removes_token() {
        let registry = GuiCancelRegistry::new();
        registry.register("session-1");
        assert!(registry.is_active("session-1"));
        registry.unregister("session-1");
        assert!(!registry.is_active("session-1"));
        assert!(!registry.request_cancel("session-1", "stop"));
    }

    #[test]
    fn guard_off_always_proceeds_even_when_halted_or_cancelled() {
        let token = GuiCancelToken::new();
        token.cancel("stop");
        let guard = evaluate_pre_action_guard_with(
            &guards_off(),
            Some(&token),
            || true,
            || Some("halted".into()),
        );
        assert_eq!(guard, PreActionGuard::Proceed);
        assert!(!guard.should_abort());
    }

    #[test]
    fn guard_on_reports_halt_before_cancel() {
        // Both halt and cancel set; halt (master kill-switch) wins.
        let token = GuiCancelToken::new();
        token.cancel("user stop");
        let guard = evaluate_pre_action_guard_with(
            &guards_on(),
            Some(&token),
            || true,
            || Some("sidecar crashed".into()),
        );
        assert_eq!(guard.cause(), "global_safety_halt");
        assert_eq!(guard.reason(), Some("sidecar crashed"));
    }

    #[test]
    fn guard_on_reports_cancel_when_not_halted() {
        let token = GuiCancelToken::new();
        token.cancel("user stop");
        let guard =
            evaluate_pre_action_guard_with(&guards_on(), Some(&token), || false, || None);
        assert_eq!(guard.cause(), "cancelled");
        assert_eq!(guard.reason(), Some("user stop"));
        assert!(guard.should_abort());
    }

    #[test]
    fn guard_on_proceeds_when_no_signal() {
        let token = GuiCancelToken::new();
        let guard =
            evaluate_pre_action_guard_with(&guards_on(), Some(&token), || false, || None);
        assert_eq!(guard, PreActionGuard::Proceed);
    }

    #[test]
    fn guard_on_halt_uses_default_reason_when_unknown() {
        let guard = evaluate_pre_action_guard_with(&guards_on(), None, || true, || None);
        assert_eq!(guard.cause(), "global_safety_halt");
        assert_eq!(guard.reason(), Some(DEFAULT_HALT_REASON));
    }
}
