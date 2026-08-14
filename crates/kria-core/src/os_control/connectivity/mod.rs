//! Connectivity domain: the `ConnectivityControl` desired-state provider
//! (design §3, §9.4).
//!
//! linux-os-control-production **Task 2.3** — "Migrate Wi-Fi and power-profile
//! controls" (OSC-015, OSC-020, OSC-025, OSC-029, OSC-031) — and **Task 3.5**
//! — "Complete Wi-Fi, Ethernet and credentials" (OSC-015, OSC-025, OSC-029).
//!
//! This module replaces the direct `nmcli` subprocess handling that used to
//! live in `tools/system_config.rs` for `get_wifi_networks`, `toggle_wifi`, and
//! `connect_wifi` (Task 2.3), and adds `disconnect_wifi`, `forget_wifi`, and
//! `activate_network_profile` (Task 3.5) over the same governed pipeline. It
//! composes the F1 runtime, mirroring `os_control::audio`'s shape:
//!
//! * [`ConnectivityState`] is a normalized observation
//!   ([`NormalizedObservation`]) whose digest is focused on exactly one
//!   dimension per [`ConnectivityFocus`] (radio, active SSID, device-connected,
//!   profile-saved, or active-profile), so no two mutations' idempotency/
//!   verification cross-contaminate.
//! * [`ConnectivityControl`] implements the generic [`DesiredStateControl`]
//!   lifecycle (observe → apply → verify → rollback) for every mutating
//!   connectivity tool. Its `apply`/`rollback` build a governed
//!   [`StructuredCommandRequest`] from the borrowed [`AdmittedMutationContext`]
//!   — the only sanctioned path to a child process — so no connectivity code
//!   touches `ExecWrapper`/`tokio::process` directly.
//! * `get_wifi_networks` is a pure read (`scan_wifi`) and is not part of the
//!   `DesiredStateControl` mutation lifecycle — its output shape (a bounded
//!   list) does not fit a single comparable "state", and the frozen manifest
//!   marks it `verificationClass: None` (no postcondition).
//! * The live transport ([`crate::os_control::linux::providers::network_manager`])
//!   is a raw, deny-live-gated adapter; deny-live tests inject
//!   [`FakeConnectivityTransport`].
//!
//! # Ethernet (OSC-015.2/.7)
//!
//! Ethernet has no separate "connect" tool. An Ethernet saved connection is
//! just another [`NetworkProfileId`], activated through the same
//! `activate_network_profile` / [`ConnectivityOp::ActivateProfile`] path a
//! Wi-Fi saved profile uses. There is no static IP/DNS/route/bridge editing
//! anywhere in this module (OSC-015.7) — only activating an *existing* saved
//! profile.
//!
//! # Duplicate SSID / device / profile clarification (OSC-015.6)
//!
//! Two access points can legitimately advertise the same SSID; two saved
//! profiles can legitimately share a display name. Nothing in this module
//! silently picks one: [`ConnectivityControl::apply`] scans the relevant
//! candidate set and, when it observes more than one distinct stable identity
//! matching the target, returns [`OsControlError::AmbiguousTarget`] — a
//! distinct error shape, never a fabricated single-candidate pick. Connecting/
//! activating an already-desired target is `Unchanged` before any such scan
//! runs (idempotency short circuits in `OsControlRuntime::run_mutation`).
//!
//! # Rollback (OSC-015.5, design §13.1)
//!
//! `toggle_wifi` and `activate_network_profile` are `RollbackClaim::
//! UserRequestable`: [`ConnectivityControl::apply`] captures the prior state
//! (radio / active profile per device) before dispatch, so a later
//! contradiction or explicit rollback call can reactivate it.  `connect_wifi`,
//! `disconnect_wifi`, and `forget_wifi` are `RollbackClaim::None` — connecting/
//! disconnecting/forgetting has no reliably restorable positive inverse
//! distinct from another typed mutation, and a forgotten profile's saved
//! configuration is not reconstructible, so no receipt for these three ever
//! claims rollback availability.
//!
//! # Secret handling (OSC-025, OSC-029)
//!
//! `connect_wifi`'s `credential` parameter is the frozen manifest's typed
//! `SecretRef`. The tool facade resolves it through
//! [`crate::os_control::secrets::CredentialStore::resolve_for_operation`]
//! under the admitted mutation context (bound to
//! [`crate::os_control::secrets::SecretPurpose::WifiPassword`] and a
//! profile/SSID-scoped [`crate::os_control::secrets::SecretScope`]) **before**
//! building [`ConnectWifiOp`] — the resolved bytes flow into this module only
//! as the existing ephemeral [`crate::os_control::secrets::SecretPayload`]
//! carrier. `SecretPayload` does not implement `Serialize`/`Display`/`Clone`,
//! so it cannot enter a plan, log, trace, or audit record. The password is
//! used solely to build the literal (non-shell) `nmcli` argv element for this
//! one dispatch, and that argv position is marked secret in the request's
//! [`RedactionMap`] so a captured
//! [`crate::os_control::linux::structured_command::StructuredCommandSummary`]
//! replaces it with the fixed redaction placeholder.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::SystemTime;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    BoundedVec, CapabilityId, ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource,
    ProviderId, SafeCandidate, SafeErrorCode, SafeField, SafeText, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{
    CommandPlan, CommandPolicy, RedactionMap, SecretStdin, StructuredCommandRequest,
    TrustedExecutable,
};
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification, UncertainDispatch,
    UncertainEffectCause, VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;
use crate::os_control::secrets::{
    SecretPayload, SecretPurpose, SecretRef, SecretResolutionRequest, SecretScope,
};

pub mod parsers;
pub mod selection;

/// Deny-live fake transport (Task 0.4 / OSC-033); test composition only.
#[cfg(feature = "os-control-test")]
pub mod fake;


pub use parsers::{
    HostConnectivity, RawActiveConnection, RawDeviceIpFacts, RawNetworkDevice, RawNetworkProfile,
    RawWifiNetwork,
};
pub use selection::{ConnectivityBackend, ProxyBackend};

// ─────────────────────────────────────────────────────────────────────────────
// Typed identities (Task 3.5, OSC-015.6) — never a raw D-Bus object path
// ─────────────────────────────────────────────────────────────────────────────

/// Maximum length (chars) of a [`NetworkDeviceId`]/[`NetworkProfileId`]/
/// [`WifiNetworkId`].
const NETWORK_ID_MAX_CHARS: usize = 128;

fn sanitize_network_id(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(NETWORK_ID_MAX_CHARS));
    for ch in raw.chars() {
        if out.chars().count() >= NETWORK_ID_MAX_CHARS {
            break;
        }
        if !ch.is_control() {
            out.push(ch);
        }
    }
    out
}

macro_rules! network_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// Construct from a raw identity string (bounded, control-char-free).
            #[must_use]
            pub fn new(raw: impl Into<String>) -> Self {
                Self(sanitize_network_id(&raw.into()))
            }

            /// Borrow the identity.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consume into the owned identity string.
            #[must_use]
            pub fn into_string(self) -> String {
                self.0
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

network_id!(
    /// A stable, typed network-device identity (a NetworkManager device — Wi-Fi
    /// adapter or Ethernet NIC), derived from its D-Bus object path / `nmcli`
    /// device name — never a raw object-path string surfaced verbatim.
    NetworkDeviceId
);
network_id!(
    /// A stable, typed network-profile identity (a saved NetworkManager
    /// connection — Wi-Fi or Ethernet), derived from its D-Bus object path /
    /// connection UUID.
    NetworkProfileId
);
network_id!(
    /// A stable, typed identity for one scanned Wi-Fi access point (from
    /// `get_wifi_networks`), distinct from [`NetworkProfileId`] so a fresh scan
    /// result and a saved profile are never confused.
    WifiNetworkId
);

/// The device kind a [`RawNetworkDevice`]/[`RawNetworkProfile`] belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkDeviceKind {
    /// A Wi-Fi radio adapter / Wi-Fi saved profile.
    Wifi,
    /// A wired Ethernet NIC / Ethernet saved profile.
    Ethernet,
    /// Any other device/profile type (not exposed for typed mutation).
    Other,
}

/// Which dimension of connectivity state a request compares against, so the
/// idempotency/verification comparator only considers the field the operation
/// changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectivityFocus {
    /// Compare the Wi-Fi radio enabled/disabled state.
    Radio,
    /// Compare the currently active SSID (`connect_wifi`).
    Connection,
    /// Compare whether a specific device is connected (`disconnect_wifi`).
    Device,
    /// Compare whether a specific profile is still saved (`forget_wifi`).
    ProfileSaved,
    /// Compare the active profile bound to a target
    /// (`activate_network_profile`).
    ActiveProfile,
    /// Compare whether a specific VPN profile is connected
    /// (`set_vpn_connection`, Task 4.2).
    VpnConnected,
    /// Compare whether a device is serving a hotspot (`set_hotspot`, Task 5.3).
    HotspotEnabled,
    /// Compare the desktop-wide proxy mode (`set_proxy_profile`, Task 5.3).
    ProxyMode,
    /// Compare which credential is bound to a profile
    /// (`replace_saved_connectivity_credential` /
    /// `delete_saved_connectivity_credential`, Task 5.6).
    CredentialBinding,
}

/// A normalized connectivity observation (design §5, §9.4). The digest binds
/// only the focused dimension so, e.g., a radio-toggle idempotency check is
/// never perturbed by the currently connected SSID, and vice versa.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectivityState {
    /// Whether the Wi-Fi radio is enabled.
    pub radio_enabled: bool,
    /// The currently active SSID, if connected.
    pub active_ssid: Option<String>,
    /// Whether the focused device is connected ([`ConnectivityFocus::Device`]).
    pub device_connected: bool,
    /// Whether the focused profile is still saved
    /// ([`ConnectivityFocus::ProfileSaved`]).
    pub profile_saved: bool,
    /// The active profile identity bound to the focused target
    /// ([`ConnectivityFocus::ActiveProfile`]).
    pub active_profile_id: Option<String>,
    /// Whether the focused VPN profile is connected
    /// ([`ConnectivityFocus::VpnConnected`]).
    pub vpn_connected: bool,
    /// Whether the focused device is serving a hotspot
    /// ([`ConnectivityFocus::HotspotEnabled`]).
    pub hotspot_enabled: bool,
    /// The desktop-wide proxy mode in the contract's vocabulary
    /// (`none`/`automatic`/`manual`), for [`ConnectivityFocus::ProxyMode`].
    /// `None` is never a "no proxy" default — an unreadable mode surfaces as an
    /// error before an observation is built.
    pub proxy_mode: Option<String>,
    /// The opaque identity of the credential bound to the focused profile
    /// ([`ConnectivityFocus::CredentialBinding`]). `None` means no credential is
    /// stored, which is a positively observed fact.
    pub credential_ref: Option<String>,
    /// The comparison focus for this observation.
    pub focus: ConnectivityFocus,
}

impl ConnectivityState {
    fn base(focus: ConnectivityFocus) -> Self {
        Self {
            radio_enabled: false,
            active_ssid: None,
            device_connected: false,
            profile_saved: false,
            active_profile_id: None,
            vpn_connected: false,
            hotspot_enabled: false,
            proxy_mode: None,
            credential_ref: None,
            focus,
        }
    }

    /// Construct a radio-focused observation.
    #[must_use]
    pub fn radio(enabled: bool) -> Self {
        Self {
            radio_enabled: enabled,
            ..Self::base(ConnectivityFocus::Radio)
        }
    }

    /// Construct a connection-focused observation.
    #[must_use]
    pub fn connection(active_ssid: Option<String>) -> Self {
        Self {
            active_ssid,
            ..Self::base(ConnectivityFocus::Connection)
        }
    }

    /// Construct a device-connected-focused observation (`disconnect_wifi`).
    #[must_use]
    pub fn device(connected: bool) -> Self {
        Self {
            device_connected: connected,
            ..Self::base(ConnectivityFocus::Device)
        }
    }

    /// Construct a profile-saved-focused observation (`forget_wifi`).
    #[must_use]
    pub fn profile_saved(saved: bool) -> Self {
        Self {
            profile_saved: saved,
            ..Self::base(ConnectivityFocus::ProfileSaved)
        }
    }

    /// Construct an active-profile-focused observation
    /// (`activate_network_profile`).
    #[must_use]
    pub fn active_profile(active_profile_id: Option<String>) -> Self {
        Self {
            active_profile_id,
            ..Self::base(ConnectivityFocus::ActiveProfile)
        }
    }

    /// Construct a VPN-connected-focused observation (`set_vpn_connection`).
    #[must_use]
    pub fn vpn(connected: bool) -> Self {
        Self {
            vpn_connected: connected,
            ..Self::base(ConnectivityFocus::VpnConnected)
        }
    }

