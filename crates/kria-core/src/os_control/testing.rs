//! Centralized deny-live test surface (OSC-033, OSC-034), Task 0.4.
//!
//! Compiled **only** under `os-control-test`. This is the single place a
//! completion test reaches for fakes, fixtures, recorders and the deny-live
//! assertion, so no test has to hand-roll a temp directory or re-derive which
//! constructor is safe.
//!
//! # Why this module can never touch the host
//!
//! * Live transport construction requires an
//!   [`crate::os_control::access::LiveHostAccessToken`], and token minting is
//!   gated to `os-control-live` — a feature that is a hard `compile_error!` when
//!   combined with `os-control-test`. So a test binary cannot link live
//!   construction at all.
//! * Every raw transport calls
//!   [`crate::os_control::access::deny_live_transport`] first. Under this feature
//!   the sentinel is armed by default and panics, so an accidental live path
//!   fails loudly instead of mutating the developer's session.
//! * The fakes re-exported here are pure in-memory maps: no bus, no child
//!   process, no device node, no keyring, no shell.
//!
//! A completion test therefore runs safely inside the owner's active desktop
//! session with no observable OS mutation, which is exactly what Task 0.4
//! requires.
//!
//! # Mandated invocation
//!
//! ```text
//! cargo test -p kria-core --no-default-features --features os-control-test
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::os_control::contract::ProviderId;
use crate::os_control::runtime::HostOsControl;

// ── Deny-live sentinel controls (re-exported, single source of truth) ────────
pub use crate::os_control::access::{
    deny_live_transport, live_composition_count, reset_trip_count, scoped_disarm,
    sentinel_is_armed, sentinel_trip_count, RawTransportKind, SentinelDisarmGuard,
};

// ── Fakes (the only providers a completion test may construct) ───────────────
pub use crate::os_control::applications::fake_association::FakeDesktopAssociationTransport;
pub use crate::os_control::linux::probe::ScriptedProbeMatrix;
pub use crate::os_control::sandbox::FakeSandboxGrantControl;
pub use crate::os_control::secrets::FakeCredentialStore;

/// Tag stamped onto receipts produced by a fake provider, so a fake-backed
/// result can never be mistaken for evidence of real host acceptance (OSC-033).
pub const FAKE_RECEIPT_TAG: &str = "fake-provider";

// ─────────────────────────────────────────────────────────────────────────────
// Scripted fake (OSC-033 §18)
// ─────────────────────────────────────────────────────────────────────────────

/// A result produced by a fake provider.
///
/// Carries [`FAKE_RECEIPT_TAG`] so a fake-backed value is self-identifying: a
/// test (or a reviewer) can tell at a glance that a receipt is evidence of a
/// *scripted* outcome, never of real host acceptance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeReceipt<T> {
    /// Always [`FAKE_RECEIPT_TAG`].
    pub tag: &'static str,
    /// The scripted payload.
    pub payload: T,
}

impl<T> FakeReceipt<T> {
    /// Always true. Exists so a caller asserts the property rather than
    /// comparing the tag string by hand.
    #[must_use]
    pub fn is_fake(&self) -> bool {
        self.tag == FAKE_RECEIPT_TAG
    }
}

/// Why a scripted fake refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestingError {
    /// No outcome was scripted for `operation`.
    ///
    /// This is the whole point of the type: an unscripted operation is
    /// **unavailable**, never a silently-invented success. It mirrors what a real
    /// machine returns when a provider is not composed, so a test that forgets to
    /// script a step fails loudly instead of passing against a fabricated fact.
    Unavailable {
        /// The operation label that had nothing scripted.
        operation: String,
    },
}

impl std::fmt::Display for TestingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable { operation } => write!(
                f,
                "no scripted outcome for `{operation}`; a fake reports unavailable rather than inventing one"
            ),
        }
    }
}

impl std::error::Error for TestingError {}

/// A minimal scripted provider: labelled outcomes consumed in order, with every
/// call recorded.
///
/// Used to prove the deny-live posture itself — a fake-backed observe → apply →
/// verify flow must complete while opening no transport at all, which is what
/// `os_control_test_safety` asserts.
pub struct ScriptedFake<T> {
    scripted: Mutex<VecDeque<(String, T)>>,
    recorder: Arc<CallRecorder>,
}

