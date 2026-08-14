//! Bluetooth adapter and device lifecycle (BlueZ).
//!
//! linux-os-control-production **Task 3.7** (OSC-021, OSC-029), design §9.
//!
//! Provides the eight frozen §10 operations: `get_bluetooth_state`,
//! `set_bluetooth_enabled`, `scan_bluetooth`, `pair_bluetooth_device`,
//! `connect_bluetooth_device`, `disconnect_bluetooth_device`,
//! `set_bluetooth_trust` and `remove_bluetooth_device`.
//!
//! # One observation type, two schemas
//!
//! The manifest declares two observation schemas — `BluetoothState` for the
//! adapter and `BluetoothDeviceState` for a device — but the governed
//! [`DesiredStateControl`] lifecycle is generic over a *single* observation
//! type. [`BluetoothObservation`] therefore carries a [`BluetoothFocus`]
//! discriminator, and its digest binds the focus **plus only the field that
//! focus concerns**. That matters for correctness: without the focus in the
//! digest, an adapter observation could compare equal to a device observation
//! and a mutation could verify against the wrong fact.
//!
//! # Privacy (OSC-029)
//!
//! Nearby-device discovery is inherently sensitive: a scan enumerates hardware
//! around the user. `get_bluetooth_state` and `scan_bluetooth` are therefore
//! **RED reads** in the frozen manifest, which routes them through
//! privacy-sensitive admission — they fail closed when the audit ledger is
//! unhealthy. Device *names* are advertised prose, so they are carried as
//! redacted labels and never used as an identity.
//!
//! # Pairing approval (OSC-021)
//!
//! `pair_bluetooth_device`, `set_bluetooth_trust` and `remove_bluetooth_device`
//! are fixed RED, so the existing policy gate already demands a durable human
//! decision before a grant is minted. Pairing consequently rides the existing
//! approval events with **no new frontend authority**, and a passkey never
//! enters argv, canonical params, or audit — so nothing to persist.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    CapabilityId, ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId,
    SafeErrorCode, SafeText, VerificationReliability,
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

pub mod selection;

/// Deny-live fake transport (Task 0.4 / OSC-033); test composition only.
#[cfg(feature = "os-control-test")]
pub mod fake;

pub use selection::BluetoothBackend;

/// Maximum length (chars) of a bounded Bluetooth token.
pub const BLUETOOTH_TOKEN_MAX_CHARS: usize = 64;
/// Maximum devices a single scan may report (bounded discovery, OSC-034).
pub const BLUETOOTH_SCAN_MAX_DEVICES: usize = 128;
/// Maximum scan duration accepted, in milliseconds. A scan is bounded so it can
/// never become an indefinite radio sweep.
pub const BLUETOOTH_SCAN_MAX_MS: u64 = 30_000;

fn sanitize_token(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(BLUETOOTH_TOKEN_MAX_CHARS));
    for ch in raw.chars() {
        if out.chars().count() >= BLUETOOTH_TOKEN_MAX_CHARS {
            break;
        }
        if !ch.is_control() {
            out.push(ch);
        }
    }
    out.trim().to_string()
}

/// A Bluetooth device address — the stable device identity.
///
/// The advertised *name* is deliberately not an identity: it is neither unique
/// nor stable, so binding operations to it would let a renamed or spoofed device
/// inherit another device's authorization.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BluetoothDeviceId(String);

/// A Bluetooth adapter address — the stable adapter identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BluetoothAdapterId(String);

macro_rules! bluetooth_id {
    ($name:ident) => {
        impl $name {
            /// Construct a bounded, control-char-free address token.
            #[must_use]
            pub fn new(raw: impl Into<String>) -> Self {
                Self(sanitize_token(&raw.into()))
            }

            /// Borrow the address token.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// A correlation-safe digest of the address.
            #[must_use]
            pub fn digest(&self) -> Digest {
                Digest::of_str(&self.0)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }
    };
}

bluetooth_id!(BluetoothDeviceId);
bluetooth_id!(BluetoothAdapterId);

/// Which fact an observation is about.
///
/// Part of the observation digest, so an adapter fact can never satisfy a device
/// postcondition (or vice versa).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BluetoothFocus {
    /// The adapter's powered state (`set_bluetooth_enabled`).
    AdapterPower,
    /// A device's paired state (`pair_bluetooth_device`).
    DevicePaired,
    /// A device's connected state (connect / disconnect).
    DeviceConnected,
    /// A device's trusted flag (`set_bluetooth_trust`).
    DeviceTrusted,
    /// Whether a device is still known to the adapter (`remove_bluetooth_device`).
    DeviceKnown,
}

