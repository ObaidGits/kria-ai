//! Task 8.1 (Requirements 6, 7, 8) — clipboard-safe cross-app helper.
//!
//! Cross-app clipboard combos (Task 8.2: copy in a browser → switch → paste in
//! an editor) borrow the **system clipboard** as a transport. That clipboard is
//! the user's: whatever they had copied there before the turn must come back
//! afterwards (Requirement 8 — *user clipboard restored*). This module provides
//! the SAVE → USE → RESTORE primitive that guarantees it, with SERIALIZED access
//! so two concurrent turns can never interleave their save/restore and clobber
//! each other's saved value.
//!
//! The whole module is gated behind the `gui_cog_crossapp` feature flag
//! ([`GuiCrossAppConfig`], default OFF until the Task 8.5 gate). While the flag
//! is OFF none of this code runs, so the executor / runtime path is
//! byte-for-byte unchanged.
//!
//! ## Design — deterministic, testable, no real display needed
//!
//! The OS clipboard sits behind the [`ClipboardBackend`] trait (read / write),
//! so tests drive an in-memory fake — no real clipboard, X11/Wayland display, or
//! `arboard` handle required (CI-safe). The save/restore lifecycle is modeled as
//! a [`ClipboardSession`]: acquiring it captures the prior clipboard value, and
//! dropping (or [`ClipboardSession::restore`]-ing) it writes that prior value
//! back — even if the cross-app operation panics or returns early, `Drop`
//! restores. [`with_clipboard`] wraps the whole save → use → restore in one
//! call.
//!
//! ## Serialization (Requirement 8)
//!
//! A single **process-wide** guard ([`CLIPBOARD_LOCK`]) serializes clipboard
//! sessions: a `ClipboardSession` holds the lock for its entire save → use →
//! restore lifetime, so a second turn that wants the clipboard waits until the
//! first turn has restored the original contents. We use a `std` mutex (not an
//! async one) deliberately: a clipboard session is a short, blocking critical
//! section, and a `std` guard's RAII `Drop` is exactly what ties "release the
//! lock" to "restored the user's value" without needing an async runtime in the
//! deterministic T2 tests.
//!
//! ## Secrets (Requirement 8 / privacy)
//!
//! Clipboard contents may be a password or other secret. This module treats the
//! value as **opaque**: it is never logged, never put in `Debug`, and never
//! placed in a surfaced summary. [`ClipboardSession::saved_summary`] /
//! [`clipboard_value_summary`] return only a non-revealing shape (present +
//! length bucket), never the bytes.

use std::sync::{Mutex, MutexGuard, OnceLock};

/// Environment variable that enables the `gui_cog_crossapp` flag (Task 8).
///
/// Truthy (`1`/`true`/`yes`/`on`) turns cross-app clipboard / combos / file-
/// manager select ON. Default (unset or any other value) keeps it OFF,
/// preserving the existing executor / runtime path byte-for-byte. The wave gate
/// (Task 8.5) flips the live/desktop path to default ON.
pub const CROSSAPP_ENV_FLAG: &str = "KRIA_GUI_COG_CROSSAPP";

/// Parse a `gui_cog_crossapp` env value as truthy (`1`/`true`/`yes`/`on`).
fn crossapp_flag_truthy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// Parse a `gui_cog_crossapp` env value as an explicit falsy opt-out
/// (`0`/`false`/`no`/`off`/empty) — the documented rollback switch. An absent
/// value (`None`) is NOT falsy: the default stays ON for the default-on path.
fn crossapp_flag_falsy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("0") | Some("false") | Some("no") | Some("off") | Some("")
    )
}

/// The `gui_cog_crossapp` feature-flag bundle (default OFF) — Task 8.1.
///
/// When enabled, the cross-app clipboard helper ([`with_clipboard`] /
/// [`ClipboardSession`]) performs SAVE → USE → RESTORE with serialized access so
/// a cross-app combo never clobbers the user's clipboard. When disabled (the
/// default), the caller's prior path runs unchanged. The wave gate (Task 8.5)
/// flips this flag ON for the live/desktop path.
///
/// Mirrors the established `GuiBrowserConfig` flag pattern exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GuiCrossAppConfig {
    /// Whether cross-app clipboard / combos / file-manager select is active.
    pub enabled: bool,
}

impl Default for GuiCrossAppConfig {
    fn default() -> Self {
        // Task 8: flag default OFF until the wave gate (Task 8.5) flips it.
        Self { enabled: false }
    }
}

