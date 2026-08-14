//! Deny-live fake [`ProcessTransport`] (OSC-013, OSC-033), Tasks 2.5 / 3.3 / 0.4.
//!
//! Compiled only under `os-control-test`. No signal is ever sent and no
//! `/proc` entry is ever read: the fake keeps an in-memory process table and
//! [`FakeProcessTransport::send_signal`] / [`FakeProcessTransport::set_priority`]
//! apply their effect to that table, so a lifecycle test drives the real
//! governed observe → apply → verify path against state the fake actually
//! changed.
//!
//! # A PID is not an identity
//!
//! The whole point of [`ProcessIdentity`]`{pid, start_time}` is that a PID is
//! reusable: the kernel will hand `4242` to an unrelated process seconds after
//! the original exits. This fake models that honestly.
//!
//! When a process table is scripted (with [`FakeProcessTransport::with_process`])
//! it is **authoritative for identity**, and a signal or renice is refused
//! rather than delivered when the target's identity does not match:
//!
//! * a live process sits at `identity.pid` but its start time differs — the PID
//!   was reused, so the original is absent and signalling would kill an
//!   innocent process. Refused with [`OsControlError::TargetChanged`] and
//!   recorded as [`ProcessRefusal::PidReuse`];
//! * nothing sits at `identity.pid` at all — the process exited between the
//!   observation and the signal. Refused as [`ProcessRefusal::Vanished`];
//! * a target explicitly scripted to exit at signal time
//!   ([`FakeProcessTransport::exits_before_signal`]) models the TOCTOU race
//!   directly: the observation saw it alive, the signal finds it gone.
//!
//! When no table is scripted the fake has no identity facts and does not
//! pretend to: liveness comes from the scripted queue and a signal is
//! delivered. It never *invents* an identity match; it declines to check one it
//! was never given.
//!
//! # Nothing is fabricated
//!
//! An unscripted liveness/priority read is an `Err`, never `false` or `0`. An
//! unscripted process table is an `Err`, never an empty list — "no processes
//! matched" and "I could not read the table" are different facts. An identity
//! absent from a table that *was* read is a genuine absence, reported with
//! [`unknown_process_identity_error`].

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{BoundedVec, Digest, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{AppliedDispatch, ApplyOutcome};

use super::{
    process_permission_denied_error, unknown_process_identity_error, BoundedCommandMetadata,
    ProcessFilter, ProcessIdentity, ProcessLifecycleState, ProcessObservation, ProcessPage,
    ProcessTransport,
};

/// Provider identity reported by the fake transport. Deliberately prefixed
/// `fake-` so a receipt produced through it can never be mistaken for evidence
/// that a real `kill(2)`/`setpriority(2)` succeeded (OSC-033).
pub const FAKE_PROCESS_PROVIDER_ID: &str = "fake-process";

/// One signal the fake **delivered** to a modelled process.
///
/// A refused signal is not recorded here — it never reached a process. See
/// [`FakeProcessTransport::refusals`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalCall {
    /// The identity the signal was delivered to.
    pub identity: ProcessIdentity,
    /// `true` for `SIGKILL` (forced), `false` for `SIGTERM` (graceful).
    pub force: bool,
}

/// A signal or renice the fake refused because the target's identity did not
/// hold at dispatch time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessRefusal {
    /// A live process holds `identity.pid` but with a different start time: the
    /// PID was reused and the requested process is gone. Signalling would have
    /// hit an unrelated process.
    PidReuse {
        /// The identity the caller asked for.
        requested: ProcessIdentity,
        /// The identity actually occupying that PID now.
        observed: ProcessIdentity,
    },
    /// Nothing holds `identity.pid`: the process exited between the observation
    /// and the dispatch.
    Vanished {
        /// The identity the caller asked for.
        requested: ProcessIdentity,
    },
}

