//! GUI Cognition execution environment (Task 0.3 — TestSubstrate).
//!
//! The execution environment is the structural data-loss-safety boundary for the
//! GUI Cognition live test path. It distinguishes a user's **real session** from
//! an isolated **test substrate** (nested compositor / dedicated seat / scratch
//! user / Xvfb headless seat) where destructive and approval live tests run
//! against scratch files + a saved/restored clipboard.
//!
//! The single safety-critical rule this type enforces (Requirement 20.3):
//!
//! > Auto-approval (HITL decision) fixtures SHALL be rejected when NOT in the
//! > test substrate.
//!
//! Trust model: the substrate marker is derived **server-side** from the process
//! environment (`KRIA_GUI_TEST_SUBSTRATE`), NOT from the request payload. A client
//! cannot fake "I'm a substrate" over the wire to coax the user's real session into
//! auto-approving a destructive action — the desktop process only reports
//! `TestSubstrate` when its launcher actually stood one up. See
//! `scripts/gui_cognition_test_substrate.sh`.

use std::path::PathBuf;

/// Environment variable set by the TestSubstrate launcher to mark the running
/// KRIA desktop process as confined to an isolated substrate.
pub const SUBSTRATE_ENV_FLAG: &str = "KRIA_GUI_TEST_SUBSTRATE";
/// Optional scratch directory the substrate confines destructive actions to.
pub const SUBSTRATE_ENV_SCRATCH_DIR: &str = "KRIA_GUI_TEST_SUBSTRATE_SCRATCH_DIR";
/// Optional flag (default on in substrate) to save+restore the user clipboard.
pub const SUBSTRATE_ENV_RESTORE_CLIPBOARD: &str = "KRIA_GUI_TEST_SUBSTRATE_RESTORE_CLIPBOARD";

/// Where a GUI Cognition turn is allowed to physically act.
///
/// `RealSession` is the safe default everywhere. `TestSubstrate` is only ever
/// produced from a deliberately-set process environment (see [`from_env`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuiExecutionEnvironment {
    /// The user's real desktop session. Auto-approval fixtures are rejected here.
    RealSession,
    /// An isolated test substrate confining destructive actions to scratch
    /// resources, with the user clipboard saved and restored around the turn.
    TestSubstrate {
        /// Scratch directory destructive file actions are confined to, if known.
        scratch_dir: Option<PathBuf>,
        /// Whether the substrate saves and restores the user clipboard.
        restore_clipboard: bool,
    },
}

impl Default for GuiExecutionEnvironment {
    fn default() -> Self {
        GuiExecutionEnvironment::RealSession
    }
}

impl GuiExecutionEnvironment {
    /// Derive the environment from the process environment.
    ///
    /// Returns [`GuiExecutionEnvironment::TestSubstrate`] only when
    /// `KRIA_GUI_TEST_SUBSTRATE` is set to a truthy value (`1`, `true`, `yes`,
    /// `on`). Any other value (including unset) yields the safe `RealSession`.
    pub fn from_env() -> Self {
        Self::from_env_lookup(|key| std::env::var(key).ok())
    }

    /// Testable core of [`from_env`] with an injectable lookup.
    pub fn from_env_lookup<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        if !is_truthy(lookup(SUBSTRATE_ENV_FLAG).as_deref()) {
            return GuiExecutionEnvironment::RealSession;
        }
        let scratch_dir = lookup(SUBSTRATE_ENV_SCRATCH_DIR)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        // Restoring the user clipboard defaults ON inside a substrate; only an
        // explicit falsey value disables it.
        let restore_clipboard = match lookup(SUBSTRATE_ENV_RESTORE_CLIPBOARD).as_deref() {
            Some(raw) => !is_falsey(Some(raw)),
            None => true,
        };
        GuiExecutionEnvironment::TestSubstrate {
            scratch_dir,
            restore_clipboard,
        }
    }

    /// Whether this environment is an isolated test substrate.
    pub fn is_test_substrate(&self) -> bool {
        matches!(self, GuiExecutionEnvironment::TestSubstrate { .. })
    }

    /// Whether auto-approval (HITL decision) fixtures may be honored here.
    ///
    /// This is the Requirement 20.3 gate: only the test substrate may auto-approve.
    pub fn allows_auto_approval(&self) -> bool {
        self.is_test_substrate()
    }

    /// Stable label for events/telemetry (never includes paths/secrets).
    pub fn label(&self) -> &'static str {
        match self {
            GuiExecutionEnvironment::RealSession => "real_session",
            GuiExecutionEnvironment::TestSubstrate { .. } => "test_substrate",
        }
    }

    /// Sanitized JSON summary for inclusion in the turn response (no raw paths).
    pub fn summary_json(&self) -> serde_json::Value {
        match self {
            GuiExecutionEnvironment::RealSession => serde_json::json!({
                "environment": "real_session",
                "allows_auto_approval": false,
            }),
            GuiExecutionEnvironment::TestSubstrate {
                scratch_dir,
                restore_clipboard,
            } => serde_json::json!({
                "environment": "test_substrate",
                "allows_auto_approval": true,
                "has_scratch_dir": scratch_dir.is_some(),
                "restore_clipboard": restore_clipboard,
            }),
        }
    }
}