impl<T> Default for ScriptedFake<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ScriptedFake<T> {
    /// An empty fake. Every operation is unavailable until scripted.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scripted: Mutex::new(VecDeque::new()),
            recorder: Arc::new(CallRecorder::new()),
        }
    }

    /// Script one outcome for `label`, appended to the queue.
    pub fn push(&self, label: impl Into<String>, payload: T) {
        self.scripted
            .lock()
            .expect("scripted fake mutex")
            .push_back((label.into(), payload));
    }

    /// Consume the next scripted outcome, which must match `label`.
    ///
    /// The label is checked rather than ignored so a test that drives operations
    /// out of order fails, instead of quietly receiving another step's payload.
    pub fn next(&self, label: &str) -> Result<FakeReceipt<T>, TestingError> {
        self.recorder.record(label);
        let mut queue = self.scripted.lock().expect("scripted fake mutex");
        match queue.front() {
            Some((scripted_label, _)) if scripted_label == label => {
                let (_, payload) = queue.pop_front().expect("front just matched");
                Ok(FakeReceipt {
                    tag: FAKE_RECEIPT_TAG,
                    payload,
                })
            }
            // Either nothing is scripted, or the next step is a different
            // operation. Both are "unavailable" — never a substituted outcome.
            _ => Err(TestingError::Unavailable {
                operation: label.to_string(),
            }),
        }
    }

    /// An owned handle to the call recorder.
    #[must_use]
    pub fn recorder(&self) -> Arc<CallRecorder> {
        Arc::clone(&self.recorder)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Test-command linter (OSC-033.8)
// ─────────────────────────────────────────────────────────────────────────────

/// Why a Cargo invocation is unsafe for this spec's completion tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestCommandViolation {
    /// A test/check invocation that does not enable `os-control-test`, so the
    /// default (or live) composition could link into a test binary.
    MissingTestFeature,
    /// Enables `os-control-test` **and** `os-control-live`. They are a hard
    /// `compile_error!`; naming both in a command is a mistake worth catching in
    /// review rather than at the end of a long build.
    DualComposition,
    /// Omits `--no-default-features`, so default features leak into a completion
    /// test and the deny-live guarantee no longer holds.
    DefaultFeaturesLeak,
    /// A completion test that enables the live composition. This is the dangerous
    /// one: it would let a test reach the real machine.
    LiveCompositionInTest,
}

impl std::fmt::Display for TestCommandViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::MissingTestFeature => "missing --features os-control-test",
            Self::DualComposition => "enables both os-control-test and os-control-live",
            Self::DefaultFeaturesLeak => "missing --no-default-features",
            Self::LiveCompositionInTest => "enables os-control-live in a test invocation",
        };
        f.write_str(text)
    }
}

/// Extract the comma/space separated feature list from a Cargo command.
fn declared_features(command: &str) -> Vec<&str> {
    let mut features = Vec::new();
    let tokens: Vec<&str> = command.split_whitespace().collect();
    for (index, token) in tokens.iter().enumerate() {
        let list = if let Some(rest) = token.strip_prefix("--features=") {
            Some(rest)
        } else if *token == "--features" {
            tokens.get(index + 1).copied()
        } else {
            None
        };
        if let Some(list) = list {
            features.extend(list.split(',').map(str::trim).filter(|f| !f.is_empty()));
        }
    }
    features
}

/// Lint one Cargo invocation against the deny-live rules (design §18).
///
/// Only `cargo test` / `cargo check` invocations are constrained: a
/// `cargo build -p kria-desktop --features os-control-live` is the *composition
/// root* and is legitimately live, so it is accepted.
pub fn lint_test_command(command: &str) -> Result<(), TestCommandViolation> {
    let features = declared_features(command);
    let has_test = features.contains(&"os-control-test");
    let has_live = features.contains(&"os-control-live");

    // Checked first and unconditionally: the two features are mutually exclusive
    // everywhere, so this is a violation even outside a test invocation.
    if has_test && has_live {
        return Err(TestCommandViolation::DualComposition);
    }

    let is_gate = command.contains("cargo test") || command.contains("cargo check");
    if !is_gate {
        // A build/run command is a composition root, not a completion test.
        return Ok(());
    }

    if has_live {
        return Err(TestCommandViolation::LiveCompositionInTest);
    }
    if !has_test {
        return Err(TestCommandViolation::MissingTestFeature);
    }
    if !command.contains("--no-default-features") {
        return Err(TestCommandViolation::DefaultFeaturesLeak);
    }
    Ok(())
}

