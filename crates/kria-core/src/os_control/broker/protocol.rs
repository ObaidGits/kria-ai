//! The closed, typed broker wire protocol: [`BrokerRequestV1`],
//! [`BrokerResponseV1`], their bindings, the closed [`BrokerPreDispatchError`],
//! and [`BrokerDispatchOutcome`].
//!
//! linux-os-control-production **Task 1.5**, design §12
//! (OSC-001, OSC-002, OSC-004, OSC-007, OSC-030, OSC-033).
//!
//! # The closed operation boundary (design §12)
//!
//! [`BrokerOperation`] is a **six-variant closed enum** and nothing else can be
//! expressed on the wire:
//!
//! 1. [`BrokerOperation::ApplyPackagePlan`]
//! 2. [`BrokerOperation::SetBoundPathOwnership`]
//! 3. [`BrokerOperation::SetFirewallEnabled`]
//! 4. [`BrokerOperation::SetPrivacyControl`]
//! 5. [`BrokerOperation::ConfigureDiscoveredPrinter`]
//! 6. [`BrokerOperation::SetBatteryChargeThresholds`]
//!
//! There is deliberately **no** generic-command, shell, arbitrary-file-write,
//! arbitrary-D-Bus, raw-device, service/unit, firmware, repository-mutation, or
//! run-as-root variant. Every field is a bounded typed value; the decoder
//! rejects any unknown operation tag ([`OperationDecodeError::UnknownOperation`])
//! and any unknown map key (closed schema / `additionalProperties:false`), so a
//! prohibited (BLACK) or generic operation is structurally unrepresentable
//! (OSC-002, OSC-030). [`BoundedBrokerEvidence`] has no field capable of holding
//! stdout/stderr/command text/secrets, so raw output cannot be encoded either
//! (OSC-007).
//!
//! # Authority binding (design §12)
//!
//! Every request and response carries the full authority binding — caller,
//! grant, action, parameter, host-target, resource-set, audit-admission,
//! operation, nonce, and expiry. A [`BrokerResponseBinding`] must byte-for-byte
//! echo the request's binding fields; the client rejects any mismatch before
//! interpreting the outcome (see [`super::client`]).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::os_control::contract::{
    AuditAdmissionId, BoundedVec, Digest, GrantId, GrantNonce, NonEmptyBoundedVec, ProviderId,
    SafeField, SafeStepId, SafeText,
};
use crate::os_control::receipt::{PartialEffectCause, UncertainEffectCause};

use super::cbor::{decode_canonical, frame, unframe, CborError, CborValue};

/// The protocol version. Only `1` is ever accepted; a differing version yields
/// [`BrokerPreDispatchError::UnsupportedVersion`] before dispatch.
pub const PROTOCOL_VERSION: u64 = 1;

/// Hard cap on the number of package-transaction steps in one request.
pub const MAX_PACKAGE_STEPS: usize = 256;
/// Hard cap on the number of normalized evidence fields in one response.
pub const MAX_EVIDENCE_FIELDS: usize = 64;
/// Hard cap on completed steps echoed in a partial outcome.
pub const MAX_COMPLETED_STEPS: usize = 256;

// ─────────────────────────────────────────────────────────────────────────────
// Schema-level decode errors (structural)
// ─────────────────────────────────────────────────────────────────────────────

/// A structural schema decode failure. Structural failures cannot be bound to a
/// response (there is no trustworthy binding to echo), so the transport rejects
/// the frame outright (design §12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    /// A required field was absent.
    MissingField {
        /// The integer field key.
        key: u64,
    },
    /// A field had the wrong CBOR type.
    WrongType,
    /// An unknown / non-integer map key was present (closed schema violation).
    UnknownKey,
    /// A value was outside its permitted bounds.
    OutOfRange,
    /// A closed-enum tag was not recognized.
    UnknownVariant,
    /// The `follow_symlinks: False` invariant was violated.
    IllegalFollowSymlinks,
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaError::MissingField { key } => write!(f, "missing required field {key}"),
            SchemaError::WrongType => f.write_str("wrong cbor type for field"),
            SchemaError::UnknownKey => f.write_str("unknown map key (closed schema)"),
            SchemaError::OutOfRange => f.write_str("value out of range"),
            SchemaError::UnknownVariant => f.write_str("unknown closed-enum tag"),
            SchemaError::IllegalFollowSymlinks => f.write_str("follow_symlinks must be false"),
        }
    }
}

impl std::error::Error for SchemaError {}

// ─────────────────────────────────────────────────────────────────────────────
// Map reader with closed-key enforcement
// ─────────────────────────────────────────────────────────────────────────────

/// Reads integer-keyed CBOR maps, removing consumed keys so [`Self::finish`] can
/// reject any leftover (unknown) key — the `additionalProperties:false` rule.
struct MapReader {
    entries: Vec<(u64, CborValue)>,
}

impl MapReader {
    fn new(value: CborValue) -> Result<Self, SchemaError> {
        match value {
            CborValue::Map(entries) => {
                let mut out = Vec::with_capacity(entries.len());
                for (k, v) in entries {
                    match k {
                        CborValue::Uint(key) => out.push((key, v)),
                        _ => return Err(SchemaError::UnknownKey),
                    }
                }
                Ok(Self { entries: out })
            }
            _ => Err(SchemaError::WrongType),
        }
    }

    fn take(&mut self, key: u64) -> Result<CborValue, SchemaError> {
        match self.entries.iter().position(|(k, _)| *k == key) {
            Some(pos) => Ok(self.entries.remove(pos).1),
            None => Err(SchemaError::MissingField { key }),
        }
    }

    fn take_uint(&mut self, key: u64) -> Result<u64, SchemaError> {
        match self.take(key)? {
            CborValue::Uint(v) => Ok(v),
            _ => Err(SchemaError::WrongType),
        }
    }

    fn take_text(&mut self, key: u64) -> Result<String, SchemaError> {
        match self.take(key)? {
            CborValue::Text(s) => Ok(s),
            _ => Err(SchemaError::WrongType),
        }
    }

    fn take_bool(&mut self, key: u64) -> Result<bool, SchemaError> {
        match self.take(key)? {
            CborValue::Bool(b) => Ok(b),
            _ => Err(SchemaError::WrongType),
        }
    }

    fn take_array(&mut self, key: u64) -> Result<Vec<CborValue>, SchemaError> {
        match self.take(key)? {
            CborValue::Array(items) => Ok(items),
            _ => Err(SchemaError::WrongType),
        }
    }

    fn finish(self) -> Result<(), SchemaError> {
        if self.entries.is_empty() {
            Ok(())
        } else {
            Err(SchemaError::UnknownKey)
        }
    }
}

fn system_time_to_millis(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn millis_to_system_time(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

fn digest_to_cbor(d: &Digest) -> CborValue {
    CborValue::Text(d.as_hex().to_string())
}

fn text_field(reader: &mut MapReader, key: u64) -> Result<Digest, SchemaError> {
    Ok(Digest::from_hex(reader.take_text(key)?))
}

// ─────────────────────────────────────────────────────────────────────────────
// Closed provider / control identity enums
// ─────────────────────────────────────────────────────────────────────────────

macro_rules! closed_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $($variant:ident = $tag:literal => $token:literal),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $(
                #[doc = concat!("The `", $token, "` variant.")]
                $variant,
            )+
        }

        impl $name {
            /// The stable wire tag.
            #[must_use]
            pub const fn tag(self) -> u64 {
                match self { $( $name::$variant => $tag, )+ }
            }

            /// The stable redaction-safe token.
            #[must_use]
            pub const fn token(self) -> &'static str {
                match self { $( $name::$variant => $token, )+ }
            }

            /// Parse a wire tag into a variant, rejecting unknown tags.
            pub fn from_tag(tag: u64) -> Result<Self, SchemaError> {
                match tag {
                    $( $tag => Ok($name::$variant), )+
                    _ => Err(SchemaError::UnknownVariant),
                }
            }

            /// Every variant (coverage tests).
            #[must_use]
            pub fn all() -> &'static [$name] {
                &[ $( $name::$variant, )+ ]
            }
        }
    };
}

closed_enum!(
    /// Recognized package providers a plan may target. No repository/key data is
    /// carried; only the provider identity.
    PackageProviderId {
        Apt = 0 => "apt",
        Snap = 1 => "snap",
        Flatpak = 2 => "flatpak",
    }
);