impl BluetoothFocus {
    /// The stable snake_case token used in the digest.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            BluetoothFocus::AdapterPower => "adapter_power",
            BluetoothFocus::DevicePaired => "device_paired",
            BluetoothFocus::DeviceConnected => "device_connected",
            BluetoothFocus::DeviceTrusted => "device_trusted",
            BluetoothFocus::DeviceKnown => "device_known",
        }
    }
}

/// The adapter's observable state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BluetoothAdapterState {
    /// The adapter identity.
    pub adapter: BluetoothAdapterId,
    /// Whether the adapter radio is powered.
    pub powered: bool,
    /// Whether the adapter is currently discovering.
    pub discovering: bool,
}

/// A device's observable state. Content-free: a redacted display label, never
/// service data or advertised payloads.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BluetoothDeviceState {
    /// The device identity (address).
    pub device: BluetoothDeviceId,
    /// A redacted human-safe label. Not an identity.
    pub label: SafeText,
    /// Whether the device is paired with this host.
    pub paired: bool,
    /// Whether the device is currently connected.
    pub connected: bool,
    /// Whether the device carries the trust flag.
    pub trusted: bool,
    /// Battery percentage when the device reports it (OSC-021 battery metadata).
    pub battery_percent: Option<u8>,
}

/// A normalized Bluetooth observation for the governed lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BluetoothObservation {
    /// Which fact this observation is about.
    pub focus: BluetoothFocus,
    /// The device the observation concerns, for device-focused facts.
    pub device: Option<BluetoothDeviceId>,
    /// The boolean value of the focused fact.
    pub value: bool,
}

impl BluetoothObservation {
    /// An adapter-power observation.
    #[must_use]
    pub fn adapter_power(powered: bool) -> Self {
        Self {
            focus: BluetoothFocus::AdapterPower,
            device: None,
            value: powered,
        }
    }

    /// A device-focused observation.
    #[must_use]
    pub fn device(focus: BluetoothFocus, device: BluetoothDeviceId, value: bool) -> Self {
        Self {
            focus,
            device: Some(device),
            value,
        }
    }
}

impl NormalizedObservation for BluetoothObservation {
    fn observation_digest(&self) -> Digest {
        // The focus and the device are part of the digest so a device fact can
        // never satisfy an adapter postcondition, and one device's state can
        // never satisfy another's.
        Digest::of_str(&format!(
            "bluetooth:{}:{}:{}",
            self.focus.as_str(),
            self.device.as_ref().map_or("-", |d| d.as_str()),
            self.value
        ))
    }
}

/// One discovered device from a bounded scan.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DiscoveredDevice {
    /// The device address (stable identity).
    pub device: BluetoothDeviceId,
    /// A redacted human-safe label.
    pub label: SafeText,
    /// Signal strength in dBm, when the adapter reports it.
    pub rssi: Option<i16>,
    /// Whether this device is already paired.
    pub paired: bool,
}

/// A bounded scan result.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BluetoothScan {
    /// The discovered devices, bounded to [`BLUETOOTH_SCAN_MAX_DEVICES`].
    pub devices: Vec<DiscoveredDevice>,
    /// Whether the scan hit the device cap and was truncated.
    pub truncated: bool,
}

/// The full state read returned by `get_bluetooth_state`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BluetoothState {
    /// The selected adapter, when one exists.
    pub adapter: Option<BluetoothAdapterState>,
    /// Known (paired or previously seen) devices, bounded.
    pub devices: Vec<BluetoothDeviceState>,
}