impl GuiCrossAppConfig {
    /// Construct an explicitly-enabled cross-app config.
    pub fn enabled() -> Self {
        Self { enabled: true }
    }

    /// Construct an explicitly-disabled cross-app config.
    pub fn disabled() -> Self {
        Self { enabled: false }
    }

    /// Derive the config from the process environment. The flag is OFF unless
    /// [`CROSSAPP_ENV_FLAG`] is truthy.
    pub fn from_env() -> Self {
        Self::from_env_lookup(|key| std::env::var(key).ok())
    }

    /// Testable core of [`from_env`](Self::from_env) with an injectable lookup.
    pub fn from_env_lookup<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        Self {
            enabled: crossapp_flag_truthy(lookup(CROSSAPP_ENV_FLAG).as_deref()),
        }
    }

    /// Derive the config from the process environment with the flag defaulting
    /// **ON** (wave gate flip, Task 8.5). Cross-app clipboard is active unless
    /// [`CROSSAPP_ENV_FLAG`] is explicitly falsy (`0`/`false`/`no`/`off`/empty),
    /// which is the documented rollback switch. An absent env value keeps the
    /// flag ON.
    pub fn from_env_default_on() -> Self {
        Self::from_env_lookup_default_on(|key| std::env::var(key).ok())
    }

    /// Testable core of [`from_env_default_on`](Self::from_env_default_on) with
    /// an injectable lookup.
    pub fn from_env_lookup_default_on<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        // Default ON: enabled unless the operator explicitly opts out via a
        // falsy env value (the rollback switch). Absent (None) is NOT falsy.
        Self {
            enabled: !crossapp_flag_falsy(lookup(CROSSAPP_ENV_FLAG).as_deref()),
        }
    }

    /// Whether cross-app clipboard / combos / file-manager select should run.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// An error from a clipboard backend (read or write). Carries only a sanitized
/// message — never the clipboard contents (Requirement 8 / privacy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardError {
    message: String,
}

impl ClipboardError {
    /// Build a backend error from a sanitized, content-free message.
    pub fn backend(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The sanitized message (never includes clipboard contents).
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "clipboard backend error: {}", self.message)
    }
}

impl std::error::Error for ClipboardError {}

/// The OS clipboard, abstracted so the save/restore lifecycle is testable with
/// an in-memory fake (no real clipboard / display needed — CI-safe).
///
/// A value of `None` represents an **empty** clipboard; `Some(text)` represents
/// text contents. Implementations MUST treat the value as opaque and MUST NOT
/// log it (Requirement 8 / privacy).
pub trait ClipboardBackend {
    /// Read the current clipboard value (`None` == empty / no text contents).
    fn read(&self) -> Result<Option<String>, ClipboardError>;

    /// Write the clipboard value. `None` clears the clipboard, restoring the
    /// "empty" state captured on acquire.
    fn write(&self, value: Option<&str>) -> Result<(), ClipboardError>;
}

/// Process-wide guard that serializes clipboard sessions (Requirement 8).
///
/// A [`ClipboardSession`] holds this lock for its whole save → use → restore
/// lifetime, so concurrent turns cannot interleave clipboard mutation. Lazily
/// initialized so it costs nothing while the `gui_cog_crossapp` flag is OFF.
fn clipboard_lock() -> &'static Mutex<()> {
    static CLIPBOARD_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    CLIPBOARD_LOCK.get_or_init(|| Mutex::new(()))
}

/// A non-revealing summary of a clipboard value for telemetry / surfaced UI.
///
/// NEVER includes the contents: returns `"<empty>"` for no contents, or
/// `"<clipboard: N chars>"` for text (length only). Used so a cross-app turn can
/// report "clipboard saved/restored" without leaking a secret (Requirement 8).
pub fn clipboard_value_summary(value: Option<&str>) -> String {
    match value {
        None => "<empty>".to_string(),
        Some(text) => format!("<clipboard: {} chars>", text.chars().count()),
    }
}

