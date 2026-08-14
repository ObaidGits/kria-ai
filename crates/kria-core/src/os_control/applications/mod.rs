//! Application domain: the graceful-close slice of `ApplicationControl`
//! (design §3, §9.3).
//!
//! linux-os-control-production **Task 2.5** — "Migrate files, processes,
//! applications, packages, scheduler, disk, clipboard and notifications"
//! (OSC-007–OSC-014, OSC-021–OSC-023).
//!
//! This module replaces the direct `sysinfo::Process::kill()` (unconditional
//! `SIGKILL`) loop that used to live in
//! `tools/app_lifecycle.rs::CloseApplication`. It implements **only** the
//! graceful-close slice of `ApplicationControl` (`ApplicationControl.close`,
//! design §9.3); launch/list/autostart/default-app remain owned by the
//! existing `IntentDispatcher`/`InstalledAppRegistry` composition
//! (`open_application`, `list_running_apps`, …) and Task 3.3's scope.
//!
//! # Split graceful close from kill (explicit Task 2.5 requirement)
//!
//! `graceful_close_application` sends `SIGTERM` to every process matching the
//! target application name and never escalates to `SIGKILL` on its own — a
//! forced, unconditional kill is the separate, PID-targeted
//! [`crate::os_control::processes`] domain's `kill_process` operation, at a
//! distinct (higher) risk tier. The two are never merged into one tool.
//!
//! * [`ApplicationCloseState`] is a normalized observation
//!   ([`NormalizedObservation`]) counting how many matching processes remain
//!   alive, so idempotency/verification are real (already-closed → zero
//!   matches → `Unchanged`).
//! * [`ApplicationCloseControl`] implements the generic
//!   [`DesiredStateControl`] lifecycle. `rollback` always reports the
//!   truthful "no inverse" fact: the frozen manifest declares
//!   `rollbackClaim: None` for `graceful_close_application`.
//! * The live transport
//!   ([`crate::os_control::linux::providers::process_control::LiveProcessControl`]
//!   also backs this domain — the same native syscall boundary — via
//!   [`crate::os_control::linux::providers::application_control`]) is a raw,
//!   deny-live-gated adapter; deny-live tests inject
//!   [`fake::FakeApplicationCloseTransport`].

use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId, SafeErrorCode,
    SafeField, SafeText, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification, UncertainDispatch,
    UncertainEffectCause, VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;

#[cfg(feature = "os-control-test")]
pub mod fake;
#[cfg(feature = "os-control-test")]
pub mod fake_association;

pub mod selection;

/// The stable provider identity for the native-syscall application-close backend.
pub const APPLICATION_CLOSE_PROVIDER_ID: &str = "application-close-native-syscall";

/// The stable provider identity for the freedesktop default-application/
/// autostart backend (Task 3.3, design §9.2).
pub const DESKTOP_ASSOCIATION_PROVIDER_ID: &str = "application-desktop-association";

/// A normalized observation of how many processes matching an application
/// name remain alive (design §5, §9.3).
#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationCloseState {
    /// The canonical (lowercased) application name matched against.
    pub name: String,
    /// The number of currently-alive processes matching that name.
    pub matching_alive: u32,
}

impl ApplicationCloseState {
    /// Construct an observation.
    #[must_use]
    pub fn new(name: impl Into<String>, matching_alive: u32) -> Self {
        Self {
            name: name.into(),
            matching_alive,
        }
    }
}

impl NormalizedObservation for ApplicationCloseState {
    fn observation_digest(&self) -> Digest {
        // Any nonzero count is "still running" for idempotency purposes; a
        // desired state of zero only ever needs to distinguish
        // "some remain" from "none remain", so the digest binds the boolean
        // rather than the exact count (a transient extra helper process must
        // not block idempotency convergence).
        Digest::of_str(&format!(
            "app-close:{}:{}",
            self.name,
            self.matching_alive > 0
        ))
    }
}

/// A fully-described graceful-close request.
#[derive(Debug, Clone)]
pub struct ApplicationCloseRequest {
    /// The canonical tool/action name the grant was minted against
    /// (`graceful_close_application`).
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params digest).
    pub params: serde_json::Value,
    /// The canonical (lowercased) application name to close.
    pub name: String,
}

impl ApplicationCloseRequest {
    /// The desired end state: zero matching processes remain alive.
    #[must_use]
    pub fn desired_state(&self) -> ApplicationCloseState {
        ApplicationCloseState::new(self.name.clone(), 0)
    }