closed_enum!(
    /// Recognized firewall providers.
    FirewallProviderId {
        Ufw = 0 => "ufw",
        Firewalld = 1 => "firewalld",
    }
);

closed_enum!(
    /// Recognized privacy controls discovered by capability probe.
    RecognizedPrivacyControl {
        CameraAccess = 0 => "camera-access",
        MicrophoneAccess = 1 => "microphone-access",
        LocationAccess = 2 => "location-access",
    }
);

closed_enum!(
    /// Recognized battery charge-threshold adapters.
    ChargeThresholdAdapterId {
        SysfsStandard = 0 => "sysfs-standard",
        ThinkpadAcpi = 1 => "thinkpad-acpi",
    }
);

closed_enum!(
    /// The action of one package-transaction step. There is no "run script" or
    /// "add repository" action.
    PackageStepAction {
        Install = 0 => "install",
        Remove = 1 => "remove",
        Upgrade = 2 => "upgrade",
    }
);

// ─────────────────────────────────────────────────────────────────────────────
// Bounded parameter value types
// ─────────────────────────────────────────────────────────────────────────────

/// A percentage bounded to `0..=100` at construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BoundedPercent(u8);

impl BoundedPercent {
    /// Construct, rejecting values above 100.
    pub fn new(value: u8) -> Result<Self, SchemaError> {
        if value > 100 {
            Err(SchemaError::OutOfRange)
        } else {
            Ok(Self(value))
        }
    }

    /// The inner value.
    #[must_use]
    pub fn get(self) -> u8 {
        self.0
    }
}

/// A bounded package name. It cannot carry a path, shell metacharacter, or
/// whitespace — only a recognizable package-identifier charset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedPackageName(String);

impl BoundedPackageName {
    /// Maximum package-name length.
    pub const MAX_CHARS: usize = 128;

    /// Construct, rejecting empty, over-long, or illegal-charset names.
    pub fn new(raw: impl Into<String>) -> Result<Self, SchemaError> {
        let raw = raw.into();
        if raw.is_empty() || raw.chars().count() > Self::MAX_CHARS {
            return Err(SchemaError::OutOfRange);
        }
        let ok = raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+' | ':'));
        if !ok {
            return Err(SchemaError::OutOfRange);
        }
        Ok(Self(raw))
    }

    /// Borrow the name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One package-transaction step: a fixed action on a bounded package name. It
/// cannot carry executable/argv/repository/key data by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageStep {
    /// The fixed action.
    pub action: PackageStepAction,
    /// The bounded package name.
    pub package: BoundedPackageName,
}

impl PackageStep {
    fn to_cbor(&self) -> CborValue {
        CborValue::Map(vec![
            (CborValue::Uint(0), CborValue::Uint(self.action.tag())),
            (
                CborValue::Uint(1),
                CborValue::Text(self.package.as_str().to_string()),
            ),
        ])
    }

    fn from_cbor(value: CborValue) -> Result<Self, SchemaError> {
        let mut r = MapReader::new(value)?;
        let action = PackageStepAction::from_tag(r.take_uint(0)?)?;
        let package = BoundedPackageName::new(r.take_text(1)?)?;
        r.finish()?;
        Ok(Self { action, package })
    }
}

/// A bounded, non-empty package transaction decoded from an approved normalized
/// plan. It carries only fixed steps — no executable/argv/repository/key data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedPackageTransaction {
    steps: NonEmptyBoundedVec<PackageStep>,
}

impl BoundedPackageTransaction {
    /// Construct from a non-empty step set.
    #[must_use]
    pub fn new(steps: NonEmptyBoundedVec<PackageStep>) -> Self {
        Self { steps }
    }

    /// Borrow the steps.
    #[must_use]
    pub fn steps(&self) -> &NonEmptyBoundedVec<PackageStep> {
        &self.steps
    }

    fn to_cbor(&self) -> CborValue {
        let mut arr = Vec::with_capacity(self.steps.len());
        arr.push(self.steps.head().to_cbor());
        for s in self.steps.tail() {
            arr.push(s.to_cbor());
        }
        CborValue::Map(vec![(CborValue::Uint(0), CborValue::Array(arr))])
    }

    fn from_cbor(value: CborValue) -> Result<Self, SchemaError> {
        let mut r = MapReader::new(value)?;
        let items = r.take_array(0)?;
        r.finish()?;
        if items.is_empty() || items.len() > MAX_PACKAGE_STEPS {
            return Err(SchemaError::OutOfRange);
        }
        let mut iter = items.into_iter();
        let head = PackageStep::from_cbor(iter.next().expect("non-empty checked"))?;
        let mut tail = BoundedVec::with_cap(MAX_PACKAGE_STEPS);
        for item in iter {
            if !tail.try_push(PackageStep::from_cbor(item)?) {
                return Err(SchemaError::OutOfRange);
            }
        }
        Ok(Self {
            steps: NonEmptyBoundedVec::new(head, tail),
        })
    }
}

/// An approved canonical path plus the expected device/inode/owner identity it
/// must still match immediately before the operation (design §12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerBoundPath {
    /// The approved absolute canonical path.
    pub path: String,
    /// Expected device number.
    pub device: u64,
    /// Expected inode number.
    pub inode: u64,
    /// Expected owner uid.
    pub owner_uid: u32,
}

impl BrokerBoundPath {
    fn to_cbor(&self) -> CborValue {
        CborValue::Map(vec![
            (CborValue::Uint(0), CborValue::Text(self.path.clone())),
            (CborValue::Uint(1), CborValue::Uint(self.device)),
            (CborValue::Uint(2), CborValue::Uint(self.inode)),
            (
                CborValue::Uint(3),
                CborValue::Uint(u64::from(self.owner_uid)),
            ),
        ])
    }

    fn from_cbor(value: CborValue) -> Result<Self, SchemaError> {
        let mut r = MapReader::new(value)?;
        let path = r.take_text(0)?;
        let device = r.take_uint(1)?;
        let inode = r.take_uint(2)?;
        let owner_uid = u32::try_from(r.take_uint(3)?).map_err(|_| SchemaError::OutOfRange)?;
        r.finish()?;
        if !path.starts_with('/') || path.chars().any(char::is_control) {
            return Err(SchemaError::OutOfRange);
        }
        Ok(Self {
            path,
            device,
            inode,
            owner_uid,
        })
    }
}

/// An existing local identity (never an arbitrary uid; the broker verifies the
/// identity exists before applying).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingLocalIdentity {
    /// The local uid.
    pub uid: u32,
    /// The redacted local account name.
    pub name: SafeText,
}

impl ExistingLocalIdentity {
    fn to_cbor(&self) -> CborValue {
        CborValue::Map(vec![
            (CborValue::Uint(0), CborValue::Uint(u64::from(self.uid))),
            (
                CborValue::Uint(1),
                CborValue::Text(self.name.as_str().to_string()),
            ),
        ])
    }

    fn from_cbor(value: CborValue) -> Result<Self, SchemaError> {
        let mut r = MapReader::new(value)?;
        let uid = u32::try_from(r.take_uint(0)?).map_err(|_| SchemaError::OutOfRange)?;
        let name = SafeText::new(r.take_text(1)?);
        r.finish()?;
        Ok(Self { uid, name })
    }
}

/// A discovered-printer identity (opaque, bounded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredPrinterId(String);

impl DiscoveredPrinterId {
    /// Maximum length.
    pub const MAX_CHARS: usize = 128;

    /// Construct, bounding length and rejecting control characters.
    pub fn new(raw: impl Into<String>) -> Result<Self, SchemaError> {
        let raw = raw.into();
        if raw.is_empty()
            || raw.chars().count() > Self::MAX_CHARS
            || raw.chars().any(char::is_control)
        {
            return Err(SchemaError::OutOfRange);
        }
        Ok(Self(raw))
    }

    /// Borrow the identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A closed set of reviewed printer options (design §12: "Printer options are a
/// closed set"). There is no free-form option string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewedPrinterOptions {
    /// Make this the default printer.
    pub set_default: bool,
    /// Share the printer.
    pub shared: bool,
    /// Accept jobs.
    pub accept_jobs: bool,
}