/// One entry in the focused test-command manifest.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TestCommandEntry {
    /// Why this invocation is listed.
    pub description: String,
    /// The invocation itself.
    pub command: String,
}

/// The authoritative list of invocations for this spec's completion tests.
///
/// Kept as a fixture rather than only in prose so the linter and the documented
/// commands cannot drift apart: [`Self::verify`] fails if they ever do.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TestCommandManifest {
    /// Invocations that must pass the linter.
    #[serde(default)]
    pub allowed: Vec<TestCommandEntry>,
    /// Invocations the linter must reject.
    #[serde(default)]
    pub rejected: Vec<TestCommandEntry>,
}

impl TestCommandManifest {
    /// Parse the manifest from TOML.
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// Cross-check every entry against the linter, returning one description per
    /// disagreement. An empty result means manifest and linter agree.
    #[must_use]
    pub fn verify(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for entry in &self.allowed {
            if let Err(violation) = lint_test_command(&entry.command) {
                problems.push(format!(
                    "allowed command was rejected ({violation}): {}",
                    entry.command
                ));
            }
        }
        for entry in &self.rejected {
            if lint_test_command(&entry.command).is_ok() {
                problems.push(format!(
                    "rejected command was accepted: {}",
                    entry.command
                ));
            }
        }
        problems
    }
}

/// Assert the deny-live posture actually held for the current test.
///
/// Call at the end of any test that exercises a provider path: it proves the
/// sentinel was armed throughout and that no raw transport was opened.
///
/// # Panics
/// If the sentinel is disarmed, or if any live transport attempt was recorded.
pub fn assert_no_live_access() {
    assert!(
        sentinel_is_armed(),
        "deny-live sentinel was disarmed during this test — a live transport could have opened"
    );
    assert_eq!(
        sentinel_trip_count(),
        0,
        "a raw live transport was attempted during this test"
    );
    assert_eq!(
        live_composition_count(),
        0,
        "a live composition root was constructed during this test"
    );
}

// ── Centralized temp-directory fixture ──────────────────────────────────────

static FIXTURE_SEQ: AtomicU64 = AtomicU64::new(0);

/// A unique temporary directory that removes itself on drop.
///
/// Centralized here (rather than per-test) so every filesystem fixture lands
/// under the OS temp root and is cleaned up even when a test panics. Built on
/// `std` only, so it adds no dependency to the library.
#[derive(Debug)]
pub struct TempFixtureDir {
    path: PathBuf,
}

