//! Pure, table-driven parsers for the connectivity (`nmcli`) fallback adapter.
//!
//! linux-os-control-production **Task 2.3** — "Migrate Wi-Fi and power-profile
//! controls" (OSC-015, OSC-020, OSC-025, OSC-029, OSC-031), design §9.4.
//!
//! These functions are the migrated home of the Wi-Fi parsers that previously
//! lived (and directly drove subprocesses) in `tools/system_config.rs`. Here
//! they are **pure** string→value functions with no process access, so the
//! governed [`super::ConnectivityControl`] provider and its transports can be
//! tested entirely with captured fixtures.
//!
//! # Ambiguity never reports success
//!
//! Every parser returns `None`/an empty result when the backend output cannot
//! be parsed into an unambiguous value. The provider maps that to a
//! non-success outcome (`Unavailable`/`Unverified`) — parser ambiguity is
//! **never** reported as a satisfied state (OSC-031).
//!
//! # `nmcli -t` escaping
//!
//! Terse (`-t`) `nmcli` output separates fields with `:` and escapes a literal
//! `:` inside a field value as `\:` (this matters for BSSIDs, which are
//! colon-separated MAC addresses). [`split_terse_fields`] is the single place
//! that un-escapes and splits a terse line so every parser below handles this
//! consistently.

/// Split one `nmcli -t` terse line into its unescaped fields. A `\:` inside a
/// field value is un-escaped to a literal `:` (e.g. a BSSID); an unescaped `:`
/// is the field separator.
#[must_use]
pub fn split_terse_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(&next) = chars.peek() {
                    // Un-escape `\:` (and, defensively, `\\`) to the literal
                    // character; any other escaped char is kept verbatim.
                    if next == ':' || next == '\\' {
                        current.push(next);
                        chars.next();
                        continue;
                    }
                }
                current.push('\\');
            }
            ':' => {
                fields.push(std::mem::take(&mut current));
            }
            other => current.push(other),
        }
    }
    fields.push(current);
    fields
}

/// Parse an `nmcli radio wifi` reply into an enabled/disabled boolean. Returns
/// `None` on unrecognized output so ambiguity never reports a fabricated state.
#[must_use]
pub fn parse_radio_state(output: &str) -> Option<bool> {
    let normalized = output.trim().to_lowercase();
    if normalized.contains("enabled") || normalized == "on" {
        Some(true)
    } else if normalized.contains("disabled") || normalized == "off" {
        Some(false)
    } else {
        None
    }
}

/// Parse an `nmcli -t -f ACTIVE,SSID device wifi list` reply into the currently
/// active SSID, if any. Returns `Ok(None)` when no row is marked active — a
/// distinct, unambiguous "not connected" result rather than a parse failure.
#[must_use]
pub fn parse_active_ssid(output: &str) -> Option<String> {
    for line in output.lines() {
        let fields = split_terse_fields(line);
        let active = fields.first().map(String::as_str).unwrap_or_default();
        let ssid = fields.get(1).map(String::as_str).unwrap_or_default();
        if active.eq_ignore_ascii_case("yes") && !ssid.is_empty() {
            return Some(ssid.to_string());
        }
    }
    None
}

/// One parsed row of `nmcli -t -f SSID,BSSID,SIGNAL,SECURITY device wifi list`.
/// `bssid` is the stable per-access-point identity used to disambiguate two
/// rows that share the same `ssid` (OSC-015 duplicate-SSID clarification).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawWifiNetwork {
    /// The advertised network name. Not unique: two access points may share it.
    pub ssid: String,
    /// The access point's hardware address; the stable per-row identity.
    pub bssid: Option<String>,
    /// Signal strength percentage (0-100), when reported.
    pub signal_percent: Option<u8>,
    /// Raw security label (e.g. `WPA2`, `--` for open).
    pub security: String,
}