impl ReviewedPrinterOptions {
    fn to_cbor(self) -> CborValue {
        CborValue::Map(vec![
            (CborValue::Uint(0), CborValue::Bool(self.set_default)),
            (CborValue::Uint(1), CborValue::Bool(self.shared)),
            (CborValue::Uint(2), CborValue::Bool(self.accept_jobs)),
        ])
    }

    fn from_cbor(value: CborValue) -> Result<Self, SchemaError> {
        let mut r = MapReader::new(value)?;
        let set_default = r.take_bool(0)?;
        let shared = r.take_bool(1)?;
        let accept_jobs = r.take_bool(2)?;
        r.finish()?;
        Ok(Self {
            set_default,
            shared,
            accept_jobs,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The closed six-operation enum
// ─────────────────────────────────────────────────────────────────────────────

/// The **only** operations the broker will ever perform (design §12). This enum
/// is closed: there is no generic-command, shell, arbitrary-file-write,
/// arbitrary-D-Bus, raw-device, service/unit, firmware, repository-mutation, or
/// run-as-root variant, and none can be added on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerOperation {
    /// Apply an approved, normalized package plan through a recognized provider.
    ApplyPackagePlan {
        /// The recognized package provider.
        provider: PackageProviderId,
        /// Digest of the approved normalized plan.
        approved_plan_digest: Digest,
        /// The bounded transaction decoded from the approved plan.
        transaction: BoundedPackageTransaction,
    },
    /// Set ownership of an approved, identity-bound path (never following
    /// symlinks).
    SetBoundPathOwnership {
        /// The approved path with expected device/inode/owner identity.
        path: BrokerBoundPath,
        /// The existing local identity to assign.
        owner: ExistingLocalIdentity,
    },
    /// Enable or disable a recognized high-level firewall.
    SetFirewallEnabled {
        /// The recognized firewall provider.
        provider: FirewallProviderId,
        /// Desired enabled state.
        enabled: bool,
    },
    /// Enable or disable a recognized privacy control.
    SetPrivacyControl {
        /// The recognized privacy control.
        control: RecognizedPrivacyControl,
        /// Desired enabled state.
        enabled: bool,
    },
    /// Configure a discovered printer with reviewed, closed-set options.
    ConfigureDiscoveredPrinter {
        /// The discovered printer identity.
        printer: DiscoveredPrinterId,
        /// The reviewed closed-set options.
        options: ReviewedPrinterOptions,
    },
    /// Set battery charge thresholds through a recognized adapter.
    SetBatteryChargeThresholds {
        /// The recognized adapter.
        adapter: ChargeThresholdAdapterId,
        /// Lower charge-start percent.
        lower_percent: BoundedPercent,
        /// Upper charge-stop percent.
        upper_percent: BoundedPercent,
    },
}

/// The wire tag for each operation variant.
impl BrokerOperation {
    /// The closed operation tag (0..=5).
    #[must_use]
    pub const fn tag(&self) -> u64 {
        match self {
            BrokerOperation::ApplyPackagePlan { .. } => 0,
            BrokerOperation::SetBoundPathOwnership { .. } => 1,
            BrokerOperation::SetFirewallEnabled { .. } => 2,
            BrokerOperation::SetPrivacyControl { .. } => 3,
            BrokerOperation::ConfigureDiscoveredPrinter { .. } => 4,
            BrokerOperation::SetBatteryChargeThresholds { .. } => 5,
        }
    }

    /// The stable redaction-safe operation token (never model prose).
    #[must_use]
    pub const fn token(&self) -> &'static str {
        match self {
            BrokerOperation::ApplyPackagePlan { .. } => "apply_package_plan",
            BrokerOperation::SetBoundPathOwnership { .. } => "set_bound_path_ownership",
            BrokerOperation::SetFirewallEnabled { .. } => "set_firewall_enabled",
            BrokerOperation::SetPrivacyControl { .. } => "set_privacy_control",
            BrokerOperation::ConfigureDiscoveredPrinter { .. } => "configure_discovered_printer",
            BrokerOperation::SetBatteryChargeThresholds { .. } => "set_battery_charge_thresholds",
        }
    }

    /// The number of closed operation variants.
    pub const COUNT: usize = 6;

    /// The canonical digest binding this exact operation and its parameters,
    /// carried in the request's `operation_digest` field and echoed by the
    /// response binding.
    #[must_use]
    pub fn operation_digest(&self) -> Digest {
        Digest::of_bytes(&self.to_cbor().to_canonical_bytes())
    }

    fn to_cbor(&self) -> CborValue {
        let mut entries = vec![(CborValue::Uint(0), CborValue::Uint(self.tag()))];
        match self {
            BrokerOperation::ApplyPackagePlan {
                provider,
                approved_plan_digest,
                transaction,
            } => {
                entries.push((CborValue::Uint(1), CborValue::Uint(provider.tag())));
                entries.push((CborValue::Uint(2), digest_to_cbor(approved_plan_digest)));
                entries.push((CborValue::Uint(3), transaction.to_cbor()));
            }
            BrokerOperation::SetBoundPathOwnership { path, owner } => {
                entries.push((CborValue::Uint(1), path.to_cbor()));
                entries.push((CborValue::Uint(2), owner.to_cbor()));
                // follow_symlinks is type-level `False`; encode explicitly so the
                // decoder can reject any `true`.
                entries.push((CborValue::Uint(3), CborValue::Bool(false)));
            }
            BrokerOperation::SetFirewallEnabled { provider, enabled } => {
                entries.push((CborValue::Uint(1), CborValue::Uint(provider.tag())));
                entries.push((CborValue::Uint(2), CborValue::Bool(*enabled)));
            }
            BrokerOperation::SetPrivacyControl { control, enabled } => {
                entries.push((CborValue::Uint(1), CborValue::Uint(control.tag())));
                entries.push((CborValue::Uint(2), CborValue::Bool(*enabled)));
            }
            BrokerOperation::ConfigureDiscoveredPrinter { printer, options } => {
                entries.push((
                    CborValue::Uint(1),
                    CborValue::Text(printer.as_str().to_string()),
                ));
                entries.push((CborValue::Uint(2), options.to_cbor()));
            }
            BrokerOperation::SetBatteryChargeThresholds {
                adapter,
                lower_percent,
                upper_percent,
            } => {
                entries.push((CborValue::Uint(1), CborValue::Uint(adapter.tag())));
                entries.push((
                    CborValue::Uint(2),
                    CborValue::Uint(u64::from(lower_percent.get())),
                ));
                entries.push((
                    CborValue::Uint(3),
                    CborValue::Uint(u64::from(upper_percent.get())),
                ));
            }
        }
        CborValue::Map(entries)
    }

    fn from_cbor(value: CborValue) -> Result<Self, OperationDecodeError> {
        let mut r = MapReader::new(value).map_err(OperationDecodeError::Invalid)?;
        let tag = r.take_uint(0).map_err(OperationDecodeError::Invalid)?;
        let op = match tag {
            0 => {
                let provider = PackageProviderId::from_tag(
                    r.take_uint(1).map_err(OperationDecodeError::Invalid)?,
                )
                .map_err(OperationDecodeError::Invalid)?;
                let approved_plan_digest =
                    text_field(&mut r, 2).map_err(OperationDecodeError::Invalid)?;
                let transaction = BoundedPackageTransaction::from_cbor(
                    r.take(3).map_err(OperationDecodeError::Invalid)?,
                )
                .map_err(OperationDecodeError::Invalid)?;
                BrokerOperation::ApplyPackagePlan {
                    provider,
                    approved_plan_digest,
                    transaction,
                }
            }
            1 => {
                let path =
                    BrokerBoundPath::from_cbor(r.take(1).map_err(OperationDecodeError::Invalid)?)
                        .map_err(OperationDecodeError::Invalid)?;
                let owner = ExistingLocalIdentity::from_cbor(
                    r.take(2).map_err(OperationDecodeError::Invalid)?,
                )
                .map_err(OperationDecodeError::Invalid)?;
                let follow = r.take_bool(3).map_err(OperationDecodeError::Invalid)?;
                if follow {
                    return Err(OperationDecodeError::Invalid(
                        SchemaError::IllegalFollowSymlinks,
                    ));
                }
                BrokerOperation::SetBoundPathOwnership { path, owner }
            }
            2 => {
                let provider = FirewallProviderId::from_tag(
                    r.take_uint(1).map_err(OperationDecodeError::Invalid)?,
                )
                .map_err(OperationDecodeError::Invalid)?;
                let enabled = r.take_bool(2).map_err(OperationDecodeError::Invalid)?;
                BrokerOperation::SetFirewallEnabled { provider, enabled }
            }
            3 => {
                let control = RecognizedPrivacyControl::from_tag(
                    r.take_uint(1).map_err(OperationDecodeError::Invalid)?,
                )
                .map_err(OperationDecodeError::Invalid)?;
                let enabled = r.take_bool(2).map_err(OperationDecodeError::Invalid)?;
                BrokerOperation::SetPrivacyControl { control, enabled }
            }
            4 => {
                let printer = DiscoveredPrinterId::new(
                    r.take_text(1).map_err(OperationDecodeError::Invalid)?,
                )
                .map_err(OperationDecodeError::Invalid)?;
                let options = ReviewedPrinterOptions::from_cbor(
                    r.take(2).map_err(OperationDecodeError::Invalid)?,
                )
                .map_err(OperationDecodeError::Invalid)?;
                BrokerOperation::ConfigureDiscoveredPrinter { printer, options }
            }
            5 => {
                let adapter = ChargeThresholdAdapterId::from_tag(
                    r.take_uint(1).map_err(OperationDecodeError::Invalid)?,
                )
                .map_err(OperationDecodeError::Invalid)?;
                let lower = u8::try_from(r.take_uint(2).map_err(OperationDecodeError::Invalid)?)
                    .map_err(|_| OperationDecodeError::Invalid(SchemaError::OutOfRange))?;
                let upper = u8::try_from(r.take_uint(3).map_err(OperationDecodeError::Invalid)?)
                    .map_err(|_| OperationDecodeError::Invalid(SchemaError::OutOfRange))?;
                let lower_percent =
                    BoundedPercent::new(lower).map_err(OperationDecodeError::Invalid)?;
                let upper_percent =
                    BoundedPercent::new(upper).map_err(OperationDecodeError::Invalid)?;
                BrokerOperation::SetBatteryChargeThresholds {
                    adapter,
                    lower_percent,
                    upper_percent,
                }
            }
            _ => return Err(OperationDecodeError::UnknownOperation),
        };
        r.finish().map_err(OperationDecodeError::Invalid)?;
        Ok(op)
    }
}

/// Why an operation payload failed to decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationDecodeError {
    /// The operation tag was outside the closed six-variant set.
    UnknownOperation,
    /// The operation tag was recognized but its parameters were invalid.
    Invalid(SchemaError),
}

// ─────────────────────────────────────────────────────────────────────────────
// Bounded broker evidence (never raw output)
// ─────────────────────────────────────────────────────────────────────────────

/// One normalized state-query field surfaced in evidence. Both key and value are
/// bounded, redacted safe text; neither can hold raw stdout/stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceField {
    /// The normalized field key.
    pub key: SafeField,
    /// The normalized field value.
    pub value: SafeText,
}

