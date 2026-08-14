//! Display domain: the `DisplayControl` desired-state provider (design §3, §9.6).
//!
//! linux-os-control-production **Task 2.2** — "Migrate brightness and prepare
//! display provider seam" (OSC-019, OSC-031, OSC-032).
//!
//! This module replaces the direct `gdbus`/`brightnessctl`/`xrandr` subprocess
//! handling that used to live in `tools/system_config.rs`. It composes the F1
//! runtime, mirroring `os_control::audio`'s shape:
//!
//! * [`DisplayState`] is a normalized observation ([`NormalizedObservation`])
//!   whose numeric brightness percentage drives
//!   [`ComparatorKind::WithinTolerance`] idempotency + verification.
//! * [`DisplayControl`] implements the generic [`DesiredStateControl`]
//!   lifecycle (observe → apply → verify → rollback) for brightness. Its
//!   `apply`/`rollback` build a governed [`StructuredCommandRequest`] from the
//!   borrowed [`AdmittedMutationContext`] — the only sanctioned path to a child
//!   process — so no display code touches `ExecWrapper`/`tokio::process`
//!   directly.
//! * The live transport ([`crate::os_control::linux::providers::gnome_display`]
//!   and friends) is a raw, deny-live-gated adapter; deny-live tests inject
//!   [`FakeDisplayTransport`].
//!
//! # Physical backlight vs. software gamma (OSC-019.2)
//!
//! [`BrightnessBackend`] distinguishes the **physical backlight** providers
//! (GNOME `SettingsDaemon.Power.Screen` D-Bus property, and the
//! `brightnessctl` hardware fallback) from the **XRandR gamma** fallback, which
//! only *simulates* brightness by scaling the output's gamma ramp. Every
//! brightness result names its backend so a caller is never told a gamma trick
//! is physical backlight control.
//!
//! # No XRandR on Wayland (OSC-019.3, OSC-032.3)
//!
//! [`BrightnessBackend::is_eligible_for`] enforces that XRandR is **X11-only**;
//! [`select_backend`] never returns it for a [`DisplayServer::Wayland`] session,
//! and [`DisplayControl::apply_argv`]/observation paths never build XRandR argv
//! outside that guard. This is asserted directly in the deny-live lifecycle test
//! (`tests/os_control_display_lifecycle.rs`).

/// Monitor configuration (apply-then-confirm) and night light.
pub mod configuration;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

use async_trait::async_trait;

use crate::os_control::capability::DisplayServer;
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    CapabilityId, ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId,
    SafeErrorCode, Tolerance, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{
    CommandPlan, CommandPolicy, StructuredCommandRequest,
};
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification,
    VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;

#[cfg(feature = "os-control-test")]
pub mod fake;
pub mod parsers;
pub mod selection;


pub use selection::BrightnessBackend;

/// The compile-time maximum percentage tolerance for display verification
/// (matches the frozen manifest's `AbsolutePercentagePoints.compileTimeMaximum`).
pub const DISPLAY_TOLERANCE_MAX: f64 = 5.0;

/// The default absolute percentage tolerance used for idempotency + verification.
pub const DISPLAY_TOLERANCE_DEFAULT: f64 = 2.0;

/// A normalized brightness observation (design §5, §9.6). The digest binds the
/// brightness percentage while `numeric_value` exposes it for
/// `WithinTolerance` comparison.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayState {
    /// Physical/software brightness, 0..=100.
    pub brightness_percent: u8,
    /// The backend that produced this observation (physical or gamma).
    pub backend: BrightnessBackend,
}

impl DisplayState {
    /// Construct a brightness observation.
    #[must_use]
    pub fn new(brightness_percent: u8, backend: BrightnessBackend) -> Self {
        Self {
            brightness_percent: brightness_percent.min(100),
            backend,
        }
    }
}

impl NormalizedObservation for DisplayState {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!("display:brightness:{}", self.brightness_percent))
    }

    fn numeric_value(&self) -> Option<f64> {
        Some(self.brightness_percent as f64)
    }
}

/// The concrete display operation this task migrates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayOp {
    /// Read-only state query (`get_display_state`).
    GetState,
    /// Set physical/software brightness to a percentage.
    SetBrightness(u8),
}

/// A fully-described display request. Carries the canonical `action`/`params`
/// so the governed [`StructuredCommandRequest`] can bind them against the grant.
#[derive(Debug, Clone)]
pub struct DisplayRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params digest).
    pub params: serde_json::Value,
    /// The concrete operation.
    pub op: DisplayOp,
}