/// Parse an `nmcli -t -f SSID,BSSID,SIGNAL,SECURITY device wifi list` reply
/// into structured rows. Rows with an empty SSID (hidden networks) are kept —
/// callers that need a stable target identity require a non-empty SSID
/// separately. Unparseable signal values are `None`, never a fabricated 0.
#[must_use]
pub fn parse_wifi_list(output: &str) -> Vec<RawWifiNetwork> {
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields = split_terse_fields(line);
            let ssid = fields.first().cloned().unwrap_or_default();
            let bssid = fields.get(1).filter(|s| !s.is_empty()).cloned();
            let signal_percent = fields
                .get(2)
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(|v| v.min(100) as u8);
            let security = fields.get(3).cloned().unwrap_or_default();
            RawWifiNetwork {
                ssid,
                bssid,
                signal_percent,
                security,
            }
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Device / profile listing (Task 3.5) — stable identity for device/profile
// targeted mutations
// ─────────────────────────────────────────────────────────────────────────────

/// One parsed row of `nmcli -t -f DEVICE,TYPE,STATE device status`. The raw
/// `nmcli` device name is the identity source the caller wraps into a stable
/// [`super::NetworkDeviceId`] — never surfaced as a raw device-node/object-path
/// string to the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawNetworkDevice {
    /// The `nmcli` device name (identity source).
    pub name: String,
    /// The raw `nmcli` device type (`wifi`, `ethernet`, or another value).
    pub device_type: String,
    /// The raw `nmcli` device state (e.g. `connected`, `disconnected`).
    pub state: String,
}

impl RawNetworkDevice {
    /// The typed device kind this row maps to.
    #[must_use]
    pub fn kind(&self) -> super::NetworkDeviceKind {
        match self.device_type.as_str() {
            "wifi" => super::NetworkDeviceKind::Wifi,
            "ethernet" => super::NetworkDeviceKind::Ethernet,
            _ => super::NetworkDeviceKind::Other,
        }
    }

    /// Whether this row's state indicates a connected device.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.state.eq_ignore_ascii_case("connected")
    }
}

/// Parse an `nmcli -t -f DEVICE,TYPE,STATE device status` reply into
/// structured rows. Rows with an empty device name are dropped — a device with
/// no stable identity source cannot back a typed [`super::NetworkDeviceId`].
#[must_use]
pub fn parse_device_status(output: &str) -> Vec<RawNetworkDevice> {
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let fields = split_terse_fields(line);
            let name = fields.first().cloned().unwrap_or_default();
            if name.is_empty() {
                return None;
            }
            Some(RawNetworkDevice {
                name,
                device_type: fields.get(1).cloned().unwrap_or_default(),
                state: fields.get(2).cloned().unwrap_or_default(),
            })
        })
        .collect()
}

/// One parsed row of `nmcli -t -f NAME,UUID,TYPE,DEVICE connection show`. The
/// `uuid` is the stable identity source the caller wraps into a
/// [`super::NetworkProfileId`] — a saved profile's display name is not unique
/// and is kept only as a redacted label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawNetworkProfile {
    /// The saved connection's display name (not unique).
    pub name: String,
    /// The connection UUID (identity source).
    pub uuid: String,
    /// The raw `nmcli` connection type (`802-11-wireless`, `802-3-ethernet`,
    /// or another value).
    pub connection_type: String,
    /// The bound device name, when the profile is currently active on one.
    pub device: Option<String>,
}

impl RawNetworkProfile {
    /// The typed device kind this profile maps to.
    #[must_use]
    pub fn kind(&self) -> super::NetworkDeviceKind {
        match self.connection_type.as_str() {
            "802-11-wireless" => super::NetworkDeviceKind::Wifi,
            "802-3-ethernet" => super::NetworkDeviceKind::Ethernet,
            _ => super::NetworkDeviceKind::Other,
        }
    }

    /// Whether this profile is currently active (bound to a device).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.device.is_some()
    }
}