/// Bounded, operation-specific normalized evidence (design §12). It contains
/// only normalized state-query fields, the provider identity, and an evidence
/// digest — **never** stdout, stderr, command text, D-Bus payloads, secrets, or
/// free-form errors (OSC-007). There is structurally no field for raw output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedBrokerEvidence {
    provider: ProviderId,
    evidence_digest: Digest,
    fields: BoundedVec<EvidenceField>,
}

impl BoundedBrokerEvidence {
    /// Construct bounded evidence, truncating fields at [`MAX_EVIDENCE_FIELDS`].
    #[must_use]
    pub fn new(
        provider: ProviderId,
        evidence_digest: Digest,
        fields: impl IntoIterator<Item = EvidenceField>,
    ) -> Self {
        Self {
            provider,
            evidence_digest,
            fields: BoundedVec::from_iter_capped(fields, MAX_EVIDENCE_FIELDS),
        }
    }

    /// The verifying provider.
    #[must_use]
    pub fn provider(&self) -> &ProviderId {
        &self.provider
    }

    /// The evidence digest.
    #[must_use]
    pub fn evidence_digest(&self) -> &Digest {
        &self.evidence_digest
    }

    /// The normalized fields.
    #[must_use]
    pub fn fields(&self) -> &[EvidenceField] {
        self.fields.as_slice()
    }

    fn to_cbor(&self) -> CborValue {
        let fields = self
            .fields
            .as_slice()
            .iter()
            .map(|f| {
                CborValue::Map(vec![
                    (
                        CborValue::Uint(0),
                        CborValue::Text(f.key.as_str().to_string()),
                    ),
                    (
                        CborValue::Uint(1),
                        CborValue::Text(f.value.as_str().to_string()),
                    ),
                ])
            })
            .collect();
        CborValue::Map(vec![
            (
                CborValue::Uint(0),
                CborValue::Text(self.provider.as_str().to_string()),
            ),
            (CborValue::Uint(1), digest_to_cbor(&self.evidence_digest)),
            (CborValue::Uint(2), CborValue::Array(fields)),
        ])
    }