impl DisplayRequest {
    /// The desired end state for a mutation. Returns `None` for the read-only
    /// [`DisplayOp::GetState`]; the backend field of the returned state is a
    /// placeholder (`Unknown`-shaped by whichever backend actually applies) and
    /// is not compared — only the numeric percentage drives verification.
    #[must_use]
    pub fn desired_state(&self, backend: BrightnessBackend) -> Option<DisplayState> {
        match self.op {
            DisplayOp::GetState => None,
            DisplayOp::SetBrightness(v) => Some(DisplayState::new(v, backend)),
        }
    }

    /// The idempotency/verification comparator for this operation.
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        match self.op {
            DisplayOp::SetBrightness(_) => ComparatorKind::WithinTolerance,
            DisplayOp::GetState => ComparatorKind::Exact,
        }
    }

    /// The numeric tolerance for this operation.
    #[must_use]
    pub fn tolerance(&self) -> Option<Tolerance> {
        match self.op {
            DisplayOp::SetBrightness(_) => Some(Tolerance {
                abs: DISPLAY_TOLERANCE_DEFAULT,
            }),
            DisplayOp::GetState => None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transport seam
// ─────────────────────────────────────────────────────────────────────────────

/// The raw display transport seam. Live implementations
/// ([`crate::os_control::linux::providers::gnome_display`] and the
/// `brightnessctl`/`xrandr_display` fallbacks) are deny-live-gated adapters;
/// deny-live tests inject [`FakeDisplayTransport`]. Reads run a query and parse
/// it; `dispatch` runs a governed [`StructuredCommandRequest`].
#[async_trait]
pub trait DisplayTransport: Send + Sync {
    /// The stable provider identity (never model prose).
    fn provider_id(&self) -> ProviderId;

    /// The selected brightness backend.
    fn selected_backend(&self) -> BrightnessBackend;

    /// Read the current brightness percentage. A parse ambiguity must surface
    /// as an error, never a fabricated state.
    async fn read_brightness(&self, ctx: &HostExecutionContext) -> Result<u8, OsControlError>;

    /// Dispatch a governed structured command (the only path to a process).
    async fn dispatch(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The rollback snapshot captured before an apply, so a contradiction can be
/// compensated back to the exact prior brightness.
#[derive(Debug, Clone)]
struct RollbackSnapshot {
    before_percent: u8,
    action: String,
    params: serde_json::Value,
}

/// The `DisplayControl` desired-state provider (design §3, §4, §9.6). Generic
/// over the [`DisplayTransport`] so the same governed logic runs over the live
/// GNOME/hardware/XRandR adapters and the deny-live fake.
///
/// Every constructor and mutation takes the session's confirmed
/// [`DisplayServer`] so the no-XRandR-on-Wayland invariant (OSC-019.3,
/// OSC-032.3) is enforced by construction: [`selection::select_backend`] is
/// consulted with the confirmed display server, never with a fabricated or
/// unchecked env hint (OSC-032.7).
pub struct DisplayControl<T: DisplayTransport> {
    transport: T,
    policy: CommandPolicy,
    tolerance: f64,
    /// Prior-brightness snapshots keyed by session id, captured in `apply` for
    /// `rollback`. Interior mutability because the provider is shared (`&self`);
    /// display ops are serialized by the display resource lease.
    snapshots: Mutex<HashMap<String, RollbackSnapshot>>,
}

impl<T: DisplayTransport> DisplayControl<T> {
    /// Compose a `DisplayControl` over a transport with the default tolerance.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self::with_tolerance(transport, DISPLAY_TOLERANCE_DEFAULT)
    }

    /// Compose with an explicit tolerance (clamped to [`DISPLAY_TOLERANCE_MAX`]).
    #[must_use]
    pub fn with_tolerance(transport: T, tolerance: f64) -> Self {
        Self {
            transport,
            policy: CommandPolicy::new(),
            tolerance: tolerance.clamp(0.0, DISPLAY_TOLERANCE_MAX),
            snapshots: Mutex::new(HashMap::new()),
        }
    }

    /// The selected backend (for the `backend` result field).
    #[must_use]
    pub fn backend(&self) -> BrightnessBackend {
        self.transport.selected_backend()
    }

    /// Borrow the underlying transport (used by tests to inspect captured argv).
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The provider identity.
    #[must_use]
    pub fn provider_id(&self) -> ProviderId {
        self.transport.provider_id()
    }

    /// The configured tolerance.
    #[must_use]
    pub fn tolerance(&self) -> f64 {
        self.tolerance
    }

    /// The evidence source for verification observations from this backend. The
    /// GNOME D-Bus property read is authoritative service state; the
    /// `brightnessctl`/`xrandr` fallbacks are structured-command queries.
    fn evidence_source(&self) -> OsEvidenceSource {
        match self.transport.selected_backend() {
            BrightnessBackend::GnomeSettingsDaemon => OsEvidenceSource::AuthoritativeServiceState,
            // The kernel's own sysfs value IS the authoritative state: it is what
            // the driver reports, not a tool's rendering of it.
            BrightnessBackend::LogindSession => OsEvidenceSource::AuthoritativeServiceState,
            BrightnessBackend::Brightnessctl | BrightnessBackend::XrandrGamma => {
                OsEvidenceSource::StructuredCommandQuery
            }
        }
    }

    /// Build the governed structured command for a mutating operation.
    fn build_command(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        action: &str,
        params: &serde_json::Value,
        args: Vec<String>,
    ) -> Result<StructuredCommandRequest, OsControlError> {
        let backend = self.transport.selected_backend();
        let executable = backend.trusted_executable()?;
        let plan = CommandPlan::new(
            CapabilityId::new(action),
            action.to_string(),
            params.clone(),
            executable,
            args,
        );
        StructuredCommandRequest::from_admitted(ctx, plan, &self.policy)
    }

    /// The argv for setting brightness to `percent` on the selected backend.
    fn apply_argv(&self, percent: u8) -> Vec<String> {
        let backend = self.transport.selected_backend();
        if backend == BrightnessBackend::LogindSession {
            // logind takes the value in the DEVICE's own units, so the device and
            // its maximum must be resolved at dispatch time. A stale maximum
            // captured at composition would mis-scale after a docking change.
            //
            // With no device, an empty argv is returned rather than a call naming
            // no device: logind would reject it, and a request that reaches the bus
            // at all is harder to reason about than one that never forms.
            return match selection::discover_backlight_device() {
                Some((device, max)) => {
                    selection::logind_set_brightness_argv(&device, max, percent)
                }
                None => Vec::new(),
            };
        }
        selection::set_brightness_argv(backend, percent)
    }

    fn satisfying(&self, observed: &DisplayState) -> SatisfyingVerification<DisplayState> {
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
impl<T: DisplayTransport> DesiredStateControl<DisplayRequest, DisplayState> for DisplayControl<T> {
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        _request: &DisplayRequest,
    ) -> Result<DisplayState, OsControlError> {
        let percent = self.transport.read_brightness(ctx).await?;
        Ok(DisplayState::new(percent, self.transport.selected_backend()))
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &DisplayRequest,
        _desired: &DisplayState,
    ) -> Result<ApplyOutcome, OsControlError> {
        // Capture the pre-apply brightness so a contradiction can be rolled
        // back to the exact prior value.
        if let Ok(before_percent) = self.transport.read_brightness(ctx.observation()).await {
            let session = ctx.grant().session_id().to_string();
            self.snapshots.lock().expect("display snapshots poisoned").insert(
                session,
                RollbackSnapshot {
                    before_percent,
                    action: request.action.clone(),
                    params: request.params.clone(),
                },
            );
        }

        let DisplayOp::SetBrightness(percent) = request.op else {
            return Err(OsControlError::InvalidRequest {
                field: crate::os_control::contract::SafeField::new("op"),
                reason: crate::os_control::contract::SafeText::new(
                    "get_display_state has no mutation to apply",
                ),
            });
        };
        let args = self.apply_argv(percent);
        let command = self.build_command(ctx, &request.action, &request.params, args)?;
        self.transport.dispatch(ctx, &command).await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        _request: &DisplayRequest,
        desired: &DisplayState,
    ) -> Result<VerificationReport<DisplayState>, OsControlError> {
        let percent = self.transport.read_brightness(ctx).await?;
        let observed = DisplayState::new(percent, self.transport.selected_backend());

        let satisfied =
            (observed.brightness_percent as f64 - desired.brightness_percent as f64).abs()
                <= self.tolerance;

        if satisfied {
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
            .expect("display snapshots poisoned")
            .get(token.session_id().as_str())
            .cloned();

        let Some(snapshot) = snapshot else {
            // No captured prior state → the effect is unobservable for compensation.
            return Ok(ApplyOutcome::Uncertain(
                crate::os_control::receipt::UncertainDispatch::new(
                    None,
                    crate::os_control::receipt::UncertainEffectCause::Unobservable,
                    crate::os_control::contract::BoundedVec::new(),
                ),
            ));
        };

        let args = self.apply_argv(snapshot.before_percent);
        let command = self.build_command(ctx, &snapshot.action, &snapshot.params, args)?;
        self.transport.dispatch(ctx, &command).await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Receipt → tool-result mapping (existing `set_brightness` fields stay compatible)
// ─────────────────────────────────────────────────────────────────────────────

use crate::os_control::receipt::{ActionLifecycle, MutationReceipt};

/// Map a governed [`MutationReceipt`] to the **existing** `set_brightness`
/// result fields (`brightness`, `backend`, `changed`, `already_in_desired_state`),
/// plus additive `lifecycle`/`verified` fields. Preserving these keeps the
/// migrated tool wire-compatible with the pre-migration handler (design §9.6,
/// Task 2.2).
#[must_use]
pub fn set_brightness_result(
    receipt: &MutationReceipt<DisplayState>,
    requested_percent: u8,
    backend: BrightnessBackend,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "brightness": requested_percent,
        "backend": backend.as_str(),
        "changed": receipt.changed(),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
        "degraded": backend.is_degraded(),
    })
}

/// Map a read-only [`DisplayState`] to the `get_display_state` result fields.
#[must_use]
pub fn display_state_result(state: &DisplayState) -> serde_json::Value {
    serde_json::json!({
        "brightness": state.brightness_percent,
        "backend": state.backend.as_str(),
        "degraded": state.backend.is_degraded(),
        "physical": state.backend.is_physical_backlight(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// `HostOsControl::display()` port seam (design §4)
// ─────────────────────────────────────────────────────────────────────────────

/// The dyn-compatible display domain port design §4 names
/// `fn display(&self) -> &dyn DisplayControl` on `HostOsControl`. Because the
/// concrete [`DisplayControl`] provider struct above is generic over its
/// [`DisplayTransport`], `HostOsControl::display()` returns this object-safe
/// supertrait instead so any transport (live GNOME/hardware/XRandR, or a
/// deny-live fake) can be composed behind one erased reference. Every
/// [`DisplayControl<T>`] implements it automatically via the blanket impl below.
pub trait DisplayControlPort: DesiredStateControl<DisplayRequest, DisplayState> {
    /// The composed backend label.
    ///
    /// Exposed on the port (mirroring [`crate::os_control::power::session::PowerSessionControlPort`])
    /// because `DisplayRequest::desired_state` needs the backend, and a handler
    /// only ever holds an erased `&dyn DisplayControlPort`.
    fn backend(&self) -> BrightnessBackend;
}

impl<T: DisplayTransport> DisplayControlPort for DisplayControl<T> {
    fn backend(&self) -> BrightnessBackend {
        self.backend()
    }
}

/// Select the eligible brightness backend for a confirmed [`DisplayServer`]
/// from the set of backends the session actually has available (OSC-019.3,
/// OSC-032.3, OSC-032.7 — never fabricate env vars to force provider access).
/// Returns `None` when no eligible backend is available in this session.
#[must_use]
pub fn select_brightness_backend(
    display_server: DisplayServer,
    available: &[BrightnessBackend],
) -> Option<BrightnessBackend> {
    selection::select_backend(display_server, available)
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn brightness_observation_uses_numeric_within_tolerance() {
        let desired = DisplayState::new(60, BrightnessBackend::Brightnessctl);
        assert_eq!(desired.numeric_value(), Some(60.0));
    }

    #[test]
    fn digest_ignores_backend_only_percentage_matters() {
        let a = DisplayState::new(60, BrightnessBackend::Brightnessctl);
        let b = DisplayState::new(60, BrightnessBackend::GnomeSettingsDaemon);
        assert_eq!(a.observation_digest(), b.observation_digest());
        let c = DisplayState::new(61, BrightnessBackend::Brightnessctl);
        assert_ne!(a.observation_digest(), c.observation_digest());
    }

    #[test]
    fn xrandr_never_selected_on_wayland() {
        use BrightnessBackend::*;
        let available = [GnomeSettingsDaemon, Brightnessctl, XrandrGamma];
        assert_eq!(
            select_brightness_backend(DisplayServer::Wayland, &available),
            Some(GnomeSettingsDaemon)
        );

        // Even when it is the ONLY thing "available", Wayland must not select it.
        assert_eq!(
            select_brightness_backend(DisplayServer::Wayland, &[XrandrGamma]),
            None
        );

        // X11 may select it as a last resort.
        assert_eq!(
            select_brightness_backend(DisplayServer::X11, &[XrandrGamma]),
            Some(XrandrGamma)
        );
    }

    #[test]
    fn desired_state_none_for_read_only_get_state() {
        let request = DisplayRequest {
            action: "get_display_state".to_string(),
            params: serde_json::json!({}),
            op: DisplayOp::GetState,
        };
        assert!(request
            .desired_state(BrightnessBackend::Brightnessctl)
            .is_none());
    }
}