/// Parse an `nmcli -t -f NAME,UUID,TYPE,DEVICE connection show` reply into
/// structured rows. Rows with an empty UUID are dropped — a profile with no
/// stable identity source cannot back a typed [`super::NetworkProfileId`].
#[must_use]
pub fn parse_connection_show(output: &str) -> Vec<RawNetworkProfile> {
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let fields = split_terse_fields(line);
            let uuid = fields.get(1).cloned().unwrap_or_default();
            if uuid.is_empty() {
                return None;
            }
            Some(RawNetworkProfile {
                name: fields.first().cloned().unwrap_or_default(),
                uuid,
                connection_type: fields.get(2).cloned().unwrap_or_default(),
                device: fields.get(3).filter(|s| !s.is_empty()).cloned(),
            })
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Active-connection listing (Tasks 4.2 / 5.3) — VPN and hotspot activation
// ─────────────────────────────────────────────────────────────────────────────

/// Split one terse `KEY:VALUE` line, re-joining a value that itself contained
/// unescaped separators (an `IP4.ROUTE[1]` value carries `, ` and `=`, and an
/// IPv6 value carries `:`). Returns `None` when the line has no separator at
/// all, which is never a key/value fact.
#[must_use]
fn split_key_value(line: &str) -> Option<(String, String)> {
    let fields = split_terse_fields(line);
    if fields.len() < 2 {
        return None;
    }
    let key = fields[0].clone();
    if key.is_empty() {
        return None;
    }
    Some((key, fields[1..].join(":")))
}

/// One parsed row of `nmcli -t -f UUID,TYPE,STATE,DEVICE connection show
/// --active`. The `uuid` is the stable identity — a profile NAME is never used
/// here because two saved profiles may share one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawActiveConnection {
    /// The active connection's profile UUID (identity source).
    pub uuid: String,
    /// The raw `nmcli` connection type (`vpn`, `wireguard`, `802-11-wireless`…).
    pub connection_type: String,
    /// The raw activation state (`activated`, `activating`, `deactivating`).
    pub state: String,
    /// The device the connection is bound to, when it has one.
    pub device: Option<String>,
}

impl RawActiveConnection {
    /// Whether this row reports a fully activated connection. `activating` is
    /// deliberately **not** activated: an in-flight activation is a different
    /// fact from a completed one.
    #[must_use]
    pub fn is_activated(&self) -> bool {
        self.state.eq_ignore_ascii_case("activated")
    }
}

/// Whether a raw `nmcli` connection type denotes a VPN-class profile. Both the
/// generic `vpn` plugin type and the native `wireguard` type qualify.
#[must_use]
pub fn is_vpn_connection_type(connection_type: &str) -> bool {
    matches!(connection_type, "vpn" | "wireguard")
}

/// Parse an `nmcli -t -f UUID,TYPE,STATE,DEVICE connection show --active` reply.
/// Rows with an empty UUID are dropped — an active connection with no stable
/// identity source cannot back a typed [`super::NetworkProfileId`].
#[must_use]
pub fn parse_active_connections(output: &str) -> Vec<RawActiveConnection> {
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let fields = split_terse_fields(line);
            let uuid = fields.first().cloned().unwrap_or_default();
            if uuid.is_empty() {
                return None;
            }
            Some(RawActiveConnection {
                uuid,
                connection_type: fields.get(1).cloned().unwrap_or_default(),
                state: fields.get(2).cloned().unwrap_or_default(),
                device: fields.get(3).filter(|s| !s.is_empty()).cloned(),
            })
        })
        .collect()
}

/// Read one property value out of an `nmcli -t -f <property> connection show
/// …` reply. Returns `None` when the requested key is absent from the output —
/// "the property was not reported" is a different fact from "the property is
/// empty", and only the caller knows which one is admissible.
#[must_use]
pub fn parse_terse_property(output: &str, key: &str) -> Option<String> {
    output
        .lines()
        .filter_map(split_key_value)
        .find(|(k, _)| k == key)
        .map(|(_, value)| value)
}

/// The value `nmcli` substitutes for a stored secret when `--show-secrets` was
/// **not** passed. Its presence is the only credential signal this codebase
/// ever reads: the real value is never requested, so it can never be logged.
pub const NMCLI_HIDDEN_SECRET: &str = "<hidden>";