/// The concrete Bluetooth operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BluetoothOp {
    /// Power the adapter on or off (`set_bluetooth_enabled`).
    SetEnabled(bool),
    /// Pair a device (`pair_bluetooth_device`). RED: a new association.
    Pair(BluetoothDeviceId),
    /// Connect a device (`connect_bluetooth_device`).
    Connect(BluetoothDeviceId),
    /// Disconnect a device (`disconnect_bluetooth_device`).
    Disconnect(BluetoothDeviceId),
    /// Set or clear a device's trust flag (`set_bluetooth_trust`).
    SetTrust {
        /// The device to change.
        device: BluetoothDeviceId,
        /// The desired trust state.
        trusted: bool,
    },
    /// Remove (unpair and forget) a device (`remove_bluetooth_device`).
    Remove(BluetoothDeviceId),
}

impl BluetoothOp {
    /// The canonical tool name this operation maps to.
    #[must_use]
    pub fn action_name(&self) -> &'static str {
        match self {
            BluetoothOp::SetEnabled(_) => "set_bluetooth_enabled",
            BluetoothOp::Pair(_) => "pair_bluetooth_device",
            BluetoothOp::Connect(_) => "connect_bluetooth_device",
            BluetoothOp::Disconnect(_) => "disconnect_bluetooth_device",
            BluetoothOp::SetTrust { .. } => "set_bluetooth_trust",
            BluetoothOp::Remove(_) => "remove_bluetooth_device",
        }
    }

    /// The device this operation targets, if any.
    #[must_use]
    pub fn device(&self) -> Option<&BluetoothDeviceId> {
        match self {
            BluetoothOp::SetEnabled(_) => None,
            BluetoothOp::Pair(d)
            | BluetoothOp::Connect(d)
            | BluetoothOp::Disconnect(d)
            | BluetoothOp::Remove(d) => Some(d),
            BluetoothOp::SetTrust { device, .. } => Some(device),
        }
    }
}

/// A fully-described Bluetooth request. Carries the canonical `action`/`params`
/// so the governed [`StructuredCommandRequest`] binds them against the grant.
#[derive(Debug, Clone)]
pub struct BluetoothRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params digest).
    /// Never a passkey (OSC-029).
    pub params: serde_json::Value,
    /// The concrete operation.
    pub op: BluetoothOp,
}

impl BluetoothRequest {
    /// The fact this operation's postcondition is about.
    #[must_use]
    pub fn focus(&self) -> BluetoothFocus {
        match self.op {
            BluetoothOp::SetEnabled(_) => BluetoothFocus::AdapterPower,
            BluetoothOp::Pair(_) => BluetoothFocus::DevicePaired,
            BluetoothOp::Connect(_) | BluetoothOp::Disconnect(_) => BluetoothFocus::DeviceConnected,
            BluetoothOp::SetTrust { .. } => BluetoothFocus::DeviceTrusted,
            BluetoothOp::Remove(_) => BluetoothFocus::DeviceKnown,
        }
    }

    /// The desired end state for this mutation.
    #[must_use]
    pub fn desired_state(&self) -> BluetoothObservation {
        match &self.op {
            BluetoothOp::SetEnabled(enabled) => BluetoothObservation::adapter_power(*enabled),
            BluetoothOp::Pair(device) => {
                BluetoothObservation::device(BluetoothFocus::DevicePaired, device.clone(), true)
            }
            BluetoothOp::Connect(device) => {
                BluetoothObservation::device(BluetoothFocus::DeviceConnected, device.clone(), true)
            }
            BluetoothOp::Disconnect(device) => {
                BluetoothObservation::device(BluetoothFocus::DeviceConnected, device.clone(), false)
            }
            BluetoothOp::SetTrust { device, trusted } => {
                BluetoothObservation::device(BluetoothFocus::DeviceTrusted, device.clone(), *trusted)
            }
            // Removal succeeds when the device is no longer known to the adapter.
            BluetoothOp::Remove(device) => {
                BluetoothObservation::device(BluetoothFocus::DeviceKnown, device.clone(), false)
            }
        }
    }

    /// The idempotency/verification comparator. Every Bluetooth postcondition is
    /// a boolean fact, so the manifest's `FreshAuthoritativeObservation` compares
    /// exactly — there is no tolerance.
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transport seam
// ─────────────────────────────────────────────────────────────────────────────

/// The raw Bluetooth transport seam.
///
/// The live implementation is a deny-live-gated adapter over BlueZ
/// (`org.bluez`, system bus) with a `bluetoothctl` structured-command fallback;
/// deny-live tests inject [`fake::FakeBluetoothTransport`].
#[async_trait]
pub trait BluetoothTransport: Send + Sync {
    /// The stable provider identity (never model prose).
    fn provider_id(&self) -> ProviderId;