    /// The idempotency/verification comparator (the frozen manifest names
    /// `ExactTypedPostcondition`).
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transport seam
// ─────────────────────────────────────────────────────────────────────────────

/// The raw application-close transport seam. The live implementation is a
/// deny-live-gated adapter over the same native `kill(2)` boundary as
/// [`crate::os_control::processes`] (no subprocess); deny-live tests inject
/// [`fake::FakeApplicationCloseTransport`].
#[async_trait]
pub trait ApplicationCloseTransport: Send + Sync {
    /// The stable provider identity (never model prose).
    fn provider_id(&self) -> ProviderId;

    /// Count how many currently-alive processes match `name` (exact name or
    /// `name-<suffix>` prefix match, preserving the pre-migration
    /// `CloseApplication` matching semantics — never a bare substring match).
    async fn count_matching_alive(
        &self,
        ctx: &HostExecutionContext,
        name: &str,
    ) -> Result<u32, OsControlError>;

    /// Send `SIGTERM` to every currently-alive process matching `name`. A
    /// native `kill(2)` syscall per match — never a subprocess, and never
    /// `SIGKILL` (that escalation is the separate `kill_process` operation).
    async fn terminate_matching(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        name: &str,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The graceful-close slice of `ApplicationControl` (design §3, §4, §9.3).
/// Generic over the [`ApplicationCloseTransport`] so the same governed logic
/// runs over the live native-syscall adapter and the deny-live fake.
pub struct ApplicationCloseControl<T: ApplicationCloseTransport> {
    transport: T,
}

impl<T: ApplicationCloseTransport> ApplicationCloseControl<T> {
    /// Compose over a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Borrow the underlying transport (used by tests).
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The provider identity.
    #[must_use]
    pub fn provider_id(&self) -> ProviderId {
        self.transport.provider_id()
    }

    fn evidence_source(&self) -> OsEvidenceSource {
        OsEvidenceSource::IndependentProviderQuery
    }

    fn satisfying(
        &self,
        observed: &ApplicationCloseState,
    ) -> SatisfyingVerification<ApplicationCloseState> {
        SatisfyingVerification::new(
            self.evidence_source(),
            VerificationReliability::Strong,
            self.transport.provider_id(),
            RedactedObservation::new(observed.clone(), observed.observation_digest()),
            None,
            SystemTime::now(),
            0,
        )
    }
}

#[async_trait]
impl<T: ApplicationCloseTransport>
    DesiredStateControl<ApplicationCloseRequest, ApplicationCloseState>
    for ApplicationCloseControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &ApplicationCloseRequest,
    ) -> Result<ApplicationCloseState, OsControlError> {
        let count = self
            .transport
            .count_matching_alive(ctx, &request.name)
            .await?;
        Ok(ApplicationCloseState::new(request.name.clone(), count))
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &ApplicationCloseRequest,
        _desired: &ApplicationCloseState,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.transport.terminate_matching(ctx, &request.name).await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &ApplicationCloseRequest,
        desired: &ApplicationCloseState,
    ) -> Result<VerificationReport<ApplicationCloseState>, OsControlError> {
        let count = self
            .transport
            .count_matching_alive(ctx, &request.name)
            .await?;
        let observed = ApplicationCloseState::new(request.name.clone(), count);

        if observed.observation_digest() == desired.observation_digest() {
            Ok(VerificationReport::Satisfied(self.satisfying(&observed)))
        } else {
            Ok(VerificationReport::Contradicted(
                VerificationContradiction::new(
                    desired.observation_digest(),
                    Some(observed.observation_digest()),
                    SafeErrorCode::from_static("os_control.incident.contradicted"),
                ),
            ))
        }
    }