/// A scripted, in-memory process transport.
///
/// # Read ordering
///
/// Liveness and priority reads are FIFO queues because one governed mutation
/// performs several in a fixed order (pre-observation → under-lease
/// re-observation → pre-apply capture → post-apply re-observation → verify).
/// Script them with successive [`Self::alive_ok`] / [`Self::priority_ok`] calls.
///
/// When a queue is drained the fake falls back, in order, to the scripted
/// process table (for liveness), then to the last value it served (a steady
/// state), and finally to an error. It never defaults to `false` or `0`.
pub struct FakeProcessTransport {
    alive_script: Mutex<VecDeque<bool>>,
    last_alive: Mutex<Option<bool>>,
    priority_script: Mutex<VecDeque<i32>>,
    /// The modelled niceness, updated by `set_priority`.
    current_nice: Mutex<Option<i32>>,
    /// The modelled process table. `None` means unscripted → a read fails
    /// closed. `Some(vec![])` is a real, readable, empty table.
    table: Mutex<Option<Vec<ProcessObservation>>>,
    /// Scripted bounded argv per identity. `BoundedCommandMetadata` is
    /// deliberately not `Clone` (it is the sole carrier of raw argv), so each
    /// slot is handed out once.
    metadata: Mutex<Vec<(ProcessIdentity, Option<BoundedCommandMetadata>)>>,
    deny_metadata: Mutex<bool>,
    /// Identities that exit exactly at dispatch time, modelling the
    /// observe-then-signal race.
    exits_before_signal: Mutex<Vec<ProcessIdentity>>,
    /// Identities that stay alive through a `SIGTERM` (a real process may
    /// install a handler and ignore it), so the graceful/forced split has
    /// observable consequences in the model.
    ignores_sigterm: Mutex<Vec<ProcessIdentity>>,
    read_fault: Mutex<Option<OsControlError>>,
    dispatch_fault: Mutex<Option<OsControlError>>,
    outcomes: Mutex<VecDeque<ApplyOutcome>>,
    signals: Mutex<Vec<SignalCall>>,
    priorities: Mutex<Vec<(ProcessIdentity, i32)>>,
    refusals: Mutex<Vec<ProcessRefusal>>,
}

impl Default for FakeProcessTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeProcessTransport {
    /// A fake with nothing scripted: no liveness, no priority, no process
    /// table, no argv. Every read fails closed until something is scripted.
    #[must_use]
    pub fn new() -> Self {
        Self {
            alive_script: Mutex::new(VecDeque::new()),
            last_alive: Mutex::new(None),
            priority_script: Mutex::new(VecDeque::new()),
            current_nice: Mutex::new(None),
            table: Mutex::new(None),
            metadata: Mutex::new(Vec::new()),
            deny_metadata: Mutex::new(false),
            exits_before_signal: Mutex::new(Vec::new()),
            ignores_sigterm: Mutex::new(Vec::new()),
            read_fault: Mutex::new(None),
            dispatch_fault: Mutex::new(None),
            outcomes: Mutex::new(VecDeque::new()),
            signals: Mutex::new(Vec::new()),
            priorities: Mutex::new(Vec::new()),
            refusals: Mutex::new(Vec::new()),
        }
    }

    /// Builder: queue the next liveness read as `alive`.
    #[must_use]
    pub fn alive_ok(self, alive: bool) -> Self {
        self.alive_script
            .lock()
            .expect("alive script mutex")
            .push_back(alive);
        self
    }

    /// Builder: queue the next priority read as `nice`.
    #[must_use]
    pub fn priority_ok(self, nice: i32) -> Self {
        self.priority_script
            .lock()
            .expect("priority script mutex")
            .push_back(nice);
        self
    }

    /// Builder: add a process to the modelled table.
    ///
    /// Once any process is added the table is *readable*, and it becomes
    /// authoritative for identity: a signal or renice whose `(pid, start_time)`
    /// does not match a table entry is refused rather than delivered.
    #[must_use]
    pub fn with_process(self, observation: ProcessObservation) -> Self {
        self.table
            .lock()
            .expect("table mutex")
            .get_or_insert_with(Vec::new)
            .push(observation);
        self
    }