    /// The selected backend.
    fn selected_backend(&self) -> BluetoothBackend;

    /// Read the adapter state. `Ok(None)` is the distinct, unambiguous "no
    /// Bluetooth adapter present" result — never a fabricated powered-off state.
    async fn read_adapter(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Option<BluetoothAdapterState>, OsControlError>;

    /// Read the known devices (paired or previously seen), bounded.
    async fn read_devices(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<BluetoothDeviceState>, OsControlError>;

    /// Run a bounded discovery scan.
    async fn scan(
        &self,
        ctx: &HostExecutionContext,
        duration_ms: u64,
    ) -> Result<BluetoothScan, OsControlError>;

    /// Dispatch a governed structured command (the only path to a process).
    async fn dispatch(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The rollback snapshot captured before an apply.
#[derive(Debug, Clone)]
struct RollbackSnapshot {
    before: BluetoothObservation,
    action: String,
    params: serde_json::Value,
}

/// The `BluetoothControl` provider (design §3, §4, §9). Generic over the
/// [`BluetoothTransport`] so the same governed logic runs over live BlueZ and
/// the deny-live fake.
pub struct BluetoothControl<T: BluetoothTransport> {
    transport: T,
    policy: CommandPolicy,
    /// Prior-state snapshots keyed by session id, captured in `apply` for
    /// `rollback`. Interior mutability because the provider is shared (`&self`);
    /// Bluetooth ops are serialized by the adapter/device resource leases.
    snapshots: Mutex<HashMap<String, RollbackSnapshot>>,
}

impl<T: BluetoothTransport> BluetoothControl<T> {
    /// Compose a `BluetoothControl` over a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            policy: CommandPolicy::new(),
            snapshots: Mutex::new(HashMap::new()),
        }
    }

    /// The selected backend (for the `backend` result field).
    #[must_use]
    pub fn backend(&self) -> BluetoothBackend {
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

    fn evidence_source(&self) -> OsEvidenceSource {
        match self.transport.selected_backend() {
            BluetoothBackend::BluezDbus => OsEvidenceSource::AuthoritativeServiceState,
            BluetoothBackend::Bluetoothctl => OsEvidenceSource::StructuredCommandQuery,
        }
    }

    /// Observe the single fact a request's postcondition concerns.
    ///
    /// Reading only the focused fact keeps verification honest: a connect is
    /// verified by the device's connected flag, not by "something changed".
    async fn observe_focus(
        &self,
        ctx: &HostExecutionContext,
        request: &BluetoothRequest,
    ) -> Result<BluetoothObservation, OsControlError> {
        let focus = request.focus();
        if focus == BluetoothFocus::AdapterPower {
            let powered = self
                .transport
                .read_adapter(ctx)
                .await?
                .map(|adapter| adapter.powered)
                // No adapter at all: fail closed rather than reporting "off",
                // which would let an enable request verify as satisfied.
                .ok_or_else(|| OsControlError::Unavailable {
                    provider: Some(self.transport.provider_id()),
                    reason: SafeText::new("no Bluetooth adapter is present on this host"),
                    retryable: false,
                })?;
            return Ok(BluetoothObservation::adapter_power(powered));
        }

        let Some(target) = request.op.device().cloned() else {
            return Err(OsControlError::InvalidRequest {
                field: crate::os_control::contract::SafeField::new("device"),
                reason: SafeText::new("this Bluetooth operation requires a device address"),
            });
        };
        let devices = self.transport.read_devices(ctx).await?;
        let found = devices.iter().find(|d| d.device == target);

        let value = match focus {
            // A device the adapter no longer knows is, by definition, unpaired,
            // disconnected and untrusted — and absent.
            BluetoothFocus::DeviceKnown => found.is_some(),
            BluetoothFocus::DevicePaired => found.is_some_and(|d| d.paired),
            BluetoothFocus::DeviceConnected => found.is_some_and(|d| d.connected),
            BluetoothFocus::DeviceTrusted => found.is_some_and(|d| d.trusted),
            BluetoothFocus::AdapterPower => unreachable!("handled above"),
        };
        Ok(BluetoothObservation::device(focus, target, value))
    }

    /// Build the governed structured command for a mutating operation.
    fn build_command(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        action: &str,
        params: &serde_json::Value,
        args: Vec<String>,
    ) -> Result<StructuredCommandRequest, OsControlError> {
        let executable = self.transport.selected_backend().trusted_executable()?;
        let plan = CommandPlan::new(
            CapabilityId::new(action),
            action.to_string(),
            params.clone(),
            executable,
            args,
        );
        StructuredCommandRequest::from_admitted(ctx, plan, &self.policy)
    }

    /// Reject a device identity that is not a canonical Bluetooth address
    /// **before** it can become an argv element. An address starting with `-`
    /// would otherwise be read by `bluetoothctl` as an option.
    fn validate_target(op: &BluetoothOp) -> Result<(), OsControlError> {
        match op.device() {
            Some(device) => selection::validate_address(device.as_str()).map(|_| ()),
            None => Ok(()),
        }
    }

    /// The argv that drives an operation toward its desired state.
    fn argv_for(op: &BluetoothOp) -> Vec<String> {
        match op {
            BluetoothOp::SetEnabled(enabled) => selection::set_power_argv(*enabled),
            BluetoothOp::Pair(d) => selection::pair_argv(d.as_str()),
            BluetoothOp::Connect(d) => selection::connect_argv(d.as_str()),
            BluetoothOp::Disconnect(d) => selection::disconnect_argv(d.as_str()),
            BluetoothOp::SetTrust { device, trusted } => {
                selection::trust_argv(device.as_str(), *trusted)
            }
            BluetoothOp::Remove(d) => selection::remove_argv(d.as_str()),
        }
    }

    /// The argv that undoes an operation, when an inverse exists.
    ///
    /// `pair` and `remove` have **no** inverse the runtime may claim: re-pairing
    /// needs a fresh human approval, and a removed device's pairing keys are gone.
    /// Returning `None` here is what keeps the receipt from advertising a rollback
    /// it cannot perform.

    fn satisfying(
        &self,
        observed: &BluetoothObservation,
    ) -> SatisfyingVerification<BluetoothObservation> {
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

/// The command that restores a previously observed Bluetooth fact.
///
/// # Why this is driven by the OBSERVED prior fact, not by the request
///
/// A compensation must restore what was actually true before, not the logical
/// opposite of what was asked for. If a connect request ran against a device that
/// was *already* connected, inverting the request would disconnect it — turning a
/// no-op into a real change the user never asked for.
///
/// # Why pairing returns `None`
///
/// Pairing exchanges keys with the device, and removal destroys them. Neither can
/// be undone by a command: re-pairing needs the device in pairing mode and a fresh
/// approval. Returning `None` here is what stops the runtime advertising a rollback
/// it cannot actually perform.
fn inverse_for_prior_fact(
    focus: BluetoothFocus,
    prior_value: bool,
    device: Option<&BluetoothDeviceId>,
) -> Option<Vec<String>> {
    match focus {
        BluetoothFocus::AdapterPower => Some(selection::set_power_argv(prior_value)),
        BluetoothFocus::DeviceConnected => device.map(|d| {
            if prior_value {
                selection::connect_argv(d.as_str())
            } else {
                selection::disconnect_argv(d.as_str())
            }
        }),
        BluetoothFocus::DeviceTrusted => {
            device.map(|d| selection::trust_argv(d.as_str(), prior_value))
        }
        // Pairing and removal have no inverse the runtime may claim.
        BluetoothFocus::DevicePaired | BluetoothFocus::DeviceKnown => None,
    }
}

#[async_trait]
impl<T: BluetoothTransport> DesiredStateControl<BluetoothRequest, BluetoothObservation>
    for BluetoothControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &BluetoothRequest,
    ) -> Result<BluetoothObservation, OsControlError> {
        self.observe_focus(ctx, request).await
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &BluetoothRequest,
        _desired: &BluetoothObservation,
    ) -> Result<ApplyOutcome, OsControlError> {
        // Capture the prior focused fact for a possible compensation. A failed
        // read must not block the mutation: it only means rollback is unavailable.
        if let Ok(before) = self.observe_focus(ctx.observation(), request).await {
            let session = ctx.grant().session_id().to_string();
            self.snapshots
                .lock()
                .expect("bluetooth snapshots poisoned")
                .insert(
                    session,
                    RollbackSnapshot {
                        before,
                        action: request.action.clone(),
                        params: request.params.clone(),
                    },
                );
        }

        // Refuse a non-canonical address before it reaches argv.
        Self::validate_target(&request.op)?;
        let args = Self::argv_for(&request.op);
        let command = self.build_command(ctx, &request.action, &request.params, args)?;
        self.transport.dispatch(ctx, &command).await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &BluetoothRequest,
        desired: &BluetoothObservation,
    ) -> Result<VerificationReport<BluetoothObservation>, OsControlError> {
        let observed = self.observe_focus(ctx, request).await?;

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
            .expect("bluetooth snapshots poisoned")
            .get(token.session_id().as_str())
            .cloned();

        let Some(snapshot) = snapshot else {
            return Ok(ApplyOutcome::Uncertain(
                crate::os_control::receipt::UncertainDispatch::new(
                    None,
                    crate::os_control::receipt::UncertainEffectCause::Unobservable,
                    crate::os_control::contract::BoundedVec::new(),
                ),
            ));
        };

        // Reconstruct the inverse from the recorded prior fact rather than from
        // the request, so a compensation restores what was actually observed.
        let inverse = inverse_for_prior_fact(
            snapshot.before.focus,
            snapshot.before.value,
            snapshot.before.device.as_ref(),
        );

        let Some(args) = inverse else {
            return Ok(ApplyOutcome::Uncertain(
                crate::os_control::receipt::UncertainDispatch::new(
                    None,
                    crate::os_control::receipt::UncertainEffectCause::Unobservable,
                    crate::os_control::contract::BoundedVec::new(),
                ),
            ));
        };

        let command = self.build_command(ctx, &snapshot.action, &snapshot.params, args)?;
        self.transport.dispatch(ctx, &command).await
    }
}

/// The object-safe Bluetooth port `HostOsControl::bluetooth()` returns.
///
/// The read passthroughs sit on the port rather than the mutation lifecycle
/// because `get_bluetooth_state` and `scan_bluetooth` are RED **reads**: they
/// never seal a mutation permit and never dispatch a command.
#[async_trait]
pub trait BluetoothControlPort:
    DesiredStateControl<BluetoothRequest, BluetoothObservation>
{
    /// The composed backend label (for the `backend` result field).
    fn backend(&self) -> BluetoothBackend;

    /// Read the full adapter + known-device state (`get_bluetooth_state`).
    async fn read_state(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<BluetoothState, OsControlError>;

    /// Run a bounded discovery scan (`scan_bluetooth`).
    ///
    /// The duration is clamped to [`BLUETOOTH_SCAN_MAX_MS`] so a caller cannot
    /// request an unbounded radio sweep.
    async fn scan(
        &self,
        ctx: &HostExecutionContext,
        duration_ms: u64,
    ) -> Result<BluetoothScan, OsControlError>;

    /// Whether `device` is already paired, used to classify `connect` risk:
    /// a new association is RED, reconnecting a known device is YELLOW.
    async fn is_paired(
        &self,
        ctx: &HostExecutionContext,
        device: &BluetoothDeviceId,
    ) -> Result<bool, OsControlError>;
}

#[async_trait]
impl<T: BluetoothTransport> BluetoothControlPort for BluetoothControl<T> {
    fn backend(&self) -> BluetoothBackend {
        self.transport.selected_backend()
    }

    async fn read_state(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<BluetoothState, OsControlError> {
        let adapter = self.transport.read_adapter(ctx).await?;
        let mut devices = self.transport.read_devices(ctx).await?;
        // Deterministic order so snapshots are stable across reads.
        devices.sort_by(|a, b| a.device.cmp(&b.device));
        Ok(BluetoothState { adapter, devices })
    }

    async fn scan(
        &self,
        ctx: &HostExecutionContext,
        duration_ms: u64,
    ) -> Result<BluetoothScan, OsControlError> {
        let bounded = duration_ms.min(BLUETOOTH_SCAN_MAX_MS);
        let mut scan = self.transport.scan(ctx, bounded).await?;
        if scan.devices.len() > BLUETOOTH_SCAN_MAX_DEVICES {
            scan.devices.truncate(BLUETOOTH_SCAN_MAX_DEVICES);
            scan.truncated = true;
        }
        scan.devices.sort_by(|a, b| a.device.cmp(&b.device));
        Ok(scan)
    }

    async fn is_paired(
        &self,
        ctx: &HostExecutionContext,
        device: &BluetoothDeviceId,
    ) -> Result<bool, OsControlError> {
        Ok(self
            .transport
            .read_devices(ctx)
            .await?
            .iter()
            .any(|d| &d.device == device && d.paired))
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    fn device(addr: &str) -> BluetoothDeviceId {
        BluetoothDeviceId::new(addr)
    }

    #[test]
    fn focus_is_part_of_the_digest_so_domains_cannot_cross_verify() {
        let adapter_on = BluetoothObservation::adapter_power(true);
        let device_connected = BluetoothObservation::device(
            BluetoothFocus::DeviceConnected,
            device("AA:BB:CC:DD:EE:FF"),
            true,
        );
        assert_ne!(
            adapter_on.observation_digest(),
            device_connected.observation_digest(),
            "an adapter fact must never satisfy a device postcondition"
        );
    }

    #[test]
    fn one_device_state_cannot_satisfy_another_devices_postcondition() {
        let a = BluetoothObservation::device(
            BluetoothFocus::DeviceConnected,
            device("AA:BB:CC:DD:EE:FF"),
            true,
        );
        let b = BluetoothObservation::device(
            BluetoothFocus::DeviceConnected,
            device("11:22:33:44:55:66"),
            true,
        );
        assert_ne!(a.observation_digest(), b.observation_digest());
    }

    #[test]
    fn desired_states_match_each_operations_intent() {
        let cases: Vec<(BluetoothOp, BluetoothFocus, bool)> = vec![
            (BluetoothOp::SetEnabled(true), BluetoothFocus::AdapterPower, true),
            (BluetoothOp::SetEnabled(false), BluetoothFocus::AdapterPower, false),
            (
                BluetoothOp::Pair(device("A")),
                BluetoothFocus::DevicePaired,
                true,
            ),
            (
                BluetoothOp::Connect(device("A")),
                BluetoothFocus::DeviceConnected,
                true,
            ),
            (
                BluetoothOp::Disconnect(device("A")),
                BluetoothFocus::DeviceConnected,
                false,
            ),
            (
                BluetoothOp::SetTrust { device: device("A"), trusted: true },
                BluetoothFocus::DeviceTrusted,
                true,
            ),
            // Removal is verified by the device no longer being known.
            (
                BluetoothOp::Remove(device("A")),
                BluetoothFocus::DeviceKnown,
                false,
            ),
        ];
        for (op, focus, value) in cases {
            let request = BluetoothRequest {
                action: op.action_name().to_string(),
                params: serde_json::json!({}),
                op: op.clone(),
            };
            assert_eq!(request.focus(), focus, "focus for {op:?}");
            let desired = request.desired_state();
            assert_eq!(desired.focus, focus);
            assert_eq!(desired.value, value, "desired value for {op:?}");
        }
    }

    #[test]
    fn pairing_and_removal_advertise_no_inverse() {
        assert!(
            inverse_for_prior_fact(BluetoothFocus::DevicePaired, false, Some(&device("A")))
                .is_none(),
            "re-pairing needs a fresh approval, so pair must not claim rollback"
        );
        assert!(
            inverse_for_prior_fact(BluetoothFocus::DeviceKnown, false, Some(&device("A")))
                .is_none(),
            "a removed device's pairing keys are gone, so remove must not claim rollback"
        );
        // A connection HAS an inverse, and it is driven by the prior fact: a
        // device observed disconnected is restored by disconnecting again, not by
        // inverting the request.
        assert!(
            inverse_for_prior_fact(
                BluetoothFocus::DeviceConnected,
                false,
                Some(&device("A"))
            )
            .is_some()
        );
    }

    #[test]
    fn addresses_are_bounded_and_control_char_free() {
        let noisy = BluetoothDeviceId::new(format!("AA:BB\u{0007}:CC{}", "X".repeat(200)));
        assert!(!noisy.as_str().contains('\u{0007}'));
        assert!(noisy.as_str().chars().count() <= BLUETOOTH_TOKEN_MAX_CHARS);
    }
}