    async fn rollback(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        _token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError> {
        // `rollbackClaim: None` in the frozen manifest — closing an
        // application is never reversible; this is never actually invoked.
        Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
            None,
            UncertainEffectCause::Unobservable,
            crate::os_control::contract::BoundedVec::new(),
        )))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Receipt → tool-result mapping
// ─────────────────────────────────────────────────────────────────────────────

use crate::os_control::receipt::{ActionLifecycle, MutationReceipt};

/// Map a governed [`MutationReceipt`] to the **existing** `close_application`
/// result fields (`name`, `processes_closed` renamed-compatible as
/// `already_in_desired_state`/`changed`), plus additive `lifecycle`/
/// `verified` fields. `processes_closed` is best-effort reported from the
/// pre-apply observation since the receipt itself only proves the *end*
/// count.
#[must_use]
pub fn graceful_close_result(
    receipt: &MutationReceipt<ApplicationCloseState>,
    name: &str,
    processes_matched_before: u32,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "name": name,
        "processes_closed": processes_matched_before,
        "changed": receipt.changed(),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// `HostOsControl::application_close()` port seam (design §4)
// ─────────────────────────────────────────────────────────────────────────────

/// The dyn-compatible application-close domain port. Because the concrete
/// [`ApplicationCloseControl`] provider struct above is generic over its
/// [`ApplicationCloseTransport`], `HostOsControl::application_close()`
/// returns this object-safe supertrait instead so any transport (live
/// native-syscall, or a deny-live fake) can be composed behind one erased
/// reference. Every [`ApplicationCloseControl<T>`] implements it
/// automatically via the blanket impl below.
pub trait ApplicationCloseControlPort:
    DesiredStateControl<ApplicationCloseRequest, ApplicationCloseState>
{
}

impl<T: ApplicationCloseTransport> ApplicationCloseControlPort for ApplicationCloseControl<T> {}

// ─────────────────────────────────────────────────────────────────────────────
// list_installed_apps — pure read wrapping InstalledAppRegistry (Task 3.3)
// ─────────────────────────────────────────────────────────────────────────────
//
// Per design §9.2 ("do not duplicate .desktop parsing"), this DTO wraps the
// existing `InstalledAppRegistry`'s already-scanned manifests rather than
// re-scanning `.desktop` files. The registry lives in `platform::app_registry`
// (outside `os_control`); the tool handler in `tools/app_lifecycle.rs`
// composes it directly rather than through a `DesiredStateControl` provider,
// mirroring how `list_running_apps` already works (a pure read with no
// mutation lifecycle at all).

/// One installed application entry (frozen manifest `ApplicationPage` item).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledApplication {
    /// The stable canonical application id.
    pub app_id: String,
    /// The human-readable display name.
    pub display_name: String,
    /// A digest binding the `.desktop` entry's content (fingerprint-derived).
    pub desktop_entry_digest: Digest,
    /// Whether the desktop entry is currently resolvable/launchable.
    pub available: bool,
}

/// A bounded page of installed applications (`list_installed_apps`'s
/// `ApplicationPage`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InstalledApplicationPage {
    /// The applications in this page.
    pub items: Vec<InstalledApplication>,
    /// Whether more applications exist beyond this page.
    pub truncated: bool,
}

/// Map an [`InstalledApplicationPage`] to the `list_installed_apps` result
/// fields.
#[must_use]
pub fn list_installed_apps_result(page: &InstalledApplicationPage) -> serde_json::Value {
    let items: Vec<serde_json::Value> = page
        .items
        .iter()
        .map(|item| {
            serde_json::json!({
                "app_id": item.app_id,
                "display_name": item.display_name,
                "desktop_entry_digest": item.desktop_entry_digest.as_hex(),
                "available": item.available,
            })
        })
        .collect();
    serde_json::json!({
        "items": items,
        "truncated": page.truncated,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Default application / MIME association + autostart (Task 3.3, OSC-013.9)
// ─────────────────────────────────────────────────────────────────────────────
//
// Both `set_default_application` (freedesktop `~/.config/mimeapps.list`) and
// `manage_autostart` (freedesktop `~/.config/autostart/*.desktop`) are plain
// `std::fs`/text-format operations against an injectable configuration root
// — never D-Bus/subprocess — mirroring `os_control::files::trash`'s
// injectable-root pattern rather than the D-Bus-fake-transport pattern used
// by audio/display/connectivity/storage. Before-state capture (OSC-013.9)
// happens inside `apply()`, exactly like `AudioControl`'s volume/mute
// rollback snapshot.

/// Which desktop-association mutation this domain owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssociationFocus {
    /// `set_default_application`: a MIME type's default application.
    DefaultApplication,
    /// `manage_autostart`: whether an app's autostart entry is enabled.
    Autostart,
}

/// A normalized desktop-association observation (design §9.2,
/// `DefaultApplicationState`/`AutostartState`). Bound to the focus + target
/// identity so a default-app observation for one MIME type never collides
/// with an autostart observation for an unrelated app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssociationState {
    /// The comparison focus.
    pub focus: AssociationFocus,
    /// The MIME type (for [`AssociationFocus::DefaultApplication`]) or app id
    /// (for [`AssociationFocus::Autostart`]) this observation targets.
    pub target: String,
    /// For [`AssociationFocus::DefaultApplication`]: the current default
    /// app id, if any. Always `None` for [`AssociationFocus::Autostart`].
    pub default_app_id: Option<String>,
    /// For [`AssociationFocus::Autostart`]: whether autostart is currently
    /// enabled. Always `false` (ignored by the digest) for
    /// [`AssociationFocus::DefaultApplication`].
    pub autostart_enabled: bool,
}

impl AssociationState {
    /// Construct a default-application-focused observation.
    #[must_use]
    pub fn default_application(mime: impl Into<String>, app_id: Option<String>) -> Self {
        Self {
            focus: AssociationFocus::DefaultApplication,
            target: mime.into(),
            default_app_id: app_id,
            autostart_enabled: false,
        }
    }

    /// Construct an autostart-focused observation.
    #[must_use]
    pub fn autostart(app_id: impl Into<String>, enabled: bool) -> Self {
        Self {
            focus: AssociationFocus::Autostart,
            target: app_id.into(),
            default_app_id: None,
            autostart_enabled: enabled,
        }
    }
}

impl NormalizedObservation for AssociationState {
    fn observation_digest(&self) -> Digest {
        match self.focus {
            AssociationFocus::DefaultApplication => Digest::of_str(&format!(
                "association:default-app:{}:{}",
                self.target,
                self.default_app_id.as_deref().unwrap_or("")
            )),
            AssociationFocus::Autostart => Digest::of_str(&format!(
                "association:autostart:{}:{}",
                self.target, self.autostart_enabled
            )),
        }
    }
}

/// The concrete desktop-association operation.
#[derive(Debug, Clone)]
pub enum AssociationOp {
    /// Set `mime`'s default application to `app_id`
    /// (`set_default_application`).
    SetDefaultApplication {
        /// The MIME type to associate.
        mime: String,
        /// The application id to make the default handler.
        app_id: String,
    },
    /// Enable/disable `app_id`'s user autostart entry (`manage_autostart`).
    SetAutostart {
        /// The application id.
        app_id: String,
        /// The desired autostart-enabled state.
        enabled: bool,
    },
}

/// A fully-described desktop-association request. Carries the canonical
/// `action`/`params` for grant binding.
#[derive(Debug, Clone)]
pub struct AssociationRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params
    /// digest).
    pub params: serde_json::Value,
    /// The concrete operation.
    pub op: AssociationOp,
}

