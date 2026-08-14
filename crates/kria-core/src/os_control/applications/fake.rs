//! Deny-live fake [`ApplicationCloseTransport`] (OSC-013, OSC-033), Task 2.5.
//!
//! Compiled only under `os-control-test`. It models the process table as a
//! plain in-memory list of running instances: no `kill(2)`, no signal, no
//! subprocess, nothing that could reach a real application.
//! `terminate_matching` records the request and **applies the effect to the
//! table**, so an observe → close → re-observe → verify lifecycle converges on
//! the fake's own state instead of on a scripted sequence.
//!
//! # Three different facts, three different answers
//!
//! `graceful_close_application` is only safe if the provider can tell these
//! apart, so the fake keeps them apart:
//!
//! | Fact | How it is scripted | What a read returns |
//! |---|---|---|
//! | The app is **not running** | [`Self::count_ok`]`(0)`, or a table with no matching instance | `Ok(0)` — a real, observed count |
//! | The app's state **could not be determined** | [`Self::count_unknown`], or [`Self::count_failure`] | [`OsControlError::Unavailable`] |
//! | Nothing was ever enumerated | a fresh [`Self::new`] | `Unavailable` — the table is *unset*, which is not the same as empty |
//!
//! Collapsing "not running" into "unknown" would make every failed enumeration
//! look like a completed close; collapsing the other way would fabricate a
//! process count nobody read.
//!
//! # Identity is the app id, never a window title
//!
//! Instances are matched by app id / desktop-file id — exactly (`gedit`) or by
//! the `name-<suffix>` prefix form the pre-migration `CloseApplication`
//! matched (`gedit-worker`), never a bare substring and never a window title.
//! Window titles are neither unique nor stable ("Untitled Document" is shared
//! by two editors, and a title changes the instant the user types), so a title
//! is carried on an instance for realism but is **never** an identity key. See
//! `two_windows_sharing_a_title_stay_distinguishable`.
//!
//! # The close/exit race
//!
//! A graceful close legitimately races with the user closing the app: the
//! process can exit on its own between the close request and verification.
//! [`Self::exits_on_its_own`] models exactly that — the instance disappears
//! without the fake's dispatch having caused it, and the dispatch then reports
//! [`ApplyOutcome::Uncertain`] with [`UncertainEffectCause::Unobservable`]
//! rather than `Applied`. The postcondition (zero alive) holds either way, but
//! the fake never lets a receipt claim credit for an exit it did not cause.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{BoundedVec, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{
    AppliedDispatch, ApplyOutcome, UncertainDispatch, UncertainEffectCause,
};

use super::ApplicationCloseTransport;

/// Provider identity reported by the fake transport. Matches the `provider`
/// named by the deny-live lifecycle suite's [`crate::os_control::MutationPlan`]
/// so the receipt and the transport agree on one identity.
pub const FAKE_APPLICATION_CLOSE_PROVIDER_ID: &str = "application-close-native-syscall";

/// One modelled running instance. `app_id` is the identity; `window_title` is
/// carried for realism and is deliberately never matched against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeAppInstance {
    /// The stable app id / desktop-file id (the only identity key).
    pub app_id: String,
    /// The process id of this instance.
    pub pid: u32,
    /// The window title, which may be duplicated across unrelated apps and may
    /// change at any moment. Never used to identify anything.
    pub window_title: Option<String>,
}

impl FakeAppInstance {
    /// A running instance of `app_id` with `pid` and no window title.
    #[must_use]
    pub fn new(app_id: impl Into<String>, pid: u32) -> Self {
        Self {
            app_id: app_id.into(),
            pid,
            window_title: None,
        }
    }

    /// Attach a window title (never an identity key).
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.window_title = Some(title.into());
        self
    }
}

/// One scripted count. `Unknown` is a first-class outcome: it is the only way
/// to say "the process table could not be enumerated" without borrowing the
/// real count `0` — which means "the app is not running" — to mean it.
#[derive(Debug, Clone)]
enum ScriptedCount {
    /// The table was enumerated and this many instances match (`0` included).
    Alive(u32),
    /// Enumeration failed; the count is indeterminate.
    Unknown(String),
}