/// An acquired clipboard session: SAVE on acquire, RESTORE on drop/release.
///
/// Holds the process-wide [`clipboard_lock`] for its entire lifetime
/// (serialized access, Requirement 8). On [`ClipboardSession::acquire`] it reads
/// and captures the prior clipboard value; the caller then performs the cross-
/// app operation through the same backend; on [`ClipboardSession::restore`] or
/// `Drop` the captured prior value is written back so the user's clipboard is
/// not clobbered.
///
/// The captured value is held opaquely — there is no API that returns the bytes
/// and `Debug` never reveals them (Requirement 8 / privacy).
pub struct ClipboardSession<'a, B: ClipboardBackend> {
    backend: &'a B,
    saved: Option<String>,
    restored: bool,
    _guard: MutexGuard<'static, ()>,
}

impl<'a, B: ClipboardBackend> ClipboardSession<'a, B> {
    /// Acquire the serialized clipboard lock and SAVE the current clipboard
    /// value. Blocks until any in-flight session has restored and released.
    pub fn acquire(backend: &'a B) -> Result<Self, ClipboardError> {
        // Hold the process-wide guard for the whole save → use → restore
        // lifetime. Recover from a poisoned lock (a prior session panicking does
        // not corrupt the unit guard state).
        let guard = clipboard_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let saved = backend.read()?;
        Ok(Self {
            backend,
            saved,
            restored: false,
            _guard: guard,
        })
    }

    /// Whether the saved (prior) clipboard was empty.
    pub fn saved_was_empty(&self) -> bool {
        self.saved.is_none()
    }

    /// A non-revealing summary of the saved clipboard value (never the bytes).
    pub fn saved_summary(&self) -> String {
        clipboard_value_summary(self.saved.as_deref())
    }

    /// Explicitly RESTORE the saved clipboard value now (idempotent). Called
    /// automatically on `Drop`; exposed so a caller can surface a restore error
    /// (Drop cannot).
    pub fn restore(&mut self) -> Result<(), ClipboardError> {
        if self.restored {
            return Ok(());
        }
        let result = self.backend.write(self.saved.as_deref());
        // Mark restored regardless so Drop does not retry a failed write in a
        // path that cannot surface the error.
        self.restored = true;
        result
    }
}

impl<B: ClipboardBackend> Drop for ClipboardSession<'_, B> {
    fn drop(&mut self) {
        // Best-effort restore even if the cross-app op panicked or returned
        // early. A restore error here cannot be surfaced (Drop), so it is
        // swallowed — but never the clipboard contents (privacy).
        let _ = self.restore();
    }
}

/// Whether a clipboard session can be acquired right now WITHOUT blocking — i.e.
/// no other session currently holds the serialized lock. Intended for tests /
/// diagnostics that assert serialization; production code uses
/// [`ClipboardSession::acquire`] / [`with_clipboard`], which block.
pub fn clipboard_lock_available() -> bool {
    match clipboard_lock().try_lock() {
        Ok(_guard) => true,
        Err(std::sync::TryLockError::WouldBlock) => false,
        // A poisoned-but-free lock is still "available" (we recover from poison
        // on acquire). Anything else is treated as unavailable.
        Err(std::sync::TryLockError::Poisoned(_)) => true,
    }
}