impl AssociationRequest {
    /// The comparison focus implied by the operation.
    #[must_use]
    pub fn focus(&self) -> AssociationFocus {
        match self.op {
            AssociationOp::SetDefaultApplication { .. } => AssociationFocus::DefaultApplication,
            AssociationOp::SetAutostart { .. } => AssociationFocus::Autostart,
        }
    }

    /// The desired end state for this mutation.
    #[must_use]
    pub fn desired_state(&self) -> AssociationState {
        match &self.op {
            AssociationOp::SetDefaultApplication { mime, app_id } => {
                AssociationState::default_application(mime.clone(), Some(app_id.clone()))
            }
            AssociationOp::SetAutostart { app_id, enabled } => {
                AssociationState::autostart(app_id.clone(), *enabled)
            }
        }
    }

    /// The idempotency/verification comparator (`ExactTypedPostcondition` in
    /// the frozen manifest for both operations).
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

/// The raw desktop-association transport seam. The live implementation
/// ([`RealDesktopAssociationTransport`]) is a plain `std::fs` adapter over an
/// injectable configuration root; deny-live tests inject
/// [`fake::FakeDesktopAssociationTransport`].
#[async_trait]
pub trait DesktopAssociationTransport: Send + Sync {
    /// The stable provider identity (never model prose).
    fn provider_id(&self) -> ProviderId;

    /// Read the current default application for `mime`, if any.
    async fn read_default_application(
        &self,
        mime: &str,
    ) -> Result<Option<String>, OsControlError>;

    /// Read whether `app_id`'s autostart entry is currently enabled.
    async fn read_autostart(&self, app_id: &str) -> Result<bool, OsControlError>;

    /// Set `mime`'s default application to `app_id`.
    async fn set_default_application(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        mime: &str,
        app_id: &str,
    ) -> Result<ApplyOutcome, OsControlError>;

    /// Enable/disable `app_id`'s user autostart entry.
    async fn set_autostart(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        app_id: &str,
        enabled: bool,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The captured prior association state for rollback (OSC-013.9), keyed by
/// session id.
#[derive(Debug, Clone)]
struct AssociationRollbackSnapshot {
    op: AssociationOp,
    before: AssociationState,
}

/// The desktop-association domain provider (`set_default_application`,
/// `manage_autostart`; design §9.2, §4). Generic over the
/// [`DesktopAssociationTransport`] so the same governed logic runs over the
/// real `std::fs` adapter and the deny-live fake.
pub struct DesktopAssociationControl<T: DesktopAssociationTransport> {
    transport: T,
    snapshots: Mutex<HashMap<String, AssociationRollbackSnapshot>>,
}

impl<T: DesktopAssociationTransport> DesktopAssociationControl<T> {
    /// Compose over a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            snapshots: Mutex::new(HashMap::new()),
        }
    }

    /// Borrow the underlying transport (used by tests).
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The provider identity.
    #[must_use]
    pub fn provider_id(&self) -> ProviderId {
        self.transport.provider_id()
    }

    fn evidence_source(&self) -> OsEvidenceSource {
        OsEvidenceSource::AuthoritativeServiceState
    }