/// A scripted, in-memory application process table.
pub struct FakeApplicationCloseTransport {
    /// Ordered scripted counts. Consumed front-to-back and always preferred
    /// over the model, so a test can drive a TOCTOU change the model would not
    /// produce on its own.
    scripted: Mutex<VecDeque<ScriptedCount>>,
    /// The modelled process table. `None` means *never enumerated* (unknown),
    /// which is why a fresh fake reports `Unavailable` rather than `0`.
    instances: Mutex<Option<Vec<FakeAppInstance>>>,
    /// Sticky: every count read fails with this reason.
    count_failure: Option<String>,
    /// App ids that exit on their own at the next count read after a close was
    /// requested — the user closing the window while we were closing it.
    exits_on_its_own: Mutex<Vec<String>>,
    /// Set once a close has been dispatched, so `exits_on_its_own` fires on the
    /// verification read rather than on the pre-observation.
    close_requested: Mutex<bool>,
    /// Whether the last dispatch's effect could be attributed to us.
    self_exit_raced: Mutex<bool>,
    /// Scripted `terminate_matching` outcome (defaults to `Applied`).
    outcome: Mutex<Option<ApplyOutcome>>,
    /// Sticky: `terminate_matching` fails and no instance is removed.
    terminate_denied: bool,
    /// The app ids `terminate_matching` was called with, in order.
    terminated: Mutex<Vec<String>>,
    /// The app ids that were counted, in order.
    reads: Mutex<Vec<String>>,
}