    fn from_cbor(value: CborValue) -> Result<Self, SchemaError> {
        let mut r = MapReader::new(value)?;
        let provider = ProviderId::new(r.take_text(0)?);
        let evidence_digest = text_field(&mut r, 1)?;
        let raw_fields = r.take_array(2)?;
        r.finish()?;
        if raw_fields.len() > MAX_EVIDENCE_FIELDS {
            return Err(SchemaError::OutOfRange);
        }
        let mut fields = BoundedVec::with_cap(MAX_EVIDENCE_FIELDS);
        for item in raw_fields {
            let mut fr = MapReader::new(item)?;
            let key = SafeField::new(fr.take_text(0)?);
            let val = SafeText::new(fr.take_text(1)?);
            fr.finish()?;
            fields.try_push(EvidenceField { key, value: val });
        }
        Ok(Self {
            provider,
            evidence_digest,
            fields,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Dispatch outcome (post-dispatch, three narrow families)
// ─────────────────────────────────────────────────────────────────────────────

/// The three — and only three — post-dispatch outcome families (design §12).
/// Each maps directly to a narrow §4 dispatch type. Once dispatch may have
/// occurred the broker can return only these; transport loss is `Uncertain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerDispatchOutcome {
    /// The effect was applied and is verifiable.
    Applied {
        /// Receipt digest.
        receipt_digest: Digest,
        /// Bounded normalized evidence.
        evidence: BoundedBrokerEvidence,
    },
    /// The effect may or may not have taken hold.
    Uncertain {
        /// Optional receipt digest.
        receipt_digest: Option<Digest>,
        /// Why the effect is uncertain.
        cause: UncertainEffectCause,
        /// Bounded normalized evidence.
        evidence: BoundedBrokerEvidence,
    },
    /// A multi-step effect completed some steps and failed one.
    PartiallyApplied {
        /// Optional receipt digest.
        receipt_digest: Option<Digest>,
        /// Completed steps (non-empty).
        completed_steps: NonEmptyBoundedVec<SafeStepId>,
        /// The step that failed.
        failed_step: SafeStepId,
        /// Why the effect is partial.
        cause: PartialEffectCause,
        /// Bounded normalized evidence.
        evidence: BoundedBrokerEvidence,
    },
}

fn uncertain_cause_tag(cause: UncertainEffectCause) -> u64 {
    match cause {
        UncertainEffectCause::ProviderReportedFailureAfterDispatch => 0,
        UncertainEffectCause::TransportLostAfterDispatch => 1,
        UncertainEffectCause::TimedOutAfterDispatch => 2,
        UncertainEffectCause::CancelledAfterDispatch => 3,
        UncertainEffectCause::Unobservable => 4,
    }
}

fn uncertain_cause_from_tag(tag: u64) -> Result<UncertainEffectCause, SchemaError> {
    Ok(match tag {
        0 => UncertainEffectCause::ProviderReportedFailureAfterDispatch,
        1 => UncertainEffectCause::TransportLostAfterDispatch,
        2 => UncertainEffectCause::TimedOutAfterDispatch,
        3 => UncertainEffectCause::CancelledAfterDispatch,
        4 => UncertainEffectCause::Unobservable,
        _ => return Err(SchemaError::UnknownVariant),
    })
}

fn partial_cause_tag(cause: PartialEffectCause) -> u64 {
    match cause {
        PartialEffectCause::StepFailedAfterCommit => 0,
        PartialEffectCause::TimedOutMidSequence => 1,
        PartialEffectCause::CancelledMidSequence => 2,
    }
}

fn partial_cause_from_tag(tag: u64) -> Result<PartialEffectCause, SchemaError> {
    Ok(match tag {
        0 => PartialEffectCause::StepFailedAfterCommit,
        1 => PartialEffectCause::TimedOutMidSequence,
        2 => PartialEffectCause::CancelledMidSequence,
        _ => return Err(SchemaError::UnknownVariant),
    })
}

fn opt_digest_to_cbor(d: &Option<Digest>) -> CborValue {
    match d {
        Some(d) => digest_to_cbor(d),
        None => CborValue::Null,
    }
}

fn opt_digest_from_cbor(v: CborValue) -> Result<Option<Digest>, SchemaError> {
    match v {
        CborValue::Null => Ok(None),
        CborValue::Text(s) => Ok(Some(Digest::from_hex(s))),
        _ => Err(SchemaError::WrongType),
    }
}

fn steps_to_cbor(steps: &NonEmptyBoundedVec<SafeStepId>) -> CborValue {
    let mut arr = Vec::with_capacity(steps.len());
    arr.push(CborValue::Text(steps.head().as_str().to_string()));
    for s in steps.tail() {
        arr.push(CborValue::Text(s.as_str().to_string()));
    }
    CborValue::Array(arr)
}

fn steps_from_cbor(items: Vec<CborValue>) -> Result<NonEmptyBoundedVec<SafeStepId>, SchemaError> {
    if items.is_empty() || items.len() > MAX_COMPLETED_STEPS {
        return Err(SchemaError::OutOfRange);
    }
    let mut iter = items.into_iter();
    let head = match iter.next().expect("non-empty checked") {
        CborValue::Text(s) => SafeStepId::new(s),
        _ => return Err(SchemaError::WrongType),
    };
    let mut tail = BoundedVec::with_cap(MAX_COMPLETED_STEPS);
    for item in iter {
        match item {
            CborValue::Text(s) => {
                tail.try_push(SafeStepId::new(s));
            }
            _ => return Err(SchemaError::WrongType),
        }
    }
    Ok(NonEmptyBoundedVec::new(head, tail))
}

impl BrokerDispatchOutcome {
    /// The wire tag (0 Applied, 1 Uncertain, 2 PartiallyApplied).
    #[must_use]
    pub const fn tag(&self) -> u64 {
        match self {
            BrokerDispatchOutcome::Applied { .. } => 0,
            BrokerDispatchOutcome::Uncertain { .. } => 1,
            BrokerDispatchOutcome::PartiallyApplied { .. } => 2,
        }
    }

    fn to_cbor(&self) -> CborValue {
        match self {
            BrokerDispatchOutcome::Applied {
                receipt_digest,
                evidence,
            } => CborValue::Map(vec![
                (CborValue::Uint(0), CborValue::Uint(0)),
                (CborValue::Uint(1), digest_to_cbor(receipt_digest)),
                (CborValue::Uint(2), evidence.to_cbor()),
            ]),
            BrokerDispatchOutcome::Uncertain {
                receipt_digest,
                cause,
                evidence,
            } => CborValue::Map(vec![
                (CborValue::Uint(0), CborValue::Uint(1)),
                (CborValue::Uint(1), opt_digest_to_cbor(receipt_digest)),
                (
                    CborValue::Uint(2),
                    CborValue::Uint(uncertain_cause_tag(*cause)),
                ),
                (CborValue::Uint(3), evidence.to_cbor()),
            ]),
            BrokerDispatchOutcome::PartiallyApplied {
                receipt_digest,
                completed_steps,
                failed_step,
                cause,
                evidence,
            } => CborValue::Map(vec![
                (CborValue::Uint(0), CborValue::Uint(2)),
                (CborValue::Uint(1), opt_digest_to_cbor(receipt_digest)),
                (CborValue::Uint(2), steps_to_cbor(completed_steps)),
                (
                    CborValue::Uint(3),
                    CborValue::Text(failed_step.as_str().to_string()),
                ),
                (
                    CborValue::Uint(4),
                    CborValue::Uint(partial_cause_tag(*cause)),
                ),
                (CborValue::Uint(5), evidence.to_cbor()),
            ]),
        }
    }

    fn from_cbor(value: CborValue) -> Result<Self, SchemaError> {
        let mut r = MapReader::new(value)?;
        let tag = r.take_uint(0)?;
        let out = match tag {
            0 => {
                let receipt_digest = text_field(&mut r, 1)?;
                let evidence = BoundedBrokerEvidence::from_cbor(r.take(2)?)?;
                BrokerDispatchOutcome::Applied {
                    receipt_digest,
                    evidence,
                }
            }
            1 => {
                let receipt_digest = opt_digest_from_cbor(r.take(1)?)?;
                let cause = uncertain_cause_from_tag(r.take_uint(2)?)?;
                let evidence = BoundedBrokerEvidence::from_cbor(r.take(3)?)?;
                BrokerDispatchOutcome::Uncertain {
                    receipt_digest,
                    cause,
                    evidence,
                }
            }
            2 => {
                let receipt_digest = opt_digest_from_cbor(r.take(1)?)?;
                let completed_steps = steps_from_cbor(r.take_array(2)?)?;
                let failed_step = SafeStepId::new(r.take_text(3)?);
                let cause = partial_cause_from_tag(r.take_uint(4)?)?;
                let evidence = BoundedBrokerEvidence::from_cbor(r.take(5)?)?;
                BrokerDispatchOutcome::PartiallyApplied {
                    receipt_digest,
                    completed_steps,
                    failed_step,
                    cause,
                    evidence,
                }
            }
            _ => return Err(SchemaError::UnknownVariant),
        };
        r.finish()?;
        Ok(out)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Closed pre-dispatch error
// ─────────────────────────────────────────────────────────────────────────────

closed_enum!(
    /// The closed set of pre-dispatch errors (design §12). Every value here maps
    /// to a `NotDispatched` response and, on the client, to an `OsControlError`
    /// proving no effect occurred.
    BrokerPreDispatchError {
        AuthenticationFailed = 0 => "authentication_failed",
        BindingMismatch = 1 => "binding_mismatch",
        ReplayDetected = 2 => "replay_detected",
        Expired = 3 => "expired",
        UnsupportedVersion = 4 => "unsupported_version",
        UnsupportedOperation = 5 => "unsupported_operation",
        InvalidParameters = 6 => "invalid_parameters",
        StalePlan = 7 => "stale_plan",
        StaleTargetIdentity = 8 => "stale_target_identity",
        UnsupportedAdapter = 9 => "unsupported_adapter",
        PolkitDenied = 10 => "polkit_denied",
        TimeoutBeforeDispatch = 11 => "timeout_before_dispatch",
    }
);

// ─────────────────────────────────────────────────────────────────────────────
// Caller binding + request id
// ─────────────────────────────────────────────────────────────────────────────

/// A digest derived from the authenticated local connection's peer credentials
/// (design §12). It is **not** a self-asserted username or PID; both the KRIA
/// client transport and the broker derive it independently from the OS-provided
/// peer credentials and the connection, and a mismatch is rejected before
/// Polkit or dispatch. See [`super::caller`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerChannelBindingDigest(Digest);

impl CallerChannelBindingDigest {
    /// Wrap a pre-derived digest.
    #[must_use]
    pub fn from_digest(digest: Digest) -> Self {
        Self(digest)
    }

    /// The underlying digest.
    #[must_use]
    pub fn digest(&self) -> &Digest {
        &self.0
    }

    /// The hex representation.
    #[must_use]
    pub fn as_hex(&self) -> &str {
        self.0.as_hex()
    }
}

/// An opaque, bounded per-request identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerRequestId(String);

impl BrokerRequestId {
    /// Maximum length.
    pub const MAX_CHARS: usize = 128;

    /// Construct, bounding length and stripping control characters.
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let cleaned: String = raw
            .chars()
            .filter(|c| !c.is_control())
            .take(Self::MAX_CHARS)
            .collect();
        Self(cleaned)
    }

    /// Borrow the identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Response binding
// ─────────────────────────────────────────────────────────────────────────────

/// Every request authority/binding field the response must byte-for-byte echo
/// (design §12). The client rejects a response whose binding differs from the
/// request's before interpreting its outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerResponseBinding {
    /// Protocol version (always `1`).
    pub protocol_version: u64,
    /// Echoed request id.
    pub request_id: BrokerRequestId,
    /// Echoed caller binding.
    pub caller_binding: CallerChannelBindingDigest,
    /// Echoed grant id.
    pub grant_id: GrantId,
    /// Echoed nonce.
    pub nonce: GrantNonce,
    /// Echoed expiry.
    pub expires_at: SystemTime,
    /// Echoed action digest.
    pub action_hash: Digest,
    /// Echoed parameter digest.
    pub parameter_hash: Digest,
    /// Echoed host-target digest.
    pub target_hash: Digest,
    /// Echoed resource-set digest.
    pub resource_set_digest: Digest,
    /// Echoed audit-admission id.
    pub audit_admission_id: AuditAdmissionId,
    /// Echoed operation digest.
    pub operation_digest: Digest,
}

impl BrokerResponseBinding {
    fn to_cbor(&self) -> CborValue {
        CborValue::Map(vec![
            (CborValue::Uint(0), CborValue::Uint(self.protocol_version)),
            (
                CborValue::Uint(1),
                CborValue::Text(self.request_id.as_str().to_string()),
            ),
            (
                CborValue::Uint(2),
                CborValue::Text(self.caller_binding.as_hex().to_string()),
            ),
            (
                CborValue::Uint(3),
                CborValue::Text(self.grant_id.as_str().to_string()),
            ),
            (
                CborValue::Uint(4),
                CborValue::Text(self.nonce.as_str().to_string()),
            ),
            (
                CborValue::Uint(5),
                CborValue::Uint(system_time_to_millis(self.expires_at)),
            ),
            (CborValue::Uint(6), digest_to_cbor(&self.action_hash)),
            (CborValue::Uint(7), digest_to_cbor(&self.parameter_hash)),
            (CborValue::Uint(8), digest_to_cbor(&self.target_hash)),
            (
                CborValue::Uint(9),
                digest_to_cbor(&self.resource_set_digest),
            ),
            (
                CborValue::Uint(10),
                CborValue::Text(self.audit_admission_id.as_str().to_string()),
            ),
            (CborValue::Uint(11), digest_to_cbor(&self.operation_digest)),
        ])
    }

    fn from_cbor(value: CborValue) -> Result<Self, SchemaError> {
        let mut r = MapReader::new(value)?;
        let protocol_version = r.take_uint(0)?;
        let request_id = BrokerRequestId::new(r.take_text(1)?);
        let caller_binding =
            CallerChannelBindingDigest::from_digest(Digest::from_hex(r.take_text(2)?));
        let grant_id = GrantId::new(r.take_text(3)?);
        let nonce = GrantNonce::new(r.take_text(4)?);
        let expires_at = millis_to_system_time(r.take_uint(5)?);
        let action_hash = text_field(&mut r, 6)?;
        let parameter_hash = text_field(&mut r, 7)?;
        let target_hash = text_field(&mut r, 8)?;
        let resource_set_digest = text_field(&mut r, 9)?;
        let audit_admission_id = AuditAdmissionId::new(r.take_text(10)?);
        let operation_digest = text_field(&mut r, 11)?;
        r.finish()?;
        Ok(Self {
            protocol_version,
            request_id,
            caller_binding,
            grant_id,
            nonce,
            expires_at,
            action_hash,
            parameter_hash,
            target_hash,
            resource_set_digest,
            audit_admission_id,
            operation_digest,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Request
// ─────────────────────────────────────────────────────────────────────────────

/// A fully authority-bound broker request (design §12). Every field binds the
/// caller, grant, action, parameter, host-target, resource-set, audit-admission,
/// operation, nonce, and expiry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerRequestV1 {
    /// Opaque per-request id.
    pub request_id: BrokerRequestId,
    /// Caller channel binding.
    pub caller_binding: CallerChannelBindingDigest,
    /// The closed operation.
    pub operation: BrokerOperation,
    /// Bound grant id.
    pub grant_id: GrantId,
    /// Bound action digest.
    pub action_hash: Digest,
    /// Bound parameter digest.
    pub parameter_hash: Digest,
    /// Bound host-target digest.
    pub target_hash: Digest,
    /// Bound resource-set digest.
    pub resource_set_digest: Digest,
    /// Bound audit-admission id.
    pub audit_admission_id: AuditAdmissionId,
    /// Bound operation digest.
    pub operation_digest: Digest,
    /// Single-use grant nonce.
    pub nonce: GrantNonce,
    /// Broker-enforced expiry (no greater than the grant deadline).
    pub expires_at: SystemTime,
}

impl BrokerRequestV1 {
    /// The response binding this request expects echoed back byte-for-byte.
    #[must_use]
    pub fn expected_binding(&self) -> BrokerResponseBinding {
        BrokerResponseBinding {
            protocol_version: PROTOCOL_VERSION,
            request_id: self.request_id.clone(),
            caller_binding: self.caller_binding.clone(),
            grant_id: self.grant_id.clone(),
            nonce: self.nonce.clone(),
            expires_at: self.expires_at,
            action_hash: self.action_hash.clone(),
            parameter_hash: self.parameter_hash.clone(),
            target_hash: self.target_hash.clone(),
            resource_set_digest: self.resource_set_digest.clone(),
            audit_admission_id: self.audit_admission_id.clone(),
            operation_digest: self.operation_digest.clone(),
        }
    }

    fn to_cbor(&self) -> CborValue {
        CborValue::Map(vec![
            (CborValue::Uint(0), CborValue::Uint(PROTOCOL_VERSION)),
            (
                CborValue::Uint(1),
                CborValue::Text(self.request_id.as_str().to_string()),
            ),
            (
                CborValue::Uint(2),
                CborValue::Text(self.caller_binding.as_hex().to_string()),
            ),
            (CborValue::Uint(3), self.operation.to_cbor()),
            (
                CborValue::Uint(4),
                CborValue::Text(self.grant_id.as_str().to_string()),
            ),
            (CborValue::Uint(5), digest_to_cbor(&self.action_hash)),
            (CborValue::Uint(6), digest_to_cbor(&self.parameter_hash)),
            (CborValue::Uint(7), digest_to_cbor(&self.target_hash)),
            (
                CborValue::Uint(8),
                digest_to_cbor(&self.resource_set_digest),
            ),
            (
                CborValue::Uint(9),
                CborValue::Text(self.audit_admission_id.as_str().to_string()),
            ),
            (CborValue::Uint(10), digest_to_cbor(&self.operation_digest)),
            (
                CborValue::Uint(11),
                CborValue::Text(self.nonce.as_str().to_string()),
            ),
            (
                CborValue::Uint(12),
                CborValue::Uint(system_time_to_millis(self.expires_at)),
            ),
        ])
    }

    /// Encode to a canonical, length-prefixed frame.
    pub fn encode_frame(&self) -> Result<Vec<u8>, CborError> {
        frame(&self.to_cbor().to_canonical_bytes())
    }

    /// Decode a fully valid request from canonical CBOR bytes (no framing).
    /// Used by round-trip golden tests; the broker uses [`Self::decode_frame`].
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, RequestDecodeError> {
        let value = decode_canonical(bytes)
            .map_err(|e| RequestDecodeError::Structural(StructuralReason::Codec(e)))?;
        Self::from_parsed(value)
    }

    /// Decode a request from a length-prefixed frame.
    pub fn decode_frame(frame_bytes: &[u8]) -> Result<Self, RequestDecodeError> {
        let body = unframe(frame_bytes)
            .map_err(|e| RequestDecodeError::Structural(StructuralReason::Codec(e)))?;
        Self::from_canonical_bytes(body)
    }

    fn from_parsed(value: CborValue) -> Result<Self, RequestDecodeError> {
        let mut r = MapReader::new(value)
            .map_err(|e| RequestDecodeError::Structural(StructuralReason::Schema(e)))?;
        let structural =
            |e: SchemaError| RequestDecodeError::Structural(StructuralReason::Schema(e));

        let version = r.take_uint(0).map_err(structural)?;
        let request_id = BrokerRequestId::new(r.take_text(1).map_err(structural)?);
        let caller_binding = CallerChannelBindingDigest::from_digest(Digest::from_hex(
            r.take_text(2).map_err(structural)?,
        ));
        let op_value = r.take(3).map_err(structural)?;
        let grant_id = GrantId::new(r.take_text(4).map_err(structural)?);
        let action_hash = text_field(&mut r, 5).map_err(structural)?;
        let parameter_hash = text_field(&mut r, 6).map_err(structural)?;
        let target_hash = text_field(&mut r, 7).map_err(structural)?;
        let resource_set_digest = text_field(&mut r, 8).map_err(structural)?;
        let audit_admission_id = AuditAdmissionId::new(r.take_text(9).map_err(structural)?);
        let operation_digest = text_field(&mut r, 10).map_err(structural)?;
        let nonce = GrantNonce::new(r.take_text(11).map_err(structural)?);
        let expires_at = millis_to_system_time(r.take_uint(12).map_err(structural)?);
        r.finish().map_err(structural)?;

        // Binding fields all parsed; build the binding for any bound rejection.
        let binding = BrokerResponseBinding {
            protocol_version: PROTOCOL_VERSION,
            request_id: request_id.clone(),
            caller_binding: caller_binding.clone(),
            grant_id: grant_id.clone(),
            nonce: nonce.clone(),
            expires_at,
            action_hash: action_hash.clone(),
            parameter_hash: parameter_hash.clone(),
            target_hash: target_hash.clone(),
            resource_set_digest: resource_set_digest.clone(),
            audit_admission_id: audit_admission_id.clone(),
            operation_digest: operation_digest.clone(),
        };

        if version != PROTOCOL_VERSION {
            return Err(RequestDecodeError::BoundRejection {
                binding: Box::new(binding),
                error: BrokerPreDispatchError::UnsupportedVersion,
            });
        }

        let operation = match BrokerOperation::from_cbor(op_value) {
            Ok(op) => op,
            Err(OperationDecodeError::UnknownOperation) => {
                return Err(RequestDecodeError::BoundRejection {
                    binding: Box::new(binding),
                    error: BrokerPreDispatchError::UnsupportedOperation,
                })
            }
            Err(OperationDecodeError::Invalid(_)) => {
                return Err(RequestDecodeError::BoundRejection {
                    binding: Box::new(binding),
                    error: BrokerPreDispatchError::InvalidParameters,
                })
            }
        };

        Ok(Self {
            request_id,
            caller_binding,
            operation,
            grant_id,
            action_hash,
            parameter_hash,
            target_hash,
            resource_set_digest,
            audit_admission_id,
            operation_digest,
            nonce,
            expires_at,
        })
    }
}

/// A structural (unbindable) decode reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuralReason {
    /// A framing / canonical-CBOR failure.
    Codec(CborError),
    /// A schema failure (missing field, wrong type, unknown key).
    Schema(SchemaError),
}

/// The outcome of decoding a request frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestDecodeError {
    /// No trustworthy binding could be recovered; the frame is rejected at the
    /// transport level and the client maps it to a pre-dispatch error.
    Structural(StructuralReason),
    /// Enough was parsed to bind a response; the broker returns a bound
    /// `NotDispatched` with the given pre-dispatch error.
    BoundRejection {
        /// The binding to echo.
        binding: Box<BrokerResponseBinding>,
        /// The bound pre-dispatch error.
        error: BrokerPreDispatchError,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// Response
// ─────────────────────────────────────────────────────────────────────────────

/// The broker's response (design §12). `NotDispatched` proves no effect; once
/// dispatch may have occurred the broker returns only `Dispatched`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerResponseV1 {
    /// No dispatch occurred; carries the echoed binding and a pre-dispatch error.
    NotDispatched {
        /// The echoed binding.
        binding: BrokerResponseBinding,
        /// The pre-dispatch error.
        error: BrokerPreDispatchError,
    },
    /// Dispatch may have occurred; carries the echoed binding and an outcome.
    Dispatched {
        /// The echoed binding.
        binding: BrokerResponseBinding,
        /// The effect-aware outcome.
        outcome: BrokerDispatchOutcome,
    },
}

impl BrokerResponseV1 {
    /// The echoed binding, regardless of family.
    #[must_use]
    pub fn binding(&self) -> &BrokerResponseBinding {
        match self {
            BrokerResponseV1::NotDispatched { binding, .. }
            | BrokerResponseV1::Dispatched { binding, .. } => binding,
        }
    }

    fn to_cbor(&self) -> CborValue {
        match self {
            BrokerResponseV1::NotDispatched { binding, error } => CborValue::Map(vec![
                (CborValue::Uint(0), CborValue::Uint(0)),
                (CborValue::Uint(1), binding.to_cbor()),
                (CborValue::Uint(2), CborValue::Uint(error.tag())),
            ]),
            BrokerResponseV1::Dispatched { binding, outcome } => CborValue::Map(vec![
                (CborValue::Uint(0), CborValue::Uint(1)),
                (CborValue::Uint(1), binding.to_cbor()),
                (CborValue::Uint(2), outcome.to_cbor()),
            ]),
        }
    }

    /// Encode to a canonical, length-prefixed frame.
    pub fn encode_frame(&self) -> Result<Vec<u8>, CborError> {
        frame(&self.to_cbor().to_canonical_bytes())
    }

    /// Decode from canonical CBOR bytes (no framing).
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, ResponseDecodeError> {
        let value = decode_canonical(bytes).map_err(ResponseDecodeError::Codec)?;
        Self::from_cbor(value).map_err(ResponseDecodeError::Schema)
    }

    /// Decode from a length-prefixed frame.
    pub fn decode_frame(frame_bytes: &[u8]) -> Result<Self, ResponseDecodeError> {
        let body = unframe(frame_bytes).map_err(ResponseDecodeError::Codec)?;
        Self::from_canonical_bytes(body)
    }

    fn from_cbor(value: CborValue) -> Result<Self, SchemaError> {
        let mut r = MapReader::new(value)?;
        let tag = r.take_uint(0)?;
        let binding = BrokerResponseBinding::from_cbor(r.take(1)?)?;
        let out = match tag {
            0 => {
                let error = BrokerPreDispatchError::from_tag(r.take_uint(2)?)?;
                BrokerResponseV1::NotDispatched { binding, error }
            }
            1 => {
                let outcome = BrokerDispatchOutcome::from_cbor(r.take(2)?)?;
                BrokerResponseV1::Dispatched { binding, outcome }
            }
            _ => return Err(SchemaError::UnknownVariant),
        };
        r.finish()?;
        Ok(out)
    }
}

/// A response decode failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseDecodeError {
    /// A framing / canonical-CBOR failure.
    Codec(CborError),
    /// A schema failure.
    Schema(SchemaError),
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    fn digest(s: &str) -> Digest {
        Digest::of_str(s)
    }

    fn sample_op() -> BrokerOperation {
        BrokerOperation::SetFirewallEnabled {
            provider: FirewallProviderId::Ufw,
            enabled: true,
        }
    }

    fn sample_request(op: BrokerOperation) -> BrokerRequestV1 {
        BrokerRequestV1 {
            request_id: BrokerRequestId::new("req-1"),
            caller_binding: CallerChannelBindingDigest::from_digest(digest("caller")),
            operation: op,
            grant_id: GrantId::new("grant-1"),
            action_hash: digest("action"),
            parameter_hash: digest("params"),
            target_hash: digest("host"),
            resource_set_digest: digest("resources"),
            audit_admission_id: AuditAdmissionId::new("adm-1"),
            operation_digest: digest("operation"),
            nonce: GrantNonce::new("nonce-1"),
            expires_at: UNIX_EPOCH + Duration::from_secs(1_000_000),
        }
    }

    fn all_sample_operations() -> Vec<BrokerOperation> {
        vec![
            BrokerOperation::ApplyPackagePlan {
                provider: PackageProviderId::Apt,
                approved_plan_digest: digest("plan"),
                transaction: BoundedPackageTransaction::new(NonEmptyBoundedVec::single(
                    PackageStep {
                        action: PackageStepAction::Install,
                        package: BoundedPackageName::new("ripgrep").unwrap(),
                    },
                )),
            },
            BrokerOperation::SetBoundPathOwnership {
                path: BrokerBoundPath {
                    path: "/etc/example.conf".into(),
                    device: 42,
                    inode: 99,
                    owner_uid: 1000,
                },
                owner: ExistingLocalIdentity {
                    uid: 1000,
                    name: SafeText::new("obaid"),
                },
            },
            BrokerOperation::SetFirewallEnabled {
                provider: FirewallProviderId::Firewalld,
                enabled: false,
            },
            BrokerOperation::SetPrivacyControl {
                control: RecognizedPrivacyControl::MicrophoneAccess,
                enabled: true,
            },
            BrokerOperation::ConfigureDiscoveredPrinter {
                printer: DiscoveredPrinterId::new("printer-xyz").unwrap(),
                options: ReviewedPrinterOptions {
                    set_default: true,
                    shared: false,
                    accept_jobs: true,
                },
            },
            BrokerOperation::SetBatteryChargeThresholds {
                adapter: ChargeThresholdAdapterId::ThinkpadAcpi,
                lower_percent: BoundedPercent::new(40).unwrap(),
                upper_percent: BoundedPercent::new(80).unwrap(),
            },
        ]
    }

    #[test]
    fn every_operation_round_trips_and_is_deterministic() {
        for op in all_sample_operations() {
            let req = sample_request(op.clone());
            let frame1 = req.encode_frame().expect("encode");
            let frame2 = req.encode_frame().expect("encode again");
            assert_eq!(frame1, frame2, "encoding must be deterministic (golden)");
            let decoded = BrokerRequestV1::decode_frame(&frame1).expect("decode");
            assert_eq!(decoded, req, "round-trip must preserve the request");
            assert_eq!(decoded.operation, op);
        }
    }

    #[test]
    fn schema_closure_enumerates_exactly_six_operations() {
        assert_eq!(BrokerOperation::COUNT, 6);
        // Tags 0..=5 decode; tag 6 is UnknownOperation.
        for op in all_sample_operations() {
            assert!(op.tag() < 6);
        }
        let mut tags: Vec<u64> = all_sample_operations()
            .iter()
            .map(BrokerOperation::tag)
            .collect();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn unknown_operation_tag_is_a_bound_unsupported_operation() {
        let req = sample_request(sample_op());
        // Replace the operation sub-map's tag with an out-of-range value.
        let mut value = req.to_cbor();
        if let CborValue::Map(entries) = &mut value {
            for (k, v) in entries.iter_mut() {
                if *k == CborValue::Uint(3) {
                    // operation map: bump tag key 0 to 99 and drop the extra
                    // params so only the tag remains.
                    *v = CborValue::Map(vec![(CborValue::Uint(0), CborValue::Uint(99))]);
                }
            }
        }
        let bytes = value.to_canonical_bytes();
        let err = BrokerRequestV1::from_canonical_bytes(&bytes).unwrap_err();
        assert!(matches!(
            err,
            RequestDecodeError::BoundRejection {
                error: BrokerPreDispatchError::UnsupportedOperation,
                ..
            }
        ));
    }

    #[test]
    fn unknown_top_level_key_is_structural() {
        let req = sample_request(sample_op());
        let mut value = req.to_cbor();
        if let CborValue::Map(entries) = &mut value {
            entries.push((CborValue::Uint(99), CborValue::Uint(0)));
        }
        let bytes = value.to_canonical_bytes();
        let err = BrokerRequestV1::from_canonical_bytes(&bytes).unwrap_err();
        assert!(matches!(
            err,
            RequestDecodeError::Structural(StructuralReason::Schema(SchemaError::UnknownKey))
        ));
    }

    #[test]
    fn follow_symlinks_true_is_rejected_as_invalid_parameters() {
        let req = sample_request(BrokerOperation::SetBoundPathOwnership {
            path: BrokerBoundPath {
                path: "/etc/x".into(),
                device: 1,
                inode: 2,
                owner_uid: 0,
            },
            owner: ExistingLocalIdentity {
                uid: 0,
                name: SafeText::new("root"),
            },
        });
        let mut value = req.to_cbor();
        if let CborValue::Map(entries) = &mut value {
            for (k, v) in entries.iter_mut() {
                if *k == CborValue::Uint(3) {
                    if let CborValue::Map(op_entries) = v {
                        for (ok, ov) in op_entries.iter_mut() {
                            if *ok == CborValue::Uint(3) {
                                *ov = CborValue::Bool(true);
                            }
                        }
                    }
                }
            }
        }
        let bytes = value.to_canonical_bytes();
        let err = BrokerRequestV1::from_canonical_bytes(&bytes).unwrap_err();
        assert!(matches!(
            err,
            RequestDecodeError::BoundRejection {
                error: BrokerPreDispatchError::InvalidParameters,
                ..
            }
        ));
    }

    #[test]
    fn extra_operation_field_cannot_encode_raw_output() {
        // A generic/raw operation field (e.g. a "command" text) has no place in
        // the closed schema: an extra operation key fails the closed-schema
        // check and maps to InvalidParameters.
        let req = sample_request(sample_op());
        let mut value = req.to_cbor();
        if let CborValue::Map(entries) = &mut value {
            for (k, v) in entries.iter_mut() {
                if *k == CborValue::Uint(3) {
                    if let CborValue::Map(op_entries) = v {
                        op_entries.push((CborValue::Uint(7), CborValue::Text("rm -rf /".into())));
                    }
                }
            }
        }
        let bytes = value.to_canonical_bytes();
        let err = BrokerRequestV1::from_canonical_bytes(&bytes).unwrap_err();
        assert!(matches!(
            err,
            RequestDecodeError::BoundRejection {
                error: BrokerPreDispatchError::InvalidParameters,
                ..
            }
        ));
    }

    #[test]
    fn bounded_percent_rejects_over_100() {
        assert!(BoundedPercent::new(101).is_err());
        assert!(BoundedPercent::new(100).is_ok());
    }

    #[test]
    fn package_name_rejects_shell_and_path_characters() {
        assert!(BoundedPackageName::new("valid-pkg.name_1:2+3").is_ok());
        for bad in ["../etc", "pkg; rm", "a b", "$(x)", ""] {
            assert!(
                BoundedPackageName::new(bad).is_err(),
                "{bad} must be rejected"
            );
        }
    }

    #[test]
    fn response_round_trips_for_every_family() {
        let binding = sample_request(sample_op()).expected_binding();
        let evidence = BoundedBrokerEvidence::new(
            ProviderId::new("ufw"),
            digest("evi"),
            [EvidenceField {
                key: SafeField::new("enabled"),
                value: SafeText::new("true"),
            }],
        );

        let responses = vec![
            BrokerResponseV1::NotDispatched {
                binding: binding.clone(),
                error: BrokerPreDispatchError::PolkitDenied,
            },
            BrokerResponseV1::Dispatched {
                binding: binding.clone(),
                outcome: BrokerDispatchOutcome::Applied {
                    receipt_digest: digest("r"),
                    evidence: evidence.clone(),
                },
            },
            BrokerResponseV1::Dispatched {
                binding: binding.clone(),
                outcome: BrokerDispatchOutcome::Uncertain {
                    receipt_digest: None,
                    cause: UncertainEffectCause::TransportLostAfterDispatch,
                    evidence: evidence.clone(),
                },
            },
            BrokerResponseV1::Dispatched {
                binding,
                outcome: BrokerDispatchOutcome::PartiallyApplied {
                    receipt_digest: Some(digest("r")),
                    completed_steps: NonEmptyBoundedVec::single(SafeStepId::new("step-1")),
                    failed_step: SafeStepId::new("step-2"),
                    cause: PartialEffectCause::StepFailedAfterCommit,
                    evidence,
                },
            },
        ];

        for resp in responses {
            let frame = resp.encode_frame().expect("encode");
            let decoded = BrokerResponseV1::decode_frame(&frame).expect("decode");
            assert_eq!(decoded, resp);
        }
    }

    #[test]
    fn pre_dispatch_error_set_is_closed_with_twelve_codes() {
        let all = BrokerPreDispatchError::all();
        assert_eq!(all.len(), 12);
        let mut tags: Vec<u64> = all.iter().map(|e| e.tag()).collect();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), 12);
        // Unknown tag rejected.
        assert!(BrokerPreDispatchError::from_tag(12).is_err());
    }
}