    fn satisfying(&self, observed: &AssociationState) -> SatisfyingVerification<AssociationState> {
        SatisfyingVerification::new(
            self.evidence_source(),
            VerificationReliability::Strong,
            self.transport.provider_id(),
            RedactedObservation::new(observed.clone(), observed.observation_digest()),
            None,
            SystemTime::now(),
            0,
        )
    }
}

#[async_trait]
impl<T: DesktopAssociationTransport> DesiredStateControl<AssociationRequest, AssociationState>
    for DesktopAssociationControl<T>
{
    async fn observe(
        &self,
        _ctx: &HostExecutionContext,
        request: &AssociationRequest,
    ) -> Result<AssociationState, OsControlError> {
        match &request.op {
            AssociationOp::SetDefaultApplication { mime, .. } => {
                let current = self.transport.read_default_application(mime).await?;
                Ok(AssociationState::default_application(mime.clone(), current))
            }
            AssociationOp::SetAutostart { app_id, .. } => {
                let enabled = self.transport.read_autostart(app_id).await?;
                Ok(AssociationState::autostart(app_id.clone(), enabled))
            }
        }
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &AssociationRequest,
        _desired: &AssociationState,
    ) -> Result<ApplyOutcome, OsControlError> {
        // Capture the pre-apply state so a rollback can restore the exact
        // prior association (OSC-013.9).
        let before = self.observe(ctx.observation(), request).await.ok();
        if let Some(before) = before {
            let session = ctx.grant().session_id().to_string();
            self.snapshots.lock().expect("association snapshots poisoned").insert(
                session,
                AssociationRollbackSnapshot {
                    op: request.op.clone(),
                    before,
                },
            );
        }

        match &request.op {
            AssociationOp::SetDefaultApplication { mime, app_id } => {
                self.transport.set_default_application(ctx, mime, app_id).await
            }
            AssociationOp::SetAutostart { app_id, enabled } => {
                self.transport.set_autostart(ctx, app_id, *enabled).await
            }
        }
    }

    async fn verify(
        &self,
        _ctx: &HostExecutionContext,
        request: &AssociationRequest,
        desired: &AssociationState,
    ) -> Result<VerificationReport<AssociationState>, OsControlError> {
        let observed = match &request.op {
            AssociationOp::SetDefaultApplication { mime, .. } => {
                let current = self.transport.read_default_application(mime).await?;
                AssociationState::default_application(mime.clone(), current)
            }
            AssociationOp::SetAutostart { app_id, .. } => {
                let enabled = self.transport.read_autostart(app_id).await?;
                AssociationState::autostart(app_id.clone(), enabled)
            }
        };

        if observed.observation_digest() == desired.observation_digest() {
            Ok(VerificationReport::Satisfied(self.satisfying(&observed)))
        } else {
            Ok(VerificationReport::Contradicted(
                VerificationContradiction::new(
                    desired.observation_digest(),
                    Some(observed.observation_digest()),
                    SafeErrorCode::from_static("os_control.incident.contradicted"),
                ),
            ))
        }
    }

    async fn rollback(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError> {
        let snapshot = self
            .snapshots
            .lock()
            .expect("association snapshots poisoned")
            .get(token.session_id().as_str())
            .cloned();

        let Some(snapshot) = snapshot else {
            return Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
                None,
                UncertainEffectCause::Unobservable,
                crate::os_control::contract::BoundedVec::new(),
            )));
        };

        match snapshot.op {
            AssociationOp::SetDefaultApplication { mime, .. } => match snapshot.before.default_app_id {
                Some(prior_app_id) => {
                    self.transport
                        .set_default_application(ctx, &mime, &prior_app_id)
                        .await
                }
                // No prior default existed — there is no restorable inverse
                // (freedesktop has no "unset default" primitive this task
                // implements); report the truthful "no inverse" fact.
                None => Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
                    None,
                    UncertainEffectCause::Unobservable,
                    crate::os_control::contract::BoundedVec::new(),
                ))),
            },
            AssociationOp::SetAutostart { app_id, .. } => {
                self.transport
                    .set_autostart(ctx, &app_id, snapshot.before.autostart_enabled)
                    .await
            }
        }
    }
}