    /// Construct a hotspot-focused observation (`set_hotspot`).
    #[must_use]
    pub fn hotspot(enabled: bool) -> Self {
        Self {
            hotspot_enabled: enabled,
            ..Self::base(ConnectivityFocus::HotspotEnabled)
        }
    }

    /// Construct a proxy-mode-focused observation (`set_proxy_profile`).
    ///
    /// `mode` is the contract vocabulary plus the profile identity, so switching
    /// between two different manual profiles is a real change rather than a
    /// no-op that "verifies" against the mode alone.
    #[must_use]
    pub fn proxy(mode: Option<String>) -> Self {
        Self {
            proxy_mode: mode,
            ..Self::base(ConnectivityFocus::ProxyMode)
        }
    }

    /// Construct a credential-binding-focused observation
    /// (`replace_saved_connectivity_credential` /
    /// `delete_saved_connectivity_credential`).
    #[must_use]
    pub fn credential(credential_ref: Option<String>) -> Self {
        Self {
            credential_ref,
            ..Self::base(ConnectivityFocus::CredentialBinding)
        }
    }
}

impl NormalizedObservation for ConnectivityState {
    fn observation_digest(&self) -> Digest {
        match self.focus {
            ConnectivityFocus::Radio => {
                Digest::of_str(&format!("wifi:radio:{}", self.radio_enabled))
            }
            ConnectivityFocus::Connection => Digest::of_str(&format!(
                "wifi:conn:{}",
                self.active_ssid.as_deref().unwrap_or("")
            )),
            ConnectivityFocus::Device => {
                Digest::of_str(&format!("wifi:device:{}", self.device_connected))
            }
            ConnectivityFocus::ProfileSaved => {
                Digest::of_str(&format!("wifi:profile_saved:{}", self.profile_saved))
            }
            ConnectivityFocus::ActiveProfile => Digest::of_str(&format!(
                "wifi:active_profile:{}",
                self.active_profile_id.as_deref().unwrap_or("")
            )),
            ConnectivityFocus::VpnConnected => {
                Digest::of_str(&format!("vpn:connected:{}", self.vpn_connected))
            }
            ConnectivityFocus::HotspotEnabled => {
                Digest::of_str(&format!("hotspot:enabled:{}", self.hotspot_enabled))
            }
            // `None` (unreadable/unset) must not collide with any concrete mode,
            // so it is digested as a distinct sentinel rather than an empty mode.
            ConnectivityFocus::ProxyMode => Digest::of_str(&format!(
                "proxy:mode:{}",
                self.proxy_mode.as_deref().unwrap_or("<undetermined>")
            )),
            ConnectivityFocus::CredentialBinding => Digest::of_str(&format!(
                "credential:ref:{}",
                self.credential_ref.as_deref().unwrap_or("<absent>")
            )),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// VPN / hotspot / proxy / saved-credential DTOs (Tasks 4.2, 5.3, 5.6)
// ─────────────────────────────────────────────────────────────────────────────

/// Which class of saved connectivity credential a listing is filtered to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectivityCredentialKind {
    /// A Wi-Fi profile's pre-shared key.
    Wifi,
    /// A VPN profile's stored secret.
    Vpn,
}

impl ConnectivityCredentialKind {
    /// The stable token used in the tool result.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wifi => "wifi",
            Self::Vpn => "vpn",
        }
    }

    /// Parse the frozen contract's `kind` enum token.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "wifi" => Some(Self::Wifi),
            "vpn" => Some(Self::Vpn),
            _ => None,
        }
    }

    /// The NetworkManager property that holds this class of credential.
    #[must_use]
    pub fn secret_property(self) -> &'static str {
        match self {
            Self::Wifi => selection::WIFI_PSK_PROPERTY,
            Self::Vpn => selection::VPN_SECRETS_PROPERTY,
        }
    }

    /// The credential-store purpose this class binds to, so a secret stored for
    /// one purpose can never be resolved for another.
    #[must_use]
    pub fn purpose(self) -> SecretPurpose {
        match self {
            Self::Wifi => SecretPurpose::WifiPassword,
            Self::Vpn => SecretPurpose::VpnCredential,
        }
    }

    /// The class a raw `nmcli` connection type belongs to, or `None` when the
    /// profile type holds no connectivity credential this domain manages.
    #[must_use]
    pub fn from_connection_type(connection_type: &str) -> Option<Self> {
        if parsers::is_vpn_connection_type(connection_type) {
            Some(Self::Vpn)
        } else if connection_type == "802-11-wireless" {
            Some(Self::Wifi)
        } else {
            None
        }
    }
}

/// One saved VPN profile (`list_vpn_profiles`, Task 4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VpnProfileSummary {
    /// The profile's stable UUID identity.
    pub profile: NetworkProfileId,
    /// The profile's human-visible label. Never used as an identity.
    pub label: String,
    /// Whether the profile is currently activated.
    pub connected: bool,
}

/// The observed hotspot state for a device (`get_hotspot_state`, Task 5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotspotFacts {
    /// The device the state describes, when one was resolved.
    pub device: Option<NetworkDeviceId>,
    /// Whether an access-point profile is currently activated on the device.
    pub enabled: bool,
    /// The activated access-point profile, when the hotspot is up.
    pub profile: Option<NetworkProfileId>,
}

/// One manual proxy endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyEndpoint {
    /// The proxy host.
    pub host: String,
    /// The proxy port.
    pub port: u16,
}

/// A recognized desktop proxy profile, matching the frozen contract's
/// `RecognizedProxyProfile` one-of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyProfile {
    /// PAC-based automatic configuration.
    Automatic {
        /// The PAC script URI (`http://` or `https://`).
        pac_uri: String,
    },
    /// Explicit per-protocol endpoints.
    Manual {
        /// The HTTP proxy endpoint, when configured.
        http: Option<ProxyEndpoint>,
        /// The HTTPS proxy endpoint, when configured.
        https: Option<ProxyEndpoint>,
        /// The SOCKS proxy endpoint, when configured.
        socks: Option<ProxyEndpoint>,
        /// Hosts excluded from proxying.
        exclusions: Vec<String>,
    },
}

/// Max number of proxy exclusions (`x-configBound: proxy_exclusion_count`).
pub const MAX_PROXY_EXCLUSIONS: usize = 64;

impl ProxyProfile {
    /// The contract mode token this profile belongs to.
    #[must_use]
    pub fn mode(&self) -> &'static str {
        match self {
            Self::Automatic { .. } => "automatic",
            Self::Manual { .. } => "manual",
        }
    }

    /// A stable, order-independent digest of the profile's content. Surfaced as
    /// `profile_digest` so a proxy target can be correlated without publishing
    /// the endpoints themselves.
    #[must_use]
    pub fn digest(&self) -> Digest {
        Digest::of_str(&self.canonical_token())
    }

    /// The canonical, comparison-stable rendering of this profile.
    #[must_use]
    fn canonical_token(&self) -> String {
        match self {
            Self::Automatic { pac_uri } => format!("automatic|pac={pac_uri}"),
            Self::Manual {
                http,
                https,
                socks,
                exclusions,
            } => {
                let render = |label: &str, endpoint: &Option<ProxyEndpoint>| match endpoint {
                    Some(e) => format!("{label}={}:{}", e.host, e.port),
                    None => format!("{label}="),
                };
                let mut sorted = exclusions.clone();
                sorted.sort();
                format!(
                    "manual|{}|{}|{}|exclude={}",
                    render("http", http),
                    render("https", https),
                    render("socks", socks),
                    sorted.join(",")
                )
            }
        }
    }

    /// The `effective` field: which concrete proxy channel actually applies.
    #[must_use]
    pub fn effective(&self) -> String {
        match self {
            Self::Automatic { .. } => "automatic:pac".to_string(),
            Self::Manual {
                http,
                https,
                socks,
                ..
            } => {
                let mut channels = Vec::new();
                if http.is_some() {
                    channels.push("http");
                }
                if https.is_some() {
                    channels.push("https");
                }
                if socks.is_some() {
                    channels.push("socks");
                }
                if channels.is_empty() {
                    // Mode says manual but no endpoint is set: report the
                    // contradiction rather than implying traffic is proxied.
                    "manual:none".to_string()
                } else {
                    format!("manual:{}", channels.join("+"))
                }
            }
        }
    }

    /// Validate the profile against the frozen contract's bounds. Rejects rather
    /// than normalises, because a silently-widened proxy target would send every
    /// application's traffic somewhere the caller did not ask for.
    pub fn validate(&self) -> Result<(), OsControlError> {
        let reject = |field: &str, reason: &str| OsControlError::InvalidRequest {
            field: SafeField::new(field),
            reason: SafeText::new(reason),
        };
        match self {
            Self::Automatic { pac_uri } => {
                if !(pac_uri.starts_with("http://") || pac_uri.starts_with("https://")) {
                    return Err(reject("profile.pac_uri", "must start with http:// or https://"));
                }
                if pac_uri.len() < 8 || pac_uri.len() > 2048 {
                    return Err(reject("profile.pac_uri", "must be 8..=2048 bytes"));
                }
                if pac_uri.chars().any(char::is_control) {
                    return Err(reject(
                        "profile.pac_uri",
                        "must not contain control characters",
                    ));
                }
                Ok(())
            }
            Self::Manual {
                http,
                https,
                socks,
                exclusions,
            } => {
                if http.is_none() && https.is_none() && socks.is_none() && exclusions.is_empty() {
                    return Err(reject(
                        "profile",
                        "a manual proxy profile must set at least one endpoint or exclusion",
                    ));
                }
                for (label, endpoint) in [("http", http), ("https", https), ("socks", socks)] {
                    if let Some(endpoint) = endpoint {
                        if endpoint.port == 0 {
                            return Err(reject(
                                "profile.port",
                                "proxy port must be in 1..=65535",
                            ));
                        }
                        validate_proxy_host(label, &endpoint.host)?;
                    }
                }
                if exclusions.len() > MAX_PROXY_EXCLUSIONS {
                    return Err(reject(
                        "profile.exclusions",
                        "too many proxy exclusions",
                    ));
                }
                for exclusion in exclusions {
                    let bare = exclusion.strip_prefix("*.").unwrap_or(exclusion);
                    validate_proxy_host("profile.exclusions", bare)?;
                }
                Ok(())
            }
        }
    }
}

/// Validate a proxy host against the contract's host pattern. Rejects an empty
/// host, a host with a scheme/port/path, and any control character.
fn validate_proxy_host(field: &str, host: &str) -> Result<(), OsControlError> {
    let reject = |reason: &str| OsControlError::InvalidRequest {
        field: SafeField::new(field),
        reason: SafeText::new(reason),
    };
    if host.is_empty() || host.len() > 253 {
        return Err(reject("proxy host must be 1..=253 bytes"));
    }
    let first = host.chars().next().unwrap_or('-');
    let last = host.chars().last().unwrap_or('-');
    if !first.is_ascii_alphanumeric() || !last.is_ascii_alphanumeric() {
        return Err(reject(
            "proxy host must start and end with an alphanumeric character",
        ));
    }
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return Err(reject(
            "proxy host must contain only letters, digits, '.' and '-' (no scheme, port or path)",
        ));
    }
    Ok(())
}

/// The observed desktop proxy state (`get_proxy_state`, Task 5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyFacts {
    /// The mode in the contract vocabulary (`none`/`automatic`/`manual`).
    pub mode: String,
    /// The configured profile, when the mode is not `none`.
    pub profile: Option<ProxyProfile>,
}

impl ProxyFacts {
    /// The comparison token used as the proxy observation's focused value: mode
    /// **plus** profile identity, so switching between two manual profiles is a
    /// real change rather than a mode-only no-op.
    #[must_use]
    pub fn comparison_token(&self) -> String {
        proxy_comparison_token(&self.mode, self.profile.as_ref())
    }

    /// The `effective` field: what actually applies to application traffic.
    #[must_use]
    pub fn effective(&self) -> String {
        match &self.profile {
            Some(profile) => profile.effective(),
            None => "none".to_string(),
        }
    }
}

/// The focused comparison value for a proxy observation or desired state.
///
/// Binding the profile digest into the token (not just the mode) is what makes
/// "switch from one manual proxy to a different manual proxy" a real change
/// instead of an already-satisfied no-op.
#[must_use]
pub fn proxy_comparison_token(mode: &str, profile: Option<&ProxyProfile>) -> String {
    match profile {
        Some(profile) => format!("{mode}#{}", profile.digest().as_hex()),
        None => format!("{mode}#none"),
    }
}