impl Default for FakeApplicationCloseTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeApplicationCloseTransport {
    /// A fake with nothing scripted and **no table enumerated**: a count fails
    /// closed until [`Self::count_ok`] or [`Self::with_running`] is called.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scripted: Mutex::new(VecDeque::new()),
            instances: Mutex::new(None),
            count_failure: None,
            exits_on_its_own: Mutex::new(Vec::new()),
            close_requested: Mutex::new(false),
            self_exit_raced: Mutex::new(false),
            outcome: Mutex::new(None),
            terminate_denied: false,
            terminated: Mutex::new(Vec::new()),
            reads: Mutex::new(Vec::new()),
        }
    }

    /// Builder: queue the next count as `alive`. `0` is a real, observed count
    /// meaning "not running" — use [`Self::count_unknown`] for
    /// "could not be determined".
    #[must_use]
    pub fn count_ok(self, alive: u32) -> Self {
        self.scripted
            .lock()
            .expect("scripted mutex")
            .push_back(ScriptedCount::Alive(alive));
        self
    }

    /// Builder: queue the next count as *indeterminate*. Distinct from
    /// `count_ok(0)`: the enumeration never completed, so a close must not be
    /// reported as verified against it.
    #[must_use]
    pub fn count_unknown(self, reason: impl Into<String>) -> Self {
        self.scripted
            .lock()
            .expect("scripted mutex")
            .push_back(ScriptedCount::Unknown(reason.into()));
        self
    }

    /// Builder: make **every** count read fail, proving a failed enumeration
    /// never becomes a fabricated count.
    #[must_use]
    pub fn count_failure(mut self, reason: impl Into<String>) -> Self {
        self.count_failure = Some(reason.into());
        self
    }

    /// Builder: enumerate the modelled process table. An **empty** table is a
    /// valid answer ("nothing is running"), unlike a table that was never
    /// enumerated at all.
    #[must_use]
    pub fn with_running(self, instances: Vec<FakeAppInstance>) -> Self {
        *self.instances.lock().expect("instances mutex") = Some(instances);
        self
    }

    /// Builder: `app_id` exits on its own after the close is requested but
    /// before verification — the user closed it while we were closing it. The
    /// resulting dispatch reports an unattributable effect rather than
    /// claiming credit.
    #[must_use]
    pub fn exits_on_its_own(self, app_id: impl Into<String>) -> Self {
        self.exits_on_its_own
            .lock()
            .expect("self-exit mutex")
            .push(app_id.into());
        self
    }

    /// Builder: script the outcome `terminate_matching` returns.
    #[must_use]
    pub fn dispatch_outcome(self, outcome: ApplyOutcome) -> Self {
        *self.outcome.lock().expect("outcome mutex") = Some(outcome);
        self
    }

    /// Builder: the OS authority refuses the signal; no instance is removed.
    #[must_use]
    pub fn terminate_denied(mut self) -> Self {
        self.terminate_denied = true;
        self
    }

    /// The app ids `terminate_matching` was called with, in order.
    #[must_use]
    pub fn terminate_calls(&self) -> Vec<String> {
        self.terminated.lock().expect("terminate mutex").clone()
    }

    /// How many close dispatches were requested. One graceful close is exactly
    /// one `terminate_matching` — never an escalation to `SIGKILL`, which is
    /// the separate `kill_process` operation.
    #[must_use]
    pub fn dispatch_count(&self) -> usize {
        self.terminated.lock().expect("terminate mutex").len()
    }

    /// The app ids that were counted, in order.
    #[must_use]
    pub fn reads(&self) -> Vec<String> {
        self.reads.lock().expect("reads mutex").clone()
    }

    /// How many count reads were served (successful or not).
    #[must_use]
    pub fn read_count(&self) -> usize {
        self.reads.lock().expect("reads mutex").len()
    }

    /// The modelled table, or `None` when it was never enumerated.
    #[must_use]
    pub fn running(&self) -> Option<Vec<FakeAppInstance>> {
        self.instances.lock().expect("instances mutex").clone()
    }

    /// Whether the last close raced with the app exiting on its own, so the
    /// effect cannot be attributed to our dispatch.
    #[must_use]
    pub fn self_exit_raced(&self) -> bool {
        *self.self_exit_raced.lock().expect("self-exit race mutex")
    }

    /// Whether `candidate` is the same application as `name`: an exact app-id
    /// match, or the `name-<suffix>` prefix form (`gedit` matches
    /// `gedit-worker`). Never a bare substring — `edit` must not match
    /// `gedit` — and never a window title.
    fn is_same_app(name: &str, candidate: &str) -> bool {
        candidate == name
            || candidate
                .strip_prefix(name)
                .is_some_and(|rest| rest.starts_with('-'))
    }

    fn unavailable(&self, reason: impl Into<String>, retryable: bool) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(ProviderId::new(FAKE_APPLICATION_CLOSE_PROVIDER_ID)),
            reason: SafeText::new(reason.into()),
            retryable,
        }
    }

    /// Drop every instance of an app scripted to exit on its own, but only once
    /// a close has been requested — before that, the app really is still up.
    fn settle_self_exits(&self) {
        if !*self.close_requested.lock().expect("close-requested mutex") {
            return;
        }
        let vanishing = self.exits_on_its_own.lock().expect("self-exit mutex").clone();
        if vanishing.is_empty() {
            return;
        }
        let mut table = self.instances.lock().expect("instances mutex");
        if let Some(instances) = table.as_mut() {
            instances.retain(|instance| {
                !vanishing
                    .iter()
                    .any(|app_id| Self::is_same_app(app_id, &instance.app_id))
            });
        }
    }

    /// Serve one count read against the scripted queue and the modelled table.
    /// Kept free of [`HostExecutionContext`] (which the fake ignores) so the
    /// "not running vs. could not be determined" rule is unit-testable without
    /// a governed chain.
    fn serve_count(&self, name: &str) -> Result<u32, OsControlError> {
        self.reads
            .lock()
            .expect("reads mutex")
            .push(name.to_string());

        if let Some(reason) = &self.count_failure {
            return Err(self.unavailable(reason.clone(), true));
        }

        let next = self.scripted.lock().expect("scripted mutex").pop_front();
        match next {
            Some(ScriptedCount::Alive(alive)) => Ok(alive),
            Some(ScriptedCount::Unknown(reason)) => Err(self.unavailable(reason, false)),
            None => {
                self.settle_self_exits();
                // An enumerated table answers 0 honestly; a table that was
                // never enumerated is unknown and must say so.
                self.running()
                    .map(|instances| {
                        instances
                            .iter()
                            .filter(|instance| Self::is_same_app(name, &instance.app_id))
                            .count() as u32
                    })
                    .ok_or_else(|| {
                        self.unavailable(
                            "no application process count scripted on the fake transport",
                            false,
                        )
                    })
            }
        }
    }

    /// Apply one graceful close to the model. Kept free of
    /// [`AdmittedMutationContext`] so the close/exit race is unit-testable.
    fn serve_terminate(&self, name: &str) -> Result<ApplyOutcome, OsControlError> {
        self.terminated
            .lock()
            .expect("terminate mutex")
            .push(name.to_string());

        if self.terminate_denied {
            // A refused signal removes nothing: the table is left as it was.
            return Err(OsControlError::PermissionDenied {
                authority: SafeText::new("fake-application-authority"),
                remediation: SafeText::new("grant signal permission for this process"),
            });
        }

        // The app may have exited on its own the moment we asked it to close.
        let raced = self
            .exits_on_its_own
            .lock()
            .expect("self-exit mutex")
            .iter()
            .any(|app_id| Self::is_same_app(app_id, name));
        *self.close_requested.lock().expect("close-requested mutex") = true;

        // SIGTERM to every matching instance, modelled as removal from the
        // table. Never an escalation, and never a match on a window title.
        //
        // When the app was scripted to exit on its own, the self-exit lands
        // FIRST: the process is already gone by the time our signal arrives, so
        // our signal reaches nothing and `signalled` stays 0. Counting the
        // removal as ours would let the receipt claim an effect the user caused.
        let mut signalled = 0_usize;
        {
            let mut table = self.instances.lock().expect("instances mutex");
            if let Some(instances) = table.as_mut() {
                let before = instances.len();
                instances.retain(|instance| !Self::is_same_app(name, &instance.app_id));
                if !raced {
                    signalled = before - instances.len();
                }
            }
        }

        if let Some(outcome) = self.outcome.lock().expect("outcome mutex").clone() {
            return Ok(outcome);
        }

        if raced && signalled == 0 {
            // The postcondition holds, but our signal is not why. Claiming
            // `Applied` here would credit the receipt with an effect it did not
            // cause; `Unobservable` is the honest answer.
            *self.self_exit_raced.lock().expect("self-exit race mutex") = true;
            return Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
                None,
                UncertainEffectCause::Unobservable,
                BoundedVec::new(),
            )));
        }

        Ok(ApplyOutcome::Applied(AppliedDispatch::new(
            None,
            BoundedVec::new(),
        )))
    }
}