impl TempFixtureDir {
    /// Create a uniquely named temp directory tagged with `label`.
    ///
    /// # Panics
    /// If the directory cannot be created.
    #[must_use]
    pub fn new(label: &str) -> Self {
        let seq = FIXTURE_SEQ.fetch_add(1, Ordering::SeqCst);
        let safe: String = label
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let path = std::env::temp_dir().join(format!(
            "kria-os-control-{safe}-{pid}-{seq}",
            pid = std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp fixture dir");
        Self { path }
    }

    /// The fixture root.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Create a child directory inside the fixture and return its path.
    ///
    /// # Panics
    /// If the directory cannot be created.
    #[must_use]
    pub fn child_dir(&self, name: &str) -> PathBuf {
        let p = self.path.join(name);
        fs::create_dir_all(&p).expect("create fixture child dir");
        p
    }

    /// Write a file inside the fixture and return its path.
    ///
    /// # Panics
    /// If the file cannot be written.
    #[must_use]
    pub fn write_file(&self, name: &str, contents: &str) -> PathBuf {
        let p = self.path.join(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create fixture parent dir");
        }
        fs::write(&p, contents).expect("write fixture file");
        p
    }
}

impl Drop for TempFixtureDir {
    fn drop(&mut self) {
        // Best-effort: a leaked temp dir must never fail a test.
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// The canonical way a completion test gets a scratch directory.
///
/// Returns a self-cleaning [`TempFixtureDir`]; call `.path()` for the root. Kept
/// as a free function because that is how the file/trash/archive/audit suites
/// already acquire fixtures.
///
/// # Panics
/// If the directory cannot be created.
#[must_use]
pub fn temp_dir() -> TempFixtureDir {
    TempFixtureDir::new("fixture")
}

// ── Fake host aggregate ─────────────────────────────────────────────────────

/// A deny-live [`HostOsControl`] aggregate that composes **no** domain ports.
///
/// `HostOsControl` requires only `provider_id`; every domain port defaults to
/// `None`. That is exactly the shape a governance test wants: it proves the
/// runtime falls back to the frozen `Unavailable` envelope for an uncomposed
/// domain instead of reaching for an ungoverned subprocess.
///
/// Every trait call is recorded, so a test can assert the runtime touched the
/// aggregate exactly once and in the expected order.
pub struct FakeHostOsControl {
    provider: String,
    recorder: Arc<CallRecorder>,
    audio: Option<Arc<dyn crate::os_control::audio::AudioControlPort>>,
    power: Option<Arc<dyn crate::os_control::power::PowerControlPort>>,
    power_session: Option<Arc<dyn crate::os_control::power::session::PowerSessionControlPort>>,
    processes: Option<Arc<dyn crate::os_control::processes::ProcessControlPort>>,
    display: Option<Arc<dyn crate::os_control::display::DisplayControlPort>>,
    connectivity: Option<Arc<dyn crate::os_control::connectivity::ConnectivityControlPort>>,
    clipboard: Option<Arc<dyn crate::os_control::clipboard::ClipboardControlPort>>,
    notifications: Option<Arc<dyn crate::os_control::notifications::NotificationControlPort>>,
    packages: Option<Arc<dyn crate::os_control::packages::PackageControlPort>>,
    storage: Option<Arc<dyn crate::os_control::storage::StorageControlPort>>,
    trash: Option<Arc<dyn crate::os_control::files::TrashControlPort>>,
    application_close:
        Option<Arc<dyn crate::os_control::applications::ApplicationCloseControlPort>>,
    desktop_association:
        Option<Arc<dyn crate::os_control::applications::DesktopAssociationControlPort>>,
}

impl FakeHostOsControl {
    /// A fake aggregate reporting `provider` as its identity, with no domain
    /// port composed. Compose ports with the `with_*` builders.
    ///
    /// Every domain starts as `None` on purpose: a suite that forgets to compose
    /// its port gets the frozen `Unavailable` envelope, which is the same thing a
    /// real machine without that provider returns. A fake that silently supplied
    /// a working port would hide exactly that case.
    #[must_use]
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            recorder: Arc::new(CallRecorder::new()),
            audio: None,
            power: None,
            power_session: None,
            processes: None,
            display: None,
            connectivity: None,
            clipboard: None,
            notifications: None,
            packages: None,
            storage: None,
            trash: None,
            application_close: None,
            desktop_association: None,
        }
    }

    /// An owned handle to this aggregate's recorder.
    ///
    /// Owned (not a borrow) on purpose: a test takes the recorder and *then*
    /// moves the aggregate into the runtime, so a borrow would not outlive the
    /// move.
    #[must_use]
    pub fn recorder(&self) -> Arc<CallRecorder> {
        Arc::clone(&self.recorder)
    }
}

impl FakeHostOsControl {
    /// Builder: compose the `audio` port into the aggregate.
    #[must_use]
    pub fn with_audio(mut self, port: Arc<dyn crate::os_control::audio::AudioControlPort>) -> Self {
        self.audio = Some(port);
        self
    }

    /// Builder: compose the `power` port into the aggregate.
    #[must_use]
    pub fn with_power(mut self, port: Arc<dyn crate::os_control::power::PowerControlPort>) -> Self {
        self.power = Some(port);
        self
    }

    /// Builder: compose the `power_session` port into the aggregate.
    #[must_use]
    pub fn with_power_session(mut self, port: Arc<dyn crate::os_control::power::session::PowerSessionControlPort>) -> Self {
        self.power_session = Some(port);
        self
    }

    /// Builder: compose the `processes` port into the aggregate.
    #[must_use]
    pub fn with_processes(mut self, port: Arc<dyn crate::os_control::processes::ProcessControlPort>) -> Self {
        self.processes = Some(port);
        self
    }

    /// Builder: compose the `display` port into the aggregate.
    #[must_use]
    pub fn with_display(mut self, port: Arc<dyn crate::os_control::display::DisplayControlPort>) -> Self {
        self.display = Some(port);
        self
    }

    /// Builder: compose the `connectivity` port into the aggregate.
    #[must_use]
    pub fn with_connectivity(mut self, port: Arc<dyn crate::os_control::connectivity::ConnectivityControlPort>) -> Self {
        self.connectivity = Some(port);
        self
    }

    /// Builder: compose the `clipboard` port into the aggregate.
    #[must_use]
    pub fn with_clipboard(mut self, port: Arc<dyn crate::os_control::clipboard::ClipboardControlPort>) -> Self {
        self.clipboard = Some(port);
        self
    }

    /// Builder: compose the `notifications` port into the aggregate.
    #[must_use]
    pub fn with_notifications(mut self, port: Arc<dyn crate::os_control::notifications::NotificationControlPort>) -> Self {
        self.notifications = Some(port);
        self
    }

    /// Builder: compose the `packages` port into the aggregate.
    #[must_use]
    pub fn with_packages(mut self, port: Arc<dyn crate::os_control::packages::PackageControlPort>) -> Self {
        self.packages = Some(port);
        self
    }

    /// Builder: compose the `storage` port into the aggregate.
    #[must_use]
    pub fn with_storage(mut self, port: Arc<dyn crate::os_control::storage::StorageControlPort>) -> Self {
        self.storage = Some(port);
        self
    }

    /// Builder: compose the `trash` port into the aggregate.
    #[must_use]
    pub fn with_trash(mut self, port: Arc<dyn crate::os_control::files::TrashControlPort>) -> Self {
        self.trash = Some(port);
        self
    }

    /// Builder: compose the `application_close` port into the aggregate.
    #[must_use]
    pub fn with_application_close(mut self, port: Arc<dyn crate::os_control::applications::ApplicationCloseControlPort>) -> Self {
        self.application_close = Some(port);
        self
    }

    /// Builder: compose the `desktop_association` port into the aggregate.
    #[must_use]
    pub fn with_desktop_association(mut self, port: Arc<dyn crate::os_control::applications::DesktopAssociationControlPort>) -> Self {
        self.desktop_association = Some(port);
        self
    }

}

impl HostOsControl for FakeHostOsControl {
    fn provider_id(&self) -> ProviderId {
        self.recorder.record("provider_id");
        ProviderId::new(&self.provider)
    }

    fn audio(&self) -> Option<&dyn crate::os_control::audio::AudioControlPort> {
        self.recorder.record("audio");
        self.audio.as_deref()
    }

    fn power(&self) -> Option<&dyn crate::os_control::power::PowerControlPort> {
        self.recorder.record("power");
        self.power.as_deref()
    }

    fn power_session(&self) -> Option<&dyn crate::os_control::power::session::PowerSessionControlPort> {
        self.recorder.record("power_session");
        self.power_session.as_deref()
    }

    fn processes(&self) -> Option<&dyn crate::os_control::processes::ProcessControlPort> {
        self.recorder.record("processes");
        self.processes.as_deref()
    }

    fn display(&self) -> Option<&dyn crate::os_control::display::DisplayControlPort> {
        self.recorder.record("display");
        self.display.as_deref()
    }

    fn connectivity(&self) -> Option<&dyn crate::os_control::connectivity::ConnectivityControlPort> {
        self.recorder.record("connectivity");
        self.connectivity.as_deref()
    }

    fn clipboard(&self) -> Option<&dyn crate::os_control::clipboard::ClipboardControlPort> {
        self.recorder.record("clipboard");
        self.clipboard.as_deref()
    }

    fn notifications(&self) -> Option<&dyn crate::os_control::notifications::NotificationControlPort> {
        self.recorder.record("notifications");
        self.notifications.as_deref()
    }

    fn packages(&self) -> Option<&dyn crate::os_control::packages::PackageControlPort> {
        self.recorder.record("packages");
        self.packages.as_deref()
    }

    fn storage(&self) -> Option<&dyn crate::os_control::storage::StorageControlPort> {
        self.recorder.record("storage");
        self.storage.as_deref()
    }

    fn trash(&self) -> Option<&dyn crate::os_control::files::TrashControlPort> {
        self.recorder.record("trash");
        self.trash.as_deref()
    }

    fn application_close(&self) -> Option<&dyn crate::os_control::applications::ApplicationCloseControlPort> {
        self.recorder.record("application_close");
        self.application_close.as_deref()
    }

    fn desktop_association(&self) -> Option<&dyn crate::os_control::applications::DesktopAssociationControlPort> {
        self.recorder.record("desktop_association");
        self.desktop_association.as_deref()
    }
}


/// Records, in order, the labels of the calls made against a fake provider.
///
/// Deliberately non-generic and label-based: the governed runtime order
/// (`observe` → `apply` → `verify` → `rollback`) is asserted as a sequence of
/// stable strings, so every fake records through the same type.
#[derive(Debug, Default)]
pub struct CallRecorder {
    labels: Mutex<Vec<String>>,
}

impl CallRecorder {
    /// A new, empty recorder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            labels: Mutex::new(Vec::new()),
        }
    }