/// A layered network diagnosis (`diagnose_network`, Task 4.2).
///
/// Every field is a discrete verdict token, never a raw address, and
/// `undetermined` is always distinct from a negative finding: reporting "no
/// internet" when the check itself could not decide is worse than reporting that
/// it could not decide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkDiagnosisFacts {
    /// `up` / `down` / `undetermined`.
    pub link: &'static str,
    /// `assigned` / `absent` / `undetermined`.
    pub address: &'static str,
    /// `default_present` / `absent` / `undetermined`.
    pub route: &'static str,
    /// `present` / `absent` / `undetermined`.
    pub gateway: &'static str,
    /// `configured` / `absent` / `undetermined`.
    pub dns: &'static str,
    /// `reachable` / `limited` / `captive_portal` / `unreachable` /
    /// `undetermined`.
    pub internet: &'static str,
    /// `detected` / `not_detected` / `undetermined`.
    pub captive_portal: &'static str,
    /// Set when the caller named a target this provider cannot probe, so the
    /// diagnosis is reported as `Degraded` rather than silently host-scoped.
    pub target_probe_unavailable: bool,
}

/// Value-free metadata about one saved connectivity credential
/// (`list_saved_connectivity_credentials`, Task 5.6). It carries **no**
/// credential value — only the profile identity, the class, and an opaque
/// reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedCredentialSummary {
    /// The owning profile's stable UUID identity.
    pub profile: NetworkProfileId,
    /// The profile's human-visible label. Never an identity.
    pub label: String,
    /// Which credential class this is.
    pub kind: ConnectivityCredentialKind,
    /// An opaque, value-free reference to the stored credential.
    pub secret_ref: String,
}

/// The opaque reference identifying the credential stored on a profile.
///
/// Derived from the profile UUID alone, so it is stable and contains no part of
/// the credential. It identifies *that a credential is bound here*, never which
/// value is bound.
#[must_use]
pub fn credential_reference(profile: &NetworkProfileId) -> String {
    format!("nm:{}", profile.as_str())
}

/// The desired credential binding after a replacement.
///
/// The `#<digest>` suffix names *which* credential must be bound. An observation
/// can only ever report presence ([`credential_reference`], with no suffix)
/// because reading the stored value back would disclose it — so a replacement is
/// never mistaken for an already-satisfied state, and its verification is
/// honestly inconclusive rather than falsely `Verified`.
#[must_use]
pub fn credential_replacement_reference(
    profile: &NetworkProfileId,
    credential: &SecretRef,
) -> String {
    format!(
        "{}#{}",
        credential_reference(profile),
        credential.digest().as_hex()
    )
}

/// The `connect_wifi` operation's parameters. Carries the SSID plus an optional
/// **ephemeral** password (never a plan/DTO-serializable field — see the module
/// docs' "Secret handling" section).
pub struct ConnectWifiOp {
    /// The target network name. Not a unique identity: [`ConnectivityControl`]
    /// disambiguates duplicate SSIDs before dispatch.
    pub ssid: String,
    /// The ephemeral credential, when the network requires one and the caller
    /// supplied it directly (a raw protected-input value) rather than through
    /// a stored `Secret_Reference`. `None` for an open network, a previously-
    /// saved profile, or when `credential` is set instead.
    pub password: Option<SecretPayload>,
    /// The frozen manifest's typed `credential?:SecretRef` parameter (Task
    /// 3.5, OSC-015.3/OSC-025.3). When set (and `password` is `None`),
    /// [`ConnectivityControl::apply`] resolves it through
    /// [`crate::os_control::secrets::CredentialStore::resolve_for_operation`]
    /// under the admitted mutation context — scoped to
    /// [`crate::os_control::secrets::SecretPurpose::WifiPassword`] and this
    /// SSID — before building the governed dispatch. The resolved bytes are
    /// used only for this one dispatch and are never copied into `password`,
    /// a log, or an audit record.
    pub credential: Option<SecretRef>,
}

impl std::fmt::Debug for ConnectWifiOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectWifiOp")
            .field("ssid", &self.ssid)
            .field("password", &self.password.is_some())
            .field("credential", &self.credential.is_some())
            .finish()
    }
}

/// The concrete connectivity mutation this task migrates.
#[derive(Debug)]
pub enum ConnectivityOp {
    /// Enable/disable the Wi-Fi radio.
    ToggleRadio(bool),
    /// Connect to a Wi-Fi network.
    ConnectWifi(ConnectWifiOp),
    /// Disconnect a device from its current Wi-Fi connection (Task 3.5,
    /// OSC-015.2). Never claims rollback (design §13.1: `disconnect_wifi` is
    /// `RollbackClaim::None`) — disconnecting has no reliably restorable
    /// prior positive action distinct from `connect_wifi`/
    /// `activate_network_profile`.
    DisconnectWifi(NetworkDeviceId),
    /// Forget (delete) a saved Wi-Fi profile (Task 3.5, OSC-015.2). RED and
    /// irreversible (design §13.1: `RollbackClaim::None`) — the caller must
    /// explicitly confirm before this op is built (forget confirmation
    /// semantic, per the task text).
    ForgetProfile(NetworkProfileId),
    /// Activate an existing saved profile — Wi-Fi **or** Ethernet (Task 3.5,
    /// OSC-015.2/.7). Ethernet has no separate "connect" operation: it is
    /// just another [`NetworkProfileId`] activated through this same
    /// variant.
    ActivateProfile {
        /// The saved profile to activate.
        profile: NetworkProfileId,
        /// The device to activate it on, when the profile does not already
        /// bind one unambiguously.
        device: Option<NetworkDeviceId>,
    },
    /// Connect or disconnect a saved **VPN** profile (Task 4.2,
    /// `set_vpn_connection`). The profile is always addressed by UUID: two
    /// saved VPN profiles may share a display name, so a name is not an
    /// identity. `RollbackClaim::None` — the inverse is just the opposite
    /// caller-visible request.
    SetVpn {
        /// The VPN profile's UUID.
        profile: NetworkProfileId,
        /// Whether the VPN should end up connected.
        connected: bool,
    },
    /// Start or stop a Wi-Fi access point on a device (Task 5.3,
    /// `set_hotspot`). Turning the machine into an access point is RED: a
    /// hotspot with no or a sub-standard passphrase is refused outright rather
    /// than silently accepted.
    SetHotspot {
        /// The Wi-Fi device to serve the hotspot on.
        device: NetworkDeviceId,
        /// Whether the hotspot should end up running.
        enabled: bool,
        /// The access-point profile to activate. When absent, the device's own
        /// unique access-point profile is resolved; an ambiguous or missing one
        /// fails closed rather than creating a new open network.
        profile: Option<NetworkProfileId>,
        /// An optional stored credential supplied for this activation. Resolved
        /// under the admitted mutation context and delivered on the child's
        /// **stdin**; it never becomes an argv element.
        credential: Option<SecretRef>,
    },
    /// Set the desktop-wide proxy configuration (Task 5.3,
    /// `set_proxy_profile`). This redirects **every** application's traffic, so
    /// an unreadable current state fails closed and the mode key is written
    /// last, after the endpoints it depends on.
    SetProxy {
        /// The contract mode (`none` / `automatic` / `manual`).
        mode: String,
        /// The profile for a non-`none` mode.
        profile: Option<ProxyProfile>,
    },
    /// Replace the credential stored on a saved profile (Task 5.6,
    /// `replace_saved_connectivity_credential`). The new value is resolved from
    /// the credential store and written on the child's **stdin**; it never
    /// reaches argv, a log, or the audit record. `RollbackClaim::CompensationOnly`
    /// — the previous value is not recoverable, so no receipt claims an inverse.
    ReplaceCredential {
        /// The owning profile's UUID.
        profile: NetworkProfileId,
        /// The replacement credential's opaque reference.
        credential: SecretRef,
    },
    /// Delete the credential stored on a saved profile (Task 5.6,
    /// `delete_saved_connectivity_credential`). The profile itself survives;
    /// only its stored secret is cleared, which is directly observable without
    /// ever reading the value.
    DeleteCredential {
        /// The owning profile's UUID.
        profile: NetworkProfileId,
    },
}

/// A fully-described connectivity request. Carries the canonical `action`/
/// `params` so the governed [`StructuredCommandRequest`] can bind them against
/// the grant.
#[derive(Debug)]
pub struct ConnectivityRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params digest).
    /// **Never** includes the raw password (OSC-025.4) — only the SSID and any
    /// non-secret fields the canonical schema declares.
    pub params: serde_json::Value,
    /// The concrete operation.
    pub op: ConnectivityOp,
}

impl ConnectivityRequest {
    /// The comparison focus implied by the operation.
    #[must_use]
    pub fn focus(&self) -> ConnectivityFocus {
        match &self.op {
            ConnectivityOp::ToggleRadio(_) => ConnectivityFocus::Radio,
            ConnectivityOp::ConnectWifi(_) => ConnectivityFocus::Connection,
            ConnectivityOp::DisconnectWifi(_) => ConnectivityFocus::Device,
            ConnectivityOp::ForgetProfile(_) => ConnectivityFocus::ProfileSaved,
            ConnectivityOp::ActivateProfile { .. } => ConnectivityFocus::ActiveProfile,
            ConnectivityOp::SetVpn { .. } => ConnectivityFocus::VpnConnected,
            ConnectivityOp::SetHotspot { .. } => ConnectivityFocus::HotspotEnabled,
            ConnectivityOp::SetProxy { .. } => ConnectivityFocus::ProxyMode,
            ConnectivityOp::ReplaceCredential { .. } | ConnectivityOp::DeleteCredential { .. } => {
                ConnectivityFocus::CredentialBinding
            }
        }
    }

    /// The desired end state for this mutation, focused on the changed
    /// dimension.
    #[must_use]
    pub fn desired_state(&self) -> ConnectivityState {
        match &self.op {
            ConnectivityOp::ToggleRadio(enabled) => ConnectivityState::radio(*enabled),
            ConnectivityOp::ConnectWifi(op) => ConnectivityState::connection(Some(op.ssid.clone())),
            // Disconnecting a device desires it to no longer be connected.
            ConnectivityOp::DisconnectWifi(_) => ConnectivityState::device(false),
            // Forgetting a profile desires it to no longer be saved.
            ConnectivityOp::ForgetProfile(_) => ConnectivityState::profile_saved(false),
            ConnectivityOp::ActivateProfile { profile, .. } => {
                ConnectivityState::active_profile(Some(profile.as_str().to_string()))
            }
            ConnectivityOp::SetVpn { connected, .. } => ConnectivityState::vpn(*connected),
            ConnectivityOp::SetHotspot { enabled, .. } => ConnectivityState::hotspot(*enabled),
            ConnectivityOp::SetProxy { mode, profile } => {
                ConnectivityState::proxy(Some(proxy_comparison_token(mode, profile.as_ref())))
            }
            // Replacing binds a *named* credential; the suffix means presence
            // alone can never satisfy it, so the write is always attempted.
            ConnectivityOp::ReplaceCredential {
                profile,
                credential,
            } => ConnectivityState::credential(Some(credential_replacement_reference(
                profile, credential,
            ))),
            // Deleting desires no credential bound — directly observable.
            ConnectivityOp::DeleteCredential { .. } => ConnectivityState::credential(None),
        }
    }

    /// The idempotency/verification comparator for every connectivity
    /// operation (the frozen manifest names `ExactTypedPostcondition` for
    /// both `toggle_wifi` and `connect_wifi`).
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transport seam
// ─────────────────────────────────────────────────────────────────────────────

/// The raw connectivity transport seam. The live implementation
/// ([`crate::os_control::linux::providers::network_manager::LiveNetworkManager`])
/// is a deny-live-gated adapter over NetworkManager D-Bus (structured `nmcli`
/// fallback until wired); deny-live tests inject
/// [`FakeConnectivityTransport`]. Reads run a query/parse; `dispatch` runs a
/// governed [`StructuredCommandRequest`].
#[async_trait]
pub trait ConnectivityTransport: Send + Sync {
    /// The stable provider identity (never model prose).
    fn provider_id(&self) -> ProviderId;

    /// The selected backend (records whether the native D-Bus path or the
    /// degraded `nmcli` fallback is in effect).
    fn selected_backend(&self) -> ConnectivityBackend;

    /// Read whether the Wi-Fi radio is enabled. A parse ambiguity must surface
    /// as an error, never a fabricated state.
    async fn read_radio_enabled(&self, ctx: &HostExecutionContext) -> Result<bool, OsControlError>;