#[async_trait]
impl ApplicationCloseTransport for FakeApplicationCloseTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(FAKE_APPLICATION_CLOSE_PROVIDER_ID)
    }

    async fn count_matching_alive(
        &self,
        _ctx: &HostExecutionContext,
        name: &str,
    ) -> Result<u32, OsControlError> {
        self.serve_count(name)
    }

    async fn terminate_matching(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        name: &str,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.serve_terminate(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_never_enumerated_table_is_unknown_not_not_running() {
        let unknown = FakeApplicationCloseTransport::new();
        assert!(matches!(
            unknown.serve_count("gedit"),
            Err(OsControlError::Unavailable { .. })
        ));

        // An *enumerated but empty* table is a different fact: the app really
        // is not running, and that is an observation, not a failure.
        let empty = FakeApplicationCloseTransport::new().with_running(vec![]);
        assert_eq!(empty.serve_count("gedit").unwrap(), 0);
    }

    #[test]
    fn an_unknown_count_is_distinct_from_a_count_of_zero() {
        let not_running = FakeApplicationCloseTransport::new().count_ok(0);
        assert_eq!(not_running.serve_count("gedit").unwrap(), 0);

        let indeterminate = FakeApplicationCloseTransport::new().count_unknown("procfs read failed");
        assert!(matches!(
            indeterminate.serve_count("gedit"),
            Err(OsControlError::Unavailable { .. })
        ));
    }

    #[test]
    fn two_windows_sharing_a_title_stay_distinguishable() {
        // Both windows are titled "Untitled Document" — a title is neither
        // unique nor stable, so identity must come from the app id.
        let transport = FakeApplicationCloseTransport::new().with_running(vec![
            FakeAppInstance::new("gedit", 101).with_title("Untitled Document"),
            FakeAppInstance::new("libreoffice-writer", 202).with_title("Untitled Document"),
        ]);

        assert_eq!(transport.serve_count("gedit").unwrap(), 1);
        assert_eq!(transport.serve_count("libreoffice-writer").unwrap(), 1);

        // Closing one must not touch the other, despite the identical title.
        transport.serve_terminate("gedit").expect("close dispatched");
        assert_eq!(transport.serve_count("gedit").unwrap(), 0);
        assert_eq!(transport.serve_count("libreoffice-writer").unwrap(), 1);
        let survivors = transport.running().expect("table enumerated");
        assert_eq!(survivors.len(), 1);
        assert_eq!(survivors[0].pid, 202);
    }

    #[test]
    fn matching_is_exact_or_suffixed_never_a_bare_substring() {
        let transport = FakeApplicationCloseTransport::new().with_running(vec![
            FakeAppInstance::new("gedit", 1),
            FakeAppInstance::new("gedit-worker", 2),
            FakeAppInstance::new("notgedit", 3),
        ]);
        // `gedit` and `gedit-worker` are the same application; `notgedit` is
        // a different one that merely contains the name.
        assert_eq!(transport.serve_count("gedit").unwrap(), 2);
        assert_eq!(transport.serve_count("edit").unwrap(), 0);
    }

    #[test]
    fn dispatch_applies_the_effect_to_the_table() {
        let transport = FakeApplicationCloseTransport::new().with_running(vec![
            FakeAppInstance::new("gedit", 1),
            FakeAppInstance::new("gedit-worker", 2),
        ]);
        assert_eq!(transport.serve_count("gedit").unwrap(), 2);
        assert!(matches!(
            transport.serve_terminate("gedit").unwrap(),
            ApplyOutcome::Applied(_)
        ));
        assert_eq!(transport.serve_count("gedit").unwrap(), 0);
        assert_eq!(transport.dispatch_count(), 1, "exactly one SIGTERM round");
        assert_eq!(transport.terminate_calls(), vec!["gedit".to_string()]);
    }

    #[test]
    fn an_app_that_exits_on_its_own_is_not_credited_to_our_close() {
        let transport = FakeApplicationCloseTransport::new()
            .with_running(vec![FakeAppInstance::new("gedit", 1)])
            .exits_on_its_own("gedit");

        // Still up when we look…
        assert_eq!(transport.serve_count("gedit").unwrap(), 1);

        // …but the user closed it in the same instant. The postcondition holds,
        // yet the dispatch must not claim it caused the exit.
        let outcome = transport.serve_terminate("gedit").unwrap();
        assert!(
            matches!(outcome, ApplyOutcome::Uncertain(_)),
            "a self-exit race is unattributable, not Applied"
        );
        assert!(transport.self_exit_raced());
        assert_eq!(transport.serve_count("gedit").unwrap(), 0);
        assert_eq!(transport.dispatch_count(), 1);
    }

    #[test]
    fn a_denied_signal_leaves_the_table_untouched() {
        let transport = FakeApplicationCloseTransport::new()
            .with_running(vec![FakeAppInstance::new("gedit", 1)])
            .terminate_denied();

        assert!(matches!(
            transport.serve_terminate("gedit"),
            Err(OsControlError::PermissionDenied { .. })
        ));
        assert_eq!(
            transport.serve_count("gedit").unwrap(),
            1,
            "a refused close must not look like a successful one"
        );
    }
}