/// Decide whether a credential is stored for a profile property, from an
/// `nmcli -t -f <property> connection show …` reply that was deliberately run
/// **without** `--show-secrets`.
///
/// * `Some(true)`  — the property reported [`NMCLI_HIDDEN_SECRET`]: a value is stored.
/// * `Some(false)` — the property was reported and is empty: no value is stored.
/// * `None`        — the property was absent or carried an unrecognised token:
///   the presence of a credential could not be determined. Never collapsed into
///   `false`, which would let a delete "verify" against a fact nobody read.
#[must_use]
pub fn parse_secret_presence(output: &str, key: &str) -> Option<bool> {
    let value = parse_terse_property(output, key)?;
    let trimmed = value.trim();
    if trimmed == NMCLI_HIDDEN_SECRET {
        Some(true)
    } else if trimmed.is_empty() {
        Some(false)
    } else {
        // A concrete value here would mean secrets were exposed on the wire.
        // Refuse to interpret it rather than reason about a leaked credential.
        None
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Host connectivity verdict (Task 4.2, `diagnose_network`)
// ─────────────────────────────────────────────────────────────────────────────

/// NetworkManager's own connectivity verdict.
///
/// [`Self::Undetermined`] exists so "the check could not decide" is never
/// reported as "there is no internet": a diagnosis that claims offline when the
/// probe itself failed is worse than one that admits it does not know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostConnectivity {
    /// Full internet reachability.
    Full,
    /// A network is reachable but the internet is not.
    Limited,
    /// A captive portal intercepted the check.
    Portal,
    /// NetworkManager positively determined there is no connectivity.
    Unreachable,
    /// NetworkManager could not determine connectivity (its `unknown` verdict,
    /// e.g. its own checker is disabled). **Not** the same as unreachable.
    Undetermined,
}

impl HostConnectivity {
    /// The stable label surfaced in a diagnosis field.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "reachable",
            Self::Limited => "limited",
            Self::Portal => "captive_portal",
            Self::Unreachable => "unreachable",
            Self::Undetermined => "undetermined",
        }
    }

    /// The captive-portal field derived from this verdict. Only `portal` proves
    /// a portal, and only `full` proves its absence; everything else is unknown.
    #[must_use]
    pub fn captive_portal(self) -> &'static str {
        match self {
            Self::Portal => "detected",
            Self::Full => "not_detected",
            _ => "undetermined",
        }
    }
}

/// Parse an `nmcli -t -f STATE,CONNECTIVITY general` reply into the verdict.
/// Returns `None` for unrecognised output so an unparseable reply surfaces as an
/// error instead of a fabricated verdict.
#[must_use]
pub fn parse_connectivity(output: &str) -> Option<HostConnectivity> {
    let line = output.lines().find(|line| !line.trim().is_empty())?;
    let fields = split_terse_fields(line);
    let verdict = fields.last()?.trim().to_ascii_lowercase();
    match verdict.as_str() {
        "full" => Some(HostConnectivity::Full),
        "limited" => Some(HostConnectivity::Limited),
        "portal" => Some(HostConnectivity::Portal),
        "none" => Some(HostConnectivity::Unreachable),
        "unknown" => Some(HostConnectivity::Undetermined),
        _ => None,
    }
}

/// Presence-only IP facts for one device, parsed from `nmcli -t -f
/// GENERAL.STATE,IP4.ADDRESS,IP4.GATEWAY,IP4.DNS,IP4.ROUTE device show <dev>`.
///
/// Only presence and counts are retained — never an address, gateway or
/// resolver value — because a diagnosis is surfaced to the model and those
/// values are sensitive metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDeviceIpFacts {
    /// The raw `GENERAL.STATE` token (e.g. `100 (connected)`), when reported.
    pub general_state: Option<String>,
    /// How many IPv4 addresses are assigned.
    pub address_count: usize,
    /// Whether a gateway is configured.
    pub has_gateway: bool,
    /// How many resolvers are configured.
    pub dns_count: usize,
    /// Whether a default (`0.0.0.0/0`) route is installed.
    pub has_default_route: bool,
}

impl RawDeviceIpFacts {
    /// The `link` diagnosis field: `up` only for a connected device, `down` for
    /// a positively reported non-connected state, `undetermined` when the state
    /// was not reported at all.
    #[must_use]
    pub fn link(&self) -> &'static str {
        // NEVER use `contains("connected")`: nmcli reports `30 (disconnected)`,
        // which *contains* that substring, so a disconnected device would be
        // reported as up — telling the user they are online when they are not,
        // and letting a connect request verify as already satisfied.
        //
        // The numeric code is authoritative (NM_DEVICE_STATE_ACTIVATED == 100);
        // the parenthesised word is a human label that varies by version.
        let Some(state) = &self.general_state else {
            return "undetermined";
        };
        let code = state
            .split_whitespace()
            .next()
            .and_then(|token| token.parse::<u32>().ok());
        match code {
            Some(100) => "up",
            Some(_) => "down",
            // No numeric code: fall back to an EXACT token match on the label so
            // `disconnected` can never satisfy `connected`.
            None => {
                let label = state.to_ascii_lowercase();
                let word = label
                    .trim()
                    .trim_start_matches('(')
                    .trim_end_matches(')')
                    .trim();
                if word == "connected" || word == "activated" {
                    "up"
                } else {
                    "down"
                }
            }
        }
    }
}