fn is_truthy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

fn is_falsey(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("0") | Some("false") | Some("no") | Some("off") | Some("")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    #[test]
    fn default_is_real_session_and_forbids_auto_approval() {
        let env = GuiExecutionEnvironment::default();
        assert_eq!(env, GuiExecutionEnvironment::RealSession);
        assert!(!env.is_test_substrate());
        assert!(!env.allows_auto_approval());
        assert_eq!(env.label(), "real_session");
    }

    #[test]
    fn unset_flag_yields_real_session() {
        let env = GuiExecutionEnvironment::from_env_lookup(lookup_from(&[]));
        assert_eq!(env, GuiExecutionEnvironment::RealSession);
        assert!(!env.allows_auto_approval());
    }

    #[test]
    fn falsey_flag_yields_real_session() {
        for raw in ["0", "false", "no", "off", ""] {
            let env = GuiExecutionEnvironment::from_env_lookup(lookup_from(&[(
                SUBSTRATE_ENV_FLAG,
                raw,
            )]));
            assert_eq!(
                env,
                GuiExecutionEnvironment::RealSession,
                "flag {raw:?} must not enable substrate"
            );
        }
    }

    #[test]
    fn truthy_flag_yields_substrate_that_allows_auto_approval() {
        for raw in ["1", "true", "YES", "On"] {
            let env = GuiExecutionEnvironment::from_env_lookup(lookup_from(&[(
                SUBSTRATE_ENV_FLAG,
                raw,
            )]));
            assert!(env.is_test_substrate(), "flag {raw:?} must enable substrate");
            assert!(env.allows_auto_approval());
            assert_eq!(env.label(), "test_substrate");
        }
    }

    #[test]
    fn substrate_threads_scratch_dir_and_clipboard_default_on() {
        let env = GuiExecutionEnvironment::from_env_lookup(lookup_from(&[
            (SUBSTRATE_ENV_FLAG, "1"),
            (SUBSTRATE_ENV_SCRATCH_DIR, "/tmp/kria-substrate/scratch"),
        ]));
        match env {
            GuiExecutionEnvironment::TestSubstrate {
                scratch_dir,
                restore_clipboard,
            } => {
                assert_eq!(
                    scratch_dir,
                    Some(PathBuf::from("/tmp/kria-substrate/scratch"))
                );
                assert!(restore_clipboard, "clipboard restore defaults on in substrate");
            }
            other => panic!("expected substrate, got {other:?}"),
        }
    }

    #[test]
    fn substrate_clipboard_restore_can_be_disabled() {
        let env = GuiExecutionEnvironment::from_env_lookup(lookup_from(&[
            (SUBSTRATE_ENV_FLAG, "1"),
            (SUBSTRATE_ENV_RESTORE_CLIPBOARD, "0"),
        ]));
        match env {
            GuiExecutionEnvironment::TestSubstrate {
                restore_clipboard, ..
            } => assert!(!restore_clipboard),
            other => panic!("expected substrate, got {other:?}"),
        }
    }

    #[test]
    fn summary_json_never_leaks_scratch_path() {
        let env = GuiExecutionEnvironment::from_env_lookup(lookup_from(&[
            (SUBSTRATE_ENV_FLAG, "1"),
            (SUBSTRATE_ENV_SCRATCH_DIR, "/home/user/secret/scratch"),
        ]));
        let summary = env.summary_json();
        let serialized = serde_json::to_string(&summary).unwrap();
        assert!(!serialized.contains("/home/user/secret"));
        assert_eq!(summary["environment"], "test_substrate");
        assert_eq!(summary["has_scratch_dir"], true);
        assert_eq!(summary["allows_auto_approval"], true);
    }
}