/// Run a cross-app clipboard operation with full SAVE → USE → RESTORE and
/// serialized access (Requirement 8).
///
/// Acquires the serialized clipboard lock, saves the user's current clipboard,
/// runs `op` (which may read/write the clipboard via the same backend to copy
/// in one app and paste in another), then RESTORES the saved value — even if
/// `op` returns an error. The user's clipboard is never left clobbered.
///
/// Returns `op`'s result; a restore failure after a successful `op` is surfaced
/// as an `Err` so the caller knows the user's clipboard may not have been
/// restored.
pub fn with_clipboard<B, F, T>(backend: &B, op: F) -> Result<T, ClipboardError>
where
    B: ClipboardBackend,
    F: FnOnce(&B) -> Result<T, ClipboardError>,
{
    let mut session = ClipboardSession::acquire(backend)?;
    let op_result = op(backend);
    let restore_result = session.restore();
    match op_result {
        // Surface a restore error only when the op itself succeeded; if the op
        // failed, return the op's error (the more actionable one).
        Ok(value) => restore_result.map(|()| value),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    /// In-memory fake clipboard backend — no real clipboard / display needed.
    /// Records every value written so a test can assert nothing leaked.
    #[derive(Default)]
    struct FakeClipboard {
        value: StdMutex<Option<String>>,
        write_log: StdMutex<Vec<Option<String>>>,
        fail_write: AtomicBool,
    }

    impl FakeClipboard {
        fn with_contents(initial: Option<&str>) -> Self {
            Self {
                value: StdMutex::new(initial.map(|s| s.to_string())),
                write_log: StdMutex::new(Vec::new()),
                fail_write: AtomicBool::new(false),
            }
        }

        fn current(&self) -> Option<String> {
            self.value.lock().unwrap().clone()
        }
    }

    impl ClipboardBackend for FakeClipboard {
        fn read(&self) -> Result<Option<String>, ClipboardError> {
            Ok(self.value.lock().unwrap().clone())
        }

        fn write(&self, value: Option<&str>) -> Result<(), ClipboardError> {
            if self.fail_write.load(Ordering::SeqCst) {
                return Err(ClipboardError::backend("write failed"));
            }
            self.write_log
                .lock()
                .unwrap()
                .push(value.map(|s| s.to_string()));
            *self.value.lock().unwrap() = value.map(|s| s.to_string());
            Ok(())
        }
    }

    // ── T1: SAVE → USE → RESTORE ────────────────────────────────────────────

    #[test]
    fn t1_original_text_contents_restored_after_use() {
        let clip = FakeClipboard::with_contents(Some("USER ORIGINAL"));
        let out = with_clipboard(&clip, |backend| {
            // Simulate a cross-app combo borrowing the clipboard.
            backend.write(Some("copied from browser"))?;
            assert_eq!(backend.read()?.as_deref(), Some("copied from browser"));
            Ok(42)
        })
        .expect("clipboard op should succeed");
        assert_eq!(out, 42);
        // The user's original clipboard is restored, not clobbered.
        assert_eq!(clip.current().as_deref(), Some("USER ORIGINAL"));
    }

    #[test]
    fn t1_empty_clipboard_restored_as_empty_after_use() {
        let clip = FakeClipboard::with_contents(None);
        with_clipboard(&clip, |backend| backend.write(Some("transient")))
            .expect("clipboard op should succeed");
        // The originally-empty clipboard is restored to empty (cleared).
        assert_eq!(clip.current(), None);
    }

    #[test]
    fn t1_clipboard_restored_even_when_op_fails() {
        let clip = FakeClipboard::with_contents(Some("USER ORIGINAL"));
        let result: Result<(), ClipboardError> = with_clipboard(&clip, |backend| {
            backend.write(Some("half-done"))?;
            Err(ClipboardError::backend("op blew up"))
        });
        assert!(result.is_err(), "op error should propagate");
        // Even though the op errored mid-way, the original is restored.
        assert_eq!(clip.current().as_deref(), Some("USER ORIGINAL"));
    }

    #[test]
    fn t1_clipboard_restored_even_when_op_panics() {
        let clip = FakeClipboard::with_contents(Some("USER ORIGINAL"));
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut session = ClipboardSession::acquire(&clip).unwrap();
            clip.write(Some("transient")).unwrap();
            // Drop the explicit handle by panicking; Drop must still restore.
            let _ = &mut session;
            panic!("cross-app op exploded");
        }))
        .is_err();
        assert!(panicked, "the closure should have panicked");
        // Drop restored the original despite the panic.
        assert_eq!(clip.current().as_deref(), Some("USER ORIGINAL"));
    }

    // ── T1/T2: SERIALIZED ACCESS ─────────────────────────────────────────────

    #[test]
    fn t2_second_session_waits_for_first_to_release() {
        let clip = FakeClipboard::with_contents(Some("ORIGINAL"));
        {
            let _session = ClipboardSession::acquire(&clip).unwrap();
            // While a session is held, the serialized lock is unavailable: a
            // second session would block rather than interleave.
            assert!(
                !clipboard_lock_available(),
                "lock must be held while a session is alive"
            );
        }
        // After the session is dropped (restore complete), the lock frees.
        assert!(
            clipboard_lock_available(),
            "lock must free once the session is released"
        );
    }

    #[test]
    fn t2_concurrent_sessions_do_not_interleave_save_restore() {
        let clip = Arc::new(FakeClipboard::with_contents(Some("ORIGINAL")));
        let started = Arc::new(AtomicBool::new(false));

        let clip_a = Arc::clone(&clip);
        let started_a = Arc::clone(&started);
        let handle = std::thread::spawn(move || {
            with_clipboard(clip_a.as_ref(), |backend| {
                started_a.store(true, Ordering::SeqCst);
                // Hold the clipboard busy for a moment to force the other turn
                // to wait for serialized access.
                std::thread::sleep(std::time::Duration::from_millis(50));
                backend.write(Some("thread-A value"))
            })
        });

        // Wait until thread A holds the session.
        while !started.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }

        // This turn must WAIT for A to restore before it runs — never interleave.
        with_clipboard(clip.as_ref(), |backend| {
            assert_eq!(
                backend.read().unwrap().as_deref(),
                Some("ORIGINAL"),
                "second turn must observe the restored original, not A's transient value"
            );
            backend.write(Some("thread-B value"))
        })
        .unwrap();

        handle.join().unwrap().unwrap();
        // Both turns restored: the user's original survives both.
        assert_eq!(clip.current().as_deref(), Some("ORIGINAL"));
    }

    // ── T2: SECRET CONTENTS NOT LOGGED / SURFACED ────────────────────────────

    #[test]
    fn t2_secret_contents_never_appear_in_summary_or_debug() {
        const SECRET: &str = "hunter2-super-secret-password";
        let clip = FakeClipboard::with_contents(Some(SECRET));
        let session = ClipboardSession::acquire(&clip).unwrap();

        // The saved summary reveals shape (length), never the secret bytes.
        let summary = session.saved_summary();
        assert!(!summary.contains(SECRET), "summary must not leak the secret");
        assert!(summary.contains("chars"), "summary should report a shape");
        assert!(!session.saved_was_empty());

        // The value-summary helper likewise never reveals the bytes.
        let value_summary = clipboard_value_summary(Some(SECRET));
        assert!(!value_summary.contains(SECRET));
        assert_eq!(clipboard_value_summary(None), "<empty>");
    }

    #[test]
    fn t2_clipboard_error_message_is_content_free() {
        let err = ClipboardError::backend("could not access clipboard");
        assert!(!format!("{err}").contains("secret"));
        assert_eq!(err.message(), "could not access clipboard");
    }

    #[test]
    fn t2_restore_failure_is_surfaced_but_op_error_takes_precedence() {
        let clip = FakeClipboard::with_contents(Some("ORIGINAL"));
        clip.fail_write.store(true, Ordering::SeqCst);
        // op succeeds, but restore write fails -> surfaced as Err.
        let result: Result<(), ClipboardError> = with_clipboard(&clip, |_backend| Ok(()));
        assert!(result.is_err(), "restore failure should surface");
    }

    // ── T1: FLAG PLUMBING (default OFF + rollback) ───────────────────────────

    #[test]
    fn t1_flag_defaults_off() {
        assert!(!GuiCrossAppConfig::default().is_enabled());
        assert!(GuiCrossAppConfig::enabled().is_enabled());
        assert!(!GuiCrossAppConfig::disabled().is_enabled());
    }

    #[test]
    fn t1_from_env_lookup_default_off_unless_truthy() {
        let off = GuiCrossAppConfig::from_env_lookup(|_| None);
        assert!(!off.is_enabled(), "absent env => OFF on the default-off path");
        for falsy in ["0", "false", "no", "off", "", "garbage"] {
            let cfg = GuiCrossAppConfig::from_env_lookup(|_| Some(falsy.to_string()));
            assert!(!cfg.is_enabled(), "{falsy:?} must be OFF");
        }
        for truthy in ["1", "true", "yes", "on", "ON", "  true  "] {
            let cfg = GuiCrossAppConfig::from_env_lookup(|_| Some(truthy.to_string()));
            assert!(cfg.is_enabled(), "{truthy:?} must be ON");
        }
    }

    #[test]
    fn t1_from_env_lookup_default_on_rollback_switch() {
        // Absent => ON (the wave-gate default).
        assert!(GuiCrossAppConfig::from_env_lookup_default_on(|_| None).is_enabled());
        // Explicit falsy => the documented rollback switch (OFF).
        for falsy in ["0", "false", "no", "off", ""] {
            let cfg = GuiCrossAppConfig::from_env_lookup_default_on(|_| Some(falsy.to_string()));
            assert!(!cfg.is_enabled(), "{falsy:?} must roll back to OFF");
        }
        // A non-falsy value keeps it ON.
        assert!(
            GuiCrossAppConfig::from_env_lookup_default_on(|_| Some("1".to_string())).is_enabled()
        );
    }

    #[test]
    fn t1_env_flag_const_is_stable() {
        assert_eq!(CROSSAPP_ENV_FLAG, "KRIA_GUI_COG_CROSSAPP");
    }
}