/// Parse an `nmcli -t -f GENERAL.STATE,IP4.ADDRESS,IP4.GATEWAY,IP4.DNS,IP4.ROUTE
/// device show <dev>` reply. Returns `None` when no `KEY:VALUE` line was
/// present at all — an unreadable reply must not become "nothing configured".
#[must_use]
pub fn parse_device_ip_facts(output: &str) -> Option<RawDeviceIpFacts> {
    let mut facts = RawDeviceIpFacts {
        general_state: None,
        address_count: 0,
        has_gateway: false,
        dns_count: 0,
        has_default_route: false,
    };
    let mut saw_any = false;
    for (key, value) in output.lines().filter_map(split_key_value) {
        saw_any = true;
        let base = key.split('[').next().unwrap_or(&key);
        let value = value.trim();
        match base {
            "GENERAL.STATE" if !value.is_empty() => {
                facts.general_state = Some(value.to_string());
            }
            "IP4.ADDRESS" if !value.is_empty() => facts.address_count += 1,
            "IP4.GATEWAY" if !value.is_empty() => facts.has_gateway = true,
            "IP4.DNS" if !value.is_empty() => facts.dns_count += 1,
            "IP4.ROUTE" if value.contains("dst = 0.0.0.0/0") => {
                facts.has_default_route = true;
            }
            _ => {}
        }
    }
    saw_any.then_some(facts)
}

// ─────────────────────────────────────────────────────────────────────────────
// Desktop proxy values (Task 5.3) — `gsettings get` output
// ─────────────────────────────────────────────────────────────────────────────

/// Parse a `gsettings get` string reply (`'manual'`). Returns `None` for any
/// output that is not a single quoted GVariant string, so an unparseable proxy
/// state fails closed rather than defaulting to "no proxy".
#[must_use]
pub fn parse_gsettings_string(output: &str) -> Option<String> {
    let trimmed = output.trim();
    let inner = trimmed
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))?;
    if inner.contains('\'') {
        return None;
    }
    Some(inner.to_string())
}

/// Parse a `gsettings get` integer reply (`8080`). Returns `None` for anything
/// that is not a bare non-negative integer in port range.
#[must_use]
pub fn parse_gsettings_port(output: &str) -> Option<u16> {
    output.trim().parse::<u16>().ok()
}

/// Parse a `gsettings get` string-array reply (`['localhost', '::1']`).
/// `Some(vec![])` for the empty array `[]`; `None` when the reply is not a
/// well-formed array of quoted strings.
#[must_use]
pub fn parse_gsettings_string_list(output: &str) -> Option<Vec<String>> {
    let trimmed = output.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))?
        .trim();
    if inner.is_empty() {
        return Some(Vec::new());
    }
    inner
        .split(',')
        .map(|item| parse_gsettings_string(item))
        .collect()
}

/// The recognised desktop proxy modes, in the frozen contract's vocabulary
/// (`none` / `automatic` / `manual`) rather than the backend's own tokens.
/// Returns `None` for any other token so an unrecognised mode is an error.
#[must_use]
pub fn proxy_mode_from_backend(token: &str) -> Option<&'static str> {
    match token {
        "none" => Some("none"),
        "auto" => Some("automatic"),
        "manual" => Some("manual"),
        _ => None,
    }
}