    /// Builder: declare the modelled table readable but **empty**. A real,
    /// answered "no processes match" — distinct from an unreadable table.
    #[must_use]
    pub fn with_empty_process_table(self) -> Self {
        *self.table.lock().expect("table mutex") = Some(Vec::new());
        self
    }

    /// Builder: script bounded argv for `identity`.
    ///
    /// Handed out once, because [`BoundedCommandMetadata`] deliberately does
    /// not implement `Clone` — it is the sole carrier of raw argument content.
    #[must_use]
    pub fn with_command_metadata(
        self,
        identity: ProcessIdentity,
        metadata: BoundedCommandMetadata,
    ) -> Self {
        self.metadata
            .lock()
            .expect("metadata mutex")
            .push((identity, Some(metadata)));
        self
    }

    /// Builder: refuse every command-metadata request with
    /// [`process_permission_denied_error`], modelling the fail-closed path when
    /// the RED tool's mandatory approval is rejected before the provider is
    /// ever asked for real argv.
    #[must_use]
    pub fn deny_all_command_metadata(self) -> Self {
        *self.deny_metadata.lock().expect("deny metadata mutex") = true;
        self
    }

    /// Builder: `identity` exits at dispatch time — the observation saw it
    /// alive, the signal finds it gone. Models the observe-then-signal race
    /// directly; the signal is refused, never delivered to whatever holds the
    /// PID next.
    #[must_use]
    pub fn exits_before_signal(self, identity: ProcessIdentity) -> Self {
        self.exits_before_signal
            .lock()
            .expect("exits mutex")
            .push(identity);
        self
    }

    /// Builder: `identity` survives a `SIGTERM` (it handles or ignores it) but
    /// not a `SIGKILL`. Lets a test prove the graceful path is genuinely
    /// weaker, so a graceful kill against such a process verifies as
    /// contradicted rather than satisfied.
    #[must_use]
    pub fn ignores_sigterm(self, identity: ProcessIdentity) -> Self {
        self.ignores_sigterm
            .lock()
            .expect("ignores sigterm mutex")
            .push(identity);
        self
    }

    /// Builder: make every liveness/priority read fail with a retryable
    /// `Unavailable`.
    #[must_use]
    pub fn read_failure(self, reason: impl Into<String>) -> Self {
        *self.read_fault.lock().expect("read fault mutex") = Some(OsControlError::Unavailable {
            provider: Some(ProviderId::new(FAKE_PROCESS_PROVIDER_ID)),
            reason: SafeText::new(reason.into()),
            retryable: true,
        });
        self
    }

    /// Builder: script an arbitrary read fault (timeout, permission denied).
    #[must_use]
    pub fn read_fault(self, error: OsControlError) -> Self {
        *self.read_fault.lock().expect("read fault mutex") = Some(error);
        self
    }

    /// Builder: script an arbitrary dispatch fault. Returned before any effect
    /// reaches the model, so it describes a dispatch that provably did nothing.
    #[must_use]
    pub fn dispatch_fault(self, error: OsControlError) -> Self {
        *self.dispatch_fault.lock().expect("dispatch fault mutex") = Some(error);
        self
    }

    /// Builder: queue the outcome the next dispatch returns. Call once per
    /// expected dispatch (an apply and its rollback are two). When drained, a
    /// dispatch reports `Applied` stamped with the fake-provider tag.
    #[must_use]
    pub fn dispatch_outcome(self, outcome: ApplyOutcome) -> Self {
        self.outcomes
            .lock()
            .expect("outcomes mutex")
            .push_back(outcome);
        self
    }

    /// The signals actually delivered, in order.
    #[must_use]
    pub fn signal_calls(&self) -> Vec<SignalCall> {
        self.signals.lock().expect("signals mutex").clone()
    }