/// Map a governed [`MutationReceipt`] to the `set_default_application`
/// result fields.
#[must_use]
pub fn set_default_application_result(
    receipt: &MutationReceipt<AssociationState>,
    mime: &str,
    app_id: &str,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "mime": mime,
        "app_id": app_id,
        "changed": receipt.changed(),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// Map a governed [`MutationReceipt`] to the `manage_autostart` result
/// fields.
#[must_use]
pub fn manage_autostart_result(
    receipt: &MutationReceipt<AssociationState>,
    app_id: &str,
    enabled: bool,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "app_id": app_id,
        "enabled": enabled,
        "changed": receipt.changed(),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// The dyn-compatible desktop-association domain port.
pub trait DesktopAssociationControlPort:
    DesiredStateControl<AssociationRequest, AssociationState>
{
}

impl<T: DesktopAssociationTransport> DesktopAssociationControlPort
    for DesktopAssociationControl<T>
{
}

// ─────────────────────────────────────────────────────────────────────────────
// Real freedesktop configuration transport (std::fs, injectable root)
// ─────────────────────────────────────────────────────────────────────────────

/// A fully-functional `std::fs`-backed desktop-association transport over an
/// **injectable** XDG config root (never `dirs::config_dir()` directly — see
/// [`RealDesktopAssociationTransport::new`]). Reads/writes
/// `<root>/mimeapps.list` (freedesktop MIME association spec's `[Default
/// Applications]` section) and `<root>/autostart/<app_id>.desktop` (XDG
/// autostart spec's `Hidden`/`X-GNOME-Autostart-enabled` convention — this
/// implementation uses the simpler `Hidden=true` inverse-of-enabled
/// convention, honored by every major desktop). Tests always inject a
/// `tempfile::TempDir` (mirrors `RealTrashTransport`'s injectable root); this
/// type never calls `dirs::config_dir()` itself.
pub struct RealDesktopAssociationTransport {
    /// The XDG config root (parent of `mimeapps.list` and `autostart/`).
    root: PathBuf,
}

impl RealDesktopAssociationTransport {
    /// Compose over an explicit config root. Creates `autostart/` under it if
    /// absent.
    pub fn new(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(root.join("autostart"))?;
        Ok(Self { root })
    }

    fn mimeapps_path(&self) -> PathBuf {
        self.root.join("mimeapps.list")
    }

    /// Directly write `mime`'s default application, bypassing the
    /// `DesiredStateControl`/`AdmittedMutationContext` lifecycle. Exposed for
    /// direct-transport unit tests (`os-control-test` only, mirroring
    /// `RealTrashTransport::trash_now`'s ctx-free core).
    #[cfg(feature = "os-control-test")]
    pub fn write_default_application_now(
        &self,
        mime: &str,
        app_id: &str,
    ) -> Result<(), OsControlError> {
        self.write_default_application_now_inner(mime, app_id)
    }

    /// Directly write `app_id`'s autostart state, bypassing the mutation
    /// lifecycle. Exposed for direct-transport unit tests.
    #[cfg(feature = "os-control-test")]
    pub fn write_autostart_now(&self, app_id: &str, enabled: bool) -> Result<(), OsControlError> {
        self.write_autostart_now_inner(app_id, enabled)
    }

    /// The autostart `.desktop` path a given `app_id` would resolve to.
    /// Exposed for direct-transport unit tests (path-safety proof).
    #[cfg(feature = "os-control-test")]
    pub fn autostart_path_for_test(&self, app_id: &str) -> Result<PathBuf, OsControlError> {
        self.autostart_path(app_id)
    }

    /// Sanitize an application id into a safe filename component: reject any
    /// path-traversal or separator character rather than attempting to
    /// escape it, so a malicious/malformed `app_id` can never write outside
    /// `autostart/` (path safety for the autostart parser/writer).
    fn safe_autostart_filename(app_id: &str) -> Result<String, OsControlError> {
        if app_id.is_empty()
            || app_id.contains('/')
            || app_id.contains('\\')
            || app_id.contains("..")
            || app_id.contains('\0')
        {
            return Err(OsControlError::InvalidRequest {
                field: SafeField::new("app_id"),
                reason: SafeText::new("app_id must not contain path separators or '..'"),
            });
        }
        Ok(app_id.to_string())
    }

    fn autostart_path(&self, app_id: &str) -> Result<PathBuf, OsControlError> {
        let name = Self::safe_autostart_filename(app_id)?;
        Ok(self.root.join("autostart").join(format!("{name}.desktop")))
    }

    fn unavailable(reason: impl Into<String>) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(ProviderId::new(DESKTOP_ASSOCIATION_PROVIDER_ID)),
            reason: SafeText::new(reason.into()),
            retryable: false,
        }
    }

    /// Read the current default application for `mime` from
    /// `mimeapps.list`'s `[Default Applications]` section, if present.
    fn read_default_application_now(&self, mime: &str) -> Result<Option<String>, OsControlError> {
        let path = self.mimeapps_path();
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Self::unavailable(format!("reading mimeapps.list: {e}"))),
        };
        Ok(parse_mimeapps_default(&contents, mime))
    }

    /// Write `mime`'s default application into `mimeapps.list`'s `[Default
    /// Applications]` section, preserving every other existing association
    /// (never truncating unrelated entries).
    fn write_default_application_now_inner(
        &self,
        mime: &str,
        app_id: &str,
    ) -> Result<(), OsControlError> {
        let path = self.mimeapps_path();
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let updated = set_mimeapps_default(&existing, mime, app_id);
        std::fs::write(&path, updated)
            .map_err(|e| Self::unavailable(format!("writing mimeapps.list: {e}")))
    }

    /// Read whether `app_id`'s autostart entry is enabled: `true` when the
    /// `.desktop` file exists and does not set `Hidden=true`.
    fn read_autostart_now(&self, app_id: &str) -> Result<bool, OsControlError> {
        let path = self.autostart_path(app_id)?;
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(Self::unavailable(format!("reading autostart entry: {e}"))),
        };
        Ok(!contents.lines().any(|l| l.trim() == "Hidden=true"))
    }

    /// Enable/disable `app_id`'s autostart entry. Enabling writes a minimal
    /// valid `.desktop` stub if none exists yet, or clears `Hidden=true` from
    /// an existing one; disabling writes/updates `Hidden=true`.
    fn write_autostart_now_inner(&self, app_id: &str, enabled: bool) -> Result<(), OsControlError> {
        let path = self.autostart_path(app_id)?;
        let existing = std::fs::read_to_string(&path).ok();
        let updated = match existing {
            Some(contents) => set_autostart_hidden(&contents, !enabled),
            None => format!(
                "[Desktop Entry]\nType=Application\nExec={app_id}\nHidden={}\n",
                !enabled
            ),
        };
        std::fs::write(&path, updated)
            .map_err(|e| Self::unavailable(format!("writing autostart entry: {e}")))
    }
}