    /// Record one call by its stable label.
    pub fn record(&self, label: impl Into<String>) {
        self.labels.lock().expect("recorder mutex").push(label.into());
    }

    /// Every recorded label, in call order.
    #[must_use]
    pub fn labels(&self) -> Vec<String> {
        self.labels.lock().expect("recorder mutex").clone()
    }

    /// How many calls were recorded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.labels.lock().expect("recorder mutex").len()
    }

    /// Whether no call was recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.labels.lock().expect("recorder mutex").is_empty()
    }

    /// Drop all recorded labels.
    pub fn clear(&self) {
        self.labels.lock().expect("recorder mutex").clear();
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn temp_fixture_creates_and_removes_itself() {
        let path = {
            let fx = TempFixtureDir::new("selftest");
            let f = fx.write_file("a/b.txt", "hi");
            assert!(f.exists());
            assert_eq!(fs::read_to_string(&f).unwrap(), "hi");
            fx.path().to_path_buf()
        };
        assert!(!path.exists(), "fixture dir must be removed on drop");
    }

    #[test]
    fn recorder_preserves_call_order() {
        let r = CallRecorder::new();
        assert!(r.is_empty());
        r.record("observe");
        r.record("apply");
        assert_eq!(r.labels(), vec!["observe", "apply"]);
        assert_eq!(r.len(), 2);
        r.clear();
        assert!(r.is_empty());
    }

    #[test]
    fn temp_dir_helper_returns_a_usable_root() {
        let fx = temp_dir();
        assert!(fx.path().is_dir());
    }

    #[test]
    fn sentinel_is_armed_under_the_test_feature() {
        assert!(sentinel_is_armed());
    }
}