    /// Read the currently active SSID, if any. `Ok(None)` is the distinct,
    /// unambiguous "not connected" result.
    async fn read_active_ssid(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Option<String>, OsControlError>;

    /// Scan for available Wi-Fi networks (bounded; used both by
    /// `get_wifi_networks` and by [`ConnectivityControl::apply`]'s duplicate-SSID
    /// disambiguation).
    async fn scan_wifi(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<RawWifiNetwork>, OsControlError>;

    /// List known network devices (Wi-Fi adapters and Ethernet NICs), bounded
    /// (Task 3.5). Backs `get_network_state`'s device resolution and
    /// device-targeted mutation preflight/ambiguity checks.
    async fn list_devices(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<RawNetworkDevice>, OsControlError>;

    /// List saved network profiles (Wi-Fi and Ethernet), bounded (Task 3.5).
    /// Backs `forget_wifi`/`activate_network_profile` identity resolution and
    /// duplicate-profile-name ambiguity checks.
    async fn list_profiles(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<RawNetworkProfile>, OsControlError>;

    /// Read whether `device` currently reports a connected state (Task 3.5;
    /// backs `disconnect_wifi` idempotency/verification).
    async fn read_device_connected(
        &self,
        ctx: &HostExecutionContext,
        device: &NetworkDeviceId,
    ) -> Result<bool, OsControlError>;

    /// Read whether `profile` is still present among saved profiles (Task 3.5;
    /// backs `forget_wifi` idempotency/verification).
    async fn read_profile_saved(
        &self,
        ctx: &HostExecutionContext,
        profile: &NetworkProfileId,
    ) -> Result<bool, OsControlError>;

    /// Read the active profile identity bound to `device` when given, or the
    /// overall active profile otherwise (Task 3.5; backs
    /// `activate_network_profile` idempotency/verification and rollback
    /// capture). `Ok(None)` is the distinct, unambiguous "no active profile"
    /// result.
    async fn read_active_profile(
        &self,
        ctx: &HostExecutionContext,
        device: Option<&NetworkDeviceId>,
    ) -> Result<Option<NetworkProfileId>, OsControlError>;

    /// Dispatch a governed structured command (the only path to a process).
    async fn dispatch(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError>;

    // ── Tasks 4.2 / 5.3 / 5.6 primitives ────────────────────────────────────

    /// List the currently active connections with their profile UUID, type,
    /// activation state and bound device. Backs VPN and hotspot state.
    async fn list_active_connections(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<RawActiveConnection>, OsControlError>;

    /// Read one property of a saved profile addressed by UUID.
    ///
    /// `Ok(None)` means the backend did not report the property at all, which is
    /// a different fact from an empty value — the caller decides which is
    /// admissible rather than this seam guessing.
    async fn read_profile_property(
        &self,
        ctx: &HostExecutionContext,
        profile: &NetworkProfileId,
        property: &str,
    ) -> Result<Option<String>, OsControlError>;

    /// Read whether a credential is stored for `property` on `profile`, without
    /// ever requesting the value. An indeterminate reply is an error, never
    /// `false`.
    async fn read_secret_present(
        &self,
        ctx: &HostExecutionContext,
        profile: &NetworkProfileId,
        property: &str,
    ) -> Result<bool, OsControlError>;

    /// Read the host connectivity verdict. `Undetermined` is a distinct verdict
    /// and is never collapsed into "unreachable".
    async fn read_connectivity(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<HostConnectivity, OsControlError>;

    /// Read presence-only IP facts for one device (no addresses are retained).
    async fn read_device_ip_facts(
        &self,
        ctx: &HostExecutionContext,
        device: &NetworkDeviceId,
    ) -> Result<RawDeviceIpFacts, OsControlError>;

    /// Read one desktop proxy key, returning the backend's raw reply for the
    /// domain's own parser. An unreadable key must surface as an error so the
    /// proxy state fails closed.
    async fn read_proxy_key(
        &self,
        ctx: &HostExecutionContext,
        schema: &str,
        key: &str,
    ) -> Result<String, OsControlError>;

    /// The desktop proxy backend this transport writes through.
    fn proxy_backend(&self) -> ProxyBackend;

    /// **Provider-only** credential resolution under the sealed mutation
    /// permit. The returned [`SecretPayload`] cannot serialize, clone, or
    /// display, so it can only travel to the governed stdin channel.
    async fn resolve_credential(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &SecretResolutionRequest,
    ) -> Result<SecretPayload, OsControlError>;
}

/// The rollback snapshot captured before an apply, so a contradiction can be
/// compensated back to the exact prior state.
#[derive(Debug, Clone)]
enum RollbackSnapshot {
    Radio {
        before_enabled: bool,
        action: String,
        params: serde_json::Value,
    },
    Connection {
        before_ssid: Option<String>,
        action: String,
        params: serde_json::Value,
    },
    /// Prior active-profile state for `activate_network_profile` (Task 3.5).
    /// `disconnect_wifi`/`forget_wifi` never capture a snapshot: both are
    /// `RollbackClaim::None` (design §13.1) and never dispatch a rollback.
    ActiveProfile {
        before_profile: Option<NetworkProfileId>,
        device: Option<NetworkDeviceId>,
        action: String,
        params: serde_json::Value,
    },
}

impl RollbackSnapshot {
    /// The canonical action name this snapshot was captured for (used to match
    /// the [`RollbackToken`]'s action-linkage digest against the correct
    /// recorded snapshot in `rollback`).
    fn action(&self) -> &str {
        match self {
            RollbackSnapshot::Radio { action, .. }
            | RollbackSnapshot::Connection { action, .. }
            | RollbackSnapshot::ActiveProfile { action, .. } => action,
        }
    }
}

/// The key connectivity snapshots are stored under: the session plus a scope
/// distinguishing the target (radio / connection / a specific device-or-
/// profile), so concurrent mutations on distinct targets within the same
/// session never clobber each other's rollback state (Task 3.5). Two
/// mutations sharing the same scope are already serialized by the write
/// resource lease on that scope.
fn snapshot_key(session: &str, scope: &str) -> String {
    format!("{session}#{scope}")
}

/// The `ConnectivityControl` desired-state provider (design §3, §4, §9.4).
/// Generic over the [`ConnectivityTransport`] so the same governed logic runs
/// over the live NetworkManager/`nmcli` adapter and the deny-live fake.
pub struct ConnectivityControl<T: ConnectivityTransport> {
    transport: T,
    policy: CommandPolicy,
    /// Prior-state snapshots keyed by session id, captured in `apply` for
    /// `rollback`. Interior mutability because the provider is shared (`&self`);
    /// connectivity ops are serialized by the radio/device resource lease.
    snapshots: Mutex<HashMap<String, RollbackSnapshot>>,
}

/// Max number of ambiguous-SSID candidates surfaced in
/// [`OsControlError::AmbiguousTarget`].
const MAX_AMBIGUOUS_CANDIDATES: usize = 16;

impl<T: ConnectivityTransport> ConnectivityControl<T> {
    /// Compose a `ConnectivityControl` over a transport.
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
    pub fn backend(&self) -> ConnectivityBackend {
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

    /// Scan for available Wi-Fi networks (`get_wifi_networks`; read-only, not
    /// part of the `DesiredStateControl` mutation lifecycle).
    pub async fn scan_wifi_networks(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<RawWifiNetwork>, OsControlError> {
        self.transport.scan_wifi(ctx).await
    }

    fn evidence_source(&self) -> OsEvidenceSource {
        match self.transport.selected_backend() {
            ConnectivityBackend::NetworkManager => OsEvidenceSource::AuthoritativeServiceState,
            ConnectivityBackend::Nmcli => OsEvidenceSource::StructuredCommandQuery,
        }
    }

    /// Build the governed structured command for a mutating operation.
    fn build_command(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        action: &str,
        params: &serde_json::Value,
        args: Vec<String>,
        redaction: RedactionMap,
    ) -> Result<StructuredCommandRequest, OsControlError> {
        let executable = self.transport.selected_backend().trusted_executable()?;
        self.build_command_full(ctx, action, params, executable, args, redaction, None)
    }

    /// Build the governed structured command for a mutating operation, with an
    /// explicit trusted executable and an optional **secret stdin** payload.
    ///
    /// A credential is delivered here and nowhere else: `stdin` is excluded from
    /// every digest, summary, trace and audit projection, so unlike an argv
    /// element it never reaches `/proc/<pid>/cmdline` or the ledger.
    fn build_command_full(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        action: &str,
        params: &serde_json::Value,
        executable: TrustedExecutable,
        args: Vec<String>,
        redaction: RedactionMap,
        stdin: Option<SecretStdin>,
    ) -> Result<StructuredCommandRequest, OsControlError> {
        let mut plan = CommandPlan::new(
            CapabilityId::new(action),
            action.to_string(),
            params.clone(),
            executable,
            args,
        );
        plan.redaction = redaction;
        if let Some(stdin) = stdin {
            plan = plan.with_secret_stdin(stdin);
        }
        StructuredCommandRequest::from_admitted(ctx, plan, &self.policy)
    }

    /// Detect a duplicate-SSID ambiguity by counting distinct access-point
    /// identities (BSSID, or a positional fallback when a row carries none)
    /// advertising the target `ssid`. Returns the candidate set only when
    /// there is more than one, so a single unambiguous match never trips this.
    fn ambiguous_candidates(rows: &[RawWifiNetwork], ssid: &str) -> Option<Vec<SafeCandidate>> {
        let mut seen: BTreeMap<String, u8> = BTreeMap::new();
        for row in rows.iter().filter(|r| r.ssid == ssid) {
            let key = row
                .bssid
                .clone()
                .unwrap_or_else(|| format!("{ssid}#{}", seen.len()));
            seen.entry(key).or_insert(row.signal_percent.unwrap_or(0));
        }
        if seen.len() > 1 {
            Some(
                seen.into_iter()
                    .map(|(identity, signal)| SafeCandidate {
                        label: SafeText::new(format!("{ssid} ({signal}%)")),
                        identity: Digest::of_str(&identity),
                    })
                    .collect(),
            )
        } else {
            None
        }
    }

    fn satisfying(
        &self,
        observed: &ConnectivityState,
    ) -> SatisfyingVerification<ConnectivityState> {
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

    // ── Tasks 4.2 / 5.3 / 5.6 composed reads ────────────────────────────────

    fn invalid(field: &str, reason: &str) -> OsControlError {
        OsControlError::InvalidRequest {
            field: SafeField::new(field),
            reason: SafeText::new(reason),
        }
    }

    /// List saved VPN profiles with their activation state (`list_vpn_profiles`).
    ///
    /// A profile's UUID is the identity; the display name is carried only as a
    /// label because two VPN profiles may share one.
    pub async fn list_vpn_profiles_read(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<VpnProfileSummary>, OsControlError> {
        let profiles = self.transport.list_profiles(ctx).await?;
        let active = self.transport.list_active_connections(ctx).await?;
        Ok(profiles
            .into_iter()
            .filter(|row| parsers::is_vpn_connection_type(&row.connection_type))
            .map(|row| {
                let connected = active
                    .iter()
                    .any(|a| a.uuid == row.uuid && a.is_activated());
                VpnProfileSummary {
                    profile: NetworkProfileId::new(row.uuid),
                    label: row.name,
                    connected,
                }
            })
            .collect())
    }

    /// Resolve a saved profile row by UUID, failing closed when it is absent.
    ///
    /// Matching is on the UUID only: a profile NAME is neither unique nor
    /// stable, so accepting one could act on a different profile entirely.
    async fn profile_row(
        &self,
        ctx: &HostExecutionContext,
        field: &str,
        profile: &NetworkProfileId,
    ) -> Result<RawNetworkProfile, OsControlError> {
        let profiles = self.transport.list_profiles(ctx).await?;
        profiles
            .into_iter()
            .find(|row| row.uuid == profile.as_str())
            .ok_or_else(|| {
                Self::invalid(
                    field,
                    "no saved profile has this UUID (a profile NAME is not accepted as an identity)",
                )
            })
    }

    /// Whether the VPN profile identified by `profile` is currently activated.
    async fn read_vpn_connected(
        &self,
        ctx: &HostExecutionContext,
        profile: &NetworkProfileId,
    ) -> Result<bool, OsControlError> {
        // Confirm the target really is a VPN profile before reporting on it: a
        // Wi-Fi profile's activation is a different fact from a VPN tunnel.
        let row = self.profile_row(ctx, "profile", profile).await?;
        if !parsers::is_vpn_connection_type(&row.connection_type) {
            return Err(Self::invalid(
                "profile",
                "the saved profile is not a VPN profile",
            ));
        }
        let active = self.transport.list_active_connections(ctx).await?;
        Ok(active
            .iter()
            .any(|a| a.uuid == profile.as_str() && a.is_activated()))
    }

    /// Read the hotspot state for `device`, or for the single Wi-Fi device when
    /// none was named.
    pub async fn hotspot_state_read(
        &self,
        ctx: &HostExecutionContext,
        device: Option<&NetworkDeviceId>,
    ) -> Result<HotspotFacts, OsControlError> {
        let resolved = match device {
            Some(device) => Some(self.wifi_device(ctx, device).await?),
            None => self.sole_wifi_device(ctx).await?,
        };
        let Some(resolved) = resolved else {
            // No Wi-Fi device at all is a fact: a hotspot cannot be running.
            return Ok(HotspotFacts {
                device: None,
                enabled: false,
                profile: None,
            });
        };
        let active = self.transport.list_active_connections(ctx).await?;
        let candidate = active.into_iter().find(|a| {
            a.is_activated() && a.device.as_deref() == Some(resolved.as_str())
        });
        let Some(candidate) = candidate else {
            return Ok(HotspotFacts {
                device: Some(resolved),
                enabled: false,
                profile: None,
            });
        };
        let profile = NetworkProfileId::new(candidate.uuid);
        let is_ap = self.profile_is_access_point(ctx, &profile).await?;
        Ok(HotspotFacts {
            device: Some(resolved),
            enabled: is_ap,
            profile: is_ap.then_some(profile),
        })
    }

    /// Whether a profile's Wi-Fi radio mode is `ap` (a hotspot profile).
    ///
    /// An absent mode property means the profile is not a Wi-Fi profile at all,
    /// which is a positive "not a hotspot" fact; an unrecognised value is an
    /// error rather than a guess.
    async fn profile_is_access_point(
        &self,
        ctx: &HostExecutionContext,
        profile: &NetworkProfileId,
    ) -> Result<bool, OsControlError> {
        let mode = self
            .transport
            .read_profile_property(ctx, profile, selection::WIFI_MODE_PROPERTY)
            .await?;
        match mode.as_deref().map(str::trim) {
            None | Some("") => Ok(false),
            Some(selection::WIFI_MODE_ACCESS_POINT) => Ok(true),
            Some("infrastructure") | Some("adhoc") | Some("mesh") => Ok(false),
            Some(_) => Err(self.unreadable("Wi-Fi radio mode was not recognised")),
        }
    }

    /// Confirm `device` exists and is a Wi-Fi device.
    async fn wifi_device(
        &self,
        ctx: &HostExecutionContext,
        device: &NetworkDeviceId,
    ) -> Result<NetworkDeviceId, OsControlError> {
        let devices = self.transport.list_devices(ctx).await?;
        let row = devices
            .into_iter()
            .find(|row| row.name == device.as_str())
            .ok_or_else(|| Self::invalid("device", "no such network device"))?;
        if row.kind() != NetworkDeviceKind::Wifi {
            return Err(Self::invalid("device", "the device is not a Wi-Fi device"));
        }
        Ok(NetworkDeviceId::new(row.name))
    }

    /// The single Wi-Fi device, or `None` when there is none. Two or more is an
    /// ambiguity, not a pick-the-first.
    async fn sole_wifi_device(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Option<NetworkDeviceId>, OsControlError> {
        let devices = self.transport.list_devices(ctx).await?;
        let wifi: Vec<&RawNetworkDevice> = devices
            .iter()
            .filter(|row| row.kind() == NetworkDeviceKind::Wifi)
            .collect();
        match wifi.len() {
            0 => Ok(None),
            1 => Ok(Some(NetworkDeviceId::new(wifi[0].name.clone()))),
            _ => {
                let candidates: Vec<SafeCandidate> = wifi
                    .into_iter()
                    .map(|row| SafeCandidate {
                        label: SafeText::new(row.name.clone()),
                        identity: Digest::of_str(&row.name),
                    })
                    .collect();
                Err(OsControlError::AmbiguousTarget {
                    kind: SafeText::new("network_device"),
                    candidates: BoundedVec::from_iter_capped(candidates, MAX_AMBIGUOUS_CANDIDATES),
                })
            }
        }
    }

    /// The device's own access-point profile, when exactly one exists.
    ///
    /// Zero or several is a failure rather than a silent choice: guessing here
    /// would broadcast an access point the caller never named.
    async fn sole_access_point_profile(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<NetworkProfileId, OsControlError> {
        let profiles = self.transport.list_profiles(ctx).await?;
        let mut found: Vec<NetworkProfileId> = Vec::new();
        for row in profiles
            .into_iter()
            .filter(|row| row.kind() == NetworkDeviceKind::Wifi)
        {
            let profile = NetworkProfileId::new(row.uuid);
            if self.profile_is_access_point(ctx, &profile).await? {
                found.push(profile);
            }
        }
        match found.len() {
            0 => Err(Self::invalid(
                "profile",
                "no saved access-point profile exists; name one explicitly rather than creating a new network",
            )),
            1 => Ok(found.remove(0)),
            _ => {
                let candidates: Vec<SafeCandidate> = found
                    .iter()
                    .map(|p| SafeCandidate {
                        label: SafeText::new("access-point profile"),
                        identity: Digest::of_str(p.as_str()),
                    })
                    .collect();
                Err(OsControlError::AmbiguousTarget {
                    kind: SafeText::new("network_profile"),
                    candidates: BoundedVec::from_iter_capped(candidates, MAX_AMBIGUOUS_CANDIDATES),
                })
            }
        }
    }

    /// Read the desktop-wide proxy state (`get_proxy_state`).
    ///
    /// Fails closed on any unparseable key: reporting "no proxy" for a state
    /// nobody could read would let a later mutation verify against a fiction,
    /// and would misreport where every application's traffic is going.
    pub async fn proxy_state_read(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<ProxyFacts, OsControlError> {
        let raw_mode = self
            .transport
            .read_proxy_key(ctx, selection::PROXY_SCHEMA, "mode")
            .await?;
        let backend_mode = parsers::parse_gsettings_string(&raw_mode)
            .ok_or_else(|| self.unreadable("desktop proxy mode could not be parsed"))?;
        let mode = parsers::proxy_mode_from_backend(&backend_mode)
            .ok_or_else(|| self.unreadable("desktop proxy mode was not recognised"))?;

        let profile = match mode {
            "none" => None,
            "automatic" => {
                let raw = self
                    .transport
                    .read_proxy_key(ctx, selection::PROXY_SCHEMA, "autoconfig-url")
                    .await?;
                let pac_uri = parsers::parse_gsettings_string(&raw)
                    .ok_or_else(|| self.unreadable("proxy PAC URI could not be parsed"))?;
                Some(ProxyProfile::Automatic { pac_uri })
            }
            _ => {
                let mut endpoints = Vec::new();
                for protocol in ["http", "https", "socks"] {
                    let schema = format!("{}.{protocol}", selection::PROXY_SCHEMA);
                    let raw_host = self.transport.read_proxy_key(ctx, &schema, "host").await?;
                    let host = parsers::parse_gsettings_string(&raw_host).ok_or_else(|| {
                        self.unreadable("proxy endpoint host could not be parsed")
                    })?;
                    let raw_port = self.transport.read_proxy_key(ctx, &schema, "port").await?;
                    let port = parsers::parse_gsettings_port(&raw_port).ok_or_else(|| {
                        self.unreadable("proxy endpoint port could not be parsed")
                    })?;
                    // An empty host or a zero port is "not configured" — a fact
                    // the backend reports positively, not a parse failure.
                    endpoints.push((host.is_empty() || port == 0).then_some(()).map_or(
                        Some(ProxyEndpoint { host, port }),
                        |()| None,
                    ));
                }
                let raw_exclusions = self
                    .transport
                    .read_proxy_key(ctx, selection::PROXY_SCHEMA, "ignore-hosts")
                    .await?;
                let exclusions = parsers::parse_gsettings_string_list(&raw_exclusions)
                    .ok_or_else(|| self.unreadable("proxy exclusion list could not be parsed"))?;
                Some(ProxyProfile::Manual {
                    http: endpoints[0].clone(),
                    https: endpoints[1].clone(),
                    socks: endpoints[2].clone(),
                    exclusions,
                })
            }
        };
        Ok(ProxyFacts {
            mode: mode.to_string(),
            profile,
        })
    }

    /// List value-free metadata for every saved connectivity credential
    /// (`list_saved_credentials`).
    ///
    /// The credential *value* is never requested from the backend, so it cannot
    /// be logged, digested, or returned. Only presence is read.
    pub async fn saved_credentials_read(
        &self,
        ctx: &HostExecutionContext,
        kind: Option<ConnectivityCredentialKind>,
    ) -> Result<Vec<SavedCredentialSummary>, OsControlError> {
        let profiles = self.transport.list_profiles(ctx).await?;
        let mut out = Vec::new();
        for row in profiles {
            let Some(row_kind) = ConnectivityCredentialKind::from_connection_type(
                &row.connection_type,
            ) else {
                continue;
            };
            if kind.is_some_and(|wanted| wanted != row_kind) {
                continue;
            }
            let profile = NetworkProfileId::new(row.uuid);
            let present = self
                .transport
                .read_secret_present(ctx, &profile, row_kind.secret_property())
                .await?;
            if !present {
                continue;
            }
            out.push(SavedCredentialSummary {
                secret_ref: credential_reference(&profile),
                profile,
                label: row.name,
                kind: row_kind,
            });
        }
        Ok(out)
    }

    /// The credential class a profile's stored secret belongs to.
    async fn credential_kind_of(
        &self,
        ctx: &HostExecutionContext,
        profile: &NetworkProfileId,
    ) -> Result<ConnectivityCredentialKind, OsControlError> {
        let row = self.profile_row(ctx, "profile", profile).await?;
        ConnectivityCredentialKind::from_connection_type(&row.connection_type).ok_or_else(|| {
            Self::invalid(
                "profile",
                "the saved profile holds no Wi-Fi or VPN credential this tool manages",
            )
        })
    }

    /// The currently bound credential reference for a profile, or `None` when no
    /// credential is stored.
    async fn read_credential_binding(
        &self,
        ctx: &HostExecutionContext,
        profile: &NetworkProfileId,
    ) -> Result<Option<String>, OsControlError> {
        let kind = self.credential_kind_of(ctx, profile).await?;
        let present = self
            .transport
            .read_secret_present(ctx, profile, kind.secret_property())
            .await?;
        Ok(present.then(|| credential_reference(profile)))
    }

    /// Produce a layered diagnosis (`diagnose`).
    ///
    /// Each layer reports its own verdict, and `undetermined` never collapses
    /// into a negative finding. `target_named` records that the caller asked
    /// about a specific host, which this provider cannot probe.
    pub async fn diagnose_read(
        &self,
        ctx: &HostExecutionContext,
        target_named: bool,
    ) -> Result<NetworkDiagnosisFacts, OsControlError> {
        let devices = self.transport.list_devices(ctx).await?;
        let routable: Vec<&RawNetworkDevice> = devices
            .iter()
            .filter(|row| {
                matches!(
                    row.kind(),
                    NetworkDeviceKind::Wifi | NetworkDeviceKind::Ethernet
                )
            })
            .take(MAX_DIAGNOSED_DEVICES)
            .collect();

        let mut link_up = false;
        let mut link_known = false;
        let mut has_address = false;
        let mut has_route = false;
        let mut has_gateway = false;
        let mut has_dns = false;
        let mut facts_read = false;

        for row in &routable {
            let device = NetworkDeviceId::new(row.name.clone());
            let facts = self.transport.read_device_ip_facts(ctx, &device).await?;
            facts_read = true;
            match facts.link() {
                "up" => {
                    link_up = true;
                    link_known = true;
                }
                "down" => link_known = true,
                _ => {}
            }
            has_address |= facts.address_count > 0;
            has_route |= facts.has_default_route;
            has_gateway |= facts.has_gateway;
            has_dns |= facts.dns_count > 0;
        }

        // No managed device at all is a fact: the link is down. A device that
        // exists but never reported a state is *unknown*, not down.
        let link = if routable.is_empty() {
            "down"
        } else if link_up {
            "up"
        } else if link_known {
            "down"
        } else {
            "undetermined"
        };
        let present_or = |flag: bool, yes: &'static str, no: &'static str| {
            if !facts_read && !routable.is_empty() {
                "undetermined"
            } else if flag {
                yes
            } else {
                no
            }
        };

        let connectivity = self.transport.read_connectivity(ctx).await?;
        Ok(NetworkDiagnosisFacts {
            link,
            address: present_or(has_address, "assigned", "absent"),
            route: present_or(has_route, "default_present", "absent"),
            gateway: present_or(has_gateway, "present", "absent"),
            dns: present_or(has_dns, "configured", "absent"),
            internet: connectivity.as_str(),
            captive_portal: connectivity.captive_portal(),
            target_probe_unavailable: target_named,
        })
    }

    /// A fail-closed read error for this provider.
    fn unreadable(&self, reason: &str) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(self.transport.provider_id()),
            reason: SafeText::new(format!("network state {reason}")),
            retryable: true,
        }
    }

    /// Reject a hotspot passphrase that WPA/WPA2 would not accept.
    ///
    /// The error names only the *length bound* — never any part of the value —
    /// so a rejected passphrase cannot be reconstructed from the error text.
    fn validate_wpa_passphrase(bytes: &[u8]) -> Result<(), OsControlError> {
        if bytes.len() < selection::WPA_PASSPHRASE_MIN_BYTES {
            return Err(Self::invalid(
                "credential",
                "the resolved hotspot passphrase is shorter than the 8-byte WPA minimum; refusing to create a weakly protected access point",
            ));
        }
        if bytes.len() > selection::WPA_PASSPHRASE_MAX_BYTES {
            return Err(Self::invalid(
                "credential",
                "the resolved hotspot passphrase exceeds the 63-byte WPA maximum",
            ));
        }
        Ok(())
    }
}

/// Max number of devices a single diagnosis reads IP facts for, so the read
/// stays bounded on a host with many interfaces.
const MAX_DIAGNOSED_DEVICES: usize = 8;

#[async_trait]
impl<T: ConnectivityTransport> DesiredStateControl<ConnectivityRequest, ConnectivityState>
    for ConnectivityControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &ConnectivityRequest,
    ) -> Result<ConnectivityState, OsControlError> {
        match &request.op {
            ConnectivityOp::ToggleRadio(_) => {
                let enabled = self.transport.read_radio_enabled(ctx).await?;
                Ok(ConnectivityState::radio(enabled))
            }
            ConnectivityOp::ConnectWifi(_) => {
                let active = self.transport.read_active_ssid(ctx).await?;
                Ok(ConnectivityState::connection(active))
            }
            ConnectivityOp::DisconnectWifi(device) => {
                let connected = self.transport.read_device_connected(ctx, device).await?;
                Ok(ConnectivityState::device(connected))
            }
            ConnectivityOp::ForgetProfile(profile) => {
                let saved = self.transport.read_profile_saved(ctx, profile).await?;
                Ok(ConnectivityState::profile_saved(saved))
            }
            ConnectivityOp::ActivateProfile { device, .. } => {
                let active = self
                    .transport
                    .read_active_profile(ctx, device.as_ref())
                    .await?;
                Ok(ConnectivityState::active_profile(
                    active.map(|p| p.into_string()),
                ))
            }
            ConnectivityOp::SetVpn { profile, .. } => Ok(ConnectivityState::vpn(
                self.read_vpn_connected(ctx, profile).await?,
            )),
            ConnectivityOp::SetHotspot { device, .. } => {
                let facts = self.hotspot_state_read(ctx, Some(device)).await?;
                Ok(ConnectivityState::hotspot(facts.enabled))
            }
            // An unreadable proxy state propagates as an error: it must never
            // become "no proxy", which is where every application's traffic goes.
            ConnectivityOp::SetProxy { .. } => Ok(ConnectivityState::proxy(Some(
                self.proxy_state_read(ctx).await?.comparison_token(),
            ))),
            ConnectivityOp::ReplaceCredential { profile, .. }
            | ConnectivityOp::DeleteCredential { profile } => Ok(ConnectivityState::credential(
                self.read_credential_binding(ctx, profile).await?,
            )),
        }
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &ConnectivityRequest,
        _desired: &ConnectivityState,
    ) -> Result<ApplyOutcome, OsControlError> {
        match &request.op {
            ConnectivityOp::ToggleRadio(enabled) => {
                if let Ok(before_enabled) =
                    self.transport.read_radio_enabled(ctx.observation()).await
                {
                    let session = ctx.grant().session_id().to_string();
                    self.snapshots
                        .lock()
                        .expect("connectivity snapshots poisoned")
                        .insert(
                            snapshot_key(&session, "radio"),
                            RollbackSnapshot::Radio {
                                before_enabled,
                                action: request.action.clone(),
                                params: request.params.clone(),
                            },
                        );
                }

                let args = selection::set_radio_argv(*enabled);
                let command = self.build_command(
                    ctx,
                    &request.action,
                    &request.params,
                    args,
                    RedactionMap::new(),
                )?;
                self.transport.dispatch(ctx, &command).await
            }
            ConnectivityOp::ConnectWifi(op) => {
                // Duplicate-SSID clarification (OSC-015): never silently pick
                // one access point when several advertise the same name.
                let rows = self.transport.scan_wifi(ctx.observation()).await?;
                if let Some(candidates) = Self::ambiguous_candidates(&rows, &op.ssid) {
                    return Err(OsControlError::AmbiguousTarget {
                        kind: SafeText::new("wifi_network"),
                        candidates: BoundedVec::from_iter_capped(
                            candidates,
                            MAX_AMBIGUOUS_CANDIDATES,
                        ),
                    });
                }

                if let Ok(before_ssid) = self.transport.read_active_ssid(ctx.observation()).await {
                    let session = ctx.grant().session_id().to_string();
                    self.snapshots
                        .lock()
                        .expect("connectivity snapshots poisoned")
                        .insert(
                            snapshot_key(&session, "connection"),
                            RollbackSnapshot::Connection {
                                before_ssid,
                                action: request.action.clone(),
                                params: request.params.clone(),
                            },
                        );
                }

                let has_password = op.password.is_some();
                let mut args = selection::connect_wifi_argv(&op.ssid, has_password);
                let mut redaction = RedactionMap::new();
                if let Some(password) = &op.password {
                    // The secret argv position is the last element; mark it
                    // secret so no captured summary/trace/audit ever shows it
                    // (OSC-025.4, OSC-029). The raw bytes are used verbatim for
                    // this one dispatch and never copied elsewhere.
                    let secret_text =
                        String::from_utf8_lossy(password.expose_secret()).into_owned();
                    redaction = redaction.with_secret_arg(args.len());
                    args.push(secret_text);
                }

                let command =
                    self.build_command(ctx, &request.action, &request.params, args, redaction)?;
                self.transport.dispatch(ctx, &command).await
            }
            ConnectivityOp::DisconnectWifi(device) => {
                // `disconnect_wifi` is `RollbackClaim::None` (design §13.1): no
                // snapshot is captured and `rollback` never dispatches for
                // this scope.
                let args = selection::disconnect_wifi_argv(device.as_str());
                let command = self.build_command(
                    ctx,
                    &request.action,
                    &request.params,
                    args,
                    RedactionMap::new(),
                )?;
                self.transport.dispatch(ctx, &command).await
            }
            ConnectivityOp::ForgetProfile(profile) => {
                // `forget_wifi` is `RollbackClaim::None` (design §13.1): a
                // forgotten profile's saved configuration is not
                // reconstructible, so no snapshot is captured and no receipt
                // for this op ever claims recoverability.
                let args = selection::forget_profile_argv(profile.as_str());
                let command = self.build_command(
                    ctx,
                    &request.action,
                    &request.params,
                    args,
                    RedactionMap::new(),
                )?;
                self.transport.dispatch(ctx, &command).await
            }
            ConnectivityOp::ActivateProfile { profile, device } => {
                // Duplicate-device clarification (OSC-015.6): when the caller
                // did not name a device, resolve it only if exactly one
                // eligible device (matching the profile's kind) exists.
                // Ethernet activation reuses this exact path — there is no
                // separate Ethernet branch.
                let resolved_device = match device {
                    Some(d) => Some(d.clone()),
                    None => {
                        let profiles = self.transport.list_profiles(ctx.observation()).await?;
                        let Some(profile_row) =
                            profiles.iter().find(|p| p.uuid == profile.as_str())
                        else {
                            // The profile disappeared between observation and
                            // apply (event invalidation, OSC-031): fail
                            // closed rather than silently activating a stale
                            // target.
                            return Err(OsControlError::InvalidRequest {
                                field: SafeField::new("profile"),
                                reason: SafeText::new("the saved network profile no longer exists"),
                            });
                        };
                        let kind = profile_row.kind();
                        let devices = self.transport.list_devices(ctx.observation()).await?;
                        let candidates: Vec<&RawNetworkDevice> =
                            devices.iter().filter(|d| d.kind() == kind).collect();
                        match candidates.len() {
                            0 => None,
                            1 => Some(NetworkDeviceId::new(candidates[0].name.clone())),
                            _ => {
                                let candidate_list: Vec<SafeCandidate> = candidates
                                    .into_iter()
                                    .map(|d| SafeCandidate {
                                        label: SafeText::new(d.name.clone()),
                                        identity: Digest::of_str(&d.name),
                                    })
                                    .collect();
                                return Err(OsControlError::AmbiguousTarget {
                                    kind: SafeText::new("network_device"),
                                    candidates: BoundedVec::from_iter_capped(
                                        candidate_list,
                                        MAX_AMBIGUOUS_CANDIDATES,
                                    ),
                                });
                            }
                        }
                    }
                };

                if let Ok(before_profile) = self
                    .transport
                    .read_active_profile(ctx.observation(), resolved_device.as_ref())
                    .await
                {
                    let session = ctx.grant().session_id().to_string();
                    let scope = resolved_device
                        .as_ref()
                        .map(|d| format!("device:{}", d.as_str()))
                        .unwrap_or_else(|| "active_profile".to_string());
                    self.snapshots
                        .lock()
                        .expect("connectivity snapshots poisoned")
                        .insert(
                            snapshot_key(&session, &scope),
                            RollbackSnapshot::ActiveProfile {
                                before_profile,
                                device: resolved_device.clone(),
                                action: request.action.clone(),
                                params: request.params.clone(),
                            },
                        );
                }

                let args = selection::activate_profile_argv(
                    profile.as_str(),
                    resolved_device.as_ref().map(NetworkDeviceId::as_str),
                );
                let command = self.build_command(
                    ctx,
                    &request.action,
                    &request.params,
                    args,
                    RedactionMap::new(),
                )?;
                self.transport.dispatch(ctx, &command).await
            }
            ConnectivityOp::SetVpn { profile, connected } => {
                selection::validate_argv_token("profile", profile.as_str())?;
                // Re-confirm the target is a VPN profile under the permit: a
                // profile could have been replaced between admission and apply.
                let row = self
                    .profile_row(ctx.observation(), "profile", profile)
                    .await?;
                if !parsers::is_vpn_connection_type(&row.connection_type) {
                    return Err(Self::invalid(
                        "profile",
                        "the saved profile is not a VPN profile",
                    ));
                }
                let args = if *connected {
                    selection::profile_up_argv(profile.as_str(), None, false)
                } else {
                    selection::profile_down_argv(profile.as_str())
                };
                let command = self.build_command(
                    ctx,
                    &request.action,
                    &request.params,
                    args,
                    RedactionMap::new(),
                )?;
                self.transport.dispatch(ctx, &command).await
            }
            ConnectivityOp::SetHotspot {
                device,
                enabled,
                profile,
                credential,
            } => {
                selection::validate_argv_token("device", device.as_str())?;
                let device = self.wifi_device(ctx.observation(), device).await?;
                let profile = match profile {
                    Some(profile) => {
                        selection::validate_argv_token("profile", profile.as_str())?;
                        let row = self
                            .profile_row(ctx.observation(), "profile", profile)
                            .await?;
                        let profile = NetworkProfileId::new(row.uuid);
                        if !self
                            .profile_is_access_point(ctx.observation(), &profile)
                            .await?
                        {
                            return Err(Self::invalid(
                                "profile",
                                "the saved profile is not an access-point (hotspot) profile",
                            ));
                        }
                        profile
                    }
                    // Resolve the device's own hotspot profile. Never create a
                    // new network: an absent or ambiguous profile fails closed.
                    None => self.sole_access_point_profile(ctx.observation()).await?,
                };

                if !*enabled {
                    let args = selection::profile_down_argv(profile.as_str());
                    let command = self.build_command(
                        ctx,
                        &request.action,
                        &request.params,
                        args,
                        RedactionMap::new(),
                    )?;
                    return self.transport.dispatch(ctx, &command).await;
                }

                // Turning this machine into an access point: refuse an open or
                // credential-less hotspot outright rather than broadcasting one.
                let key_mgmt = self
                    .transport
                    .read_profile_property(
                        ctx.observation(),
                        &profile,
                        selection::WIFI_KEY_MGMT_PROPERTY,
                    )
                    .await?;
                let secured = key_mgmt
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|value| !value.is_empty() && value != "none");
                if !secured {
                    return Err(Self::invalid(
                        "profile",
                        "refusing to start an unencrypted hotspot: the profile has no key management configured",
                    ));
                }

                let stdin = match credential {
                    Some(reference) => {
                        // Provider-only resolution under the sealed permit. The
                        // payload cannot serialize, so it can only reach stdin.
                        let payload = self
                            .transport
                            .resolve_credential(
                                ctx,
                                &SecretResolutionRequest {
                                    reference: reference.clone(),
                                    purpose: SecretPurpose::HotspotCredential,
                                    scope: SecretScope::new(format!(
                                        "network-profile/{}",
                                        profile.as_str()
                                    )),
                                },
                            )
                            .await?;
                        Self::validate_wpa_passphrase(payload.expose_secret())?;
                        Some(SecretStdin::new(selection::passwd_file_body(
                            selection::WIFI_PSK_PROPERTY,
                            payload.expose_secret(),
                        )))
                    }
                    None => {
                        // No credential supplied: the profile must already hold
                        // one. A hotspot with neither is refused.
                        let stored = self
                            .transport
                            .read_secret_present(
                                ctx.observation(),
                                &profile,
                                selection::WIFI_PSK_PROPERTY,
                            )
                            .await?;
                        if !stored {
                            return Err(Self::invalid(
                                "credential",
                                "the hotspot profile stores no passphrase; supply a credential reference rather than starting an unprotected access point",
                            ));
                        }
                        None
                    }
                };

                let args = selection::profile_up_argv(
                    profile.as_str(),
                    Some(device.as_str()),
                    stdin.is_some(),
                );
                let executable = self.transport.selected_backend().trusted_executable()?;
                let command = self.build_command_full(
                    ctx,
                    &request.action,
                    &request.params,
                    executable,
                    args,
                    RedactionMap::new(),
                    stdin,
                )?;
                self.transport.dispatch(ctx, &command).await
            }
            ConnectivityOp::SetProxy { mode, profile } => {
                let backend_mode = parsers::proxy_mode_to_backend(mode).ok_or_else(|| {
                    Self::invalid("mode", "must be one of none, automatic, manual")
                })?;
                // Fail closed on an unreadable current state: redirecting every
                // application's traffic from a state nobody could read is not a
                // decision this provider is willing to make.
                let _current = self.proxy_state_read(ctx.observation()).await?;

                match (mode.as_str(), profile.as_ref()) {
                    ("none", Some(_)) => {
                        return Err(Self::invalid(
                            "profile",
                            "must be omitted when mode is none",
                        ));
                    }
                    ("none", None) => {}
                    (_, None) => {
                        return Err(Self::invalid(
                            "profile",
                            "is required for the automatic and manual modes",
                        ));
                    }
                    (mode, Some(profile)) => {
                        if profile.mode() != mode {
                            return Err(Self::invalid(
                                "profile",
                                "the profile kind must match the requested mode",
                            ));
                        }
                        profile.validate()?;
                    }
                }

                let executable = self.transport.proxy_backend().trusted_executable()?;
                let mut writes = Vec::new();
                match profile.as_ref() {
                    Some(ProxyProfile::Automatic { pac_uri }) => writes.push((
                        selection::PROXY_SCHEMA.to_string(),
                        "autoconfig-url".to_string(),
                        selection::gvariant_string("profile.pac_uri", pac_uri)?,
                    )),
                    Some(ProxyProfile::Manual {
                        http,
                        https,
                        socks,
                        exclusions,
                    }) => {
                        for (protocol, endpoint) in
                            [("http", http), ("https", https), ("socks", socks)]
                        {
                            let schema = format!("{}.{protocol}", selection::PROXY_SCHEMA);
                            // An omitted endpoint is written as cleared, so a
                            // stale endpoint from a previous profile cannot keep
                            // receiving traffic.
                            let (host, port) = match endpoint {
                                Some(endpoint) => (endpoint.host.as_str(), endpoint.port),
                                None => ("", 0),
                            };
                            writes.push((
                                schema.clone(),
                                "host".to_string(),
                                selection::gvariant_string("profile.host", host)?,
                            ));
                            writes.push((schema, "port".to_string(), port.to_string()));
                        }
                        writes.push((
                            selection::PROXY_SCHEMA.to_string(),
                            "ignore-hosts".to_string(),
                            selection::gvariant_string_list("profile.exclusions", exclusions)?,
                        ));
                    }
                    None => {}
                }
                // The mode key is written LAST: if an endpoint write fails, the
                // desktop stays on its previous, known-good configuration rather
                // than switching to a half-written proxy.
                writes.push((
                    selection::PROXY_SCHEMA.to_string(),
                    "mode".to_string(),
                    selection::gvariant_string("mode", backend_mode)?,
                ));

                let mut outcome = None;
                for (schema, key, value) in writes {
                    let args = selection::proxy_set_argv(&schema, &key, &value);
                    let command = self.build_command_full(
                        ctx,
                        &request.action,
                        &request.params,
                        executable.clone(),
                        args,
                        RedactionMap::new(),
                        None,
                    )?;
                    outcome = Some(self.transport.dispatch(ctx, &command).await?);
                }
                // `writes` always ends with the mode key, so this is never empty.
                outcome.ok_or_else(|| self.unreadable("proxy write plan was empty"))
            }
            ConnectivityOp::ReplaceCredential {
                profile,
                credential,
            } => {
                selection::validate_argv_token("profile", profile.as_str())?;
                let kind = self.credential_kind_of(ctx.observation(), profile).await?;
                let payload = self
                    .transport
                    .resolve_credential(
                        ctx,
                        &SecretResolutionRequest {
                            reference: credential.clone(),
                            purpose: kind.purpose(),
                            scope: SecretScope::new(format!(
                                "network-profile/{}",
                                profile.as_str()
                            )),
                        },
                    )
                    .await?;
                let bytes = payload.expose_secret();
                match kind {
                    ConnectivityCredentialKind::Wifi => Self::validate_wpa_passphrase(bytes)?,
                    ConnectivityCredentialKind::Vpn => {
                        if bytes.is_empty() {
                            return Err(Self::invalid(
                                "credential",
                                "the resolved credential is empty",
                            ));
                        }
                    }
                }
                // The editor script is line-oriented, so an embedded newline
                // would let a credential inject an editor command.
                if bytes.iter().any(|byte| matches!(byte, b'\n' | b'\r' | 0)) {
                    return Err(Self::invalid(
                        "credential",
                        "the resolved credential must not contain a newline or NUL byte",
                    ));
                }
                // The value travels on stdin only: argv carries just the profile
                // UUID, so the credential never reaches the argv digest, the
                // audit record, or /proc/<pid>/cmdline.
                let stdin = SecretStdin::new(selection::editor_set_secret_script(
                    kind.secret_property(),
                    bytes,
                ));
                let executable = self.transport.selected_backend().trusted_executable()?;
                let command = self.build_command_full(
                    ctx,
                    &request.action,
                    &request.params,
                    executable,
                    selection::profile_edit_argv(profile.as_str()),
                    RedactionMap::new(),
                    Some(stdin),
                )?;
                self.transport.dispatch(ctx, &command).await
            }
            ConnectivityOp::DeleteCredential { profile } => {
                selection::validate_argv_token("profile", profile.as_str())?;
                let kind = self.credential_kind_of(ctx.observation(), profile).await?;
                // Clearing writes an empty value: no secret is involved at all,
                // and the previous value is not recoverable, so no receipt for
                // this op ever advertises a rollback.
                let args =
                    selection::clear_profile_secret_argv(profile.as_str(), kind.secret_property());
                let command = self.build_command(
                    ctx,
                    &request.action,
                    &request.params,
                    args,
                    RedactionMap::new(),
                )?;
                self.transport.dispatch(ctx, &command).await
            }
        }
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &ConnectivityRequest,
        desired: &ConnectivityState,
    ) -> Result<VerificationReport<ConnectivityState>, OsControlError> {
        // A replaced credential is a special case: presence is observable, the
        // value is not. Confirming presence proves the write did not erase the
        // secret, but it cannot prove *which* value is stored — so this reports
        // `Inconclusive` rather than fabricating either verdict. Reading the
        // value back to compare would mean asking the backend to disclose it.
        if let ConnectivityOp::ReplaceCredential { profile, .. } = &request.op {
            return Ok(match self.read_credential_binding(ctx, profile).await? {
                Some(_) => VerificationReport::Inconclusive {
                    reason: SafeText::new(
                        "a credential is stored for the profile, but its value cannot be read back without disclosing it",
                    ),
                },
                // No credential at all after a replacement is decisive failure.
                None => VerificationReport::Contradicted(VerificationContradiction::new(
                    desired.observation_digest(),
                    Some(ConnectivityState::credential(None).observation_digest()),
                    SafeErrorCode::from_static("os_control.incident.contradicted"),
                )),
            });
        }

        let observed = match &request.op {
            ConnectivityOp::ToggleRadio(_) => {
                ConnectivityState::radio(self.transport.read_radio_enabled(ctx).await?)
            }
            ConnectivityOp::ConnectWifi(_) => {
                ConnectivityState::connection(self.transport.read_active_ssid(ctx).await?)
            }
            ConnectivityOp::DisconnectWifi(device) => {
                ConnectivityState::device(self.transport.read_device_connected(ctx, device).await?)
            }
            ConnectivityOp::ForgetProfile(profile) => ConnectivityState::profile_saved(
                self.transport.read_profile_saved(ctx, profile).await?,
            ),
            ConnectivityOp::ActivateProfile { device, .. } => {
                let active = self
                    .transport
                    .read_active_profile(ctx, device.as_ref())
                    .await?;
                ConnectivityState::active_profile(active.map(|p| p.into_string()))
            }
            ConnectivityOp::SetVpn { profile, .. } => {
                ConnectivityState::vpn(self.read_vpn_connected(ctx, profile).await?)
            }
            ConnectivityOp::SetHotspot { device, .. } => {
                ConnectivityState::hotspot(self.hotspot_state_read(ctx, Some(device)).await?.enabled)
            }
            ConnectivityOp::SetProxy { .. } => {
                ConnectivityState::proxy(Some(self.proxy_state_read(ctx).await?.comparison_token()))
            }
            ConnectivityOp::DeleteCredential { profile } => {
                ConnectivityState::credential(self.read_credential_binding(ctx, profile).await?)
            }
            // Handled above with its own inconclusive verdict.
            ConnectivityOp::ReplaceCredential { .. } => unreachable!(),
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
        let session = token.session_id().as_str();
        let session_prefix = format!("{session}#");
        // The token binds an action-name digest (design §13.1 rollback-token
        // action linkage); match the recorded snapshot whose own action digest
        // agrees, so concurrent rollback-eligible mutations on distinct scopes
        // within the same session (e.g. two `activate_network_profile` calls
        // for different devices) never resolve to the wrong snapshot. The
        // lock is scoped to this block so the guard is dropped before any
        // `.await` below (a `MutexGuard` is not `Send`).
        let snapshot = {
            let snapshots = self
                .snapshots
                .lock()
                .expect("connectivity snapshots poisoned");
            snapshots
                .iter()
                .filter(|(key, _)| key.starts_with(&session_prefix))
                .find(|(_, snap)| Digest::of_str(snap.action()) == *token.action_hash())
                .map(|(_, v)| v.clone())
        };

        let Some(snapshot) = snapshot else {
            return Ok(unobservable_uncertain());
        };

        match snapshot {
            RollbackSnapshot::Radio {
                before_enabled,
                action,
                params,
            } => {
                let args = selection::set_radio_argv(before_enabled);
                let command =
                    self.build_command(ctx, &action, &params, args, RedactionMap::new())?;
                self.transport.dispatch(ctx, &command).await
            }
            RollbackSnapshot::Connection {
                before_ssid: Some(ssid),
                action,
                params,
            } => {
                // Restore without a password: the prior connection either was
                // unauthenticated or used a saved NetworkManager profile that
                // already carries its own credential state.
                let args = selection::connect_wifi_argv(&ssid, false);
                let command =
                    self.build_command(ctx, &action, &params, args, RedactionMap::new())?;
                self.transport.dispatch(ctx, &command).await
            }
            // No prior connection existed: there is no positive inverse action
            // to dispatch (a "connect to nothing" is not representable).
            RollbackSnapshot::Connection {
                before_ssid: None, ..
            } => Ok(unobservable_uncertain()),
            RollbackSnapshot::ActiveProfile {
                before_profile: Some(profile),
                device,
                action,
                params,
            } => {
                let args = selection::activate_profile_argv(
                    profile.as_str(),
                    device.as_ref().map(NetworkDeviceId::as_str),
                );
                let command =
                    self.build_command(ctx, &action, &params, args, RedactionMap::new())?;
                self.transport.dispatch(ctx, &command).await
            }
            // No prior active profile existed: there is no positive inverse
            // action to dispatch.
            RollbackSnapshot::ActiveProfile {
                before_profile: None,
                ..
            } => Ok(unobservable_uncertain()),
        }
    }
}

fn unobservable_uncertain() -> ApplyOutcome {
    ApplyOutcome::Uncertain(UncertainDispatch::new(
        None,
        UncertainEffectCause::Unobservable,
        BoundedVec::new(),
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// Receipt → tool-result mapping (existing tools/results stay compatible)
// ─────────────────────────────────────────────────────────────────────────────

use crate::os_control::receipt::{ActionLifecycle, MutationReceipt};

/// Map a governed [`MutationReceipt`] to the **existing** `toggle_wifi` result
/// fields (`wifi`, `changed`, `already_in_desired_state`), plus additive
/// `backend`/`lifecycle`/`verified` fields (design §9.4, Task 2.3).
#[must_use]
pub fn toggle_wifi_result(
    receipt: &MutationReceipt<ConnectivityState>,
    requested_enabled: bool,
    backend: ConnectivityBackend,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "wifi": if requested_enabled { "on" } else { "off" },
        "backend": backend.as_str(),
        "changed": receipt.changed(),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// Map a governed [`MutationReceipt`] to the **existing** `connect_wifi` result
/// fields (`connected`, `changed`, `already_in_desired_state`), plus additive
/// `backend`/`lifecycle`/`verified` fields. Never includes the password or any
/// raw command output (OSC-025.4).
#[must_use]
pub fn connect_wifi_result(
    receipt: &MutationReceipt<ConnectivityState>,
    ssid: &str,
    backend: ConnectivityBackend,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "connected": ssid,
        "backend": backend.as_str(),
        "changed": receipt.changed(),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// Map scanned Wi-Fi rows to the **existing** `get_wifi_networks` result shape
/// (`networks: [{ssid, signal, security}]`), plus an additive `bssid` field so
/// a caller (or a later duplicate-SSID-aware UI) can disambiguate without a
/// wire-breaking change.
#[must_use]
pub fn wifi_networks_result(rows: &[RawWifiNetwork]) -> serde_json::Value {
    let networks: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "ssid": row.ssid,
                "signal": row.signal_percent.map(|v| v.to_string()).unwrap_or_default(),
                "security": row.security,
                "bssid": row.bssid,
            })
        })
        .collect();
    serde_json::json!({ "networks": networks })
}

/// Map a governed [`MutationReceipt`] to the `disconnect_wifi` result fields
/// (Task 3.5). Never claims rollback (design §13.1: `RollbackClaim::None`).
#[must_use]
pub fn disconnect_wifi_result(
    receipt: &MutationReceipt<ConnectivityState>,
    device: &NetworkDeviceId,
    backend: ConnectivityBackend,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "device": device.as_str(),
        "backend": backend.as_str(),
        "changed": receipt.changed(),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// Map a governed [`MutationReceipt`] to the `forget_wifi` result fields
/// (Task 3.5). Never claims rollback: a forgotten profile's saved
/// configuration is not reconstructible (design §13.1: `RollbackClaim::None`).
#[must_use]
pub fn forget_wifi_result(
    receipt: &MutationReceipt<ConnectivityState>,
    profile: &NetworkProfileId,
    backend: ConnectivityBackend,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "profile": profile.as_str(),
        "backend": backend.as_str(),
        "changed": receipt.changed(),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// Map a governed [`MutationReceipt`] to the `activate_network_profile` result
/// fields (Task 3.5; Ethernet activation reuses this same mapping — there is
/// no separate Ethernet result shape).
#[must_use]
pub fn activate_network_profile_result(
    receipt: &MutationReceipt<ConnectivityState>,
    profile: &NetworkProfileId,
    backend: ConnectivityBackend,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "profile": profile.as_str(),
        "backend": backend.as_str(),
        "changed": receipt.changed(),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// Map listed device rows to the `get_network_state` result's device summary
/// (Task 3.5). Bounded (device counts are small on a laptop).
#[must_use]
pub fn network_devices_result(rows: &[RawNetworkDevice]) -> serde_json::Value {
    let devices: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "device": row.name,
                "kind": match row.kind() {
                    NetworkDeviceKind::Wifi => "wifi",
                    NetworkDeviceKind::Ethernet => "ethernet",
                    NetworkDeviceKind::Other => "other",
                },
                "connected": row.is_connected(),
            })
        })
        .collect();
    serde_json::json!({ "devices": devices })
}

/// Map listed profile rows to a saved-profile summary (Task 3.5), used by
/// `get_network_state`/`activate_network_profile` preflight presentation.
/// Carries only the opaque UUID identity, never a raw object path.
#[must_use]
pub fn network_profiles_result(rows: &[RawNetworkProfile]) -> serde_json::Value {
    let profiles: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "profile": row.uuid,
                "name": row.name,
                "kind": match row.kind() {
                    NetworkDeviceKind::Wifi => "wifi",
                    NetworkDeviceKind::Ethernet => "ethernet",
                    NetworkDeviceKind::Other => "other",
                },
                "active": row.is_active(),
            })
        })
        .collect();
    serde_json::json!({ "profiles": profiles })
}

// ─────────────────────────────────────────────────────────────────────────────
// `HostOsControl::connectivity()` port seam (design §4)
// ─────────────────────────────────────────────────────────────────────────────

/// The dyn-compatible connectivity domain port design §4 names
/// `fn connectivity(&self) -> &dyn ConnectivityControl` on `HostOsControl`.
/// Because the concrete [`ConnectivityControl`] provider struct above is
/// generic over its [`ConnectivityTransport`], `HostOsControl::connectivity()`
/// returns this object-safe supertrait instead so any transport (live
/// NetworkManager/`nmcli`, or a deny-live fake) can be composed behind one
/// erased reference. Every [`ConnectivityControl<T>`] implements it
/// automatically via the blanket impl below.
#[async_trait]
pub trait ConnectivityControlPort:
    DesiredStateControl<ConnectivityRequest, ConnectivityState>
{
    /// Scan for available Wi-Fi networks (erased passthrough for the
    /// read-only `get_wifi_networks` tool, which is not part of the mutation
    /// lifecycle).
    async fn scan_wifi(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<RawWifiNetwork>, OsControlError>;

    /// List known network devices (erased passthrough backing
    /// `get_network_state`'s device summary and device-targeted preflight,
    /// Task 3.5).
    async fn list_devices(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<RawNetworkDevice>, OsControlError>;

    /// List saved network profiles (erased passthrough backing
    /// `get_network_state`'s profile summary and `forget_wifi`/
    /// `activate_network_profile` identity resolution, Task 3.5).
    async fn list_profiles(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<RawNetworkProfile>, OsControlError>;

    /// The composed backend label (for the `backend` result field).
    fn backend(&self) -> ConnectivityBackend;

    // ── Tasks 4.2 / 5.3 / 5.6 reads ─────────────────────────────────────────

    /// List saved VPN profiles with their activation state
    /// (`list_vpn_profiles`).
    async fn list_vpn_profiles(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<VpnProfileSummary>, OsControlError>;

    /// Produce a layered network diagnosis (`diagnose_network`).
    /// `target_named` records that the caller asked about a specific host.
    async fn diagnose(
        &self,
        ctx: &HostExecutionContext,
        target_named: bool,
    ) -> Result<NetworkDiagnosisFacts, OsControlError>;

    /// Read the hotspot state for a device, or the sole Wi-Fi device
    /// (`get_hotspot_state`).
    async fn get_hotspot_state(
        &self,
        ctx: &HostExecutionContext,
        device: Option<&NetworkDeviceId>,
    ) -> Result<HotspotFacts, OsControlError>;

    /// Read the desktop-wide proxy state (`get_proxy_state`).
    async fn get_proxy_state(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<ProxyFacts, OsControlError>;

    /// List value-free metadata for saved connectivity credentials
    /// (`list_saved_connectivity_credentials`).
    async fn list_saved_credentials(
        &self,
        ctx: &HostExecutionContext,
        kind: Option<ConnectivityCredentialKind>,
    ) -> Result<Vec<SavedCredentialSummary>, OsControlError>;

    /// The desktop proxy backend label (for the `backend` result field).
    fn proxy_backend(&self) -> ProxyBackend;
}

#[async_trait]
impl<T: ConnectivityTransport> ConnectivityControlPort for ConnectivityControl<T> {
    async fn scan_wifi(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<RawWifiNetwork>, OsControlError> {
        self.scan_wifi_networks(ctx).await
    }

    async fn list_devices(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<RawNetworkDevice>, OsControlError> {
        self.transport.list_devices(ctx).await
    }

    async fn list_profiles(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<RawNetworkProfile>, OsControlError> {
        self.transport.list_profiles(ctx).await
    }

    fn backend(&self) -> ConnectivityBackend {
        ConnectivityControl::backend(self)
    }

    async fn list_vpn_profiles(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<VpnProfileSummary>, OsControlError> {
        self.list_vpn_profiles_read(ctx).await
    }

    async fn diagnose(
        &self,
        ctx: &HostExecutionContext,
        target_named: bool,
    ) -> Result<NetworkDiagnosisFacts, OsControlError> {
        self.diagnose_read(ctx, target_named).await
    }

    async fn get_hotspot_state(
        &self,
        ctx: &HostExecutionContext,
        device: Option<&NetworkDeviceId>,
    ) -> Result<HotspotFacts, OsControlError> {
        self.hotspot_state_read(ctx, device).await
    }

    async fn get_proxy_state(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<ProxyFacts, OsControlError> {
        self.proxy_state_read(ctx).await
    }

    async fn list_saved_credentials(
        &self,
        ctx: &HostExecutionContext,
        kind: Option<ConnectivityCredentialKind>,
    ) -> Result<Vec<SavedCredentialSummary>, OsControlError> {
        self.saved_credentials_read(ctx, kind).await
    }

    fn proxy_backend(&self) -> ProxyBackend {
        self.transport.proxy_backend()
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn radio_observation_digest_ignores_connection_focus_fields() {
        let a = ConnectivityState::radio(true);
        let b = ConnectivityState::radio(true);
        assert_eq!(a.observation_digest(), b.observation_digest());
        let c = ConnectivityState::radio(false);
        assert_ne!(a.observation_digest(), c.observation_digest());
    }

    #[test]
    fn connection_observation_digest_distinguishes_none_from_empty() {
        let connected = ConnectivityState::connection(Some("MyNet".to_string()));
        let disconnected = ConnectivityState::connection(None);
        assert_ne!(
            connected.observation_digest(),
            disconnected.observation_digest()
        );
    }

    #[test]
    fn desired_state_focuses_on_the_changed_dimension() {
        let toggle = ConnectivityRequest {
            action: "toggle_wifi".to_string(),
            params: serde_json::json!({ "enabled": true }),
            op: ConnectivityOp::ToggleRadio(true),
        };
        assert_eq!(toggle.focus(), ConnectivityFocus::Radio);
        assert!(toggle.desired_state().radio_enabled);

        let connect = ConnectivityRequest {
            action: "connect_wifi".to_string(),
            params: serde_json::json!({ "ssid": "MyNet" }),
            op: ConnectivityOp::ConnectWifi(ConnectWifiOp {
                ssid: "MyNet".to_string(),
                password: None,
                credential: None,
            }),
        };
        assert_eq!(connect.focus(), ConnectivityFocus::Connection);
        assert_eq!(
            connect.desired_state().active_ssid,
            Some("MyNet".to_string())
        );
    }

    #[test]
    fn ambiguous_candidates_none_for_single_access_point() {
        let rows = vec![RawWifiNetwork {
            ssid: "MyNet".to_string(),
            bssid: Some("AA:BB:CC:DD:EE:01".to_string()),
            signal_percent: Some(80),
            security: "WPA2".to_string(),
        }];
        assert!(ConnectivityControl::<
            crate::os_control::connectivity::fake::FakeConnectivityTransport,
        >::ambiguous_candidates(&rows, "MyNet")
        .is_none());
    }

    #[test]
    fn ambiguous_candidates_detected_for_two_distinct_access_points() {
        let rows = vec![
            RawWifiNetwork {
                ssid: "MyNet".to_string(),
                bssid: Some("AA:BB:CC:DD:EE:01".to_string()),
                signal_percent: Some(80),
                security: "WPA2".to_string(),
            },
            RawWifiNetwork {
                ssid: "MyNet".to_string(),
                bssid: Some("AA:BB:CC:DD:EE:02".to_string()),
                signal_percent: Some(40),
                security: "WPA2".to_string(),
            },
            RawWifiNetwork {
                ssid: "OtherNet".to_string(),
                bssid: Some("AA:BB:CC:DD:EE:03".to_string()),
                signal_percent: Some(90),
                security: "WPA2".to_string(),
            },
        ];
        let candidates = ConnectivityControl::<
            crate::os_control::connectivity::fake::FakeConnectivityTransport,
        >::ambiguous_candidates(&rows, "MyNet")
        .expect("two access points sharing an SSID must be ambiguous");
        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn wifi_networks_result_preserves_existing_field_names() {
        let rows = vec![RawWifiNetwork {
            ssid: "MyNet".to_string(),
            bssid: Some("AA:BB:CC:DD:EE:01".to_string()),
            signal_percent: Some(80),
            security: "WPA2".to_string(),
        }];
        let value = wifi_networks_result(&rows);
        let networks = value["networks"].as_array().expect("networks array");
        assert_eq!(networks[0]["ssid"], "MyNet");
        assert_eq!(networks[0]["signal"], "80");
        assert_eq!(networks[0]["security"], "WPA2");
    }
}