/// Parse `mimeapps.list`'s `[Default Applications]` section for `mime`'s
/// current default app id, if set. A minimal, dependency-free `.ini`-style
/// parser bounded to the one section this task needs (never a general `.ini`
/// library dependency for a two-key lookup).
fn parse_mimeapps_default(contents: &str, mime: &str) -> Option<String> {
    let mut in_section = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == "[Default Applications]";
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            if key.trim() == mime {
                // The value may be a `;`-separated list; the first entry is
                // the primary default.
                return value.split(';').next().map(|s| s.trim().to_string());
            }
        }
    }
    None
}

/// Set `mime`'s default application in `mimeapps.list`'s `[Default
/// Applications]` section, preserving every other key/section verbatim.
fn set_mimeapps_default(contents: &str, mime: &str, app_id: &str) -> String {
    let mut out = String::new();
    let mut in_section = false;
    let mut found_section = false;
    let mut wrote_key = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_section && !wrote_key {
                out.push_str(&format!("{mime}={app_id}\n"));
                wrote_key = true;
            }
            in_section = trimmed == "[Default Applications]";
            if in_section {
                found_section = true;
            }
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_section {
            if let Some((key, _)) = trimmed.split_once('=') {
                if key.trim() == mime {
                    out.push_str(&format!("{mime}={app_id}\n"));
                    wrote_key = true;
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if in_section && !wrote_key {
        out.push_str(&format!("{mime}={app_id}\n"));
    }
    if !found_section {
        out.push_str("[Default Applications]\n");
        out.push_str(&format!("{mime}={app_id}\n"));
    }
    out
}

/// Set/clear `Hidden=true` in an existing autostart `.desktop` file's
/// contents, preserving every other key verbatim.
fn set_autostart_hidden(contents: &str, hidden: bool) -> String {
    let mut out = String::new();
    let mut wrote = false;
    for line in contents.lines() {
        if line.trim().starts_with("Hidden=") {
            out.push_str(&format!("Hidden={hidden}\n"));
            wrote = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !wrote {
        out.push_str(&format!("Hidden={hidden}\n"));
    }
    out
}

#[async_trait]
impl DesktopAssociationTransport for RealDesktopAssociationTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(DESKTOP_ASSOCIATION_PROVIDER_ID)
    }

    async fn read_default_application(
        &self,
        mime: &str,
    ) -> Result<Option<String>, OsControlError> {
        self.read_default_application_now(mime)
    }

    async fn read_autostart(&self, app_id: &str) -> Result<bool, OsControlError> {
        self.read_autostart_now(app_id)
    }

    async fn set_default_application(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        mime: &str,
        app_id: &str,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.write_default_application_now_inner(mime, app_id)?;
        Ok(ApplyOutcome::Applied(
            crate::os_control::receipt::AppliedDispatch::new(
                Some(Digest::of_str(&format!("{mime}:{app_id}"))),
                crate::os_control::contract::BoundedVec::new(),
            ),
        ))
    }

    async fn set_autostart(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        app_id: &str,
        enabled: bool,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.write_autostart_now_inner(app_id, enabled)?;
        Ok(ApplyOutcome::Applied(
            crate::os_control::receipt::AppliedDispatch::new(
                Some(Digest::of_str(&format!("{app_id}:{enabled}"))),
                crate::os_control::contract::BoundedVec::new(),
            ),
        ))
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn digest_distinguishes_zero_from_nonzero_matches_but_not_exact_count() {
        let none = ApplicationCloseState::new("gedit", 0);
        let one = ApplicationCloseState::new("gedit", 1);
        let three = ApplicationCloseState::new("gedit", 3);
        assert_ne!(none.observation_digest(), one.observation_digest());
        // Distinct nonzero counts still converge to the same "some remain"
        // digest — the desired state is "none remain", not an exact count.
        assert_eq!(one.observation_digest(), three.observation_digest());
    }

    #[test]
    fn digest_binds_the_application_name() {
        let gedit = ApplicationCloseState::new("gedit", 1);
        let firefox = ApplicationCloseState::new("firefox", 1);
        assert_ne!(gedit.observation_digest(), firefox.observation_digest());
    }

    #[test]
    fn desired_state_is_always_zero_matches() {
        let req = ApplicationCloseRequest {
            action: "graceful_close_application".to_string(),
            params: serde_json::json!({ "app_id": "gedit" }),
            name: "gedit".to_string(),
        };
        assert_eq!(req.desired_state().matching_alive, 0);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Default-application / autostart association tests (OSC-013.9) —
    // Task 3.3
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn association_digest_binds_focus_and_target() {
        let a = AssociationState::default_application("text/plain", Some("gedit".to_string()));
        let b = AssociationState::default_application("text/plain", Some("gedit".to_string()));
        assert_eq!(a.observation_digest(), b.observation_digest());

        let different_app =
            AssociationState::default_application("text/plain", Some("kate".to_string()));
        assert_ne!(a.observation_digest(), different_app.observation_digest());

        let different_mime =
            AssociationState::default_application("text/html", Some("gedit".to_string()));
        assert_ne!(a.observation_digest(), different_mime.observation_digest());

        // A default-application observation never collides with an
        // autostart observation, even with a coincidentally equal target
        // string.
        let autostart = AssociationState::autostart("text/plain", true);
        assert_ne!(a.observation_digest(), autostart.observation_digest());
    }

    #[test]
    fn autostart_digest_binds_app_id_and_enabled() {
        let enabled = AssociationState::autostart("gedit", true);
        let disabled = AssociationState::autostart("gedit", false);
        assert_ne!(enabled.observation_digest(), disabled.observation_digest());

        let other_app = AssociationState::autostart("kate", true);
        assert_ne!(enabled.observation_digest(), other_app.observation_digest());
    }

    #[tokio::test]
    async fn real_desktop_association_transport_round_trips_default_application() {
        let dir = tempfile::tempdir().unwrap();
        let transport = RealDesktopAssociationTransport::new(dir.path()).unwrap();

        assert_eq!(
            transport.read_default_application("text/plain").await.unwrap(),
            None
        );

        transport
            .write_default_application_now("text/plain", "gedit")
            .expect("write succeeds");
        assert_eq!(
            transport.read_default_application("text/plain").await.unwrap(),
            Some("gedit".to_string())
        );

        // Setting a second MIME type preserves the first (never truncates
        // unrelated entries).
        transport
            .write_default_application_now("text/html", "firefox")
            .unwrap();
        assert_eq!(
            transport.read_default_application("text/plain").await.unwrap(),
            Some("gedit".to_string())
        );
        assert_eq!(
            transport.read_default_application("text/html").await.unwrap(),
            Some("firefox".to_string())
        );
    }

    #[tokio::test]
    async fn real_desktop_association_transport_round_trips_autostart() {
        let dir = tempfile::tempdir().unwrap();
        let transport = RealDesktopAssociationTransport::new(dir.path()).unwrap();

        // No entry yet → disabled.
        assert!(!transport.read_autostart("myapp").await.unwrap());

        transport.write_autostart_now("myapp", true).unwrap();
        assert!(transport.read_autostart("myapp").await.unwrap());

        transport.write_autostart_now("myapp", false).unwrap();
        assert!(!transport.read_autostart("myapp").await.unwrap());
    }

    #[test]
    fn autostart_filename_rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let transport = RealDesktopAssociationTransport::new(dir.path()).unwrap();

        for malicious in ["../evil", "a/b", "a\\b", "a\0b", ""] {
            let err = transport.autostart_path_for_test(malicious);
            assert!(
                err.is_err(),
                "expected path-traversal rejection for {malicious:?}"
            );
        }
    }

    #[test]
    fn mimeapps_parser_ignores_other_sections() {
        let contents = "[Added Associations]\ntext/plain=other.desktop\n\n[Default Applications]\ntext/plain=gedit.desktop\n";
        assert_eq!(
            parse_mimeapps_default(contents, "text/plain"),
            Some("gedit.desktop".to_string())
        );
        assert_eq!(parse_mimeapps_default(contents, "text/html"), None);
    }
}