/// Build an observation-only [`crate::os_control::context::HostExecutionContext`]
/// for a provider test.
///
/// Several live providers expose pure-read helpers (`path_exists`, a sensor scan,
/// a settings read) that need a context but no grant. Rebuilding seven arguments
/// in every test invited drift, so it is written once here.
///
/// The deny-live sentinel still guards every process spawn, so a test holding this
/// context cannot reach the host through a command — only through the direct
/// `/proc` and `/sys` reads a provider performs itself.
#[must_use]
pub fn observation_context_for_test() -> crate::os_control::context::HostExecutionContext {
    use crate::os_control::context::{
        AuditAdmissionToken, HostExecutionContext, RedactionPolicy, SessionContext,
    };
    use crate::os_control::contract::{
        ActionId, AuditAdmissionId, CorrelationId, Digest, SessionId,
    };

    let audit_token = AuditAdmissionToken::for_test(
        AuditAdmissionId::new("provider-test-admission"),
        Digest::of_str("provider-test-resources"),
    );
    HostExecutionContext::for_test(
        CorrelationId::new("provider-test-correlation"),
        ActionId::new("provider-test-action"),
        audit_token.observation_authority(),
        std::sync::Arc::new(SessionContext::new(SessionId::new("provider-test-session"))),
        tokio_util::sync::CancellationToken::new(),
        std::time::Instant::now() + std::time::Duration::from_secs(30),
        RedactionPolicy::default(),
    )
}