    /// The `(identity, nice)` renices actually delivered, in order.
    #[must_use]
    pub fn priority_calls(&self) -> Vec<(ProcessIdentity, i32)> {
        self.priorities.lock().expect("priorities mutex").clone()
    }

    /// Dispatches refused because the target's identity did not hold, in order.
    #[must_use]
    pub fn refusals(&self) -> Vec<ProcessRefusal> {
        self.refusals.lock().expect("refusals mutex").clone()
    }

    /// How many mutating dispatches the fake accepted and applied (signals plus
    /// renices). Refusals are excluded: nothing was applied.
    #[must_use]
    pub fn dispatch_count(&self) -> usize {
        self.signals.lock().expect("signals mutex").len()
            + self.priorities.lock().expect("priorities mutex").len()
    }

    /// The modelled process table, when one is scripted.
    #[must_use]
    pub fn modelled_table(&self) -> Option<Vec<ProcessObservation>> {
        self.table.lock().expect("table mutex").clone()
    }

    /// The error an unscripted read returns. Never a value.
    fn unscripted(&self, reason: &str) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(ProviderId::new(FAKE_PROCESS_PROVIDER_ID)),
            reason: SafeText::new(reason),
            retryable: false,
        }
    }

    /// Resolve `identity` against a scripted table.
    ///
    /// `Ok(index)` is an exact `(pid, start_time)` match. `Err(refusal)` says
    /// why the identity does not hold: the PID is now held by a different
    /// process, or nothing holds it at all.
    ///
    /// A `start_time` of `0` means the caller never captured one (see
    /// [`ProcessIdentity`]), so reuse cannot be detected and a PID match is the
    /// best available answer — a narrower guarantee, stated rather than
    /// silently claimed.
    fn resolve(
        table: &[ProcessObservation],
        identity: ProcessIdentity,
    ) -> Result<usize, ProcessRefusal> {
        if let Some(index) = table.iter().position(|obs| {
            obs.identity.pid == identity.pid
                && (identity.start_time == 0 || obs.identity.start_time == identity.start_time)
        }) {
            return Ok(index);
        }
        match table.iter().find(|obs| obs.identity.pid == identity.pid) {
            Some(occupant) => Err(ProcessRefusal::PidReuse {
                requested: identity,
                observed: occupant.identity,
            }),
            None => Err(ProcessRefusal::Vanished {
                requested: identity,
            }),
        }
    }

    /// Whether `filter` admits `obs`. `app_id` and `owner` match **exactly**,
    /// never as substrings: filtering for `code` must not also match `vscode`.
    fn matches(filter: &ProcessFilter, obs: &ProcessObservation) -> bool {
        if let Some(state) = filter.state {
            if obs.state != state {
                return false;
            }
        }
        if let Some(owner) = &filter.owner {
            if &obs.owner != owner {
                return false;
            }
        }
        if let Some(app_id) = &filter.app_id {
            if &obs.executable_label != app_id {
                return false;
            }
        }
        if let Some(min_cpu) = filter.min_cpu_percent {
            if obs.cpu_percent < min_cpu {
                return false;
            }
        }
        if let Some(min_memory) = filter.min_memory_bytes {
            if obs.memory_bytes < min_memory {
                return false;
            }
        }
        true
    }

    /// Pop the next scripted outcome, defaulting to a fake-tagged `Applied`.
    fn next_outcome(&self) -> ApplyOutcome {
        self.outcomes
            .lock()
            .expect("outcomes mutex")
            .pop_front()
            .unwrap_or_else(|| {
                ApplyOutcome::Applied(AppliedDispatch::new(
                    Some(Digest::of_str(
                        crate::os_control::testing::FAKE_RECEIPT_TAG,
                    )),
                    BoundedVec::new(),
                ))
            })
    }

    /// Validate `identity` for a mutating dispatch against the scripted table,
    /// recording and returning a refusal when it does not hold.
    ///
    /// `Ok(())` when the table is unscripted: the fake was given no identity
    /// facts and does not invent a match.
    fn admit_mutation(&self, identity: ProcessIdentity) -> Result<(), OsControlError> {
        if self
            .exits_before_signal
            .lock()
            .expect("exits mutex")
            .contains(&identity)
        {
            // Model the exit before refusing, so a later observation agrees the
            // process is gone.
            if let Some(table) = self.table.lock().expect("table mutex").as_mut() {
                table.retain(|obs| obs.identity != identity);
            }
            self.refusals
                .lock()
                .expect("refusals mutex")
                .push(ProcessRefusal::Vanished {
                    requested: identity,
                });
            return Err(OsControlError::TargetChanged);
        }

        let guard = self.table.lock().expect("table mutex");
        let Some(table) = guard.as_ref() else {
            return Ok(());
        };
        match Self::resolve(table, identity) {
            Ok(_) => Ok(()),
            Err(refusal) => {
                drop(guard);
                self.refusals.lock().expect("refusals mutex").push(refusal);
                Err(OsControlError::TargetChanged)
            }
        }
    }
}