/// The backend token for a contract proxy mode. Returns `None` for an
/// unrecognised mode so an invalid request is rejected before any argv is built.
#[must_use]
pub fn proxy_mode_to_backend(mode: &str) -> Option<&'static str> {
    match mode {
        "none" => Some("none"),
        "automatic" => Some("auto"),
        "manual" => Some("manual"),
        _ => None,
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn split_terse_fields_handles_escaped_colons() {
        // A BSSID's colons are escaped by nmcli terse output.
        assert_eq!(
            split_terse_fields(r"MyNet\:Guest:AA\:BB\:CC\:DD\:EE\:FF:70:WPA2"),
            vec!["MyNet:Guest", "AA:BB:CC:DD:EE:FF", "70", "WPA2"]
        );
        assert_eq!(split_terse_fields("a:b:c"), vec!["a", "b", "c"]);
        assert_eq!(split_terse_fields(""), vec![""]);
    }

    #[test]
    fn radio_state_table() {
        assert_eq!(parse_radio_state("enabled"), Some(true));
        assert_eq!(parse_radio_state("disabled"), Some(false));
        assert_eq!(parse_radio_state("on"), Some(true));
        assert_eq!(parse_radio_state("off"), Some(false));
        assert_eq!(parse_radio_state("garbage"), None);
        assert_eq!(parse_radio_state(""), None);
    }

    #[test]
    fn active_ssid_table() {
        let output = "no:Other\nyes:MyHomeNet\nno:Guest";
        assert_eq!(parse_active_ssid(output), Some("MyHomeNet".to_string()));
        assert_eq!(parse_active_ssid("no:Other\nno:Guest"), None);
        assert_eq!(parse_active_ssid(""), None);
        // An active row with an empty SSID is not a usable identity.
        assert_eq!(parse_active_ssid("yes:"), None);
    }

    #[test]
    fn wifi_list_table() {
        let output = "HomeNet:AA\\:BB\\:CC\\:DD\\:EE\\:01:80:WPA2\nHomeNet:AA\\:BB\\:CC\\:DD\\:EE\\:02:40:WPA2\nGuest::60:--";
        let rows = parse_wifi_list(output);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].ssid, "HomeNet");
        assert_eq!(rows[0].bssid.as_deref(), Some("AA:BB:CC:DD:EE:01"));
        assert_eq!(rows[0].signal_percent, Some(80));
        assert_eq!(rows[0].security, "WPA2");
        assert_eq!(rows[1].bssid.as_deref(), Some("AA:BB:CC:DD:EE:02"));
        assert_eq!(rows[2].bssid, None);
        assert_eq!(rows[2].security, "--");
    }

    #[test]
    fn wifi_list_unparseable_signal_is_none_not_zero() {
        let rows = parse_wifi_list("Net:AA\\:BB\\:CC\\:DD\\:EE\\:01:not-a-number:WPA2");
        assert_eq!(rows[0].signal_percent, None);
    }

    #[test]
    fn wifi_list_empty_input_is_empty_vec() {
        assert!(parse_wifi_list("").is_empty());
        assert!(parse_wifi_list("\n\n").is_empty());
    }

    #[test]
    fn device_status_table() {
        let output = "wlan0:wifi:connected\neth0:ethernet:disconnected\nlo:loopback:unmanaged";
        let rows = parse_device_status(output);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].name, "wlan0");
        assert_eq!(rows[0].kind(), super::super::NetworkDeviceKind::Wifi);
        assert!(rows[0].is_connected());
        assert_eq!(rows[1].kind(), super::super::NetworkDeviceKind::Ethernet);
        assert!(!rows[1].is_connected());
        assert_eq!(rows[2].kind(), super::super::NetworkDeviceKind::Other);
    }

    #[test]
    fn device_status_empty_name_dropped() {
        let rows = parse_device_status(":wifi:connected\nwlan0:wifi:connected");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "wlan0");
    }

    #[test]
    fn connection_show_table() {
        let output = "HomeNet:11111111-1111-1111-1111-111111111111:802-11-wireless:wlan0\nWiredNet:22222222-2222-2222-2222-222222222222:802-3-ethernet:\nOldSaved:33333333-3333-3333-3333-333333333333:802-11-wireless:";
        let rows = parse_connection_show(output);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].name, "HomeNet");
        assert_eq!(rows[0].kind(), super::super::NetworkDeviceKind::Wifi);
        assert!(rows[0].is_active());
        assert_eq!(rows[1].kind(), super::super::NetworkDeviceKind::Ethernet);
        assert!(!rows[1].is_active());
        assert!(!rows[2].is_active());
    }

    #[test]
    fn connection_show_empty_uuid_dropped() {
        let rows = parse_connection_show(
            "Bad::802-11-wireless:\nGood:44444444-4444-4444-4444-444444444444:802-11-wireless:",
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Good");
    }

    // ── Tasks 4.2 / 5.3 / 5.6 parsers ───────────────────────────────────────

    #[test]
    fn active_connections_table() {
        let output = "11111111-1111-1111-1111-111111111111:vpn:activated:tun0\n\
                      22222222-2222-2222-2222-222222222222:802-11-wireless:activated:wlan0\n\
                      33333333-3333-3333-3333-333333333333:wireguard:activating:";
        let rows = parse_active_connections(output);
        assert_eq!(rows.len(), 3);
        assert!(rows[0].is_activated());
        assert_eq!(rows[0].device.as_deref(), Some("tun0"));
        assert!(is_vpn_connection_type(&rows[0].connection_type));
        assert!(!is_vpn_connection_type(&rows[1].connection_type));
        // `activating` is an in-flight fact, not an activated one.
        assert!(!rows[2].is_activated());
        assert!(is_vpn_connection_type(&rows[2].connection_type));
        assert_eq!(rows[2].device, None);
    }

    #[test]
    fn active_connections_empty_uuid_dropped() {
        let rows = parse_active_connections(":vpn:activated:tun0");
        assert!(rows.is_empty());
    }

    #[test]
    fn terse_property_absent_key_is_none_not_empty() {
        let output = "802-11-wireless.mode:ap";
        assert_eq!(
            parse_terse_property(output, "802-11-wireless.mode"),
            Some("ap".to_string())
        );
        // Absent is distinct from empty: the caller decides which is admissible.
        assert_eq!(
            parse_terse_property(output, "802-11-wireless-security.key-mgmt"),
            None
        );
        assert_eq!(
            parse_terse_property("802-11-wireless-security.key-mgmt:", "802-11-wireless-security.key-mgmt"),
            Some(String::new())
        );
    }

    #[test]
    fn secret_presence_table() {
        let key = "802-11-wireless-security.psk";
        assert_eq!(
            parse_secret_presence("802-11-wireless-security.psk:<hidden>", key),
            Some(true)
        );
        assert_eq!(
            parse_secret_presence("802-11-wireless-security.psk:", key),
            Some(false)
        );
    }

    #[test]
    fn secret_presence_unrecognised_output_is_an_error_not_a_default() {
        let key = "802-11-wireless-security.psk";
        // Absent property: presence could not be determined.
        assert_eq!(parse_secret_presence("", key), None);
        assert_eq!(parse_secret_presence("connection.id:HomeNet", key), None);
        // A concrete token would mean secrets were exposed; refuse to interpret.
        assert_eq!(
            parse_secret_presence("802-11-wireless-security.psk:something", key),
            None
        );
    }

    #[test]
    fn connectivity_unknown_is_undetermined_never_unreachable() {
        assert_eq!(
            parse_connectivity("connected:unknown"),
            Some(HostConnectivity::Undetermined)
        );
        assert_eq!(
            parse_connectivity("connected:none"),
            Some(HostConnectivity::Unreachable)
        );
        assert_ne!(
            parse_connectivity("connected:unknown"),
            parse_connectivity("connected:none")
        );
        assert_eq!(HostConnectivity::Undetermined.as_str(), "undetermined");
        assert_eq!(HostConnectivity::Unreachable.as_str(), "unreachable");
    }

    #[test]
    fn connectivity_table_and_captive_portal_derivation() {
        assert_eq!(
            parse_connectivity("connected:full"),
            Some(HostConnectivity::Full)
        );
        assert_eq!(parse_connectivity("full"), Some(HostConnectivity::Full));
        assert_eq!(
            parse_connectivity("connected:limited"),
            Some(HostConnectivity::Limited)
        );
        assert_eq!(
            parse_connectivity("connected:portal"),
            Some(HostConnectivity::Portal)
        );
        assert_eq!(HostConnectivity::Portal.captive_portal(), "detected");
        assert_eq!(HostConnectivity::Full.captive_portal(), "not_detected");
        // Neither proven present nor proven absent.
        assert_eq!(HostConnectivity::Limited.captive_portal(), "undetermined");
        assert_eq!(
            HostConnectivity::Unreachable.captive_portal(),
            "undetermined"
        );
    }

    #[test]
    fn connectivity_unrecognised_output_is_an_error_not_a_default() {
        assert_eq!(parse_connectivity("connected:garbage"), None);
        assert_eq!(parse_connectivity(""), None);
        assert_eq!(parse_connectivity("\n \n"), None);
    }

    #[test]
    fn device_ip_facts_table() {
        let output = "GENERAL.STATE:100 (connected)\n\
                      IP4.ADDRESS[1]:192.168.1.20/24\n\
                      IP4.GATEWAY:192.168.1.1\n\
                      IP4.DNS[1]:192.168.1.1\n\
                      IP4.DNS[2]:1.1.1.1\n\
                      IP4.ROUTE[1]:dst = 0.0.0.0/0, nh = 192.168.1.1, mt = 600\n\
                      IP4.ROUTE[2]:dst = 192.168.1.0/24, nh = 0.0.0.0, mt = 600";
        let facts = parse_device_ip_facts(output).expect("parsed");
        assert_eq!(facts.address_count, 1);
        assert!(facts.has_gateway);
        assert_eq!(facts.dns_count, 2);
        assert!(facts.has_default_route);
        assert_eq!(facts.link(), "up");
    }

    #[test]
    fn device_ip_facts_disconnected_is_down_but_unreported_is_undetermined() {
        let down = parse_device_ip_facts("GENERAL.STATE:30 (disconnected)").expect("parsed");
        assert_eq!(down.link(), "down");
        assert_eq!(down.address_count, 0);
        assert!(!down.has_default_route);

        // The device reported IP rows but no state: "up" is not a safe default.
        let unknown = parse_device_ip_facts("IP4.ADDRESS[1]:10.0.0.2/24").expect("parsed");
        assert_eq!(unknown.link(), "undetermined");
    }

    #[test]
    fn device_ip_facts_unrecognised_output_is_an_error_not_a_default() {
        // No KEY:VALUE line at all — must not become "nothing is configured".
        assert_eq!(parse_device_ip_facts(""), None);
        assert_eq!(parse_device_ip_facts("garbage without a separator"), None);
    }

    #[test]
    fn gsettings_value_table() {
        assert_eq!(parse_gsettings_string("'manual'"), Some("manual".into()));
        assert_eq!(parse_gsettings_string("'none'\n"), Some("none".into()));
        assert_eq!(parse_gsettings_string("''"), Some(String::new()));
        assert_eq!(parse_gsettings_port("8080"), Some(8080));
        assert_eq!(parse_gsettings_port("0"), Some(0));
        assert_eq!(parse_gsettings_string_list("[]"), Some(Vec::new()));
        assert_eq!(
            parse_gsettings_string_list("['localhost', '127.0.0.0/8']"),
            Some(vec!["localhost".to_string(), "127.0.0.0/8".to_string()])
        );
    }

    #[test]
    fn gsettings_unrecognised_output_is_an_error_not_a_default() {
        // An unquoted / malformed reply must fail closed, never become "none".
        assert_eq!(parse_gsettings_string("manual"), None);
        assert_eq!(parse_gsettings_string(""), None);
        assert_eq!(parse_gsettings_string("'a''b'"), None);
        assert_eq!(parse_gsettings_port("not-a-port"), None);
        assert_eq!(parse_gsettings_port("-1"), None);
        assert_eq!(parse_gsettings_port("99999"), None);
        assert_eq!(parse_gsettings_string_list("localhost"), None);
        assert_eq!(parse_gsettings_string_list("[localhost]"), None);
    }

    #[test]
    fn proxy_mode_vocabulary_round_trips_and_rejects_unknown() {
        assert_eq!(proxy_mode_from_backend("auto"), Some("automatic"));
        assert_eq!(proxy_mode_from_backend("manual"), Some("manual"));
        assert_eq!(proxy_mode_from_backend("none"), Some("none"));
        assert_eq!(proxy_mode_from_backend("automatic"), None);
        assert_eq!(proxy_mode_from_backend(""), None);
        assert_eq!(proxy_mode_to_backend("automatic"), Some("auto"));
        assert_eq!(proxy_mode_to_backend("auto"), None);
        for mode in ["none", "automatic", "manual"] {
            let backend = proxy_mode_to_backend(mode).expect("known mode");
            assert_eq!(proxy_mode_from_backend(backend), Some(mode));
        }
    }
}