#[async_trait]
impl ProcessTransport for FakeProcessTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(FAKE_PROCESS_PROVIDER_ID)
    }

    async fn read_alive(
        &self,
        _ctx: &HostExecutionContext,
        identity: ProcessIdentity,
    ) -> Result<bool, OsControlError> {
        if let Some(fault) = self.read_fault.lock().expect("read fault mutex").clone() {
            return Err(fault);
        }
        // 1. An explicitly scripted read always wins: that is how a test stages
        //    a post-apply contradiction on purpose.
        if let Some(alive) = self
            .alive_script
            .lock()
            .expect("alive script mutex")
            .pop_front()
        {
            *self.last_alive.lock().expect("last alive mutex") = Some(alive);
            return Ok(alive);
        }
        // 2. Otherwise the modelled table answers, PID-reuse safe: a reused PID
        //    means the *requested* process is absent, never the occupant's
        //    liveness reported in its place.
        if let Some(table) = self.table.lock().expect("table mutex").as_ref() {
            let alive = match Self::resolve(table, identity) {
                Ok(index) => table[index].state != ProcessLifecycleState::Zombie,
                Err(_) => false,
            };
            return Ok(alive);
        }
        // 3. Then the last value served (a steady state), and finally an error.
        self.last_alive
            .lock()
            .expect("last alive mutex")
            .ok_or_else(|| self.unscripted("no process liveness scripted on the fake transport"))
    }

    async fn read_priority(
        &self,
        _ctx: &HostExecutionContext,
        _identity: ProcessIdentity,
    ) -> Result<i32, OsControlError> {
        if let Some(fault) = self.read_fault.lock().expect("read fault mutex").clone() {
            return Err(fault);
        }
        let next = self
            .priority_script
            .lock()
            .expect("priority script mutex")
            .pop_front();
        let mut current = self.current_nice.lock().expect("current nice mutex");
        if let Some(nice) = next {
            *current = Some(nice);
        }
        current.ok_or_else(|| self.unscripted("no process priority scripted on the fake transport"))
    }

    async fn send_signal(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        identity: ProcessIdentity,
        force: bool,
    ) -> Result<ApplyOutcome, OsControlError> {
        if let Some(fault) = self
            .dispatch_fault
            .lock()
            .expect("dispatch fault mutex")
            .clone()
        {
            return Err(fault);
        }
        self.admit_mutation(identity)?;

        // Delivered, never sent: no `kill(2)` reaches the host.
        self.signals
            .lock()
            .expect("signals mutex")
            .push(SignalCall { identity, force });

        let outcome = self.next_outcome();
        let took_effect = matches!(
            outcome,
            ApplyOutcome::Applied(_) | ApplyOutcome::Accepted(_)
        );
        let survives = !force
            && self
                .ignores_sigterm
                .lock()
                .expect("ignores sigterm mutex")
                .contains(&identity);

        if took_effect && !survives {
            // Apply the effect to the model: the process is gone, so a
            // re-observation reports it dead because it *is* dead here.
            if let Some(table) = self.table.lock().expect("table mutex").as_mut() {
                table.retain(|obs| obs.identity != identity);
            }
            *self.last_alive.lock().expect("last alive mutex") = Some(false);
        }
        Ok(outcome)
    }

    async fn set_priority(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        identity: ProcessIdentity,
        nice: i32,
    ) -> Result<ApplyOutcome, OsControlError> {
        if let Some(fault) = self
            .dispatch_fault
            .lock()
            .expect("dispatch fault mutex")
            .clone()
        {
            return Err(fault);
        }
        self.admit_mutation(identity)?;

        self.priorities
            .lock()
            .expect("priorities mutex")
            .push((identity, nice));

        let outcome = self.next_outcome();
        if matches!(
            outcome,
            ApplyOutcome::Applied(_) | ApplyOutcome::Accepted(_)
        ) {
            *self.current_nice.lock().expect("current nice mutex") = Some(nice);
        }
        Ok(outcome)
    }

    async fn list_observations(
        &self,
        _ctx: &HostExecutionContext,
        filter: &ProcessFilter,
        cursor: usize,
        limit: usize,
    ) -> Result<ProcessPage, OsControlError> {
        let guard = self.table.lock().expect("table mutex");
        // Unscripted is "could not read the table", which is not an empty table.
        let table = guard
            .as_ref()
            .ok_or_else(|| self.unscripted("no process table scripted on the fake transport"))?;

        let matched: Vec<ProcessObservation> = table
            .iter()
            .filter(|obs| Self::matches(filter, obs))
            .cloned()
            .collect();
        let items: Vec<ProcessObservation> =
            matched.iter().skip(cursor).take(limit).cloned().collect();
        let truncated = cursor.saturating_add(items.len()) < matched.len();
        Ok(ProcessPage { items, truncated })
    }

    async fn read_observation(
        &self,
        _ctx: &HostExecutionContext,
        identity: ProcessIdentity,
    ) -> Result<ProcessObservation, OsControlError> {
        let guard = self.table.lock().expect("table mutex");
        let table = guard
            .as_ref()
            .ok_or_else(|| self.unscripted("no process table scripted on the fake transport"))?;
        // A reused PID means the requested identity is absent — never the
        // unrelated occupant's observation returned in its place.
        match Self::resolve(table, identity) {
            Ok(index) => Ok(table[index].clone()),
            Err(_) => Err(unknown_process_identity_error()),
        }
    }

    async fn read_command_metadata(
        &self,
        _ctx: &HostExecutionContext,
        identity: ProcessIdentity,
        purpose: &str,
    ) -> Result<BoundedCommandMetadata, OsControlError> {
        // Fail closed before touching the table: a denied RED request must
        // never reveal whether the process even exists.
        if *self.deny_metadata.lock().expect("deny metadata mutex") {
            return Err(process_permission_denied_error());
        }
        if purpose.trim().is_empty() {
            return Err(OsControlError::InvalidRequest {
                field: crate::os_control::contract::SafeField::new("purpose"),
                reason: SafeText::new(
                    "command metadata is a mandatory-approval read; it requires a stated purpose",
                ),
            });
        }

        let mut slots = self.metadata.lock().expect("metadata mutex");
        let Some(slot) = slots
            .iter_mut()
            .find(|(scripted, _)| *scripted == identity)
            .map(|(_, metadata)| metadata)
        else {
            return Err(unknown_process_identity_error());
        };
        slot.take().ok_or_else(|| {
            self.unscripted(
                "the fake's scripted command metadata was already consumed; \
                 BoundedCommandMetadata is deliberately not Clone, so script one per read",
            )
        })
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    fn observation(pid: u32, start_time: u64, label: &str) -> ProcessObservation {
        ProcessObservation::new(
            ProcessIdentity::new(pid, start_time),
            label,
            Digest::of_str(label),
            "1000",
            ProcessLifecycleState::Running,
            1,
            1024,
        )
    }

    #[test]
    fn resolve_distinguishes_pid_reuse_from_a_vanished_process() {
        let table = vec![observation(100, 111, "gedit")];

        assert_eq!(Ok(0), FakeProcessTransport::resolve(&table, ProcessIdentity::new(100, 111)));

        // Same PID, different start time: the requested process is gone and an
        // unrelated one holds its PID.
        assert_eq!(
            Err(ProcessRefusal::PidReuse {
                requested: ProcessIdentity::new(100, 999),
                observed: ProcessIdentity::new(100, 111),
            }),
            FakeProcessTransport::resolve(&table, ProcessIdentity::new(100, 999))
        );

        // Nothing holds the PID at all.
        assert_eq!(
            Err(ProcessRefusal::Vanished {
                requested: ProcessIdentity::new(4242, 1),
            }),
            FakeProcessTransport::resolve(&table, ProcessIdentity::new(4242, 1))
        );

        // A `start_time` of 0 means "not captured": reuse cannot be detected,
        // so a PID match is the best available answer.
        assert_eq!(Ok(0), FakeProcessTransport::resolve(&table, ProcessIdentity::new(100, 0)));
    }

    #[tokio::test]
    async fn a_reused_pid_is_refused_instead_of_signalling_the_wrong_process() {
        let fake = FakeProcessTransport::new().with_process(observation(100, 111, "gedit"));
        let stale = ProcessIdentity::new(100, 555);

        let refused = fake.admit_mutation(stale);
        assert!(matches!(refused, Err(OsControlError::TargetChanged)));
        assert_eq!(
            fake.refusals(),
            vec![ProcessRefusal::PidReuse {
                requested: stale,
                observed: ProcessIdentity::new(100, 111),
            }]
        );
        assert_eq!(fake.dispatch_count(), 0, "a refused signal is never delivered");
        // The innocent occupant is untouched.
        assert_eq!(fake.modelled_table().unwrap().len(), 1);
    }

    #[test]
    fn a_process_that_exits_before_the_signal_is_refused_and_leaves_the_model_consistent() {
        let identity = ProcessIdentity::new(77, 900);
        let fake = FakeProcessTransport::new()
            .with_process(observation(77, 900, "doomed"))
            .exits_before_signal(identity);

        assert!(matches!(
            fake.admit_mutation(identity),
            Err(OsControlError::TargetChanged)
        ));
        assert_eq!(
            fake.refusals(),
            vec![ProcessRefusal::Vanished {
                requested: identity
            }]
        );
        assert!(
            fake.modelled_table().unwrap().is_empty(),
            "the modelled process really did exit"
        );
        assert_eq!(fake.dispatch_count(), 0);
    }

    #[test]
    fn an_unscripted_table_is_an_error_and_an_empty_table_is_an_answer() {
        assert!(FakeProcessTransport::new().modelled_table().is_none());
        assert_eq!(
            FakeProcessTransport::new()
                .with_empty_process_table()
                .modelled_table(),
            Some(Vec::new())
        );
    }

    #[test]
    fn an_unscripted_table_does_not_block_a_signal_it_cannot_check() {
        // No identity facts were given, so the fake declines to check rather
        // than inventing a match — or inventing a refusal.
        let fake = FakeProcessTransport::new();
        assert!(fake.admit_mutation(ProcessIdentity::new(1, 0)).is_ok());
        assert!(fake.refusals().is_empty());
    }

    #[test]
    fn app_id_filtering_is_exact_never_a_substring() {
        let filter = ProcessFilter {
            app_id: Some("code".to_string()),
            ..Default::default()
        };
        assert!(FakeProcessTransport::matches(
            &filter,
            &observation(10, 1, "code")
        ));
        assert!(!FakeProcessTransport::matches(
            &filter,
            &observation(20, 2, "vscode")
        ));
        assert!(!FakeProcessTransport::matches(
            &filter,
            &observation(30, 3, "code-helper")
        ));
    }
}
